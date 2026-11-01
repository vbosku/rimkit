//! The Logs tab: game log warnings/errors, grouped, blamed on mods, filterable.

use super::style::{self, SM};
use super::{duration, loading_placeholder, severity_color};
use crate::controller::{Action, Controller, LogFile};
use crate::model::issues::Severity;
use crate::model::logs::{Group, Level, Report};
use crate::model::startup::{Startup, XmlCache};
use eframe::egui::{
    self, CentralPanel, Color32, ComboBox, CornerRadius, FontId, Frame, Margin, Panel, RichText, ScrollArea, Sense, Separator, TextEdit, Ui, pos2,
};

use std::collections::{HashMap, HashSet};

const SOURCE_ROW_H: f32 = 30.0;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Entries,
    Startup,
}

#[derive(Clone, PartialEq)]
pub enum SourceFilter {
    All,
    Game,
    Mod(String),
}

pub struct LogsTab {
    view: View,
    file: LogFile,
    errors: bool,
    warnings: bool,
    search: String,
    pub source: SourceFilter,
    expanded: HashSet<usize>,
    requested: bool,
    /// Set when the user wants to jump to a mod in the Mods tab.
    pub goto_mod: Option<String>,
}

impl Default for LogsTab {
    fn default() -> Self {
        Self {
            view: View::Entries,
            file: LogFile::Current,
            errors: true,
            warnings: true,
            search: String::new(),
            source: SourceFilter::All,
            expanded: HashSet::new(),
            requested: false,
            goto_mod: None,
        }
    }
}

