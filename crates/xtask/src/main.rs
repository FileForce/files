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
    #[serde(default)]
    windows: Windows,
}

#[derive(Deserialize)]
#[serde(default)]
struct Windows {
    msi: bool,
    manufacturer: String,
    upgrade_code: String,
    timestamp_url: String,
}

impl Default for Windows {
    fn default() -> Self {
        Self { msi: true, manufacturer: String::new(), upgrade_code: String::new(), timestamp_url: "http://timestamp.digicert.com".into() }
    }
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
        // The .ico build.rs embeds into the Windows exe (taskbar, window, Explorer).
        Some("windows-icon") => load().and_then(|(root, config)| windows_icon(&root, &config, &root.join("packaging/windows/icon.ico"))),
        Some("bundle") => load().and_then(|(root, config)| bundle(&root, config, &args[1..])),
        Some("doctor") => load().and_then(|(root, config)| doctor(&root, &config)),
        Some("license") => license(&workspace_root(), &args[1..]),
        Some("package-extension") => package_extension(&workspace_root(), &args[1..]),
        Some("updates") => updates(&workspace_root(), &args[1..]),
        // Copies the bundled VLC into a folder, to try the plugin list: `cargo xtask vlc DIR`.
        Some("vlc") => match args.get(1) {
            Some(target) => vlc_source().ok_or_else(|| "VLC not found: set FILES_VLC_DIR".to_string()).and_then(|source| copy_vlc(&source, Path::new(target))),
            None => Err("usage: cargo xtask vlc TARGET_DIR".to_string()),
        },
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
    } else if cfg!(windows) {
        bundle_windows(root, &config, &version, &icons, &dist)
    } else {
        Err("bundling is set up for macOS, Linux and Windows".to_string())
    }
}

