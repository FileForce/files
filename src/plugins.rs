//! Pro extensions that ship outside the app as signed plugin packages
//! (`files_sdk::plugin`). Nothing of a package runs until its signature is
//! the publisher's and, for a Pro plugin, the license covers it: an
//! unlicensed plugin is never loaded into memory.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use files_sdk::plugin::{PluginManifest, abi};
use files_sdk::{Accessibility, Context, Icon, Manifest, MenuItem, Outcome, Permission, Scope, Targets, Tier};

/// A plugin loaded into Files.
pub struct LoadedPlugin {
    // Kept alive as long as the app: the function pointers point into it.
    _library: libloading::Library,
    manifest: &'static Manifest,
    run: abi::Run,
    free: abi::Free,
}

impl files_sdk::Extension for LoadedPlugin {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }

    fn run(&self, command: &str, context: &Context) -> std::io::Result<Outcome> {
        let command = std::ffi::CString::new(command).map_err(std::io::Error::other)?;
        let context = std::ffi::CString::new(serde_json::to_string(context).map_err(std::io::Error::other)?).map_err(std::io::Error::other)?;
        // SAFETY: the library exports these with the signatures in `abi`
        // (checked by name and ABI version at load); strings are NUL-terminated
        // and the reply is freed by the library that allocated it.
        let reply = unsafe {
            let raw = (self.run)(command.as_ptr(), context.as_ptr());
            let text = abi::read(raw);
            (self.free)(raw);
            text
        };
        let reply: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
        if let Some(error) = reply.get("error").and_then(serde_json::Value::as_str) {
            return Err(std::io::Error::other(error.to_string()));
        }
        serde_json::from_value(reply.get("ok").cloned().unwrap_or_default()).map_err(std::io::Error::other)
    }
}

/// Where installed packages live: `<settings folder>/extensions`.
pub fn folder() -> Option<PathBuf> {
    Some(crate::settings::settings_path()?.parent()?.join("extensions"))
}

/// An installed package, as Settings lists it (read without loading it).
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loaded,
    /// Signed, but the license does not cover it.
    Locked,
    /// Not signed by the publisher (or changed since): never loaded.
    Rejected(String),
}

fn loaded() -> &'static Mutex<Vec<&'static LoadedPlugin>> {
    static LOADED: OnceLock<Mutex<Vec<&'static LoadedPlugin>>> = OnceLock::new();
    LOADED.get_or_init(Default::default)
}

/// Plugins loaded so far.
pub fn registry() -> Vec<&'static dyn files_sdk::Extension> {
    let removed = removed().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    loaded()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|plugin| !removed.contains(plugin.manifest.id))
        .map(|plugin| *plugin as &dyn files_sdk::Extension)
        .collect()
}

/// Uninstalled this session: a loaded library cannot be unloaded safely, so
/// it is taken out of the registry at once and gone after a restart.
fn removed() -> &'static Mutex<std::collections::HashSet<String>> {
    static REMOVED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    REMOVED.get_or_init(Default::default)
}

