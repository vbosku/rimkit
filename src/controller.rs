//! Controller: owns the state and every action the view can trigger.
//! The view only reads public fields and calls `apply`/`poll`. Anything that touches the disk
//! for more than a blink runs as a background `Task`, so the UI never freezes.

use crate::model::backups::{self, Backup};
use crate::model::changes::{self, Changes};
use crate::model::issues::{self, Issue, Severity};
use crate::model::logs::{self, ModIndex, Report};
use crate::model::modlist_io::{self, Entry};
use crate::model::game;
use crate::model::startup::{self, Startup};
use crate::model::store::LaunchRecord;
use crate::model::textures::{self, ModTextures};
use crate::model::{mod_info, mods_config, sort, store, ModInfo, Settings, Source};
use crate::task::{self, Task, Waker};
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

/// Everything the view can ask for. The view never mutates state directly.
pub enum Action {
    Select(String),
    /// Activate these mods (in this order) at a position of the active list, or at the end.
    Activate(Vec<String>, Option<usize>),
    Deactivate(Vec<String>),
    /// Move these active mods, keeping their relative order, to just before the item now at `usize`.
    MoveTo(Vec<String>, usize),
    OpenFolder(String),
    Undo,
    Redo,
    Sort,
    Save,
    Discard,
    Reload,
    SelectList(Option<String>),
    SaveListAs(String),
    DeleteList,
    ApplySettings(Settings),
    LoadLog(LogFile),
    /// Load a backup's mods into the active list (undoable; written only on Save).
    RestoreBackup(PathBuf),
    OpenBackupsFolder,
    /// Replace the active list with the mods in a ModsConfig.xml / .rml / save file (undoable).
    Import(PathBuf),
    /// Write the active list as `<name>.rml` into the game's ModLists folder.
    ExportList(String),
    OpenModListsFolder,
    OpenUrl(String),
    /// Start RimWorld through Steam, optionally saving first (the game reads the saved list).
    Play { save_first: bool },
    ScanTextures,
    OpenTextureCache,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogFile {
    Current,
    Previous,
}

impl LogFile {
    pub fn file_name(self) -> &'static str {
        match self {
            LogFile::Current => "Player.log",
            LogFile::Previous => "Player-prev.log",
        }
    }
}

/// Result of a background rescan.
struct Scan {
    mods: Vec<ModInfo>,
    active: io::Result<Vec<String>>,
    game_version: String,
    backups: Vec<Backup>,
    millis: u128,
}

/// Result of a background save: what was written, and the backup list after it.
struct Imported {
    file: PathBuf,
    entries: io::Result<Option<Vec<Entry>>>,
}

/// A save's `<meta>` (with the mod list) sits at the top; no need to read 30+ MB.
const IMPORT_READ_LIMIT: u64 = 4 << 20;

struct Saved {
    snapshot: Vec<String>,
    backup: Option<PathBuf>,
    backups: Vec<Backup>,
}

pub struct LogResult {
    pub file: LogFile,
    pub report: io::Result<Report>,
    /// Startup timeline of the launch this log belongs to.
    pub startup: Option<Startup>,
    pub millis: u128,
}

/// Every mod's textures (by package id) and the engine's texture cache (by mod folder, lowercase).
pub struct TextureReport {
    pub mods: HashMap<String, ModTextures>,
    pub cache: HashMap<String, u64>,
    pub millis: u128,
}

impl TextureReport {
    pub fn cache_bytes(&self) -> u64 {
        self.cache.values().sum()
    }
}

pub struct Controller {
    pub settings: Settings,
    pub game_version: String,
    pub mods: Vec<ModInfo>,
    by_id: HashMap<String, usize>,
    pub active: Vec<String>,
    saved: Vec<String>,
    undo: Vec<Vec<String>>,
    redo: Vec<Vec<String>>,
    pub selected: Option<String>,
    pub lists: Vec<String>,
    pub status: String,
    /// Problems of the active list, refreshed on every change.
    pub issues: HashMap<String, Vec<Issue>>,
    /// Unsaved changes, refreshed on every change.
    pub changes: Changes,
    pub log: Option<LogResult>,
    /// Newest first.
    pub backups: Vec<Backup>,
    pub game_running: bool,
    pub textures: Option<TextureReport>,
    /// Past launches' startup timings, newest first.
    pub launches: Vec<LaunchRecord>,
    texture_task: Option<Task<TextureReport>>,
    /// Workshop ids learned from imported lists/saves, to link mods that aren't installed.
    known_steam_ids: HashMap<String, u64>,
    play_after_save: bool,
    game_events: Option<mpsc::Receiver<bool>>,
    import: Option<Task<Imported>>,
    waker: Waker,
    scan: Option<Task<Scan>>,
    save: Option<Task<io::Result<Saved>>>,
    log_task: Option<Task<LogResult>>,
}

