//! Licenses for paid ("Pro") extensions.
//!
//! A license is a token `FILES1.<payload>.<signature>`: the payload is JSON
//! (who, which products, until when), the signature is Ed25519 over
//! `FILES1.<payload>` made with the publisher's private key, which never
//! ships. The app embeds only public keys, so a key cannot be forged or
//! edited (adding a product, moving the expiry) without breaking the
//! signature. Checks run offline.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "FILES1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct License {
    /// Order or license number, for support.
    pub id: String,
    pub email: String,
    /// Product ids the license unlocks (`kubernetes`, …); `*` unlocks all.
    pub products: Vec<String>,
    /// Seconds since the Unix epoch.
    pub issued: i64,
    /// `None`: never expires.
    pub expires: Option<i64>,
}

impl License {
    pub fn covers(&self, product: &str) -> bool {
        self.products.iter().any(|known| known == product || known == "*")
    }

    pub fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|expires| now >= expires)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LicenseError {
    /// Not a Files license at all (typo, truncated paste).
    Malformed,
    /// Well formed, but not signed by any trusted key: forged or edited.
    BadSignature,
    Expired(License),
}

impl std::fmt::Display for LicenseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(formatter, "This is not a Files license key"),
            Self::BadSignature => write!(formatter, "This license key is not valid (its signature does not match)"),
            Self::Expired(_) => write!(formatter, "This license has expired"),
        }
    }
}

/// Checks `token` against the trusted public keys (raw 32-byte Ed25519,
/// URL-safe base64). Several keys allow rotating them.
pub fn verify(token: &str, public_keys: &[&str], now: i64) -> Result<License, LicenseError> {
    let token = token.trim();
    let mut parts = token.split('.');
    let (Some(prefix), Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(LicenseError::Malformed);
    };
    if prefix != PREFIX {
        return Err(LicenseError::Malformed);
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| LicenseError::Malformed)?;
    let signed = format!("{PREFIX}.{payload}");
    let trusted = public_keys.iter().filter_map(|key| URL_SAFE_NO_PAD.decode(key).ok()).any(|key| {
        UnparsedPublicKey::new(&ED25519, key).verify(signed.as_bytes(), &signature).is_ok()
    });
    if !trusted {
        return Err(LicenseError::BadSignature);
    }
    let json = URL_SAFE_NO_PAD.decode(payload).map_err(|_| LicenseError::Malformed)?;
    let license: License = serde_json::from_slice(&json).map_err(|_| LicenseError::Malformed)?;
    if license.expired(now) {
        return Err(LicenseError::Expired(license));
    }
    Ok(license)
}

/// Signs a license with a PKCS#8 Ed25519 private key (the publisher's
/// tooling uses this; the app never has a private key).
pub fn issue(license: &License, private_key_pkcs8: &[u8]) -> Result<String, String> {
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(private_key_pkcs8).map_err(|error| format!("bad private key: {error}"))?;
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(license).map_err(|error| error.to_string())?);
    let signed = format!("{PREFIX}.{payload}");
    let signature = URL_SAFE_NO_PAD.encode(pair.sign(signed.as_bytes()).as_ref());
    Ok(format!("{signed}.{signature}"))
}

/// A new key pair: (PKCS#8 private key, public key as URL-safe base64).
pub fn generate_keys() -> Result<(Vec<u8>, String), String> {
    use ring::signature::KeyPair as _;
    let random = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random).map_err(|error| error.to_string())?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|error| error.to_string())?;
    Ok((pkcs8.as_ref().to_vec(), URL_SAFE_NO_PAD.encode(pair.public_key().as_ref())))
}

/// The public key of a PKCS#8 private key (to check which key is which).
pub fn public_key_of(private_key_pkcs8: &[u8]) -> Result<String, String> {
    use ring::signature::KeyPair as _;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(private_key_pkcs8).map_err(|error| format!("bad private key: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> License {
        License {
            id: "ORD-1".into(),
            email: "buyer@example.com".into(),
            products: vec!["kubernetes".into()],
            issued: 1_800_000_000,
            expires: Some(1_900_000_000),
        }
    }

    #[test]
    fn signed_licenses_verify_and_tampering_is_caught() {
        let (private, public) = generate_keys().unwrap();
        let token = issue(&sample(), &private).unwrap();
        let license = verify(&token, &[&public], 1_850_000_000).unwrap();
        assert!(license.covers("kubernetes") && !license.covers("docker"));

        // Editing the payload (here: adding every product) breaks the signature.
        let mut parts: Vec<&str> = token.split('.').collect();
        let mut forged = sample();
        forged.products = vec!["*".into()];
        let forged_payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
        parts[1] = &forged_payload;
        assert_eq!(verify(&parts.join("."), &[&public], 1_850_000_000), Err(LicenseError::BadSignature));

        // A key from someone else's key pair is not trusted.
        let (other_private, _) = generate_keys().unwrap();
        let foreign = issue(&forged, &other_private).unwrap();
        assert_eq!(verify(&foreign, &[&public], 1_850_000_000), Err(LicenseError::BadSignature));

        assert!(matches!(verify(&token, &[&public], 1_950_000_000), Err(LicenseError::Expired(_))));
        assert_eq!(verify("hello", &[&public], 0), Err(LicenseError::Malformed));
        assert_eq!(public_key_of(&private).unwrap(), public);
    }
}
