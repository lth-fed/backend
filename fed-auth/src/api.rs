use std::ops::Deref;

use lettre::AsyncTransport as _;
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::payload::{Json, PlainText};
use poem_openapi::{Object, OpenApi};
use sqlx::query;
use uuid::Uuid;

use crate::oidc::ACCESS_TOKEN_VALID_FOR;
use crate::{Context, WEBSITE_DOMAIN, context, jwt, random_id};

#[derive(Object, Clone)]
pub(crate) struct EmailLoginRequest {
    email: String,
    name: String,
    code: String,
}
#[derive(Object)]
struct EmailApproveResponse {
    token: String,
}
#[derive(Object, Clone)]
struct TestLoginRequest {
    stil_id: String,
    name: String,
    code: String,
}

#[derive(Object)]
struct ApiKeyRequest {
    key: Uuid,
}
#[derive(Object)]
struct ApiKeyResponse {
    access_token: String,
    expires_in: u64,
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
    /// Corresponds to the login happening at the `IdP` in `SAML2`.
    #[oai(path = "/providers/email/login", method = "post")]
    async fn email_login(
        &self,
        body: Json<EmailLoginRequest>,
        headers: &poem::http::HeaderMap,
    ) -> MinilithResult<()> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        if !self.auth_sessions.contains_key(&body.code) {
            return Err(MinilithEndpointError::unauthorized("code not valid", ""));
        }
        if !body.name.contains(' ') || body.name.len() < 5 {
            return Err(MinilithEndpointError::bad_user_input(
                "name invalid",
                "",
                "name has to contain both first- and surname and be at least 5 characters long",
                "name",
            ));
        }
        let token = random_id();
        // having this as format_args made the await point for lettre fail because format_args is
        // not Send??
        let link = format!("{WEBSITE_DOMAIN}/providers/email/approve/?token={token}");
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
                    .wrap_err_bad_user("please enter a valid email address", "email")?
                    .into())
                .subject("Logga in med teknologappens inloggningstjänst")
                .header(lettre::message::header::ContentType::TEXT_HTML)
                .body(html)
                .map_err(|err| {
                    MinilithEndpointError::internal_error(format!("format email: {err:?}"))
                })?;
            email.send(message).await.map_err(|err| {
                MinilithEndpointError::internal_error(format!("failed to send mail: {err:?}"))
            })?;
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
    ) -> MinilithResult<PlainText<String>> {
        if headers
            .get("origin")
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let Some(login_data) = self.email_token_holding.get(&body.token) else {
            return Err(MinilithEndpointError::unauthorized("token not valid", ""));
        };
        let Some(mut session) = self.auth_sessions.get(&login_data.code) else {
            return Err(MinilithEndpointError::unauthorized("session not valid", ""));
        };
        session.validated_user = Some(context::UserData {
            sub: format!("mail:{}", login_data.email),
            full_name: login_data.name,
            email: login_data.email,
        });
        self.auth_sessions
            .insert(login_data.code.clone(), session.clone());

        Ok(PlainText(
            session.provider_callback_next_url(&login_data.code),
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
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "origin has to be from our domain",
                "",
            ));
        }
        let Some(mut session) = self.auth_sessions.get(&body.code) else {
            return Err(MinilithEndpointError::unauthorized("code not valid", ""));
        };
        session.validated_user = Some(context::UserData {
            sub: format!("test:{}", body.stil_id),
            full_name: body.name.clone(),
            email: format!("{}@student.lu.se", body.stil_id),
        });
        self.auth_sessions
            .insert(body.code.clone(), session.clone());

        Ok(PlainText(session.provider_callback_next_url(&body.code)))
    }
    #[oai(path = "/api-key-get-access-token", method = "post")]
    async fn get_at(&self, body: Json<ApiKeyRequest>) -> MinilithResult<Json<ApiKeyResponse>> {
        let user_id = query!("select user_id from api_keys where key = $1", body.key)
            .fetch_one(&self.db)
            .await
            .wrap_err_unauthorized("KEY")?;
        let now = jsonwebtoken::get_current_timestamp();
        let claims = jwt::AccesTokenClaims {
            sub: user_id.user_id,
            aud: "teknologappen".to_owned(),
            exp: now + ACCESS_TOKEN_VALID_FOR,
            iat: now,
            nbf: now,
        };
        let access_token = jwt::encode(claims, &self.private_key)
            .ok_or_else(|| MinilithEndpointError::internal_error("contact app developers"))?;
        Ok(Json(ApiKeyResponse {
            access_token,
            expires_in: ACCESS_TOKEN_VALID_FOR,
        }))
    }
}
