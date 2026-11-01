//! The Mods tab: Inactive | Active | Details, with a shared filter, problem banners and drag & drop.

use super::previews::Previews;
use super::selection::Selection;
use super::style::{self, MD, SM, XS};
use super::{severity_color, spinner_line};
use crate::controller::{Action, Controller};
use crate::model::issues::Severity;
use crate::model::ModInfo;
use eframe::egui::{
    self, Align2, CentralPanel, Color32, ComboBox, FontId, Frame, LayerId, Margin, Order, Panel, Rect, Response, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, pos2, vec2,
};

const ROW_H: f32 = 44.0;
const THUMB: egui::Vec2 = vec2(64.0, 36.0);
/// Row columns: load-order number and version text widths.
const NUM_W: f32 = 28.0;
const VERSION_W: f32 = 28.0;
/// Decode size (px) for row thumbnails and the details preview.
const THUMB_PX: u32 = 128;
const PREVIEW_PX: u32 = 640;

#[derive(PartialEq, Clone, Copy)]
enum Field {
    All,
    Name,
    Author,
    PackageId,
    Version,
}

const FIELDS: [(Field, &str); 5] = [
    (Field::All, "Any field"),
    (Field::Name, "Name"),
    (Field::Author, "Author"),
    (Field::PackageId, "Package id"),
    (Field::Version, "Version"),
];

/// Extra filter set from the banners: only changed mods, or only mods with a given problem level.
#[derive(PartialEq, Clone, Copy)]
enum Only {
    Everything,
    Changed,
    Problems(Severity),
}

/// Where a dragged mod would land if released now.
#[derive(Clone, Copy, PartialEq)]
enum Drop {
    Inactive,
    Active(usize),
}

pub struct ModsTab {
    filter: String,
    field: Field,
    only: Only,
    pub focus_filter: bool,
    /// Set when the user asks to see this mod's log entries.
    pub open_logs_for: Option<String>,
    /// Picked rows (both lists); Shift/Ctrl-click like a file explorer.
    sel: Selection,
    /// Mods being dragged, in list order.
    dragging: Option<Vec<String>>,
    drop: Option<Drop>,
}

impl Default for ModsTab {
    fn default() -> Self {
        Self {
            filter: String::new(),
            field: Field::All,
            only: Only::Everything,
            focus_filter: false,
            open_logs_for: None,
            sel: Selection::default(),
            dragging: None,
            drop: None,
        }
    }
}

impl ModsTab {
    pub fn show(&mut self, ui: &mut Ui, c: &Controller, previews: &mut Previews, actions: &mut Vec<Action>) {
        self.drop = None;
        if c.loading() && c.mods.is_empty() {
            CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| {
                ui.centered_and_justified(|ui| spinner_line(ui, "Scanning mods…"));
            });
            return;
        }
        // A banner filter that no longer matches anything would hide everything; drop it.
        if (self.only == Only::Changed && !c.is_dirty()) || matches!(self.only, Only::Problems(s) if c.issue_count(s) == 0) {
            self.only = Only::Everything;
        }
        let needle = self.filter.trim().to_lowercase();

        Panel::top("filter").frame(style::bar(ui)).show(ui, |ui| self.filter_bar(ui));

        let inactive: Vec<&str> = c.inactive().into_iter().map(|m| m.id.as_str()).filter(|id| self.matches(c, id, &needle)).collect();
        let active: Vec<(usize, &str)> =
            c.active.iter().enumerate().map(|(i, id)| (i, id.as_str())).filter(|(_, id)| self.matches(c, id, &needle)).collect();

