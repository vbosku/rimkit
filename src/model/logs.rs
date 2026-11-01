//! RimWorld 1.7 (Waylith) Player.log analysis: warnings and errors, grouped and blamed on a mod.
//!
//! Entry format: `2026-10-20 21:55:58.616 ERROR [Verse] message`, followed by continuation lines
//! (message lines, stack frames `at Ns.Type.Method`, Harmony patch lines `- POSTFIX owner: ...`).

use super::ModInfo;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Level {
    Warning,
    Error,
}

/// Identical messages (numbers ignored) collapse into one group.
#[derive(Clone, Debug)]
pub struct Group {
    pub level: Level,
    pub category: String,
    /// First line of the message, as written the first time.
    pub title: String,
    /// Full text of the first occurrence (capped).
    pub text: String,
    pub count: usize,
    pub first: String,
    pub last: String,
    pub mod_id: Option<String>,
}

#[derive(Default, Debug)]
pub struct Report {
    /// Errors first, then by count.
    pub groups: Vec<Group>,
    pub lines: usize,
    /// Warning + error entries before grouping.
    pub entries: usize,
    /// False if no Waylith-style entry header was found at all.
    pub recognized: bool,
    /// Timestamp of the engine's "Device-local memory is over budget" note: the GPU ran out of VRAM.
    pub vram_over_budget: Option<String>,
}

/// Namespaces that belong to the game, engine or runtime, never to a mod.
const GAME_NAMESPACES: [&str; 11] =
    ["verse", "rimworld", "rimworldwaylith", "system", "harmonylib", "brutal", "waylith", "unitystubs", "unityengine", "mono", "microsoft"];
const MAX_LINES: usize = 80;
const TS_LEN: usize = 23; // "2026-10-20 21:55:58.616"

/// Lookups used to blame a log entry on a mod.
#[derive(Default)]
pub struct ModIndex {
    by_name: HashMap<String, String>,
    by_folder: HashMap<String, String>,
    by_namespace: HashMap<String, String>,
    by_id: HashMap<String, String>,
}

impl ModIndex {
    pub fn new(mods: &[ModInfo]) -> Self {
        let mut idx = Self::default();
        for m in mods {
            let id = || m.id.clone();
            idx.by_name.entry(m.name.to_lowercase()).or_insert_with(id);
            idx.by_id.entry(m.id.clone()).or_insert_with(id);
            if let Some(folder) = m.path.file_name() {
                idx.by_folder.entry(folder.to_string_lossy().to_lowercase()).or_insert_with(id);
            }
            let id_tail = m.id.rsplit('.').next().unwrap_or(&m.id);
            for ns in m.assemblies.iter().map(String::as_str).chain([id_tail, m.name.as_str()]) {
                idx.by_namespace.entry(key(ns)).or_insert_with(id);
            }
        }
        idx
    }

    /// Best guess of the mod responsible for an entry's text, strongest evidence first:
    /// `[Mod Name]` prefix, a path inside the mod folder, a stack frame from its assembly, a Harmony patch it owns.
    pub fn attribute(&self, text: &str) -> Option<String> {
        self.by_prefix(text).or_else(|| self.by_path(text)).or_else(|| self.by_frame(text)).or_else(|| self.by_patch(text)).cloned()
    }

    fn by_prefix(&self, text: &str) -> Option<&String> {
        let name = text.strip_prefix('[')?.split(']').next()?;
        self.by_name.get(&name.to_lowercase())
    }

    fn by_path(&self, text: &str) -> Option<&String> {
        let lower = text.to_lowercase().replace('/', "\\");
        ["294100\\", "\\mods\\"].iter().find_map(|marker| {
            lower.match_indices(marker).find_map(|(i, _)| {
                let folder = lower[i + marker.len()..].split(|c: char| c == '\\' || c.is_whitespace()).next()?;
                self.by_folder.get(folder)
            })
        })
    }

    fn by_frame(&self, text: &str) -> Option<&String> {
        text.lines().filter_map(|l| l.trim_start().strip_prefix("at ")).find_map(|frame| {
            let ns = key(frame.trim_start_matches('.').split(['.', '+', '(', ':']).next()?);
            if GAME_NAMESPACES.contains(&ns.as_str()) { None } else { self.by_namespace.get(&ns) }
        })
    }

