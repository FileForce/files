//! Previews for formats Files does not know: an extension that reads a kind
//! of file (a CAD drawing, a game save, a notebook) shows it in Quick Look
//! and the preview panel. Needs [`crate::Permission::ReadFiles`]; the user
//! can take it away, and the extension's previews stop.

use std::io;
use std::path::{Path, PathBuf};

/// What a preview shows.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewContent {
    /// Text, as is; `monospace` for code and logs.
    Text { text: String, monospace: bool },
    /// A picture the extension made (PNG, JPEG…), e.g. in its cache folder.
    Image(PathBuf),
    /// Pictures one under the other, scrolling: pages, slides, frames.
    Pages(Vec<PathBuf>),
    /// Facts about the file, as label and value rows.
    Details(Vec<(String, String)>),
}

pub trait PreviewProvider: Send + Sync {
    /// Whether it previews the file at `path`.
    fn handles(&self, path: &Path) -> bool;
    /// The preview (called off the UI thread; may take a moment).
    fn preview(&self, path: &Path) -> io::Result<PreviewContent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Notes;

    impl PreviewProvider for Notes {
        fn handles(&self, path: &Path) -> bool {
            path.extension().is_some_and(|extension| extension == "note")
        }

        fn preview(&self, _: &Path) -> io::Result<PreviewContent> {
            Ok(PreviewContent::Text { text: "hello".into(), monospace: false })
        }
    }

    #[test]
    fn a_provider_says_what_it_previews() {
        assert!(Notes.handles(Path::new("a.note")) && !Notes.handles(Path::new("a.txt")));
        assert_eq!(Notes.preview(Path::new("a.note")).unwrap(), PreviewContent::Text { text: "hello".into(), monospace: false });
    }
}