/// An .ico holding PNG images (Windows Vista and later read them as is).
fn write_ico(pngs: &[(u32, Vec<u8>)], target: &Path) -> Result {
    let mut ico = Vec::new();
    ico.extend([0u8, 0, 1, 0]);
    ico.extend((pngs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (size, png) in pngs {
        // 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend([side, side, 0, 0]);
        ico.extend(1u16.to_le_bytes()); // colour planes
        ico.extend(32u16.to_le_bytes()); // bits per pixel
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in pngs {
        ico.extend(png);
    }
    std::fs::write(target, ico).map_err(|error| format!("{}: {error}", target.display()))
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The WiX v4+ source for the installer: Program Files, a Start menu entry,
/// in-place upgrades.
fn wix_source(config: &Config, version: &str, exe: &str) -> String {
    let name = escape_xml(&config.app.name);
    format!(
        r#"<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="{name}" Manufacturer="{manufacturer}" Version="{version}" UpgradeCode="{upgrade}" Scope="perMachine">
    <MajorUpgrade DowngradeErrorMessage="A newer version of {name} is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Icon Id="AppIcon" SourceFile="icon.ico" />
    <Property Id="ARPPRODUCTICON" Value="AppIcon" />
    <Property Id="ARPURLINFOABOUT" Value="{homepage}" />
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="{name}">
        <Component Id="MainExecutable">
          <File Id="AppExe" Source="{exe}" KeyPath="yes" />
        </Component>
        <Directory Id="VLCFOLDER" Name="vlc" />
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder">
      <Component Id="StartMenuShortcut">
        <Shortcut Id="StartMenu" Name="{name}" Target="[INSTALLFOLDER]{exe}" WorkingDirectory="INSTALLFOLDER">
          <!-- Toasts and the taskbar know the app by this id (the app sets the same one). -->
          <ShortcutProperty Key="System.AppUserModel.ID" Value="{identifier}" />
        </Shortcut>
        <RegistryValue Root="HKLM" Key="Software\{manufacturer}\{name}" Name="installed" Type="integer" Value="1" KeyPath="yes" />
        <!-- "Open folders in Files" (Settings) points Explorer at the exe: gone with it. -->
        <RemoveRegistryKey Root="HKCU" Key="Software\Classes\Directory\shell\openinfiles" Action="removeOnUninstall" />
        <RemoveRegistryKey Root="HKCU" Key="Software\Classes\Drive\shell\openinfiles" Action="removeOnUninstall" />
      </Component>
    </StandardDirectory>
    <Feature Id="Main">
      <ComponentRef Id="MainExecutable" />
      <ComponentRef Id="StartMenuShortcut" />
      <Files Directory="VLCFOLDER" Include="vlc\**" />
    </Feature>
  </Package>
</Wix>
"#,
        manufacturer = escape_xml(&config.windows.manufacturer),
        upgrade = config.windows.upgrade_code,
        identifier = escape_xml(&config.app.identifier),
        homepage = "https://github.com/FileForce/files",
    )
}

// ------------------------------------------------------------------- VLC

/// Plugins video preview needs (by name, without `lib…_plugin`): reading
/// files, the common containers and codecs, picture handed to the app
/// (vmem), sound out, and the converters in between. About 20 MB of VLC's 80.
const VLC_PLUGINS: &[&str] = &[
    // access and containers (http/https/tls: music streamed while it downloads)
    "http", "https", "gnutls", "securetransport", "prefetch", "cache_read",
    "filesystem", "mp4", "mkv", "avi", "es", "ps", "ts", "ogg", "wav", "avformat", "h26x", "mpgv",
    // decoders (hardware first)
    "avcodec", "videotoolbox", "d3d11va", "dxva2", "faad", "mpg123", "vorbis", "opus", "flac", "araw", "lpcm", "a52", "dts",
    // packetizers
    "packetizer_h264", "packetizer_hevc", "packetizer_mpeg4video", "packetizer_mpeg4audio", "packetizer_mpegaudio",
    "packetizer_mpegvideo", "packetizer_av1", "packetizer_vc1", "packetizer_mlp", "packetizer_flac", "packetizer_a52",
    "packetizer_dts", "packetizer_copy",
    // picture to the app, and pixel format conversion
    "vmem", "swscale", "chroma_copy", "i420_rgb", "i420_yuy2", "i422_i420", "i422_yuy2", "yuy2_i420", "yuy2_i422",
    "rv32", "grey_yuv", "yuvp", "cvpx", "chain", "deinterlace",
    // sound out and conversion
    "auhal", "mmdevice", "wasapi", "directsound", "waveout", "pulse", "alsa", "audio_format", "float_mixer", "integer_mixer",
    "ugly_resampler", "speex_resampler", "soxr", "samplerate", "simple_channel_mixer", "trivial_channel_mixer", "remap",
    "scaletempo", "audio_volume", "equalizer",
    // misc that playback asks for
    "console_logger", "xml", "inflate", "stats",
];

/// The VLC to copy from: FILES_VLC_DIR, or the installed one. On Linux the
/// distribution's (`/usr/lib/x86_64-linux-gnu` with `vlc/plugins`).
fn vlc_source() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("FILES_VLC_DIR").map(PathBuf::from),
        cfg!(target_os = "macos").then(|| PathBuf::from("/Applications/VLC.app/Contents/MacOS")),
        cfg!(windows).then(|| PathBuf::from(r"C:\Program Files\VideoLAN\VLC")),
        cfg!(target_os = "linux").then(|| PathBuf::from("/usr/lib/x86_64-linux-gnu")),
    ];
    candidates.into_iter().flatten().find(|dir| plugins_of(dir).is_dir())
}

/// Where a VLC keeps its plugins.
fn plugins_of(source: &Path) -> PathBuf {
    if cfg!(target_os = "linux") { source.join("vlc").join("plugins") } else { source.join("plugins") }
}

fn plugin_wanted(file_name: &str) -> bool {
    let stem = file_name.split('.').next().unwrap_or_default();
    let name = stem.strip_prefix("lib").unwrap_or(stem).strip_suffix("_plugin").unwrap_or(stem);
    VLC_PLUGINS.contains(&name)
}

fn copy_plugins(from: &Path, to: &Path, copied: &mut usize) -> Result {
    for entry in std::fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            copy_plugins(&path, &to.join(&name), copied)?;
        } else if plugin_wanted(&name) {
            std::fs::create_dir_all(to).map_err(|error| error.to_string())?;
            std::fs::copy(&path, to.join(&name)).map_err(|error| format!("{name}: {error}"))?;
            *copied += 1;
        }
    }
    Ok(())
}

