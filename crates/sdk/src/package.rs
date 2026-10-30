//! Signed extension packages: a paid extension is downloaded as a file plus
//! an Ed25519 signature of its SHA-256; Files loads it only when the
//! signature comes from a trusted publisher key, so a patched or swapped
//! package is refused.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{ED25519, UnparsedPublicKey};
use sha2::{Digest as _, Sha256};

/// `sha256:<hex>` of a package.
pub fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!("sha256:{}", hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// Signs a package (publisher tooling).
pub fn sign(bytes: &[u8], private_key_pkcs8: &[u8]) -> Result<String, String> {
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(private_key_pkcs8).map_err(|error| format!("bad private key: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(pair.sign(digest(bytes).as_bytes()).as_ref()))
}

/// True when `signature` is a trusted key's signature of exactly `bytes`.
pub fn verify(bytes: &[u8], signature: &str, public_keys: &[&str]) -> bool {
    let Ok(signature) = URL_SAFE_NO_PAD.decode(signature.trim()) else {
        return false;
    };
    let digest = digest(bytes);
    public_keys
        .iter()
        .filter_map(|key| URL_SAFE_NO_PAD.decode(key).ok())
        .any(|key| UnparsedPublicKey::new(&ED25519, key).verify(digest.as_bytes(), &signature).is_ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_changed_byte_is_refused() {
        let (private, public) = crate::license::generate_keys().unwrap();
        let package = b"kubernetes extension v1".to_vec();
        let signature = super::sign(&package, &private).unwrap();
        assert!(super::verify(&package, &signature, &[&public]));
        let mut patched = package.clone();
        patched[0] ^= 1;
        assert!(!super::verify(&patched, &signature, &[&public]), "a patched package must be refused");
        assert!(!super::verify(&package, "garbage", &[&public]));
    }
}