/// Deletes an installed package (and its signature). Its commands stop now.
pub fn uninstall(path: &Path) -> Result<(), String> {
    let id = files_sdk::plugin::unpack(&std::fs::read(path).map_err(|error| error.to_string())?).map(|(manifest, _)| manifest.id).ok();
    std::fs::remove_file(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let _ = std::fs::remove_file(signature_path(path));
    if let Some(id) = id {
        files_sdk::log::record("files", files_sdk::log::Kind::Fs, format!("Uninstalled extension {id}"), Some(path.display().to_string()));
        removed().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(id);
    }
    Ok(())
}

fn is_loaded(id: &str) -> bool {
    loaded().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().any(|plugin| plugin.manifest.id == id)
}

/// Reads and checks one package: signature, format, ABI.
fn read_package(path: &Path, keys: &[&str]) -> Result<(PluginManifest, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let signature = std::fs::read_to_string(signature_path(path)).map_err(|_| "no signature (.sig) next to it".to_string())?;
    if !files_sdk::package::verify(&bytes, &signature, keys) {
        return Err("not signed by the Files publisher, or changed after signing".to_string());
    }
    let (manifest, library) = files_sdk::plugin::unpack(&bytes)?;
    Ok((manifest, library.to_vec()))
}

fn signature_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// Every package in `folder`, loading those that are signed and licensed.
/// `allowed` says whether the license covers a product.
pub fn load_all(folder: &Path, keys: &[&str], allowed: &dyn Fn(&str) -> bool) -> Vec<Installed> {
    let mut installed = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return installed;
    };
    let mut paths: Vec<PathBuf> =
        entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "filesext")).collect();
    paths.sort();
    for path in paths {
        let (manifest, library) = match read_package(&path, keys) {
            Ok(read) => read,
            Err(reason) => {
                files_sdk::log::record("files", files_sdk::log::Kind::Error, format!("Extension refused: {}", path.display()), Some(reason.clone()));
                let name = path.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                installed.push(Installed { manifest: placeholder(&name), path, status: Status::Rejected(reason) });
                continue;
            }
        };
        // Installed again after being removed this session: usable again.
        removed().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&manifest.id);
        let licensed = manifest.product.as_deref().is_none_or(allowed);
        let status = if !licensed {
            Status::Locked
        } else if is_loaded(&manifest.id) {
            Status::Loaded
        } else {
            match load(folder, &manifest, &library) {
                Ok(()) => Status::Loaded,
                Err(reason) => Status::Rejected(reason),
            }
        };
        installed.push(Installed { manifest, path, status });
    }
    installed
}

fn placeholder(name: &str) -> PluginManifest {
    PluginManifest {
        id: name.to_string(),
        name: name.to_string(),
        version: "?".into(),
        summary: String::new(),
        publisher: "?".into(),
        product: None,
        permissions: Vec::new(),
        menu: Vec::new(),
        menu_color: None,
        abi: files_sdk::plugin::ABI_VERSION,
    }
}

/// Writes the verified library to a private file, re-checks it, loads it.
fn load(folder: &Path, manifest: &PluginManifest, library: &[u8]) -> Result<(), String> {
    let cache = folder.join(".loaded");
    std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700));
    }
    let digest = files_sdk::package::digest(library);
    let file = cache.join(format!("{}-{}.{}", manifest.id, &digest[7..19], std::env::consts::DLL_EXTENSION));
    std::fs::write(&file, library).map_err(|error| error.to_string())?;
    // Nothing may swap the file between the check and the load.
    if std::fs::read(&file).map(|bytes| files_sdk::package::digest(&bytes)).ok().as_deref() != Some(digest.as_str()) {
        return Err("the library changed on disk before loading".to_string());
    }
    // SAFETY: the library is the publisher-signed one just verified; its
    // exports are looked up by name and their ABI version checked first.
    let plugin = unsafe {
        let library = libloading::Library::new(&file).map_err(|error| error.to_string())?;
        let abi_version: libloading::Symbol<abi::AbiVersion> = library.get(b"files_plugin_abi").map_err(|error| error.to_string())?;
        if abi_version() != files_sdk::plugin::ABI_VERSION {
            return Err("the library speaks another extension ABI".to_string());
        }
        let run = *library.get::<abi::Run>(b"files_plugin_run").map_err(|error| error.to_string())?;
        let free = *library.get::<abi::Free>(b"files_plugin_free").map_err(|error| error.to_string())?;
        LoadedPlugin { _library: library, manifest: leak_manifest(manifest), run, free }
    };
    files_sdk::permissions::declare(plugin.manifest);
    files_sdk::log::record("files", files_sdk::log::Kind::Info, format!("Loaded extension {} {}", manifest.name, manifest.version), Some(digest));
    loaded().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(Box::leak(Box::new(plugin)));
    Ok(())
}

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_string().into_boxed_str())
}