impl LogsTab {
    pub fn show(&mut self, ui: &mut Ui, c: &Controller, actions: &mut Vec<Action>) {
        // First visit (and once mods are known, so entries can be blamed): read the log.
        if !self.requested && !c.loading() {
            self.requested = true;
            actions.push(Action::LoadLog(self.file));
        }
        Panel::top("log_bar").frame(style::bar(ui)).show(ui, |ui| self.toolbar(ui, c, actions));

        let log = c.log.as_ref().filter(|l| l.file == self.file);
        let Some(report) = log.map(|l| &l.report) else {
            loading_placeholder(ui, c, "Waiting for the mod scan…");
            return;
        };
        let report = match report {
            Ok(r) => r,
            Err(e) => {
                CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| {
                        ui.colored_label(ui.visuals().error_fg_color, format!("Can't read {}: {e}", c.log_path(self.file).display()));
                });
                return;
            }
        };
        if !report.recognized {
            CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| {
                ui.label(format!("{} lines, but none look like a RimWorld 1.7 log entry.", report.lines));
            });
            return;
        }

        if self.view == View::Startup {
            let startup = log.and_then(|l| l.startup.as_ref());
            CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| match startup {
                Some(s) => self.startup_view(ui, c, s),
                None => {
                    ui.weak("No startup found in this log.");
                }
            });
            return;
        }
        Panel::left("log_sources").frame(style::pane(ui)).resizable(true).default_size(300.0).min_size(200.0).show(ui, |ui| self.sources(ui, c, report));
        CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| self.groups(ui, c, report));
    }

    fn toolbar(&mut self, ui: &mut Ui, c: &Controller, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            let before = self.file;
            ComboBox::from_id_salt("log_file").selected_text(self.file.file_name()).show_ui(ui, |ui| {
                for f in [LogFile::Current, LogFile::Previous] {
                    ui.selectable_value(&mut self.file, f, f.file_name());
                }
            });
            if self.file != before {
                self.expanded.clear();
                actions.push(Action::LoadLog(self.file));
            }
            if ui.add_enabled(!c.log_loading(), egui::Button::new("Reload")).on_hover_text("Read the log again").clicked() {
                self.expanded.clear();
                actions.push(Action::LoadLog(self.file));
            }
            ui.separator();
            ui.selectable_value(&mut self.view, View::Entries, "Entries");
            ui.selectable_value(&mut self.view, View::Startup, "Startup");
            if self.view == View::Entries {
                ui.separator();
                ui.toggle_value(&mut self.errors, "Errors");
                ui.toggle_value(&mut self.warnings, "Warnings");
                ui.add(TextEdit::singleline(&mut self.search).hint_text("Search messages").desired_width(260.0));
            }
            if let Some(Ok(r)) = c.log.as_ref().filter(|l| l.file == self.file).map(|l| &l.report) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let blamed = r.groups.iter().filter(|g| g.mod_id.is_some()).count();
                    ui.weak(format!("{} lines · {} entries · {} groups · {blamed} blamed on mods", r.lines, r.entries, r.groups.len()));
                });
            }
        });
    }

    /// Where the loading time of this launch went, plus earlier launches to compare.
    fn startup_view(&mut self, ui: &mut Ui, c: &Controller, s: &Startup) {
        let accent = ui.visuals().selection.bg_fill;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            style::header(ui, "Startup", Some(format!("{} · launched {}", duration(s.total), s.started)));
            match &s.xml_cache {
                Some(XmlCache::Rebuilt(reason)) => style::note(ui, ui.visuals().warn_fg_color, |ui| {
                    ui.label(RichText::new("Gagarin's XML cache was rebuilt this launch, so XML loading was slower than usual.").strong());
                    ui.weak(reason);
                }),
                Some(XmlCache::Used) => style::note(ui, ui.visuals().weak_text_color(), |ui| {
                    ui.label("Gagarin's XML cache was used.");
                }),
                None => {}
            }

            ui.add_space(SM);
            ui.strong("Phases");
            for p in &s.phases {
                style::bar_row(ui, p.name, (p.secs / s.total.max(0.001)) as f32, &duration(p.secs), accent);
            }
            if let Some(at) = s.save_load_at {
                ui.weak(format!("Then the main menu for {}; a save started loading {} after launch.", duration(at - s.total), duration(at)));
            }

            style::separator(ui);
            ui.strong("Longest silences");
            ui.weak("Nothing was logged for this long. The line just before tells who was working.");
            for g in &s.gaps {
                self.gap_row(ui, c, g);
            }

            if !c.launches.is_empty() {
                style::separator(ui);
                ui.strong("Recent launches");
                let longest = c.launches.iter().map(|r| r.total).fold(0.0, f64::max).max(0.001);
                for r in c.launches.iter().take(10) {
                    let color = if r.started == s.started { accent } else { accent.gamma_multiply(0.5) };
                    style::bar_row(ui, &r.started, (r.total / longest) as f32, &duration(r.total), color);
                }
            }
        });
    }

    fn gap_row(&mut self, ui: &mut Ui, c: &Controller, g: &crate::model::startup::Gap) {
        let (rect, resp) = style::row(ui, SOURCE_ROW_H, false, Sense::click());
        let v = ui.visuals().clone();
        let painter = ui.painter();
        let mid = rect.center().y;
        let x = rect.left() + SM;
        painter.text(pos2(x + 56.0, mid), egui::Align2::RIGHT_CENTER, duration(g.secs), FontId::proportional(14.0), v.warn_fg_color);
        painter.text(pos2(x + 120.0, mid), egui::Align2::RIGHT_CENTER, format!("+{}:{:02}", g.at as u64 / 60, g.at as u64 % 60), FontId::proportional(12.5), v.weak_text_color());
        let mut right = rect.right() - style::SCROLL_GUTTER;
        if let Some(id) = &g.mod_id {
            let galley = painter.layout_no_wrap(c.name_of(id).to_string(), FontId::proportional(13.5), v.hyperlink_color);
            right -= galley.size().x;
            painter.galley(pos2(right, mid - galley.size().y / 2.0), galley, v.hyperlink_color);
            right -= style::MD;
        }
        style::text_truncated(painter, pos2(x + 136.0, mid), &g.before, FontId::proportional(13.5), v.text_color(), right - x - 136.0);
        let resp = resp.on_hover_text(&g.before);
        if resp.clicked() {
            if let Some(id) = &g.mod_id {
                self.goto_mod = Some(id.clone());
            }
        }
    }

    fn sources(&mut self, ui: &mut Ui, c: &Controller, report: &Report) {
        style::header(ui, "Sources", None);
        // (errors, warnings) per mod, summing repeated entries.
        let mut per_mod: HashMap<Option<&str>, (usize, usize)> = HashMap::new();
        for g in &report.groups {
            let e = per_mod.entry(g.mod_id.as_deref()).or_default();
            match g.level {
                Level::Error => e.0 += g.count,
                Level::Warning => e.1 += g.count,
            }
        }
        let total = per_mod.values().fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        let game = per_mod.remove(&None).unwrap_or_default();
        let mut mods: Vec<(&str, (usize, usize))> = per_mod.into_iter().filter_map(|(k, v)| Some((k?, v))).collect();
        mods.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| c.name_of(a.0).cmp(c.name_of(b.0))));

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            self.source_row(ui, SourceFilter::All, "All", total);
            self.source_row(ui, SourceFilter::Game, "Game / unknown", game);
            style::separator(ui);
            for (id, counts) in mods {
                self.source_row(ui, SourceFilter::Mod(id.to_string()), c.name_of(id), counts);
            }
        });
    }

    /// Same height and full-width hitbox for every source; counts right-aligned clear of the scrollbar,
    /// the name gets the rest of the width and ends in "…" when it doesn't fit.
    fn source_row(&mut self, ui: &mut Ui, filter: SourceFilter, label: &str, (errors, warnings): (usize, usize)) {
        let selected = self.source == filter;
        let (rect, resp) = style::row(ui, SOURCE_ROW_H, selected, Sense::click());
        let v = ui.visuals().clone();
        let painter = ui.painter();
        let mid = rect.center().y;
        let mut right = rect.right() - style::SCROLL_GUTTER;
        for (n, color) in [(warnings, v.warn_fg_color), (errors, v.error_fg_color)] {
            if n > 0 {
                let galley = painter.layout_no_wrap(n.to_string(), FontId::proportional(13.0), color);
                right -= galley.size().x;
                painter.galley(pos2(right, mid - galley.size().y / 2.0), galley, color);
                right -= style::SM;
            }
        }
        let left = rect.left() + style::SM;
        let color = if selected { v.strong_text_color() } else { v.text_color() };
        style::text_truncated(painter, pos2(left, mid), label, FontId::proportional(14.0), color, right - style::SM - left);
        if resp.on_hover_text(label).clicked() {
            self.source = filter;
        }
    }

    fn groups(&mut self, ui: &mut Ui, c: &Controller, report: &Report) {
        let needle = self.search.trim().to_lowercase();
        let shown: Vec<(usize, &Group)> = report
            .groups
            .iter()
            .enumerate()
            .filter(|(_, g)| match g.level {
                Level::Error => self.errors,
                Level::Warning => self.warnings,
            })
            .filter(|(_, g)| match &self.source {
                SourceFilter::All => true,
                SourceFilter::Game => g.mod_id.is_none(),
                SourceFilter::Mod(id) => g.mod_id.as_deref() == Some(id),
            })
            .filter(|(_, g)| needle.is_empty() || g.text.to_lowercase().contains(&needle))
            .collect();

        if shown.is_empty() {
            ui.weak("Nothing to show with these filters.");
            return;
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (i, g) in shown {
                self.card(ui, c, i, g);
            }
        });
    }

    fn card(&mut self, ui: &mut Ui, c: &Controller, i: usize, g: &Group) {
        let color = severity_color(ui, if g.level == Level::Error { Severity::Error } else { Severity::Warning });
        let open = self.expanded.contains(&i);
        style::boxed(color.gamma_multiply(0.35), ui.visuals().faint_bg_color).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    badge(ui, if g.level == Level::Error { "ERROR" } else { "WARN" }, color);
                    if g.count > 1 {
                        ui.label(RichText::new(format!("×{}", g.count)).strong());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(&g.last[11..]).on_hover_text(format!("First: {}\nLast: {}", g.first, g.last));
                        match &g.mod_id {
                            Some(id) => {
                                let link = ui.link(c.name_of(id)).on_hover_text("Best guess from the message, file paths and stack trace. Click to open in Mods.");
                                if link.clicked() {
                                    self.goto_mod = Some(id.clone());
                                }
                            }
                            None => {
                                ui.weak("Game / unknown");
                            }
                        }
                        let title = egui::Label::new(&g.title).truncate().sense(Sense::click());
                        let r = ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| ui.add(title)).inner;
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            if open { self.expanded.remove(&i) } else { self.expanded.insert(i) };
                        }
                    });
                });
                if open {
                    ui.add(Separator::default().spacing(SM));
                    ui.horizontal(|ui| {
                        ui.weak(format!("[{}]  first {}  ·  last {}", g.category, g.first, g.last));
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(g.text.clone());
                        }
                    });
                    ui.add(egui::Label::new(RichText::new(clip_lines(&g.text)).monospace().size(12.0)).wrap());
                }
            });
    }
}

/// Cuts very long lines (e.g. XML dumps of a save) for display; Copy still gets the full text.
fn clip_lines(text: &str) -> String {
    const MAX: usize = 300;
    let clip = |l: &str| match l.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}…", &l[..i]),
        None => l.to_string(),
    };
    text.lines().map(clip).collect::<Vec<_>>().join("\n")
}

fn badge(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new().fill(color.gamma_multiply(0.2)).corner_radius(CornerRadius::same(3)).inner_margin(Margin::symmetric(5, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).color(color).size(11.0).strong());
    });
}
