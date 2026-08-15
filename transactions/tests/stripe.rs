#![allow(unused_crate_dependencies, reason = "this is a test file")]
#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use poem::http::StatusCode;
use sqlx::PgPool;

mod lib;

#[sqlx::test(fixtures("base"))]
async fn stripe_callback_accepts_json_with_charset(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;

    let response = app
        .post("/v0/stripe-callback")
        .header("content-type", "application/json; charset=utf-8")
        .body("{}")
        .send()
        .await;

    // The deliberately missing signature is rejected by the handler.
    // Reaching this proves the JSON media type was not rejected with 415.
    response.assert_status(StatusCode::FORBIDDEN);
    Ok(())
}
