//! Packaging for Files, driven by `Bundle.toml`:
//!
//! - `cargo xtask icons`  — render `packaging/icon/icon.svg` to every size
//!   (PNGs, `AppIcon.icns` on macOS);
//! - `cargo xtask bundle` — release build → `Files.app` (+ `.dmg`) or an
//!   AppImage, signed and notarized as the config says;
//! - `cargo xtask doctor` — what is still missing to publish;
//! - `cargo xtask license keygen | issue | sign-package` — licenses for paid
//!   extensions (the private key stays in `~/.config/files-license/`).
//!
//! Flags override the config: `--sign "Developer ID Application: …"`,
//! `--notarize`, `--no-dmg`. `FILES_SIGNING_IDENTITY` overrides too.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    app: App,
    macos: Macos,
    linux: Linux,
    #[serde(default)]
    updates: Updates,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Updates {
    url: String,
    download_base: String,
}

#[derive(Deserialize)]
struct App {
    name: String,
    identifier: String,
    version: String,
    icon: String,
    copyright: String,
    category: String,
    binary: String,
}

#[derive(Deserialize)]
struct Macos {
    minimum_system_version: String,
    signing_identity: String,
    entitlements: String,
    hardened_runtime: bool,
    notarize: bool,
    notary_profile: String,
    dmg: bool,
}

#[derive(Deserialize)]
struct Linux {
    appimage: bool,
    categories: String,
}

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("icons") => load().and_then(|(root, config)| icons(&root, &config).map(drop)),
        Some("bundle") => load().and_then(|(root, config)| bundle(&root, config, &args[1..])),
        Some("doctor") => load().and_then(|(root, config)| doctor(&root, &config)),
        Some("license") => license(&workspace_root(), &args[1..]),
        Some("package-extension") => package_extension(&workspace_root(), &args[1..]),
        _ => {
            eprintln!(
                "usage: cargo xtask <icons | bundle [--sign IDENTITY] [--notarize] [--no-dmg] | doctor\n\
                 \x20      | license keygen | license issue --email E --products p1,p2 [--days N] [--id ORDER]\n\
                 \x20      | license sign-package FILE | package-extension CRATE [--debug]>"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    // crates/xtask → the workspace.
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("workspace").to_path_buf()
}

fn load() -> Result<(PathBuf, Config)> {
    let root = workspace_root();
    let source = std::fs::read_to_string(root.join("Bundle.toml")).map_err(|error| format!("Bundle.toml: {error}"))?;
    let config = toml::from_str(&source).map_err(|error| format!("Bundle.toml: {error}"))?;
    Ok((root, config))
}

fn version(root: &Path, config: &Config) -> Result<String> {
    if config.app.version != "cargo" {
        return Ok(config.app.version.clone());
    }
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let value: toml::Value = toml::from_str(&manifest).map_err(|error| error.to_string())?;
    value
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "no [package] version in Cargo.toml".to_string())
}

fn run(command: &mut Command) -> Result {
    let line = format!("{command:?}");
    println!("  $ {}", line.replace('"', ""));
    let status = command.status().map_err(|error| format!("{line}: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{line} exited with {status}")) }
}

fn output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

// ------------------------------------------------------------ icons

