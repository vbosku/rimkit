//! The Textures tab: where VRAM goes, per mod, and what the game already caches.

use super::style::{self, MD, SM};
use super::{bytes, loading_placeholder};
use crate::controller::{Action, Controller, LogFile, TextureReport};
use crate::model::textures::ModTextures;
use crate::model::ModInfo;
use eframe::egui::{self, Align2, CentralPanel, ComboBox, FontId, Panel, Rect, RichText, ScrollArea, Sense, Ui, pos2};

const ROW_H: f32 = 30.0;
/// Column widths, right to left: VRAM value, VRAM bar, cache, DDS, PNG, texture count.
const COLS: [f32; 6] = [84.0, 140.0, 96.0, 56.0, 84.0, 84.0];
const HEADERS: [&str; 6] = ["VRAM", "", "Engine cache", "DDS", "PNG", "Textures"];

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Vram,
    Count,
    Png,
    Name,
}

const SORTS: [(SortBy, &str); 4] = [(SortBy::Vram, "VRAM"), (SortBy::Count, "Texture count"), (SortBy::Png, "PNG size"), (SortBy::Name, "Name")];

pub struct TexturesTab {
    requested: bool,
    active_only: bool,
    sort: SortBy,
    /// Set when the user wants to open a mod in the Mods tab.
    pub goto_mod: Option<String>,
}

impl Default for TexturesTab {
    fn default() -> Self {
        Self { requested: false, active_only: true, sort: SortBy::Vram, goto_mod: None }
    }
}

impl TexturesTab {
    pub fn show(&mut self, ui: &mut Ui, c: &Controller, actions: &mut Vec<Action>) {
        if !self.requested && !c.loading() {
            self.requested = true;
            actions.push(Action::ScanTextures);
            if c.log.is_none() {
                // The log says whether the GPU ran out of VRAM last session.
                actions.push(Action::LoadLog(LogFile::Current));
            }
        }
        Panel::top("tex_bar").frame(style::bar(ui)).show(ui, |ui| self.toolbar(ui, c, actions));
        let Some(report) = &c.textures else {
            loading_placeholder(ui, c, "Waiting for the mod scan…");
            return;
        };
        CentralPanel::default().frame(style::pane(ui)).show(ui, |ui| self.body(ui, c, report));
    }

