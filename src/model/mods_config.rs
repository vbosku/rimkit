//! Reads/writes the `<activeMods>` block of RimWorld's ModsConfig.xml.
//! Everything else in the file is preserved byte for byte.

use roxmltree::Document;
use std::fs;
use std::io;
use std::path::Path;


pub fn read_active(path: &Path) -> io::Result<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let doc = Document::parse(raw.trim_start_matches('\u{feff}')).map_err(io::Error::other)?;
    Ok(doc
        .descendants()
        .find(|n| n.has_tag_name("activeMods"))
        .map(|n| {
            n.children()
                .filter(|c| c.has_tag_name("li"))
                .filter_map(|c| c.text())
                .map(|t| t.trim().to_lowercase())
                .collect()
        })
        .unwrap_or_default())
}

pub fn write_active(path: &Path, ids: &[String]) -> io::Result<()> {
    let ids = dedupe(ids.iter().cloned());
    let new = match fs::read_to_string(path) {
        Ok(old) => {
            let text = replace_block(&old, "activeMods", &ids).ok_or_else(|| io::Error::other("ModsConfig.xml has no <activeMods> block"))?;
            // Older tools wrote DLCs here more than once; keep each once.
            match super::modlist_io::section(&text, "knownExpansions") {
                Some(known) => replace_block(&text, "knownExpansions", &dedupe(known)).unwrap_or(text),
                None => text,
            }
        }
        Err(_) => format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  {}\n</ModsConfigData>\n", block("activeMods", &ids)),
    };
    // Atomic: a crash mid-write leaves the old file intact, never a half-written one.
    let tmp = path.with_extension("xml.tmp");
    fs::write(&tmp, new)?;
    fs::rename(&tmp, path)
}

/// Drops repeated ids (case-insensitive), keeping the first so load order stays the same.
pub fn dedupe(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter(|id| seen.insert(id.to_lowercase())).collect()
}

fn block(tag: &str, items: &[String]) -> String {
    let lis: String = items.iter().map(|id| format!("\n    <li>{id}</li>")).collect();
    format!("<{tag}>{lis}\n  </{tag}>")
}

/// `text` with its `<tag>…</tag>` replaced by a fresh block; None if there is no such block.
fn replace_block(text: &str, tag: &str, items: &[String]) -> Option<String> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let a = text.find(&open)?;
    let b = a + text[a..].find(&close)? + close.len();
    Some(format!("{}{}{}", &text[..a], block(tag, items), &text[b..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_drops_repeats_keeping_first() {
        let dir = std::env::temp_dir().join("rimkit_dedupe_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ModsConfig.xml");
        fs::write(&path, "<ModsConfigData>\n  <activeMods>\n  </activeMods>\n  <knownExpansions>\n    <li>ludeon.rimworld.royalty</li>\n    <li>Ludeon.RimWorld.Royalty</li>\n    <li>ludeon.rimworld.biotech</li>\n  </knownExpansions>\n</ModsConfigData>\n").unwrap();
        let ids: Vec<String> = ["a", "b", "A", "c", "b"].iter().map(|s| s.to_string()).collect();
        write_active(&path, &ids).unwrap();
        assert_eq!(read_active(&path).unwrap(), ["a", "b", "c"]);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.to_lowercase().matches("ludeon.rimworld.royalty").count(), 1);
        assert!(text.contains("ludeon.rimworld.biotech"));
    }

    #[test]
    fn round_trip_keeps_rest_of_file() {
        let dir = std::env::temp_dir().join("rimkit_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ModsConfig.xml");
        let original = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.7</version>\n  <activeMods>\n    <li>a</li>\n  </activeMods>\n  <knownExpansions>\n    <li>x</li>\n  </knownExpansions>\n</ModsConfigData>\n";
        fs::write(&path, original).unwrap();
        let ids = vec!["brrainz.harmony".to_string(), "ludeon.rimworld".to_string()];
        write_active(&path, &ids).unwrap();
        assert_eq!(read_active(&path).unwrap(), ids);
        let new = fs::read_to_string(&path).unwrap();
        assert!(new.starts_with('\u{feff}') && new.contains("<knownExpansions>\n    <li>x</li>"));
        // Written atomically (temp file + rename): nothing left behind next to it.
        let mut left: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
        left.sort();
        assert_eq!(left, ["ModsConfig.xml"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
