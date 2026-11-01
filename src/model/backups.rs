//! Timestamped copies of ModsConfig.xml taken before every save; the newest `KEEP` are kept.

use super::mods_config;
use super::time::{civil_from_days, days_from_civil};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const KEEP: usize = 20;
const PREFIX: &str = "ModsConfig-";
const DAY: u64 = 86_400;

#[derive(Clone, Debug)]
pub struct Backup {
    pub path: PathBuf,
    /// When it was taken.
    pub time: SystemTime,
    /// Active mods in it.
    pub mods: usize,
}

/// Copies `source` into `dir` unless it's missing or identical to the newest backup.
/// Returns the new backup's path. Old backups beyond `keep` are deleted.
pub fn create(source: &Path, dir: &Path, now: SystemTime, keep: usize) -> io::Result<Option<PathBuf>> {
    let Ok(content) = fs::read(source) else { return Ok(None) };
    if list(dir).first().is_some_and(|newest| fs::read(&newest.path).is_ok_and(|old| old == content)) {
        return Ok(None);
    }
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name(now));
    if path.exists() {
        return Ok(None); // two saves within the same second: keep the older state
    }
    fs::write(&path, &content)?;
    for old in list(dir).into_iter().skip(keep) {
        let _ = fs::remove_file(old.path);
    }
    Ok(Some(path))
}

/// Backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let time = parse_name(path.file_name()?.to_str()?)?;
            let mods = mods_config::read_active(&path).map_or(0, |ids| ids.len());
            Some(Backup { path, time, mods })
        })
        .collect();
    out.sort_by(|a, b| b.time.cmp(&a.time));
    out
}

/// "ModsConfig-20261021-215012.xml" (UTC), so names sort by time.
pub fn file_name(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days((secs / DAY) as i64);
    let rem = secs % DAY;
    format!("{PREFIX}{y:04}{m:02}{d:02}-{:02}{:02}{:02}.xml", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(time: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(time).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..DAY => format!("{} h ago", secs / 3600),
        _ if secs < 2 * DAY => "1 day ago".into(),
        _ => format!("{} days ago", secs / DAY),
    }
}

fn parse_name(name: &str) -> Option<SystemTime> {
    let (date, clock) = name.strip_prefix(PREFIX)?.strip_suffix(".xml")?.split_once('-')?;
    if date.len() != 8 || clock.len() != 6 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let days = days_from_civil(num(&date[..4])?, num(&date[4..6])?, num(&date[6..])?);
    let secs = days * DAY as i64 + num(&clock[..2])? * 3600 + num(&clock[2..4])? * 60 + num(&clock[4..])?;
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rimkit_backups_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(ids: &[&str]) -> String {
        let lis: String = ids.iter().map(|id| format!("<li>{id}</li>")).collect();
        format!("<ModsConfigData><activeMods>{lis}</activeMods></ModsConfigData>")
    }

    #[test]
    fn names_are_utc_timestamps() {
        assert_eq!(file_name(at(1792619412)), "ModsConfig-20261021-215012.xml");
        assert_eq!(file_name(at(951782400)), "ModsConfig-20000229-000000.xml");
    }

    #[test]
    fn ago_is_human() {
        let now = at(1_000_000);
        assert_eq!(ago(at(1_000_000 - 20), now), "just now");
        assert_eq!(ago(at(1_000_000 - 5 * 60), now), "5 min ago");
        assert_eq!(ago(at(1_000_000 - 3 * 3600), now), "3 h ago");
        assert_eq!(ago(at(1_000_000 - 2 * 86400), now), "2 days ago");
        assert_eq!(ago(at(1_000_000 - 86400), now), "1 day ago");
    }

    #[test]
    fn copies_and_skips_duplicates() {
        let dir = temp("dupes");
        let src = dir.join("ModsConfig.xml");
        let out = dir.join("backups");
        fs::write(&src, config(&["a", "b"])).unwrap();

        let first = create(&src, &out, at(1000), KEEP).unwrap().expect("first backup");
        assert_eq!(fs::read_to_string(&first).unwrap(), config(&["a", "b"]));
        assert_eq!(create(&src, &out, at(2000), KEEP).unwrap(), None, "same content: no new backup");

        fs::write(&src, config(&["a"])).unwrap();
        assert!(create(&src, &out, at(3000), KEEP).unwrap().is_some());
        let all = list(&out);
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].time, all[0].mods), (at(3000), 1), "newest first");
        assert_eq!((all[1].time, all[1].mods), (at(1000), 2));
    }

    #[test]
    fn missing_source_is_not_an_error() {
        let dir = temp("missing");
        assert_eq!(create(&dir.join("nope.xml"), &dir.join("b"), at(1), KEEP).unwrap(), None);
    }

    #[test]
    fn keeps_only_the_newest() {
        let dir = temp("prune");
        let src = dir.join("ModsConfig.xml");
        let out = dir.join("backups");
        for i in 0..5u64 {
            fs::write(&src, config(&[&format!("m{i}")])).unwrap();
            create(&src, &out, at(1000 + i * 60), 3).unwrap();
        }
        let times: Vec<SystemTime> = list(&out).iter().map(|b| b.time).collect();
        assert_eq!(times, [at(1240), at(1180), at(1120)]);
    }

    #[test]
    fn ignores_unrelated_files() {
        let dir = temp("unrelated");
        fs::write(dir.join("notes.txt"), "hi").unwrap();
        fs::write(dir.join("ModsConfig-garbage.xml"), "x").unwrap();
        assert!(list(&dir).is_empty());
    }
}
