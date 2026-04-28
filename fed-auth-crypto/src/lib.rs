use std::cell::RefCell;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer as _};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use serde::{Deserialize, Serialize};

#[allow(
    clippy::unwrap_used,
    reason = "now is guaranteed after UNIX_EPOCH for the systems we are running"
)]
fn s_since1970(st: SystemTime) -> u64 {
    st.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
}

/// Generate a new [`SigningKey`] securely.
#[must_use]
pub fn generate_signing_key() -> SigningKey {
    use rand::RngExt as _;
    let secret_key = UnwrapErr(SysRng).random();
    SigningKey::from_bytes(&secret_key)
}
/// Get a [`VerifyingKey`] from the private [`SigningKey`].
#[must_use]
// mostly here for documentation, considering the contents are super simple
pub fn generate_verifying_key(signing_key: &SigningKey) -> VerifyingKey {
    signing_key.verifying_key()
}
/// Get the bytes of a [`VerifyingKey`] to send to the validators.
/// They convert it back using `VerifyingKey::try_from<&[u8]>`.
#[must_use]
// same as above
pub fn get_verifying_key_bytes(key: &VerifyingKey) -> &[u8] {
    key.as_bytes()
}
/// The result of the validation; is the JWT valid or invalid?
#[must_use]
#[derive(Debug)]
pub enum ValidationResult {
    /// Valid, with the string data:
    ///
    /// > we could use `CompactString` here if we'd want to avoid allocations even more, though I
    /// > think that'll just be confusing and annoying for no real effect.
    Valid { subject: String },
    /// Invalid. This JWT should not be considered.
    Invalid,
}
impl ValidationResult {
    #[cfg(test)]
    #[track_caller]
    fn unwrap_subject(self) -> String {
        let Self::Valid { subject } = self else {
            panic!("expected valid validation result!");
        };
        subject
    }
    #[cfg(test)]
    #[track_caller]
    fn expect_invalid(self) {
        assert!(
            matches!(self, Self::Invalid),
            "expected invalid validation result"
        );
    }
}
/// Return [`ValidationResult::Invalid`] if this is [`None`].
macro_rules! none_invalid {
    ($e: expr) => {{
        let Some(value) = $e else {
            return ValidationResult::Invalid;
        };
        value
    }};
}
/// Return [`ValidationResult::Invalid`] if this is [`Err`].
macro_rules! try_invalid {
    ($e: expr) => {{
        let Ok(value) = $e else {
            return ValidationResult::Invalid;
        };
        value
    }};
}
// so we don't have to allocate buffers each time we validate a JWT
thread_local! {
    // signature is just 64 bytes
    static JWT_DECODE_CACHE: RefCell<(Vec<u8>, Vec<u8>)> = RefCell::new((Vec::with_capacity(1024), Vec::with_capacity(64)));
    static JWT_ENCODE_CACHE: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(1024));
}

#[derive(Serialize, Deserialize, Debug)]
struct JwtPayload<'a> {
    /// Expires (seconds since [`SystemTime::UNIX_EPOCH`]).
    exp: u64,
    /// Not valid before...
    nbf: u64,
    /// Token creation time.
    iat: Option<u64>,

    /// Issuer.
    iss: Option<&'a str>,
    /// Subject (i.e. logged in user for our use case).
    sub: Option<&'a str>,
}
#[derive(Serialize, Deserialize, Debug)]
struct JwtHeader<'a> {
    /// `EdDSA`.
    alg: &'a str,
    /// `ed25519`.
    crv: Option<&'a str>,
    /// `application/json`.
    typ: Option<&'a str>,
}

/// The only valid also & curve is the ones encoded by this crate.
pub fn validate_jwt(jwt: &str, verifying_key: &VerifyingKey) -> ValidationResult {
    let mut iter = jwt.splitn(3, '.');
    // TODO: validate header, especially exp
    let header = none_invalid!(iter.next());
    let payload = none_invalid!(iter.next());
    let signed = &jwt[..(header.len() + 1 + payload.len())];
    let signature = none_invalid!(iter.next());

    JWT_DECODE_CACHE.with(|cell| {
        let mut lock = cell.borrow_mut();
        let (base64_buffer, sig) = &mut *lock;

        base64_buffer.clear();
        try_invalid!(BASE64_URL_SAFE_NO_PAD.decode_vec(header, base64_buffer));

        let header: JwtHeader = try_invalid!(serde_json::from_slice(base64_buffer));
        if !matches!(
            header,
            JwtHeader {
                alg: "EdDSA",
                crv: Some("ed25519"),
                typ: Some("application/json")
            }
        ) {
            return ValidationResult::Invalid;
        }

        base64_buffer.clear();
        try_invalid!(BASE64_URL_SAFE_NO_PAD.decode_vec(payload, base64_buffer));
        let now = s_since1970(SystemTime::now());
        let payload: JwtPayload = try_invalid!(serde_json::from_slice(base64_buffer));
        if payload.exp < now || payload.nbf > now {
            return ValidationResult::Invalid;
        }
        let subject = none_invalid!(payload.sub).to_owned();

        sig.clear();
        try_invalid!(BASE64_URL_SAFE_NO_PAD.decode_vec(signature, sig));

        let signature = try_invalid!(Signature::from_slice(sig));
        try_invalid!(verifying_key.verify_strict(signed.as_bytes(), &signature));
        ValidationResult::Valid { subject }
    })
}
/// The only valid is bearer with the same requirements as [`validate_jwt`].
pub fn validate_authorization_header(
    header: &str,
    verifying_key: &VerifyingKey,
) -> ValidationResult {
    let jwt = none_invalid!(header.strip_prefix("Bearer "));
    validate_jwt(jwt, verifying_key)
}

