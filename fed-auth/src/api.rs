use std::ops::Deref;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use poem_openapi::payload::{Json, PlainText};
use poem_openapi::{Enum, Object, OpenApi};
use sqlx::query;
use uuid::Uuid;

use crate::context::{CallbackUrl, CallbackUrlVersion, ValidatedAuthSession, ValidatedUser};
use crate::oidc::{ACCESS_TOKEN_VALID_FOR, CALLBACK_TOKEN_VALID_FOR};
use crate::{Context, ContextWrapper, jwt};

#[derive(Object, Clone)]
pub(crate) struct EmailLoginRequest {
    email: String,
    code: String,
    language: EmailLanguage,
}
#[derive(Clone, Copy, Debug, Enum)]
#[oai(rename_all = "lowercase")]
enum EmailLanguage {
    En,
    Sv,
}
#[derive(Object)]
struct EmailApproveRequest {
    token: Uuid,
    language: Option<String>,
}
#[derive(Object, Clone)]
struct TestLoginRequest {
    stil_id: String,
    code: String,
}

/// Completes a login where the provider did not supply a name (email, test). `LU` always does.
#[derive(Object, Clone)]
pub(crate) struct InfoCompletion {
    code: String,
    name: String,
}

#[derive(Object)]
struct ApiKeyRequest {
    key: Uuid,
    /// Should be something like `https://api.teknologappen.se/v0/user/auth-callback/v1`.
    #[oai(flatten)]
    server_callback: Option<CallbackUrl>,
    /// Defaults to `API user`.
    default_name: Option<String>,
    /// Defaults to `en`.
    default_language: Option<String>,
}
#[derive(Object)]
struct ApiKeyResponse {
    access_token: String,
    expires_in: u64,
}
#[derive(Object)]
struct PasswordAssignRequest {
    password: String,
}
#[derive(Object)]
struct PasswordLoginRequest {
    /// The user's id as minilith knows it (`email:...` or `lund-university:...`).
    user_id: String,
    password: String,
    /// Session code from the OIDC authorize request.
    code: String,
    language: Option<String>,
}
#[derive(Object)]
struct AdminApiKeyRequest {
    user_id: String,
    email: String,
}
#[derive(Object)]
struct AdminApiKeyResponse {
    key: Uuid,
}

fn bearer_token(headers: &poem::http::HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// The `client_id` (JWT audience) of an access token which the [`User`] extractor already
/// validated.
fn token_client_id(
    jwks: &jsonwebtoken::jwk::JwkSet,
    headers: &poem::http::HeaderMap,
) -> MinilithResult<String> {
    #[derive(serde::Deserialize)]
    struct Audience {
        aud: String,
    }

    let token = bearer_token(headers)
        .ok_or_else(|| MinilithEndpointError::unauthorized("missing access token", ""))?;
    let header = jsonwebtoken::decode_header(token)
        .wrap_err_unauthorized("access token has no valid header")?;
    let jwk = jwks
        .find(header.kid.as_deref().unwrap_or_default())
        .ok_or(())
        .wrap_err_unauthorized("access token has an unknown key ID")?;
    let key = jsonwebtoken::DecodingKey::from_jwk(jwk)
        .wrap_err_internal("internal error, jwk invalid")?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
    validation.validate_aud = false;
    jsonwebtoken::decode::<Audience>(token, &key, &validation)
        .wrap_err_unauthorized("invalid access token")
        .map(|data| data.claims.aud)
}

#[derive(Clone)]
pub(crate) struct MainRouter {
    pub context: ContextWrapper,
}
impl Deref for MainRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi]
impl MainRouter {
    /// Stores the display name for the session, then continues the login.
    ///
    /// # Errors
    ///
    /// `name`: must contain a first and surname and be at least 5 characters long.
    #[oai(path = "/personal-information", method = "post")]
    async fn info_completion(
        &self,
        body: Json<InfoCompletion>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<PlainText<String>> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != self.website_domain)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let sub = sqlx::query_scalar!(
            "select sub from session_validated_users where session_id = $1",
            body.code
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;

        // email logins are handled by the board and might not have a real name
        if !sub.starts_with("email:")
            && (body
                .name
                .split_once(' ')
                .is_none_or(|(first_name, surname)| first_name.is_empty() || surname.is_empty())
                || body.name.len() < 5)
        {
            return Err(MinilithEndpointError::bad_user_input(
                "name invalid",
                "",
                "name has to contain both first- and surname and be at least 5 characters long",
                "name",
            ));
        }

        sqlx::query!(
            "update session_validated_users set full_name = $1 where session_id = $2",
            body.name,
            body.code
        )
        .execute(&self.db)
        .await?;

        let validated_user = self
            .get_validated_session(&body.code)
            .await?
            .wrap_err_internal("we've just inserted it")?;

        let url = self
            .provider_callback_next_url(&body.code, &validated_user)
            .await
            .wrap_err_internal("noalert provider")?;
        Ok(PlainText(url))
    }
    /// Corresponds to the login happening at the `IdP` in `SAML2`.
    #[oai(path = "/providers/email/login", method = "post")]
    async fn email_login(
        &self,
        body: Json<EmailLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<()> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != self.website_domain)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        if !self.check_has_session(&body.code).await {
            return Err(MinilithEndpointError::unauthorized("code not valid", ""));
        }
        let token = Uuid::new_v4();
        let link = format!(
            "{}/providers/email/approve/?token={token}",
            self.website_domain
        );
        if let Some(email_client) = &self.email_client {
            let (from_name, subject, description) = match body.language {
                EmailLanguage::En => (
                    "Teknologappen login service",
                    "Log in to Teknologappen",
                    "Someone requested a login using this email address. If this was not you, \
                    you can ignore this email. Follow the link to log in.",
                ),
                EmailLanguage::Sv => (
                    "Teknologappens inloggningstjänst",
                    "Logga in på Teknologappen",
                    "Någon har försökt logga in med den här e-postadressen. Om det inte var du \
                    kan du ignorera det här mejlet. Följ länken för att logga in.",
                ),
            };
            let html = format!("<p>{description}</p><p><a href=\"{link}\">{link}</a></p>");
            email_client
                .send_html(from_name, [body.email.as_str()], subject, html)
                .await?;
        } else {
            println!(
                "Someone tried to log in with the email {}. Click the link below to continue.",
                body.email
            );
            println!("{link}");
        }

