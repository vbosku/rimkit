//! Startup timeline of the newest launch, from Player.log: where the loading time went.
//!
//! Phases are cut at marker lines seen in real 1.7 logs. Startup ends at the last line before the
//! first long silence (the main menu waiting for the player), so idle time never counts as loading.

use super::logs::{self, ModIndex};
use super::{mod_info, time};
use std::io::BufRead;

/// A silence this long after the XML phase means the game reached the main menu.
pub const IDLE_SECS: f64 = 20.0;
/// Silences at least this long are listed, blaming whatever logged just before.
pub const GAP_SECS: f64 = 1.0;
const MAX_GAPS: usize = 15;

pub const PHASES: [&str; 5] = ["Engine start", "Mod assemblies", "Mod startup code", "XML and patches", "Defs, translations, textures"];

/// Gagarin log lines that mean its XML cache was thrown away this launch.
const CACHE_MISS: [&str; 4] = ["clearing cache", "Mod list changed", "Cache not found", "Cache disabled"];

#[derive(Clone, Debug, PartialEq)]
pub struct Phase {
    pub name: &'static str,
    pub secs: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
    /// Seconds after the launch started.
    pub at: f64,
    pub secs: f64,
    /// First line of the message logged just before the silence.
    pub before: String,
    pub mod_id: Option<String>,
}

