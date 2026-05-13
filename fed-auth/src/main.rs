#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
#![allow(
    missing_debug_implementations,
    reason = "we can't add debug to e.g. Context"
)]
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::Deref;
use std::path::PathBuf;

use mini_moka::sync::Cache;
use poem::http::Method;
use poem::middleware::Cors;
use poem::{EndpointExt as _, handler};
use samael::metadata::{
    AttributeConsumingService, ContactPerson, ContactType, EntityDescriptor, LocalizedName,
    LocalizedUri, RequestedAttribute,
};
use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};

use base64::Engine as _;
use color_eyre::{Section as _, eyre::Context as _};
use ed25519_dalek::pkcs8::{DecodePrivateKey as _, EncodePublicKey as _};
use jsonwebtoken::EncodingKey;
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::payload::{Binary, Form, Json, PlainText, Response};
use poem_openapi::{ApiResponse, Object, OpenApi, OpenApiService};
use samael::traits::ToXml as _;
use serde::Serialize;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing::{debug, error};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use uuid::Uuid;
use xmltree::XMLNode;

const PRIVACY_POLICY: &str = include_str!("./privacy-policy.html");

#[handler]
fn pp_file() -> poem::web::Html<&'static str> {
    poem::web::Html(PRIVACY_POLICY)
}

#[derive(Clone)]
pub struct Context {
    pub db: PgPool,
    pub private_key: EncodingKey,
    pub public_key: Vec<u8>,
    pub service_provider: ServiceProvider,
    pub saml_private_key: openssl::pkey::PKey<openssl::pkey::Private>,
    pub auth_request_id_cache: Cache<String, ()>,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
    let key = base64::prelude::BASE64_STANDARD
        .decode(key)
        .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
    let ed_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)
        .wrap_err("`PRIVATE_KEY` not valid EdDSA key")?;
    let verifying_key = ed_key.verifying_key();
    let encoding_key = EncodingKey::from_ed_der(&key);

    let db = setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
        .await
        .wrap_err("Failed to set up the database")
        .suggestion("Start the database with `docker compose up -d`")?;

    let resp = reqwest::get("https://testidpv4.lu.se/idp/shibboleth")
        .await?
        .text()
        .await?;
    let idp_metadata: EntityDescriptor = samael::metadata::de::from_str(&resp)?;

    let saml_pk = std::env::var("SAML_PRIVATE_KEY").wrap_err("`SAML_PRIVATE_KEY` not detected")?;
    let saml_pk = base64::prelude::BASE64_STANDARD
        .decode(saml_pk)
        .wrap_err("`SAML_PRIVATE_KEY` not base64 encoded")?;
    let saml_pk = openssl::pkey::PKey::from_rsa(openssl::rsa::Rsa::private_key_from_pem(&saml_pk)?)
        .wrap_err("`SAML_PRIVATE_KEY` not valid base64 encoded PEM private key")?;
    let saml_cert = std::env::var("SAML_CERTIFICATE").wrap_err("`SAML_PUBLIC_KEY` not detected")?;
    let saml_cert = base64::prelude::BASE64_STANDARD
        .decode(saml_cert)
        .wrap_err("`SAML_CERTIFICATE` not base64 encoded")?;
    let saml_cert = openssl::x509::X509::from_pem(&saml_cert)?;
    let saml_cert = saml_cert.to_der()?;
    let saml_cert = samael::crypto::CertificateDer::from(saml_cert);

    let sp = ServiceProviderBuilder::default()
        .entity_id("teknologappen".to_owned())
        .key(saml_pk.clone())
        .certificate(saml_cert)
        .allow_idp_initiated(false)
        .force_authn(true)
        .contact_person(ContactPerson {
            contact_type: Some(ContactType::Technical.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        })
        .idp_metadata(idp_metadata)
        .acs_url("https://auth.teknologappen.se/saml2/acs".to_owned())
        // doesn't actually exist but is required by samael to exist
        .slo_url("https://auth.teknologappen.se/saml2/slo".to_owned())
        .build()?;

    let context = Context {
        db,
        private_key: encoding_key,
        public_key: verifying_key
            .to_public_key_der()
            .wrap_err("internal error: failed to encode verifying key to DER")?
            .into_vec(),
        service_provider: sp,
        saml_private_key: saml_pk,
        // keep them for 30 minutes
        auth_request_id_cache: Cache::builder()
            .time_to_live(std::time::Duration::from_mins(30))
            .build(),
    };
    #[cfg(debug_assertions)]
    let server_url = "http://localhost:8001";
    #[cfg(not(debug_assertions))]
    let server_url = "https://auth.teknologappen.se";
    let api_service = OpenApiService::new(
        (
            MainRouter {
                context: context.clone(),
            },
            SamlRouter { context },
        ),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(server_url);
    let ui = api_service.swagger_ui();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        // .allow_origin("mocksaml.com")
        .allow_credentials(false);

    Server::new(TcpListener::bind("[::]:8001"))
        .run(
            Route::new()
                .nest("/", api_service)
                .nest("/docs", ui)
                .nest("/privacy-statement/", pp_file)
                .with(cors),
        )
        .await?;

    Ok(())
}

async fn setup_db(db_url: &str) -> color_eyre::Result<PgPool> {
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

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .wrap_err("Failed to run migrations")?;

    Ok(db)
}

#[derive(Object)]
struct Refresh {
    /// The refresh token.
    refresh_token: Uuid,
    /// The domain for which this token is for.
    domain: String,
}
#[derive(Object)]
struct RefreshResponse {
    refresh_token: Uuid,
    access_token: String,
}
#[derive(ApiResponse)]
enum RefreshError {
    /// Returns when the user either doesn't have a token or the token is invalid.
    #[oai(status = 401)]
    TokenInvalid,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Serialize)]
