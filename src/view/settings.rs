use super::style::{MD, SM};
use crate::model::Settings;
use eframe::egui::{self, TextEdit};
use std::path::PathBuf;

/// Settings being edited in the window; only applied on "Save & reload".
pub struct Draft {
    base: Settings,
    fields: [String; 3],
}

pub enum Outcome {
    Open,
    Cancel,
    Apply(Settings),
}

const LABELS: [&str; 3] = ["RimWorld folder", "Workshop folder (294100)", "Config folder (ModsConfig.xml)"];

impl Draft {
    pub fn from(s: &Settings) -> Self {
        let p = |p: &PathBuf| p.display().to_string();
        Self { base: s.clone(), fields: [p(&s.game_dir), p(&s.workshop_dir), p(&s.config_dir)] }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 120.0])
            .show(ctx, |ui| {
            egui::Grid::new("settings").num_columns(2).spacing([MD, SM]).show(ui, |ui| {
                for (label, field) in LABELS.iter().zip(&mut self.fields) {
                    ui.label(*label);
                    ui.add(TextEdit::singleline(field).desired_width(480.0).min_size([480.0, 0.0].into()));
                    ui.end_row();
                }
            });
            ui.add_space(MD);
            ui.horizontal(|ui| {
                if ui.button("Save & reload").clicked() {
                    let [game, workshop, config] = self.fields.clone().map(PathBuf::from);
                    outcome = Outcome::Apply(Settings { game_dir: game, workshop_dir: workshop, config_dir: config, ..self.base.clone() });
                }
                if ui.button("Defaults").clicked() {
                    *self = Draft::from(&Settings { current_list: self.base.current_list.clone(), ..Settings::default() });
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        if open { outcome } else { Outcome::Cancel }
    }
}