        let active_ids: Vec<&str> = active.iter().map(|(_, id)| *id).collect();
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sel.clear();
        }

        Panel::left("inactive").frame(style::pane(ui)).resizable(true).default_size(360.0).min_size(220.0).show(ui, |ui| {
            let total = count(inactive.len(), c.inactive().len());
            style::header_with(ui, "Inactive", Some(total), |ui| self.list_buttons(ui, &inactive, false, actions));
            let rows: Vec<(Option<usize>, &str)> = inactive.iter().map(|id| (None, *id)).collect();
            self.list(ui, c, previews, "inactive_list", &rows, actions);
        });

        Panel::right("details").frame(style::pane(ui)).resizable(true).default_size(420.0).min_size(260.0).show(ui, |ui| {
            match &c.selected {
                Some(id) => self.details(ui, c, previews, id, actions),
                None => {
                    style::header(ui, "Details", None);
                    ui.weak("Select a mod to see its details.");
                }
            }
        });

        CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| {
            let total = count(active.len(), c.active.len());
            style::header_with(ui, "Active", Some(total), |ui| self.list_buttons(ui, &active_ids, true, actions));
            self.banners(ui, c, actions);
            let rows: Vec<(Option<usize>, &str)> = active.iter().map(|(i, id)| (Some(*i), *id)).collect();
            self.list(ui, c, previews, "active_list", &rows, actions);
        });

        self.finish_drag(ui, c, actions);
    }

    fn filter_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter");
            let r = ui.add(TextEdit::singleline(&mut self.filter).hint_text("Filter active and inactive mods (Ctrl+F)").desired_width(360.0));
            if std::mem::take(&mut self.focus_filter) {
                r.request_focus();
            }
            ComboBox::from_id_salt("field").selected_text(FIELDS.iter().find(|f| f.0 == self.field).map_or("", |f| f.1)).show_ui(ui, |ui| {
                for (f, label) in FIELDS {
                    ui.selectable_value(&mut self.field, f, label);
                }
            });
            if !self.filter.is_empty() && ui.button("Clear").clicked() {
                self.filter.clear();
            }
        });
    }

    /// "Unsaved: …  [Show] [Discard] [Save]" and "● 2 errors ● 5 warnings ● 60 not for 1.7" chips.
    fn banners(&mut self, ui: &mut Ui, c: &Controller, actions: &mut Vec<Action>) {
        let v = ui.visuals().clone();
        if c.is_dirty() {
            banner(ui, v.warn_fg_color, |ui| {
                dot(ui, v.warn_fg_color);
                ui.label(format!("Unsaved: {}", c.changes.summary()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save").clicked() {
                        actions.push(Action::Save);
                    }
                    if ui.button("Discard").on_hover_text("Go back to the saved list (undoable)").clicked() {
                        actions.push(Action::Discard);
                    }
                    let showing = self.only == Only::Changed;
                    if ui.selectable_label(showing, "Show").on_hover_text("Only show changed mods").clicked() {
                        self.only = if showing { Only::Everything } else { Only::Changed };
                    }
                });
            });
        }
        let counts = [Severity::Error, Severity::Warning, Severity::Info].map(|s| (s, c.issue_count(s)));
        if let Some(worst) = counts.iter().find(|(_, n)| *n > 0).map(|(s, _)| *s) {
            banner(ui, severity_color(ui, worst), |ui| {
                for (s, n) in counts.into_iter().filter(|(_, n)| *n > 0) {
                    let label = match s {
                        Severity::Error => format!("{n} with errors"),
                        Severity::Warning => format!("{n} load order"),
                        Severity::Info => format!("{n} not marked for {}", c.game_version),
                    };
                    dot(ui, severity_color(ui, s));
                    let on = self.only == Only::Problems(s);
                    if ui.selectable_label(on, label).on_hover_text("Click to show only these").clicked() {
                        self.only = if on { Only::Everything } else { Only::Problems(s) };
                    }
                }
                if c.issue_count(Severity::Warning) > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Sort").on_hover_text("Fix load order").clicked() {
                            actions.push(Action::Sort);
                        }
                    });
                }
            });
        }
    }

    fn matches(&self, c: &Controller, id: &str, needle: &str) -> bool {
        let shown = match self.only {
            Only::Everything => true,
            Only::Changed => c.changes.contains(id),
            Only::Problems(s) => c.issues_of(id).iter().any(|i| i.severity() == s),
        };
        if !shown {
            return false;
        }
        if needle.is_empty() {
            return true;
        }
        let Some(m) = c.info(id) else { return id.contains(needle) };
        let has = |s: &str| s.to_lowercase().contains(needle);
        let version = || m.versions.iter().any(|v| v.contains(needle));
        match self.field {
            Field::All => has(&m.name) || has(&m.author) || m.id.contains(needle) || version(),
            Field::Name => has(&m.name),
            Field::Author => has(&m.author),
            Field::PackageId => m.id.contains(needle),
            Field::Version => version(),
        }
    }

    /// One virtualized list. `rows` = (position in the active list, id); position is None for inactive mods.
    fn list(
        &mut self,
        ui: &mut Ui,
        c: &Controller,
        previews: &mut Previews,
        salt: &str,
        rows: &[(Option<usize>, &str)],
        actions: &mut Vec<Action>,
    ) {
        let is_active_list = salt == "active_list";
        let panel = ui.available_rect_before_wrap();
        let pointer = ui.ctx().pointer_hover_pos().filter(|p| panel.contains(*p));
        let visible: Vec<&str> = rows.iter().map(|(_, id)| *id).collect();

        ScrollArea::vertical().id_salt(salt).auto_shrink(false).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for &(pos, id) in &rows[range] {
                let r = row(ui, c, previews, id, pos, self.sel.contains(id));
                if r.clicked() {
                    let m = ui.input(|i| i.modifiers);
                    self.sel.click(id, &visible, m.shift, m.command);
                    actions.push(Action::Select(id.into()));
                }
                if r.double_clicked() {
                    // Double-click moves the row to the other list, with the rest of the selection.
                    let ids = self.targets(id, &visible);
                    actions.push(if is_active_list { Action::Deactivate(ids) } else { Action::Activate(ids, None) });
                }
                if r.drag_started() {
                    if !self.sel.contains(id) {
                        self.sel.click(id, &visible, false, false);
                    }
                    self.dragging = Some(self.sel.in_list(&visible));
                    actions.push(Action::Select(id.into()));
                }
                let targets = self.targets(id, &visible);
                r.context_menu(|ui| context_menu(ui, c, &targets, pos.is_some(), actions));

                // Insertion point while dragging over the active list.
                if let (Some(p), Some(pos), true) = (pointer, pos, self.dragging.is_some()) {
                    if r.rect.y_range().contains(p.y) {
                        let after = p.y > r.rect.center().y;
                        self.drop = Some(Drop::Active(pos + after as usize));
                        let y = if after { r.rect.bottom() } else { r.rect.top() };
                        ui.painter().hline(r.rect.x_range(), y, Stroke::new(2.0, ui.visuals().selection.stroke.color));
                    }
                }
            }
        });

        // Anywhere on the inactive panel, or below the last active row.
        if self.dragging.is_some() && pointer.is_some() {
            if !is_active_list {
                self.drop = Some(Drop::Inactive);
            } else if self.drop.is_none() {
                self.drop = Some(Drop::Active(c.active.len()));
            }
        }
    }

    /// "Activate N / Clear / Select all" (+ "Sort" on the active list) on a list's header.
    /// Added right to left, so "Sort" and "Select all" never move.
    fn list_buttons(&mut self, ui: &mut Ui, visible: &[&str], is_active_list: bool, actions: &mut Vec<Action>) {
        if is_active_list {
            if ui.button("Sort").on_hover_text("Sort the load order by loadAfter / loadBefore / dependencies").clicked() {
                actions.push(Action::Sort);
            }
            ui.separator();
        }
        let hint = "Select every row shown. Click, Shift+click and Ctrl+click also select.";
        if ui.add_enabled(!visible.is_empty(), egui::Button::new("Select all")).on_hover_text(hint).clicked() {
            self.sel.select_all(visible);
        }
        let picked = self.sel.in_list(visible);
        if !picked.is_empty() {
            if ui.button("Clear").on_hover_text("Unselect (Esc)").clicked() {
                self.sel.clear();
            }
            let n = picked.len();
            let (label, action) = if is_active_list {
                (format!("Deactivate {n}"), Action::Deactivate(picked))
            } else {
                (format!("Activate {n}"), Action::Activate(picked, None))
            };
            if ui.button(label).clicked() {
                actions.push(action);
            }
        }
    }

    /// The rows an action on `id` applies to: the selection if `id` is part of it, else just `id`.
    fn targets(&self, id: &str, visible: &[&str]) -> Vec<String> {
        if self.sel.contains(id) { self.sel.in_list(visible) } else { vec![id.to_string()] }
    }

    fn finish_drag(&mut self, ui: &Ui, c: &Controller, actions: &mut Vec<Action>) {
        let Some(ids) = self.dragging.clone() else { return };
        let ctx = ui.ctx();
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(p) = ctx.pointer_hover_pos() {
            let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, ui.scope_id().with("drag")));
            let label = match ids.as_slice() {
                [one] => c.name_of(one).to_string(),
                many => format!("{} mods", many.len()),
            };
            let galley = painter.layout_no_wrap(label, FontId::proportional(14.0), ui.visuals().strong_text_color());
            let rect = Rect::from_min_size(p + vec2(14.0, 4.0), galley.size() + vec2(12.0, 6.0));
            painter.rect_filled(rect, 4.0, ui.visuals().window_fill);
            painter.galley(rect.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        }
        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        self.dragging = None;
        let from_active = ids.first().is_some_and(|id| c.is_active(id));
        match (self.drop, from_active) {
            (Some(Drop::Active(at)), true) => actions.push(Action::MoveTo(ids, at)),
            (Some(Drop::Active(at)), false) => actions.push(Action::Activate(ids, Some(at))),
            (Some(Drop::Inactive), true) => actions.push(Action::Deactivate(ids)),
            _ => {}
        }
    }

    fn details(&mut self, ui: &mut Ui, c: &Controller, previews: &mut Previews, id: &str, actions: &mut Vec<Action>) {
        let Some(m) = c.info(id) else {
            ui.heading(id);
            ui.colored_label(ui.visuals().error_fg_color, "Not installed.");
            return;
        };
        ScrollArea::vertical().id_salt("details").auto_shrink(false).show(ui, |ui| {
            Frame::NONE.inner_margin(Margin { right: MD as i8, ..Default::default() }).show(ui, |ui| self.details_body(ui, c, previews, m, actions));
        });
    }

    fn details_body(&mut self, ui: &mut Ui, c: &Controller, previews: &mut Previews, m: &ModInfo, actions: &mut Vec<Action>) {
        let id = m.id.as_str();
        big_preview(ui, previews, m);
        ui.add_space(MD - style::GAP_Y);
        ui.heading(&m.name);
        ui.weak(&m.author);
        ui.add_space(XS);
        ui.horizontal(|ui| {
            let (label, action) =
                if c.is_active(id) { ("Deactivate", Action::Deactivate(vec![id.into()])) } else { ("Activate", Action::Activate(vec![id.into()], None)) };
            if ui.button(label).clicked() {
                actions.push(action);
            }
            if ui.button("Open folder").clicked() {
                actions.push(Action::OpenFolder(id.into()));
            }
            if let Some(url) = c.steam_link(id) {
                if ui.button("Workshop page").clicked() {
                    actions.push(Action::OpenUrl(url));
                }
            }
        });

        let messages = c.messages(id);
        let log = c.log_counts(id);
        if !messages.is_empty() || log.is_some() {
            ui.add_space(XS);
        }
        for (s, msg, url) in messages {
            ui.horizontal(|ui| {
                dot(ui, severity_color(ui, s));
                ui.colored_label(severity_color(ui, s), msg);
                if let Some(url) = url {
                    if ui.small_button("Get it").on_hover_text(&url).clicked() {
                        actions.push(Action::OpenUrl(url));
                    }
                }
            });
        }
        if let Some((errors, warnings)) = log {
            ui.horizontal(|ui| {
                dot(ui, if errors > 0 { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color });
                ui.label(format!("Game log: {errors} errors, {warnings} warnings"));
                if ui.small_button("View").clicked() {
                    self.open_logs_for = Some(id.into());
                }
            });
        }
        style::separator(ui);

        egui::Grid::new("facts").num_columns(2).spacing([MD, XS]).min_row_height(ui.text_style_height(&egui::TextStyle::Body)).show(ui, |ui| {
            let mut fact = |k: &str, v: String| {
                ui.weak(k);
                ui.add(egui::Label::new(v).wrap());
                ui.end_row();
            };
            fact("Package id", m.id.clone());
            fact("Source", m.source.label().into());
            fact("Versions", m.versions.join(", "));
            fact("Folder", m.path.display().to_string());
        });
        if [&m.dependencies, &m.load_after, &m.load_before, &m.incompatible].iter().any(|l| !l.is_empty()) {
            ui.add_space(SM - style::GAP_Y);
            id_list(ui, c, "Requires", &m.dependencies);
            id_list(ui, c, "Loads after", &m.load_after);
            id_list(ui, c, "Loads before", &m.load_before);
            id_list(ui, c, "Incompatible with", &m.incompatible);
        }

        style::separator(ui);
        ui.label(RichText::new(&m.description).size(13.5));
    }
}

