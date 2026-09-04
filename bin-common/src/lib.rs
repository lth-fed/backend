use color_eyre::eyre::Context as _;
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;
use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
use poem::listener::{Listener, RustlsCertificate, RustlsConfig, TcpListener};
use poem::{Endpoint, Middleware, Request, Response};
use sqlx::Postgres;
use sqlx::migrate::{MigrateDatabase as _, Migrator};
use sqlx::postgres::PgPoolOptions;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

pub type PgPool = sqlx_tracing::Pool<Postgres>;
pub type Transaction<'a> = sqlx_tracing::Transaction<'a, Postgres>;

pub const APP_VERSION_HEADER: &str = "x-tappen-app-version";

#[derive(Clone, Debug)]
pub struct AppVersionMetrics {
    requests: opentelemetry::metrics::Counter<u64>,
}

impl AppVersionMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            requests: opentelemetry::global::meter("bin-common")
                .u64_counter("teknologappen.client.requests")
                .build(),
        }
    }
}

impl Default for AppVersionMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Endpoint> Middleware<E> for AppVersionMetrics {
    type Output = AppVersionMetricsEndpoint<E>;

    fn transform(&self, ep: E) -> Self::Output {
        AppVersionMetricsEndpoint {
            inner: ep,
            requests: self.requests.clone(),
        }
    }
}

#[derive(Debug)]
pub struct AppVersionMetricsEndpoint<E> {
    inner: E,
    requests: opentelemetry::metrics::Counter<u64>,
}

impl<E: Endpoint> Endpoint for AppVersionMetricsEndpoint<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> poem::Result<Self::Output> {
        if let Some(version) = req
            .headers()
            .get(APP_VERSION_HEADER)
            .and_then(valid_app_version)
        {
            let attribute = opentelemetry::KeyValue::new("app.commit", version.to_owned());
            opentelemetry::Context::current()
                .span()
                .set_attribute(attribute.clone());
            self.requests.add(1, &[attribute]);
        }

        self.inner
            .call(req)
            .await
            .map(poem::IntoResponse::into_response)
    }
}

fn valid_app_version(value: &poem::http::HeaderValue) -> Option<&str> {
    value.to_str().ok().filter(|value| {
        !value.is_empty()
            && value.len() <= 80
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    })
}

#[cfg(test)]
mod app_version_tests {
    use super::valid_app_version;

    #[test]
    fn accepts_commit_like_versions() {
        let version = poem::http::HeaderValue::from_static("1.4.0-a81f3c_dirty");
        assert_eq!(valid_app_version(&version), Some("1.4.0-a81f3c_dirty"));
    }

    #[test]
    fn rejects_values_that_could_create_unbounded_metric_labels() {
        let spaces = poem::http::HeaderValue::from_static("arbitrary version");
        let too_long = poem::http::HeaderValue::from_static(concat!(
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "aaaaaaaaaa",
            "a",
        ));

        assert_eq!(valid_app_version(&spaces), None);
        assert_eq!(valid_app_version(&too_long), None);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DebugConfig {
    pub enabled: bool,
    pub service_urls: bool,
}

impl DebugConfig {
    #[must_use]
    pub fn from_env() -> Self {
        let service_urls = std::env::var("DEBUG").as_deref() == Ok("1");
        Self {
            enabled: cfg!(debug_assertions) || service_urls,
            service_urls,
        }
    }
}

/// # Errors
///
/// Fails if DB communication or migration fails.
pub async fn setup_db(
    db_url: &str,
    migrator: Option<Migrator>,
    connections: u32,
) -> color_eyre::Result<PgPool> {
    if !Postgres::database_exists(db_url)
        .await
        .wrap_err("Failed to check if database exists")?
    {
        Postgres::create_database(db_url).await?;
    }

    let db = PgPoolOptions::new()
        .max_connections(connections)
        .acquire_slow_threshold(std::time::Duration::from_secs(5))
        .connect(db_url)
        .await
        .wrap_err("Failed to create database pool")?;

    if let Some(migrator) = migrator {
        migrator
            .run(&db)
            .await
            .wrap_err("Failed to run migrations")?;
    }

    Ok(sqlx_tracing::Pool::from(db))
}

/// # Errors
///
/// Errors if we fail to connect to OTEL collector.
pub fn get_otel(
    service_name: &'static str,
    test: bool,
) -> color_eyre::Result<opentelemetry_sdk::trace::SdkTracerProvider> {
    let debug = DebugConfig::from_env();
    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name)
        .with_attributes([opentelemetry::KeyValue::new(
            "deployment.environment.name",
            if debug.enabled { "dev" } else { "production" },
        )])
        .build();

    if test {
        // For tests; this may be run several times.
        let _: Result<(), _> = tracing_subscriber::fmt().try_init();
        return Ok(opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_simple_exporter(opentelemetry_sdk::testing::trace::NoopSpanExporter::new())
            .build());
    }

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_protocol(opentelemetry_otlp::Protocol::Grpc)
        .build()?;
    let batch_span = BatchSpanProcessor::builder(span_exporter, opentelemetry_sdk::runtime::Tokio)
        .with_batch_config(
            opentelemetry_sdk::trace::BatchConfigBuilder::default()
                .with_max_queue_size(4096)
                .build(),
        )
        .build();

    let tracer = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_span_processor(batch_span)
        .build();

    let log_exporter = opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .with_protocol(opentelemetry_otlp::Protocol::Grpc)
        .build()?;
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let processor =
        BatchLogProcessor::builder(log_exporter, opentelemetry_sdk::runtime::Tokio).build();
    let logger = opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_resource(resource.clone())
        .with_log_processor(processor)
        .build();
    let otel_layer =
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logger);
    let otel_span_appender =
        tracing_opentelemetry::layer().with_tracer(tracer.tracer(service_name));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer)
        .with(otel_span_appender)
        .with(tracing_subscriber::fmt::layer())
        .init();

    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
    let metrics_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_protocol(opentelemetry_otlp::Protocol::Grpc)
        .build()?;

    let reader =
        PeriodicReader::builder(metrics_exporter, opentelemetry_sdk::runtime::Tokio).build();
    let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .build();
    opentelemetry::global::set_meter_provider(metrics);

    Ok(tracer)
}

