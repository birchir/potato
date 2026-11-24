//! The look of the window: colors, type, icons and the controls every page is built from. Pages (app.rs) only
//! arrange these, so the look changes here in one place. Quiet and dense like Linear: near-black greys in a few
//! steps, thin lines instead of boxes in boxes, white for the one thing to press, and green only for "running"
//! or "done".

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, Frame, Margin, Pos2, RichText, Sense, Stroke, Ui, vec2};

/// behind everything: the sidebar sits straight on it
pub const CANVAS: Color32 = Color32::from_rgb(8, 9, 10);
/// the page, one step up, in a rounded panel
pub const PANEL: Color32 = Color32::from_rgb(15, 16, 17);
/// a group of rows on the page
pub const CARD: Color32 = Color32::from_rgb(20, 21, 23);
/// hovered rows, the chosen page, secondary buttons
pub const CARD_HI: Color32 = Color32::from_rgb(28, 29, 32);
pub const LINE: Color32 = Color32::from_rgb(35, 36, 40);
pub const LINE_HI: Color32 = Color32::from_rgb(52, 53, 58);
pub const TEXT: Color32 = Color32::from_rgb(238, 238, 240);
pub const DIM: Color32 = Color32::from_rgb(178, 179, 184);
pub const FAINT: Color32 = Color32::from_rgb(138, 139, 145);   // 5.6:1 on CARD
pub const OFF: Color32 = Color32::from_rgb(70, 71, 77);
pub const GOOD: Color32 = Color32::from_rgb(76, 183, 130);
pub const BAD: Color32 = Color32::from_rgb(235, 87, 87);

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
    // pixel art stays sharp only on whole screen pixels: snap the size and corner to them
    let ppp = ui.ctx().pixels_per_point();
    let px = (px * ppp).round().max(1.0) / ppp;
    let (rect, _) = ui.allocate_exact_size(vec2(rows[0].len() as f32 * px, rows.len() as f32 * px), Sense::hover());
    let origin = (rect.min.to_vec2() * ppp).round() / ppp;
    for (y, row) in rows.iter().enumerate() {
        for (x, k) in row.chars().enumerate() {
            let Some(&color) = colors.get(&k) else { continue };
            let min = origin.to_pos2() + vec2(x as f32 * px, y as f32 * px);
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
    let st = Stroke::new(1.4, color);
    match kind {
        Icon::Grid => {
            let q = s * 0.36;
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let center = c + vec2(dx * (q / 2.0 + 1.2), dy * (q / 2.0 + 1.2));
                p.rect_stroke(egui::Rect::from_center_size(center, vec2(q, q)), 1.5, st, egui::StrokeKind::Middle);
            }
        }
        Icon::People => {
            p.circle_stroke(c + vec2(0.0, -s * 0.18), s * 0.18, st);
            let arc: Vec<Pos2> = (0..=16).map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                c + vec2(a.cos() * s * 0.36, s * 0.42 + a.sin() * s * 0.32)
            }).collect();
            p.add(egui::Shape::line(arc, st));
        }
        Icon::Monitor => {
            let r = egui::Rect::from_center_size(c + vec2(0.0, -s * 0.08), vec2(s * 0.9, s * 0.6));
            p.rect_stroke(r, 2.0, st, egui::StrokeKind::Middle);
            p.line_segment([c + vec2(-s * 0.2, s * 0.4), c + vec2(s * 0.2, s * 0.4)], st);
        }
        Icon::Bolt => {
            let pts = [(0.1, -0.45), (-0.26, 0.06), (0.0, 0.06), (-0.1, 0.45), (0.26, -0.08), (0.0, -0.08)]
                .map(|(x, y)| c + vec2(x * s, y * s));
            p.add(egui::Shape::closed_line(pts.to_vec(), st));
        }
        Icon::Sliders => {
            for (i, k) in [0.3_f32, 0.72, 0.42].into_iter().enumerate() {
                let y = c.y + (i as f32 - 1.0) * s * 0.3;
                p.line_segment([egui::pos2(c.x - s * 0.42, y), egui::pos2(c.x + s * 0.42, y)], st);
                p.circle_filled(egui::pos2(c.x - s * 0.42 + k * s * 0.84, y), 2.4, color);
            }
        }
    }
}

/// A full-width sidebar row that lights up on hover and stays lit while chosen. The whole row is the target
/// (Fitts's law). `draw` paints its contents into the row's rect with the ink color to use.
fn side_row(ui: &mut Ui, name: &str, picked: bool, draw: impl FnOnce(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.1));
    let p = ui.painter();
    if picked {
        p.rect_filled(rect, 6.0, CARD_HI);
    } else if hover > 0.0 {
        p.rect_filled(rect, 6.0, CARD.gamma_multiply(hover));
    }
    draw(p, rect, if picked { TEXT } else { DIM.lerp_to_gamma(TEXT, hover * 0.6) });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, picked, name));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A page in the sidebar: icon and name, and a red dot when something there needs you.
