//! Quick Actions: the built-in ones (Convert Image, Rotate), the user's
//! scripts (Python, Lua, JavaScript, Rust, Go, shell) kept as TOML in the
//! settings folder, and what extensions add through `files_sdk::quick_actions`.

use std::io;
use std::path::{Path, PathBuf};

use files_sdk::quick_actions::{Applies, Field, Outcome, QuickAction, QuickActionProvider, answer, fitting, sibling_with_extension};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------- built in

const IMAGES: [&str; 6] = ["png", "jpg", "jpeg", "heic", "webp", "jxl"];

/// JPEG XL is written by libjxl's cjxl (and read by djxl, or macOS).
fn jxl_tool(name: &str) -> Option<PathBuf> {
    which(name)
}

/// JPEG and JPEG XL quality for a choice: High, Medium, Low.
fn quality(choice: Option<&str>) -> u8 {
    match choice {
        Some("Low") => 60,
        Some("Medium") => 80,
        _ => 92,
    }
}

pub struct BuiltIn;

impl QuickActionProvider for BuiltIn {
    fn actions(&self) -> Vec<QuickAction> {
        vec![
            QuickAction {
                id: "convert-image".into(),
                title: "Convert Image…".into(),
                applies: Applies::to(&IMAGES),
                order: 0,
                ask: vec![
                    Field::Choice {
                        key: "format".into(),
                        label: "Convert to".into(),
                        // JPEG XL when cjxl (libjxl) is installed.
                        options: ["PNG", "JPEG", "WebP", "HEIC", "JPEG XL"].into_iter().filter(|format| *format != "JPEG XL" || jxl_tool("cjxl").is_some()).map(String::from).collect(),
                    },
                    Field::Choice {
                        key: "quality".into(),
                        label: "Quality (JPEG, JPEG XL; a JPEG becomes JPEG XL losslessly at High)".into(),
                        options: ["High", "Medium", "Low"].map(String::from).to_vec(),
                    },
                ],
            },
            QuickAction {
                id: "rotate-image".into(),
                title: "Rotate Image…".into(),
                applies: Applies::to(&IMAGES),
                order: 1,
                ask: vec![
                    Field::Choice { key: "turn".into(), label: "Turn".into(), options: ["90° right", "90° left", "180°", "Flip horizontally", "Flip vertically"].map(String::from).to_vec() },
                    Field::Choice { key: "save".into(), label: "Save".into(), options: ["As a copy", "Over the original"].map(String::from).to_vec() },
                ],
            },
        ]
    }

    fn run(&self, id: &str, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome> {
        let mut made = Vec::new();
        for path in selection {
            made.push(match id {
                "convert-image" => convert_with(path, answer(answers, "format").unwrap_or("PNG"), quality(answer(answers, "quality")))?,
                "rotate-image" => rotate(path, answer(answers, "turn").unwrap_or("90° right"), answer(answers, "save") == Some("Over the original"))?,
                _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("No action {id}"))),
            });
        }
        let what = if id == "convert-image" { "Converted" } else { "Rotated" };
        Ok(Outcome { summary: format!("{what} {} image{}", made.len(), if made.len() == 1 { "" } else { "s" }), made })
    }
}

fn is_heic(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("heic") || extension.eq_ignore_ascii_case("heif"))
}

/// macOS reads and writes HEIC with sips; elsewhere it is not available.
fn sips(source: &Path, format: &str, target: &Path) -> io::Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "HEIC needs macOS"));
    }
    let output = files_sdk::log::run("quick-actions", files_sdk::log::command("/usr/bin/sips").args(["-s", "format", format]).arg(source).arg("--out").arg(target))?;
    if output.status.success() { Ok(()) } else { Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string())) }
}

/// The picture, decoded (HEIC through a temporary JPEG, JPEG XL through djxl
/// or macOS).
fn decode(path: &Path) -> io::Result<image::DynamicImage> {
    if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jxl")) {
        let temporary = std::env::temp_dir().join(format!("file-force-qa-{}-in.png", std::process::id()));
        match jxl_tool("djxl") {
            Some(djxl) => {
                let output = files_sdk::log::run("quick-actions", files_sdk::log::command(djxl).arg(path).arg(&temporary))?;
                if !output.status.success() {
                    return Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()));
                }
            }
            None => sips(path, "png", &temporary)?,
        }
        let picture = image::open(&temporary).map_err(io::Error::other);
        let _ = std::fs::remove_file(&temporary);
        return picture;
    }
    if is_heic(path) {
        let temporary = std::env::temp_dir().join(format!("file-force-qa-{}.jpg", std::process::id()));
        sips(path, "jpeg", &temporary)?;
        let picture = image::open(&temporary).map_err(io::Error::other);
        let _ = std::fs::remove_file(&temporary);
        return picture;
    }
    image::open(path).map_err(io::Error::other)
}

