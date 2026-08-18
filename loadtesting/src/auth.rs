use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::error::{Result, ResultContext as _, error};

const CLIENT_ID: &str = "teknologappen";

#[derive(Debug)]
pub struct Session {
    pub index: usize,
    pub user_id: String,
    access_token: String,
    refresh_token: String,
    refresh_after: Instant,
}

impl Session {
    pub async fn access_token(&mut self, http: &Client, token_url: &Url) -> Result<&str> {
        if Instant::now() >= self.refresh_after {
            let response = http
                .post(token_url.clone())
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", self.refresh_token.as_str()),
                ])
                .send()
                .await
                .context(format!("refresh token request for {}", self.user_id))?;
            let response = require_success(response, "refresh token exchange").await?;
            let tokens: TokenResponse = response
                .json()
                .await
                .context("decode refresh token response")?;
            self.install(tokens);
        }
        Ok(&self.access_token)
    }

    fn install(&mut self, tokens: TokenResponse) {
        self.access_token = tokens.access_token;
        self.refresh_token = tokens.refresh_token;
        self.refresh_after = Instant::now() + Duration::from_secs(tokens.expires_in / 2);
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Serialize)]
struct TestApproval<'a> {
    code: &'a str,
    stil_id: &'a str,
}

#[derive(Serialize)]
struct PersonalInformation<'a> {
    code: &'a str,
    name: String,
}

pub async fn login(
    http: &Client,
    auth_url: &Url,
    redirect_url: &Url,
    callback_url: &Url,
    user_prefix: &str,
    index: usize,
) -> Result<Session> {
    let stil_id = format!("{user_prefix}-{}", index + 1);
    let user_id = format!("test:{stil_id}");
    let state = Uuid::new_v4().simple().to_string();
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let authorize_url = auth_url
        .join("oidc/v1/authorize")
        .context("construct authorize URL")?;
    let response = http
        .get(authorize_url)
        .query(&[
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect_url.as_str()),
            ("response_type", "code"),
            ("scope", "openid"),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("providers", "test"),
            ("callback_url_v1", callback_url.as_str()),
        ])
        .send()
        .await
        .context(format!("start login for {user_id}"))?;
    if response.status() != StatusCode::FOUND {
        return Err(response_error(response, "authorize request").await);
    }
    let provider_url = redirect_location(&response, "authorize response")?;
    let provider_code = query_value(&provider_url, "code")?;

    let approve_url = auth_url
        .join("api/v0/providers/test/approve")
        .context("construct test approval URL")?;
    let response = http
        .post(approve_url)
        .json(&TestApproval {
            code: &provider_code,
            stil_id: &stil_id,
        })
        .send()
        .await
        .context(format!("approve test login for {user_id}"))?;
    let response = require_success(response, "test-provider approval").await?;
    let mut completion_url = parse_plain_url(response, "test-provider approval").await?;

    if completion_url.path().contains("/personal-information/") {
        let code = query_value(&completion_url, "code")?;
        let personal_information_url = auth_url
            .join("api/v0/personal-information")
            .context("construct personal-information URL")?;
        let response = http
            .post(personal_information_url)
            .json(&PersonalInformation {
                code: &code,
                name: format!("Loadtest Client {}", index + 1),
            })
            .send()
            .await
            .context(format!("complete personal information for {user_id}"))?;
        let response = require_success(response, "personal-information completion").await?;
        completion_url = parse_plain_url(response, "personal-information completion").await?;
    }

    let returned_state = query_value(&completion_url, "state")?;
    if returned_state != state {
        return Err(error(format!("OIDC state mismatch for {user_id}")));
    }
    let authorization_code = query_value(&completion_url, "code")?;
    let token_url = auth_url
        .join("oidc/v1/token")
        .context("construct token URL")?;
    let response = http
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", authorization_code.as_str()),
            ("redirect_uri", redirect_url.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .context(format!("exchange authorization code for {user_id}"))?;
    let response = require_success(response, "authorization-code exchange").await?;
    let tokens: TokenResponse = response
        .json()
        .await
        .context("decode authorization-code response")?;
    let mut session = Session {
        index,
        user_id,
        access_token: String::new(),
        refresh_token: String::new(),
        refresh_after: Instant::now(),
    };
    session.install(tokens);
    Ok(session)
}

fn redirect_location(response: &reqwest::Response, context: &str) -> Result<Url> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| error(format!("{context} did not include Location")))?
        .to_str()
        .context(format!("decode Location from {context}"))?;
    Url::parse(location).context(format!("parse Location from {context}"))
}

fn query_value(url: &Url, name: &str) -> Result<String> {
    url.query_pairs()
        .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
        .ok_or_else(|| {
            error(format!(
                "{} did not include query parameter {name}",
                url.path()
            ))
        })
}

async fn parse_plain_url(response: reqwest::Response, context: &str) -> Result<Url> {
    let body = response
        .text()
        .await
        .context(format!("read {context} response"))?;
    Url::parse(body.trim()).context(format!("parse URL from {context}"))
}

async fn require_success(response: reqwest::Response, context: &str) -> Result<reqwest::Response> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(response_error(response, context).await)
    }
}

async fn response_error(response: reqwest::Response, context: &str) -> crate::error::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(1_000).collect();
    error(format!("{context} returned {status}: {body}"))
}
