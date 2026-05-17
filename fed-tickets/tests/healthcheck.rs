#![allow(unused_crate_dependencies, reason = "this is a test file")]
#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use sqlx::PgPool;

mod lib;

#[sqlx::test]
async fn refresh(db: PgPool) -> color_eyre::Result<()> {
    let app = lib::get_test_client(db).await?;
    let response = app.get("/v0/healthcheck").send().await;
    response.assert_status_is_ok();
    Ok(())
}