fn encode(picture: &image::DynamicImage, target: &Path, format: &str) -> io::Result<()> {
    encode_with(picture, target, format, 92)
}

fn encode_with(picture: &image::DynamicImage, target: &Path, format: &str, quality: u8) -> io::Result<()> {
    match format {
        "JPEG XL" => {
            let temporary = std::env::temp_dir().join(format!("file-force-qa-{}-out.png", std::process::id()));
            picture.save_with_format(&temporary, image::ImageFormat::Png).map_err(io::Error::other)?;
            let result = cjxl(&temporary, target, quality);
            let _ = std::fs::remove_file(&temporary);
            result
        }
        "JPEG" => {
            let file = std::fs::File::create(target)?;
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), quality);
            encoder.encode_image(&picture.to_rgb8()).map_err(io::Error::other)
        }
        "HEIC" => {
            let temporary = std::env::temp_dir().join(format!("file-force-qa-{}.png", std::process::id()));
            picture.save_with_format(&temporary, image::ImageFormat::Png).map_err(io::Error::other)?;
            let result = sips(&temporary, "heic", target);
            let _ = std::fs::remove_file(&temporary);
            result
        }
        "WebP" => picture.to_rgba8().save_with_format(target, image::ImageFormat::WebP).map_err(io::Error::other),
        _ => picture.save_with_format(target, image::ImageFormat::Png).map_err(io::Error::other),
    }
}

/// cjxl at a quality (100 is lossless); a JPEG source at 92 and up is
/// transcoded without loss, ~20% smaller and reversible.
fn cjxl(source: &Path, target: &Path, quality: u8) -> io::Result<()> {
    let tool = jxl_tool("cjxl").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "JPEG XL needs cjxl (libjxl): brew install jpeg-xl"))?;
    let jpeg = source.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg"));
    let mut command = files_sdk::log::command(tool);
    command.arg(source).arg(target);
    if jpeg && quality >= 92 {
        command.arg("--lossless_jpeg=1");
    } else {
        command.args(["--lossless_jpeg=0", "-q", &quality.to_string()]);
    }
    let output = files_sdk::log::run("quick-actions", &mut command)?;
    if output.status.success() { Ok(()) } else { Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string())) }
}

fn extension_for(format: &str) -> &'static str {
    match format {
        "JPEG XL" => "jxl",
        "JPEG" => "jpg",
        "WebP" => "webp",
        "HEIC" => "heic",
        _ => "png",
    }
}

/// `photo.heic` → `photo.png` beside it.
pub fn convert(path: &Path, format: &str) -> io::Result<PathBuf> {
    convert_with(path, format, 92)
}

/// The same, at a JPEG / JPEG XL quality (60…100).
pub fn convert_with(path: &Path, format: &str, quality: u8) -> io::Result<PathBuf> {
    let target = sibling_with_extension(path, extension_for(format));
    // A JPEG straight to JPEG XL: cjxl keeps its exact data (no re-encoding).
    let jpeg = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg"));
    if format == "JPEG XL" && jpeg {
        cjxl(path, &target, quality)?;
        return Ok(target);
    }
    encode_with(&decode(path)?, &target, format, quality)?;
    Ok(target)
}

/// Turns or mirrors a picture, as a copy (`photo rotated.jpg`) or in place.
pub fn rotate(path: &Path, turn: &str, replace: bool) -> io::Result<PathBuf> {
    let picture = decode(path)?;
    let turned = match turn {
        "90° left" => picture.rotate270(),
        "180°" => picture.rotate180(),
        "Flip horizontally" => picture.fliph(),
        "Flip vertically" => picture.flipv(),
        _ => picture.rotate90(),
    };
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
    let format = match extension.as_str() {
        "jpg" | "jpeg" => "JPEG",
        "webp" => "WebP",
        "heic" | "heif" => "HEIC",
        "jxl" => "JPEG XL",
        _ => "PNG",
    };
    let target = if replace {
        path.to_path_buf()
    } else {
        let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let wanted = path.with_file_name(format!("{stem} rotated.{extension}"));
        if wanted.exists() { files_sdk::fs::unique_path(&wanted) } else { wanted }
    };
    encode(&turned, &target, format)?;
    Ok(target)
}