pub fn nav_item(ui: &mut Ui, kind: Icon, name: &str, picked: bool, alert: bool) -> egui::Response {
    side_row(ui, name, picked, |p, rect, ink| {
        icon(p, kind, egui::pos2(rect.left() + 20.0, rect.center().y), 15.0, ink);
        p.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.5), ink);
        if alert {
            p.circle_filled(egui::pos2(rect.right() - 14.0, rect.center().y), 3.5, BAD);
        }
    })
}

/// An account in the sidebar: its avatar (green dot while Roblox runs) and name.
pub fn nav_account(ui: &mut Ui, pic: Option<&egui::TextureHandle>, name: &str, live: bool, picked: bool) -> egui::Response {
    side_row(ui, name, picked, |p, rect, ink| {
        let c = egui::pos2(rect.left() + 20.0, rect.center().y);
        paint_avatar(p, pic, name, c, 18.0, live, if picked { CARD_HI } else { CANVAS });
        let text = p.layout(name.to_string(), egui::FontId::proportional(13.5), ink, rect.width() - 50.0);
        p.galley(egui::pos2(rect.left() + 38.0, rect.center().y - text.size().y / 2.0), text, ink);
    })
}

/// A group of settings: its title (and a short note) above a box with thin lines between its rows.
pub fn card<R>(ui: &mut Ui, title: &str, note: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    if !title.is_empty() {
        ui.horizontal(|ui| {
            ui.label(semi(title, 14.0).color(TEXT));
            ui.add_space(4.0);
            ui.add(egui::Label::new(small(note)).truncate());
        });
        ui.add_space(4.0);
    }
    let r = Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        body(ui)
    });
    ui.add_space(22.0);
    r.inner
}

fn divider(ui: &Ui, rect: egui::Rect, last: bool) {
    if !last {
        ui.painter().hline(rect.x_range().shrink(16.0), rect.bottom(), Stroke::new(1.0, LINE));
    }
}

/// One setting: label and note on the left, its control on the right, a line under it unless it's `last`.
/// The control gets the label's id for `labelled_by`, so screen readers name it.
pub fn row(ui: &mut Ui, label: &str, note: &str, last: bool, control: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(16, 11)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            let id = ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 200.0).max(160.0));   // the note wraps instead of running under the control
                let id = ui.label(RichText::new(label).size(13.5).color(TEXT)).id;
                if !note.is_empty() {
                    ui.label(small(note));
                }
                id
            }).inner;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| control(ui, id));
        });
    });
    divider(ui, r.response.rect, last);
}

/// A row with more controls than fit beside its text: label and note on top, the controls in a line under them.
pub fn stack_row(ui: &mut Ui, label: &str, note: &str, last: bool, controls: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(16, 11)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0;
        let id = ui.label(RichText::new(label).size(13.5).color(TEXT)).id;
        ui.label(small(note));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| controls(ui, id));
    });
    divider(ui, r.response.rect, last);
}

/// A whole row you can click (an account in a list): it lights up on hover. Returns the click.
pub fn click_row(ui: &mut Ui, id: egui::Id, name: &str, last: bool, body: impl FnOnce(&mut Ui)) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);   // drawn under the row once its size is known
    let r = Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    let rect = r.response.rect;
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.1));
    ui.painter().set(bg, egui::Shape::rect_filled(rect.shrink(1.0), 9.0, CARD_HI.gamma_multiply(hover)));
    divider(ui, rect, last);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A setting with an on/off switch where the whole row is the button (Fitts's law: a big target). Returns
/// true when it flipped.
pub fn switch_row(ui: &mut Ui, label: &str, note: &str, last: bool, on: &mut bool) -> bool {
    let bg = ui.painter().add(egui::Shape::Noop);
    let mut knob = egui::Rect::NOTHING;
    let r = Frame::new().inner_margin(Margin::symmetric(16, 11)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 70.0).max(160.0));
                ui.label(RichText::new(label).size(13.5).color(TEXT));
                if !note.is_empty() {
                    ui.label(small(note));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                knob = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::hover()).0;
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
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.1));
    ui.painter().set(bg, egui::Shape::rect_filled(rect.shrink(1.0), 9.0, CARD_HI.gamma_multiply(hover * 0.7)));
    paint_switch(ui, knob, *on, resp.is_pointer_button_down_on(), resp.id);
    divider(ui, rect, last);
    flipped
}

