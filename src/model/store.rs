//! App settings and named modlists, stored as JSON in %APPDATA%\rimkit.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub game_dir: PathBuf,
    pub workshop_dir: PathBuf,
    pub config_dir: PathBuf,
    pub current_list: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let game_dir = PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\RimWorld");
        let workshop_dir = PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\workshop\content\294100");
        let config_dir = env_dir("USERPROFILE").join(r"AppData\LocalLow\Ludeon Studios\RimWorld by Ludeon Studios\Config");
        Self { game_dir, workshop_dir, config_dir, current_list: None }
    }
}

impl Settings {
    pub fn load() -> Self {
        read_json(&app_dir().join("settings.json")).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        write_json(&app_dir().join("settings.json"), self)
    }

    pub fn mods_config(&self) -> PathBuf {
        self.config_dir.join("ModsConfig.xml")
    }
}

pub fn list_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(lists_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn load_list(name: &str) -> io::Result<Vec<String>> {
    read_json(&list_path(name))
}

pub fn save_list(name: &str, ids: &[String]) -> io::Result<()> {
    write_json(&list_path(name), &ids)
}

pub fn delete_list(name: &str) -> io::Result<()> {
    fs::remove_file(list_path(name))
}

/// Keeps a list name usable as a file name.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c)).collect::<String>().trim().to_string()
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_default()
}

fn app_dir() -> PathBuf {
    env_dir("APPDATA").join("rimkit")
}

/// Timestamped ModsConfig.xml copies taken before each save.
pub fn backups_dir() -> PathBuf {
    app_dir().join("backups")
}

fn lists_dir() -> PathBuf {
    app_dir().join("lists")
}

fn list_path(name: &str) -> PathBuf {
    lists_dir().join(format!("{}.json", clean_name(name)))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> io::Result<T> {
    serde_json::from_str(&fs::read_to_string(path)?).map_err(io::Error::other)
}

fn write_json<T: Serialize + ?Sized>(path: &PathBuf, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(value).map_err(io::Error::other)?)
}

/// One past launch's startup timing, kept to compare launches (Player-prev.log only holds one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub started: String,
    pub total: f64,
    pub phases: Vec<(String, f64)>,
}

pub const HISTORY_KEEP: usize = 30;

/// `history` plus `record` (replacing one with the same start time), newest first, at most `HISTORY_KEEP`.
pub fn merge_history(history: Vec<LaunchRecord>, record: LaunchRecord) -> Vec<LaunchRecord> {
    let mut out: Vec<LaunchRecord> = history.into_iter().filter(|r| r.started != record.started).collect();
    out.push(record);
    // "YYYY-MM-DD HH:MM:SS" sorts chronologically as text.
    out.sort_by(|a, b| b.started.cmp(&a.started));
    out.truncate(HISTORY_KEEP);
    out
}

pub fn load_history() -> Vec<LaunchRecord> {
    read_json(&app_dir().join("startup_history.json")).unwrap_or_default()
}

pub fn save_history(history: &[LaunchRecord]) -> io::Result<()> {
    write_json(&app_dir().join("startup_history.json"), history)
}

/// Window size to reopen with. Position isn't kept: the window always opens centered.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

pub const MIN_WINDOW: [f32; 2] = [800.0, 400.0];
const MAX_WINDOW: [f32; 2] = [7680.0, 4320.0];

impl Default for WindowState {
    fn default() -> Self {
        Self { width: 1400.0, height: 850.0, maximized: false }
    }
}

impl WindowState {
    /// Within sane limits, whatever got saved (a minimized or half-created window can report anything).
    pub fn sane(self) -> Self {
        if !(self.width.is_finite() && self.height.is_finite()) {
            return Self::default();
        }
        let width = self.width.clamp(MIN_WINDOW[0], MAX_WINDOW[0]);
        let height = self.height.clamp(MIN_WINDOW[1], MAX_WINDOW[1]);
        Self { width, height, ..self }
    }
}

pub fn load_window() -> WindowState {
    read_json::<WindowState>(&app_dir().join("window.json")).map(WindowState::sane).unwrap_or_default()
}

pub fn save_window(state: &WindowState) -> io::Result<()> {
    write_json(&app_dir().join("window.json"), state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(started: &str, total: f64) -> LaunchRecord {
        LaunchRecord { started: started.into(), total, phases: vec![("Engine start".into(), 1.0)] }
    }

    #[test]
    fn history_is_newest_first_without_repeats() {
        let h = merge_history(vec![], rec("2026-10-20 21:00:00", 90.0));
        let h = merge_history(h, rec("2026-10-21 10:00:00", 80.0));
        let h = merge_history(h, rec("2026-10-20 21:00:00", 95.0));
        let starts: Vec<(&str, f64)> = h.iter().map(|r| (r.started.as_str(), r.total)).collect();
        assert_eq!(starts, [("2026-10-21 10:00:00", 80.0), ("2026-10-20 21:00:00", 95.0)]);
    }

    #[test]
    fn window_state_is_kept_sane() {
        let w = |width, height| WindowState { width, height, maximized: true }.sane();
        assert_eq!(w(1600.0, 900.0), WindowState { width: 1600.0, height: 900.0, maximized: true });
        assert_eq!(w(150.0, 180.0), WindowState { width: 800.0, height: 400.0, maximized: true }, "the tiny corner window");
        assert_eq!(w(99999.0, 99999.0), WindowState { width: 7680.0, height: 4320.0, maximized: true });
        assert_eq!(w(f32::NAN, 500.0), WindowState::default());
    }

    #[test]
    fn history_is_capped() {
        let mut h = Vec::new();
        for i in 0..40 {
            h = merge_history(h, rec(&format!("2026-10-{:02} 10:00:00", i % 28 + 1), i as f64));
        }
        assert!(h.len() <= HISTORY_KEEP);
    }
}
