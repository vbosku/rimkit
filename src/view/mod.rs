//! View: draws the state and turns user input into `Action`s.

mod logs_tab;
mod mods_tab;
mod previews;
mod selection;
mod settings;
mod style;
mod textures_tab;

use crate::controller::{Action, Controller};
use crate::model::issues::Severity;
use crate::model::store::{self, WindowState};
use crate::task::{self, Task};
use eframe::egui::{self, Button, Color32, ComboBox, Key, KeyboardShortcut, Modifiers, Panel, TextEdit, Ui};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Mods,
    Logs,
    Textures,
}

pub struct App {
    c: Controller,
    tab: Tab,
    mods: mods_tab::ModsTab,
    logs: logs_tab::LogsTab,
    textures: textures_tab::TexturesTab,
    previews: previews::Previews,
    settings: Option<settings::Draft>,
    new_list: String,
    /// Name typed in the open "Export mod list" dialog.
    export: Option<String>,
    focus_export: bool,
    /// Asking whether to save before starting the game.
    play_prompt: bool,
    /// Size to reopen with; saved by eframe's `save` hook.
    window: WindowState,
    window_checked: bool,
    fonts_checked: bool,
    /// Reads the CJK fallback font file (~20 MB) off the UI thread.
    font_task: Option<Task<Option<Vec<u8>>>>,
    timing: Option<Timing>,
}

/// Opt-in startup timing: with `RIMKIT_TIMING=<file>` set, appends "first_frame_ms mods_loaded_ms" to it.
struct Timing {
    start: std::time::Instant,
    file: std::path::PathBuf,
    first_frame: Option<u128>,
}