/// The switch: the knob shrinks a touch when pressed (anticipation), stretches as it travels (squash and
/// stretch), overshoots and settles (follow through), all eased in and out.
fn paint_switch(ui: &Ui, rect: egui::Rect, on: bool, pressed: bool, id: egui::Id) {
    let ctx = ui.ctx();
    let t = ctx.animate_bool_with_time_and_easing(id.with("knob"), on, dur(0.22), egui::emath::easing::back_out);
    let press = ctx.animate_bool_with_time(id.with("press"), pressed, dur(0.08));
    let fill = ctx.animate_bool_with_time(id.with("fill"), on, dur(0.15));
    let r = rect.height() / 2.0;
    let p = ui.painter();
    p.rect_filled(rect, r, LINE_HI.lerp_to_gamma(TEXT, fill));
    let travel = (t.clamp(0.0, 1.0) * std::f32::consts::PI).sin();   // biggest mid-way
    let kr = (r - 3.0) * (1.0 - 0.15 * press);
    let (w, h) = (kr * (1.0 + 0.4 * travel), kr * (1.0 - 0.1 * travel));
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    let knob = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(w * 2.0, h * 2.0));
    p.rect_filled(knob, h, DIM.lerp_to_gamma(CANVAS, fill));
}

/// A few choices side by side; the chosen one sits on a lighter block that slides to the new choice.
/// Returns true when the choice changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, label: egui::Id, value: &mut T, options: &[(T, &str)]) -> bool {
    let font = egui::FontId::proportional(13.0);
    let widths: Vec<f32> = options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 22.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 4.0, 30.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 7.0, CANVAS);
    p.rect_stroke(rect, 7.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    let mut changed = false;
    let mut picked_at = None;
    let mut cells = Vec::new();
    for ((v, l), w) in options.iter().zip(&widths) {
        let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
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
        let id = label.with("seg-pick");
        let left = ui.ctx().animate_value_with_time(id.with("l"), cell.left(), dur(0.18));
        let width = ui.ctx().animate_value_with_time(id.with("w"), cell.width(), dur(0.18));
        let r = egui::Rect::from_min_size(egui::pos2(left, cell.top()), vec2(width, cell.height()));
        p.rect_filled(r, 5.0, CARD_HI);
        p.rect_stroke(r, 5.0, Stroke::new(1.0, LINE_HI), egui::StrokeKind::Inside);
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
    let font = egui::FontId::new(13.0, FontFamily::Name("semi".into()));
    let ink = if !ui.is_enabled() { OFF } else if primary { CANVAS } else { TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, ink);
    let size = vec2((galley.size().x + 24.0 + if busy { 20.0 } else { 0.0 }).max(64.0), 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let ctx = ui.ctx();
    let press = ctx.animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), dur(0.07));
    let hover = ctx.animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.1));
    let rect = egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    let p = ui.painter();
    if primary && ui.is_enabled() {
        p.rect_filled(rect, 7.0, TEXT.lerp_to_gamma(Color32::from_gray(255), hover));
    } else {
        p.rect_filled(rect, 7.0, CARD_HI.lerp_to_gamma(Color32::from_rgb(38, 39, 43), hover));
        p.rect_stroke(rect, 7.0, Stroke::new(1.0, LINE_HI.lerp_to_gamma(Color32::from_gray(80), hover)), egui::StrokeKind::Inside);
    }
    let mut x = rect.center().x - galley.size().x / 2.0;
    if busy {
        x += 10.0;
        egui::Spinner::new().size(13.0).color(DIM).paint_at(ui, egui::Rect::from_center_size(egui::pos2(x - 13.0, rect.center().y), vec2(13.0, 13.0)));
    }
    p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A text-only button, for "Later" and "All Accounts" links.
pub fn link(ui: &mut Ui, text: &str) -> egui::Response {
    let r = ui.add(egui::Label::new(RichText::new(text).size(13.0).color(DIM)).sense(Sense::click()));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A dot and a word, like "● Running": green when on, grey when off. No box around it.
pub fn status(ui: &mut Ui, text: &str, on: bool) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(7.0, 14.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.5, if on { GOOD } else { OFF });
        ui.label(RichText::new(text).size(12.5).color(if on { DIM } else { FAINT }));
    });
}

/// A round avatar: the Roblox headshot when there is one, else the name's first letter. A green dot at the
/// bottom right while Roblox is running; `ring` is the color behind it, so the dot cuts out cleanly.
fn paint_avatar(p: &egui::Painter, pic: Option<&egui::TextureHandle>, name: &str, c: Pos2, size: f32, live: bool, ring: Color32) {
    let rect = egui::Rect::from_center_size(c, vec2(size, size));
    p.circle_filled(c, size / 2.0, Color32::from_rgb(42, 43, 48));
    match pic {
        Some(t) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            p.add(egui::epaint::RectShape::filled(rect, size / 2.0, Color32::WHITE).with_texture(t.id(), uv));
        }
        None => {
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            p.text(c, egui::Align2::CENTER_CENTER, letter, egui::FontId::new(size * 0.45, FontFamily::Name("semi".into())), DIM);
        }
    }
    if live {
        let d = c + vec2(size * 0.36, size * 0.36);
        p.circle_filled(d, (size * 0.13).max(3.0) + 2.0, ring);
        p.circle_filled(d, (size * 0.13).max(3.0), GOOD);
    }
}

