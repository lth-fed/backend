use color_eyre::eyre::Context as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;
use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
use sqlx::Postgres;
use sqlx::migrate::{MigrateDatabase as _, Migrator};
use sqlx::postgres::PgPoolOptions;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

pub type PgPool = sqlx_tracing::Pool<Postgres>;

/// # Errors
///
/// Fails if DB communication or migration fails.
pub async fn setup_db(db_url: &str, migrator: Option<Migrator>) -> color_eyre::Result<PgPool> {
    if !Postgres::database_exists(db_url)
        .await
        .wrap_err("Failed to check if database exists")?
    {
        Postgres::create_database(db_url).await?;
    }

    let db = PgPoolOptions::new()
        .max_connections(50)
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
pub fn get_otel(service_name: &'static str) -> color_eyre::Result<opentelemetry_sdk::trace::SdkTracerProvider> {
    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name)
        .with_attributes([opentelemetry::KeyValue::new(
            "deployment.environment.name",
            #[cfg(debug_assertions)]
            "dev",
            #[cfg(not(debug_assertions))]
            "production",
        )])
        .build();

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
    let otel_span_appender = tracing_opentelemetry::layer().with_tracer(tracer.tracer(service_name));
    let _: Result<(), _> = tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer)
        .with(otel_span_appender)
        .with(tracing_subscriber::fmt::layer())
        // For tests; this may be run several times.
        .try_init();

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
