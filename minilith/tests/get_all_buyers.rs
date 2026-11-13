use sqlx::PgPool;
use uuid::Uuid;

mod lib;

fn id(suffix: u128) -> Uuid {
    Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + suffix)
}

#[sqlx::test(fixtures("get_all_buyers"))]
async fn get_all_buyers_returns_buyers(db: PgPool) {
    let context = minilith::Context::new(Some(db.clone().into()), false)
        .await
        .unwrap();

    let invited_name = context.encrypt("Invited");
    let other_name = context.encrypt("Other");

    sqlx::query!(
        r#"
        update users
        set name = $1
        where id = 'email:invited@example.com'
        "#,
        invited_name
    )
    .execute(&db)
    .await
    .unwrap();

    sqlx::query!(
        r#"
        update users
        set name = $1
        where id = 'email:other@example.com'
        "#,
        other_name
    )
    .execute(&db)
    .await
    .unwrap();

    let client = lib::get_test_client(db.clone()).await.unwrap();

    let response = client
        .post("/v0/tickets/validate/get_all_buyers")
        .header("Authorization", "Bearer email:cohost@example.com")
        .content_type("application/json")
        .body(
            serde_json::json!({
                "validate_activity": id(6),
            })
            .to_string(),
        )
        .send()
        .await;

    response.assert_status_is_ok();

    let body = response.json().await;
    let buyers = body.value().array();

    buyers.assert_len(2);

    let invited = buyers
        .iter()
        .find(|buyer| buyer.object().get("purchased_ticket_id").string() == id(8).to_string())
        .unwrap()
        .object();

    invited
        .get("owner_id")
        .assert_string("email:invited@example.com");
    invited.get("owner_name").assert_string("Invited");
    invited.get("purchaser_name").assert_string("Invited");
    invited.get("has_been_transferred").assert_bool(false);

    let invited_addons = invited.get("purchased_addons").array();
    invited_addons.assert_len(1);

    let invited_addon = invited_addons.get(0).object();

    invited_addon.get("id").assert_string(&id(12).to_string());

    invited_addon
        .get("name")
        .object()
        .get("en")
        .assert_string("Lunch");

    invited_addon
        .get("multiple_alternatives")
        .assert_bool(false);

    invited_addon.get("has_text_field").assert_bool(true);
    invited_addon.get("required").assert_bool(true);

    invited_addon
        .get("selected_text")
        .assert_string("No onions");

    let invited_selected_options = invited_addon.get("selected_options").array();
    invited_selected_options.assert_len(1);
    invited_selected_options.get(0).assert_i64(0);

    let invited_options = invited_addon.get("options").array();
    invited_options.assert_len(2);

    let vegetarian = invited_options
        .iter()
        .find(|option| option.object().get("id").string() == id(13).to_string())
        .unwrap()
        .object();

    vegetarian
        .get("name")
        .object()
        .get("en")
        .assert_string("Vegetarian");
    vegetarian.get("idx").assert_i64(0);
    vegetarian.get("price").assert_i64(2000);

    let meat = invited_options
        .iter()
        .find(|option| option.object().get("id").string() == id(14).to_string())
        .unwrap()
        .object();

    meat.get("name").object().get("en").assert_string("Meat");
    meat.get("idx").assert_i64(1);
    meat.get("price").assert_i64(3000);

    let transferred = buyers
        .iter()
        .find(|buyer| buyer.object().get("purchased_ticket_id").string() == id(10).to_string())
        .unwrap()
        .object();

    transferred
        .get("owner_id")
        .assert_string("email:invited@example.com");
    transferred.get("owner_name").assert_string("Invited");
    transferred.get("purchaser_name").assert_string("Other");
    transferred.get("has_been_transferred").assert_bool(true);

    let transferred_addons = transferred.get("purchased_addons").array();
    transferred_addons.assert_len(1);

    let transferred_addon = transferred_addons.get(0).object();

    transferred_addon
        .get("id")
        .assert_string(&id(12).to_string());

    transferred_addon
        .get("name")
        .object()
        .get("en")
        .assert_string("Lunch");

    transferred_addon
        .get("multiple_alternatives")
        .assert_bool(false);

    transferred_addon.get("has_text_field").assert_bool(true);
    transferred_addon.get("required").assert_bool(true);

    transferred_addon
        .get("selected_text")
        .assert_string("Extra hungry");

    let transferred_selected_options = transferred_addon.get("selected_options").array();

    transferred_selected_options.assert_len(1);
    transferred_selected_options.get(0).assert_i64(1);

    // This is particularly important: both tickets use the same addon,
    // so both must receive the complete list of available options.
    let transferred_options = transferred_addon.get("options").array();
    transferred_options.assert_len(2);

    let vegetarian = transferred_options
        .iter()
        .find(|option| option.object().get("id").string() == id(13).to_string())
        .unwrap()
        .object();

    vegetarian
        .get("name")
        .object()
        .get("en")
        .assert_string("Vegetarian");
    vegetarian.get("idx").assert_i64(0);
    vegetarian.get("price").assert_i64(2000);

    let meat = transferred_options
        .iter()
        .find(|option| option.object().get("id").string() == id(14).to_string())
        .unwrap()
        .object();

    meat.get("name").object().get("en").assert_string("Meat");
    meat.get("idx").assert_i64(1);
    meat.get("price").assert_i64(3000);
}
