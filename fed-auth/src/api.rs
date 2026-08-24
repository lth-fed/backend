use std::ops::Deref;

use fed_auth_verifier::callbacks::Guild;
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use poem_openapi::payload::{Json, PlainText};
use poem_openapi::{Enum, Object, OpenApi};
use sqlx::query;
use uuid::Uuid;

use crate::context::{ValidatedAuthSession, ValidatedUser};
use crate::oidc::ACCESS_TOKEN_VALID_FOR;
use crate::{Context, ContextWrapper, WEBSITE_DOMAIN, jwt};

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
struct EmailApproveResponse {
    token: Uuid,
}
#[derive(Object, Clone)]
struct TestLoginRequest {
    stil_id: String,
    code: String,
}

#[derive(Object, Clone)]
pub(crate) struct InfoCompletion {
    code: String,
    name: String,
    /// MUST be provided if the sub starts with `lund-university:`.
    personal_number: Option<String>,
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
    async fn get_guild(&self, pn: &str, sub: &str) -> MinilithResult<Option<Guild>> {
        let resp = self
            .reqwest_client
            .post("https://medcheck.tlth.se")
            .timeout(std::time::Duration::from_secs(5))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("id={pn}"))
            .send()
            .await
            .wrap_err_internal(format!("noalert medcheck: transport error (for {sub})"))?
            .error_for_status()
            .wrap_err_internal(format!("noalert medcheck: status error (for {sub})"))?;
        let body = resp
            .text()
            .await
            .wrap_err_internal("medcheck: transport body error")?;
        let Some((g1, g2)) = (|| {
            let needle = "<div class=\"guilds\">";
            let idx = body.find(needle)?;
            let guild = body.get((idx + needle.len())..)?;
            let g1 = guild.get(..1)?;
            let g2 = guild.get(..2)?;
            Some((g1, g2))
        })() else {
            return Ok(None);
        };
        match g2 {
            "do" => return Ok(Some(Guild::Doct)),
            "in" => return Ok(Some(Guild::Ing)),
            _ => {}
        }
        let guild = match g1 {
            "f" => Guild::F,
            "e" => Guild::E,
            "m" => Guild::M,
            "v" => Guild::V,
            "a" => Guild::A,
            "k" => Guild::K,
            "d" => Guild::D,
            "w" => Guild::W,
            "i" => Guild::I,
            _ => return Ok(None),
        };
        Ok(Some(guild))
    }
    /// # Errors
    ///
    /// - `personal_number`: `xxxxxxxxxx` where x: /[0-9]/
    #[oai(path = "/personal-information", method = "post")]
    async fn info_completion(
        &self,
        body: Json<InfoCompletion>,
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
        let sub = sqlx::query_scalar!(
            "select sub from session_validated_users where session_id = $1",
            body.code
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;

        // if email (admin) login, don't enforce name!
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
        let guild = if let Some(pn) = &body.personal_number {
            if pn.len() != 10 || !pn.chars().all(|char| "0123456789".contains(char)) {
                return Err(MinilithEndpointError::bad_user_input(
                    "personal_number invalid",
                    "",
                    "personal number has to be in the format of 10 numbers without any characters between",
                    "personal_number",
                ));
            }

            // TODO: remove hack, we fall back to E if medcheck is bad
            Some(
                self.get_guild(pn, &sub)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(Guild::E),
            )
        } else {
            None
        };

        sqlx::query!(
            "update session_validated_users set
                full_name = $1,
                lth_guild = $2
            where session_id = $3",
            body.name,
            guild.as_ref().map(Guild::to_str),
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
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
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
        let link = format!("{WEBSITE_DOMAIN}/providers/email/approve/?token={token}");
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
                .await
                .wrap_err_internal("failed to send email")?;
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
            lth_guild: None,
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
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
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
            lth_guild: None,
        };
        self.validate_session(&body.code, &user).await?;

        Ok(PlainText(
            self.provider_callback_next_url(&body.code, &ValidatedAuthSession { session, user })
                .await
                .wrap_err_internal("noalert provider")?,
        ))
    }
    #[oai(path = "/api-key-get-access-token", method = "post")]
    async fn get_at(&self, body: Json<ApiKeyRequest>) -> MinilithResult<Json<ApiKeyResponse>> {
        let row = query!(
            "select user_id, client_id from api_keys where key = $1",
            body.key
        )
        .fetch_one(&self.db)
        .await
        .wrap_err_unauthorized("KEY")?;
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
}
