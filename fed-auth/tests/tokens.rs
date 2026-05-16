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
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={INITIAL_TOKEN}"),
        )
        .send()
        .await;
    response.assert_status_is_ok();
    response.assert_header_exist("set-cookie");
    let json = response.json().await;
    json.value().object().get("access_token").string();
    Ok(())
}

#[sqlx::test(fixtures("refresh-token"))]
async fn valid(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={INITIAL_TOKEN}"),
        )
        .send()
        .await;
    response.assert_status_is_ok();
    response.assert_header_exist("set-cookie");
    let json = response.json().await;
    let access_token = json.value().object().get("access_token").string();
    let response = app
        .post("/api/v0/verify-access-token")
        .header("origin", "https://localhost")
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
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={INITIAL_TOKEN}"),
        )
        .send()
        .await;
    response.assert_header_exist("set-cookie");
    let response = app
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={INITIAL_TOKEN}"),
        )
        .send()
        .await;
    response.assert_status(StatusCode::UNAUTHORIZED);
    Ok(())
}
#[sqlx::test(fixtures("refresh-token"))]
async fn refresh_returns_new_refresh(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={INITIAL_TOKEN}"),
        )
        .send()
        .await;
    response.assert_header_exist("set-cookie");
    let new_token = response.0.header("set-cookie").unwrap();
    let new_token = new_token
        .strip_prefix("teknologappen-auth-refresh-token=")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let response = app
        .post("/api/v0/refresh")
        .header("origin", "https://localhost")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={new_token}"),
        )
        .send()
        .await;
    response.assert_header_exist("set-cookie");
    Ok(())
}