/// The host-side manifest of a plugin (kept for the app's lifetime).
fn leak_manifest(manifest: &PluginManifest) -> &'static Manifest {
    let menu: Vec<MenuItem> = manifest
        .menu
        .iter()
        .map(|item| {
            let targets = item.targets.iter().fold(None::<Targets>, |all, target| {
                let target = match target.as_str() {
                    "file" => Targets::FILE,
                    "folder" => Targets::FOLDER,
                    _ => Targets::BACKGROUND,
                };
                Some(all.map_or(target, |all| all | target))
            });
            MenuItem {
                command: leak(&item.command),
                title: leak(&item.title),
                icon: None,
                targets: targets.unwrap_or(Targets::ALL),
                scope: Scope::Anywhere,
                accessibility: Accessibility { label: None, hint: None },
            }
        })
        .collect();
    let adds: Vec<&'static str> = manifest.menu.iter().map(|item| leak(&format!("Context menu → {}", item.title))).collect();
    let permissions: Vec<Permission> = manifest.permissions.iter().filter_map(|key| Permission::from_key(key)).collect();
    Box::leak(Box::new(Manifest {
        id: leak(&manifest.id),
        name: leak(&manifest.name),
        version: leak(&manifest.version),
        source: leak(&format!("{} · plugin", manifest.publisher)),
        summary: leak(&manifest.summary),
        icon: Icon::Builtin("puzzle"),
        adds: Box::leak(adds.into_boxed_slice()),
        permissions: Box::leak(permissions.into_boxed_slice()),
        commands: &[],
        menu: Box::leak(menu.into_boxed_slice()),
        sidebar: &[],
        menu_color: manifest.menu_color.as_deref().map(leak),
        tier: match &manifest.product {
            Some(product) => Tier::Pro { product: leak(product) },
            None => Tier::Free,
        },
    }))
}

/// Installs a package from a local path or an `https://` URL (its `.sig` is
/// fetched from the same place). Refused unless signed by the publisher.
pub fn install(source: &str, folder: &Path, keys: &[&str]) -> Result<PluginManifest, String> {
    let source = source.trim();
    let (bytes, signature) = if source.starts_with("https://") {
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut curl = std::process::Command::new("curl");
            curl.args(["--fail", "--silent", "--show-error", "--location", url]);
            let output = files_sdk::log::run_with_timeout("files", &mut curl, std::time::Duration::from_secs(120)).map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(output.stdout)
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        };
        (fetch(source)?, String::from_utf8_lossy(&fetch(&format!("{source}.sig"))?).into_owned())
    } else {
        let path = Path::new(source);
        let bytes = std::fs::read(path).map_err(|error| format!("{source}: {error}"))?;
        let signature = std::fs::read_to_string(signature_path(path)).map_err(|_| format!("no {source}.sig next to the package"))?;
        (bytes, signature)
    };
    if !files_sdk::package::verify(&bytes, &signature, keys) {
        return Err("This package is not signed by the Files publisher: not installed".to_string());
    }
    let (manifest, _) = files_sdk::plugin::unpack(&bytes)?;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let target = folder.join(format!("{}.filesext", manifest.id));
    std::fs::write(&target, &bytes).map_err(|error| error.to_string())?;
    std::fs::write(signature_path(&target), signature.trim()).map_err(|error| error.to_string())?;
    files_sdk::log::record("files", files_sdk::log::Kind::Fs, format!("Installed extension {} {}", manifest.name, manifest.version), Some(target.display().to_string()));
    Ok(manifest)
}