impl App {
    pub fn new(ctx: &egui::Context, c: Controller, started: std::time::Instant, window: WindowState) -> Self {
        style::install(ctx);
        Self {
            c,
            tab: Tab::Mods,
            mods: mods_tab::ModsTab::default(),
            logs: logs_tab::LogsTab::default(),
            textures: textures_tab::TexturesTab::default(),
            previews: previews::Previews::new(ctx),
            settings: None,
            new_list: String::new(),
            export: None,
            focus_export: false,
            play_prompt: false,
            window,
            window_checked: false,
            fonts_checked: false,
            font_task: None,
            timing: std::env::var_os("RIMKIT_TIMING").map(|f| Timing { start: started, file: f.into(), first_frame: None }),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let typing = ctx.egui_wants_keyboard_input();
        let pressed = |m, k| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        if pressed(Modifiers::COMMAND, Key::S) {
            actions.push(Action::Save);
        }
        if pressed(Modifiers::COMMAND, Key::F) {
            self.tab = Tab::Mods;
            self.mods.focus_filter = true;
        }
        if pressed(Modifiers::NONE, Key::F5) {
            actions.push(Action::Reload);
        }
        if typing {
            return; // let text fields keep their own undo/redo
        }
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || pressed(Modifiers::COMMAND, Key::Y) {
            actions.push(Action::Redo);
        }
        if pressed(Modifiers::COMMAND, Key::Z) {
            actions.push(Action::Undo);
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Settings…").clicked() {
                    self.settings = Some(settings::Draft::from(&self.c.settings));
                }
                menu_item(ui, "Save", "Ctrl+S", true, Action::Save, actions);
                menu_item(ui, "Reload mods", "F5", !self.c.loading(), Action::Reload, actions);
                ui.menu_button("Backups", |ui| self.backups_menu(ui, actions));
                ui.separator();
                ui.menu_button("Import mod list", |ui| self.import_menu(ui, actions));
                if ui.button("Export mod list…").clicked() {
                    self.export = Some(self.c.settings.current_list.clone().unwrap_or_else(|| "My mods".into()));
                    self.focus_export = true;
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                menu_item(ui, "Undo", "Ctrl+Z", self.c.can_undo(), Action::Undo, actions);
                menu_item(ui, "Redo", "Ctrl+Y", self.c.can_redo(), Action::Redo, actions);
                menu_item(ui, "Sort load order", "", true, Action::Sort, actions);
                menu_item(ui, "Discard changes", "", self.c.is_dirty(), Action::Discard, actions);
            });
        });
    }

    /// A ModsConfig.xml backup is taken before every save; picking one loads it into the list (Save to apply).
    fn backups_menu(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let now = std::time::SystemTime::now();
        if self.c.backups.is_empty() {
            ui.weak("No backups yet: one is taken before every save.");
        }
        for b in &self.c.backups {
            let label = format!("{} · {} mods", crate::model::backups::ago(b.time, now), b.mods);
            if ui.button(label).on_hover_text(format!("Load into the list (Save to apply)\n{}", b.path.display())).clicked() {
                actions.push(Action::RestoreBackup(b.path.clone()));
            }
        }
        ui.separator();
        if ui.button("Open backups folder").clicked() {
            actions.push(Action::OpenBackupsFolder);
        }
    }

    /// The game's mod lists and recent saves; any other file can be dropped onto the window.
    fn import_menu(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let sources = self.c.import_sources();
        if sources.is_empty() {
            ui.weak("No mod lists or saves found.");
        }
        for path in sources {
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into());
            if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                actions.push(Action::Import(path));
            }
        }
        ui.separator();
        ui.weak("Or drop a ModsConfig.xml, .rml or save file onto the window.");
    }

    /// "Export mod list" dialog: name the .rml written to the game's ModLists folder. Enter exports.
    fn export_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(name) = &mut self.export else { return };
        let (mut open, mut done) = (true, false);
        egui::Window::new("Export mod list")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 160.0])
            .show(ctx, |ui| {
                ui.label("Saves the active mods as an .rml file in the game's ModLists folder.");
                ui.weak("The game's own mod manager can load it, and you can share the file.");
                ui.add_space(style::MD);
                let field = ui.add(TextEdit::singleline(name).hint_text("List name").desired_width(420.0));
                if std::mem::take(&mut self.focus_export) {
                    field.request_focus();
                }
                let ready = !name.trim().is_empty();
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(style::MD);
                ui.horizontal(|ui| {
                    let primary = Button::new(egui::RichText::new("Export").strong().color(Color32::WHITE)).fill(ui.visuals().selection.bg_fill);
                    if (ui.add_enabled(ready, primary).clicked() || (enter && ready)) && !done {
                        actions.push(Action::ExportList(name.clone()));
                        done = true;
                    }
                    if ui.button("Open ModLists folder").clicked() {
                        actions.push(Action::OpenModListsFolder);
                    }
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                });
            });
        if done || !open {
            self.export = None;
        }
    }

    /// Accent "Play" button; asks first when there are unsaved changes (the game reads the saved list).
    fn play_button(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let running = self.c.game_running;
        let text = egui::RichText::new(if running { "Running…" } else { "Play" }).strong().color(Color32::WHITE);
        let button = Button::new(text).fill(ui.visuals().selection.bg_fill);
        let hint = if running { "RimWorld is running" } else { "Start RimWorld through Steam" };
        if ui.add_enabled(!running, button).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
            if self.c.is_dirty() {
                self.play_prompt = true;
            } else {
                actions.push(Action::Play { save_first: false });
            }
        }
    }

    fn play_prompt(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.play_prompt {
            return;
        }
        let mut open = true;
        egui::Window::new("Unsaved changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 160.0])
            .show(ctx, |ui| {
                ui.label(format!("{}.", self.c.changes.summary()));
                ui.label("RimWorld loads the saved mod list, not the one shown here.");
                ui.add_space(style::MD);
                ui.horizontal(|ui| {
                    let primary = Button::new(egui::RichText::new("Save and play").strong().color(Color32::WHITE)).fill(ui.visuals().selection.bg_fill);
                    if ui.add(primary).clicked() {
                        actions.push(Action::Play { save_first: true });
                        self.play_prompt = false;
                    }
                    if ui.button("Play without saving").clicked() {
                        actions.push(Action::Play { save_first: false });
                        self.play_prompt = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.play_prompt = false;
                    }
                });
            });
        if !open {
            self.play_prompt = false;
        }
    }

    /// Files dragged onto the window: import the first one; show a hint while hovering.
    fn drop_files(&self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let (hovering, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.first().map(|f| f.path().to_path_buf())));
        if let Some(path) = dropped {
            actions.push(Action::Import(path));
        }
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop_hint")));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(170));
            let font = egui::FontId::proportional(22.0);
            painter.text(screen.center(), egui::Align2::CENTER_CENTER, "Drop to import this mod list", font, Color32::WHITE);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Mods, "Mods");
            ui.selectable_value(&mut self.tab, Tab::Logs, "Logs");
            ui.selectable_value(&mut self.tab, Tab::Textures, "Textures");
            if self.tab != Tab::Mods {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.play_button(ui, actions));
                return;
            }
            ui.separator();

            ui.label("List:");
            let current = self.c.settings.current_list.clone();
            ComboBox::from_id_salt("list")
                .selected_text(current.as_deref().unwrap_or("(none)"))
                .width(180.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "(none)").clicked() {
                        actions.push(Action::SelectList(None));
                    }
                    for name in &self.c.lists {
                        if ui.selectable_label(current.as_ref() == Some(name), name).clicked() {
                            actions.push(Action::SelectList(Some(name.clone())));
                        }
                    }
                });
            if current.is_some() && ui.button("Delete").on_hover_text("Delete this list").clicked() {
                actions.push(Action::DeleteList);
            }
            ui.add(TextEdit::singleline(&mut self.new_list).hint_text("New list name").desired_width(140.0));
            if ui.button("Save as list").clicked() {
                actions.push(Action::SaveListAs(std::mem::take(&mut self.new_list)));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.play_button(ui, actions);
                ui.separator();
                let save = Button::new(if self.c.is_dirty() { "Save*" } else { "Save" });
                if ui.add(save).on_hover_text("Write ModsConfig.xml (+ current list) — Ctrl+S").clicked() {
                    actions.push(Action::Save);
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let jobs = self.c.jobs();
            for (label, progress) in &jobs {
                ui.spinner();
                ui.label(label);
                if let Some(p) = progress {
                    ui.add(egui::ProgressBar::new(*p).desired_width(140.0).show_percentage());
                }
                ui.separator();
            }
            let pending = self.previews.pending();
            if pending > 0 {
                ui.spinner();
                ui.weak(format!("Loading previews ({pending})"));
                ui.separator();
            }
            if jobs.is_empty() {
                ui.label(&self.c.status);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let v = &self.c.game_version;
                ui.weak(format!("RimWorld {v} · {} active · {} installed", self.c.active.len(), self.c.mods.len()));
                if self.c.is_dirty() {
                    ui.colored_label(ui.visuals().warn_fg_color, "unsaved changes");
                }
            });
        });
    }

    /// Remembers the window size (not while minimized or maximized), and fixes a window the OS opened tiny.
    fn track_window(&mut self, ctx: &egui::Context) {
        let vp = ctx.input(|i| i.viewport().clone());
        let Some(inner) = vp.inner_rect else { return };
        let tiny = inner.width() < store::MIN_WINDOW[0] - 1.0 || inner.height() < store::MIN_WINDOW[1] - 1.0;
        if !std::mem::replace(&mut self.window_checked, true) && tiny {
            let size = egui::vec2(self.window.width, self.window.height);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if let Some(monitor) = vp.monitor_size {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(((monitor - size) / 2.0).max(egui::Vec2::ZERO).to_pos2()));
            }
            return;
        }
        if vp.minimized == Some(true) || tiny {
            return;
        }
        self.window.maximized = vp.maximized == Some(true);
        if !self.window.maximized {
            self.window = WindowState { width: inner.width(), height: inner.height(), ..self.window }.sane();
        }
    }

    fn busy(&self) -> bool {
        !self.c.jobs().is_empty() || self.previews.pending() > 0 || self.font_task.is_some()
    }
}

