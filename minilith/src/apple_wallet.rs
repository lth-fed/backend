//! Signing Apple Wallet passes.
//!
//! A `.pkpass` is a zip of `pass.json`, its images, a `manifest.json` mapping every file to its
//! SHA-1, and `signature`: a detached PKCS #7 signature of the manifest, made with the pass type
//! certificate and carrying Apple's WWDR intermediate. See Apple's "Building a pass".
//!
//! What goes into `pass.json` is the caller's business; this module only packages and signs.

use std::fmt::Write as _;
use std::path::Path;

use color_eyre::eyre::{Context as _, eyre};
use openssl::hash::MessageDigest;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkcs12::Pkcs12;
use openssl::pkey::{PKey, Private};
use openssl::sign::Signer;
use openssl::stack::Stack;
use openssl::x509::X509;
use tracing::warn;

/// The images every pass carries, from `assets/wallet`. The icon is required; Wallet shows it on
/// the lock screen and in notifications. The logo sits in the pass's top-left corner.
const IMAGES: [(&str, &[u8]); 6] = [
    ("icon.png", include_bytes!("../assets/wallet/icon.png")),
    (
        "icon@2x.png",
        include_bytes!("../assets/wallet/icon@2x.png"),
    ),
    (
        "icon@3x.png",
        include_bytes!("../assets/wallet/icon@3x.png"),
    ),
    ("logo.png", include_bytes!("../assets/wallet/logo.png")),
    (
        "logo@2x.png",
        include_bytes!("../assets/wallet/logo@2x.png"),
    ),
    (
        "logo@3x.png",
        include_bytes!("../assets/wallet/logo@3x.png"),
    ),
];

/// Signs passes with the pass type certificate.
pub struct ApplePassSigner {
    pub pass_type_identifier: String,
    pub team_identifier: String,
    certificate: X509,
    key: PKey<Private>,
    wwdr: X509,
}

impl std::fmt::Debug for ApplePassSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplePassSigner")
            .field("pass_type_identifier", &self.pass_type_identifier)
            .field("team_identifier", &self.team_identifier)
            .finish_non_exhaustive()
    }
}

impl ApplePassSigner {
    /// Reads `APPLE_PASS_TYPE_IDENTIFIER`, `APPLE_PASS_TEAM_IDENTIFIER`, `APPLE_PASS_P12_PATH`,
    /// `APPLE_PASS_P12_PASSWORD` and `APPLE_PASS_WWDR_PATH`.
    ///
    /// Returns `None`, with a warning, when none of them are set: passes are then simply not
    /// offered. A partial set is an error, since it's a deployment mistake rather than a choice.
    ///
    /// The `.p12` must use an algorithm OpenSSL 3 reads without its legacy provider (AES, not the
    /// RC2/3DES Keychain Access exports by default). See `.env.example` for the conversion.
    ///
    /// # Errors
    ///
    /// A partial configuration, or files that can't be read or decrypted.
    pub fn from_env() -> color_eyre::Result<Option<Self>> {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        let values = (
            var("APPLE_PASS_TYPE_IDENTIFIER"),
            var("APPLE_PASS_TEAM_IDENTIFIER"),
            var("APPLE_PASS_P12_PATH"),
            var("APPLE_PASS_P12_PASSWORD"),
            var("APPLE_PASS_WWDR_PATH"),
        );
        let (pass_type_identifier, team_identifier, p12_path, p12_password, wwdr_path) =
            match values {
                (None, None, None, None, None) => {
                    warn!("Apple Wallet passes disabled because `APPLE_PASS_*` is not set");
                    return Ok(None);
                }
                (Some(kind), Some(team), Some(p12), Some(password), Some(wwdr)) => {
                    (kind, team, p12, password, wwdr)
                }
                _ => {
                    return Err(eyre!(
                        "Apple Wallet passes are partly configured: set all of \
                        `APPLE_PASS_TYPE_IDENTIFIER`, `APPLE_PASS_TEAM_IDENTIFIER`, \
                        `APPLE_PASS_P12_PATH`, `APPLE_PASS_P12_PASSWORD` and `APPLE_PASS_WWDR_PATH`, \
                        or none of them"
                    ));
                }
            };

        let p12 = std::fs::read(&p12_path).wrap_err("Failed to read `APPLE_PASS_P12_PATH`")?;
        let parsed = Pkcs12::from_der(&p12)
            .and_then(|p12| p12.parse2(&p12_password))
            .wrap_err(
                "Failed to open `APPLE_PASS_P12_PATH` (wrong password, or a legacy RC2 .p12?)",
            )?;
        let (Some(certificate), Some(key)) = (parsed.cert, parsed.pkey) else {
            return Err(eyre!(
                "`APPLE_PASS_P12_PATH` must hold the certificate and its key"
            ));
        };
        let wwdr = read_certificate(Path::new(&wwdr_path))
            .wrap_err("Failed to read `APPLE_PASS_WWDR_PATH`")?;

        Ok(Some(Self {
            pass_type_identifier,
            team_identifier,
            certificate,
            key,
            wwdr,
        }))
    }

