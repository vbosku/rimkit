use roxmltree::{Document, Node, ParsingOptions};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    Official,
    #[default]
    Local,
    Workshop,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Official => "Official",
            Source::Local => "Local",
            Source::Workshop => "Workshop",
        }
    }
}

/// Everything we read from a mod's About/About.xml. Ids are lowercase.
#[derive(Clone, Debug, Default)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub versions: Vec<String>,
    pub path: PathBuf,
    pub source: Source,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub dependencies: Vec<String>,
    pub incompatible: Vec<String>,
    /// DLL file stems (lowercase) shipped in any Assemblies folder; used to blame log entries.
    pub assemblies: Vec<String>,
    /// (dependency id, where to get it): Workshop or download URL from `<modDependencies>`.
    pub links: Vec<(String, String)>,
}

impl ModInfo {
    /// Where to get dependency `id`, if this mod says.
    pub fn link_for(&self, id: &str) -> Option<&str> {
        self.links.iter().find(|(dep, _)| dep == id).map(|(_, url)| url.as_str())
    }

    /// Workshop mods live in a folder named after their Workshop file id.
    pub fn steam_id(&self) -> Option<u64> {
        (self.source == Source::Workshop).then(|| self.path.file_name()?.to_str()?.parse().ok()).flatten()
    }

    pub fn supports(&self, game_version: &str) -> bool {
        self.source == Source::Official || game_version.is_empty() || self.versions.iter().any(|v| v == game_version)
    }
}

/// Scans every root (official Data, local Mods, Steam workshop). First id found wins.
pub fn scan(roots: &[(PathBuf, Source)], game_version: &str) -> Vec<ModInfo> {
    let dirs: Vec<(PathBuf, Source)> = roots
        .iter()
        .filter_map(|(root, src)| fs::read_dir(root).ok().map(|rd| (rd, *src)))
        .flat_map(|(rd, src)| rd.flatten().map(move |e| (e.path(), src)))
        .filter(|(p, _)| p.is_dir())
        .collect();

    // About.xml parsing is I/O bound; spread it over a few threads.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunk = dirs.len().div_ceil(threads).max(1);
    let parsed: Vec<Option<ModInfo>> = std::thread::scope(|s| {
        let handles: Vec<_> = dirs
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(|(p, src)| parse(p, *src, game_version)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });

    let mut seen = HashSet::new();
    let mut mods: Vec<ModInfo> = parsed.into_iter().flatten().filter(|m| seen.insert(m.id.clone())).collect();
    mods.sort_by_cached_key(|m| m.name.to_lowercase());
    mods
}

fn parse(dir: &Path, source: Source, game_version: &str) -> Option<ModInfo> {
    let raw = fs::read_to_string(dir.join("About").join("About.xml")).ok()?;
    let opts = ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = Document::parse_with_options(raw.trim_start_matches('\u{feff}'), opts).ok()?;
    let root = doc.root_element();
    let folder = dir.file_name()?.to_string_lossy().to_string();

    let id = Some(text(root, "packageId")).filter(|s| !s.is_empty()).unwrap_or_else(|| folder.clone()).to_lowercase();
    let name = Some(text(root, "name")).filter(|s| !s.is_empty()).unwrap_or(folder);
    let versioned = |tag: &str| [list(root, tag), by_version(root, tag, game_version)].concat();

    let (mut dependencies, mut links) = (Vec::new(), Vec::new());
    let dep_items = child(root, "modDependencies")
        .into_iter()
        .chain(version_node(root, "modDependencies", game_version))
        .flat_map(|n| n.children().filter(|c| c.has_tag_name("li")));
    for li in dep_items {
        let id = text(li, "packageId").to_lowercase();
        if id.is_empty() || dependencies.contains(&id) {
            continue;
        }
        if let Some(url) = [text(li, "steamWorkshopUrl"), text(li, "downloadUrl")].into_iter().find(|u| !u.is_empty()) {
            links.push((id.clone(), url));
        }
        dependencies.push(id);
    }

    Some(ModInfo {
        id,
        name,
        author: text(root, "author"),
        description: clean_description(&text(root, "description")),
        versions: list(root, "supportedVersions"),
        path: dir.to_path_buf(),
        source,
        load_after: [versioned("loadAfter"), versioned("forceLoadAfter")].concat(),
        load_before: [versioned("loadBefore"), versioned("forceLoadBefore")].concat(),
        dependencies,
        incompatible: versioned("incompatibleWith"),
        assemblies: assemblies(dir),
        links,
    })
}

/// DLL stems in `<mod>/Assemblies` and `<mod>/<any>/Assemblies` (e.g. 1.6/Assemblies).
fn assemblies(dir: &Path) -> Vec<String> {
    let subdirs = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir());
    let mut out: Vec<String> = std::iter::once(dir.to_path_buf())
        .chain(subdirs)
        .filter_map(|d| fs::read_dir(d.join("Assemblies")).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .filter(|s| s != "0harmony")
        .collect();
    out.sort();
    out.dedup();
    out
}

fn child<'a>(node: Node<'a, 'a>, tag: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|c| c.has_tag_name(tag))
}