/// Renders the SVG at `size`×`size` with transparency.
fn render(svg: &[u8], size: u32, target: &Path) -> Result {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).map_err(|error| format!("icon.svg: {error}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("bad icon size")?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(target).map_err(|error| format!("{}: {error}", target.display()))
}

/// Every icon file the bundles need; returns the generated folder.
fn icons(root: &Path, config: &Config) -> Result<PathBuf> {
    let svg = std::fs::read(root.join(&config.app.icon)).map_err(|error| format!("{}: {error}", config.app.icon))?;
    let out = root.join("target/icons");
    let iconset = out.join("AppIcon.iconset");
    std::fs::create_dir_all(&iconset).map_err(|error| error.to_string())?;
    println!("Icons → {}", out.display());
    // The sizes macOS asks for in an .iconset.
    for (size, name) in [
        (16, "icon_16x16"),
        (32, "icon_16x16@2x"),
        (32, "icon_32x32"),
        (64, "icon_32x32@2x"),
        (128, "icon_128x128"),
        (256, "icon_128x128@2x"),
        (256, "icon_256x256"),
        (512, "icon_256x256@2x"),
        (512, "icon_512x512"),
        (1024, "icon_512x512@2x"),
    ] {
        render(&svg, size, &iconset.join(format!("{name}.png")))?;
    }
    for size in [256, 512, 1024] {
        render(&svg, size, &out.join(format!("icon-{size}.png")))?;
    }
    if cfg!(target_os = "macos") {
        run(Command::new("iconutil").args(["-c", "icns", "-o"]).arg(out.join("AppIcon.icns")).arg(&iconset))?;
    }
    Ok(out)
}

// ------------------------------------------------------------ bundle

fn bundle(root: &Path, mut config: Config, flags: &[String]) -> Result {
    let mut iter = flags.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--sign" => config.macos.signing_identity = iter.next().ok_or("--sign needs an identity")?.clone(),
            "--notarize" => config.macos.notarize = true,
            "--no-dmg" => config.macos.dmg = false,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if let Ok(identity) = std::env::var("FILES_SIGNING_IDENTITY") {
        config.macos.signing_identity = identity;
    }
    let version = version(root, &config)?;
    println!("Building {} {version} (release)…", config.app.name);
    run(Command::new(env!("CARGO")).current_dir(root).args(["build", "--release", "--package", &config.app.binary]))?;
    let icons = icons(root, &config)?;
    let dist = root.join("target/dist");
    std::fs::create_dir_all(&dist).map_err(|error| error.to_string())?;
    if cfg!(target_os = "macos") {
        bundle_macos(root, &config, &version, &icons, &dist)
    } else if cfg!(target_os = "linux") {
        bundle_linux(root, &config, &version, &icons, &dist)
    } else {
        Err("bundling is set up for macOS and Linux".to_string())
    }
}

fn info_plist(config: &Config, version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>{name}</string>
    <key>CFBundleDisplayName</key><string>{name}</string>
    <key>CFBundleIdentifier</key><string>{identifier}</string>
    <key>CFBundleExecutable</key><string>{binary}</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>{minimum}</string>
    <key>LSApplicationCategoryType</key><string>{category}</string>
    <key>NSHumanReadableCopyright</key><string>{copyright}</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
"#,
        name = config.app.name,
        identifier = config.app.identifier,
        binary = config.app.binary,
        minimum = config.macos.minimum_system_version,
        category = config.app.category,
        copyright = config.app.copyright,
    )
}