    /// Packages `pass_json` with the standard images, and signs it.
    ///
    /// # Errors
    ///
    /// OpenSSL failing to sign.
    pub fn sign(&self, pass_json: &[u8]) -> color_eyre::Result<Vec<u8>> {
        let mut files: Vec<(&str, &[u8])> = Vec::with_capacity(IMAGES.len() + 3);
        files.push(("pass.json", pass_json));
        files.extend(IMAGES);
        let manifest = manifest(&files);

        let mut chain = Stack::new()?;
        chain.push(self.wwdr.clone())?;
        // Detached, so the manifest isn't embedded; binary, so OpenSSL doesn't rewrite line
        // endings. The default signed attributes (including the signing time) stay: Wallet expects
        // them.
        let signature = Pkcs7::sign(
            &self.certificate,
            &self.key,
            &chain,
            &manifest,
            Pkcs7Flags::DETACHED | Pkcs7Flags::BINARY,
        )?
        .to_der()?;

        files.push(("manifest.json", &manifest));
        files.push(("signature", &signature));
        Ok(zip_stored(&files))
    }
}

fn read_certificate(path: &Path) -> color_eyre::Result<X509> {
    let bytes = std::fs::read(path)?;
    // Apple ships the WWDR certificate as DER (`.cer`); accept PEM too.
    Ok(X509::from_der(&bytes).or_else(|_| X509::from_pem(&bytes))?)
}

/// `manifest.json`: every file name mapped to the hex SHA-1 of its contents.
fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: serde_json::Map<String, serde_json::Value> = files
        .iter()
        .map(|(name, data)| {
            let digest = openssl::sha::sha1(data);
            let hex = digest.iter().fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
            ((*name).to_owned(), serde_json::Value::String(hex))
        })
        .collect();
    serde_json::Value::Object(entries).to_string().into_bytes()
}

/// A zip archive of `files`, uncompressed. Wallet accepts stored entries, and the images are
/// already compressed PNGs, so deflating would buy almost nothing.
fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    // The DOS date for 1980-01-01, the earliest a zip can say; no time is recorded, so the archive
    // for the same pass is byte-for-byte the same apart from the signature.
    const DOS_DATE: u16 = 0x21;

    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let crc = crc32fast::hash(data);
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);

        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0_u16.to_le_bytes()); // flags
        out.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0_u16.to_le_bytes()); // time
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes()); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0_u16.to_le_bytes()); // flags
        central.extend_from_slice(&0_u16.to_le_bytes()); // method
        central.extend_from_slice(&0_u16.to_le_bytes()); // time
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&0_u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = u32::try_from(out.len()).unwrap_or(u32::MAX);
    let central_size = u32::try_from(central.len()).unwrap_or(u32::MAX);
    let count = u16::try_from(files.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // this disk, disk with the central directory
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // comment
    out
}

/// The signature in a pass's barcode, which ties it to the ticket *and* its current owner: a
/// transfer changes the owner, so the old owner's pass stops validating without anything having
/// to reach their phone.
///
/// HMAC-SHA256 under a key derived from `secret` for this one purpose, so the key is never used
/// for anything else and the format can be replaced by bumping the version in the label.
///
/// # Errors
///
/// OpenSSL failing to compute an HMAC.
pub fn barcode_signature(
    secret: &[u8],
    ticket_id: uuid::Uuid,
    owner_id: &str,
) -> color_eyre::Result<String> {
    let key = hmac_sha256(secret, b"wallet-pass-barcode-v1")?;
    let mac = hmac_sha256(&key, format!("{ticket_id}\n{owner_id}").as_bytes())?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        mac,
    ))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> color_eyre::Result<Vec<u8>> {
    let key = PKey::hmac(key)?;
    let mut signer = Signer::new(MessageDigest::sha256(), &key)?;
    signer.update(data)?;
    Ok(signer.sign_to_vec()?)
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    reason = "a wrong index fails the test, which is the point"
)]
mod tests {
    use super::*;

    #[test]
    fn manifest_maps_each_file_to_its_sha1() {
        let manifest = manifest(&[("pass.json", b"{}"), ("icon.png", b"")]);
        let parsed: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        // `sha1("{}")` and `sha1("")`.
        assert_eq!(
            parsed["pass.json"],
            "bf21a9e8fbc5a3846fb05b4fa0859e0917b2202f"
        );
        assert_eq!(
            parsed["icon.png"],
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[test]
    fn zip_lists_every_file_in_its_central_directory() {
        let zip = zip_stored(&[("a.txt", b"hello"), ("b.txt", b"")]);
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], &0x0605_4b50_u32.to_le_bytes());
        let count = u16::from_le_bytes([zip[end + 10], zip[end + 11]]);
        assert_eq!(count, 2);
        let central_offset =
            u32::from_le_bytes([zip[end + 16], zip[end + 17], zip[end + 18], zip[end + 19]]);
        // Header (30) + name (5) + data (5), then header (30) + name (5).
        assert_eq!(central_offset, 75);
        assert_eq!(&zip[30..35], b"a.txt");
        assert_eq!(&zip[35..40], b"hello");
    }

    #[test]
    fn barcode_signature_depends_on_the_owner() {
        let ticket = uuid::Uuid::nil();
        let secret = [7_u8; 32];
        let mine = barcode_signature(&secret, ticket, "test:aa1234aa-s").unwrap();
        assert_eq!(
            mine,
            barcode_signature(&secret, ticket, "test:aa1234aa-s").unwrap()
        );
        assert_ne!(
            mine,
            barcode_signature(&secret, ticket, "test:er7826an-s").unwrap()
        );
        assert_ne!(
            mine,
            barcode_signature(&[8_u8; 32], ticket, "test:aa1234aa-s").unwrap()
        );
        // 32 bytes, unpadded base64url.
        assert_eq!(mine.len(), 43);
    }
}