impl Controller {
    /// Starts immediately with empty lists; the mod scan runs in the background.
    pub fn new(waker: Waker) -> Self {
        let mut c = Self::empty(Settings::load(), waker);
        c.lists = store::list_names();
        c.launches = store::load_history();
        c.reload();
        c.game_events = Some(watch_game(c.waker.clone()));
        c
    }

    fn empty(settings: Settings, waker: Waker) -> Self {
        Self {
            settings,
            game_version: String::new(),
            mods: Vec::new(),
            by_id: HashMap::new(),
            active: Vec::new(),
            saved: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            lists: Vec::new(),
            status: String::new(),
            issues: HashMap::new(),
            changes: Changes::default(),
            log: None,
            backups: Vec::new(),
            game_running: false,
            textures: None,
            launches: Vec::new(),
            texture_task: None,
            known_steam_ids: HashMap::new(),
            play_after_save: false,
            game_events: None,
            import: None,
            waker,
            scan: None,
            save: None,
            log_task: None,
        }
    }

    pub fn apply(&mut self, action: Action) {
        // While mods are (re)loading the lists are about to be replaced; ignore edits.
        let harmless = matches!(
            action,
            Action::Select(_) | Action::OpenFolder(_) | Action::LoadLog(_) | Action::OpenBackupsFolder | Action::OpenModListsFolder | Action::OpenUrl(_) | Action::OpenTextureCache
        );
        if self.loading() && !harmless {
            return;
        }
        match action {
            Action::Select(id) => self.selected = Some(id),
            Action::Activate(ids, at) => self.activate(&ids, at),
            Action::Deactivate(ids) => self.deactivate(&ids),
            Action::MoveTo(ids, target) => self.move_to(&ids, target),
            Action::OpenFolder(id) => self.open_folder(&id),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Sort => self.sort(),
            Action::Save => self.save(),
            Action::Discard => self.discard(),
            Action::Reload => self.reload(),
            Action::SelectList(name) => self.select_list(name),
            Action::SaveListAs(name) => self.save_list_as(&name),
            Action::DeleteList => self.delete_current_list(),
            Action::ApplySettings(s) => self.apply_settings(s),
            Action::LoadLog(file) => self.load_log(file),
            Action::RestoreBackup(path) => self.restore_backup(&path),
            Action::OpenBackupsFolder => self.open_path(store::backups_dir()),
            Action::Import(path) => self.import(path),
            Action::ExportList(name) => self.export_list(&name),
            Action::OpenModListsFolder => self.open_path(self.mod_lists_dir()),
            Action::OpenUrl(url) => self.open_target(&url),
            Action::Play { save_first } => self.play(save_first),
            Action::ScanTextures => self.scan_textures(),
            Action::OpenTextureCache => self.open_path(self.texture_cache_dir()),
        }
    }

    /// Picks up finished background work. Call once per frame.
    pub fn poll(&mut self) {
        if let Some(scan) = task::take(&mut self.scan) {
            self.finish_scan(scan);
        }
        if let Some(result) = task::take(&mut self.save) {
            self.finish_save(result);
        }
        if let Some(result) = task::take(&mut self.import) {
            self.finish_import(result);
        }
        let events: Vec<bool> = self.game_events.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for running in events {
            if self.game_running && !running {
                self.status = "RimWorld closed: reading its log again.".into();
                self.load_log(LogFile::Current);
            }
            self.game_running = running;
        }
        if let Some(result) = task::take(&mut self.log_task) {
            self.status = match &result.report {
                Ok(r) => format!("Read {} in {} ms: {} log entries.", result.file.file_name(), result.millis, r.entries),
                Err(e) => format!("Can't read {}: {e}", result.file.file_name()),
            };
            if let Some(s) = &result.startup {
                self.record_launch(s);
            }
            self.log = Some(result);
        }
        if let Some(report) = task::take(&mut self.texture_task) {
            let count: usize = report.mods.values().map(ModTextures::count).sum();
            self.status = format!("Scanned {count} textures in {} ms.", report.millis);
            self.textures = Some(report);
        }
    }