fn bundle_macos(root: &Path, config: &Config, version: &str, icons: &Path, dist: &Path) -> Result {
    let app = dist.join(format!("{}.app", config.app.name));
    let _ = std::fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    for dir in ["MacOS", "Resources"] {
        std::fs::create_dir_all(contents.join(dir)).map_err(|error| error.to_string())?;
    }
    let binary = root.join("target/release").join(&config.app.binary);
    std::fs::copy(&binary, contents.join("MacOS").join(&config.app.binary)).map_err(|error| format!("{}: {error}", binary.display()))?;
    std::fs::copy(icons.join("AppIcon.icns"), contents.join("Resources/AppIcon.icns")).map_err(|error| error.to_string())?;
    std::fs::write(contents.join("Info.plist"), info_plist(config, version)).map_err(|error| error.to_string())?;
    std::fs::write(contents.join("PkgInfo"), "APPL????").map_err(|error| error.to_string())?;
    println!("App → {}", app.display());

    let identity = config.macos.signing_identity.as_str();
    let ad_hoc = identity == "-";
    println!("Signing ({})…", if ad_hoc { "ad hoc" } else { identity });
    let mut sign = Command::new("codesign");
    sign.args(["--force", "--deep", "--sign", identity]);
    if !ad_hoc {
        sign.arg("--timestamp");
    }
    if config.macos.hardened_runtime {
        sign.args(["--options", "runtime"]);
    }
    sign.arg("--entitlements").arg(root.join(&config.macos.entitlements)).arg(&app);
    run(&mut sign)?;
    run(Command::new("codesign").args(["--verify", "--deep", "--strict", "--verbose=2"]).arg(&app))?;

    if config.macos.notarize {
        if ad_hoc || !identity.starts_with("Developer ID Application") {
            return Err("notarization needs a \"Developer ID Application\" identity (see cargo xtask doctor)".to_string());
        }
        println!("Notarizing (profile {})…", config.macos.notary_profile);
        let zip = dist.join(format!("{}-{version}.zip", config.app.name));
        run(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(&app).arg(&zip))?;
        run(Command::new("xcrun").args(["notarytool", "submit"]).arg(&zip).args(["--keychain-profile", &config.macos.notary_profile, "--wait"]))?;
        run(Command::new("xcrun").args(["stapler", "staple"]).arg(&app))?;
        let _ = std::fs::remove_file(&zip);
    }

    if config.macos.dmg {
        let dmg = dist.join(format!("{}-{version}.dmg", config.app.name));
        let _ = std::fs::remove_file(&dmg);
        // A folder with the app and a link to /Applications to drag it onto.
        let staging = dist.join("dmg");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
        run(Command::new("ditto").arg(&app).arg(staging.join(format!("{}.app", config.app.name))))?;
        std::os::unix::fs::symlink("/Applications", staging.join("Applications")).map_err(|error| error.to_string())?;
        run(Command::new("hdiutil")
            .args(["create", "-volname", &config.app.name, "-fs", "HFS+", "-format", "UDZO", "-ov", "-srcfolder"])
            .arg(&staging)
            .arg(&dmg))?;
        let _ = std::fs::remove_dir_all(&staging);
        if !ad_hoc {
            run(Command::new("codesign").args(["--force", "--timestamp", "--sign", identity]).arg(&dmg))?;
            if config.macos.notarize {
                run(Command::new("xcrun").args(["notarytool", "submit"]).arg(&dmg).args(["--keychain-profile", &config.macos.notary_profile, "--wait"]))?;
                run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
            }
        }
        println!("Disk image → {}", dmg.display());
        publish_update(config, version, &dmg, "macos")?;
    }
    Ok(())
}

/// Signs an update with the publisher key (the one that signs licenses)
/// and writes `latest.json` next to it: upload both to the website.
fn publish_update(config: &Config, version: &str, file: &Path, platform: &str) -> Result {
    let Ok(private) = read_private_key() else {
        println!("No publisher key: the update is not signed, Files will only show a download link (cargo xtask license keygen)");
        return Ok(());
    };
    let bytes = std::fs::read(file).map_err(|error| error.to_string())?;
    let signature = files_sdk::package::sign(&bytes, &private)?;
    let mut sig = file.as_os_str().to_owned();
    sig.push(".sig");
    std::fs::write(&sig, &signature).map_err(|error| error.to_string())?;
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let base = config.updates.download_base.trim_end_matches('/');
    if base.is_empty() {
        println!("Set [updates] download_base in Bundle.toml to write latest.json");
        return Ok(());
    }
    let latest = file.with_file_name("latest.json");
    let mut json: serde_json::Value = std::fs::read_to_string(&latest).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_else(|| serde_json::json!({}));
    // Keep the notes written by hand for this version.
    if json["version"].as_str() != Some(version) {
        json = serde_json::json!({ "version": version, "notes": "" });
    }
    json[platform] = serde_json::json!({ "url": format!("{base}/{name}"), "signature": signature });
    json["published"] = serde_json::json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs()).to_string());
    std::fs::write(&latest, serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    println!("Update → {} (+ .sig) · upload it with {} to {base}/", latest.display(), name);
    if config.updates.url.is_empty() {
        println!("Set [updates] url = \"{base}/latest.json\" so Files checks it");
    }
    Ok(())
}

