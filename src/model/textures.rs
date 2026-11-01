//! Texture inventory: how much VRAM each mod's textures need and whether the game already cached them.
//!
//! What RimWorld 1.7 does (read from its code): PNGs are compressed to BC7 by the engine and kept in
//! `TextureCache/v<N>/<variant>/<mod folder>-<hash>.zip`; a `.dds` next to a `.png` replaces it
//! (`ModContentLoader.ShadowedByDds`). BC7 is 1 byte per pixel, plus a third for mipmaps.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModTextures {
    pub png: usize,
    pub png_bytes: u64,
    pub dds: usize,
    pub dds_bytes: u64,
    /// Pixels of every texture the game will load (a PNG shadowed by a DDS counts once, as the DDS).
    pub pixels: u64,
    /// Files whose header couldn't be read.
    pub unreadable: usize,
}

impl ModTextures {
    pub fn count(&self) -> usize {
        self.png + self.dds
    }

    /// Estimated GPU memory once compressed: BC7, 1 byte/pixel, +1/3 for mipmaps.
    pub fn vram_bytes(&self) -> u64 {
        self.pixels * 4 / 3
    }
}

/// (width, height) from the first 24 bytes of a PNG.
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    let ok = head.len() >= 24 && head.starts_with(b"\x89PNG\r\n\x1a\n") && &head[12..16] == b"IHDR";
    ok.then(|| (u32_at(head, 16, u32::from_be_bytes), u32_at(head, 20, u32::from_be_bytes)))
}

/// (width, height) from the first 20 bytes of a DDS.
pub fn dds_size(head: &[u8]) -> Option<(u32, u32)> {
    let ok = head.len() >= 20 && head.starts_with(b"DDS ");
    ok.then(|| (u32_at(head, 16, u32::from_le_bytes), u32_at(head, 12, u32::from_le_bytes)))
}

fn u32_at(b: &[u8], at: usize, from: fn([u8; 4]) -> u32) -> u32 {
    from([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Folders the game loads for this mod: `loadFolders.xml`'s entry for the game version if there is one,
/// else the mod root, `Common`, and the newest version folder not above the game's.
pub fn content_folders(mod_dir: &Path, game_version: &str) -> Vec<PathBuf> {
    if let Some(listed) = load_folders(mod_dir, game_version) {
        return listed;
    }
    let mut out = vec![mod_dir.to_path_buf()];
    if mod_dir.join("Common").is_dir() {
        out.push(mod_dir.join("Common"));
    }
    let game = parse_version(game_version);
    let newest = fs::read_dir(mod_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((parse_version(e.file_name().to_str()?)?, e.path())))
        .filter(|(v, _)| game.is_some_and(|g| *v <= g))
        .max_by_key(|(v, _)| *v);
    out.extend(newest.map(|(_, p)| p));
    out
}

fn load_folders(mod_dir: &Path, game_version: &str) -> Option<Vec<PathBuf>> {
    let raw = fs::read_to_string(mod_dir.join("loadFolders.xml")).ok()?;
    let doc = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).ok()?;
    let tag = format!("v{game_version}");
    let entry = doc.root_element().children().find(|c| c.has_tag_name(tag.as_str()))?;
    let folders = entry.children().filter(|c| c.has_tag_name("li")).filter_map(|li| li.text()).map(str::trim);
    Some(folders.map(|f| if f == "/" || f.is_empty() { mod_dir.to_path_buf() } else { mod_dir.join(f) }).collect())
}

/// "1.6" -> (1, 6)
fn parse_version(s: &str) -> Option<(u32, u32)> {
    let (major, minor) = s.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Every texture under `<folder>/Textures` of the loaded folders.
pub fn scan(mod_dir: &Path, game_version: &str) -> ModTextures {
    let mut files = Vec::new();
    for folder in content_folders(mod_dir, game_version) {
        walk(&folder.join("Textures"), &mut files);
    }
    let ext = |p: &Path| p.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    let stem = |p: &Path| p.with_extension("").to_string_lossy().to_lowercase();
    let dds_stems: HashSet<String> = files.iter().filter(|(p, _)| ext(p) == "dds").map(|(p, _)| stem(p)).collect();

    let mut t = ModTextures::default();
    for (path, bytes) in &files {
        let size = match ext(path).as_str() {
            "png" if !dds_stems.contains(&stem(path)) => {
                t.png += 1;
                t.png_bytes += bytes;
                png_size(&head(path, 24))
            }
            "dds" => {
                t.dds += 1;
                t.dds_bytes += bytes;
                dds_size(&head(path, 20))
            }
            _ => continue,
        };
        match size {
            Some((w, h)) => t.pixels += w as u64 * h as u64,
            None => t.unreadable += 1,
        }
    }
    t
}

/// (path, size) of every file below `dir`.
fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push((path, entry.metadata().map_or(0, |m| m.len())));
        }
    }
}

fn head(path: &Path, n: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = fs::File::open(path) {
        let _ = f.take(n).read_to_end(&mut buf);
    }
    buf
}

