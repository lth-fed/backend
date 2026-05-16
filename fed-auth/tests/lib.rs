/// # Errors
#[cfg(test)]
pub async fn get_test_client(
    db: sqlx::PgPool,
) -> color_eyre::Result<poem::test::TestClient<impl poem::Endpoint>> {
    Ok(poem::test::TestClient::new(
        fed_auth::get_endpoint(Some(db)).await?,
    ))
}
