//! The look of the window: colors, type, icons and the controls every page is built from. Pages (app.rs) only
//! arrange these, so the look changes here in one place.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, Frame, Margin, Pos2, RichText, Sense, Stroke, Ui, vec2};

pub const BG: Color32 = Color32::from_rgb(9, 9, 11);
pub const RAIL: Color32 = Color32::from_rgb(0, 0, 0);
pub const CARD: Color32 = Color32::from_rgb(19, 19, 22);
pub const CARD_HI: Color32 = Color32::from_rgb(30, 30, 35);
pub const LINE: Color32 = Color32::from_rgb(38, 38, 44);
pub const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
pub const DIM: Color32 = Color32::from_rgb(180, 180, 186);
pub const FAINT: Color32 = Color32::from_rgb(142, 142, 150);   // 5.6:1 on CARD
pub const OFF: Color32 = Color32::from_rgb(64, 64, 70);
pub const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
pub const BAD: Color32 = Color32::from_rgb(248, 113, 113);
/// potato gold: the one color that means "Potato", used sparingly (the chosen page, Roblox in the chart)
pub const ACCENT: Color32 = Color32::from_rgb(240, 182, 82);

/// Windows' "Animation effects" setting: off turns every animation here off (reduced motion).
pub static MOTION: AtomicBool = AtomicBool::new(true);

/// An animation's length, or none when Windows has animations off. Kept short: under the 400 ms people notice.
pub fn dur(seconds: f32) -> f32 {
    if MOTION.load(Ordering::Relaxed) { seconds } else { 0.0 }
}

pub fn semi(text: &str, size: f32) -> RichText {
    RichText::new(text).size(size).family(FontFamily::Name("semi".into()))
}

pub fn small(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).size(12.5).color(FAINT)
}

/// The pixel potato from assets/logo.txt, `px` points per pixel.
pub fn logo(ui: &mut Ui, px: f32) {
    let lines: Vec<&str> = include_str!("../assets/logo.txt").lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let colors: HashMap<char, Color32> = lines.iter().filter(|l| l.get(1..2) == Some("="))
        .filter_map(|l| Some((l.chars().next()?, Color32::from_hex(l.get(2..9)?).ok()?))).collect();
    let rows: Vec<&str> = lines.into_iter().filter(|l| l.get(1..2) != Some("=")).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(rows[0].len() as f32 * px, rows.len() as f32 * px), Sense::hover());
    for (y, row) in rows.iter().enumerate() {
        for (x, k) in row.chars().enumerate() {
            let Some(&color) = colors.get(&k) else { continue };
            let min = rect.min + vec2(x as f32 * px, y as f32 * px);
            ui.painter().rect_filled(egui::Rect::from_min_size(min, vec2(px, px)), 0.0, color);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Grid,
    People,
    Monitor,
    Bolt,
    Sliders,
}

/// Simple line icons, drawn so they stay sharp at any scale.
pub fn icon(p: &egui::Painter, kind: Icon, c: Pos2, s: f32, color: Color32) {
    let st = Stroke::new(1.7, color);
    match kind {
        Icon::Grid => {
            let q = s * 0.38;
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let center = c + vec2(dx * (q / 2.0 + 1.5), dy * (q / 2.0 + 1.5));
                p.rect_stroke(egui::Rect::from_center_size(center, vec2(q, q)), 2.5, st, egui::StrokeKind::Middle);
            }
        }
        Icon::People => {
            p.circle_stroke(c + vec2(0.0, -s * 0.18), s * 0.17, st);
            let arc: Vec<Pos2> = (0..=16).map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                c + vec2(a.cos() * s * 0.34, s * 0.38 + a.sin() * s * 0.3)
            }).collect();
            p.add(egui::Shape::line(arc, st));
        }
        Icon::Monitor => {
            let r = egui::Rect::from_center_size(c + vec2(0.0, -s * 0.08), vec2(s * 0.86, s * 0.56));
            p.rect_stroke(r, 3.0, st, egui::StrokeKind::Middle);
            p.line_segment([c + vec2(0.0, s * 0.2), c + vec2(0.0, s * 0.34)], st);
            p.line_segment([c + vec2(-s * 0.2, s * 0.36), c + vec2(s * 0.2, s * 0.36)], st);
        }
        Icon::Bolt => {
            let pts = [(0.08, -0.42), (-0.24, 0.06), (0.0, 0.06), (-0.08, 0.42), (0.24, -0.08), (0.0, -0.08)]
                .map(|(x, y)| c + vec2(x * s, y * s));
            p.add(egui::Shape::closed_line(pts.to_vec(), st));
        }
        Icon::Sliders => {
            for (i, k) in [0.25_f32, 0.7, 0.4].into_iter().enumerate() {
                let y = c.y + (i as f32 - 1.0) * s * 0.28;
                p.line_segment([egui::pos2(c.x - s * 0.38, y), egui::pos2(c.x + s * 0.38, y)], st);
                p.circle_filled(egui::pos2(c.x - s * 0.38 + k * s * 0.76, y), 3.2, color);
            }
        }
    }
}

