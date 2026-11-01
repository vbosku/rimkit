//! Import/export of mod lists in RimWorld's own formats:
//! ModsConfig.xml (`<activeMods>`), the game's saved mod lists (`.rml`) and save files (`.rws`, `<meta>`).

/// One mod of an imported/exported list. Ids are lowercase.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    /// Steam Workshop file id, when the source knows it.
    pub steam_id: Option<u64>,
    pub name: Option<String>,
}

/// Mods listed in any supported file's text. None if it holds no mod list.
pub fn read(text: &str) -> Option<Vec<Entry>> {
    if let Some(ids) = section(text, "activeMods") {
        return Some(ids.into_iter().map(|id| Entry { id: id.to_lowercase(), steam_id: None, name: None }).collect());
    }
    let ids = section(text, "modIds")?;
    let steam = section(text, "modSteamIds").unwrap_or_default();
    let names = section(text, "modNames").unwrap_or_default();
    Some(
        ids.iter()
            .enumerate()
            .map(|(i, id)| Entry {
                id: id.to_lowercase(),
                steam_id: steam.get(i).and_then(|s| s.parse().ok()).filter(|&n| n != 0),
                name: names.get(i).cloned(),
            })
            .collect(),
    )
}

/// A `.rml` mod list, laid out like the ones the game's own mod manager saves.
pub fn write_rml(entries: &[Entry], game_version: &str) -> String {
    let items = |value: &dyn Fn(&Entry) -> String| -> String {
        entries.iter().map(|e| format!("\t\t\t<li>{}</li>\n", escape(&value(e)))).collect()
    };
    let ids = items(&|e| e.id.clone());
    let steam = items(&|e| e.steam_id.unwrap_or(0).to_string());
    let names = items(&|e| e.name.clone().unwrap_or_else(|| e.id.clone()));
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<savedModList>\n\t<meta>\n\t\t<gameVersion>{}</gameVersion>\n\
         \t\t<modIds>\n{ids}\t\t</modIds>\n\t\t<modSteamIds>\n{steam}\t\t</modSteamIds>\n\t\t<modNames>\n{names}\t\t</modNames>\n\t</meta>\n\
         \t<modList>\n\t\t<ids>\n{ids}\t\t</ids>\n\t\t<names>\n{names}\t\t</names>\n\t</modList>\n</savedModList>\n",
        escape(game_version)
    )
}

/// `<li>` values inside the first `<tag>…</tag>`. Only that fragment is parsed, so huge saves are fine.
pub(crate) fn section(text: &str, tag: &str) -> Option<Vec<String>> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let start = text.find(&open)?;
    let end = start + text[start..].find(&close)? + close.len();
    let doc = roxmltree::Document::parse(&text[start..end]).ok()?;
    let items = doc.root_element().children().filter(|c| c.has_tag_name("li"));
    Some(items.map(|li| li.text().unwrap_or("").trim().to_string()).collect())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, steam: Option<u64>, name: Option<&str>) -> Entry {
        Entry { id: id.into(), steam_id: steam, name: name.map(Into::into) }
    }

    #[test]
    fn reads_mods_config() {
        let text = "\u{feff}<?xml version=\"1.0\"?><ModsConfigData><activeMods><li>Brrainz.Harmony</li><li>ludeon.rimworld</li></activeMods><knownExpansions><li>x</li></knownExpansions></ModsConfigData>";
        assert_eq!(read(text).unwrap(), [e("brrainz.harmony", None, None), e("ludeon.rimworld", None, None)]);
    }

    #[test]
    fn reads_save_meta_with_steam_ids_and_names() {
        let text = "<savegame><meta><gameVersion>1.7</gameVersion>
            <modIds><li>ludeon.rimworld</li><li>brrainz.harmony</li></modIds>
            <modSteamIds><li>0</li><li>2009463077</li></modSteamIds>
            <modNames><li>Core</li><li>Harmony</li></modNames>
            </meta><game><huge>...</huge>";
        assert_eq!(read(text).unwrap(), [e("ludeon.rimworld", None, Some("Core")), e("brrainz.harmony", Some(2009463077), Some("Harmony"))]);
    }

    #[test]
    fn rml_round_trip() {
        let list = [e("brrainz.harmony", Some(2009463077), Some("Harmony & <stuff>")), e("ludeon.rimworld", None, Some("Core"))];
        let text = write_rml(&list, "1.7.4984 rev1242");
        assert!(text.contains("<savedModList>") && text.contains("<gameVersion>1.7.4984 rev1242</gameVersion>"));
        assert_eq!(read(&text).unwrap(), list);
    }

    #[test]
    fn not_a_mod_list() {
        assert_eq!(read("<foo><bar/></foo>"), None);
        assert_eq!(read("not xml at all"), None);
    }
}