// ------------------------------------------------------------- scripts

/// Languages a script can be written in, with the program that runs it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Python,
    Lua,
    JavaScript,
    Rust,
    Go,
    Shell,
}

impl Language {
    pub const ALL: [Language; 6] = [Language::Python, Language::Lua, Language::JavaScript, Language::Rust, Language::Go, Language::Shell];

    pub fn label(self) -> &'static str {
        match self {
            Language::Python => "Python",
            Language::Lua => "Lua",
            Language::JavaScript => "JavaScript",
            Language::Rust => "Rust",
            Language::Go => "Go",
            Language::Shell => "Shell",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Language::Python => "py",
            Language::Lua => "lua",
            Language::JavaScript => "js",
            Language::Rust => "rs",
            Language::Go => "go",
            Language::Shell => if cfg!(windows) { "cmd" } else { "sh" },
        }
    }

    fn program(self) -> &'static str {
        match self {
            Language::Python => if cfg!(windows) { "python" } else { "python3" },
            Language::Lua => "lua",
            Language::JavaScript => "node",
            Language::Rust => "rustc",
            Language::Go => "go",
            Language::Shell => if cfg!(windows) { "cmd" } else { "sh" },
        }
    }

    /// Its program is installed.
    pub fn available(self) -> bool {
        which(self.program()).is_some()
    }

    /// A starting point for a new script.
    pub fn template(self) -> &'static str {
        match self {
            Language::Python => "import libfiles\n\n# libfiles: selection(), ask(), notify(), made(), reveal(), progress().\nanswers = libfiles.ask(\"My action\", [libfiles.choice(\"mode\", \"Mode\", [\"Fast\", \"Careful\"])])\nfor path in libfiles.selection():\n    libfiles.progress(path)\n    libfiles.made(path)\nprint(\"Done · \" + answers[\"mode\"])\n",
            Language::Lua => "local files = require(\"libfiles\")\n\n-- libfiles: selection(), ask(), notify(), made(), reveal(), progress().\nlocal answers = files.ask(\"My action\", { files.choice(\"mode\", \"Mode\", { \"Fast\", \"Careful\" }) })\nif not answers then return end\nfor _, path in ipairs(files.selection()) do files.made(path) end\nprint(\"Done · \" .. answers.mode)\n",
            Language::JavaScript => "const files = require('libfiles');\n\n// libfiles: selection(), ask(), notify(), made(), reveal(), progress().\nconst answers = files.ask('My action', [files.choice('mode', 'Mode', ['Fast', 'Careful'])]);\nfor (const path of files.selection()) files.made(path);\nconsole.log(`Done · ${answers.mode}`);\n",
            Language::Rust => "// The selected files are the arguments; answers are FILES_ANSWER_<KEY>.\nfn main() {\n    for path in std::env::args().skip(1) {\n        println!(\"made: {path}\");\n    }\n    println!(\"Done\");\n}\n",
            Language::Go => "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\n// The selected files are the arguments; answers are FILES_ANSWER_<KEY>.\nfunc main() {\n\tfor _, path := range os.Args[1:] {\n\t\tfmt.Println(\"made:\", path)\n\t}\n\tfmt.Println(\"Done\")\n}\n",
            Language::Shell => if cfg!(windows) { "@echo off\r\nrem The selected files are the arguments; answers are FILES_ANSWER_<KEY>.\r\nfor %%f in (%*) do echo made: %%~f\r\necho Done\r\n" } else { "#!/bin/sh\n# The selected files are the arguments; answers are FILES_ANSWER_<KEY>.\nfor path in \"$@\"; do echo \"made: $path\"; done\necho Done\n" },
        }
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let extra: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::split_paths(&paths)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .flat_map(|dir| extra.iter().map(move |suffix| dir.join(format!("{program}{suffix}"))))
        .find(|path| path.is_file())
}