        sqlx::query!(
            "insert into email_token_holding (id, email, code)
            values ($1, $2, $3)",
            token,
            body.email,
            body.code
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/email/approve", method = "post")]
    async fn mail_approve(
        &self,
        Json(body): Json<EmailApproveRequest>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<PlainText<String>> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != self.website_domain)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let Some(login_data) = sqlx::query!(
            "select * from email_token_holding where id = $1",
            body.token,
        )
        .fetch_optional(&self.db)
        .await?
        else {
            return Err(MinilithEndpointError::unauthorized("token not valid", ""));
        };
        let Some(session) = self.get_session(&login_data.code).await? else {
            return Err(MinilithEndpointError::unauthorized("session not valid", ""));
        };
        let user = ValidatedUser {
            sub: format!("email:{}", login_data.email),
            full_name: None,
            email: Some(login_data.email),
            language: body.language,
        };
        self.validate_session(&login_data.code, &user).await?;

        Ok(PlainText(
            self.provider_callback_next_url(
                &login_data.code,
                &ValidatedAuthSession { session, user },
            )
            .await
            .wrap_err_internal("noalert provider")?,
        ))
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/test/approve", method = "post")]
    async fn test_approve(
        &self,
        body: Json<TestLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<PlainText<String>> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != self.website_domain)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let Some(session) = self.get_session(&body.code).await? else {
            return Err(MinilithEndpointError::unauthorized("code not valid", ""));
        };
        let sub = format!("test:{}", body.stil_id);
        let user = ValidatedUser {
            sub,
            full_name: None,
            email: None,
            language: None,
        };
        self.validate_session(&body.code, &user).await?;

        Ok(PlainText(
            self.provider_callback_next_url(&body.code, &ValidatedAuthSession { session, user })
                .await
                .wrap_err_internal("noalert provider")?,
        ))
    }
    #[oai(path = "/api-key-get-access-token", method = "post")]
    async fn get_at(
        &self,
        Json(body): Json<ApiKeyRequest>,
    ) -> MinilithResult<Json<ApiKeyResponse>> {
        let row = query!(
            "select user_id, client_id from api_keys where key = $1",
            body.key
        )
        .fetch_one(&self.db)
        .await
        .wrap_err_unauthorized("KEY")?;

        if let Some(cb_url) = body.server_callback {
            let token = jwt::encode(
                &jwt::StandardClaims::new(
                    &row.client_id,
                    CALLBACK_TOKEN_VALID_FOR,
                    &ValidatedUser {
                        sub: row.user_id.clone(),
                        email: None,
                        full_name: Some(body.default_name.unwrap_or_else(|| "API user".to_owned())),
                        language: Some(body.default_language.unwrap_or_else(|| "en".to_owned())),
                    },
                ),
                &self.private_key,
            )?;
            match cb_url.as_latest() {
                CallbackUrlVersion::V1 { .. } => {
                    let url = cb_url.url_for_request(self.debug);
                    let resp = self
                        .reqwest_client
                        .post(url.as_ref())
                        .body(token)
                        .send()
                        .await
                        .wrap_err_internal("auth callback POST transport failed")?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.ok();
                        return Err(MinilithEndpointError::internal_error(
                            "auth callback POST failed",
                            (status, body),
                        ));
                    }
                }
            }
        }