/// Engine texture cache: bytes cached per mod folder name (lowercase), across every cache version/variant.
pub fn cache_index(texture_cache: &Path) -> HashMap<String, u64> {
    let subdirs = |dir: &Path| -> Vec<fs::DirEntry> {
        fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).collect()
    };
    let versions = subdirs(texture_cache).into_iter().filter(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.strip_prefix('v').is_some_and(|n| n.parse::<u32>().is_ok())
    });
    let mut out: HashMap<String, u64> = HashMap::new();
    for variant in versions.flat_map(|v| subdirs(&v.path())) {
        for file in fs::read_dir(variant.path()).into_iter().flatten().flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            let Some(folder) = name.strip_suffix(".zip").and_then(|s| s.rsplit_once('-')).map(|(f, _)| f.to_lowercase()) else { continue };
            *out.entry(folder).or_default() += file.metadata().map_or(0, |m| m.len());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([0u8; 40]);
        b
    }

    fn dds(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"DDS \x7c\0\0\0\x07\x10\0\0".to_vec();
        b.extend(h.to_le_bytes());
        b.extend(w.to_le_bytes());
        b.extend([0u8; 108]);
        b
    }

    fn put(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_image_headers() {
        assert_eq!(png_size(&png(64, 32)), Some((64, 32)));
        assert_eq!(dds_size(&dds(128, 256)), Some((128, 256)));
        assert_eq!(png_size(b"not a png at all, nope nope"), None);
        assert_eq!(dds_size(&png(1, 1)), None);
    }

    #[test]
    fn default_folders_pick_newest_version_not_above_game() {
        let m = temp("rimkit_tex_folders");
        for d in ["Common", "1.5", "1.6", "1.8", "Source"] {
            fs::create_dir_all(m.join(d)).unwrap();
        }
        let got = content_folders(&m, "1.7");
        assert_eq!(got, [m.clone(), m.join("Common"), m.join("1.6")]);
    }

    #[test]
    fn load_folders_xml_wins() {
        let m = temp("rimkit_tex_loadfolders");
        fs::create_dir_all(m.join("1.7")).unwrap();
        fs::write(m.join("loadFolders.xml"), "<loadFolders><v1.6><li>1.6</li></v1.6><v1.7><li>/</li><li>Extra</li><li IfModActive=\"x.y\">Compat/X</li></v1.7></loadFolders>").unwrap();
        assert_eq!(content_folders(&m, "1.7"), [m.clone(), m.join("Extra"), m.join("Compat/X")]);
    }

    #[test]
    fn scan_counts_loaded_textures_and_dds_shadowing() {
        let m = temp("rimkit_tex_scan");
        put(&m.join("Textures/Things/a.png"), &png(64, 32));
        put(&m.join("Textures/Things/b.png"), &png(128, 128));
        put(&m.join("Textures/Things/B.dds"), &dds(128, 128));
        put(&m.join("1.7/Textures/c.png"), &png(16, 16));
        put(&m.join("1.6/Textures/old.png"), &png(512, 512));
        put(&m.join("Textures/broken.png"), b"junk");
        let t = scan(&m, "1.7");
        assert_eq!((t.png, t.dds, t.unreadable), (3, 1, 1), "b.png is shadowed, 1.6 not loaded");
        assert_eq!(t.pixels, 64 * 32 + 128 * 128 + 16 * 16);
        assert_eq!(t.vram_bytes(), t.pixels * 4 / 3);
    }

    #[test]
    fn cache_index_sums_zips_per_mod_folder() {
        let c = temp("rimkit_tex_cache");
        put(&c.join("v23/bc7x/2009463077-aaaa.zip"), &[0; 10]);
        put(&c.join("v23/bc7x/2009463077-bbbb.zip"), &[0; 5]);
        put(&c.join("v23/bc7x/2009463077-bbbb.zip.lock"), &[0; 99]);
        put(&c.join("v23/bc7x/Core-cccc.zip"), &[0; 7]);
        put(&c.join("installs/x/pages.x.zip"), &[0; 3]);
        let idx = cache_index(&c);
        assert_eq!(idx.get("2009463077"), Some(&15));
        assert_eq!(idx.get("core"), Some(&7));
        assert_eq!(idx.len(), 2);
    }

    /// Real install, read-only: `cargo test --release real_textures -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_textures() {
        use crate::model::{mod_info, Settings, Source};
        let s = Settings::default();
        let roots = [(s.game_dir.join("Data"), Source::Official), (s.game_dir.join("Mods"), Source::Local), (s.workshop_dir.clone(), Source::Workshop)];
        let mods = mod_info::scan(&roots, "1.7");
        let t0 = std::time::Instant::now();
        let mut all: Vec<(String, ModTextures)> = std::thread::scope(|sc| {
            let hs: Vec<_> = mods.chunks(mods.len().div_ceil(8)).map(|c| sc.spawn(move || c.iter().map(|m| (m.name.clone(), scan(&m.path, "1.7"))).collect::<Vec<_>>())).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let cache = cache_index(&s.config_dir.parent().unwrap().join("TextureCache"));
        let n: usize = all.iter().map(|(_, t)| t.count()).sum();
        let vram: u64 = all.iter().map(|(_, t)| t.vram_bytes()).sum();
        let png: u64 = all.iter().map(|(_, t)| t.png_bytes).sum();
        println!("{n} textures, {} MB PNG, est VRAM {} MB, scanned in {:?}; cache entries {}", png >> 20, vram >> 20, t0.elapsed(), cache.len());
        all.sort_by(|a, b| b.1.vram_bytes().cmp(&a.1.vram_bytes()));
        for (name, t) in all.iter().take(10) {
            println!("{:>6} MB VRAM {:>5} tex {:>4} MB png {:>3} dds  {name}", t.vram_bytes() >> 20, t.count(), t.png_bytes >> 20, t.dds);
        }
    }
}
