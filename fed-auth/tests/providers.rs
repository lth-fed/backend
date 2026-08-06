#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use poem::http::Uri;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
mod lib;

#[sqlx::test]
async fn test(db: PgPool) -> color_eyre::Result<()> {
    #[derive(Serialize)]
    struct DataBody {
        code: String,
        name: String,
        stil_id: String,
    }
    #[derive(Serialize)]
    struct ConfirmBody {
        code: String,
        accepted: bool,
    }
    #[derive(Serialize)]
    struct PoBody {
        code: String,
        name: String,
    }
    #[derive(Deserialize)]
    struct PoQuery {
        code: String,
        sub: String,
    }
    #[derive(Deserialize)]
    struct ResponseQuery {
        code: String,
        state: String,
    }

    let app = lib::get_test_client(db).await?;
    let response = app
        .get("/oidc/v1/authorize?client_id=esek&redirect_uri=https%3A%2F%2Fauth.esek.se%2Fcallback&response_type=code&scope=openid&state=d8b16b2b-7270-481b-ad4d-460cf36cde7f&code_challenge=0eEMCayyIHqsvVRCbxGQ_Q1JnuxkIaiInrC7fNndDZg&code_challenge_method=S256&providers=test")
        .send()
        .await;
    response.assert_status(StatusCode::FOUND);
    let url: Uri = response.0.header("location").unwrap().parse().unwrap();
    println!("Url: {url}");
    let query = url.query().unwrap();
    let code = query.strip_prefix("code=").unwrap();
    println!("{code}");
    let response = app
        .post("/api/v0/providers/test/approve")
        .body_json(&DataBody {
            code: code.to_owned(),
            name: "Erik Davidsson".to_owned(),
            stil_id: "er8380da-s".to_owned(),
        })
        .send()
        .await;
    response.assert_status_is_ok();
    let response = app
        .post("/oidc/v1/confirm-datasharing")
        .body_json(&ConfirmBody {
            code: code.to_owned(),
            accepted: true,
        })
        .send()
        .await;
    response.assert_status_is_ok();
    println!("{:?}", response.0);
    let body = response.json().await;
    let body_url = body.value().object().get("url").string();
    println!("body.url: {body_url}");
    let url: Uri = body_url.parse().unwrap();
    let query = url.query().unwrap();
    let params: PoQuery = serde_urlencoded::from_str(query).unwrap();
    assert_eq!(params.sub, "test:er8380da-s");
    let mut response = app
        .post("/api/v0/personal-information")
        .body_json(&PoBody {
            code: params.code,
            name: "Erik Davidsson".to_owned(),
        })
        .send()
        .await;
    let body = response.0.take_body().into_string().await.unwrap();
    println!("body: {body}");
    let url: Uri = body.parse().unwrap();
    let query = url.query().unwrap();
    let params: ResponseQuery = serde_urlencoded::from_str(query).unwrap();
    let code = &params.code;
    assert_eq!(params.state, "d8b16b2b-7270-481b-ad4d-460cf36cde7f");
    assert_eq!(url.host(), Some("auth.esek.se"));
    assert_eq!(url.path(), "/callback");

    let response = app
        .get(format!("/oidc/v1/token?code={code}&code_verifier=mCDrfQLIngfAIJo4tr54iKLJKpgWM-jsjX3VGa8YV0U&grant_type=authorization_code&client_id=esek&redirect_uri=https%3A%2F%2Fauth.esek.se%2Fcallback"))
        .send()
        .await;
    response.assert_status_is_ok();
    let json = response.json().await;
    let access_token = json.value().object().get("access_token").string();
    println!("at: {access_token}");
    json.value().object().get("refresh_token").string();
    json.value().object().get("id_token").string();

    let response = app
        .post("/oidc/v1/userinfo")
        .header("authorization", format!("Bearer {access_token}"))
        .send()
        .await;
    response.assert_status_is_ok();
    response
        .json()
        .await
        .value()
        .object()
        .get("sub")
        .assert_string("test:er8380da-s");
    Ok(())
}
