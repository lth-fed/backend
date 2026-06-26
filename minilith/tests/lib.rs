use jsonwebtoken::jwk::JwkSet;

/// # Errors
#[cfg(test)]
pub async fn get_test_client(
    db: sqlx::PgPool,
) -> color_eyre::Result<poem::test::TestClient<impl poem::Endpoint>> {
    let context = minilith::Context::new(Some(db.into())).await?;
    Ok(poem::test::TestClient::new(minilith::get_endpoint(
        context,
        fed_auth_verifier::AuthContext::from_jwks("teknologappen", JwkSet { keys: vec![] }),
    )?))
}