fn bundle_linux(root: &Path, config: &Config, version: &str, icons: &Path, dist: &Path) -> Result {
    let appdir = dist.join(format!("{}.AppDir", config.app.name));
    let _ = std::fs::remove_dir_all(&appdir);
    std::fs::create_dir_all(appdir.join("usr/bin")).map_err(|error| error.to_string())?;
    let binary = root.join("target/release").join(&config.app.binary);
    std::fs::copy(&binary, appdir.join("usr/bin").join(&config.app.binary)).map_err(|error| error.to_string())?;
    std::fs::copy(icons.join("icon-512.png"), appdir.join(format!("{}.png", config.app.identifier))).map_err(|error| error.to_string())?;
    std::fs::write(
        appdir.join(format!("{}.desktop", config.app.identifier)),
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={}\nIcon={}\nCategories={}\nX-AppImage-Version={version}\n",
            config.app.name, config.app.binary, config.app.identifier, config.linux.categories
        ),
    )
    .map_err(|error| error.to_string())?;
    let app_run = appdir.join("AppRun");
    std::fs::write(&app_run, format!("#!/bin/sh\nexec \"$(dirname \"$0\")/usr/bin/{}\" \"$@\"\n", config.app.binary)).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&app_run, std::fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    }
    println!("AppDir → {}", appdir.display());
    if config.linux.appimage {
        let image = dist.join(format!("{}-{version}-x86_64.AppImage", config.app.name));
        run(Command::new("appimagetool").arg(&appdir).arg(&image))
            .map_err(|error| format!("{error} · install appimagetool from https://github.com/AppImage/appimagetool"))?;
        println!("AppImage → {}", image.display());
        publish_update(config, version, &image, "linux")?;
    }
    Ok(())
}

// ------------------------------------------------------------ licenses

/// The publisher's private key: outside the repository, readable only by you.
fn private_key_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".config/files-license/private.pk8")
}

fn read_private_key() -> Result<Vec<u8>> {
    let path = private_key_path();
    std::fs::read(&path).map_err(|_| format!("no private key at {} · run: cargo xtask license keygen", path.display()))
}

fn license(root: &Path, args: &[String]) -> Result {
    let public_keys = root.join("packaging/license/public.keys");
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = private_key_path();
            if path.exists() {
                return Err(format!("{} already exists · refusing to replace it (licenses signed with it would stop working)", path.display()));
            }
            let (private, public) = files_sdk::license::generate_keys()?;
            std::fs::create_dir_all(path.parent().expect("parent")).map_err(|error| error.to_string())?;
            std::fs::write(&path, private).map_err(|error| error.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())?;
            }
            let mut keys = std::fs::read_to_string(&public_keys).unwrap_or_default();
            keys.push_str(&format!("{public}\n"));
            std::fs::create_dir_all(public_keys.parent().expect("parent")).map_err(|error| error.to_string())?;
            std::fs::write(&public_keys, keys).map_err(|error| error.to_string())?;
            println!("Private key → {} (back it up; never commit it)", path.display());
            println!("Public key  → {} (built into the app)", public_keys.display());
            Ok(())
        }
        Some("issue") => {
            let mut email = None;
            let mut products = Vec::new();
            let mut days: Option<i64> = None;
            let mut id = None;
            let mut iter = args[1..].iter();
            while let Some(flag) = iter.next() {
                let value = || iter.clone().next().cloned().ok_or(format!("{flag} needs a value"));
                match flag.as_str() {
                    "--email" => email = Some(value()?),
                    "--products" => products = value()?.split(',').map(|product| product.trim().to_string()).filter(|product| !product.is_empty()).collect(),
                    "--days" => days = Some(value()?.parse().map_err(|_| "--days needs a number")?),
                    "--id" => id = Some(value()?),
                    other => return Err(format!("unknown flag {other}")),
                }
                iter.next();
            }
            let email = email.ok_or("--email is required")?;
            if products.is_empty() {
                return Err("--products is required (e.g. kubernetes, or * for all)".to_string());
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64);
            let license = files_sdk::license::License {
                id: id.unwrap_or_else(|| format!("L-{now}")),
                email,
                products,
                issued: now,
                expires: days.map(|days| now + days * 86_400),
            };
            let private = read_private_key()?;
            let token = files_sdk::license::issue(&license, &private)?;
            // Self-check against the keys the app trusts.
            let keys = std::fs::read_to_string(&public_keys).unwrap_or_default();
            let trusted: Vec<&str> = keys.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
            files_sdk::license::verify(&token, &trusted, now).map_err(|error| format!("the new key does not verify: {error} (is this private key's public key in packaging/license/public.keys?)"))?;
            println!("{token}");
            Ok(())
        }
        Some("sign-package") => {
            let file = args.get(1).ok_or("sign-package needs a file")?;
            let bytes = std::fs::read(file).map_err(|error| format!("{file}: {error}"))?;
            let signature = files_sdk::package::sign(&bytes, &read_private_key()?)?;
            let target = format!("{file}.sig");
            std::fs::write(&target, &signature).map_err(|error| error.to_string())?;
            println!("{} · signature → {target}", files_sdk::package::digest(&bytes));
            Ok(())
        }
        _ => Err("usage: cargo xtask license <keygen | issue --email E --products p1,p2 [--days N] [--id ORDER] | sign-package FILE>".to_string()),
    }
}