    /// Running background jobs as (label, progress 0..1 if known).
    pub fn jobs(&self) -> Vec<(String, Option<f32>)> {
        let mut out = Vec::new();
        if let Some(t) = &self.scan {
            out.push((t.label.clone(), None));
        }
        if let Some(t) = &self.save {
            out.push((t.label.clone(), None));
        }
        if let Some(t) = &self.import {
            out.push((t.label.clone(), None));
        }
        if let Some(t) = &self.log_task {
            out.push((t.label.clone(), Some(t.fraction())));
        }
        if let Some(t) = &self.texture_task {
            out.push((t.label.clone(), Some(t.fraction())));
        }
        out
    }

    pub fn loading(&self) -> bool {
        self.scan.is_some()
    }

    pub fn log_loading(&self) -> bool {
        self.log_task.is_some()
    }

    fn open_folder(&mut self, id: &str) {
        if let Some(path) = self.info(id).map(|m| m.path.clone()) {
            self.open_path(path);
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        let _ = std::fs::create_dir_all(&path);
        self.open_target(&path.to_string_lossy());
    }

    fn open_target(&mut self, target: &str) {
        if let Err(e) = game::open(target) {
            self.status = format!("Can't open {target}: {e}");
        }
    }

    // ---- queries ----

    pub fn info(&self, id: &str) -> Option<&ModInfo> {
        self.by_id.get(id).map(|&i| &self.mods[i])
    }

    pub fn inactive(&self) -> Vec<&ModInfo> {
        let active: HashSet<&str> = self.active.iter().map(String::as_str).collect();
        self.mods.iter().filter(|m| !active.contains(m.id.as_str())).collect()
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.active.iter().any(|a| a == id)
    }

    pub fn is_dirty(&self) -> bool {
        !self.changes.is_empty()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn issues_of(&self, id: &str) -> &[Issue] {
        self.issues.get(id).map_or(&[], Vec::as_slice)
    }

    /// Worst problem of a mod, if any.
    pub fn severity(&self, id: &str) -> Option<Severity> {
        self.issues_of(id).iter().map(Issue::severity).max()
    }

    pub fn issue_count(&self, severity: Severity) -> usize {
        issues::count(&self.issues, severity)
    }

    /// (severity, text, where to get the missing mod if known) for each problem of a mod.
    pub fn messages(&self, id: &str) -> Vec<(Severity, String, Option<String>)> {
        let link = |issue: &Issue| match issue {
            Issue::MissingDependency(dep) => self.info(id).and_then(|m| m.link_for(dep)).map(String::from).or_else(|| self.steam_link(dep)),
            Issue::NotInstalled => self.steam_link(id),
            _ => None,
        };
        self.issues_of(id).iter().map(|i| (i.severity(), i.message(|x| self.name_of(x).to_string()), link(i))).collect()
    }

    /// Workshop page of a mod, from its folder (installed) or from imported lists (not installed).
    pub fn steam_link(&self, id: &str) -> Option<String> {
        let steam_id = self.info(id).and_then(ModInfo::steam_id).or_else(|| self.known_steam_ids.get(id).copied())?;
        Some(game::workshop_url(steam_id))
    }

    /// (errors, warnings) the loaded log blames on this mod.
    pub fn log_counts(&self, id: &str) -> Option<(usize, usize)> {
        let report = self.log.as_ref()?.report.as_ref().ok()?;
        let mine = report.groups.iter().filter(|g| g.mod_id.as_deref() == Some(id));
        let (errors, warnings) = mine.fold((0, 0), |(e, w), g| match g.level {
            logs::Level::Error => (e + g.count, w),
            logs::Level::Warning => (e, w + g.count),
        });
        (errors + warnings > 0).then_some((errors, warnings))
    }

    /// True if some mod name needs glyphs beyond the default Latin/Greek/Cyrillic fonts.
    pub fn needs_cjk(&self) -> bool {
        self.mods.iter().any(|m| m.name.chars().any(|c| c >= '\u{2E80}'))
    }

    pub fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.info(id).map_or(id, |m| m.name.as_str())
    }

    // ---- edits (all undoable) ----

    /// Inserts the not-yet-active `ids`, in their order, at `at` (or the end). One undo step.
    pub fn activate(&mut self, ids: &[String], at: Option<usize>) {
        self.edit(|list| {
            let new: Vec<String> = mods_config::dedupe(ids.iter().filter(|id| !list.contains(id)).cloned());
            let at = at.unwrap_or(list.len()).min(list.len());
            list.splice(at..at, new);
        });
    }

    pub fn deactivate(&mut self, ids: &[String]) {
        self.edit(|list| list.retain(|a| !ids.contains(a)));
    }

    /// Moves `ids` (keeping their list order) to just before the item currently at `target` (len = end).
    pub fn move_to(&mut self, ids: &[String], target: usize) {
        self.edit(|list| {
            // Insert before the first row at/after `target` that isn't moving itself.
            let before = list.iter().skip(target).find(|a| !ids.contains(a)).cloned();
            let (moving, mut rest): (Vec<String>, Vec<String>) = list.drain(..).partition(|a| ids.contains(a));
            let at = before.and_then(|b| rest.iter().position(|a| *a == b)).unwrap_or(rest.len());
            rest.splice(at..at, moving);
            *list = rest;
        });
    }

    pub fn sort(&mut self) {
        let sorted = sort::sort(&self.active, |id| self.info(id));
        let changed = sorted != self.active;
        self.edit(|list| *list = sorted);
        self.status = if changed { "Sorted.".into() } else { "Already in order.".into() };
    }

    /// Back to what's saved on disk (undoable).
    pub fn discard(&mut self) {
        let saved = self.saved.clone();
        self.edit(|list| *list = saved);
        self.status = "Discarded unsaved changes.".into();
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.active, prev));
            self.refresh();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.active, next));
            self.refresh();
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut Vec<String>)) {
        let before = self.active.clone();
        f(&mut self.active);
        if before != self.active {
            self.undo.push(before);
            self.redo.clear();
            self.refresh();
        }
    }

    /// Recomputes everything derived from the active list.
    fn refresh(&mut self) {
        self.issues = issues::check(&self.active, |id| self.info(id), &self.game_version);
        self.changes = changes::diff(&self.saved, &self.active);
    }

    // ---- persistence (background) ----

    /// Rescans mods and re-reads ModsConfig.xml in the background. Drops undo history when done.
    pub fn reload(&mut self) {
        let s = self.settings.clone();
        self.scan = Some(Task::spawn("Scanning mods…", &self.waker, move |_| {
            let t = Instant::now();
            let game_version = mod_info::game_version(&s.game_dir);
            let roots = [
                (s.game_dir.join("Data"), Source::Official),
                (s.game_dir.join("Mods"), Source::Local),
                (s.workshop_dir.clone(), Source::Workshop),
            ];
            let mods = mod_info::scan(&roots, &game_version);
            let active = mods_config::read_active(&s.mods_config());
            let backups = backups::list(&store::backups_dir());
            Scan { mods, active, game_version, backups, millis: t.elapsed().as_millis() }
        }));
    }

    fn finish_scan(&mut self, scan: Scan) {
        self.mods = scan.mods;
        self.game_version = scan.game_version;
        self.by_id = self.mods.iter().enumerate().map(|(i, m)| (m.id.clone(), i)).collect();
        self.undo.clear();
        self.redo.clear();
        match scan.active {
            Ok(active) => {
                // The file on disk stays as "saved", so a cleaned list shows as unsaved until written.
                self.saved = active.clone();
                self.active = mods_config::dedupe(active);
                let repeats = self.saved.len() - self.active.len();
                self.status = match repeats {
                    0 => format!("Loaded {} mods in {} ms.", self.mods.len(), scan.millis),
                    n => format!("ModsConfig.xml listed {n} mod(s) twice: repeats removed, Save to write the fix."),
                };
            }
            Err(e) => {
                self.active.clear();
                self.saved.clear();
                self.status = format!("Can't read ModsConfig.xml: {e}");
            }
        }
        self.backups = scan.backups;
        self.refresh();
    }

    /// Writes ModsConfig.xml (and the current modlist) in the background,
    /// after copying the current file into the rotating backups.
    pub fn save(&mut self) {
        if self.save.is_some() {
            return;
        }
        let (snapshot, path, list) = (self.active.clone(), self.settings.mods_config(), self.settings.current_list.clone());
        self.save = Some(Task::spawn("Saving…", &self.waker, move |_| {
            let dir = store::backups_dir();
            let backup = backups::create(&path, &dir, SystemTime::now(), backups::KEEP)?;
            mods_config::write_active(&path, &snapshot)?;
            if let Some(name) = list {
                store::save_list(&name, &snapshot)?;
            }
            Ok(Saved { snapshot, backup, backups: backups::list(&dir) })
        }));
    }

    fn finish_save(&mut self, result: io::Result<Saved>) {
        let play = std::mem::take(&mut self.play_after_save) && result.is_ok();
        self.status = match result {
            Ok(saved) => {
                self.saved = saved.snapshot;
                self.backups = saved.backups;
                self.refresh();
                let what = match &self.settings.current_list {
                    Some(name) => format!("Saved game config and list \"{name}\""),
                    None => "Saved game config".into(),
                };
                match saved.backup {
                    Some(_) => format!("{what} (previous version backed up)."),
                    None => format!("{what}."),
                }
            }
            Err(e) => format!("Save failed, nothing was changed (game not started): {e}"),
        };
        if play {
            self.launch();
        }
    }

    fn play(&mut self, save_first: bool) {
        if save_first && self.is_dirty() {
            self.play_after_save = true;
            self.save();
        } else {
            self.launch();
        }
    }

    fn launch(&mut self) {
        self.status = match game::launch() {
            Ok(()) => "Starting RimWorld through Steam…".into(),
            Err(e) => format!("Can't start RimWorld: {e}"),
        };
    }

    /// Adds a launch to the startup history (kept across sessions to compare launches).
    fn record_launch(&mut self, s: &Startup) {
        if s.started.is_empty() {
            return;
        }
        let record = LaunchRecord { started: s.started.clone(), total: s.total, phases: s.phases.iter().map(|p| (p.name.to_string(), p.secs)).collect() };
        self.launches = store::merge_history(std::mem::take(&mut self.launches), record);
        if let Err(e) = store::save_history(&self.launches) {
            self.status = format!("Can't save startup history: {e}");
        }
    }

    pub fn texture_cache_dir(&self) -> PathBuf {
        self.settings.config_dir.parent().unwrap_or(&self.settings.config_dir).join("TextureCache")
    }

    /// Bytes the engine has cached for this mod's textures, once a texture scan ran.
    pub fn texture_cache_of(&self, m: &ModInfo) -> Option<u64> {
        let folder = m.path.file_name()?.to_string_lossy().to_lowercase();
        self.textures.as_ref()?.cache.get(&folder).copied()
    }

    /// Reads every mod's texture headers on all cores (~0.5 s for 19k textures), plus the engine cache index.
    pub fn scan_textures(&mut self) {
        if self.texture_task.is_some() || self.loading() {
            return;
        }
        let mods: Vec<(String, PathBuf)> = self.mods.iter().map(|m| (m.id.clone(), m.path.clone())).collect();
        let (version, cache_dir) = (self.game_version.clone(), self.texture_cache_dir());
        self.texture_task = Some(Task::spawn("Scanning textures…", &self.waker, move |progress| {
            let t = Instant::now();
            let (done, total) = (AtomicUsize::new(0), mods.len() as u64);
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
            let per_mod: HashMap<String, ModTextures> = std::thread::scope(|s| {
                let handles: Vec<_> = mods
                    .chunks(mods.len().div_ceil(threads).max(1))
                    .map(|chunk| {
                        let (done, version) = (&done, &version);
                        s.spawn(move || {
                            let scan = |(id, path): &(String, PathBuf)| {
                                let found = textures::scan(path, version);
                                progress.set(done.fetch_add(1, Ordering::Relaxed) as u64 + 1, total);
                                (id.clone(), found)
                            };
                            chunk.iter().map(scan).collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            });
            TextureReport { mods: per_mod, cache: textures::cache_index(&cache_dir), millis: t.elapsed().as_millis() }
        }));
    }

    /// The game's own mod-list folder, next to Config.
    pub fn mod_lists_dir(&self) -> PathBuf {
        self.settings.config_dir.parent().unwrap_or(&self.settings.config_dir).join("ModLists")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.settings.config_dir.parent().unwrap_or(&self.settings.config_dir).join("Saves")
    }

    /// Files offered in File > Import: the game's mod lists, then the newest saves.
    pub fn import_sources(&self) -> Vec<PathBuf> {
        let files = |dir: PathBuf, ext: &str| -> Vec<(std::time::SystemTime, PathBuf)> {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
                .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
                .collect()
        };
        let mut lists = files(self.mod_lists_dir(), "rml");
        lists.sort_by(|a, b| a.1.cmp(&b.1));
        let mut saves = files(self.saves_dir(), "rws");
        saves.sort_by(|a, b| b.0.cmp(&a.0));
        lists.into_iter().chain(saves.into_iter().take(8)).map(|(_, p)| p).collect()
    }

    /// Reads a mod list file in the background; the list changes when it's done (undoable).
    pub fn import(&mut self, file: PathBuf) {
        let label = format!("Importing {}…", file.file_name().map_or(String::new(), |n| n.to_string_lossy().into()));
        self.import = Some(Task::spawn(label, &self.waker, move |_| {
            let entries = std::fs::File::open(&file).and_then(|f| {
                let mut head = Vec::new();
                io::Read::read_to_end(&mut io::Read::take(f, IMPORT_READ_LIMIT), &mut head)?;
                Ok(modlist_io::read(String::from_utf8_lossy(&head).trim_start_matches('\u{feff}')))
            });
            Imported { file, entries }
        }));
    }

    fn finish_import(&mut self, result: Imported) {
        let name = result.file.file_name().map_or(String::new(), |n| n.to_string_lossy().into());
        let entries = match result.entries {
            Ok(Some(entries)) => entries,
            Ok(None) => return self.status = format!("{name} has no mod list in it."),
            Err(e) => return self.status = format!("Can't read {name}: {e}"),
        };
        for e in &entries {
            if let Some(steam_id) = e.steam_id {
                self.known_steam_ids.insert(e.id.clone(), steam_id);
            }
        }
        let mut seen = HashSet::new();
        let ids: Vec<String> = entries.into_iter().map(|e| e.id).filter(|id| seen.insert(id.clone())).collect();
        let missing = ids.iter().filter(|id| self.info(id).is_none()).count();
        let count = ids.len();
        self.edit(|list| *list = ids);
        self.status = format!("Imported {count} mods from {name} ({missing} not installed). Save to apply, or Undo.");
    }

    /// Writes the active list as a `.rml` into the game's ModLists folder (shareable, loadable in-game).
    pub fn export_list(&mut self, name: &str) {
        let name = store::clean_name(name);
        if name.is_empty() {
            self.status = "Type a name for the exported list.".into();
            return;
        }
        let entries: Vec<Entry> = self
            .active
            .iter()
            .map(|id| Entry {
                id: id.clone(),
                steam_id: self.info(id).and_then(ModInfo::steam_id).or_else(|| self.known_steam_ids.get(id).copied()),
                name: Some(self.name_of(id).to_string()),
            })
            .collect();
        let path = self.mod_lists_dir().join(format!("{name}.rml"));
        let result = std::fs::create_dir_all(self.mod_lists_dir()).and_then(|_| std::fs::write(&path, modlist_io::write_rml(&entries, &self.game_version)));
        self.status = match result {
            Ok(()) => format!("Exported {} mods to {}", entries.len(), path.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Puts a backup's mods in the active list. Nothing is written until Save.
    pub fn restore_backup(&mut self, path: &std::path::Path) {
        match mods_config::read_active(path) {
            Ok(ids) => {
                self.edit(|list| *list = ids);
                self.status = "Backup loaded into the list. Save to apply it, or Undo.".into();
            }
            Err(e) => self.status = format!("Can't read backup: {e}"),
        }
    }

    pub fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.persist_settings();
        self.reload();
    }

    fn persist_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.status = format!("Can't save settings: {e}");
        }
    }

    pub fn log_path(&self, file: LogFile) -> PathBuf {
        self.settings.config_dir.parent().unwrap_or(&self.settings.config_dir).join("Logs").join(file.file_name())
    }

    /// Parses a game log in the background, blaming entries on the installed mods.
    pub fn load_log(&mut self, file: LogFile) {
        if self.log_task.is_some() {
            return;
        }
        let (path, mods) = (self.log_path(file), self.mods.clone());
        let label = format!("Reading {}…", file.file_name());
        self.log_task = Some(Task::spawn(label, &self.waker, move |progress| {
            let t = Instant::now();
            let index = ModIndex::new(&mods);
            let report = std::fs::File::open(&path).map(|f| {
                let total = f.metadata().map_or(0, |m| m.len());
                let reader = io::BufReader::with_capacity(1 << 20, f);
                logs::parse(reader, &index, |done| progress.set(done, total))
            });
            // Startup only needs the first few thousand lines, so this second pass is ~2 ms.
            let startup = std::fs::File::open(&path).ok().and_then(|f| startup::analyze(io::BufReader::new(f), &index));
            LogResult { file, report, startup, millis: t.elapsed().as_millis() }
        }));
    }

    // ---- modlists ----

    /// Switches to a list: its mods become the active ones (undoable). `None` = no list.
    pub fn select_list(&mut self, name: Option<String>) {
        if let Some(n) = &name {
            match store::load_list(n) {
                Ok(ids) => self.edit(|list| *list = ids),
                Err(e) => {
                    self.status = format!("Can't load list \"{n}\": {e}");
                    return;
                }
            }
        }
        self.status = name.as_ref().map_or("No list selected.".into(), |n| format!("Loaded list \"{n}\"."));
        self.settings.current_list = name;
        self.persist_settings();
    }

    /// Saves the current active mods as a new (or overwritten) list and selects it.
    pub fn save_list_as(&mut self, name: &str) {
        let name = store::clean_name(name);
        if name.is_empty() {
            self.status = "List name is empty.".into();
            return;
        }
        if let Err(e) = store::save_list(&name, &self.active) {
            self.status = format!("Can't save list: {e}");
            return;
        }
        self.lists = store::list_names();
        self.status = format!("Saved list \"{name}\".");
        self.settings.current_list = Some(name);
        self.persist_settings();
    }

    pub fn delete_current_list(&mut self) {
        let Some(name) = self.settings.current_list.take() else { return };
        self.status = match store::delete_list(&name) {
            Ok(()) => format!("Deleted list \"{name}\"."),
            Err(e) => format!("Can't delete list: {e}"),
        };
        self.lists = store::list_names();
        self.persist_settings();
    }
}

/// Polls every 2 s whether RimWorld is running (native snapshot, ~1 ms) and reports each change.
fn watch_game(waker: Waker) -> mpsc::Receiver<bool> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut last = false;
        loop {
            let running = game::is_running(game::EXE);
            if running != last {
                if tx.send(running).is_err() {
                    return;
                }
                waker();
                last = running;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn controller(active: &[&str]) -> Controller {
        let mut c = Controller::empty(Settings::default(), Arc::new(|| {}));
        let m = |id: &str| ModInfo { id: id.into(), name: id.into(), versions: vec!["1.7".into()], ..Default::default() };
        let mut b = m("b");
        b.dependencies = vec!["a".into()];
        b.links = vec![("a".into(), "steam://url/CommunityFilePage/1".into())];
        c.finish_scan(Scan {
            mods: vec![m("a"), b, m("c")],
            active: Ok(active.iter().map(|s| s.to_string()).collect()),
            game_version: "1.7".into(),
            backups: Vec::new(),
            millis: 0,
        });
        c
    }

    #[test]
    fn edits_refresh_changes_and_issues() {
        let mut c = controller(&["a", "b"]);
        assert!(!c.is_dirty() && c.issues.is_empty());
        c.apply(Action::Deactivate(vec!["a".into()]));
        assert_eq!(c.changes.deactivated, ["a"]);
        assert_eq!(c.issues_of("b"), [Issue::MissingDependency("a".into())]);
        assert_eq!(c.severity("b"), Some(Severity::Error));
        c.apply(Action::Undo);
        assert!(!c.is_dirty() && c.issues.is_empty());
        c.apply(Action::Redo);
        assert!(c.is_dirty());
    }

    #[test]
    fn discard_restores_saved_and_is_undoable() {
        let mut c = controller(&["a", "b"]);
        c.apply(Action::Activate(vec!["c".into()], Some(0)));
        c.apply(Action::MoveTo(vec!["b".into()], 0));
        assert_eq!(c.active, ["b", "c", "a"]);
        c.apply(Action::Discard);
        assert_eq!(c.active, ["a", "b"]);
        assert!(!c.is_dirty());
        c.apply(Action::Undo);
        assert_eq!(c.active, ["b", "c", "a"]);
    }

    fn ids(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn batch_edits_keep_order_and_undo_in_one_step() {
        let mut c = controller(&["a"]);
        c.apply(Action::Activate(ids(&["c", "b", "a"]), Some(0)));
        assert_eq!(c.active, ["c", "b", "a"], "already active ones are left where they are");
        c.apply(Action::Deactivate(ids(&["c", "a"])));
        assert_eq!(c.active, ["b"]);
        c.apply(Action::Undo);
        assert_eq!(c.active, ["c", "b", "a"]);
        c.apply(Action::Undo);
        assert_eq!(c.active, ["a"]);
    }

    #[test]
    fn moving_several_keeps_their_relative_order() {
        let moved = |picked: &[&str], target: usize| {
            let mut c = controller(&["a", "b", "c", "d", "e"]);
            c.apply(Action::MoveTo(ids(picked), target));
            c.active
        };
        assert_eq!(moved(&["b", "d"], 0), ["b", "d", "a", "c", "e"]);
        assert_eq!(moved(&["b", "d"], 5), ["a", "c", "e", "b", "d"]);
        assert_eq!(moved(&["b", "d"], 3), ["a", "c", "b", "d", "e"], "dropped on one of the moving rows");
        assert_eq!(moved(&["d", "b"], 0), ["b", "d", "a", "c", "e"], "list order, not pick order");
    }

    #[test]
    fn restoring_a_backup_is_an_undoable_edit() {
        let dir = std::env::temp_dir().join("rimkit_restore_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ModsConfig-20261021-215012.xml");
        std::fs::write(&file, "<ModsConfigData><activeMods><li>c</li><li>a</li></activeMods></ModsConfigData>").unwrap();

        let mut c = controller(&["a", "b"]);
        c.apply(Action::RestoreBackup(file));
        assert_eq!(c.active, ["c", "a"]);
        assert!(c.is_dirty(), "restored, but not saved until the user saves");
        c.apply(Action::Undo);
        assert_eq!(c.active, ["a", "b"]);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs `poll` until background jobs are done.
    fn finish_jobs(c: &mut Controller) {
        let t = Instant::now();
        while !c.jobs().is_empty() {
            c.poll();
            assert!(t.elapsed().as_secs() < 5, "background job never finished");
        }
    }

    #[test]
    fn export_then_import_round_trip() {
        let dir = temp("rimkit_io_test");
        let mut c = controller(&["a", "b"]);
        c.settings.config_dir = dir.join("Config");
        c.apply(Action::ExportList("my list".into()));
        let file = dir.join("ModLists").join("my list.rml");
        assert!(file.exists(), "exported next to the game's own mod lists");

        c.apply(Action::Deactivate(vec!["a".into()]));
        c.apply(Action::Import(file));
        finish_jobs(&mut c);
        assert_eq!(c.active, ["a", "b"]);
        c.apply(Action::Undo);
        assert_eq!(c.active, ["b"]);
    }

    #[test]
    fn imported_steam_ids_link_missing_mods() {
        let dir = temp("rimkit_import_save_test");
        let save = dir.join("colony.rws");
        std::fs::write(&save, "<savegame><meta><modIds><li>a</li><li>ghost</li></modIds><modSteamIds><li>0</li><li>42</li></modSteamIds></meta></savegame>").unwrap();
        let mut c = controller(&["a"]);
        c.apply(Action::Import(save));
        finish_jobs(&mut c);
        assert_eq!(c.active, ["a", "ghost"]);
        let ghost = c.messages("ghost");
        assert_eq!(ghost[0].2.as_deref(), Some("steam://url/CommunityFilePage/42"));
    }

    #[test]
    fn missing_dependency_messages_carry_the_link() {
        let mut c = controller(&["a", "b"]);
        c.apply(Action::Deactivate(vec!["a".into()]));
        let b = c.messages("b");
        assert_eq!(b[0].1, "Needs a");
        assert_eq!(b[0].2.as_deref(), Some("steam://url/CommunityFilePage/1"));
    }

    #[test]
    fn repeated_entries_are_dropped_on_load() {
        let c = controller(&["a", "b", "a"]);
        assert_eq!(c.active, ["a", "b"]);
        assert!(c.is_dirty(), "the cleaned list still has to be saved");
        assert!(c.status.contains("twice"), "{}", c.status);
    }

    #[test]
    fn edits_are_ignored_while_loading() {
        let mut c = controller(&["a"]);
        c.reload();
        c.apply(Action::Activate(vec!["c".into()], None));
        c.apply(Action::Select("c".into()));
        assert_eq!(c.active, ["a"]);
        assert_eq!(c.selected.as_deref(), Some("c"));
    }
}
