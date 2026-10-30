//! The smallest Pro plugin: one context-menu command. Copy this crate to
//! start a paid extension; `cargo xtask package-extension files-ext-hello-pro`
//! turns it into a signed `.filesext`.

use files_sdk::plugin::{ABI_VERSION, Plugin, PluginManifest, PluginMenuItem};
use files_sdk::{Context, Outcome};

pub struct HelloPro;

impl Plugin for HelloPro {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "hello-pro".into(),
            name: "Hello Pro".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            summary: "Example of a paid extension.".into(),
            publisher: "Files".into(),
            product: Some("hello-pro".into()),
            permissions: vec![],
            menu: vec![PluginMenuItem {
                command: "hello-pro.greet".into(),
                title: "Hello from Pro".into(),
                targets: vec!["file".into(), "folder".into(), "background".into()],
            }],
            menu_color: Some("pink".into()),
            abi: ABI_VERSION,
        }
    }

    fn run(&self, command: &str, context: &Context) -> Result<Outcome, String> {
        match command {
            "hello-pro.greet" => Ok(Outcome {
                notice: Some(format!("Hello from Pro · {} item(s) in {}", context.selection.len(), context.folder.clone().unwrap_or_default())),
                ..Default::default()
            }),
            other => Err(format!("unknown command {other}")),
        }
    }
}

files_sdk::export_plugin!(HelloPro);
