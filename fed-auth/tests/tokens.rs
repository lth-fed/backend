#![allow(unused_crate_dependencies, reason = "this is a test file")]
#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use reqwest::StatusCode;
use sqlx::PgPool;
mod lib;

const INITIAL_TOKEN: &str = "2e9c8efc-c612-4fe7-af15-37b381e712fa";

#[sqlx::test(fixtures("refresh-token"))]
async fn refresh(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={INITIAL_TOKEN}"
        ))
        .send()
        .await;
    response.assert_status_is_ok();
    let json = response.json().await;
    json.value().object().get("access_token").string();
    json.value().object().get("refresh_token").string();
    json.value().object().get("id_token").string();
    Ok(())
}

#[sqlx::test(fixtures("refresh-token"))]
async fn valid(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={INITIAL_TOKEN}"
        ))
        .send()
        .await;
    response.assert_status_is_ok();
    let json = response.json().await;
    let access_token = json.value().object().get("access_token").string();
    let response = app
        .post("/oidc/v1/userinfo")
        .header("authorization", format!("Bearer {access_token}"))
        .send()
        .await;
    response.assert_status_is_ok();
    Ok(())
}

#[sqlx::test(fixtures("refresh-token"))]
async fn refresh_consumes_token(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={INITIAL_TOKEN}"
        ))
        .send()
        .await;
    response.assert_status_is_ok();
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={INITIAL_TOKEN}"
        ))
        .send()
        .await;
    response.assert_status(StatusCode::BAD_REQUEST);
    Ok(())
}
#[sqlx::test(fixtures("refresh-token"))]
async fn refresh_returns_new_refresh(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={INITIAL_TOKEN}"
        ))
        .send()
        .await;
    let body = response.json().await;
    let new_token = body.value().object().get("refresh_token").string();
    let response = app
        .get(format!(
            "/oidc/v1/token?grant_type=refresh_token&refresh_token={new_token}"
        ))
        .send()
        .await;
    response.assert_status_is_ok();
    Ok(())
}