    fn toolbar(&mut self, ui: &mut Ui, c: &Controller, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            let scanning = c.jobs().iter().any(|(label, _)| label.starts_with("Scanning textures"));
            if ui.add_enabled(!scanning && !c.loading(), egui::Button::new("Rescan")).clicked() {
                actions.push(Action::ScanTextures);
            }
            ui.toggle_value(&mut self.active_only, "Active mods only");
            ui.label("Sort by");
            ComboBox::from_id_salt("tex_sort").selected_text(SORTS.iter().find(|s| s.0 == self.sort).map_or("", |s| s.1)).show_ui(ui, |ui| {
                for (s, label) in SORTS {
                    ui.selectable_value(&mut self.sort, s, label);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open texture cache").on_hover_text(c.texture_cache_dir().display().to_string()).clicked() {
                    actions.push(Action::OpenTextureCache);
                }
            });
        });
    }

    fn body(&mut self, ui: &mut Ui, c: &Controller, report: &TextureReport) {
        let mut rows: Vec<(&ModInfo, &ModTextures)> = c
            .mods
            .iter()
            .filter(|m| !self.active_only || c.is_active(&m.id))
            .filter_map(|m| Some((m, report.mods.get(&m.id)?)))
            .filter(|(_, t)| t.count() > 0)
            .collect();
        match self.sort {
            SortBy::Vram => rows.sort_by(|a, b| b.1.vram_bytes().cmp(&a.1.vram_bytes())),
            SortBy::Count => rows.sort_by(|a, b| b.1.count().cmp(&a.1.count())),
            SortBy::Png => rows.sort_by(|a, b| b.1.png_bytes.cmp(&a.1.png_bytes)),
            SortBy::Name => rows.sort_by_cached_key(|(m, _)| m.name.to_lowercase()),
        }
        let sum = |f: fn(&ModTextures) -> u64| rows.iter().map(|(_, t)| f(t)).sum::<u64>();
        let (count, png, dds, vram) = (sum(|t| t.count() as u64), sum(|t| t.png_bytes), sum(|t| t.dds_bytes), sum(ModTextures::vram_bytes));
        let cached = rows.iter().filter(|(m, _)| c.texture_cache_of(m).is_some()).count();

        style::header(ui, "Textures", Some(format!("{count} textures in {} mods", rows.len())));
        if let Some(at) = c.log.as_ref().and_then(|l| l.report.as_ref().ok()).and_then(|r| r.vram_over_budget.as_deref()) {
            style::note(ui, ui.visuals().warn_fg_color, |ui| {
                ui.label(RichText::new(format!("Your GPU ran out of video memory in the last session ({at}).")).strong());
                ui.label("Textures then spill into system RAM, which stutters. The mods at the top of this list are the ones filling it.");
            });
        }
        ui.add_space(SM);
        ui.horizontal_wrapped(|ui| {
            stat(ui, &bytes(vram), "VRAM if all were loaded");
            stat(ui, &bytes(png), "as PNG on disk");
            stat(ui, &bytes(dds), "as DDS");
            stat(ui, &bytes(report.cache_bytes()), &format!("engine cache, {cached} of {} mods", rows.len()));
        });
        style::separator(ui);

        let biggest = rows.iter().map(|(_, t)| t.vram_bytes()).max().unwrap_or(1).max(1);
        header_row(ui);
        ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for (m, t) in &rows[range] {
                let r = self.row(ui, c, m, t, biggest);
                if r.clicked() {
                    self.goto_mod = Some(m.id.clone());
                }
            }
        });
    }

    fn row(&self, ui: &mut Ui, c: &Controller, m: &ModInfo, t: &ModTextures, biggest: u64) -> egui::Response {
        let (rect, resp) = style::row(ui, ROW_H, c.selected.as_deref() == Some(m.id.as_str()), Sense::click());
        let v = ui.visuals().clone();
        let painter = ui.painter();
        let mid = rect.center().y;
        let x = columns(rect);
        let font = FontId::proportional(13.5);
        let num = |i: usize, text: String, color| {
            painter.text(pos2(x[i], mid), Align2::RIGHT_CENTER, text, font.clone(), color);
        };
        num(0, bytes(t.vram_bytes()), v.strong_text_color());
        let bar = Rect::from_min_max(pos2(x[1] - COLS[1] + SM, mid - 5.0), pos2(x[1], mid + 5.0));
        painter.rect_filled(bar, 3.0, v.faint_bg_color.gamma_multiply(2.5));
        let filled = Rect::from_min_size(bar.min, egui::vec2(bar.width() * t.vram_bytes() as f32 / biggest as f32, bar.height()));
        painter.rect_filled(filled, 3.0, v.selection.bg_fill);
        let cache = match c.texture_cache_of(m) {
            Some(b) => (bytes(b), v.weak_text_color()),
            None => ("not yet".into(), v.warn_fg_color),
        };
        num(2, cache.0, cache.1);
        num(3, if t.dds > 0 { t.dds.to_string() } else { "–".into() }, v.weak_text_color());
        num(4, bytes(t.png_bytes), v.weak_text_color());
        num(5, t.count().to_string(), v.text_color());
        let name_right = x[5] - COLS[5];
        style::text_truncated(painter, pos2(rect.left() + SM, mid), &m.name, FontId::proportional(14.0), v.text_color(), name_right - rect.left() - SM - MD);
        resp.on_hover_text("Click to open in Mods")
    }
}

/// Right edge of each column, right to left, inside a full-width row.
fn columns(rect: Rect) -> [f32; 6] {
    let mut x = [0.0; 6];
    let mut right = rect.right() - style::SCROLL_GUTTER;
    for (i, w) in COLS.iter().enumerate() {
        x[i] = right;
        right -= w;
    }
    x
}

fn header_row(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::hover());
    let x = columns(rect);
    let color = ui.visuals().weak_text_color();
    let painter = ui.painter();
    for (i, h) in HEADERS.iter().enumerate() {
        painter.text(pos2(x[i], rect.center().y), Align2::RIGHT_CENTER, *h, FontId::proportional(12.5), color);
    }
    painter.text(pos2(rect.left() + SM, rect.center().y), Align2::LEFT_CENTER, "Mod", FontId::proportional(12.5), color);
}

/// Big number with a caption under it.
fn stat(ui: &mut Ui, value: &str, caption: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(value).size(20.0).strong());
        ui.weak(caption);
    });
    ui.add_space(style::LG);
}