/// What the Gagarin mod (XML cache) did this launch.
#[derive(Clone, Debug, PartialEq)]
pub enum XmlCache {
    Used,
    Rebuilt(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Startup {
    /// "2026-10-20 21:55:04"
    pub started: String,
    pub phases: Vec<Phase>,
    pub total: f64,
    /// Longest first.
    pub gaps: Vec<Gap>,
    pub xml_cache: Option<XmlCache>,
    /// Seconds after launch when a save started loading, if one did.
    pub save_load_at: Option<f64>,
}

pub fn analyze(mut reader: impl BufRead, index: &ModIndex) -> Option<Startup> {
    let mut started: Option<(f64, String)> = None;
    let mut entries: Vec<(f64, String)> = Vec::new();
    let mut save_load_at = None;
    let mut buf = Vec::new();
    while reader.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
        let line = String::from_utf8_lossy(logs::trim_eol(&buf)).into_owned();
        buf.clear();
        if let Some(rest) = line.strip_prefix("Started:") {
            let ts = rest.trim().get(..23).unwrap_or("");
            started = time::parse_timestamp(ts).map(|t| (t, ts[..19].to_string()));
            continue;
        }
        let Some((ts, _, rest)) = logs::header(line.as_bytes()) else { continue };
        let Some(t) = std::str::from_utf8(ts).ok().and_then(time::parse_timestamp) else { continue };
        let rest = String::from_utf8_lossy(rest);
        let message = rest.strip_prefix('[').and_then(|r| r.split_once("] ")).map_or(rest.as_ref(), |(_, m)| m).to_string();
        if message.starts_with("Loading game from file") {
            save_load_at = Some(t);
            break;
        }
        entries.push((t, message));
    }
    if entries.is_empty() {
        return None;
    }
    let (t0, started) = started.unwrap_or_else(|| (entries[0].0, String::new()));

    let find = |from: usize, hit: &dyn Fn(&str) -> bool| (from..entries.len()).find(|&i| hit(&entries[i].1));
    let game = find(0, &|m| m.starts_with("RimWorld 1."));
    let harmony = find(game.unwrap_or(0), &|m| m.contains("Harmony Mod v"));
    let xml = find(harmony.or(game).unwrap_or(0), &|m| m.starts_with("GAGARIN:") || m.contains("Patch operation"));
    let xml_end = xml.and_then(|x| find(x, &|m| m.contains("Cache created!")).or_else(|| entries.iter().rposition(|(_, m)| m.contains("Patch operation"))));
    let after = xml_end.or(xml).or(harmony).or(game).unwrap_or(0);
    let end = (after..entries.len() - 1).find(|&i| entries[i + 1].0 - entries[i].0 >= IDLE_SECS).unwrap_or(entries.len() - 1);

    // A missing marker gives its phase's time to the next phase.
    let at = |i: Option<usize>| i.filter(|&i| i <= end).map(|i| entries[i].0);
    let marks = [at(game), at(harmony), at(xml), at(xml_end), Some(entries[end].0)];
    let mut phases = Vec::new();
    let mut prev = t0;
    for (name, mark) in PHASES.iter().zip(marks) {
        let next = mark.unwrap_or(prev).max(prev);
        if next > prev {
            phases.push(Phase { name, secs: next - prev });
        }
        prev = next;
    }

    let mut gaps: Vec<Gap> = (0..end)
        .filter(|&i| entries[i + 1].0 - entries[i].0 >= GAP_SECS)
        .map(|i| Gap {
            at: entries[i].0 - t0,
            secs: entries[i + 1].0 - entries[i].0,
            before: mod_info::clean_description(&entries[i].1),
            mod_id: index.attribute(&entries[i].1),
        })
        .collect();
    gaps.sort_by(|a, b| b.secs.total_cmp(&a.secs));
    gaps.truncate(MAX_GAPS);

    let gagarin: Vec<&str> = entries.iter().filter_map(|(_, m)| m.strip_prefix("GAGARIN:")).map(str::trim).collect();
    let xml_cache = match gagarin.iter().find(|m| CACHE_MISS.iter().any(|k| m.contains(k))) {
        Some(reason) => Some(XmlCache::Rebuilt(mod_info::clean_description(reason))),
        None => (!gagarin.is_empty()).then_some(XmlCache::Used),
    };

    Some(Startup { started, phases, total: entries[end].0 - t0, gaps, xml_cache, save_load_at: save_load_at.map(|t| t - t0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "=== Waylith log ===
Started:   2026-10-20 23:59:50.000 +02:00
Version:   1.7.4984.1242

2026-10-20 23:59:52.000  INFO [VulkanStartup] Vulkan loader: bundled
    at Brutal.Logging.LogCategory.Info() [0x0]
2026-10-20 23:59:53.000  INFO [Verse] RimWorld 1.7.4984 rev1242
2026-10-20 23:59:54.000  WARN [Verse] Mod Foo dependency needs a downloadUrl
2026-10-21 00:00:10.000  INFO [Verse] Harmony Mod v3.0.1.0; Harmony library v3.0.0.0
2026-10-21 00:00:12.000  INFO [Verse] [Dubs] Loaded
2026-10-21 00:00:15.000  WARN [Verse] GAGARIN: Mod list changed clearing cache
2026-10-21 00:00:30.000  WARN [Verse] GAGARIN: <color=white>Cache created!</color> creating cache took 0 seconds
2026-10-21 00:00:30.500 ERROR [Verse] [Dubs] Patch operation failed
2026-10-21 00:00:40.000  INFO [Verse] [ShowMeYourHands]: Defined hand definitions of 412 weapons
2026-10-21 00:01:50.000  INFO [Verse] Loading game from file Colony with mods:
2026-10-21 00:01:55.000  INFO [Verse] after load
";

    fn run() -> Startup {
        let mods = vec![crate::model::ModInfo { id: "dubs.mod".into(), name: "Dubs".into(), ..Default::default() }];
        analyze(LOG.as_bytes(), &ModIndex::new(&mods)).unwrap()
    }

    #[test]
    fn phases_follow_markers_across_midnight() {
        let s = run();
        assert_eq!(s.started, "2026-10-20 23:59:50");
        let secs: Vec<(&str, f64)> = s.phases.iter().map(|p| (p.name, p.secs)).collect();
        assert_eq!(
            secs,
            [
                ("Engine start", 3.0),
                ("Mod assemblies", 17.0),
                ("Mod startup code", 5.0),
                ("XML and patches", 15.0),
                ("Defs, translations, textures", 10.0)
            ]
        );
        assert_eq!(s.total, 50.0);
    }

    #[test]
    fn main_menu_idle_is_not_loading_time() {
        let s = run();
        assert_eq!(s.save_load_at, Some(120.0));
        assert!(s.gaps.iter().all(|g| g.secs < 70.0), "the 70 s main-menu wait is not a loading gap");
    }

    #[test]
    fn gaps_are_longest_first_and_blamed() {
        let s = run();
        assert_eq!(s.gaps[0].secs, 16.0);
        assert_eq!(s.gaps[0].before, "Mod Foo dependency needs a downloadUrl");
        assert_eq!(s.gaps[0].at, 4.0);
        let dubs = s.gaps.iter().find(|g| g.before.starts_with("[Dubs] Patch")).unwrap();
        assert_eq!((dubs.secs, dubs.mod_id.as_deref()), (9.5, Some("dubs.mod")));
    }

    #[test]
    fn gagarin_rebuild_reason() {
        assert_eq!(run().xml_cache, Some(XmlCache::Rebuilt("Mod list changed clearing cache".into())));
    }

    #[test]
    fn not_a_log() {
        assert_eq!(analyze("hello\nworld\n".as_bytes(), &ModIndex::default()), None);
    }

    /// Real Player.log, read-only: `cargo test --release real_startup -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_startup() {
        let s = crate::model::Settings::default();
        let path = s.config_dir.parent().unwrap().join("Logs").join("Player.log");
        let t = std::time::Instant::now();
        let r = analyze(std::io::BufReader::new(std::fs::File::open(path).unwrap()), &ModIndex::default()).unwrap();
        println!("parsed in {:?}; started {} total {:.1}s save at {:?} cache {:?}", t.elapsed(), r.started, r.total, r.save_load_at, r.xml_cache);
        for p in &r.phases { println!("  {:<30} {:6.1}s", p.name, p.secs); }
        for g in r.gaps.iter().take(6) { println!("  gap {:5.1}s at +{:5.1}s after: {}", g.secs, g.at, g.before.chars().take(90).collect::<String>()); }
    }
}