/// "694 MB", "5.0 GB"
pub(crate) fn bytes(n: u64) -> String {
    const MB: u64 = 1 << 20;
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= MB => format!("{} MB", n / MB),
        n if n >= 1 << 10 => format!("{} KB", n >> 10),
        n => format!("{n} B"),
    }
}

/// "41.0 s", "1 min 35 s"
pub(crate) fn duration(secs: f64) -> String {
    if secs < 60.0 {
        return format!("{secs:.1} s");
    }
    let s = secs.round() as u64;
    format!("{} min {:02} s", s / 60, s % 60)
}

/// Centered spinner (with a progress bar when the job reports one) while a tab's data loads.
pub(crate) fn loading_placeholder(ui: &mut Ui, c: &Controller, fallback: &str) {
    egui::CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 2.0 - 30.0);
            match c.jobs().into_iter().find(|(_, p)| p.is_some()) {
                Some((label, Some(p))) => {
                    spinner_line(ui, &label);
                    ui.add(egui::ProgressBar::new(p).desired_width(320.0).show_percentage());
                }
                _ => spinner_line(ui, fallback),
            }
        });
    });
}

/// Spinner + text on one line.
pub(crate) fn spinner_line(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

pub(crate) fn severity_color(ui: &Ui, s: Severity) -> Color32 {
    match s {
        Severity::Error => ui.visuals().error_fg_color,
        Severity::Warning => ui.visuals().warn_fg_color,
        Severity::Info => ui.visuals().weak_text_color(),
    }
}

/// Bytes of the first Windows CJK font found.
fn read_cjk_font() -> Option<Vec<u8>> {
    const CANDIDATES: [&str; 3] = [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\YuGothM.ttc", r"C:\Windows\Fonts\malgun.ttf"];
    CANDIDATES.iter().find_map(|p| std::fs::read(p).ok())
}

/// Adds a CJK font as a last-resort fallback, so Chinese/Japanese/Korean mod names render.
fn add_fallback_font(ctx: &egui::Context, bytes: Vec<u8>) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("cjk".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in fonts.families.values_mut() {
        family.push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

fn menu_item(ui: &mut Ui, text: &str, keys: &str, enabled: bool, action: Action, actions: &mut Vec<Action>) {
    if ui.add_enabled(enabled, Button::new(text).shortcut_text(keys)).clicked() {
        actions.push(action);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(t) = self.timing.as_mut().filter(|t| t.first_frame.is_none()) {
            t.first_frame = Some(t.start.elapsed().as_millis());
        }
        let was_loading = self.c.loading();
        self.c.poll();
        self.previews.poll(&ctx);
        if was_loading && !self.c.loading() {
            if let Some(t) = self.timing.take() {
                let line = format!("{} {}\n", t.first_frame.unwrap_or(0), t.start.elapsed().as_millis());
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(&t.file).and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
            }
            // Fresh scan: previews may have changed, and names may need CJK glyphs.
            self.previews.clear();
            if !self.fonts_checked && self.c.needs_cjk() {
                let repaint = ctx.clone();
                self.font_task = Some(Task::spawn("Loading fonts…", &(std::sync::Arc::new(move || repaint.request_repaint()) as task::Waker), |_| read_cjk_font()));
            }
            self.fonts_checked = true;
        }
        if let Some(bytes) = task::take(&mut self.font_task).flatten() {
            add_fallback_font(&ctx, bytes);
        }

        self.track_window(&ctx);
        let mut actions = Vec::new();
        self.shortcuts(&ctx, &mut actions);

        Panel::top("menu").frame(style::bar(ui)).show(ui, |ui| self.menu_bar(ui, &mut actions));
        Panel::top("toolbar").frame(style::bar(ui)).show(ui, |ui| self.toolbar(ui, &mut actions));
        Panel::bottom("status").frame(style::bar(ui)).show(ui, |ui| self.status_bar(ui));

        match self.tab {
            Tab::Mods => self.mods.show(ui, &self.c, &mut self.previews, &mut actions),
            Tab::Logs => self.logs.show(ui, &self.c, &mut actions),
            Tab::Textures => self.textures.show(ui, &self.c, &mut actions),
        }

        // Cross-tab jumps.
        if let Some(id) = self.mods.open_logs_for.take() {
            self.logs.source = logs_tab::SourceFilter::Mod(id);
            self.tab = Tab::Logs;
        }
        if let Some(id) = self.logs.goto_mod.take().or_else(|| self.textures.goto_mod.take()) {
            actions.push(Action::Select(id));
            self.tab = Tab::Mods;
        }

        if let Some(draft) = &mut self.settings {
            match draft.show(&ctx) {
                settings::Outcome::Open => {}
                settings::Outcome::Cancel => self.settings = None,
                settings::Outcome::Apply(s) => {
                    actions.push(Action::ApplySettings(s));
                    self.settings = None;
                }
            }
        }

        self.play_prompt(&ctx, &mut actions);
        self.export_dialog(&ctx, &mut actions);
        self.drop_files(&ctx, &mut actions);

        for a in actions {
            self.c.apply(a);
        }

        // The pointer "thinks" while anything runs in the background (the UI stays usable).
        if self.busy() {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
        }
    }

    /// Called by eframe on exit and every 30 s.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        let _ = store::save_window(&self.window);
    }
}
