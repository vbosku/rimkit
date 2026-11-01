//! Spacing in one place. Every margin, padding and gap in the view comes from these tokens.

use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{Color32, Context, CornerRadius, FontId, Frame, Margin, Painter, Pos2, Rect, Response, Sense, Separator, Stroke, Ui, vec2};

pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 12.0;
pub const LG: f32 = 16.0;

/// Vertical gap between stacked widgets (rows, banners, cards).
pub const GAP_Y: f32 = 6.0;
pub const CONTROL_H: f32 = 26.0;
/// Right space kept free in scrolling lists so the scrollbar never covers content.
pub const SCROLL_GUTTER: f32 = LG;

/// Global egui spacing, applied once at startup.
pub fn install(ctx: &Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(SM, GAP_Y);
        s.spacing.button_padding = vec2(10.0, 4.0);
        s.spacing.interact_size.y = CONTROL_H;
        s.spacing.icon_spacing = 6.0;
        s.spacing.window_margin = Margin::same(MD as i8);
        s.spacing.menu_margin = Margin::same(6);
    });
}

/// Frame for horizontal bars: menu, toolbar, filter bar, status bar.
pub fn bar(ui: &Ui) -> Frame {
    Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(MD as i8, 6))
}

/// Frame for content panes: inactive/active lists, details, log sources/cards.
/// Same margins for side and central panels so their headers line up.
pub fn pane(ui: &Ui) -> Frame {
    Frame::central_panel(ui.style()).inner_margin(Margin::symmetric(MD as i8, 10))
}

/// Inline box (banners, log cards): tinted fill + outline in `color`.
pub fn boxed(color: Color32, fill: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(10, 6))
}

/// Separator with the same breathing room everywhere.
pub fn separator(ui: &mut Ui) {
    ui.add(Separator::default().spacing(MD));
}

/// Pane title with an optional dimmed count, then a separator.
pub fn header(ui: &mut Ui, title: &str, count: Option<String>) {
    header_with(ui, title, count, |_| {});
}

/// Like `header`, with buttons on the right (added right to left). Same height either way, so panes line up.
pub fn header_with(ui: &mut Ui, title: &str, count: Option<String>, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_H);
        ui.heading(title);
        if let Some(c) = count {
            ui.weak(c);
        }
        ui.with_layout(eframe::egui::Layout::right_to_left(eframe::egui::Align::Center), right);
    });
    separator(ui);
}

/// Full-width list row: the whole row is the hitbox, with selection/hover background.
/// The caller paints the content inside the returned rect.
pub fn row(ui: &mut Ui, height: f32, selected: bool, sense: Sense) -> (Rect, Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let v = ui.visuals();
    if selected {
        // Tint + accent bar instead of a solid fill, so colored text (errors, warnings) stays readable.
        ui.painter().rect_filled(rect, 3.0, v.selection.bg_fill.gamma_multiply(0.35));
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.5, v.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, v.widgets.hovered.weak_bg_fill);
    }
    (rect, resp)
}

/// One line of text starting at `left_center`, ending in "…" if wider than `max_width`.
pub fn text_truncated(painter: &Painter, left_center: Pos2, text: &str, font: FontId, color: Color32, max_width: f32) {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width(max_width.max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(left_center - vec2(0.0, galley.size().y / 2.0), galley, color);
}

/// Width of the label column in bar rows (startup phases, launches).
pub const BAR_LABEL_W: f32 = 240.0;
const BAR_VALUE_W: f32 = 96.0;
pub const BAR_ROW_H: f32 = 26.0;

/// `label | ████████░░░░ | value` with the bar filled to `fraction` (0..1).
pub fn bar_row(ui: &mut Ui, label: &str, fraction: f32, value: &str, color: Color32) -> Response {
    let (rect, resp) = row(ui, BAR_ROW_H, false, Sense::hover());
    let v = ui.visuals().clone();
    let painter = ui.painter();
    let mid = rect.center().y;
    text_truncated(painter, Pos2::new(rect.left() + SM, mid), label, FontId::proportional(14.0), v.text_color(), BAR_LABEL_W - SM);
    let value_right = rect.right() - SCROLL_GUTTER;
    painter.text(Pos2::new(value_right, mid), eframe::egui::Align2::RIGHT_CENTER, value, FontId::proportional(13.5), v.text_color());
    let track = Rect::from_min_max(Pos2::new(rect.left() + BAR_LABEL_W + SM, mid - 6.0), Pos2::new(value_right - BAR_VALUE_W, mid + 6.0));
    if track.width() > 0.0 {
        painter.rect_filled(track, 3.0, v.faint_bg_color.gamma_multiply(2.5));
        let fill = Rect::from_min_size(track.min, vec2(track.width() * fraction.clamp(0.0, 1.0), track.height()));
        painter.rect_filled(fill, 3.0, color);
    }
    resp
}

/// Full-width explanatory box (neutral, warning or error tint).
pub fn note(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui)) {
    boxed(color.gamma_multiply(0.5), color.gamma_multiply(0.10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}