// ------------------------------------------------------------ plugins

/// Builds a plugin crate (release), reads its manifest, packs and signs it:
/// `target/dist/extensions/<id>-<version>.filesext` + `.sig`.
fn package_extension(root: &Path, args: &[String]) -> Result {
    let krate = args.first().ok_or("usage: cargo xtask package-extension <crate> [--debug]")?;
    let debug = args.iter().any(|arg| arg == "--debug");
    let mut build = Command::new(env!("CARGO"));
    build.current_dir(root).args(["build", "--package", krate]);
    if !debug {
        build.arg("--release");
    }
    run(&mut build)?;
    let library = root
        .join("target")
        .join(if debug { "debug" } else { "release" })
        .join(format!("{}{}{}", std::env::consts::DLL_PREFIX, krate.replace('-', "_"), std::env::consts::DLL_SUFFIX));
    let bytes = std::fs::read(&library).map_err(|error| format!("{}: {error} (is it a cdylib?)", library.display()))?;
    // SAFETY: our own freshly built plugin; only its manifest export is called.
    let manifest: files_sdk::plugin::PluginManifest = unsafe {
        let loaded = libloading::Library::new(&library).map_err(|error| error.to_string())?;
        let manifest: libloading::Symbol<files_sdk::plugin::abi::ManifestJson> =
            loaded.get(b"files_plugin_manifest").map_err(|_| "no files_plugin_manifest: add files_sdk::export_plugin!(…)".to_string())?;
        let free: libloading::Symbol<files_sdk::plugin::abi::Free> = loaded.get(b"files_plugin_free").map_err(|error| error.to_string())?;
        let raw = manifest();
        let json = files_sdk::plugin::abi::read(raw);
        free(raw);
        serde_json::from_str(&json).map_err(|error| format!("bad manifest: {error}"))?
    };
    let package = files_sdk::plugin::pack(&manifest, &bytes)?;
    let signature = files_sdk::package::sign(&package, &read_private_key()?)?;
    let dist = root.join("target/dist/extensions");
    std::fs::create_dir_all(&dist).map_err(|error| error.to_string())?;
    let target = dist.join(format!("{}-{}.filesext", manifest.id, manifest.version));
    std::fs::write(&target, &package).map_err(|error| error.to_string())?;
    std::fs::write(dist.join(format!("{}-{}.filesext.sig", manifest.id, manifest.version)), &signature).map_err(|error| error.to_string())?;
    println!("{} {} ({}) → {}", manifest.name, manifest.version, manifest.product.as_deref().map_or("free".to_string(), |product| format!("Pro: {product}")), target.display());
    println!("Install: Settings → Extensions → Pro extensions → paste this path (the .sig must sit next to it)");
    Ok(())
}