fn text(node: Node, tag: &str) -> String {
    child(node, tag).and_then(|n| n.text()).unwrap_or("").trim().to_string()
}

/// `<tag><li>a</li><li>b</li></tag>` -> ["a", "b"] (lowercase).
fn list(node: Node, tag: &str) -> Vec<String> {
    child(node, tag).map(li_values).unwrap_or_default()
}

fn li_values(node: Node) -> Vec<String> {
    node.children()
        .filter(|c| c.has_tag_name("li"))
        .filter_map(|li| li.text())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// `<tagByVersion><v1.6>...</v1.6></tagByVersion>` for the running game version.
fn version_node<'a>(node: Node<'a, 'a>, tag: &str, game_version: &str) -> Option<Node<'a, 'a>> {
    child(node, &format!("{tag}ByVersion")).and_then(|n| child(n, &format!("v{game_version}")))
}

fn by_version(node: Node, tag: &str, game_version: &str) -> Vec<String> {
    version_node(node, tag, game_version).map(li_values).unwrap_or_default()
}

/// Turns literal "\n" into newlines and strips Unity rich-text tags.
pub(crate) fn clean_description(s: &str) -> String {
    const TAGS: [&str; 4] = ["b", "i", "color", "size"];
    let s = s.replace("\\n", "\n");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let end = tail.find('>');
        let tag = end.map(|e| tail[..e].trim_start_matches('/').split('=').next().unwrap_or("").to_lowercase());
        match (end, tag) {
            (Some(e), Some(t)) if TAGS.contains(&t.as_str()) => rest = &tail[e + 1..],
            _ => {
                out.push('<');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// "1.7.4984 rev1242" -> "1.7"
pub fn game_version(game_dir: &Path) -> String {
    let raw = fs::read_to_string(game_dir.join("Version.txt")).unwrap_or_default();
    raw.split('.').take(2).collect::<Vec<_>>().join(".").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_links_and_workshop_id() {
        let root = std::env::temp_dir().join("rimkit_modinfo_test");
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("2009463077");
        fs::create_dir_all(dir.join("About")).unwrap();
        fs::write(
            dir.join("About").join("About.xml"),
            "<ModMetaData><name>Thing</name><packageId>a.thing</packageId><modDependencies>
                <li><packageId>Brrainz.Harmony</packageId><steamWorkshopUrl>steam://url/CommunityFilePage/2009463077</steamWorkshopUrl></li>
                <li><packageId>b.lib</packageId><downloadUrl>https://example.com/lib</downloadUrl></li>
                <li><packageId>c.none</packageId></li>
            </modDependencies></ModMetaData>",
        )
        .unwrap();
        let mods = scan(&[(root.clone(), Source::Workshop)], "1.7");
        let m = &mods[0];
        assert_eq!(m.dependencies, ["brrainz.harmony", "b.lib", "c.none"]);
        assert_eq!(m.link_for("brrainz.harmony"), Some("steam://url/CommunityFilePage/2009463077"));
        assert_eq!(m.link_for("b.lib"), Some("https://example.com/lib"));
        assert_eq!(m.link_for("c.none"), None);
        assert_eq!(m.steam_id(), Some(2009463077));
    }
}
