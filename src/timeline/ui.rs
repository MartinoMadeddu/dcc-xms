use bevy_egui::egui;

use super::{Playback, RulerUnit, TimelineSource, TimelineState};
use crate::core::anim::AnimData;

mod xsi {
    use bevy_egui::egui::Color32;
    pub const PANEL_BG:   Color32 = Color32::from_rgb(104, 104, 104);
    pub const TRACK_BG:   Color32 = Color32::from_rgb( 78,  78,  78);
    pub const INPUT_BAR:  Color32 = Color32::from_rgb(116, 116, 116);
    pub const CLIP_BAR:   Color32 = Color32::from_rgb( 96, 122, 148);
    pub const CLIP_EDGE:  Color32 = Color32::from_rgb(160, 195, 225);
    pub const TICK:       Color32 = Color32::from_rgb(150, 150, 150);
    pub const TICK_MINOR: Color32 = Color32::from_rgb(112, 112, 112);
    pub const TEXT:       Color32 = Color32::from_rgb(230, 230, 230);
    pub const TEXT_DIM:   Color32 = Color32::from_rgb(185, 185, 185);
    pub const PLAYHEAD:   Color32 = Color32::from_rgb(235, 190,  95);
}

const TRACK_H:   f32 = 40.0;
const TRACK_PAD: f32 = 12.0;

/// Frame range of `other` in frames of `clip`'s rate, matched by timecode.
fn range_in(clip: &AnimData, other: &AnimData) -> (i64, i64) {
    (
        clip.frame_at(other.tc_seconds(other.start_frame)),
        clip.frame_at(other.tc_seconds(other.end_frame())),
    )
}

/// Advances playback and draws the panel. `dt` is the frame's delta time.
/// `keys` is false while a text field has keyboard focus.
pub fn draw_timeline(
    ui:    &mut egui::Ui,
    pb:    &mut Playback,
    state: &TimelineState,
    dt:    f64,
    keys:  bool,
) {
    egui::Frame::none().fill(xsi::PANEL_BG).inner_margin(6.0).show(ui, |ui| {
        match state {
            TimelineState::Source(src) => draw_source(ui, pb, src, dt, keys),
            TimelineState::NoTimeData { node_name } => {
                pb.playing = false;
                draw_empty(ui, &format!("\"{node_name}\" has no time data. Select a node that outputs a clip."));
            }
            TimelineState::Nothing => {
                pb.playing = false;
                draw_empty(ui, "No clip. Add a Load FBX or Test Clip node and select it.");
            }
        }
    });
}

fn draw_empty(ui: &mut egui::Ui, msg: &str) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::TEXT, egui::RichText::new("Timeline").strong());
        ui.separator();
        ui.colored_label(xsi::TEXT_DIM, msg);
    });
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TRACK_H), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, xsi::TRACK_BG);
}