/// Creates a JSON web token.
///
/// # Panics
///
/// None.
#[must_use]
pub fn create_jwt(
    issuer: &str,
    subject: &str,
    valid_for: Duration,
    signing_key: &SigningKey,
) -> String {
    debug_assert!(!issuer.contains('"'), "issuer contains quotes");
    let now = SystemTime::now();
    // heuristic + base64 heuristic
    let mut jwt = String::with_capacity(196 + (issuer.len()) * 3 / 2);
    JWT_ENCODE_CACHE.with(|cell| {
        let mut lock = cell.borrow_mut();

        let header = JwtHeader {
            alg: "EdDSA",
            crv: Some("ed25519"),
            typ: Some("application/json"),
        };
        lock.clear();
        #[allow(
            clippy::unwrap_used,
            reason = "write to Vec will never fail
            & serde_derive will not fail for strings & integers"
        )]
        serde_json::to_writer(&mut *lock, &header).unwrap();
        BASE64_URL_SAFE_NO_PAD.encode_string(&*lock, &mut jwt);

        jwt.push('.');
        let payload = JwtPayload {
            exp: s_since1970(now + valid_for),
            nbf: s_since1970(now),
            iat: Some(s_since1970(now)),
            iss: Some(issuer),
            sub: Some(subject),
        };
        lock.clear();
        #[allow(
            clippy::unwrap_used,
            reason = "write to Vec will never fail
            & serde_derive will not fail for strings & integers"
        )]
        serde_json::to_writer(&mut *lock, &payload).unwrap();
        BASE64_URL_SAFE_NO_PAD.encode_string(&*lock, &mut jwt);

        let sig = signing_key.sign(jwt.as_bytes());

        jwt.push('.');
        BASE64_URL_SAFE_NO_PAD.encode_string(sig.to_bytes(), &mut jwt);

        jwt
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skvk() -> (SigningKey, VerifyingKey) {
        let sk = generate_signing_key();
        let vk = generate_verifying_key(&sk);
        (sk, vk)
    }

    #[test]
    fn creation() {
        let (sk, _vk) = skvk();
        let _jwt = create_jwt("me", "Maja", Duration::from_secs(10), &sk);
    }
    #[test]
    fn data_shape() {
        let (sk, _vk) = skvk();
        let jwt = create_jwt("me", "Maja", Duration::from_secs(10), &sk);
        assert_eq!(
            jwt.chars().filter(|ch| *ch == '.').count(),
            2,
            "should have 3 sections with 2 dots"
        );
        assert_eq!(
            jwt.chars()
                .filter(|ch| !ch.is_alphanumeric() && !"-_.".contains(*ch))
                .count(),
            0,
            "should only contain url save chars"
        );
    }
    #[test]
    fn validation() {
        let (sk, vk) = skvk();
        let jwt = create_jwt("me", "Maja", Duration::from_secs(10), &sk);
        validate_jwt(&jwt, &vk).unwrap_subject();
    }
    #[test]
    fn validation_with_concat_tamper() {
        let (sk, vk) = skvk();
        let mut jwt = create_jwt("me", "Maja", Duration::from_secs(10), &sk);
        jwt.push_str("hehe");
        validate_jwt(&jwt, &vk).expect_invalid();
    }
    #[test]
    fn validation_with_self_signed() {
        let (sk, vk) = skvk();
        let (sk2, vk2) = skvk();
        let mut jwt = create_jwt("me", "Maja", Duration::from_secs(10), &sk);
        jwt.truncate(jwt.rfind('.').unwrap());
        let sig = sk2.sign(jwt.as_bytes());
        jwt.push('.');
        BASE64_URL_SAFE_NO_PAD.encode_string(sig.to_bytes(), &mut jwt);
        validate_jwt(&jwt, &vk).expect_invalid();
        println!("{jwt}");
        validate_jwt(&jwt, &vk2).unwrap_subject();
    }
}
