use serde::Serialize;
use tracing::error;

pub const JWT_AUDIENCE: &str = "teknologappen.se";

pub fn encode(claims: impl Serialize, signing_key: &jsonwebtoken::EncodingKey) -> Option<String> {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
        &GeneralClaims::new(claims),
        signing_key,
    )
    .inspect_err(|err| {
        error!("failed to encode JWT: {err}");
    })
    .ok()
}

#[derive(Serialize, Clone, Debug)]
pub struct GeneralClaims<T> {
    exp: u64,
    nbf: u64,
    aud: String,
    #[serde(flatten)]
    pub other_claims: T,
}
impl<T> GeneralClaims<T> {
    pub fn new(claims: T) -> Self {
        let now = jsonwebtoken::get_current_timestamp();
        Self {
            exp: now + 60 * 60,
            nbf: now,
            aud: JWT_AUDIENCE.into(),
            other_claims: claims,
        }
    }
}
#[derive(Serialize)]
pub struct AccesTokenClaims {
    pub sub: String,
}