/// A question in a script's file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ask {
    pub key: String,
    pub label: String,
    /// "choice", "text" or "number".
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub default: String,
}

fn default_kind() -> String {
    "text".into()
}

/// One script, as kept in `quick-actions/<name>.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub name: String,
    pub language: Language,
    /// Extensions it is for; empty: any file.
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub folders: bool,
    #[serde(default)]
    pub min: usize,
    #[serde(default)]
    pub max: usize,
    #[serde(default = "default_order")]
    pub order: i32,
    #[serde(default)]
    pub ask: Vec<Ask>,
    pub code: String,
}

fn default_order() -> i32 {
    100
}

pub fn scripts_dir() -> Option<PathBuf> {
    Some(crate::settings::settings_path()?.parent()?.join("quick-actions"))
}

/// A script's file name, from its name.
pub fn file_name(name: &str) -> String {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
    format!("{}.toml", base.trim_matches('-'))
}

pub fn scripts() -> Vec<(PathBuf, Script)> {
    let Some(dir) = scripts_dir() else { return Vec::new() };
    let mut scripts: Vec<(PathBuf, Script)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "toml"))
        .filter_map(|path| Some((path.clone(), toml::from_str(&std::fs::read_to_string(&path).ok()?).ok()?)))
        .collect();
    scripts.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    scripts
}