/// A page in the left rail: icon over its name, the whole block one big target (Fitts's law).
pub fn rail_item(ui: &mut Ui, kind: Icon, name: &str, picked: bool, alert: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered() || picked, dur(0.12));
    let p = ui.painter();
    if hover > 0.0 {
        p.rect_filled(rect.shrink2(vec2(8.0, 2.0)), 12.0, if picked { CARD_HI } else { CARD.gamma_multiply(hover) });
    }
    let ink = if picked { TEXT } else { DIM.lerp_to_gamma(TEXT, hover * 0.5) };
    icon(p, kind, rect.center() - vec2(0.0, 9.0), 22.0, if picked { ACCENT } else { ink });
    p.text(rect.center() + vec2(0.0, 15.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.5), ink);
    if alert {
        p.circle_filled(rect.center() + vec2(13.0, -18.0), 4.0, BAD);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, picked, name));
    resp
}

/// A card: rounded, one step lighter than the page, with an optional title and a note on its right.
pub fn card<R>(ui: &mut Ui, title: &str, right: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    let r = Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE.gamma_multiply(0.6))).corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(0, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if !title.is_empty() {
                Frame::new().inner_margin(Margin { left: 18, right: 18, top: 10, bottom: 4 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(semi(title, 15.0).color(TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(small(right)));
                    });
                });
            }
            body(ui)
        });
    ui.add_space(14.0);
    r.inner
}

/// One setting: label and note on the left, its control on the right, a line under it unless it's `last`.
/// The control gets the label's id for `labelled_by`, so screen readers name it.
pub fn row(ui: &mut Ui, label: &str, note: &str, last: bool, control: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            let id = ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 190.0).max(160.0));   // the note wraps instead of running under the control
                let id = ui.label(RichText::new(label).size(14.0).color(TEXT)).id;
                if !note.is_empty() {
                    ui.label(small(note));
                }
                id
            }).inner;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| control(ui, id));
        });
    });
    if !last {
        let rect = r.response.rect;
        ui.painter().hline(rect.x_range().shrink(18.0), rect.bottom(), Stroke::new(1.0, LINE));
    }
}

/// A row with more controls than fit beside its text: label and note on top, the controls in a line under them.
pub fn stack_row(ui: &mut Ui, label: &str, note: &str, last: bool, controls: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        let id = ui.label(semi(label, 15.0).color(TEXT)).id;
        ui.label(small(note));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| controls(ui, id));
    });
    if !last {
        let rect = r.response.rect;
        ui.painter().hline(rect.x_range().shrink(18.0), rect.bottom(), Stroke::new(1.0, LINE));
    }
}

