use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::Object;
use serde::Serialize;

pub fn encode(
    claims: &StandardClaims<impl Serialize>,
    signing_key: &jsonwebtoken::EncodingKey,
) -> MinilithResult<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.kid = Some("main".to_owned());
    jsonwebtoken::encode(&header, claims, signing_key).wrap_err_internal("failed to encode JWT")
}

#[derive(Serialize)]
pub struct StandardClaims<T: Serialize> {
    pub exp: u64,
    pub iat: u64,
    pub nbf: u64,
    pub aud: String,
    #[serde(flatten)]
    pub inner: T,
}
impl<T: Serialize> StandardClaims<T> {
    pub fn new(aud: impl Into<String>, lifetime: u64, inner: T) -> Self {
        let now = jsonwebtoken::get_current_timestamp();
        Self {
            exp: now + lifetime,
            iat: now,
            nbf: now,
            aud: aud.into(),
            inner,
        }
    }
}
#[derive(Serialize)]
pub struct AccesTokenClaims {
    pub sub: String,
}
#[derive(Object, Serialize)]
pub struct UserInfoClaims {
    pub sub: String,
}
