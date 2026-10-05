//! Light and dark colour themes.
//!
//! Every panel keeps its own colour table, written for the light (grey)
//! theme. In dark mode each of those colours goes through `c`, which maps it
//! to its dark counterpart, so one switch restyles the whole interface. The
//! egui widget style (buttons, fields, selection) is set per theme in `apply`.
//! The choice is remembered between sessions.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy_egui::egui::{self, Color32, Stroke};

static DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool { DARK.load(Ordering::Relaxed) }

/// Switch theme, restyle egui and remember the choice.
pub fn set_dark(ctx: &egui::Context, dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    apply(ctx);
    if let Some(file) = crate::file_browser::config_path("theme.txt") {
        if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(file, if dark { "dark" } else { "light" });
    }
}

/// Read the remembered theme and style egui. Call once at startup.
pub fn init(ctx: &egui::Context) {
    let saved = crate::file_browser::config_path("theme.txt")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.trim() == "dark")
        .unwrap_or(false);
    DARK.store(saved, Ordering::Relaxed);
    apply(ctx);
}

/// A colour of the light theme, or its dark counterpart in dark mode.
pub fn c(r: u8, g: u8, b: u8) -> Color32 {
    if is_dark() { dark_of(r, g, b) } else { Color32::from_rgb(r, g, b) }
}

/// A surface that sits on top of a canvas (a node body or title). In dark
/// mode it is lifted above the plain mapping so nodes stand out from the
/// canvas behind them.
pub fn raised(r: u8, g: u8, b: u8) -> Color32 {
    if !is_dark() { return Color32::from_rgb(r, g, b); }
    let d = dark_of(r, g, b);
    Color32::from_rgb(d.r().saturating_add(24), d.g().saturating_add(24), d.b().saturating_add(24))
}

/// An outline. Darker than its surroundings in the light theme, lighter in
/// the dark theme, where a darker line would disappear.
pub fn outline(r: u8, g: u8, b: u8) -> Color32 {
    if is_dark() { Color32::from_gray(96) } else { Color32::from_rgb(r, g, b) }
}

/// Grey level of the light theme to grey level of the dark theme. The light
/// theme's surfaces sit between 45 and 130; they keep their order but move
/// down to 12..50, which leaves room for much stronger text contrast.
fn dark_level(v: f32) -> f32 {
    const POINTS: [(f32, f32); 6] = [(40.0, 12.0), (72.0, 22.0), (100.0, 30.0), (118.0, 40.0), (130.0, 50.0), (150.0, 64.0)];
    if v <= POINTS[0].0 { return POINTS[0].1; }
    for w in POINTS.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if v <= x1 { return y0 + (v - x0) / (x1 - x0) * (y1 - y0); }
    }
    v
}

fn dark_of(r: u8, g: u8, b: u8) -> Color32 {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    // Text, wires, outlines and bright accents already read well on dark.
    if max >= 150 { return Color32::from_rgb(r, g, b); }
    if max - min > 12 {
        // Tinted surface (selection, coloured node bodies, clip bar): keep
        // the hue and stay clearly above the neutral surfaces around it.
        let k = 0.7;
        return Color32::from_rgb((r as f32 * k) as u8, (g as f32 * k) as u8, (b as f32 * k) as u8);
    }
    // Neutral surface: remap the level, keep whatever slight tint it has.
    let avg = (r as f32 + g as f32 + b as f32) / 3.0;
    let k   = dark_level(avg) / avg.max(1.0);
    Color32::from_rgb((r as f32 * k) as u8, (g as f32 * k) as u8, (b as f32 * k) as u8)
}

