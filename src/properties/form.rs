//! The layout every node's properties share.
//!
//! A node's parameters are in groups. A group is a card with a title; under
//! it, rows. A row is a label, right-aligned in a column of the same width
//! in every group, and its value to the right. Values fill the width they
//! are given, so sliders, fields and lists line up down the panel.
//!
//! Explanations go in tooltips, on a group's title or a row's label, not in
//! the panel. What the panel shows is the parameters and the state of the
//! node: what came in, what goes out, what went wrong.

use bevy_egui::egui;

/// Colours of the panel. Light theme values; `theme::c` gives the dark
/// counterpart.
#[allow(non_snake_case)]
pub mod ink {
    use bevy_egui::egui::Color32;
    pub fn PANEL_BG() -> Color32 { crate::theme::c(118, 118, 118) }
    pub fn GROUP_BG() -> Color32 { crate::theme::c(108, 108, 108) }
    pub fn TITLE() -> Color32 { crate::theme::c(248, 248, 248) }
    pub fn LABEL() -> Color32 { crate::theme::c(222, 222, 222) }
    pub fn VALUE() -> Color32 { crate::theme::c(240, 240, 240) }
    pub fn DIM() -> Color32 { crate::theme::c(200, 200, 200) }
    /// The rail of a slider, so it shows on a group.
    pub fn RAIL() -> Color32 { crate::theme::c(146, 146, 146) }
    pub fn OK() -> Color32 { Color32::from_rgb(150, 210, 150) }
    pub fn BAD() -> Color32 { Color32::from_rgb(235, 140, 120) }
}

/// Space between groups.
pub const GAP: f32 = 6.0;

/// How a status line reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status { Ok, Bad, Info }

/// Rows of one group.
pub struct Rows<'a> {
    pub ui:  &'a mut egui::Ui,
    label_w: f32,
}

/// Width of the label column: the same in every group of a panel.
fn label_width(ui: &egui::Ui) -> f32 { (ui.available_width() * 0.34).clamp(76.0, 132.0) }

/// A group with a title, and an optional tooltip on the title.
pub fn group(ui: &mut egui::Ui, title: &str, hint: Option<&str>, body: impl FnOnce(&mut Rows)) {
    group_with(ui, title, hint, |_| {}, body);
}

/// A group whose title row also holds small controls, at its right.
pub fn group_with(
    ui: &mut egui::Ui, title: &str, hint: Option<&str>,
    header: impl FnOnce(&mut egui::Ui), body: impl FnOnce(&mut Rows),
) {
    let label_w = label_width(ui);
    egui::Frame::none()
        .fill(ink::GROUP_BG())
        .inner_margin(egui::Margin { left: 8.0, right: 8.0, top: 5.0, bottom: 7.0 })
        .rounding(4.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let r = ui.label(egui::RichText::new(title).strong().size(12.5).color(ink::TITLE()));
                if let Some(h) = hint { r.on_hover_text(h); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), header);
            });
            ui.add_space(3.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            body(&mut Rows { ui, label_w });
        });
    ui.add_space(GAP);
}

