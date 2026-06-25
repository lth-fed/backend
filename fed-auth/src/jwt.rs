use poem_openapi::Object;
use serde::Serialize;
use tracing::error;

pub fn encode(claims: impl Serialize, signing_key: &jsonwebtoken::EncodingKey) -> Option<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.kid = Some("main".to_owned());
    jsonwebtoken::encode(&header, &claims, signing_key)
        .inspect_err(|err| {
            error!("failed to encode JWT: {err}");
        })
        .ok()
}

#[derive(Object, Serialize)]
pub struct AccesTokenClaims {
    pub sub: String,
}