/// A setting with an on/off switch where the whole row is the button (Fitts's law: a big target). Returns
/// true when it flipped.
pub fn switch_row(ui: &mut Ui, label: &str, note: &str, last: bool, on: &mut bool) -> bool {
    let bg = ui.painter().add(egui::Shape::Noop);   // hover tint, drawn under the row once its size is known
    let mut knob = egui::Rect::NOTHING;
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 70.0).max(160.0));
                ui.label(RichText::new(label).size(14.0).color(TEXT));
                if !note.is_empty() {
                    ui.label(small(note));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                knob = ui.allocate_exact_size(vec2(42.0, 22.0), Sense::hover()).0;
            });
        });
    });
    let rect = r.response.rect;
    let resp = ui.interact(rect, ui.scope_id().with(("switch", label)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *on, label));
    let flipped = resp.clicked();
    if flipped {
        *on = !*on;
    }
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
    ui.painter().set(bg, egui::Shape::rect_filled(rect, 10.0, Color32::from_white_alpha((hover * 6.0) as u8)));
    paint_switch(ui, knob, *on, resp.is_pointer_button_down_on(), resp.id);
    if !last {
        ui.painter().hline(rect.x_range().shrink(18.0), rect.bottom(), Stroke::new(1.0, LINE));
    }
    flipped
}

/// The switch: the knob shrinks a touch when pressed (anticipation), stretches as it travels (squash and
/// stretch), overshoots and settles (follow through), all eased in and out.
fn paint_switch(ui: &Ui, rect: egui::Rect, on: bool, pressed: bool, id: egui::Id) {
    let ctx = ui.ctx();
    let t = ctx.animate_bool_with_time_and_easing(id.with("knob"), on, dur(0.24), egui::emath::easing::back_out);
    let press = ctx.animate_bool_with_time(id.with("press"), pressed, dur(0.08));
    let fill = ctx.animate_bool_with_time(id.with("fill"), on, dur(0.16));
    let r = rect.height() / 2.0;
    let p = ui.painter();
    p.rect_filled(rect, r, Color32::from_gray(0).lerp_to_gamma(TEXT, fill));
    p.rect_stroke(rect, r, Stroke::new(1.0, FAINT.lerp_to_gamma(TEXT, fill)), egui::StrokeKind::Inside);
    let travel = (t.clamp(0.0, 1.0) * std::f32::consts::PI).sin();   // biggest mid-way
    let kr = (r - 5.0) * (1.0 - 0.18 * press);
    let (w, h) = (kr * (1.0 + 0.55 * travel), kr * (1.0 - 0.12 * travel));
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    let knob = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(w * 2.0, h * 2.0));
    p.rect_filled(knob, h, DIM.lerp_to_gamma(RAIL, fill));
}

/// A few choices side by side; the chosen one sits on a lighter pill that slides to the new choice.
/// Returns true when the choice changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, label: egui::Id, value: &mut T, options: &[(T, &str)]) -> bool {
    let font = egui::FontId::proportional(13.0);
    let widths: Vec<f32> = options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 26.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 6.0, 34.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 9.0, Color32::from_rgb(10, 10, 12));
    p.rect_stroke(rect, 9.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    let mut x = rect.left() + 3.0;
    let mut changed = false;
    let mut picked_at = None;
    let mut cells = Vec::new();
    for ((v, l), w) in options.iter().zip(&widths) {
        let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), vec2(*w, rect.height() - 6.0));
        let resp = ui.interact(cell, label.with(("seg", *l)), Sense::click());
        let on = *value == *v;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), on, *l));
        if resp.clicked() && !on {
            *value = *v;
            changed = true;
        }
        if *value == *v {
            picked_at = Some(cell);
        }
        cells.push((cell, *l, resp.hovered()));
        x += w;
    }
    if let Some(cell) = picked_at {
        let id = label.with("seg-pill");
        let left = ui.ctx().animate_value_with_time(id.with("l"), cell.left(), dur(0.18));
        let width = ui.ctx().animate_value_with_time(id.with("w"), cell.width(), dur(0.18));
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(left, cell.top()), vec2(width, cell.height())), 7.0, CARD_HI.lerp_to_gamma(Color32::from_gray(58), 0.5));
    }
    for (cell, l, hovered) in cells {
        let on = picked_at == Some(cell);
        p.text(cell.center(), egui::Align2::CENTER_CENTER, l, font.clone(), if on { TEXT } else if hovered { DIM } else { FAINT });
    }
    changed
}