/// Loads what the current license allows from the extensions folder.
pub fn load_installed(settings: &crate::settings::AppSettings) -> Vec<Installed> {
    let Some(folder) = folder() else { return Vec::new() };
    let license = crate::licensing::status(settings).and_then(Result::ok);
    let keys = crate::licensing::trusted();
    load_all(&folder, &keys, &|product| license.as_ref().is_some_and(|license| license.covers(product)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the example plugin and returns its library.
    fn hello_pro_library() -> Vec<u8> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let status = std::process::Command::new(env!("CARGO"))
            .current_dir(root)
            .args(["build", "--quiet", "--package", "files-ext-hello-pro"])
            .status()
            .unwrap();
        assert!(status.success(), "the example plugin did not build");
        let name = format!("{}files_ext_hello_pro{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
        std::fs::read(root.join("target/debug").join(name)).unwrap()
    }

    #[test]
    fn signed_licensed_plugins_load_and_others_never_do() {
        let (private, public) = files_sdk::license::generate_keys().unwrap();
        let keys = [public.as_str()];
        let manifest = PluginManifest {
            id: "hello-pro".into(),
            name: "Hello Pro".into(),
            version: "1.0.0".into(),
            summary: "Example".into(),
            publisher: "Files".into(),
            product: Some("hello-pro".into()),
            permissions: vec![],
            menu: vec![files_sdk::plugin::PluginMenuItem { command: "hello-pro.greet".into(), title: "Hello from Pro".into(), targets: vec!["file".into()] }],
            menu_color: Some("pink".into()),
            abi: files_sdk::plugin::ABI_VERSION,
        };
        let package = files_sdk::plugin::pack(&manifest, &hello_pro_library()).unwrap();
        let dir = std::env::temp_dir().join(format!("file-force-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let source = dir.join("download");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("hello-pro.filesext");
        std::fs::write(&file, &package).unwrap();
        std::fs::write(signature_path(&file), files_sdk::package::sign(&package, &private).unwrap()).unwrap();

        // A package signed by someone else is not even installed.
        let (stranger, _) = files_sdk::license::generate_keys().unwrap();
        let forged = source.join("forged.filesext");
        std::fs::write(&forged, &package).unwrap();
        std::fs::write(signature_path(&forged), files_sdk::package::sign(&package, &stranger).unwrap()).unwrap();
        let installed = dir.join("extensions");
        assert!(install(&forged.display().to_string(), &installed, &keys).unwrap_err().contains("not signed"));

        install(&file.display().to_string(), &installed, &keys).unwrap();

        // No license: listed as locked, never loaded.
        let listed = load_all(&installed, &keys, &|_| false);
        assert_eq!(listed[0].status, Status::Locked);
        assert!(!registry().iter().any(|plugin| plugin.manifest().id == "hello-pro"));

        // Changed after signing (one byte): refused, even with a license.
        let tampered = dir.join("tampered");
        std::fs::create_dir_all(&tampered).unwrap();
        let mut patched = package.clone();
        let last = patched.len() - 1;
        patched[last] ^= 1;
        std::fs::write(tampered.join("hello-pro.filesext"), &patched).unwrap();
        std::fs::copy(signature_path(&installed.join("hello-pro.filesext")), signature_path(&tampered.join("hello-pro.filesext"))).unwrap();
        assert!(matches!(load_all(&tampered, &keys, &|_| true)[0].status, Status::Rejected(_)));
        assert!(!registry().iter().any(|plugin| plugin.manifest().id == "hello-pro"));

        // Licensed: loaded, and its command runs inside the plugin.
        let listed = load_all(&installed, &keys, &|product| product == "hello-pro");
        assert_eq!(listed[0].status, Status::Loaded);
        let plugin = registry().into_iter().find(|plugin| plugin.manifest().id == "hello-pro").expect("not registered");
        assert_eq!(plugin.manifest().tier, Tier::Pro { product: "hello-pro" });
        assert_eq!(plugin.manifest().menu[0].title, "Hello from Pro");
        let context = Context { selection: vec!["a".into(), "b".into()], folder: Some("/work".into()), ..Default::default() };
        let outcome = plugin.run("hello-pro.greet", &context).unwrap();
        assert_eq!(outcome.notice.as_deref(), Some("Hello from Pro · 2 item(s) in /work"));
        assert!(plugin.run("nope", &context).unwrap_err().to_string().contains("unknown command"));

        // Uninstall: files gone, and its commands stop at once.
        uninstall(&installed.join("hello-pro.filesext")).unwrap();
        assert!(!installed.join("hello-pro.filesext").exists() && !installed.join("hello-pro.filesext.sig").exists());
        assert!(!registry().iter().any(|plugin| plugin.manifest().id == "hello-pro"), "an uninstalled plugin still runs");
        // Installing it again brings it back.
        install(&file.display().to_string(), &installed, &keys).unwrap();
        load_all(&installed, &keys, &|product| product == "hello-pro");
        assert!(registry().iter().any(|plugin| plugin.manifest().id == "hello-pro"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