pub fn avatar(ui: &mut Ui, pic: Option<&egui::TextureHandle>, name: &str, size: f32, live: bool, ring: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_avatar(ui.painter(), pic, name, rect.center(), size, live, ring);
}

/// A thin bar for how full something is: white, red past 85%.
pub fn bar(ui: &mut Ui, width: f32, share: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, LINE_HI);
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * share.clamp(0.0, 1.0), 4.0)), 2.0, if share > 0.85 { BAD } else { TEXT });
}

/// One slim strip of numbers split by thin lines: a small label, the number, a short line under it.
pub fn stats(ui: &mut Ui, items: &[(&str, String, String)]) {
    Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(10)).show(ui, |ui| {
        let w = ui.available_width() / items.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, (label, value, note)) in items.iter().enumerate() {
                let r = Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width(w - 32.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(*label).size(12.0).color(FAINT));
                    ui.label(semi(value, 22.0).color(TEXT));
                    ui.add(egui::Label::new(small(note.as_str())).truncate());
                }));
                if i > 0 {
                    let rect = r.response.rect;
                    ui.painter().vline(rect.left(), rect.y_range().shrink(12.0), Stroke::new(1.0, LINE));
                }
            }
        });
    });
    ui.add_space(22.0);
}

/// Lines of fractions (0 to 1) over time, oldest left. The first line gets a faint fill under it.
pub fn chart(ui: &mut Ui, size: egui::Vec2, lines: &[(&[f32], Color32)]) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    for k in 0..=4 {
        let y = rect.bottom() - rect.height() * k as f32 / 4.0;
        p.hline(rect.x_range(), y, Stroke::new(1.0, LINE.gamma_multiply(if k == 0 { 1.0 } else { 0.6 })));
        p.text(egui::pos2(rect.right() - 2.0, y - 3.0), egui::Align2::RIGHT_BOTTOM, format!("{}%", k * 25), egui::FontId::proportional(10.5), OFF);
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
                mesh.colored_vertex(*pt, color.gamma_multiply(0.1));
                mesh.colored_vertex(egui::pos2(pt.x, rect.bottom()), color.gamma_multiply(0.0));
                if j > 0 {
                    let b = (j as u32 - 1) * 2;
                    mesh.add_triangle(b, b + 1, b + 2);
                    mesh.add_triangle(b + 1, b + 2, b + 3);
                }
            }
            p.add(mesh);
        }
        p.add(egui::Shape::line(pts.clone(), Stroke::new(1.6, *color)));
        if let Some(last) = pts.last() {
            p.circle_filled(*last, 3.0, *color);
        }
    }
}

/// A short line in a legend: the line's color, then its name.
pub fn legend(ui: &mut Ui, color: Color32, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(2.0, color));
    ui.label(small(text));
}

/// Two lines of text in the middle of an empty area.
pub fn centered_note(ui: &Ui, rect: egui::Rect, title: &str, note: &str) {
    let p = ui.painter();
    p.text(rect.center() - vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, title, egui::FontId::new(13.5, FontFamily::Name("semi".into())), DIM);
    p.text(rect.center() + vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, note, egui::FontId::proportional(12.0), FAINT);
}

/// A filled green circle with a tick (done) or an empty ring (not yet), for checklists.
pub fn check(ui: &mut Ui, done: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    if done {
        p.circle_filled(c, 8.0, GOOD);
        p.add(egui::Shape::line(vec![c + vec2(-3.6, 0.2), c + vec2(-1.0, 2.8), c + vec2(3.8, -2.6)], Stroke::new(1.8, CANVAS)));
    } else {
        p.circle_stroke(c, 7.5, Stroke::new(1.4, LINE_HI));
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
    // egui's dark-theme edge shading fattens dark text on the white buttons, and hinting snaps letters to whole
    // pixels, which looks jagged at 100% scaling: a middle gamma and no hinting keep every text smooth
    v.text_options.color_transfer_function = egui::epaint::FontColorTransferFunction::Gamma(0.8);
    v.text_options.font_hinting = false;
    v.panel_fill = CANVAS;
    v.window_fill = CARD_HI;
    v.window_stroke = Stroke::new(1.0, LINE_HI);
    v.override_text_color = Some(TEXT);
    v.extreme_bg_color = CANVAS;   // text boxes
    v.selection.bg_fill = Color32::from_rgb(64, 65, 72);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(7);
    }
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE_HI);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(38, 39, 43);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(80));
    v.widgets.open.weak_bg_fill = Color32::from_rgb(38, 39, 43);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.5));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    });
}