pub fn save_script(script: &Script, previous: Option<&Path>) -> io::Result<PathBuf> {
    let dir = scripts_dir().ok_or_else(|| io::Error::other("No settings folder"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(&script.name));
    std::fs::write(&path, toml::to_string_pretty(script).map_err(io::Error::other)?)?;
    if let Some(previous) = previous.filter(|previous| *previous != path) {
        let _ = std::fs::remove_file(previous);
    }
    Ok(path)
}

pub struct Scripts;

impl QuickActionProvider for Scripts {
    fn actions(&self) -> Vec<QuickAction> {
        scripts()
            .into_iter()
            .map(|(path, script)| QuickAction {
                id: path.display().to_string(),
                title: script.name.clone(),
                applies: Applies { extensions: script.extensions.clone(), folders: script.folders, min: script.min.max(1), max: script.max },
                order: script.order,
                ask: script
                    .ask
                    .iter()
                    .map(|ask| match ask.kind.as_str() {
                        "choice" => Field::Choice { key: ask.key.clone(), label: ask.label.clone(), options: ask.options.clone() },
                        "number" => Field::Number { key: ask.key.clone(), label: ask.label.clone(), default: ask.default.parse().unwrap_or(0.) },
                        _ => Field::Text { key: ask.key.clone(), label: ask.label.clone(), default: ask.default.clone() },
                    })
                    .collect(),
            })
            .collect()
    }

    fn run(&self, id: &str, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome> {
        let script: Script = toml::from_str(&std::fs::read_to_string(id)?).map_err(io::Error::other)?;
        run_script(&script, selection, answers)
    }
}

/// libfiles, shipped with every script (`import libfiles`, `require`).
const LIBFILES: [(&str, &str); 3] = [
    ("libfiles.py", include_str!("../assets/libfiles/libfiles.py")),
    ("libfiles.js", include_str!("../assets/libfiles/libfiles.js")),
    ("libfiles.lua", include_str!("../assets/libfiles/libfiles.lua")),
];

/// A dialog a running script asked for (libfiles `ask`), waiting for the
/// UI: the answers, or `None` when the user cancelled.
pub struct ScriptAsk {
    pub title: String,
    pub fields: Vec<Field>,
    pub reply: std::sync::mpsc::Sender<Option<Vec<(String, String)>>>,
}

static ASKS: std::sync::Mutex<Vec<ScriptAsk>> = std::sync::Mutex::new(Vec::new());

/// Dialogs scripts are waiting for (the UI opens them).
pub fn take_asks() -> Vec<ScriptAsk> {
    std::mem::take(&mut *ASKS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// A libfiles field from its JSON.
fn field(value: &serde_json::Value) -> Option<Field> {
    let key = value["key"].as_str()?.to_string();
    let label = value["label"].as_str().unwrap_or(&key).to_string();
    Some(match value["kind"].as_str().unwrap_or("text") {
        "choice" => Field::Choice { key, label, options: value["options"].as_array()?.iter().filter_map(|option| option.as_str().map(String::from)).collect() },
        "number" => Field::Number { key, label, default: value["default"].as_str().and_then(|text| text.parse().ok()).unwrap_or(0.) },
        _ => Field::Text { key, label, default: value["default"].as_str().unwrap_or("").to_string() },
    })
}

/// Asks the UI (tests: the first option or default) and waits.
fn ask_ui(title: String, fields: Vec<Field>) -> Option<Vec<(String, String)>> {
    if cfg!(test) {
        return Some(
            fields
                .iter()
                .map(|field| match field {
                    Field::Choice { key, options, .. } => (key.clone(), options.first().cloned().unwrap_or_default()),
                    Field::Text { key, default, .. } => (key.clone(), default.clone()),
                    Field::Number { key, default, .. } => (key.clone(), default.to_string()),
                })
                .collect(),
        );
    }
    let (reply, answer) = std::sync::mpsc::channel();
    ASKS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(ScriptAsk { title, fields, reply });
    answer.recv().ok().flatten()
}

/// Runs a script on the selection. It talks to Files through libfiles
/// (`@files {json}` lines: ask, notify, made, reveal, progress); `made:
/// <path>` lines work too, and its last other line is the summary.
pub fn run_script(script: &Script, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome> {
    use std::io::{BufRead as _, Read as _, Write as _};
    let program = which(script.language.program()).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} is not installed", script.language.label())))?;
    let work = std::env::temp_dir().join(format!("file-force-script-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&work)?;
    for (name, code) in LIBFILES {
        std::fs::write(work.join(name), code)?;
    }
    let source = work.join(format!("action.{}", script.language.extension()));
    std::fs::write(&source, &script.code)?;
    let cleanup = |work: &Path| {
        let _ = std::fs::remove_dir_all(work);
    };
    let mut command = match script.language {
        Language::Rust => {
            let binary = work.join(if cfg!(windows) { "action.exe" } else { "action" });
            let built = files_sdk::log::run("quick-actions", files_sdk::log::command(&program).arg("-O").arg(&source).arg("-o").arg(&binary))?;
            if !built.status.success() {
                cleanup(&work);
                return Err(io::Error::other(format!("rustc: {}", String::from_utf8_lossy(&built.stderr).lines().take(6).collect::<Vec<_>>().join("\n"))));
            }
            files_sdk::log::command(binary)
        }
        Language::Go => {
            let mut command = files_sdk::log::command(&program);
            command.arg("run").arg(&source);
            command
        }
        Language::Shell if cfg!(windows) => {
            let mut command = files_sdk::log::command(&program);
            command.arg("/C").arg(&source);
            command
        }
        _ => {
            let mut command = files_sdk::log::command(&program);
            command.arg(&source);
            command
        }
    };
    command.args(selection);
    if let Some(first) = selection.first().and_then(|path| path.parent()) {
        command.current_dir(first);
    }
    // libfiles beside the script, found by each language's own lookup.
    command.env("PYTHONPATH", &work).env("NODE_PATH", &work).env("LUA_PATH", format!("{}/?.lua;;", work.display())).env("PYTHONUNBUFFERED", "1");
    command.env("FILES_SELECTION", selection.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n"));
    for (key, value) in answers {
        command.env(format!("FILES_ANSWER_{}", key.to_uppercase()), value);
    }
    command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    files_sdk::log::spawned("quick-actions", &command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            cleanup(&work);
            return Err(error);
        }
    };
    let mut stdin = child.stdin.take();
    let mut stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    });
    let mut made: Vec<PathBuf> = Vec::new();
    let mut summary = None;
    if let Some(stdout) = child.stdout.take() {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(json) = line.strip_prefix("@files ") {
                let Ok(message) = serde_json::from_str::<serde_json::Value>(json) else { continue };
                match message["op"].as_str() {
                    Some("ask") => {
                        let fields = message["fields"].as_array().map(|fields| fields.iter().filter_map(field).collect()).unwrap_or_default();
                        let reply = match ask_ui(message["title"].as_str().unwrap_or("Quick Action").to_string(), fields) {
                            Some(answers) => serde_json::json!({ "answers": answers.into_iter().map(|(key, value)| (key, serde_json::Value::String(value))).collect::<serde_json::Map<_, _>>() }),
                            None => serde_json::json!({ "cancelled": true }),
                        };
                        if let Some(stdin) = stdin.as_mut() {
                            let _ = writeln!(stdin, "{reply}");
                            let _ = stdin.flush();
                        }
                    }
                    Some("notify") => {
                        let level = match message["level"].as_str() {
                            Some("error") => files_sdk::log::Level::Error,
                            Some("warning") => files_sdk::log::Level::Warning,
                            Some("info") => files_sdk::log::Level::Info,
                            _ => files_sdk::log::Level::Success,
                        };
                        files_sdk::log::notify("quick-actions", level, message["title"].as_str().unwrap_or("Quick Action").to_string(), message["body"].as_str().map(String::from));
                    }
                    Some("made") => made.extend(message["path"].as_str().map(PathBuf::from).filter(|path| path.exists())),
                    Some("reveal") => crate::desktop::opened_from_system(message["path"].as_str().map(String::from).into_iter().collect()),
                    Some("progress") => {
                        files_sdk::log::record("quick-actions", files_sdk::log::Kind::Info, format!("{}: {}", script.name, message["text"].as_str().unwrap_or("")), None);
                    }
                    _ => {}
                }
            } else if let Some(path) = line.strip_prefix("made:") {
                made.extend(Some(PathBuf::from(path.trim())).filter(|path| path.exists()));
            } else if !line.trim().is_empty() {
                summary = Some(line.trim().to_string());
            }
        }
    }
    drop(stdin);
    let status = child.wait();
    let stderr = errors.join().unwrap_or_default();
    cleanup(&work);
    if !status?.success() {
        return Err(io::Error::other(stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("The script failed").to_string()));
    }
    Ok(Outcome { summary: summary.unwrap_or_else(|| "Done".into()), made })
}

// ------------------------------------------------------------- together

/// Where an action comes from, so a run goes back to it.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    BuiltIn,
    Script,
    Extension(&'static str),
}

/// Every action that fits the selection: built-in, scripts, extensions'.
pub fn for_selection(selection: &[PathBuf]) -> Vec<(Source, QuickAction)> {
    let mut all: Vec<(Source, QuickAction)> = fitting(BuiltIn.actions(), selection).into_iter().map(|action| (Source::BuiltIn, action)).collect();
    all.extend(fitting(Scripts.actions(), selection).into_iter().map(|action| (Source::Script, action)));
    for extension in crate::views::extension_ui::registry() {
        if let Some(provider) = extension.quick_actions() {
            let id = extension.manifest().id;
            all.extend(fitting(provider.actions(), selection).into_iter().map(|action| (Source::Extension(id), action)));
        }
    }
    all
}

pub fn run(source: &Source, id: &str, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome> {
    match source {
        Source::BuiltIn => BuiltIn.run(id, selection, answers),
        Source::Script => Scripts.run(id, selection, answers),
        Source::Extension(extension) => {
            files_sdk::permissions::check(extension, files_sdk::Permission::ReadFiles)?;
            crate::views::extension_ui::registry()
                .into_iter()
                .find(|candidate| candidate.manifest().id == *extension)
                .and_then(|candidate| candidate.quick_actions().map(|provider| provider.run(id, selection, answers)))
                .unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::NotFound, "The extension is gone")))
        }
    }
}

/// The code inside an agent's answer: the first fenced block, else all of it.
pub fn code_from_answer(answer: &str) -> String {
    let Some(start) = answer.find("```") else { return answer.trim().to_string() };
    let after = &answer[start + 3..];
    let body = after.split_once('\n').map_or(after, |(_, rest)| rest);
    body.split("```").next().unwrap_or(body).trim_end().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_convert_and_rotate() {
        let dir = std::env::temp_dir().join(format!("file-force-qa-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("wide.png");
        image::RgbImage::new(40, 10).save(&source).unwrap();
        let jpeg = convert(&source, "JPEG").unwrap();
        assert_eq!(jpeg.extension().unwrap(), "jpg");
        let low = convert_with(&source, "JPEG", quality(Some("Low"))).unwrap();
        assert!(image::open(&low).is_ok(), "a low-quality JPEG still opens");
        let ask = &BuiltIn.actions()[0].ask;
        assert!(ask.iter().any(|field| field.key() == "quality"), "Convert asks for the quality");
        let webp = convert(&source, "WebP").unwrap();
        assert!(image::open(&webp).is_ok());
        let turned = rotate(&source, "90° right", false).unwrap();
        assert_eq!(image::image_dimensions(&turned).unwrap(), (10, 40), "a copy, turned");
        assert_eq!(image::image_dimensions(&source).unwrap(), (40, 10), "the original is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_python_script_asks_through_libfiles_and_reports() {
        if !Language::Python.available() {
            return;
        }
        let file = std::env::temp_dir().join(format!("file-force-qa-py-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let code = "import libfiles\nanswers = libfiles.ask('Pick', [libfiles.choice('size', 'Size', ['small', 'big'])])\nfor path in libfiles.selection():\n    libfiles.made(path)\nprint('Chose ' + answers['size'])\n";
        let script = Script { name: "Py".into(), language: Language::Python, code: code.into(), ..Default::default() };
        let outcome = run_script(&script, std::slice::from_ref(&file), &[]).unwrap();
        assert_eq!(outcome.summary, "Chose small", "the dialog's answer reached the script");
        assert_eq!(outcome.made, vec![file.clone()]);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn the_menu_shows_what_fits_the_selection() {
        let titles = |paths: &[&str]| for_selection(&paths.iter().map(PathBuf::from).collect::<Vec<_>>()).into_iter().map(|(_, action)| action.title).collect::<Vec<_>>();
        assert_eq!(titles(&["a.heic", "b.PNG"]), ["Convert Image…", "Rotate Image…"]);
        assert!(!titles(&["notes.txt"]).iter().any(|title| title.starts_with("Convert")), "no image actions for text");
        assert!(!titles(&["a.png", "notes.txt"]).iter().any(|title| title.starts_with("Rotate")), "every selected item must fit");
    }

    #[test]
    fn a_shell_script_runs_on_the_selection_and_reports() {
        if cfg!(windows) {
            return;
        }
        let file = std::env::temp_dir().join(format!("file-force-qa-arg-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let script = Script { name: "Echo".into(), language: Language::Shell, code: "for p in \"$@\"; do echo \"made: $p\"; done\necho \"Counted $# with $FILES_ANSWER_MODE\"\n".into(), ..Default::default() };
        let outcome = run_script(&script, std::slice::from_ref(&file), &[("mode".into(), "fast".into())]).unwrap();
        assert_eq!(outcome.summary, "Counted 1 with fast");
        assert_eq!(outcome.made, vec![file.clone()]);
        assert_eq!(code_from_answer("Here:\n```python\nprint(1)\n```\nok"), "print(1)\n");
        assert_eq!(file_name("Make Thumbnails!"), "make-thumbnails.toml");
        let _ = std::fs::remove_file(&file);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod heic_live {
    #[test]
    #[ignore]
    fn heic_round_trip() {
        let dir = std::env::temp_dir().join(format!("file-force-heic-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("square.png");
        image::RgbImage::from_pixel(64, 32, image::Rgb([200, 40, 90])).save(&png).unwrap();
        let heic = super::convert(&png, "HEIC").unwrap();
        let back = super::convert(&heic, "PNG").unwrap();
        let turned = super::rotate(&heic, "90° left", false).unwrap();
        // JPEG qualities and JPEG XL (when cjxl is installed).
        let photo = image::RgbImage::from_fn(400, 300, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x * y) % 255) as u8]));
        let jpeg = dir.join("photo.jpg");
        photo.save(&jpeg).unwrap();
        let small = super::convert_with(&png, "JPEG", 60).unwrap();
        let big = std::fs::metadata(&jpeg).unwrap().len();
        if super::jxl_tool("cjxl").is_some() {
            let jxl = super::convert_with(&jpeg, "JPEG XL", 92).unwrap();
            println!("jpeg {big} → lossless jxl {} · low jpeg {}", std::fs::metadata(&jxl).unwrap().len(), std::fs::metadata(&small).unwrap().len());
            let decoded = super::convert(&jxl, "PNG").unwrap();
            assert_eq!(image::image_dimensions(&decoded).unwrap(), (400, 300));
        }
        println!("{} {:?} {}", heic.display(), image::image_dimensions(&back).unwrap(), turned.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