impl<'a> Rows<'a> {
    /// A row: the label, then whatever `value` draws in the value column.
    pub fn row<R>(&mut self, label: &str, hint: Option<&str>, value: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let label_w = self.label_w;
        self.ui.horizontal(|ui| {
            let h = ui.spacing().interact_size.y;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(label_w, h), egui::Sense::hover());
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            painter.text(rect.right_center() - egui::vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, label,
                egui::FontId::proportional(12.5), ink::LABEL());
            if let Some(hint) = hint { resp.on_hover_text(hint); }
            else if label.len() > 16 { resp.on_hover_text(label); }
            let w = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(w, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(w);
                value(ui)
            }).inner
        }).inner
    }

    /// Rows that are greyed out and cannot be changed while `on` is false.
    pub fn enabled(&mut self, on: bool, body: impl FnOnce(&mut Rows)) {
        let label_w = self.label_w;
        self.ui.add_enabled_ui(on, |ui| body(&mut Rows { ui, label_w }));
    }

    /// Content across the whole group, without a label.
    pub fn wide<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R { add(self.ui) }

    /// A number on a slider, with its unit.
    pub fn slider(&mut self, label: &str, hint: Option<&str>, v: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> egui::Response {
        self.row(label, hint, |ui| {
            slider_look(ui);
            ui.add(egui::Slider::new(v, range).suffix(unit).max_decimals(3))
        })
    }

    /// A whole number on a slider.
    pub fn slider_u32(&mut self, label: &str, hint: Option<&str>, v: &mut u32, range: std::ops::RangeInclusive<u32>, unit: &str) -> egui::Response {
        self.row(label, hint, |ui| {
            slider_look(ui);
            ui.add(egui::Slider::new(v, range).suffix(unit))
        })
    }

    /// A number with no fixed range, dragged or typed.
    pub fn drag(&mut self, label: &str, hint: Option<&str>, v: &mut f32, speed: f64, unit: &str) -> egui::Response {
        self.row(label, hint, |ui| {
            let h = ui.spacing().interact_size.y;
            ui.add_sized([ui.available_width(), h], egui::DragValue::new(v).speed(speed).max_decimals(3).suffix(unit))
        })
    }

    /// A whole number, dragged or typed, within a range.
    pub fn drag_u32(&mut self, label: &str, hint: Option<&str>, v: &mut u32, range: std::ops::RangeInclusive<u32>) -> egui::Response {
        self.row(label, hint, |ui| {
            let h = ui.spacing().interact_size.y;
            ui.add_sized([ui.available_width(), h], egui::DragValue::new(v).range(range).speed(0.1))
        })
    }

    /// X, Y and Z side by side.
    pub fn xyz(&mut self, label: &str, hint: Option<&str>, v: &mut [f32; 3], speed: f64, unit: &str) -> bool {
        self.row(label, hint, |ui| xyz_fields(ui, v, speed, unit))
    }

    /// Two values side by side: U and V.
    pub fn uv(&mut self, label: &str, hint: Option<&str>, v: &mut [f32; 2], speed: f64) -> bool {
        self.row(label, hint, |ui| {
            ui.horizontal(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let w = ((ui.available_width() - gap) / 2.0).max(30.0);
                let h = ui.spacing().interact_size.y;
                let mut changed = false;
                for (a, p) in v.iter_mut().zip(["U ", "V "]) {
                    changed |= ui.add_sized([w, h], egui::DragValue::new(a).speed(speed).max_decimals(4).prefix(p)).changed();
                }
                changed
            }).inner
        })
    }

    pub fn toggle(&mut self, label: &str, hint: Option<&str>, v: &mut bool) -> egui::Response {
        self.row(label, hint, |ui| check(ui, v))
    }

    /// One of a few named choices: side by side when they fit, a list when not.
    pub fn choice<T: PartialEq + Clone>(&mut self, label: &str, hint: Option<&str>, v: &mut T, options: &[(T, &str)]) -> bool {
        let id = self.ui.id().with(label);
        self.row(label, hint, |ui| choice_widget(ui, id, v, options))
    }

    pub fn text(&mut self, label: &str, hint: Option<&str>, v: &mut String, placeholder: &str) -> egui::Response {
        self.row(label, hint, |ui| {
            ui.add(egui::TextEdit::singleline(v).hint_text(placeholder).desired_width(ui.available_width()))
        })
    }

    /// A value that is shown, not edited.
    pub fn value(&mut self, label: &str, text: impl Into<String>) {
        let text = text.into();
        self.row(label, None, |ui| ui.add(egui::Label::new(egui::RichText::new(text).color(ink::VALUE())).wrap()));
    }

    /// A line of state in the value column: loaded, missing, solved.
    pub fn status(&mut self, s: Status, text: impl Into<String>) {
        let text = text.into();
        self.row("", None, |ui| status_label(ui, s, &text));
    }

    /// Buttons in the value column.
    pub fn buttons<R>(&mut self, label: &str, hint: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        self.row(label, hint, |ui| ui.horizontal_wrapped(add).inner)
    }
}

/// A checkbox whose box shows on the group's background in every theme.
pub fn check(ui: &mut egui::Ui, v: &mut bool) -> egui::Response {
    let w = &mut ui.visuals_mut().widgets;
    for s in [&mut w.inactive, &mut w.hovered] {
        s.bg_stroke = egui::Stroke::new(1.0, ink::LABEL());
    }
    w.inactive.bg_fill = ink::PANEL_BG();
    ui.add(egui::Checkbox::without_text(v))
}

/// Sliders fill the value column, with a rail that shows on the group.
pub fn slider_look(ui: &mut egui::Ui) {
    ui.spacing_mut().slider_width = (ui.available_width() - 66.0).max(40.0);
    ui.visuals_mut().widgets.inactive.bg_fill = ink::RAIL();
}

pub fn status_label(ui: &mut egui::Ui, s: Status, text: &str) -> egui::Response {
    let (mark, col) = match s { Status::Ok => ("✔ ", ink::OK()), Status::Bad => ("× ", ink::BAD()), Status::Info => ("", ink::DIM()) };
    ui.add(egui::Label::new(egui::RichText::new(format!("{mark}{text}")).color(col)).wrap())
}

/// X, Y, Z fields filling the width.
pub fn xyz_fields(ui: &mut egui::Ui, v: &mut [f32; 3], speed: f64, unit: &str) -> bool {
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let w = ((ui.available_width() - 2.0 * gap) / 3.0).max(30.0);
        let h = ui.spacing().interact_size.y;
        let mut changed = false;
        for (a, p) in v.iter_mut().zip(["X ", "Y ", "Z "]) {
            changed |= ui.add_sized([w, h], egui::DragValue::new(a).speed(speed).max_decimals(3).prefix(p).suffix(unit)).changed();
        }
        changed
    }).inner
}

/// Side-by-side buttons for up to four short choices, else a drop-down.
pub fn choice_widget<T: PartialEq + Clone>(ui: &mut egui::Ui, id: egui::Id, v: &mut T, options: &[(T, &str)]) -> bool {
    let chars: usize = options.iter().map(|o| o.1.len()).sum();
    let fits = options.len() <= 4 && (chars as f32) * 7.5 + options.len() as f32 * 18.0 < ui.available_width();
    let before = options.iter().position(|o| o.0 == *v);
    if fits {
        ui.horizontal(|ui| {
            for (t, name) in options {
                if ui.selectable_label(*v == *t, *name).clicked() { *v = t.clone(); }
            }
        });
    } else {
        let current = options.iter().find(|o| o.0 == *v).map(|o| o.1).unwrap_or("");
        egui::ComboBox::from_id_source(id).width(ui.available_width() - 8.0).selected_text(current).show_ui(ui, |ui| {
            for (t, name) in options {
                if ui.selectable_label(*v == *t, *name).clicked() { *v = t.clone(); }
            }
        });
    }
    options.iter().position(|o| o.0 == *v) != before
}
