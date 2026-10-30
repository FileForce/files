use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "images/*.jpg"]
struct EmbeddedAssets;

#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = EmbeddedAssets::get(path) {
            return Ok(Some(file.data));
        }
        AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AllAssets.list(path)?;
        paths.extend(
            EmbeddedAssets::iter().filter_map(|name| name.starts_with(path).then(|| name.into())),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