        let claims = jwt::StandardClaims::new(
            row.client_id,
            ACCESS_TOKEN_VALID_FOR,
            jwt::AccesTokenClaims { sub: row.user_id },
        );
        let access_token =
            jwt::encode(&claims, &self.private_key).wrap_err_internal("auth jwt encode at")?;
        Ok(Json(ApiKeyResponse {
            access_token,
            expires_in: ACCESS_TOKEN_VALID_FOR,
        }))
    }
    /// Stores an argon2id password for the bearer's user, scoped to one client.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/password/assign", method = "post")]
    async fn assign_password(
        &self,
        user: fed_auth_verifier::User,
        headers: &poem::http::HeaderMap,
        Json(body): Json<PasswordAssignRequest>,
    ) -> MinilithResult<()> {
        if !(8..=128).contains(&body.password.len()) {
            return Err(MinilithEndpointError::bad_user_input(
                "password invalid",
                "",
                "password has to be between 8 and 128 characters long",
                "password",
            ));
        }
        let client_id = token_client_id(&self.jwks, headers)?;
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(body.password.as_bytes(), &salt)
            .map_err(|err| MinilithEndpointError::internal_error("password hashing failed", err))?;
        let hash = hash.to_string();
        sqlx::query!(
            "insert into password_associations
                (user_id, client_id, password_salt, password_hash)
            values ($1, $2, $3, $4)
            on conflict (user_id, client_id) do update set
                password_salt = excluded.password_salt,
                password_hash = excluded.password_hash",
            user.get_id(),
            client_id,
            salt.as_str().as_bytes(),
            hash.as_bytes(),
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Logs in with a password for an existing OIDC session.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/providers/password/login", method = "post")]
    async fn password_login(
        &self,
        Json(body): Json<PasswordLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<PlainText<String>> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != self.website_domain)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let row = sqlx::query!(
            "select password_salt, password_hash
            from password_associations
            where user_id = $1
            and client_id = (select client_id from sessions where id = $2)",
            body.user_id,
            body.code,
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| MinilithEndpointError::unauthorized("invalid credentials", ""))?;
        let stored_hash = std::str::from_utf8(&row.password_hash)
            .ok()
            .and_then(|hash| PasswordHash::new(hash).ok())
            .ok_or_else(|| {
                MinilithEndpointError::internal_error("stored password hash is corrupt", "")
            })?;
        Argon2::default()
            .verify_password(body.password.as_bytes(), &stored_hash)
            .map_err(|_| MinilithEndpointError::unauthorized("invalid credentials", ""))?;

        let Some(session) = self.get_session(&body.code).await? else {
            return Err(MinilithEndpointError::unauthorized("code not valid", ""));
        };
        let user = ValidatedUser {
            sub: body.user_id,
            email: None,
            full_name: None,
            language: body.language,
        };
        self.validate_session(&body.code, &user).await?;
        Ok(PlainText(
            self.provider_callback_next_url(&body.code, &ValidatedAuthSession { session, user })
                .await
                .wrap_err_internal("noalert provider")?,
        ))
    }

    /// Creates an API key for an existing user. Requires `Authorization: Bearer <admin token>`;
    /// the key belongs to the admin token's `client_id`.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/admin/api-keys", method = "post")]
    async fn create_api_key(
        &self,
        headers: &poem::http::HeaderMap,
        Json(body): Json<AdminApiKeyRequest>,
    ) -> MinilithResult<Json<AdminApiKeyResponse>> {
        let token = bearer_token(headers)
            .ok_or_else(|| MinilithEndpointError::unauthorized("missing admin token", ""))?;
        let key = token
            .parse::<Uuid>()
            .map_err(|_| MinilithEndpointError::unauthorized("invalid admin token", ""))?;
        let token_client =
            sqlx::query_scalar!("select client_id from admin_tokens where key = $1", key)
                .fetch_optional(&self.db)
                .await?
                .ok_or_else(|| MinilithEndpointError::unauthorized("invalid admin token", ""))?;
        let api_key = sqlx::query_scalar!(
            "insert into api_keys (key, user_id, email, client_id)
            values (uuidv4(), $1, $2, $3) returning key",
            body.user_id,
            body.email,
            token_client,
        )
        .fetch_one(&self.db)
        .await?;
        Ok(Json(AdminApiKeyResponse { key: api_key }))
    }
}