/// libvlc, libvlccore and the preview plugins into `target`:
/// `lib/` + `plugins/` on macOS, the DLLs + `plugins\` on Windows.
fn copy_vlc(source: &Path, target: &Path) -> Result {
    let _ = std::fs::remove_dir_all(target);
    let (libraries, lib_dir, library_source): (&[&str], PathBuf, PathBuf) = if cfg!(target_os = "macos") {
        (&["libvlc.dylib", "libvlc.5.dylib", "libvlccore.dylib", "libvlccore.9.dylib"], target.join("lib"), source.join("lib"))
    } else if cfg!(windows) {
        (&["libvlc.dll", "libvlccore.dll"], target.to_path_buf(), source.to_path_buf())
    } else {
        (&["libvlc.so.5", "libvlccore.so.9"], target.join("lib"), source.to_path_buf())
    };
    std::fs::create_dir_all(&lib_dir).map_err(|error| error.to_string())?;
    for library in libraries {
        let from = library_source.join(library);
        if from.exists() {
            // `fs::copy` follows links: the real file under the soname.
            std::fs::copy(&from, lib_dir.join(library)).map_err(|error| format!("{library}: {error}"))?;
        }
    }
    let mut copied = 0;
    copy_plugins(&plugins_of(source), &target.join("plugins"), &mut copied)?;
    #[cfg(target_os = "linux")]
    copy_linux_dependencies(target, &lib_dir)?;
    // VLC is LGPL: its notice travels with it.
    let notice = "This app includes libVLC from VideoLAN (https://www.videolan.org), licensed under the GNU LGPL 2.1 or later.\n\
                  Source code: https://code.videolan.org/videolan/vlc\n";
    std::fs::write(target.join("VLC-LICENSE.txt"), notice).map_err(|error| error.to_string())?;
    println!("VLC → {} ({copied} plugins)", target.display());
    Ok(())
}

/// Libraries every Linux desktop has: taken from the system, never bundled
/// (the usual AppImage exclude list).
#[cfg(target_os = "linux")]
const SYSTEM_LIBRARIES: &[&str] = &[
    "linux-vdso", "ld-linux", "libc.so", "libm.so", "libdl.so", "libpthread.so", "librt.so", "libresolv.so", "libutil.so",
    "libgcc_s.so", "libstdc++.so", "libasound.so", "libGL", "libEGL", "libGLX", "libGLdispatch", "libX11", "libxcb", "libdrm",
    "libgbm", "libfontconfig", "libfreetype", "libharfbuzz", "libz.so", "libexpat", "libdbus-1", "libsystemd", "libudev",
    "libwayland", "libvulkan",
];