struct Claims {
    sub: String,
}

#[derive(Clone)]
pub struct MainRouter {
    pub context: Context,
}
impl Deref for MainRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi(prefix_path = "/api/v0")]
impl MainRouter {
    /// Returns the key as DER.
    #[oai(path = "/verify-key.der", method = "get")]
    async fn get_verify_key(&self) -> Response<Binary<Vec<u8>>> {
        Response::new(Binary(self.public_key.clone()))
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/refresh", method = "post")]
    async fn refresh(&self, body: Json<Refresh>) -> Result<Json<RefreshResponse>, RefreshError> {
        let mut conn = self
            .db
            .begin()
            .await
            .inspect_err(|err| {
                error!("failed to open DB transaction: {err}");
            })
            .map_err(|_| RefreshError::Unknown)?;
        let get_query = sqlx::query!(
            "select * from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            body.0.refresh_token,
            body.0.domain
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        sqlx::query!(
            "delete from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            body.0.refresh_token,
            body.0.domain
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        let new_refresh = Uuid::new_v4();
        sqlx::query!(
            "insert into auth_refresh_tokens values ($1, $2, $3)",
            get_query.user_id,
            get_query.domain,
            new_refresh
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        conn.commit()
            .await
            .inspect_err(|err| {
                error!("failed to commit DB transaction: {err}");
            })
            .map_err(|_| RefreshError::Unknown)?;

        let claims = Claims {
            sub: get_query.user_id,
        };
        let access_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
            &claims,
            &self.private_key,
        )
        .inspect_err(|err| {
            error!("failed to encode JWT: {err}");
        })
        .map_err(|_| RefreshError::Unknown)?;

        Ok(Json(RefreshResponse {
            refresh_token: new_refresh,
            access_token,
        }))
    }
}
#[derive(ApiResponse, Debug, Clone, Copy)]
pub enum MetadataResponseError {
    /// Unable to produce correct metadata, invalid configuration.
    #[oai(status = 500)]
    MetadataInvalid,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
pub enum AcsResponseError {
    /// No `SAMLResponse` prop.
    #[oai(status = 400)]
    NoSamlResponse,
    /// Invalid ACS response.
    #[oai(status = 400)]
    InvalidAcsResponse,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
pub enum RedirectError {
    /// Unknown internal server error in URL creation.
    /// See logs.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Clone)]
pub struct SamlRouter {
    pub context: Context,
}
impl Deref for SamlRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
fn add_metadata(metadata: &mut EntityDescriptor) -> Result<(), MetadataResponseError> {
    let org_name = vec![
        LocalizedName {
            lang: Some("se".into()),
            value: "Utvecklarna bakom Teknologappen".into(),
        },
        LocalizedName {
            lang: Some("en".into()),
            value: "The developers behind Teknologappen".into(),
        },
    ];
    metadata.organization = Some(samael::metadata::Organization {
        organization_names: Some(org_name.clone()),
        organization_display_names: Some(org_name),
        organization_urls: Some(vec![
            LocalizedUri {
                lang: Some("se".into()),
                value: "https://teknologappen.se".into(),
            },
            LocalizedUri {
                lang: Some("en".into()),
                value: "https://teknologappen.se".into(),
            },
        ]),
    });
    let Some(sp_desc) = metadata
        .sp_sso_descriptors
        .as_mut()
        .and_then(|descs| descs.first_mut())
    else {
        error!("Failed to get sp sso descriptor");
        return Err(MetadataResponseError::MetadataInvalid);
    };
    let attribute_name_format = "urn:oasis:names:tc:SAML:2.0:attrname-format:uri";
    sp_desc.attribute_consuming_services = Some(vec![AttributeConsumingService {
        index: 1,
        is_default: None,
        service_names: vec![
            LocalizedName {
                lang: Some("sv".into()),
                value: "Teknologappens inloggningstjänst".into(),
            },
            LocalizedName {
                lang: Some("en".into()),
                value: "The login service for teknologappen".into(),
            },
        ],
        service_descriptions: None,
        request_attributes: vec![
            RequestedAttribute {
                friendly_name: Some("samlSubjectID".into()),
                name: "urn:oasis:names:tc:SAML:attribute:subject-id".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("mail".into()),
                name: "urn:oid:0.9.2342.19200300.100.1.3".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("displayName".into()),
                name: "urn:oid:2.16.840.1.113730.3.1.241".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
        ],
    }]);
    // sp_desc.name_id_formats = Some(vec!["urn:oasis:names:tc:SAML:2.0:nameid-format:transient".into()]);
    Ok(())
}
fn add_metadata_extensions(meta: &mut xmltree::Element) -> Result<usize, MetadataResponseError> {
    // the xmlns are needed for parsing, they are removed later. Copied from an example SP
    // metadata: https://metadata.qa.swamid.se/?rawXML=1361
    let descriptor_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdattr:EntityAttributes>
        <samla:Attribute Name="http://macedir.org/entity-category" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:uri">
            <samla:AttributeValue>https://refeds.org/category/personalized</samla:AttributeValue>
        </samla:Attribute>
    </mdattr:EntityAttributes>
        <mdrpi:RegistrationInfo registrationAuthority="http://www.swamid.se/" registrationInstant="2026-05-13T11:22:11Z">
        <mdrpi:RegistrationPolicy xml:lang="en">http://swamid.se/policy/mdrps</mdrpi:RegistrationPolicy>
    </mdrpi:RegistrationInfo>
</md:Extensions>"#;
    let spsso_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdui:UIInfo>
        <mdui:DisplayName xml:lang="en">Teknologappen and guild logins for members of TLTH</mdui:DisplayName>
        <mdui:DisplayName xml:lang="sv">Teknologappen och sektionsinlogg för medlemmar i TLTH</mdui:DisplayName>
        <mdui:Description xml:lang="sv">Teknologappenlogin, utvecklat av E, D, och F-sektionen</mdui:Description>
        <mdui:Description xml:lang="en">Teknologappen login, developed by the E, D, and F guilds</mdui:Description>
        <mdui:InformationURL xml:lang="sv">https://teknologappen.se</mdui:InformationURL>
        <mdui:InformationURL xml:lang="en">https://teknologappen.se</mdui:InformationURL>
        <mdui:PrivacyStatementURL xml:lang="en">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
        <mdui:PrivacyStatementURL xml:lang="sv">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
    </mdui:UIInfo>
</md:Extensions>"#;
    let mut desc_meta = xmltree::Element::parse(Cursor::new(descriptor_extensions))
        .inspect_err(|err| error!("Failed to parse metadata: {err}"))
        .map_err(|_| MetadataResponseError::MetadataInvalid)?;
    let mut spsso_meta = xmltree::Element::parse(Cursor::new(spsso_extensions))
        .inspect_err(|err| error!("Failed to parse metadata: {err}"))
        .map_err(|_| MetadataResponseError::MetadataInvalid)?;
    meta.namespaces = desc_meta.namespaces.take();
    spsso_meta.namespaces = None;

    meta.children.push(XMLNode::Element(desc_meta));

    let Some(spsso) = meta.get_mut_child("SPSSODescriptor") else {
        error!("Metadata is not an object!");
        return Err(MetadataResponseError::MetadataInvalid);
    };
    spsso.children.push(XMLNode::Element(spsso_meta));

    Ok(descriptor_extensions.len() + spsso_extensions.len())
}
#[OpenApi(prefix_path = "/saml2")]
impl SamlRouter {
    /// Returns the SAML2 metadata.
    ///
    /// The body actually is `application/xml` but since [`poem-openapi`] is cringe I can't just
    /// add a string as an XML response.
    #[oai(path = "/metadata", method = "get")]
    async fn metadata(&self) -> Result<Response<Binary<Vec<u8>>>, MetadataResponseError> {
        let mut metadata = self
            .service_provider
            .metadata()
            .inspect_err(|err| error!("Failed to get metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;
        add_metadata(&mut metadata)?;

        let metadata = metadata
            .to_string()
            .inspect_err(|err| error!("Failed to convert metadata to string: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;

        let mut meta = xmltree::Element::parse(Cursor::new(&metadata))
            .inspect_err(|err| error!("Failed to parse metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;

        let exts_len = add_metadata_extensions(&mut meta)?;

        let mut metadata = Cursor::new(Vec::with_capacity(metadata.len() + exts_len + 100));
        meta.write(&mut metadata)
            .inspect_err(|err| error!("Failed to serialize updated metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;
        let metadata = metadata.into_inner();
        Ok(
            Response::new(Binary(metadata))
                .header("content-type", "application/xml; charset=utf-8"),
        )
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/acs", method = "post")]
    #[allow(clippy::panic, reason = "yes")]
    async fn acs(&self, body: Form<HashMap<String, String>>) -> Result<(), AcsResponseError> {
        // we'd want the library to take an iterator instead of &[&str]
        let ids: Vec<_> = self
            .auth_request_id_cache
            .iter()
            .map(|entry| entry.key().to_owned())
            .collect();
        let ids: Vec<_> = ids.iter().map(String::as_str).collect();

        let saml_response = body
            .get("SAMLResponse")
            .ok_or(AcsResponseError::NoSamlResponse)?;
        let ass = self
            .service_provider
            .parse_base64_response(saml_response, Some(&ids))
            .inspect_err(|err| error!("Invalid ACS response: {err}"))
            .map_err(|_| AcsResponseError::InvalidAcsResponse)?;
        // stil-id
        // ass.subject.unwrap().name_id.unwrap().value;
        //
        // - tappen hemsidan: användare vill logga in med auth
        // - skickar till lu_redirect med body av continue url & callback (put that & save origin header in relay state)
        //   - origin & callback host must match
        // - login sker
        // - auth får tillbaka token & relay state
        // - auth sparar (token, origin, continue, callback) ett tag med ett ID
        // - visar en sida för användaren om hur den vill dela sina uppgifter (redirect från post sidan med ?id=...)
        // - om nej, redirect back / postMessage, no ID
        // - om ja, redirect back / postMessage, query params ID, make request set http only cookie & callback to server
        //
        // - komponenter:
        // - auth hemsida: loginsätt
        // - auth hemsida: godkänna
        // - lu_redirect auth spara (token, origin-url, continue url) & redirect till godkänna sida
        // - ny endpoint! set cookie: id -> set-cookie, remove ID from DB
        // - tappen client lib (refresh tokens in requests (middleware), start this whole process
        //   (lu_redirect with redirect or iframe), handle ID (both query & postMessage))
        //
        // todo:
        // - mail about testing towards swam id
        println!("{ass:#?}");
        Ok(())
    }
    /// Get URL to redirect user to to authenticate by LU SSO
    #[oai(path = "/lu-redirect", method = "post")]
    async fn lu_redirect(&self) -> Result<PlainText<String>, RedirectError> {
        let req = self
            .service_provider
            .make_authentication_request("https://testidpv4.lu.se/idp/profile/SAML2/Redirect/SSO")
            .inspect_err(|err| error!("Failed to make LU SSO request {err}"))
            .map_err(|_| RedirectError::Unknown)?;
        self.auth_request_id_cache.insert(req.id.clone(), ());
        debug!("Added ID {} to auth request id cache", req.id);
        let redirect = req
            .signed_redirect("", &self.saml_private_key)
            .inspect_err(|err| error!("Failed to make LU SSO redirect {err}"))
            .map_err(|_| RedirectError::Unknown)?
            .ok_or_else(|| {
                error!("Failed to create LU SSO link");
                RedirectError::Unknown
            })?;
        Ok(PlainText(redirect.to_string()))
    }
}