    fn by_patch(&self, text: &str) -> Option<&String> {
        text.lines().filter_map(|l| l.trim_start().strip_prefix("- ")).find_map(|l| {
            let mut words = l.split_whitespace();
            let kind = words.next()?;
            if !["PREFIX", "POSTFIX", "TRANSPILER", "FINALIZER"].contains(&kind) {
                return None;
            }
            let owner = words.next()?.trim_end_matches(':').to_lowercase();
            self.by_id.get(&owner).or_else(|| self.by_namespace.get(&key(owner.rsplit('.').next()?)))
        })
    }
}

/// Lowercase alphanumerics only, for fuzzy namespace/assembly matching.
fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Lowercase, and every alphanumeric token holding a digit becomes `#`,
/// so "Lord_2169" / "Lord_2417" and "0x26E7…" / "0x26F1…" group together.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.chars().any(|c| c.is_ascii_digit()) {
            out.push('#');
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// An entry being read: everything up to the next header line.
struct Entry {
    level: Level,
    category: String,
    ts: String,
    text: String,
    lines: usize,
}

/// Streams the log; `progress` gets the bytes read so far every few MB.
pub fn parse(mut reader: impl BufRead, index: &ModIndex, progress: impl Fn(u64)) -> Report {
    let mut report = Report::default();
    let mut slots: HashMap<(Level, String), usize> = HashMap::new();
    let mut current: Option<Entry> = None;
    let mut buf = Vec::with_capacity(4096);
    let (mut read, mut reported) = (0u64, 0u64);

    let mut finish = |entry: Option<Entry>, report: &mut Report| {
        let Some(e) = entry else { return };
        report.entries += 1;
        let title = e.text.lines().next().unwrap_or("").to_string();
        match slots.get(&(e.level, normalize(&title))) {
            Some(&i) => {
                let g = &mut report.groups[i];
                g.count += 1;
                g.last = e.ts;
            }
            None => {
                slots.insert((e.level, normalize(&title)), report.groups.len());
                let mod_id = index.attribute(&e.text);
                report.groups.push(Group { level: e.level, category: e.category, title, text: e.text, count: 1, first: e.ts.clone(), last: e.ts, mod_id });
            }
        }
    };

    while let Ok(n) = reader.read_until(b'\n', &mut buf) {
        if n == 0 {
            break;
        }
        read += n as u64;
        report.lines += 1;
        if read - reported > 4 << 20 {
            reported = read;
            progress(read);
        }
        let line = trim_eol(&buf);
        if let Some((ts, level, rest)) = header(line) {
            report.recognized = true;
            if level.is_none() && report.vram_over_budget.is_none() && rest.starts_with(b"[TextureResidency]") && rest.windows(11).any(|w| w == b"over budget") {
                report.vram_over_budget = Some(String::from_utf8_lossy(ts).into());
            }
            finish(current.take(), &mut report);
            current = level.map(|level| {
                let rest = String::from_utf8_lossy(rest);
                let (category, message) = match rest.strip_prefix('[').and_then(|r| r.split_once("] ")) {
                    Some((c, m)) => (c.to_string(), m.to_string()),
                    None => (String::new(), rest.to_string()),
                };
                Entry { level, category, ts: String::from_utf8_lossy(ts).into(), text: message, lines: 1 }
            });
        } else if let Some(e) = current.as_mut().filter(|e| e.lines < MAX_LINES) {
            e.text.push('\n');
            e.text.push_str(&String::from_utf8_lossy(line));
            e.lines += 1;
        }
        buf.clear();
    }
    finish(current.take(), &mut report);
    progress(read);
    report.groups.sort_by(|a, b| b.level.cmp(&a.level).then(b.count.cmp(&a.count)));
    report
}

pub(crate) fn trim_eol(line: &[u8]) -> &[u8] {
    let end = line.iter().rposition(|&b| b != b'\n' && b != b'\r').map_or(0, |i| i + 1);
    &line[..end]
}