/// Copies what VLC and its plugins link against (except the system's), so
/// the AppImage plays on distributions without VLC; then checks with `ldd`
/// that nothing is missing.
#[cfg(target_os = "linux")]
fn copy_linux_dependencies(target: &Path, lib_dir: &Path) -> Result {
    fn objects(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                objects(&path, found);
            } else if path.to_string_lossy().contains(".so") {
                found.push(path);
            }
        }
    }
    let ldd = |object: &Path| output(Command::new("ldd").arg(object).env("LD_LIBRARY_PATH", lib_dir)).unwrap_or_default();
    let mut queue = Vec::new();
    objects(target, &mut queue);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(object) = queue.pop() {
        for line in ldd(&object).lines() {
            // "libavcodec.so.60 => /lib/x86_64-linux-gnu/libavcodec.so.60 (0x…)"
            let Some((name, rest)) = line.trim().split_once(" => ") else { continue };
            let Some(path) = rest.split_whitespace().next().filter(|path| path.starts_with('/')) else { continue };
            if SYSTEM_LIBRARIES.iter().any(|system| name.starts_with(system)) || !seen.insert(name.to_string()) {
                continue;
            }
            let copy = lib_dir.join(name);
            if !copy.exists() {
                std::fs::copy(path, &copy).map_err(|error| format!("{name}: {error}"))?;
                queue.push(copy);
            }
        }
    }
    let mut all = Vec::new();
    objects(target, &mut all);
    let missing: Vec<String> = all
        .iter()
        .flat_map(|object| ldd(object).lines().filter(|line| line.contains("not found")).map(|line| format!("{}: {}", object.display(), line.trim())).collect::<Vec<_>>())
        .collect();
    if !missing.is_empty() {
        return Err(format!("bundled VLC cannot load:\n{}", missing.join("\n")));
    }
    println!("VLC dependencies → {} libraries", seen.len());
    Ok(())
}

/// The bundled VLC: required in CI (FILES_REQUIRE_VLC=1), optional locally.
fn bundle_vlc(target: &Path) -> Result {
    match vlc_source() {
        Some(source) => copy_vlc(&source, target),
        None if std::env::var_os("FILES_REQUIRE_VLC").is_some() => Err("VLC not found to bundle (install it or set FILES_VLC_DIR)".to_string()),
        None => {
            println!("VLC not found: videos will play only where VLC is installed");
            Ok(())
        }
    }
}

