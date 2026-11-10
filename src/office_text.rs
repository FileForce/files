//! The text of office documents for Quick Look and the preview panel:
//! Word (.docx), OpenDocument (.odt) and PowerPoint (.pptx) are zip files
//! of XML, read here on every system — so a document scrolls page after
//! page instead of showing a picture of its first page.

use std::io::Read as _;
use std::path::Path;

/// Text beyond this is not shown (a 500-page report stays light).
const LIMIT: usize = 200_000;

pub fn applies(path: &Path) -> bool {
    path.extension().is_some_and(|extension| matches!(extension.to_string_lossy().to_lowercase().as_str(), "docx" | "odt" | "pptx"))
}

fn entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut file = zip.by_name(name).ok()?;
    let mut text = String::new();
    file.by_ref().take(64 * 1024 * 1024).read_to_string(&mut text).ok()?;
    Some(text)
}

fn decode(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text runs of an XML document: `text_tags` hold text, `breaks`
/// end a paragraph, `tabs` are tabs.
fn runs(xml: &str, text_tags: &[&str], breaks: &[&str], tabs: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        let closing = tag.starts_with('/');
        let empty = tag.ends_with('/');
        if !closing && !empty && text_tags.contains(&name) {
            let close = format!("</{name}>");
            if let Some(stop) = after.find(&close) {
                out.push_str(&decode(&after[..stop]));
                rest = &after[stop + close.len()..];
                continue;
            }
        }
        if breaks.contains(&name) && (closing || empty) {
            out.push('\n');
        }
        if tabs.contains(&name) && empty {
            out.push('\t');
        }
        rest = after;
        if out.len() > LIMIT {
            break;
        }
    }
    out
}

/// OpenDocument keeps text straight inside its paragraphs: every text node
/// of the body, a line per paragraph or heading.
fn odt_text(xml: &str) -> String {
    let body = xml.find("<office:body").map_or(xml, |start| &xml[start..]);
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        out.push_str(&decode(&rest[..start]));
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start + 1..start + end];
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        match name {
            "text:p" | "text:h" if tag.starts_with('/') || tag.ends_with('/') => out.push('\n'),
            "text:line-break" => out.push('\n'),
            "text:tab" => out.push('\t'),
            "text:s" => out.push(' '),
            _ => {}
        }
        rest = &rest[start + end + 1..];
        if out.len() > LIMIT {
            break;
        }
    }
    out
}

/// Blank-line clean-up: at most one empty line in a row.
fn tidy(text: String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

/// The document's text, or `None` when it is not one of these (or broken).
pub fn text(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let text = match extension.as_str() {
        "docx" => runs(&entry(&mut zip, "word/document.xml")?, &["w:t"], &["w:p", "w:br", "w:cr"], &["w:tab"]),
        "odt" => odt_text(&entry(&mut zip, "content.xml")?),
        "pptx" => {
            // Slides in order: ppt/slides/slide1.xml, slide2.xml…
            let mut slides: Vec<(usize, String)> = (0..zip.len())
                .filter_map(|index| zip.by_index(index).ok().and_then(|file| file.name().ok().map(|name| name.to_string())))
                .filter_map(|name| name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse::<usize>().ok().map(|number| (number, name)))
                .collect();
            slides.sort();
            let mut out = String::new();
            for (number, name) in slides {
                let body = runs(&entry(&mut zip, &name)?, &["a:t"], &["a:p", "a:br"], &[]);
                out.push_str(&format!("— Slide {number} —\n{}\n\n", body.trim()));
            }
            out
        }
        _ => return None,
    };
    let mut text = tidy(text);
    if text.len() > LIMIT {
        let cut = (0..=LIMIT).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n\n… (the preview shows the beginning)");
    }
    (!text.is_empty()).then_some(text)
}

/// `text`, kept for the last few documents (previews redraw often).
pub fn cached(path: &Path) -> Option<String> {
    type Entry = (std::path::PathBuf, Option<std::time::SystemTime>, Option<String>);
    static LAST: std::sync::Mutex<Vec<Entry>> = std::sync::Mutex::new(Vec::new());
    let modified = std::fs::metadata(path).ok()?.modified().ok();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, _, text)) = last.iter().find(|(cached, when, _)| cached == path && *when == modified) {
        return text.clone();
    }
    let read = text(path);
    last.retain(|(cached, _, _)| cached != path);
    last.push((path.to_path_buf(), modified, read.clone()));
    if last.len() > 4 {
        last.remove(0);
    }
    read
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn a_word_document_reads_as_paragraphs() {
        let path = std::env::temp_dir().join(format!("file-force-office-{}.docx", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(br#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Quarterly &amp; yearly</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Line </w:t><w:tab/><w:t>two</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        zip.finish().unwrap();
        assert_eq!(text(&path).as_deref(), Some("Quarterly & yearly\nLine \ttwo"));
        assert!(applies(&path) && !applies(Path::new("a.pdf")));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn read_a_real_document() {
        for path in std::env::var("DOCS").unwrap().split('|') {
            let text = super::text(std::path::Path::new(path)).unwrap_or_default();
            println!("== {path}: {} lines\n{}", text.lines().count(), text.lines().take(3).collect::<Vec<_>>().join(" | "));
        }
    }
}