/// A box for a whole number up to `max`, with its unit after it; empty means 0 (off). The number counts once
/// typing is done (Enter or clicking away): while typing 2500 MB the limit would otherwise be 2, 25 and 250 MB
/// for a moment, and clients over those would restart.
pub fn number_box(ui: &mut Ui, label: egui::Id, value: &mut u32, max: u32, unit: &str, empty: &str) {
    ui.label(RichText::new(unit).color(FAINT));
    let draft = label.with("draft");
    let mut text = ui.data(|d| d.get_temp::<String>(draft)).unwrap_or_else(|| if *value == 0 { String::new() } else { value.to_string() });
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0).hint_text(empty).align(egui::Align2::CENTER_CENTER)).labelled_by(label);
    text.retain(|c| c.is_ascii_digit());
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(draft, text));
    } else {
        if r.lost_focus() {
            *value = text.parse::<u32>().unwrap_or(0).min(max);
        }
        ui.data_mut(|d| d.remove::<String>(draft));
    }
}

/// A dropdown of fixed choices, for lists too long for `segmented`. Returns true when the choice changed.
pub fn dropdown<T: PartialEq + Copy>(ui: &mut Ui, salt: &str, label: egui::Id, value: &mut T, options: &[(T, String)]) -> bool {
    let shown = options.iter().find(|(v, _)| v == value).map_or(String::new(), |(_, l)| l.clone());
    let mut changed = false;
    let r = egui::ComboBox::from_id_salt(salt).selected_text(shown).width(150.0).show_ui(ui, |ui| {
        for (v, l) in options {
            changed |= ui.selectable_value(value, *v, l).changed();
        }
    });
    r.response.labelled_by(label);
    changed
}

/// The one white button on a screen, for the thing to do next.
pub fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    button(ui, text, false, true)
}

/// The app's buttons: one size and style, so they line up. They squash a little when pressed and show a
/// spinner while their work runs.
pub fn action(ui: &mut Ui, text: &str, busy: bool) -> egui::Response {
    button(ui, text, busy, false)
}

pub fn button(ui: &mut Ui, text: &str, busy: bool, primary: bool) -> egui::Response {
    let font = if primary { egui::FontId::new(13.5, FontFamily::Name("semi".into())) } else { egui::FontId::proportional(13.5) };
    let ink = if !ui.is_enabled() { FAINT } else if primary { RAIL } else { TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, ink);
    let size = vec2((galley.size().x + 30.0 + if busy { 20.0 } else { 0.0 }).max(84.0), 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let ctx = ui.ctx();
    let press = ctx.animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), dur(0.07));
    let hover = ctx.animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
    let rect = egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    let p = ui.painter();
    if primary && ui.is_enabled() {
        p.rect_filled(rect, 9.0, TEXT.lerp_to_gamma(Color32::from_gray(205), hover));
    } else {
        p.rect_filled(rect, 9.0, CARD_HI.lerp_to_gamma(Color32::from_gray(46), hover));
        p.rect_stroke(rect, 9.0, Stroke::new(1.0, LINE.lerp_to_gamma(Color32::from_gray(90), hover)), egui::StrokeKind::Inside);
    }
    let mut x = rect.center().x - galley.size().x / 2.0;
    if busy {
        x += 10.0;
        egui::Spinner::new().size(14.0).color(DIM).paint_at(ui, egui::Rect::from_center_size(egui::pos2(x - 14.0, rect.center().y), vec2(14.0, 14.0)));
    }
    p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    resp
}

/// A small rounded label with a dot, like "● Live" or "1.2 GB".
pub fn chip(ui: &mut Ui, text: &str, dot: Option<Color32>) {
    Frame::new().fill(CARD_HI).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(9, 3)).show(ui, |ui| {
        // always dot first, even inside a right-to-left row
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if let Some(c) = dot {
                let (r, _) = ui.allocate_exact_size(vec2(7.0, 12.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.2, c);
            }
            ui.label(RichText::new(text).size(12.0).color(DIM));
        });
    });
}

/// A round badge with the first letter of a name.
pub fn avatar(ui: &mut Ui, name: &str, size: f32, live: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, if live { ACCENT.gamma_multiply(0.22) } else { CARD_HI });
    p.circle_stroke(rect.center(), size / 2.0, Stroke::new(1.2, if live { ACCENT } else { LINE }));
    let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    p.text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::new(size * 0.42, FontFamily::Name("semi".into())), if live { ACCENT } else { DIM });
}

