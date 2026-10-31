//! Paid ("Pro") extensions: a license key, signed with the publisher's
//! private key, unlocks them. Only the public keys are built in
//! (`packaging/license/public.keys`), so keys cannot be forged or edited; the
//! check is offline. See `cargo xtask license` for issuing keys.

use files_sdk::license::{License, LicenseError};
use files_sdk::{Manifest, Tier};

use crate::settings::AppSettings;

const TRUSTED: &str = include_str!("../packaging/license/public.keys");

const UPDATES: &str = include_str!("../packaging/license/updates.keys");

/// Keys that may sign updates (a separate key the release CI holds).
pub(crate) fn trusted_updates() -> Vec<&'static str> {
    UPDATES.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect()
}

pub(crate) fn trusted() -> Vec<&'static str> {
    TRUSTED.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64)
}

/// The license in settings: `None` when no key was entered.
pub fn status(settings: &AppSettings) -> Option<Result<License, LicenseError>> {
    let key = settings.license.key.trim();
    (!key.is_empty()).then(|| files_sdk::license::verify(key, &trusted(), now()))
}

/// Free extensions always; Pro ones when a valid license covers them.
pub fn unlocked(manifest: &Manifest, settings: &AppSettings) -> bool {
    unlocked_with(manifest, settings, &trusted())
}

pub fn unlocked_with(manifest: &Manifest, settings: &AppSettings, keys: &[&str]) -> bool {
    match manifest.tier {
        Tier::Free => true,
        Tier::Pro { product } => files_sdk::license::verify(settings.license.key.trim(), keys, now()).is_ok_and(|license| license.covers(product)),
    }
}

/// On in Settings *and* allowed by the license: what every caller checks
/// before an extension may run.
pub fn active(settings: &AppSettings, index: usize) -> bool {
    crate::extensions::enabled(&settings.extensions, index)
        && crate::extensions::CATALOG.get(index).is_some_and(|manifest| unlocked(manifest, settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    static PRO: Manifest = Manifest {
        id: "kubernetes",
        name: "Kubernetes",
        version: "1.0.0",
        source: "Marketplace",
        summary: "",
        icon: files_sdk::Icon::Builtin("cloud"),
        adds: &[],
        permissions: &[],
        commands: &[],
        menu: &[],
        sidebar: &[],
        menu_color: None,
        tier: Tier::Pro { product: "kubernetes" },
    };

    #[test]
    fn pro_extensions_need_a_valid_covering_license() {
        let (private, public) = files_sdk::license::generate_keys().unwrap();
        let keys = [public.as_str()];
        let mut settings = AppSettings::default();
        assert!(!unlocked_with(&PRO, &settings, &keys), "no key, no Pro");
        let issue = |products: &[&str], expires: Option<i64>| {
            files_sdk::license::issue(
                &License { id: "T".into(), email: "a@b.c".into(), products: products.iter().map(|p| p.to_string()).collect(), issued: 0, expires },
                &private,
            )
            .unwrap()
        };
        settings.license.key = issue(&["kubernetes"], None);
        assert!(unlocked_with(&PRO, &settings, &keys));
        settings.license.key = issue(&["docker"], None);
        assert!(!unlocked_with(&PRO, &settings, &keys), "a license for another product");
        settings.license.key = issue(&["*"], Some(1));
        assert!(!unlocked_with(&PRO, &settings, &keys), "expired");
        // Free extensions never need a key; the built-in keys reject ours.
        assert!(unlocked(crate::extensions::CATALOG[0], &AppSettings::default()));
        settings.license.key = issue(&["kubernetes"], None);
        assert!(!unlocked(&PRO, &settings), "a key signed by someone else's private key is refused");
        assert!(!trusted().is_empty(), "the app ships a public key");
    }
}
