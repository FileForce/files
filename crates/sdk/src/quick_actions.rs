//! Quick Actions (the context menu, after Copy): an extension offers
//! actions for what is selected — only those that fit it, in its order —
//! asks for what it needs in a dialog Files draws, then runs.

use std::io;
use std::path::{Path, PathBuf};

/// Which selections an action is for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applies {
    /// Lowercase extensions without the dot; empty: any.
    pub extensions: Vec<String>,
    /// Folders too (else files only).
    pub folders: bool,
    pub min: usize,
    /// 0: no limit.
    pub max: usize,
}

impl Applies {
    pub fn to(extensions: &[&str]) -> Self {
        Self { extensions: extensions.iter().map(|extension| extension.to_string()).collect(), folders: false, min: 1, max: 0 }
    }

    /// Every selected item fits, and there are as many as wanted.
    pub fn fits(&self, selection: &[PathBuf]) -> bool {
        let count = selection.len();
        if count < self.min.max(1) || (self.max > 0 && count > self.max) {
            return false;
        }
        selection.iter().all(|path| {
            if path.is_dir() {
                return self.folders;
            }
            self.extensions.is_empty()
                || path.extension().is_some_and(|extension| self.extensions.iter().any(|wanted| wanted.eq_ignore_ascii_case(&extension.to_string_lossy())))
        })
    }
}

/// A question the dialog asks before the action runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// One of `options`, the first chosen to begin with.
    Choice { key: String, label: String, options: Vec<String> },
    Text { key: String, label: String, default: String },
    Number { key: String, label: String, default: f64 },
}

impl Field {
    pub fn key(&self) -> &str {
        match self {
            Field::Choice { key, .. } | Field::Text { key, .. } | Field::Number { key, .. } => key,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuickAction {
    /// Stable id the provider knows it by.
    pub id: String,
    pub title: String,
    pub applies: Applies,
    /// Lower comes first in the menu.
    pub order: i32,
    /// Asked in a dialog first; empty: runs at once.
    pub ask: Vec<Field>,
}

/// What a run did, for the notification.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub summary: String,
    /// Files it made or changed (revealed in Files).
    pub made: Vec<PathBuf>,
}

pub trait QuickActionProvider: Send + Sync {
    /// Every action it has; Files keeps those that fit the selection.
    fn actions(&self) -> Vec<QuickAction>;
    /// Runs `id` on `selection` with the dialog's `answers` (off the UI thread).
    fn run(&self, id: &str, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome>;
}

/// The actions of `providers` that fit `selection`, in order then by title.
pub fn fitting(actions: Vec<QuickAction>, selection: &[PathBuf]) -> Vec<QuickAction> {
    let mut fitting: Vec<QuickAction> = actions.into_iter().filter(|action| action.applies.fits(selection)).collect();
    fitting.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    fitting
}

/// The answer to `key`, if given.
pub fn answer<'a>(answers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    answers.iter().find(|(asked, _)| asked == key).map(|(_, value)| value.as_str())
}

/// `photo.heic` → `photo.png` beside it, never over an existing file.
pub fn sibling_with_extension(path: &Path, extension: &str) -> PathBuf {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let target = path.with_file_name(format!("{stem}.{extension}"));
    if !target.exists() {
        return target;
    }
    crate::fs::unique_path(&target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_actions_that_fit_the_selection_show_in_order() {
        let action = |id: &str, extensions: &[&str], order| QuickAction { id: id.into(), title: id.into(), applies: Applies::to(extensions), order, ask: Vec::new() };
        let all = vec![action("b-rotate", &["png", "jpg"], 20), action("a-convert", &["png", "jpg", "heic"], 10), action("pdf", &["pdf"], 0)];
        let photos = [PathBuf::from("a.PNG"), PathBuf::from("b.jpg")];
        let ids: Vec<String> = fitting(all.clone(), &photos).into_iter().map(|action| action.id).collect();
        assert_eq!(ids, ["a-convert", "b-rotate"], "the pdf action is left out, the rest sorted by order");
        assert!(fitting(all, &[PathBuf::from("a.png"), PathBuf::from("b.pdf")]).is_empty(), "every item must fit");
        assert_eq!(sibling_with_extension(Path::new("/nowhere/photo.heic"), "png"), PathBuf::from("/nowhere/photo.png"));
    }
}