fn draw_source(ui: &mut egui::Ui, pb: &mut Playback, src: &TimelineSource, dt: f64, keys: bool) {
    let clip = &src.clip;
    let fps  = clip.rate.fps();

    // ── Ranges, all in absolute frames at the clip's rate ────────────────────
    let (out_start, out_end) = (clip.start_frame, clip.end_frame());
    let (mut ext_start, mut ext_end) = (out_start, out_end);
    // The incoming clip is shown only where it shares time with the output.
    // After a Set Timecode the two are hours apart and it would only shrink
    // the view.
    let input_range = src.input.as_ref()
        .map(|i| range_in(clip, i))
        .filter(|(a, b)| *a <= out_end && *b >= out_start);
    if let Some((a, b)) = input_range {
        ext_start = ext_start.min(a);
        ext_end   = ext_end.max(b);
    }

    let set_frame = |pb: &mut Playback, f: i64| {
        pb.time = clip.tc_seconds(f.clamp(ext_start, ext_end));
    };

    // ── Advance ──────────────────────────────────────────────────────────────
    if pb.playing {
        pb.frac += dt * fps;
        let whole = pb.frac.floor();
        pb.frac  -= whole;
        let f = clip.frame_at(pb.time) + whole as i64;
        if f < out_start {
            set_frame(pb, out_start);
        } else if f > out_end {
            if pb.looping && out_end > out_start {
                set_frame(pb, out_start);
            } else {
                set_frame(pb, out_end);
                pb.playing = false;
            }
        } else {
            set_frame(pb, f);
        }
    } else {
        pb.frac = 0.0;
        let f = clip.frame_at(pb.time);
        if f < ext_start || f > ext_end {
            set_frame(pb, if f < ext_start { out_start } else { out_end });
        }
    }

    // ── Keys ─────────────────────────────────────────────────────────────────
    if keys {
        let (space, left, right, home, end) = ui.input(|i| (
            i.key_pressed(egui::Key::Space),
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
        ));
        let cur = clip.frame_at(pb.time);
        if space { pb.playing = !pb.playing; }
        if left  { pb.playing = false; set_frame(pb, cur - 1); }
        if right { pb.playing = false; set_frame(pb, cur + 1); }
        if home  { set_frame(pb, out_start); }
        if end   { set_frame(pb, out_end); }
    }

    let mut frame = clip.frame_at(pb.time).clamp(ext_start, ext_end);

    // ── Transport row ────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("First frame (Home)").clicked() { set_frame(pb, out_start); }
        if ui.button("◀").on_hover_text("Previous frame (Left)").clicked() {
            pb.playing = false; set_frame(pb, frame - 1);
        }
        let label = if pb.playing { "⏸" } else { "▶" };
        if ui.add(egui::Button::new(label).min_size(egui::vec2(34.0, 0.0)))
            .on_hover_text("Play / pause (Space)").clicked()
        {
            pb.playing = !pb.playing;
            if pb.playing && frame >= out_end { set_frame(pb, out_start); }
        }
        if ui.button("▶").on_hover_text("Next frame (Right)").clicked() {
            pb.playing = false; set_frame(pb, frame + 1);
        }
        if ui.button("⏭").on_hover_text("Last frame (End)").clicked() { set_frame(pb, out_end); }
        ui.checkbox(&mut pb.looping, "Loop");

        ui.separator();
        ui.colored_label(xsi::TEXT_DIM, "Frame");
        let mut f = frame;
        if ui.add(egui::DragValue::new(&mut f).range(ext_start..=ext_end).speed(0.25)).changed() {
            pb.playing = false;
            set_frame(pb, f);
        }
        ui.colored_label(xsi::TEXT,
            egui::RichText::new(clip.timecode(frame).to_string()).monospace().size(14.0));

        ui.separator();
        ui.selectable_value(&mut pb.ruler, RulerUnit::Frames,   "Frames");
        ui.selectable_value(&mut pb.ruler, RulerUnit::Timecode, "Timecode");

        // ── What the timeline is following ───────────────────────────────────
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let rate = format!("{} fps{}", clip.rate.label(), if clip.drop_frame { " DF" } else { "" });
            ui.colored_label(xsi::TEXT_DIM, format!(
                "{}  |  {} frames  |  {} - {}  |  {} joints",
                rate, clip.frames,
                clip.timecode(out_start), clip.timecode(out_end),
                clip.joints.len(),
            ));
            ui.colored_label(xsi::TEXT, egui::RichText::new(format!(
                "{}{} / {}",
                if src.from_selection { "" } else { "viewed: " },
                src.node_name, clip.name,
            )).strong());
        });
    });

    frame = clip.frame_at(pb.time).clamp(ext_start, ext_end);

    // ── Scrub track ──────────────────────────────────────────────────────────
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TRACK_H), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, xsi::TRACK_BG);

    let x0   = rect.min.x + TRACK_PAD;
    let x1   = rect.max.x - TRACK_PAD;
    let span = (ext_end - ext_start).max(1) as f32;
    let ppf  = (x1 - x0) / span;                         // pixels per frame
    let x_of = |f: i64| x0 + (f - ext_start) as f32 * ppf;

    let bar_top = rect.min.y + 18.0;
    let bar = |a: i64, b: i64| egui::Rect::from_min_max(
        egui::pos2(x_of(a) - 1.0, bar_top), egui::pos2(x_of(b) + 1.0, rect.max.y - 3.0));

    // Clip entering the node (context), then the clip the node outputs.
    if let Some((a, b)) = input_range {
        painter.rect_filled(bar(a, b), 2.0, xsi::INPUT_BAR);
    }
    painter.rect_filled(bar(out_start, out_end), 2.0, xsi::CLIP_BAR);
    painter.rect_stroke(bar(out_start, out_end), 2.0, egui::Stroke::new(1.0_f32, xsi::CLIP_EDGE));

    // ── Ruler ────────────────────────────────────────────────────────────────
    // Tick spacing follows the clip's timebase so major ticks land on seconds.
    let tb = clip.rate.timebase();
    let mut steps: Vec<i64> = vec![1, 2, 5, 10];
    for m in [1, 2, 5, 10, 30, 60, 120, 300, 600, 1800, 3600] { steps.push(tb * m); }
    steps.sort_unstable();
    steps.dedup();

    let label_px = if pb.ruler == RulerUnit::Timecode { 96.0 } else { 56.0 };
    let major_i  = steps.iter().position(|s| *s as f32 * ppf >= label_px).unwrap_or(steps.len() - 1);
    let major    = steps[major_i];
    let minor    = steps[..major_i].iter().copied()
        .find(|s| *s as f32 * ppf >= 5.0 && major % *s == 0);

    if let Some(minor) = minor {
        let mut f = ext_start.div_euclid(minor) * minor;
        while f <= ext_end {
            if f >= ext_start && f % major != 0 {
                let x = x_of(f);
                painter.line_segment(
                    [egui::pos2(x, rect.min.y + 12.0), egui::pos2(x, bar_top)],
                    egui::Stroke::new(1.0_f32, xsi::TICK_MINOR));
            }
            f += minor;
        }
    }
    let mut f = ext_start.div_euclid(major) * major;
    while f <= ext_end {
        if f >= ext_start {
            let x = x_of(f);
            painter.line_segment(
                [egui::pos2(x, rect.min.y + 5.0), egui::pos2(x, bar_top)],
                egui::Stroke::new(1.0_f32, xsi::TICK));
            let text = match pb.ruler {
                RulerUnit::Frames   => f.to_string(),
                RulerUnit::Timecode => clip.timecode(f).to_string(),
            };
            painter.text(egui::pos2(x + 3.0, rect.min.y + 2.0), egui::Align2::LEFT_TOP,
                text, egui::FontId::proportional(10.0), xsi::TEXT_DIM);
        }
        f += major;
    }

    // ── Scrub ────────────────────────────────────────────────────────────────
    if resp.dragged() || resp.clicked() || resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let f = ext_start + ((p.x - x0) / ppf).round() as i64;
            pb.playing = false;
            set_frame(pb, f);
            frame = clip.frame_at(pb.time).clamp(ext_start, ext_end);
        }
    }

    // ── Playhead ─────────────────────────────────────────────────────────────
    let px = x_of(frame);
    painter.line_segment(
        [egui::pos2(px, rect.min.y), egui::pos2(px, rect.max.y)],
        egui::Stroke::new(2.0_f32, xsi::PLAYHEAD));
    let tag = match pb.ruler {
        RulerUnit::Frames   => frame.to_string(),
        RulerUnit::Timecode => clip.timecode(frame).to_string(),
    };
    let galley = painter.layout_no_wrap(tag, egui::FontId::proportional(10.0), egui::Color32::BLACK);
    let w      = galley.size().x + 8.0;
    let tag_x  = (px + 2.0).min(rect.max.x - w);
    let tag_r  = egui::Rect::from_min_size(
        egui::pos2(tag_x, rect.max.y - 16.0), egui::vec2(w, 14.0));
    painter.rect_filled(tag_r, 2.0, xsi::PLAYHEAD);
    painter.galley(egui::pos2(tag_r.min.x + 4.0, tag_r.min.y + 1.0), galley, egui::Color32::BLACK);
}
