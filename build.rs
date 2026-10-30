//! Hands `Bundle.toml` values to the app at compile time, so packaging,
//! updates and About read from one config: FILES_UPDATE_URL,
//! FILES_WEBSITE, FILES_COPYRIGHT, FILES_APP_NAME.

fn main() {
    println!("cargo:rerun-if-changed=Bundle.toml");
    let source = std::fs::read_to_string("Bundle.toml").unwrap_or_default();
    // A tiny reader for `key = "value"` under `[section]`: build scripts
    // should stay dependency-free.
    let mut section = String::new();
    let mut values = std::collections::HashMap::new();
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).to_string();
        } else if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            if let Some(value) = value.strip_prefix('"').and_then(|value| value.split('"').next()) {
                values.insert(format!("{section}.{}", key.trim()), value.to_string());
            }
        }
    }
    for (env, key) in [
        ("FILES_UPDATE_URL", "updates.url"),
        ("FILES_WEBSITE", "updates.website"),
        ("FILES_COPYRIGHT", "app.copyright"),
        ("FILES_APP_NAME", "app.name"),
    ] {
        println!("cargo:rustc-env={env}={}", values.get(key).cloned().unwrap_or_default());
    }
}
