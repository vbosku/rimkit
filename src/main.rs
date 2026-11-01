#![windows_subsystem = "windows"]

mod controller;
mod model;
mod task;
mod view;

use eframe::egui;
use model::store;
use std::sync::{Arc, OnceLock};

fn main() -> eframe::Result {
    let started = std::time::Instant::now();
    // Start the mod scan right away so it overlaps window/GPU setup (~150 ms) instead of following it.
    // The UI context doesn't exist yet; background jobs wake it once it does.
    let ui_ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let waker = {
        let ui_ctx = ui_ctx.clone();
        Arc::new(move || {
            if let Some(ctx) = ui_ctx.get() {
                ctx.request_repaint();
            }
        })
    };
    let controller = controller::Controller::new(waker);

    // Only the size is remembered; the window always opens centered (a saved position can be off-screen).
    let window = store::load_window();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RimKit")
        .with_inner_size([window.width, window.height])
        .with_min_inner_size(store::MIN_WINDOW)
        .with_maximized(window.maximized);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, centered: true, persist_window: false, ..Default::default() };
    eframe::run_native(
        "RimKit",
        options,
        Box::new(move |cc| {
            let _ = ui_ctx.set(cc.egui_ctx.clone());
            Ok(Box::new(view::App::new(&cc.egui_ctx, controller, started, window)))
        }),
    )
}

/// Window/taskbar icon (the exe's own icon comes from assets/rimkit.rc).
fn icon() -> Option<egui::IconData> {
    let rgba = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.to_rgba8();
    Some(egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}
