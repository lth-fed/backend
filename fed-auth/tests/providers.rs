#![allow(special_module_name, reason = "test shennanigans")]
#![cfg(test)]

use poem::http::Uri;
use serde::Serialize;
use sqlx::PgPool;
mod lib;

#[sqlx::test]
async fn test(db: PgPool) -> color_eyre::Result<()> {
    #[derive(Serialize)]
    struct RequestBody {
        continue_url: String,
    }
    #[derive(Serialize)]
    struct DataBody {
        id: String,
        name: String,
        stil_id: String,
    }
    #[derive(Serialize)]
    struct ConfirmBody {
        id: String,
        accepted: bool,
    }

    let app = lib::get_test_client(db).await?;
    let response = app
        .post("/api/v0/providers/test")
        .header("content-type", "application/json")
        .header("origin", "https://auth.esek.se")
        .body_json(&RequestBody {
            continue_url: "https://auth.esek.se/auth/return".to_owned(),
        })
        .send()
        .await;
    response.assert_status_is_ok();
    let body = response.0.into_body().into_string().await.unwrap();
    println!("body: {body}");
    let url: Uri = body.parse().unwrap();
    println!("Url: {url}");
    let query = url.query().unwrap();
    let id = query.strip_prefix("id=").unwrap();
    println!("{id}");
    // the test provider doesn't check name
    // let response = app
    //     .post("/api/v0/providers/test/approve")
    //     .header("origin", "https://auth.esek.se")
    //     .header("content-type", "application/json")
    //     .body_json(&Body {
    //         id: id.to_owned(),
    //         name: "Hej".to_owned(),
    //         stil_id: "er8380da-s".to_owned(),
    //     })
    //     .send()
    //     .await;
    // response.assert_status(StatusCode::BAD_REQUEST);
    let response = app
        .post("/api/v0/providers/test/approve")
        .body_json(&DataBody {
            id: id.to_owned(),
            name: "Erik Davidsson".to_owned(),
            stil_id: "er8380da-s".to_owned(),
        })
        .send()
        .await;
    response.assert_status_is_ok();
    let response = app
        .post("/api/v0/confirm-datasharing")
        .body_json(&ConfirmBody {
            id: id.to_owned(),
            accepted: true,
        })
        .send()
        .await;
    response.assert_status_is_ok();
    println!("{:?}", response.0);
    response.assert_header_exist("set-cookie");
    let token = response.0.header("set-cookie").unwrap();
    let token = token
        .strip_prefix("teknologappen-auth-refresh-token=")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let json = response.json().await;
    let url = json.value().object().get("url").string();
    assert!(url.starts_with("https://auth.esek.se/auth/return?validated=true"));

    println!("refresh token: {token}");
    let response = app
        .post("/api/v0/refresh")
        .header("origin", "https://auth.esek.se")
        .header(
            "cookie",
            format!("teknologappen-auth-refresh-token={token}"),
        )
        .send()
        .await;
    response.assert_status_is_ok();
    response.assert_header_exist("set-cookie");
    let json = response.json().await;
    let access_token = json.value().object().get("access_token").string();

    let response = app
        .post("/api/v0/verify-access-token")
        .header("origin", "https://auth.esek.se")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await;
    response.assert_status_is_ok();
    Ok(())
}