fn count(shown: usize, total: usize) -> String {
    if shown == total { total.to_string() } else { format!("{shown} / {total}") }
}

/// Full-width tinted strip; every banner has the same height whether or not it holds buttons.
fn banner(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui)) {
    style::boxed(color.gamma_multiply(0.5), color.gamma_multiply(0.12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(style::CONTROL_H);
            add(ui);
        });
    });
}

/// A small filled circle, sized like a glyph.
fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Preview thumbnail, or a letter tile while loading / when the mod has none.
fn thumb(ui: &Ui, rect: Rect, previews: &mut Previews, m: Option<&ModInfo>, name: &str) {
    let painter = ui.painter();
    if let Some(tex) = m.and_then(|m| previews.get(&m.path, THUMB_PX)) {
        let fitted = fit(tex.size_vec2(), rect);
        painter.rect_filled(rect, 3.0, Color32::from_black_alpha(60));
        painter.image(tex.id(), fitted, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        painter.rect_filled(rect, 3.0, ui.visuals().faint_bg_color.gamma_multiply(2.0));
        let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
        painter.text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(16.0), ui.visuals().weak_text_color());
    }
}

/// `size` scaled to fit inside `rect`, centered.
fn fit(size: egui::Vec2, rect: Rect) -> Rect {
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

fn big_preview(ui: &mut Ui, previews: &mut Previews, m: &ModInfo) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, (w * 9.0 / 16.0).min(260.0)), Sense::hover());
    thumb(ui, rect, previews, Some(m), &m.name);
    if let Some(tex) = previews.get(&m.path, PREVIEW_PX) {
        // Full-size version replaces the thumbnail once decoded.
        ui.painter().image(tex.id(), fit(tex.size_vec2(), rect), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// A single clickable/draggable mod row, painted by hand so it stays cheap.
fn row(ui: &mut Ui, c: &Controller, previews: &mut Previews, id: &str, pos: Option<usize>, selected: bool) -> Response {
    let (rect, resp) = style::row(ui, ROW_H, selected, Sense::click_and_drag());
    let v = ui.visuals().clone();

    let info = c.info(id);
    let severity = pos.and(c.severity(id));
    // | SM | number NUM_W | SM | thumb | MD | name … | SM | dot | SM | version VERSION_W | scrollbar gutter |
    let mid = rect.center().y;
    let mut x = rect.left() + SM;
    if let Some(p) = pos {
        ui.painter().text(pos2(x + NUM_W, mid), Align2::RIGHT_CENTER, (p + 1).to_string(), FontId::monospace(12.0), v.weak_text_color());
        x += NUM_W + SM;
    }
    thumb(ui, Rect::from_min_size(pos2(x, mid - THUMB.y / 2.0), THUMB), previews, info, c.name_of(id));
    x += THUMB.x + MD;
    let version_right = rect.right() - style::SCROLL_GUTTER;
    let dot_x = version_right - VERSION_W - SM - XS;

    let painter = ui.painter();
    let version = info.and_then(|m| m.versions.last()).map_or("", String::as_str);
    let name_rect = Rect::from_min_max(pos2(x, rect.top()), pos2(dot_x - XS - SM, rect.bottom()));
    let color = match (info, severity) {
        (None, _) | (_, Some(Severity::Error)) => v.error_fg_color,
        (_, Some(Severity::Warning)) => v.warn_fg_color,
        _ => v.text_color(),
    };
    style::text_truncated(painter, pos2(x, mid), c.name_of(id), FontId::proportional(14.5), color, name_rect.width());
    let vcol = if info.is_some_and(|m| m.supports(&c.game_version)) { v.weak_text_color() } else { v.warn_fg_color };
    painter.text(pos2(version_right, mid), Align2::RIGHT_CENTER, version, FontId::proportional(12.0), vcol);
    if let Some(s) = severity.filter(|s| *s > Severity::Info) {
        painter.circle_filled(pos2(dot_x, mid), XS, severity_color(ui, s));
    }

    let messages = if pos.is_some() { c.messages(id) } else { Vec::new() };
    if messages.is_empty() {
        resp
    } else {
        resp.on_hover_text(messages.into_iter().map(|(_, m, _)| m).collect::<Vec<_>>().join("\n"))
    }
}

/// Right-click menu for `ids` (the clicked row, or the whole selection it belongs to).
fn context_menu(ui: &mut Ui, c: &Controller, ids: &[String], is_active: bool, actions: &mut Vec<Action>) {
    let mut item = |ui: &mut Ui, text: &str, action: Action| {
        if ui.button(text).clicked() {
            actions.push(action);
            ui.close();
        }
    };
    let n = if ids.len() > 1 { format!(" {} mods", ids.len()) } else { String::new() };
    if is_active {
        item(ui, &format!("Deactivate{n}"), Action::Deactivate(ids.to_vec()));
        item(ui, "Move to top", Action::MoveTo(ids.to_vec(), 0));
        item(ui, "Move to bottom", Action::MoveTo(ids.to_vec(), c.active.len()));
    } else {
        item(ui, &format!("Activate{n}"), Action::Activate(ids.to_vec(), None));
    }
    if let [one] = ids {
        if c.info(one).is_some() {
            item(ui, "Open folder", Action::OpenFolder(one.clone()));
        }
    }
}

/// Related mods; active ones are bright, missing/inactive ones dimmed.
fn id_list(ui: &mut Ui, c: &Controller, title: &str, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    ui.collapsing(format!("{title} ({})", ids.len()), |ui| {
        for dep in ids {
            let text = RichText::new(c.name_of(dep));
            ui.label(if c.is_active(dep) { text } else { text.weak() }).on_hover_text(dep);
        }
    });
}
