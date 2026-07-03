use jsonwebtoken::jwk::JwkSet;

/// # Errors
#[cfg(test)]
pub async fn get_test_client(
    db: sqlx::PgPool,
) -> color_eyre::Result<poem::test::TestClient<impl poem::Endpoint>> {
    Ok(poem::test::TestClient::new(
        minilith::get_endpoint(Some(db.into()), async {
            Ok(fed_auth_verifier::AuthContext::from_jwks(
                "teknologappen",
                JwkSet { keys: vec![] },
            ))
        })
        .await?,
    ))
}