/// # Panics
///
/// Panics if we fail to install a signal handler.
#[allow(clippy::expect_used, reason = "on startup & to register a handler")]
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        signal(SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[must_use]
pub fn listeners(port: u16) -> impl Listener {
    TcpListener::bind(format!("[::]:{port}"))
        .combine(TcpListener::bind(format!("[::]:{}", port + 50)).rustls(debug_rustls_config()))
}
#[must_use]
pub fn debug_rustls_config() -> RustlsConfig {
    RustlsConfig::new().fallback(RustlsCertificate::new().key(TEST_KEY).cert(TEST_CERT))
}

const TEST_CERT: &str = "
-----BEGIN CERTIFICATE-----
MIIEADCCAmigAwIBAgICAcgwDQYJKoZIhvcNAQELBQAwLDEqMCgGA1UEAwwhcG9u
eXRvd24gUlNBIGxldmVsIDIgaW50ZXJtZWRpYXRlMB4XDTE2MDgxMzE2MDcwNFoX
DTIyMDIwMzE2MDcwNFowGTEXMBUGA1UEAwwOdGVzdHNlcnZlci5jb20wggEiMA0G
CSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCpVhh1/FNP2qvWenbZSghari/UThwe
dynfnHG7gc3JmygkEdErWBO/CHzHgsx7biVE5b8sZYNEDKFojyoPHGWK2bQM/FTy
niJCgNCLdn6hUqqxLAml3cxGW77hAWu94THDGB1qFe+eFiAUnDmob8gNZtAzT6Ky
b/JGJdrEU0wj+Rd7wUb4kpLInNH/Jc+oz2ii2AjNbGOZXnRz7h7Kv3sO9vABByYe
LcCj3qnhejHMqVhbAT1MD6zQ2+YKBjE52MsQKU/xhUpu9KkUyLh0cxkh3zrFiKh4
Vuvtc+n7aeOv2jJmOl1dr0XLlSHBlmoKqH6dCTSbddQLmlK7dms8vE01AgMBAAGj
gb4wgbswDAYDVR0TAQH/BAIwADALBgNVHQ8EBAMCBsAwHQYDVR0OBBYEFMeUzGYV
bXwJNQVbY1+A8YXYZY8pMEIGA1UdIwQ7MDmAFJvEsUi7+D8vp8xcWvnEdVBGkpoW
oR6kHDAaMRgwFgYDVQQDDA9wb255dG93biBSU0EgQ0GCAXswOwYDVR0RBDQwMoIO
dGVzdHNlcnZlci5jb22CFXNlY29uZC50ZXN0c2VydmVyLmNvbYIJbG9jYWxob3N0
MA0GCSqGSIb3DQEBCwUAA4IBgQBsk5ivAaRAcNgjc7LEiWXFkMg703AqDDNx7kB1
RDgLalLvrjOfOp2jsDfST7N1tKLBSQ9bMw9X4Jve+j7XXRUthcwuoYTeeo+Cy0/T
1Q78ctoX74E2nB958zwmtRykGrgE/6JAJDwGcgpY9kBPycGxTlCN926uGxHsDwVs
98cL6ZXptMLTR6T2XP36dAJZuOICSqmCSbFR8knc/gjUO36rXTxhwci8iDbmEVaf
BHpgBXGU5+SQ+QM++v6bHGf4LNQC5NZ4e4xvGax8ioYu/BRsB/T3Lx+RlItz4zdU
XuxCNcm3nhQV2ZHquRdbSdoyIxV5kJXel4wCmOhWIq7A2OBKdu5fQzIAzzLi65EN
RPAKsKB4h7hGgvciZQ7dsMrlGw0DLdJ6UrFyiR5Io7dXYT/+JP91lP5xsl6Lhg9O
FgALt7GSYRm2cZdgi9pO9rRr83Br1VjQT1vHz6yoZMXSqc4A2zcN2a2ZVq//rHvc
FZygs8miAhWPzqnpmgTj1cPiU1M=
-----END CERTIFICATE-----
";

const TEST_KEY: &str = "
-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEAqVYYdfxTT9qr1np22UoIWq4v1E4cHncp35xxu4HNyZsoJBHR
K1gTvwh8x4LMe24lROW/LGWDRAyhaI8qDxxlitm0DPxU8p4iQoDQi3Z+oVKqsSwJ
pd3MRlu+4QFrveExwxgdahXvnhYgFJw5qG/IDWbQM0+ism/yRiXaxFNMI/kXe8FG
+JKSyJzR/yXPqM9ootgIzWxjmV50c+4eyr97DvbwAQcmHi3Ao96p4XoxzKlYWwE9
TA+s0NvmCgYxOdjLEClP8YVKbvSpFMi4dHMZId86xYioeFbr7XPp+2njr9oyZjpd
Xa9Fy5UhwZZqCqh+nQk0m3XUC5pSu3ZrPLxNNQIDAQABAoIBAFKtZJgGsK6md4vq
kyiYSufrcBLaaEQ/rkQtYCJKyC0NAlZKFLRy9oEpJbNLm4cQSkYPXn3Qunx5Jj2k
2MYz+SgIDy7f7KHgr52Ew020dzNQ52JFvBgt6NTZaqL1TKOS1fcJSSNIvouTBerK
NCSXHzfb4P+MfEVe/w1c4ilE+kH9SzdEo2jK/sRbzHIY8TX0JbmQ4SCLLayr22YG
usIxtIYcWt3MMP/G2luRnYzzBCje5MXdpAhlHLi4TB6x4h5PmBKYc57uOVNngKLd
YyrQKcszW4Nx5v0a4HG3A5EtUXNCco1+5asXOg2lYphQYVh2R+1wgu5WiDjDVu+6
EYgjFSkCgYEA0NBk6FDoxE/4L/4iJ4zIhu9BptN8Je/uS5c6wRejNC/VqQyw7SHb
hRFNrXPvq5Y+2bI/DxtdzZLKAMXOMjDjj0XEgfOIn2aveOo3uE7zf1i+njxwQhPu
uSYA9AlBZiKGr2PCYSDPnViHOspVJjxRuAgyWM1Qf+CTC0D95aj0oz8CgYEAz5n4
Cb3/WfUHxMJLljJ7PlVmlQpF5Hk3AOR9+vtqTtdxRjuxW6DH2uAHBDdC3OgppUN4
CFj55kzc2HUuiHtmPtx8mK6G+otT7Lww+nLSFL4PvZ6CYxqcio5MPnoYd+pCxrXY
JFo2W7e4FkBOxb5PF5So5plg+d0z/QiA7aFP1osCgYEAtgi1rwC5qkm8prn4tFm6
hkcVCIXc+IWNS0Bu693bXKdGr7RsmIynff1zpf4ntYGpEMaeymClCY0ppDrMYlzU
RBYiFNdlBvDRj6s/H+FTzHRk2DT/99rAhY9nzVY0OQFoQIXK8jlURGrkmI/CYy66
XqBmo5t4zcHM7kaeEBOWEKkCgYAYnO6VaRtPNQfYwhhoFFAcUc+5t+AVeHGW/4AY
M5qlAlIBu64JaQSI5KqwS0T4H+ZgG6Gti68FKPO+DhaYQ9kZdtam23pRVhd7J8y+
xMI3h1kiaBqZWVxZ6QkNFzizbui/2mtn0/JB6YQ/zxwHwcpqx0tHG8Qtm5ZAV7PB
eLCYhQKBgQDALJxU/6hMTdytEU5CLOBSMby45YD/RrfQrl2gl/vA0etPrto4RkVq
UrkDO/9W4mZORClN3knxEFSTlYi8YOboxdlynpFfhcs82wFChs+Ydp1eEsVHAqtu
T+uzn0sroycBiBfVB949LExnzGDFUkhG0i2c2InarQYLTsIyHCIDEA==
-----END RSA PRIVATE KEY-----
";