// ------------------------------------------------------------ doctor

/// What stands between this checkout and a published release.
fn doctor(root: &Path, config: &Config) -> Result {
    let mut missing = 0;
    let mut check = |ok: bool, what: &str, fix: &str| {
        println!("{} {what}", if ok { "✓" } else { "✗" });
        if !ok {
            missing += 1;
            println!("    → {fix}");
        }
    };
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    check(
        manifest.contains("repository = \"https://github.com/"),
        "Cargo.toml has a GitHub repository (update checks use its releases)",
        "add `repository = \"https://github.com/OWNER/REPO\"` under [package]",
    );
    check(root.join("LICENSE").exists() || root.join("LICENSE-MIT").exists(), "a LICENSE file", "add LICENSE (the crates say MIT)");
    check(root.join(&config.app.icon).exists(), "the icon source", &format!("add {}", config.app.icon));
    let keys = std::fs::read_to_string(root.join("packaging/license/public.keys")).unwrap_or_default();
    let private = std::fs::read(private_key_path()).ok().and_then(|key| files_sdk::license::public_key_of(&key).ok());
    check(
        private.as_ref().is_some_and(|public| keys.lines().any(|line| line.trim() == public)),
        "a license signing key whose public key ships in the app",
        "cargo xtask license keygen (keep ~/.config/files-license/private.pk8 backed up and secret)",
    );
    check(config.app.identifier.contains('.') && !config.app.identifier.ends_with('.'), "a reverse-DNS identifier", "set [app] identifier");
    if cfg!(target_os = "macos") {
        let identities = output(Command::new("security").args(["find-identity", "-v", "-p", "codesigning"])).unwrap_or_default();
        let developer_id = identities.lines().find(|line| line.contains("Developer ID Application")).map(str::trim);
        check(
            developer_id.is_some(),
            "a \"Developer ID Application\" certificate (needed to run on other Macs)",
            "create it at developer.apple.com → Certificates (an \"Apple Development\" one only works on your Macs)",
        );
        check(
            config.macos.signing_identity.starts_with("Developer ID Application"),
            "Bundle.toml signs with that certificate",
            &format!(
                "set [macos] signing_identity = \"{}\" (or FILES_SIGNING_IDENTITY)",
                developer_id.and_then(|line| line.split('"').nth(1)).unwrap_or("Developer ID Application: Name (TEAMID)")
            ),
        );
        let profile = output(Command::new("xcrun").args(["notarytool", "history", "--keychain-profile", &config.macos.notary_profile])).is_some();
        check(
            profile,
            &format!("a notarytool Keychain profile \"{}\"", config.macos.notary_profile),
            &format!("xcrun notarytool store-credentials {} --apple-id YOU@MAIL --team-id TEAMID", config.macos.notary_profile),
        );
        check(config.macos.notarize, "notarization is on", "set [macos] notarize = true (or pass --notarize)");
    }
    if cfg!(target_os = "linux") {
        check(output(Command::new("appimagetool").arg("--version")).is_some(), "appimagetool", "https://github.com/AppImage/appimagetool");
    }
    println!();
    if missing == 0 {
        println!("Ready: cargo xtask bundle");
    } else {
        println!("{missing} thing(s) left before publishing. A local, ad-hoc signed build works now: cargo xtask bundle");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_parses_and_the_icon_renders() {
        let (root, config) = load().unwrap();
        assert_eq!(config.app.name, "Files");
        let plist = info_plist(&config, "1.2.3");
        assert!(plist.contains("<string>dev.fileforce.files</string>") && plist.contains("<string>1.2.3</string>"));
        let svg = std::fs::read(root.join(&config.app.icon)).unwrap();
        let target = std::env::temp_dir().join(format!("xtask-icon-{}.png", std::process::id()));
        render(&svg, 64, &target).unwrap();
        let png = std::fs::read(&target).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let _ = std::fs::remove_file(&target);
    }
}
