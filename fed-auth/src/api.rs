use std::ops::Deref;

use fed_auth_verifier::User;
use lettre::AsyncTransport as _;
use poem::web::cookie::CookieJar;
use poem_openapi::payload::{Binary, Json, PlainText, Response};
use poem_openapi::{ApiResponse, Object, OpenApi};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::{Context, DOMAIN, context, cookie, jwt, random_id};

const ALLOWED_DOMAINS: &[&str] = &[
    "https://teknologappen.se",
    "https://auth.esek.se",
    "https://fsektionen.se",
    "https://auth.dsek.se",
];

#[derive(Object)]
struct RefreshResponse {
    access_token: String,
}
#[derive(ApiResponse)]
enum RefreshError {
    /// No origin, the request must be CORS.
    #[oai(status = 400)]
    NoOrigin,
    /// Returns when the user either doesn't have a token or the token is invalid.
    #[oai(status = 401)]
    TokenInvalid,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Object)]
struct ConfirmRequest {
    accepted: bool,
    id: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum ConfirmError {
    /// Confirmation took too long.
    #[oai(status = 400)]
    CacheFlushed,
    /// Authentication is not valid!
    #[oai(status = 400)]
    AuthNotValid,
    /// This request must come from our own domain.
    #[oai(status = 400)]
    Cors,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
    /// Database error.
    #[oai(status = 500)]
    DbError,
    /// Callback post request failed. See server logs.
    #[oai(status = 503)]
    CallbackFailed,
}
#[derive(Object)]
pub struct ProviderRequest {
    continue_url: String,
    callback: Option<context::CallbackUrl>,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum ProviderError {
    /// The client must send origin, i.e. this must be a CORS request. It must also be UTF-8.
    #[oai(status = 400)]
    InvalidOrigin,
    /// Unknown internal server error in URL creation.
    /// See logs.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Object, Clone)]
pub(crate) struct EmailLoginRequest {
    email: String,
    name: String,
    id: String,
}
#[derive(Object)]
struct EmailApproveResponse {
    token: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum EmailLoginError {
    /// The client took too long.
    #[oai(status = 401)]
    Timeout,
    /// Invalid e-mail address.
    #[oai(status = 400)]
    InvalidEmail,
    /// Invalid name.
    #[oai(status = 400)]
    InvalidName,
    /// This request must come from our own domain.
    #[oai(status = 400)]
    Cors,
    /// Error sending email.
    #[oai(status = 500)]
    EmailError,
}
#[derive(Object, Clone)]
struct TestLoginRequest {
    stil_id: String,
    name: String,
    id: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum TestLoginError {
    /// This request must come from our own domain.
    #[oai(status = 400)]
    Cors,
    /// No such login ID.
    #[oai(status = 401)]
    InvalidId,
}

#[derive(Clone)]
pub(crate) struct MainRouter {
    pub context: Context,
}
impl Deref for MainRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi]
impl MainRouter {
    /// Returns the key as DER.
    #[oai(path = "/verify-key.der", method = "get")]
    async fn get_verify_key(&self) -> Response<Binary<Vec<u8>>> {
        Response::new(Binary(self.public_key.clone()))
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/refresh", method = "post")]
    async fn refresh(
        &self,
        headers: &poem::http::HeaderMap,
        cookies: &CookieJar,
    ) -> Result<Json<RefreshResponse>, RefreshError> {
        let origin = headers
            .get("origin")
            .and_then(|header| header.to_str().ok())
            .ok_or(RefreshError::NoOrigin)?;

        let Some(refresh_token) = cookies.get(cookie::REFRESH_TOKEN_COOKIE) else {
            return Err(RefreshError::TokenInvalid);
        };
        let Ok(refresh_token) = refresh_token.value_str().parse::<Uuid>() else {
            return Err(RefreshError::TokenInvalid);
        };

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
            refresh_token,
            origin,
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        sqlx::query!(
            "delete from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            refresh_token,
            origin,
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::Unknown)?;

        let new_refresh = Uuid::new_v4();
        sqlx::query!(
            "insert into auth_refresh_tokens values ($1, $2, $3)",
            get_query.user_id,
            get_query.domain,
            new_refresh
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::Unknown)?;

        conn.commit()
            .await
            .inspect_err(|err| {
                error!("failed to commit DB transaction: {err}");
            })
            .map_err(|_| RefreshError::Unknown)?;

        let claims = jwt::AccesTokenClaims {
            sub: get_query.user_id,
        };
        let access_token = jwt::encode(claims, &self.private_key).ok_or(RefreshError::Unknown)?;

        cookies.add(cookie::get(new_refresh));

        Ok(Json(RefreshResponse { access_token }))
    }
    /// Removes the refresh token.
    #[oai(path = "/logout", method = "post")]
    async fn logout(&self, cookies: &CookieJar, headers: &poem::http::HeaderMap) {
        let origin = headers
            .get("origin")
            .and_then(|header| header.to_str().ok());
        if let (Some(origin), Some(refresh_token)) = (
            origin,
            cookies
                .get(cookie::REFRESH_TOKEN_COOKIE)
                .and_then(|cookie| cookie.value_str().parse::<Uuid>().ok()),
        ) {
            // We don't care if the user was actually logged in!
            // So just try to remove it
            let _: Result<_, _> = sqlx::query!(
                "delete from auth_refresh_tokens where refresh_token = $1 and domain = $2",
                refresh_token,
                origin,
            )
            .execute(&self.db)
            .await;
        }
        cookies.add(cookie::remove());
    }
    /// Verifies that your access token is correct.
    #[oai(path = "/verify-access-token", method = "post")]
    async fn verify_access_token(&self, _user: User) {}
    #[oai(path = "/confirm-datasharing", method = "post")]
    async fn confirm_datasharing(
        &self,
        body: Json<ConfirmRequest>,
        headers: &poem::http::HeaderMap,
        cookies: &CookieJar,
    ) -> Result<PlainText<String>, ConfirmError> {
        if headers.get("origin").is_some_and(|origin| origin != DOMAIN) {
            return Err(ConfirmError::Cors);
        }
        let Some(data) = self.auth_sessions.get(&body.id) else {
            warn!(
                "Tried to confirm datasharing with an ID which is not in the database ({})",
                body.id
            );
            return Err(ConfirmError::CacheFlushed);
        };
        if !ALLOWED_DOMAINS.contains(&data.origin.as_str()) {
            warn!(
                "Someone tried to log in from a disallowed domain ({})!",
                data.origin
            );
            return Ok(PlainText(format!(
                "{}{}validated=false",
                data.continue_url,
                if data.continue_url.contains('?') {
                    '&'
                } else {
                    '?'
                },
            )));
        }
        let Some(user_data) = data.validated_user else {
            warn!(
                "Tried to confirm datasharing for a request which was not validated ({})",
                body.id
            );
            return Err(ConfirmError::AuthNotValid);
        };

        if body.accepted {
            if let Some(cb_url) = &data.callback {
                let token =
                    jwt::encode(&user_data, &self.private_key).ok_or(ConfirmError::Unknown)?;
                match cb_url.as_latest() {
                    context::CallbackUrlVersion::V1 { url } => {
                        self.reqwest_client
                            .post(url)
                            .body(token)
                            .send()
                            .await
                            .inspect_err(|err| error!("auth callback POST failed: {err}"))
                            .map_err(|_| ConfirmError::CallbackFailed)?;
                    }
                }
            }

            let refresh_token = Uuid::new_v4();
            sqlx::query!(
                "insert into auth_refresh_tokens values ($1, $2, $3)",
                user_data.sub,
                data.origin,
                refresh_token
            )
            .execute(&self.db)
            .await
            .inspect_err(|err| error!("Error inserting refresh token into DB: {err}"))
            .map_err(|_| ConfirmError::DbError)?;
            cookies.add(cookie::get(refresh_token));
        }

        self.auth_sessions.invalidate(&body.id);
        Ok(PlainText(format!(
            "{}{}validated={}",
            data.continue_url,
            if data.continue_url.contains('?') {
                '&'
            } else {
                '?'
            },
            body.accepted
        )))
    }

    #[allow(clippy::unused_self, reason = "makes the developer experience nicer")]
    fn check_redirect_provider<'a>(
        &self,
        headers: &'a poem::http::HeaderMap,
        body: &Json<ProviderRequest>,
    ) -> Result<&'a str, ProviderError> {
        let origin = headers
            .get("origin")
            .and_then(|header| header.to_str().ok())
            .ok_or(ProviderError::InvalidOrigin)?;
        if let Some(cb_url) = &body.callback {
            let cb_url: poem::http::Uri = cb_url
                .as_latest()
                .url()
                .parse()
                .map_err(|_| ProviderError::InvalidOrigin)?;
            if Some(origin) != cb_url.host() {
                return Err(ProviderError::InvalidOrigin);
            }
        }
        Ok(origin)
    }
    fn redirect_provider(
        &self,
        body: &Json<ProviderRequest>,
        id: &str,
        origin: &str,
        url: String,
    ) -> PlainText<String> {
        let data = context::AuthSession {
            origin: origin.to_owned(),
            callback: body.callback.clone(),
            continue_url: body.continue_url.clone(),

            validated_user: None,
        };
        self.auth_sessions.insert(id.to_owned(), data);

        PlainText(url)
    }
    /// Get URL to redirect user to to authenticate by LU SSO
    #[oai(path = "/providers/lu", method = "post")]
    async fn lu(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<ProviderRequest>,
    ) -> Result<PlainText<String>, ProviderError> {
        let origin = self.check_redirect_provider(headers, &body)?;

        let req = self
            .service_provider
            // .make_authentication_request("https://testidpv4.lu.se/idp/profile/SAML2/Redirect/SSO")
            .make_authentication_request("https://mocksaml.com/api/saml/sso")
            .inspect_err(|err| error!("Failed to make LU SSO request {err}"))
            .map_err(|_| ProviderError::Unknown)?;
        let redirect = req
            .signed_redirect("", &self.saml_private_key)
            .inspect_err(|err| error!("Failed to make LU SSO redirect {err}"))
            .map_err(|_| ProviderError::Unknown)?
            .ok_or_else(|| {
                error!("Failed to create LU SSO link");
                ProviderError::Unknown
            })?;
        self.saml2_request_id_cache.insert(req.id.clone(), ());
        debug!("Added ID {} to auth request id cache", req.id);

        Ok(self.redirect_provider(&body, &req.id, origin, redirect.to_string()))
    }
    #[oai(path = "/providers/email", method = "post")]
    async fn email(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<ProviderRequest>,
    ) -> Result<PlainText<String>, ProviderError> {
        let origin = self.check_redirect_provider(headers, &body)?;
        let id = random_id();
        let redirect = format!("{DOMAIN}/providers/email/?id={id}");

        Ok(self.redirect_provider(&body, &id, origin, redirect))
    }
    #[oai(path = "/providers/test", method = "post")]
    async fn test_provider(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<ProviderRequest>,
    ) -> Result<PlainText<String>, ProviderError> {
        let origin = self.check_redirect_provider(headers, &body)?;
        let id = random_id();
        let redirect = format!("{DOMAIN}/providers/test/?id={id}");

        Ok(self.redirect_provider(&body, &id, origin, redirect))
    }
    /// Corresponds to the login happening at the `IdP` in `SAML2`.
    #[oai(path = "/providers/email/login", method = "post")]
    async fn email_login(
        &self,
        body: Json<EmailLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> Result<(), EmailLoginError> {
        if headers.get("origin").is_some_and(|origin| origin != DOMAIN) {
            return Err(EmailLoginError::Cors);
        }
        if !self.auth_sessions.contains_key(&body.id) {
            return Err(EmailLoginError::Timeout);
        }
        if !body.name.contains(' ') || body.name.len() < 5 {
            return Err(EmailLoginError::InvalidName);
        }
        let token = random_id();
        // having this as format_args made the await point for lettre fail because format_args is
        // not Send??
        let link = format!("{DOMAIN}/providers/email/approve/?token={token}");
        if let Some((from, email)) = &self.email {
            let html = format!(
                "<p>Någon har försökt logga in med denna e-post adress. Om detta inte var du bör du slänga detta mailet. Tryck på länken för att logga in.</p><p><a href='{link}'>{link}</a>"
            );
            let message = lettre::Message::builder()
                .from(lettre::message::Mailbox::new(
                    Some("Teknologappens inloggningstjänst".to_owned()),
                    from.clone(),
                ))
                .to(body
                    .email
                    .parse::<lettre::Address>()
                    .map_err(|_| EmailLoginError::InvalidEmail)?
                    .into())
                .subject("Logga in med teknologappens inloggningstjänst")
                .header(lettre::message::header::ContentType::TEXT_HTML)
                .body(html)
                .inspect_err(|err| error!("Error when formatting a mail: {err}"))
                .map_err(|_| EmailLoginError::EmailError)?;
            email
                .send(message)
                .await
                .inspect_err(|err| error!("failed to send email: {err}"))
                .map_err(|_| EmailLoginError::EmailError)?;
        } else {
            println!(
                "Someone tried to log in with the email {}. Click the link below to continue.",
                body.email
            );
            println!("{link}");
        }

        self.email_token_holding.insert(token, (*body).clone());

        Ok(())
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/email/approve", method = "post")]
    async fn mail_approve(
        &self,
        body: Json<EmailApproveResponse>,
        headers: &poem::http::HeaderMap,
    ) -> Result<PlainText<String>, EmailLoginError> {
        if headers.get("origin").is_some_and(|origin| origin != DOMAIN) {
            return Err(EmailLoginError::Cors);
        }
        let Some(login_data) = self.email_token_holding.get(&body.token) else {
            return Err(EmailLoginError::Timeout);
        };
        let Some(mut data) = self.auth_sessions.get(&login_data.id) else {
            return Err(EmailLoginError::Timeout);
        };
        data.validated_user = Some(context::UserData {
            sub: format!("mail:{}", login_data.email),
            full_name: login_data.name,
            email: login_data.email,
        });
        self.auth_sessions
            .insert(login_data.id.clone(), data.clone());

        Ok(PlainText(format!(
            "/confirm-datasharing/?id={}&origin={}",
            login_data.id, data.origin
        )))
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/test/approve", method = "post")]
    async fn test_approve(
        &self,
        body: Json<TestLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> Result<PlainText<String>, TestLoginError> {
        if headers.get("origin").is_some_and(|origin| origin != DOMAIN) {
            return Err(TestLoginError::Cors);
        }
        let Some(mut data) = self.auth_sessions.get(&body.id) else {
            return Err(TestLoginError::InvalidId);
        };
        data.validated_user = Some(context::UserData {
            sub: format!("test:{}", body.stil_id),
            full_name: body.name.clone(),
            email: format!("{}@student.lu.se", body.stil_id),
        });
        self.auth_sessions.insert(body.id.clone(), data.clone());

        Ok(PlainText(format!(
            "/confirm-datasharing/?id={}&origin={}",
            body.id, data.origin
        )))
    }
}