/// Lines of fractions (0 to 1) over time, oldest left, with a soft fill under the first.
pub fn chart(ui: &mut Ui, size: egui::Vec2, lines: &[(&[f32], Color32)]) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    for k in 0..=3 {
        let y = rect.bottom() - rect.height() * k as f32 / 3.0;
        p.hline(rect.x_range(), y, Stroke::new(1.0, LINE.gamma_multiply(0.7)));
    }
    for (i, (vals, color)) in lines.iter().enumerate() {
        if vals.len() < 2 {
            continue;
        }
        let pts: Vec<Pos2> = vals.iter().enumerate().map(|(j, v)| {
            egui::pos2(rect.left() + rect.width() * j as f32 / (vals.len() - 1) as f32, rect.bottom() - rect.height() * v.clamp(0.0, 1.0))
        }).collect();
        if i == 0 {
            let mut mesh = egui::Mesh::default();
            for (j, pt) in pts.iter().enumerate() {
                mesh.colored_vertex(*pt, color.gamma_multiply(0.18));
                mesh.colored_vertex(egui::pos2(pt.x, rect.bottom()), color.gamma_multiply(0.0));
                if j > 0 {
                    let b = (j as u32 - 1) * 2;
                    mesh.add_triangle(b, b + 1, b + 2);
                    mesh.add_triangle(b + 1, b + 2, b + 3);
                }
            }
            p.add(mesh);
        }
        p.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, *color)));
        if let Some(last) = pts.last() {
            p.circle_filled(*last, 3.5, *color);
        }
    }
}

/// Two lines of text in the middle of an empty area.
pub fn centered_note(ui: &Ui, rect: egui::Rect, title: &str, note: &str) {
    let p = ui.painter();
    p.text(rect.center() - vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, title, egui::FontId::new(14.0, FontFamily::Name("semi".into())), DIM);
    p.text(rect.center() + vec2(0.0, 11.0), egui::Align2::CENTER_CENTER, note, egui::FontId::proportional(12.0), FAINT);
}

/// A green tick in a circle (done) or an empty ring (not yet), for checklists.
pub fn check(ui: &mut Ui, done: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    if done {
        p.circle_filled(c, 10.0, GOOD.gamma_multiply(0.2));
        p.add(egui::Shape::line(vec![c + vec2(-4.5, 0.2), c + vec2(-1.2, 3.6), c + vec2(5.0, -3.4)], Stroke::new(2.0, GOOD)));
    } else {
        p.circle_stroke(c, 9.0, Stroke::new(1.5, OFF));
    }
}

pub fn style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("geist".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Geist-Regular.ttf"))));
    fonts.font_data.insert("geist-semi".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Geist-SemiBold.ttf"))));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "geist".into());
    fonts.families.insert(FontFamily::Name("semi".into()), vec!["geist-semi".into(), "geist".into()]);
    ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    ctx.set_fonts(fonts);

    // always dark: egui otherwise follows Windows' light mode for its own buttons, boxes and checkboxes
    ctx.set_theme(egui::Theme::Dark);
    let mut v = egui::Visuals::dark();
    // egui's dark-theme edge shading fattens dark text on the white buttons and toasts, and hinting snaps letters
    // to whole pixels, which looks jagged at 100% scaling: a middle gamma and no hinting keep every text smooth
    v.text_options.color_transfer_function = egui::epaint::FontColorTransferFunction::Gamma(0.8);
    v.text_options.font_hinting = false;
    v.panel_fill = BG;
    v.window_fill = CARD_HI;
    v.override_text_color = Some(TEXT);
    v.extreme_bg_color = Color32::from_rgb(10, 10, 12);   // text boxes
    v.selection.bg_fill = Color32::from_rgb(70, 70, 76);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(9);
    }
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 44, 50);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(90, 90, 98));
    v.widgets.open.weak_bg_fill = Color32::from_rgb(44, 44, 50);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(14.0, 8.0);
        s.spacing.interact_size.y = 34.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
    });
}