/// The app icon as a multi-size .ico.
fn windows_icon(root: &Path, config: &Config, target: &Path) -> Result {
    let scratch = std::env::temp_dir().join(format!("xtask-ico-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let svg = std::fs::read(root.join(&config.app.icon)).map_err(|error| error.to_string())?;
    let pngs = [16, 24, 32, 48, 64, 256]
        .into_iter()
        .map(|size| {
            let path = scratch.join(format!("ico-{size}.png"));
            render(&svg, size, &path)?;
            Ok((size, std::fs::read(&path).map_err(|error| error.to_string())?))
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_ico(&pngs, target)?;
    let _ = std::fs::remove_dir_all(&scratch);
    println!("Icon → {}", target.display());
    Ok(())
}

fn bundle_windows(root: &Path, config: &Config, version: &str, icons: &Path, dist: &Path) -> Result {
    if config.windows.upgrade_code.is_empty() {
        return Err("set [windows] upgrade_code in Bundle.toml (a GUID that never changes)".to_string());
    }
    let staging = dist.join("msi");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let exe = format!("{}.exe", config.app.binary);
    std::fs::copy(root.join("target/release").join(&exe), staging.join(&exe)).map_err(|error| format!("{exe}: {error}"))?;
    let pngs = [16, 32, 48, 256]
        .into_iter()
        .map(|size| {
            let path = icons.join(format!("ico-{size}.png"));
            let svg = std::fs::read(root.join(&config.app.icon)).map_err(|error| error.to_string())?;
            render(&svg, size, &path)?;
            Ok((size, std::fs::read(&path).map_err(|error| error.to_string())?))
        })
        .collect::<Result<Vec<_>>>()?;
    write_ico(&pngs, &staging.join("icon.ico"))?;
    sign_windows(config, &staging.join(&exe))?;
    bundle_vlc(&staging.join("vlc"))?;
    std::fs::write(staging.join("Files.wxs"), wix_source(config, version, &exe)).map_err(|error| error.to_string())?;
    if !config.windows.msi {
        return Ok(());
    }
    let msi = dist.join(format!("{}-{version}-x64.msi", config.app.name));
    run(Command::new("wix").current_dir(&staging).args(["build", "-arch", "x64", "-o"]).arg(&msi).arg("Files.wxs"))
        .map_err(|error| format!("{error} · install WiX 5: dotnet tool install --global wix --version 5.0.2"))?;
    sign_windows(config, &msi)?;
    println!("Installer → {}", msi.display());
    publish_update(config, version, &msi, "windows")
}

/// signtool with the .pfx in FILES_WINDOWS_PFX (skipped when it is not set).
fn sign_windows(config: &Config, file: &Path) -> Result {
    let Ok(pfx) = std::env::var("FILES_WINDOWS_PFX") else {
        println!("Not signing {} (no FILES_WINDOWS_PFX): SmartScreen will warn on first run", file.display());
        return Ok(());
    };
    let password = std::env::var("FILES_WINDOWS_PFX_PASSWORD").unwrap_or_default();
    run(Command::new("signtool")
        .args(["sign", "/fd", "sha256", "/td", "sha256", "/tr", &config.windows.timestamp_url, "/f", &pfx, "/p", &password])
        .arg(file))
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
    <key>NSMotionUsageDescription</key><string>Music's spatial audio follows your head with AirPods or Beats, so the sound stays put when you turn.</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
    <!-- Folders: lets Settings make Files the folder viewer instead of Finder. -->
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Folder</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key><array><string>public.folder</string><string>public.volume</string></array>
        </dict>
    </array>
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
    bundle_vlc(&contents.join("Frameworks").join("vlc"))?;
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
        #[cfg(unix)]
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
    let Ok(private) = read_update_key() else {
        println!("No update key: the update is not signed, Files will only show a download link (cargo xtask updates keygen)");
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
            "[Desktop Entry]\nType=Application\nName={}\nExec={} %U\nIcon={}\nCategories={}\nMimeType=inode/directory;\nStartupWMClass={}\nX-AppImage-Version={version}\n",
            config.app.name, config.app.binary, config.app.identifier, config.linux.categories, config.app.identifier
        ),
    )
    .map_err(|error| error.to_string())?;
    bundle_vlc(&appdir.join("usr/lib/vlc"))?;
    let app_run = appdir.join("AppRun");
    // The bundled VLC's libraries come first; the app finds libvlc itself.
    std::fs::write(
        &app_run,
        format!(
            "#!/bin/sh\nhere=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexport LD_LIBRARY_PATH=\"$here/usr/lib/vlc/lib${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}\"\nexec \"$here/usr/bin/{}\" \"$@\"\n",
            config.app.binary
        ),
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&app_run, std::fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    }
    println!("AppDir → {}", appdir.display());
    if config.linux.appimage {
        let image = dist.join(format!("{}-{version}-x86_64.AppImage", config.app.name));
        let mut tool = Command::new("appimagetool");
        // A runtime downloaded beforehand (CI): no network during the build.
        if let Some(runtime) = std::env::var_os("APPIMAGE_RUNTIME").filter(|runtime| Path::new(runtime).is_file()) {
            tool.arg("--runtime-file").arg(runtime);
        }
        run(tool.arg(&appdir).arg(&image))
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

/// The key that signs updates — separate from the license key, so the CI
/// that holds it cannot mint licenses. From FILES_UPDATE_KEY (base64, CI) or
/// `~/.config/files-license/updates.pk8`.
fn update_key_path() -> PathBuf {
    private_key_path().with_file_name("updates.pk8")
}

fn read_update_key() -> Result<Vec<u8>> {
    use base64::Engine as _;
    if let Ok(encoded) = std::env::var("FILES_UPDATE_KEY") {
        if !encoded.trim().is_empty() {
            return base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|error| format!("FILES_UPDATE_KEY: {error}"));
        }
    }
    let path = update_key_path();
    std::fs::read(&path).map_err(|_| format!("no update key at {} · run: cargo xtask updates keygen", path.display()))
}

/// `cargo xtask updates keygen`: the update signing key pair.
fn updates(root: &Path, args: &[String]) -> Result {
    use base64::Engine as _;
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = update_key_path();
            if path.exists() {
                return Err(format!("{} already exists · refusing to replace it (installed copies would refuse new updates)", path.display()));
            }
            let (private, public) = files_sdk::license::generate_keys()?;
            std::fs::create_dir_all(path.parent().expect("parent")).map_err(|error| error.to_string())?;
            std::fs::write(&path, &private).map_err(|error| error.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())?;
            }
            let keys = root.join("packaging/license/updates.keys");
            let mut text = std::fs::read_to_string(&keys).unwrap_or_default();
            text.push_str(&format!("{public}\n"));
            std::fs::write(&keys, text).map_err(|error| error.to_string())?;
            println!("Private key → {} (back it up)", path.display());
            println!("Public key  → {} (built into the app)", keys.display());
            println!("For CI: the base64 below goes into the FILES_UPDATE_KEY secret");
            println!("{}", base64::engine::general_purpose::STANDARD.encode(&private));
            Ok(())
        }
        Some("sign") => {
            let file = args.get(1).ok_or("usage: cargo xtask updates sign FILE")?;
            let bytes = std::fs::read(file).map_err(|error| format!("{file}: {error}"))?;
            let signature = files_sdk::package::sign(&bytes, &read_update_key()?)?;
            std::fs::write(format!("{file}.sig"), &signature).map_err(|error| error.to_string())?;
            println!("{file}.sig");
            Ok(())
        }
        _ => Err("usage: cargo xtask updates <keygen | sign FILE>".to_string()),
    }
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
    fn mac_bundle_can_open_folders() {
        let (_, config) = load().unwrap();
        let plist = info_plist(&config, "1.2.3");
        assert!(plist.contains("<string>public.folder</string>") && plist.contains("<string>Viewer</string>"), "{plist}");
    }

    #[test]
    fn windows_installer_source_and_icon() {
        let (root, config) = load().unwrap();
        let wxs = wix_source(&config, "1.2.3", "file-force.exe");
        assert!(wxs.contains(r#"Version="1.2.3""#) && wxs.contains(&config.windows.upgrade_code), "{wxs}");
        assert!(wxs.contains(r#"Source="file-force.exe""#) && wxs.contains("ProgramFiles64Folder") && wxs.contains("MajorUpgrade"));
        assert!(wxs.contains(r#"Key="Software\Classes\Directory\shell\openinfiles" Action="removeOnUninstall""#), "uninstalling gives folders back to Explorer");
        assert!(wxs.contains(&format!(r#"Key="System.AppUserModel.ID" Value="{}""#, config.app.identifier)), "toasts need the shortcut's AppUserModelID");
        assert!(wxs.contains(r#"<Directory Id="VLCFOLDER" Name="vlc" />"#) && wxs.contains(r#"Include="vlc\**""#), "VLC goes into the MSI");
        // The VLC plugin list matches both systems' file names.
        assert!(plugin_wanted("libavcodec_plugin.dylib") && plugin_wanted("libvmem_plugin.dll") && plugin_wanted("libmmdevice_plugin.dll"));
        assert!(!plugin_wanted("libaccess_srt_plugin.dylib") && !plugin_wanted("libqt_plugin.dll"), "players' UIs and streaming stay out");
        let svg = std::fs::read(root.join(&config.app.icon)).unwrap();
        let dir = std::env::temp_dir().join(format!("xtask-ico-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pngs: Vec<(u32, Vec<u8>)> = [16, 256]
            .into_iter()
            .map(|size| {
                render(&svg, size, &dir.join("p.png")).unwrap();
                (size, std::fs::read(dir.join("p.png")).unwrap())
            })
            .collect();
        write_ico(&pngs, &dir.join("icon.ico")).unwrap();
        let ico = std::fs::read(dir.join("icon.ico")).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0], "ICO header with two images");
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0, "256 is stored as 0");
        let first = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(&ico[first + 1..first + 4], b"PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