/// Style egui's own widgets for the current theme.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let grey = Color32::from_gray;

    if is_dark() {
        let text     = grey(232);
        let text_dim = grey(180);
        let v = &mut style.visuals;
        v.dark_mode        = true;
        v.panel_fill       = grey(40);
        v.window_fill      = grey(46);
        v.extreme_bg_color = grey(22);          // text fields, drag values
        v.faint_bg_color   = grey(48);
        v.code_bg_color    = grey(22);
        v.window_stroke    = Stroke::new(1.0_f32, grey(84));

        v.widgets.noninteractive.bg_fill      = grey(40);
        v.widgets.noninteractive.weak_bg_fill = grey(40);
        v.widgets.noninteractive.bg_stroke    = Stroke::new(1.0_f32, grey(68));   // separators
        v.widgets.noninteractive.fg_stroke    = Stroke::new(1.0_f32, text_dim);

        v.widgets.inactive.bg_fill      = grey(70);
        v.widgets.inactive.weak_bg_fill = grey(70);                               // buttons
        v.widgets.inactive.bg_stroke    = Stroke::NONE;
        v.widgets.inactive.fg_stroke    = Stroke::new(1.0_f32, text);

        v.widgets.hovered.bg_fill      = grey(92);
        v.widgets.hovered.weak_bg_fill = grey(92);
        v.widgets.hovered.bg_stroke    = Stroke::new(1.0_f32, grey(150));
        v.widgets.hovered.fg_stroke    = Stroke::new(1.5_f32, Color32::WHITE);

        v.widgets.active.bg_fill      = grey(110);
        v.widgets.active.weak_bg_fill = grey(110);
        v.widgets.active.bg_stroke    = Stroke::new(1.0_f32, Color32::WHITE);
        v.widgets.active.fg_stroke    = Stroke::new(2.0_f32, Color32::WHITE);

        v.widgets.open.bg_fill      = grey(30);
        v.widgets.open.weak_bg_fill = grey(58);
        v.widgets.open.bg_stroke    = Stroke::new(1.0_f32, grey(110));
        v.widgets.open.fg_stroke    = Stroke::new(1.0_f32, text);

        v.selection.bg_fill = Color32::from_rgb(52, 96, 148);
        v.selection.stroke  = Stroke::new(1.0_f32, Color32::from_rgb(170, 210, 245));
        v.hyperlink_color   = Color32::from_rgb(140, 190, 240);
    } else {
        // The original grey theme, unchanged.
        let defaults  = egui::Visuals::dark();
        let bg_fill       = Color32::from_rgb(118, 118, 118);
        let bg_fill_dark  = Color32::from_rgb(100, 100, 100);
        let bg_fill_mid   = Color32::from_rgb(110, 110, 110);
        let stroke_subtle = Stroke::new(1.0_f32, Color32::from_rgb(80, 80, 80));
        let text_col      = Color32::from_rgb(230, 230, 230);
        let text_dim      = Color32::from_rgb(190, 190, 190);

        // Start from egui's defaults so nothing set by the dark theme lingers.
        style.visuals = defaults;
        style.visuals.panel_fill           = bg_fill;
        style.visuals.window_fill          = bg_fill;
        style.visuals.extreme_bg_color     = bg_fill_dark;
        style.visuals.faint_bg_color       = bg_fill_mid;
        style.visuals.code_bg_color        = bg_fill_dark;
        style.visuals.window_stroke        = stroke_subtle;
        style.visuals.widgets.noninteractive.bg_fill   = bg_fill;
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text_dim);
        style.visuals.widgets.inactive.bg_fill         = bg_fill_mid;
        style.visuals.widgets.inactive.fg_stroke       = Stroke::new(1.0_f32, text_col);
        style.visuals.widgets.hovered.bg_fill          = Color32::from_rgb(140, 140, 140);
        style.visuals.widgets.hovered.fg_stroke        = Stroke::new(1.0_f32, text_col);
        style.visuals.widgets.active.bg_fill           = Color32::from_rgb(150, 150, 150);
        style.visuals.widgets.active.fg_stroke         = Stroke::new(1.0_f32, Color32::WHITE);
        style.visuals.widgets.open.bg_fill             = bg_fill_dark;
        style.visuals.widgets.open.fg_stroke           = Stroke::new(1.0_f32, text_col);
        style.visuals.selection.bg_fill    = Color32::from_rgb(100, 130, 160);
        style.visuals.selection.stroke     = Stroke::new(1.0_f32, Color32::from_rgb(160, 195, 225));
        style.visuals.hyperlink_color      = Color32::from_rgb(160, 195, 230);
    }

    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f32 {
        let f = |v: u8| { let x = v as f32 / 255.0; if x <= 0.03928 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) } };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (hi, lo) = if lum(a) > lum(b) { (lum(a), lum(b)) } else { (lum(b), lum(a)) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn dark_surfaces_keep_their_order() {
        // Levels used as surfaces in the light theme, darkest to lightest.
        let levels = [45u8, 58, 66, 72, 78, 90, 100, 105, 108, 118, 130];
        let mapped: Vec<u8> = levels.iter().map(|v| dark_of(*v, *v, *v).r()).collect();
        assert!(mapped.windows(2).all(|w| w[0] <= w[1]), "{mapped:?}");
        assert!(*mapped.last().unwrap() <= 50 && mapped[0] >= 12);
    }

    #[test]
    fn dark_theme_has_more_text_contrast() {
        let panel = (118, 118, 118);
        for text in [(230u8, 230u8, 230u8), (210, 210, 210), (190, 190, 190), (170, 170, 170)] {
            let light = contrast(Color32::from_rgb(text.0, text.1, text.2), Color32::from_rgb(panel.0, panel.1, panel.2));
            let dark  = contrast(dark_of(text.0, text.1, text.2), dark_of(panel.0, panel.1, panel.2));
            assert!(dark > light * 2.0, "{text:?}: {light:.1} -> {dark:.1}");
            assert!(dark >= 5.5, "{text:?}: {dark:.1}");
        }
    }

    #[test]
    fn nodes_stand_out_from_the_canvas_in_dark_mode() {
        DARK.store(true, Ordering::Relaxed);
        let canvas = c(100, 100, 100);
        let (body, title, edge) = (raised(130, 130, 130), raised(105, 105, 105), outline(70, 70, 70));
        DARK.store(false, Ordering::Relaxed);
        assert!(contrast(body, canvas) > 1.6, "{:.2}", contrast(body, canvas));
        assert!(contrast(title, canvas) > 1.3, "{:.2}", contrast(title, canvas));
        assert!(contrast(edge, canvas) > 2.0);
        // Light theme is returned untouched.
        assert_eq!(raised(130, 130, 130), Color32::from_rgb(130, 130, 130));
        assert_eq!(outline(70, 70, 70), Color32::from_rgb(70, 70, 70));
    }

    #[test]
    fn selection_stays_visible_on_dark_surfaces() {
        // Selected row on the panel, selected node on the canvas, clip on the track.
        for (sel, bg) in [((90, 105, 120), (118, 118, 118)), ((110, 120, 135), (130, 130, 130)), ((96, 122, 148), (78, 78, 78))] {
            let c = contrast(dark_of(sel.0, sel.1, sel.2), dark_of(bg.0, bg.1, bg.2));
            assert!(c > 1.35, "{sel:?} on {bg:?}: {c:.2}");
        }
    }
}
