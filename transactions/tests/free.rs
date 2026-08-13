#![allow(unused_crate_dependencies, reason = "this is a test file")]
#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known;
use uuid::Uuid;

mod lib;

#[sqlx::test(fixtures("base"))]
async fn free(db: PgPool) -> color_eyre::Result<()> {
    #[derive(Serialize)]
    struct Ware {
        name: String,
        amount: i64,
        currency: String,
        tax: f64,
    }
    #[derive(Serialize)]
    struct Body {
        id: Uuid,
        timeout: String,
        wares: Vec<Ware>,
    }
    #[derive(Serialize)]
    struct ReceiptBody {
        language: String,
        customer_name: String,
    }

    let app = lib::get_test_client(db).await?;

    let id = app
        .post("/v0/init")
        .header("authorization", "bearer hehe-super-secure")
        .send()
        .await
        .json()
        .await
        .value()
        .string()
        .parse::<Uuid>()
        .unwrap();

    let body = Body {
        id,
        timeout: (OffsetDateTime::now_utc() + time::Duration::HOUR)
            .format(&well_known::Iso8601::DEFAULT)
            .unwrap(),
        wares: vec![Ware {
            name: "Lunchbiljett".to_owned(),
            amount: 0,
            tax: 1.25,
            currency: "SEK".to_owned(),
        }],
    };

    let response = app
        .post("/v0/free")
        .header("authorization", "bearer hehe-super-secure")
        .body_json(&body)
        .send()
        .await;
    response.assert_status_is_ok();
    let mut receipt = app
        .post(format!("/v0/{id}/receipt"))
        .header("authorization", "bearer hehe-super-secure")
        .body_json(&ReceiptBody {
            language: "sv".to_owned(),
            customer_name: "Erik Davidsson".to_owned(),
        })
        .send()
        .await;
    receipt.assert_status_is_ok();
    receipt.assert_content_type("application/octet-stream");
    let body = receipt.0.take_body().into_bytes().await.unwrap();
    assert!(body.starts_with(b"%PDF"));
    Ok(())
}