/// `2026-10-20 21:55:58.616 ERROR [Verse] msg` -> (timestamp, Some(level) or None for INFO/DEBUG, rest).
pub(crate) fn header(line: &[u8]) -> Option<(&[u8], Option<Level>, &[u8])> {
    let is_ts = line.len() > TS_LEN
        && line[..4].iter().all(u8::is_ascii_digit)
        && line[4] == b'-'
        && line[7] == b'-'
        && line[10] == b' '
        && line[13] == b':'
        && line[TS_LEN] == b' ';
    if !is_ts {
        return None;
    }
    let after = &line[TS_LEN..];
    let start = after.iter().position(|&b| b != b' ')?;
    let after = &after[start..];
    let end = after.iter().position(|&b| b == b' ').unwrap_or(after.len());
    let level = match &after[..end] {
        b"ERROR" | b"FATAL" | b"CRITICAL" | b"EXCEPTION" => Some(Level::Error),
        b"WARN" | b"WARNING" => Some(Level::Warning),
        _ => None,
    };
    let rest = after.get(end + 1..).unwrap_or(&[]);
    Some((&line[..TS_LEN], level, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mods() -> Vec<ModInfo> {
        let m = |id: &str, name: &str, path: &str, asm: &[&str]| ModInfo {
            id: id.into(),
            name: name.into(),
            path: PathBuf::from(path),
            assemblies: asm.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        vec![
            m("epona.facial", "Epona Facial Animation（Dynastic Rise）", r"C:\Steam\workshop\content\294100\111", &[]),
            m("mu.milk", "poisonless mu girl milk", r"C:\Steam\workshop\content\294100\3601148404", &[]),
            m("ferny.surveillanceshift", "Surveillance Shift", r"C:\Steam\workshop\content\294100\222", &["surveillanceshift"]),
            m("readable.numbers", "Readable Numbers", r"C:\RimWorld\Mods\ReadableNumbers", &["rimworldreadablenumbers"]),
        ]
    }

    const LOG: &str = "=== Waylith log ===
Version:   1.7.4984.1242

2026-10-20 21:55:05.472  INFO [VulkanStartup] Vulkan loader: bundled
    at Brutal.Logging.LogCategory.Info(LogString`1 message) [0x0]
2026-10-20 21:55:58.616 ERROR [Verse] [Epona Facial Animation（Dynastic Rise）] Patch operation failed
file: C:\\Steam\\workshop\\content\\294100\\111\\Patches\\x.xml
    at Verse.PatchOperation.Complete(String modIdentifier) [0x34]
2026-10-20 21:55:58.617 ERROR [Verse] Patch operation Verse.PatchOperationRemove(/Defs/ThingDef) failed
file: C:\\Steam\\workshop\\content\\294100\\3601148404\\1.6\\Patches\\patch.xml
2026-10-20 21:56:31.104 ERROR [Verse] Error in static constructor of RimworldReadableNumbers.RnPatcher
   at HarmonyLib.PatchClassProcessor.Patch()
   at RimworldReadableNumbers.Patches.Thing.Method()
2026-10-20 21:57:49.467 ERROR [Verse] SaveableFromNode exception: System.NullReferenceException
   at Verse.Find.get_FactionManager()
    - POSTFIX ferny.SurveillanceShift: Verse.Pawn.ExposeData <- Void Patch:Postfix(Pawn __instance)
2026-10-20 21:58:00.001  WARN [Verse] Could not resolve reference to object with loadID Lord_2169
2026-10-20 21:58:00.002  WARN [Verse] Could not resolve reference to object with loadID Lord_2417
2026-10-20 21:58:00.003  WARN [Verse] Could not resolve reference to object with loadID Lord_33
2026-10-20 21:58:01.000 ERROR [Verse] Cannot get current title for null faction.
    at RimWorld.Pawn_RoyaltyTracker.GetCurrentTitleInFaction(Faction faction) [0x3]
";

    fn report() -> Report {
        parse(LOG.as_bytes(), &ModIndex::new(&mods()), |_| {})
    }

    #[test]
    fn normalize_ignores_numbers_and_case() {
        assert_eq!(normalize("Lord_2169 at X"), normalize("lord_33 at x"));
        assert_ne!(normalize("abc"), normalize("abd"));
        assert_eq!(normalize("allocation 0x26E72840088 (250000 bytes)"), normalize("allocation 0x26F1AA7C618 (16 bytes)"));
    }

    #[test]
    fn skips_info_and_groups_repeats() {
        let r = report();
        assert!(r.recognized);
        assert_eq!(r.entries, 8);
        assert_eq!(r.groups.len(), 6);
        let lords = r.groups.iter().find(|g| g.title.contains("Lord_")).unwrap();
        assert_eq!((lords.count, lords.level), (3, Level::Warning));
        assert_eq!(lords.title, "Could not resolve reference to object with loadID Lord_2169");
        assert_eq!((lords.first.as_str(), lords.last.as_str()), ("2026-10-20 21:58:00.001", "2026-10-20 21:58:00.003"));
        assert!(!r.groups.iter().any(|g| g.title.contains("Vulkan")));
    }

    #[test]
    fn errors_sorted_first() {
        let r = report();
        let first_warning = r.groups.iter().position(|g| g.level == Level::Warning).unwrap();
        assert!(r.groups[..first_warning].iter().all(|g| g.level == Level::Error));
        assert_eq!(r.groups.len() - first_warning, 1);
    }

    #[test]
    fn keeps_category_and_stack_text() {
        let g = report().groups.into_iter().find(|g| g.title.starts_with("Cannot get current title")).unwrap();
        assert_eq!(g.category, "Verse");
        assert!(g.text.contains("Pawn_RoyaltyTracker.GetCurrentTitleInFaction"));
    }

    #[test]
    fn blames_the_right_mod() {
        let r = report();
        let blame = |needle: &str| r.groups.iter().find(|g| g.title.contains(needle)).unwrap().mod_id.clone();
        assert_eq!(blame("[Epona Facial").as_deref(), Some("epona.facial"));
        assert_eq!(blame("PatchOperationRemove").as_deref(), Some("mu.milk"));
        assert_eq!(blame("RnPatcher").as_deref(), Some("readable.numbers"));
        assert_eq!(blame("SaveableFromNode").as_deref(), Some("ferny.surveillanceshift"));
        assert_eq!(blame("null faction"), None);
    }

    #[test]
    fn local_mods_folder_paths_are_blamed() {
        let idx = ModIndex::new(&mods());
        let text = "Failed to load\nfile: C:\\RimWorld\\Mods\\ReadableNumbers\\Defs\\a.xml";
        assert_eq!(idx.attribute(text).as_deref(), Some("readable.numbers"));
    }

    #[test]
    fn notes_when_vram_ran_out() {
        let log = "2026-10-20 22:10:56.689  INFO [TextureResidency] Device-local memory is over budget. 'Pattern_Gloss' and later GameTextures go to system memory.\n";
        let r = parse(log.as_bytes(), &ModIndex::default(), |_| {});
        assert_eq!(r.vram_over_budget.as_deref(), Some("2026-10-20 22:10:56.689"));
        assert_eq!(report().vram_over_budget, None);
    }

    #[test]
    fn unknown_format_is_flagged() {
        let r = parse("just some text\nnothing here\n".as_bytes(), &ModIndex::default(), |_| {});
        assert!(!r.recognized);
        assert_eq!(r.lines, 2);
    }

    /// Parses the real Player.log; run with `cargo test --release real_log -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_log() {
        use crate::model::{mod_info, Settings, Source};
        let s = Settings::default();
        let roots = [(s.game_dir.join("Data"), Source::Official), (s.game_dir.join("Mods"), Source::Local), (s.workshop_dir.clone(), Source::Workshop)];
        let t = std::time::Instant::now();
        let mods = mod_info::scan(&roots, "1.7");
        println!("scan with assemblies: {:?}", t.elapsed());
        let path = s.config_dir.parent().unwrap().join("Logs").join("Player.log");
        let t = std::time::Instant::now();
        let file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).unwrap());
        let r = parse(file, &ModIndex::new(&mods), |_| {});
        println!("{} lines, {} entries, {} groups in {:?}", r.lines, r.entries, r.groups.len(), t.elapsed());
        for g in r.groups.iter().take(25) {
            let title: String = g.title.chars().take(90).collect();
            println!("{:?} x{:<5} {:<28} {}", g.level, g.count, g.mod_id.as_deref().unwrap_or("-"), title);
        }
        let blamed = r.groups.iter().filter(|g| g.mod_id.is_some()).count();
        println!("blamed {blamed}/{}", r.groups.len());
    }
}
