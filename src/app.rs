//! The window: a sidebar on the left (the pages, then every account), and the page in a rounded panel with a
//! slim top bar holding the PC's live numbers and the actions for every Roblox. The look lives in widgets.rs;
//! all the work happens in engine.rs. Closing the window hides it to the tray.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, vec2};

use crate::config::{Preset, Priority};
use crate::engine::{self, Engine, State, UpdateState, gb};
use crate::power::Item;
use crate::preview::Kind;
use crate::rdp::Patch;
use crate::widgets::*;

pub fn run() -> eframe::Result {
    crate::system::enable_privileges();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Potato")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default())
            .with_inner_size([1180.0, 800.0]).with_min_inner_size([900.0, 600.0]),
        centered: true,   // Windows otherwise opens it wherever it last cascaded windows, often off to the side
        ..Default::default()
    };
    eframe::run_native("Potato", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[derive(Clone, PartialEq)]
enum Page {
    Overview,
    Accounts,
    Account(String),
    RemoteDesktop,
    Performance,
    Settings,
}

/// The smallest each live picture on the Overview may be at each size, in points: as many fit across as can,
/// then they stretch to fill the row.
const SMALL: u32 = 200;
const MEDIUM: u32 = 280;
const LARGE: u32 = 400;

pub struct App {
    engine: Arc<Mutex<Engine>>,
    page: Page,
    /// Restart Roblox was clicked once and asks for a second click until then
    confirm_restart: Option<Instant>,
    confirm_debloat: Option<Instant>,
    /// the alt Remove was clicked on once, and when
    confirm_remove: Option<(String, Instant)>,
    update_dismissed: bool,
    /// what's typed in each account's FPS box
    fps_text: HashMap<String, String>,
    /// what's typed in the boxes whose value only counts once it's complete (a time, a link)
    shutdown_text: Option<String>,
    webhook_text: Option<String>,
    /// the Remote Desktop page's new alt boxes
    new_name: String,
    new_pass: String,
    /// the action running in the background, if any (so the window never waits on it)
    busy: Arc<Mutex<Option<&'static str>>>,
    /// the last result shown as a toast, and when it appeared (the window's own clock, so animation stays in step)
    toast: Option<(String, f64)>,
    /// Windows' power and update settings as last read (None until the first read is back)
    power: Arc<Mutex<Option<crate::power::State>>>,
    /// the live pictures' size on the Overview (SMALL, MEDIUM or LARGE)
    tile_w: u32,
    avatars: crate::avatars::Avatars,
    /// the window, so the tray can show it again (None off Windows: closing then really closes)
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: Option<isize>,
    /// the live pictures on the Overview, and the windows they show (looked up once a second)
    thumbs: crate::preview::Thumbs,
    wins: Vec<crate::preview::Win>,
    wins_at: Option<Instant>,
    #[cfg(windows)]
    _tray: Option<tray_icon::TrayIcon>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> App {
        MOTION.store(crate::system::animations_on(), Ordering::Relaxed);
        style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let engine = Engine::start(move || ctx.request_repaint());
        let hwnd = window_handle(cc);
        #[cfg(windows)]
        if let Some(h) = hwnd {
            crate::tray::name_taskbar(h);
        }
        let app = App {
            #[cfg(windows)]
            _tray: hwnd.and_then(|h| crate::tray::create(h, cc.egui_ctx.clone(), engine.clone())),
            engine,
            page: Page::Overview,
            confirm_restart: None,
            confirm_debloat: None,
            confirm_remove: None,
            update_dismissed: false,
            fps_text: HashMap::new(),
            shutdown_text: None,
            webhook_text: None,
            new_name: String::new(),
            new_pass: String::new(),
            busy: Arc::default(),
            toast: None,
            power: Arc::default(),
            tile_w: MEDIUM,
            avatars: Default::default(),
            thumbs: crate::preview::Thumbs::new(hwnd),
            wins: Vec::new(),
            wins_at: None,
            hwnd,
        };
        app.read_power(&cc.egui_ctx);
        app
    }

    /// Read Windows' power and update settings on a thread (powercfg takes a moment).
    fn read_power(&self, ctx: &egui::Context) {
        let (power, ctx) = (self.power.clone(), ctx.clone());
        std::thread::spawn(move || {
            *power.lock().unwrap() = Some(crate::power::read());
            ctx.request_repaint();
        });
    }

    /// Change Windows settings in the background, then read them back so the page shows what Windows has.
    fn apply_power(&self, ctx: &egui::Context, items: Vec<(Item, bool)>) {
        let power = self.power.clone();
        self.run_action("power", ctx, move |e| {
            for (item, on) in items {
                match crate::power::apply(item, on) {
                    Err(m) => e.note = m,
                    Ok(()) if item == Item::UpdatesOff => {
                        e.note = if on { "Windows Updates Stopped. Undo It Before Using The Microsoft Store" } else { "Windows Updates Are Back On" }.into();
                    }
                    Ok(()) => {}
                }
            }
            *power.lock().unwrap() = Some(crate::power::read());
        });
    }
}

#[cfg(windows)]
fn window_handle(cc: &eframe::CreationContext) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
fn window_handle(_: &eframe::CreationContext) -> Option<isize> {
    None
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        if let Some(h) = self.hwnd && ui.ctx().input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            crate::tray::hide(h);   // keeps working in the tray; Quit Potato is in the tray menu
        }
        let engine = self.engine.clone();
        let mut e = engine.lock().unwrap();
        let before = e.config.clone();
        let accounts = e.rdp.lock().unwrap().accounts.clone();
        if self.wins_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.wins = crate::preview::windows(&accounts);
            self.wins_at = Some(Instant::now());
        }
        self.avatars.load(ui.ctx(), e.profiles.iter().map(|p| (p.name.clone(), p.local.clone())));
        egui::Panel::left("side").exact_size(224.0).resizable(false).frame(Frame::new().fill(CANVAS).inner_margin(Margin { left: 10, right: 10, top: 16, bottom: 12 }))
            .show(ui, |ui| self.sidebar(ui, &e));
        egui::CentralPanel::default().frame(Frame::new().fill(CANVAS).inner_margin(Margin { left: 0, right: 8, top: 8, bottom: 8 })).show(ui, |ui| {
            Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(12)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing.y = 0.0;
                self.top_bar(ui, &mut e);
                let line = ui.available_rect_before_wrap();
                ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, LINE));
                egui::ScrollArea::vertical().id_salt("page").show(ui, |ui| {
                    Frame::new().inner_margin(Margin { left: 28, right: 28, top: 22, bottom: 24 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        // settings don't stretch across a wide screen; the live pictures may
                        let wide = matches!(self.page, Page::Overview | Page::Accounts);
                        ui.set_width(ui.available_width().min(if wide { 1600.0 } else { 1040.0 }));
                        self.banner(ui, &e);
                        match self.page.clone() {
                            Page::Overview => self.overview_page(ui, &mut e),
                            Page::Accounts => self.accounts_page(ui, &e),
                            Page::Account(sid) => self.account_page(ui, &mut e, &sid),
                            Page::RemoteDesktop => self.rdp_page(ui, &mut e),
                            Page::Performance => self.performance_page(ui, &mut e),
                            Page::Settings => self.settings_page(ui, &mut e),
                        }
                    });
                });
            });
        });
        let rdp_msg = e.rdp.lock().unwrap().msg.take();
        if let Some(msg) = rdp_msg {
            e.note = msg;
        }
        if !e.note.is_empty() {
            self.toast = Some((std::mem::take(&mut e.note), ui.ctx().input(|i| i.time)));
        }
        self.thumbs.end_frame();   // pictures not drawn this frame (another page, scrolled away) go
        self.draw_toast(ui.ctx());
        if e.config != before {
            e.changed();
        }
    }
}

/// Two columns side by side on a wide page, one under the other on a narrow one.
fn columns(ui: &mut Ui, body: impl FnOnce(&mut [Ui])) {
    let n = if ui.available_width() >= 820.0 { 2 } else { 1 };
    ui.columns(n, |cols| {
        if n == 2 {
            for c in cols.iter_mut() {
                c.spacing_mut().item_spacing.x = 10.0;
            }
        }
        body(cols)
    });
}

/// The column for the `i`th of two blocks: side by side, or both in the one column.
fn col(cols: &mut [Ui], i: usize) -> &mut Ui {
    let n = cols.len();
    &mut cols[i.min(n - 1)]
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A fixed-width spot in a list row, so every row's columns line up.
fn cell(ui: &mut Ui, w: f32, body: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(w, 36.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        body(ui);
    });
}

impl App {
    fn sidebar(&mut self, ui: &mut Ui, e: &Engine) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            logo(ui, 1.4);
            ui.add_space(2.0);
            ui.label(semi("Potato", 15.0).color(TEXT));
        });
        ui.add_space(18.0);
        let rdp_warn = e.rdp.lock().unwrap().reverted(e.config.rdp_patched);
        let pages = [
            (Icon::Grid, "Overview", Page::Overview, false),
            (Icon::People, "Accounts", Page::Accounts, false),
            (Icon::Monitor, "RDPs", Page::RemoteDesktop, rdp_warn),
            (Icon::Bolt, "Performance", Page::Performance, false),
            (Icon::Sliders, "Settings", Page::Settings, false),
        ];
        for (kind, name, page, alert) in pages {
            if nav_item(ui, kind, name, self.page == page, alert).clicked() {
                self.page = page;
            }
        }
        if !e.profiles.is_empty() {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("Your Accounts").size(12.0).color(FAINT));
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("side-accounts").max_height(ui.available_height() - 30.0).show(ui, |ui| {
                for p in &e.profiles {
                    let live = e.clients.iter().any(|c| c.sid == p.sid);
                    let pic = self.avatars.texture(ui.ctx(), &p.name);
                    if nav_account(ui, pic.as_ref(), &p.name, live, self.page == Page::Account(p.sid.clone())).clicked() {
                        self.page = Page::Account(p.sid.clone());
                    }
                }
            });
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Potato {}", env!("CARGO_PKG_VERSION"))).size(11.5).color(OFF));
            });
        });
    }

    /// The page's name, the PC's live numbers, and the actions for every Roblox.
    fn top_bar(&mut self, ui: &mut Ui, e: &mut Engine) {
        Frame::new().inner_margin(Margin { left: 24, right: 16, top: 10, bottom: 10 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                match &self.page.clone() {
                    Page::Account(sid) => {
                        if link(ui, "Accounts").clicked() {
                            self.page = Page::Accounts;
                        }
                        ui.label(RichText::new("›").size(14.0).color(OFF));
                        let name = e.profiles.iter().find(|p| &p.sid == sid).map_or("Account".to_string(), |p| p.name.clone());
                        ui.label(semi(&name, 14.0).color(TEXT));
                    }
                    page => {
                        let title = match page {
                            Page::Overview => "Overview",
                            Page::Accounts | Page::Account(_) => "Accounts",
                            Page::RemoteDesktop => "RDPs",
                            Page::Performance => "Performance",
                            Page::Settings => "Settings",
                        };
                        ui.label(semi(title, 14.0).color(TEXT));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let busy = *self.busy.lock().unwrap();
                    if action(ui, "Free Roblox RAM", busy == Some("trim")).on_hover_text("Frees the RAM every Roblox isn't using right now").clicked() {
                        self.run_action("trim", ui.ctx(), |e| e.trim_all(true));
                    }
                    let armed = self.confirm_restart.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                    if action(ui, if armed { "Click Again To Restart" } else { "Restart Roblox" }, busy == Some("restart"))
                        .on_hover_text("Closes every Roblox and opens each one again in the same server").clicked() {
                        if armed {
                            self.confirm_restart = None;
                            self.run_action("restart", ui.ctx(), |e| e.restart_clients());
                        } else {
                            self.confirm_restart = Some(Instant::now());
                        }
                    }
                    if armed {
                        ui.ctx().request_repaint_after(Duration::from_millis(250));
                    }
                    if action(ui, "Free PC RAM", busy == Some("clean")).on_hover_text("Clears files Windows keeps in RAM just in case").clicked() {
                        self.run_action("clean", ui.ctx(), |e| e.clean(true));
                    }
                    if ui.available_width() < 330.0 {
                        return;   // a narrow window: the buttons matter more than the numbers
                    }
                    ui.add_space(16.0);
                    let cpu = (e.cpu_pct.values().sum::<f32>().clamp(0.0, 100.0) + 0.0).round();
                    meter(ui, "Roblox CPU", &format!("{cpu:.0}%"), cpu / 100.0);
                    ui.add_space(12.0);
                    let (used, total) = e.memory;
                    // counts to the new value instead of jumping (follow through)
                    let shown = ui.ctx().animate_value_with_time(egui::Id::unique("ram-used"), used as f32, dur(0.6));
                    meter(ui, "RAM", &format!("{} / {}", gb(shown as u64), gb(total)), if total > 0 { used as f32 / total as f32 } else { 0.0 });
                });
            });
        });
    }

    /// Results float up at the bottom and fade away (staging, exaggerated feedback).
    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, at)) = &self.toast else { return };
        let age = (ctx.input(|i| i.time) - at) as f32;
        let (show, life) = (4.0, dur(0.22).max(0.001));
        if age > show + life {
            self.toast = None;
            return;
        }
        let come = egui::emath::easing::back_out((age / life).min(1.0));
        let go = 1.0 - ((age - show) / life).clamp(0.0, 1.0);
        egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, vec2(112.0, -24.0 + (1.0 - come) * 18.0))
            .interactable(false).show(ctx, |ui| {
                ui.set_opacity(come.min(1.0) * go);
                Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0, LINE_HI)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(16, 10))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(140) }).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 3.5, GOOD);
                            ui.add(egui::Label::new(RichText::new(text.as_str()).size(13.0).color(TEXT)).extend());   // one line, never squeezed by the window edge
                        });
                    });
            });
        ctx.request_repaint();
    }

    /// Run an action on its own thread, so the window answers at once (Doherty threshold).
    fn run_action(&self, name: &'static str, ctx: &egui::Context, work: impl FnOnce(&mut Engine) + Send + 'static) {
        let mut busy = self.busy.lock().unwrap();
        if busy.is_some() {
            return;
        }
        *busy = Some(name);
        let (engine, flag, ctx) = (self.engine.clone(), self.busy.clone(), ctx.clone());
        std::thread::spawn(move || {
            work(&mut engine.lock().unwrap());
            *flag.lock().unwrap() = None;
            ctx.request_repaint();
        });
    }

    /// A message across the top of the page, with what to do about it.
    fn notice(ui: &mut Ui, warn: bool, title: &str, note: &str, buttons: impl FnOnce(&mut Ui)) {
        Frame::new().fill(if warn { Color32::from_rgb(38, 18, 19) } else { CARD }).stroke(Stroke::new(1.0, if warn { BAD.gamma_multiply(0.6) } else { LINE_HI }))
            .corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 240.0).max(200.0));
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(semi(title, 13.5).color(TEXT));
                        ui.label(RichText::new(note).size(12.5).color(DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
                });
            });
        ui.add_space(18.0);
    }

    fn banner(&mut self, ui: &mut Ui, e: &Engine) {
        self.rdp_banner(ui, e);
        let state = e.update.lock().unwrap().clone();
        let (title, note) = match &state {
            UpdateState::None => return,
            UpdateState::Available(_) if self.update_dismissed => return,
            UpdateState::Available(r) => (format!("Potato {} Is Out", r.tag), "Takes a few seconds and restarts by itself.".to_string()),
            UpdateState::Installing(tag) => (format!("Installing Potato {tag}"), "Downloading and checking it.".to_string()),
            UpdateState::Failed(m) => ("The Update Didn't Install".to_string(), m.clone()),
        };
        Self::notice(ui, false, &title, &note, |ui| match state {
            UpdateState::Available(r) => {
                if primary(ui, "Update Now").clicked() {
                    engine::install_update(r, e.update.clone());
                }
                if link(ui, "Later").clicked() {
                    self.update_dismissed = true;
                }
            }
            UpdateState::Installing(_) => { ui.spinner(); }
            UpdateState::Failed(_) => {
                if action(ui, "Close", false).clicked() {
                    *e.update.lock().unwrap() = UpdateState::None;
                }
            }
            UpdateState::None => {}
        });
    }

    /// Windows undid Multiple RDPs (an update put its own file back): say so on
    /// every page, with the fix.
    fn rdp_banner(&mut self, ui: &mut Ui, e: &Engine) {
        let r = e.rdp.lock().unwrap().clone();
        if !r.reverted(e.config.rdp_patched) {
            return;
        }
        let fits = r.patch == Patch::Unpatched;
        let note = if fits {
            "Only one RDP can be signed in until you turn it back on. Stop Windows Updates so it doesn't happen again."
        } else {
            "Potato can't turn it back on for this Windows version yet, so only one RDP can be signed in."
        };
        let page = self.page.clone();
        let mut go = false;
        Self::notice(ui, true, "A Windows Update Turned Off Multiple RDPs", note, |ui| {
            if fits {
                if button(ui, if r.busy == Some("patch") { "Turning On" } else { "Turn It Back On" }, r.busy == Some("patch"), true).clicked() {
                    engine::refresh_rdp(e.rdp.clone(), Some("patch"), crate::rdp::patch);
                }
            } else if page != Page::RemoteDesktop && action(ui, "Go To RDPs", false).clicked() {
                go = true;
            }
        });
        if go {
            self.page = Page::RemoteDesktop;
        }
    }
}

/// A live number in the top bar: its name and value on one line, a thin bar under for how full it is.
fn meter(ui: &mut Ui, label: &str, value: &str, share: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(124.0, 30.0), Sense::hover());
    let p = ui.painter();
    let name = p.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), FAINT);
    let x = rect.left() + name.size().x + 6.0;
    p.galley(egui::pos2(rect.left(), rect.top() + 4.0), name, FAINT);
    p.text(egui::pos2(x, rect.top() + 4.0), egui::Align2::LEFT_TOP, value, egui::FontId::proportional(12.0), TEXT);
    let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.bottom() - 6.0), vec2(rect.width(), 4.0));
    p.rect_filled(bar, 2.0, LINE_HI);
    p.rect_filled(egui::Rect::from_min_size(bar.min, vec2(bar.width() * share.clamp(0.0, 1.0), 4.0)), 2.0, if share > 0.85 { BAD } else { TEXT });
}

/// One live tile: who it is, the window to picture (None: a Remote Desktop that isn't open), and its Roblox.
struct Tile {
    name: String,
    win: Option<crate::preview::Win>,
    /// the Windows user whose Roblox it is (for its RAM, CPU and Free RAM)
    user: Option<String>,
    rdp: Option<String>,
}

impl App {
    /// The PC's numbers, RAM over time, then a live picture of every Remote Desktop and Roblox window.
    fn overview_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let accounts = e.rdp.lock().unwrap().accounts.clone();
        let (used, total) = e.memory;
        let roblox: u64 = e.clients.iter().map(|c| c.ram).sum();
        let cpu = (e.cpu_pct.values().sum::<f32>().clamp(0.0, 100.0) + 0.0).round();
        let rdps_open = self.wins.iter().filter(|w| matches!(w.kind, Kind::Rdp(_))).count();
        stats(ui, &[
            ("RAM In Use", gb(used), format!("of {}", gb(total))),
            ("Roblox Is Using", gb(roblox), format!("across {}", plural(e.clients.len(), "Roblox", "Roblox"))),
            ("Freed By Potato", gb(e.freed), "since Potato started".into()),
            ("Roblox CPU", format!("{cpu:.0}%"), "of the whole PC".into()),
            ("RDPs Open", rdps_open.to_string(), format!("of {}", plural(accounts.len(), "alt", "alts"))),
        ]);
        // a tile per Remote Desktop alt (open or not), each other Remote Desktop window, then each Roblox window here
        let mut tiles: Vec<Tile> = accounts.iter().map(|n| Tile {
            name: n.clone(),
            win: self.wins.iter().find(|w| w.kind == Kind::Rdp(n.clone())).cloned(),
            user: Some(n.clone()), rdp: Some(n.clone()),
        }).collect();
        for w in &self.wins {
            if let Kind::Rdp(label) = &w.kind && !accounts.contains(label) {
                tiles.push(Tile { name: label.clone(), win: Some(w.clone()), user: Some(label.clone()), rdp: None });
            }
        }
        for w in self.wins.iter().filter(|w| w.kind == Kind::Roblox) {
            let user = e.clients.iter().find(|c| c.pid == w.pid).map(|c| c.user.clone());
            tiles.push(Tile { name: user.clone().unwrap_or("Roblox".into()), win: Some(w.clone()), user, rdp: None });
        }
        ui.horizontal(|ui| {
            ui.label(semi("Live", 14.0).color(TEXT));
            ui.add_space(4.0);
            ui.label(small("Click a picture to go to that window"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let id = ui.scope_id().with("tile-size");
                segmented(ui, id, &mut self.tile_w, &[(SMALL, "Small"), (MEDIUM, "Medium"), (LARGE, "Large")]);
            });
        });
        ui.add_space(6.0);
        if tiles.is_empty() {
            card(ui, "", "", |ui| {
                Frame::new().inner_margin(Margin::same(28)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(semi("Nothing Open Yet", 14.0).color(TEXT));
                        ui.label(small("Open Roblox, or add an alt on the RDPs page, and it shows up here live."));
                    });
                });
            });
            ui.add_space(12.0);
            self.ram_card(ui, e);
            return;
        }
        // pictures fill the row; each size has fewer across than the one below it, so all three always differ
        let (gap, avail) = (10.0, ui.available_width());
        let fit = |min: u32| (((avail + gap) / (min as f32 + gap)) as usize).max(1);
        let small = fit(SMALL);
        let medium = fit(MEDIUM).min(small - 1).max(1);
        let cols = match self.tile_w {
            SMALL => small,
            MEDIUM => medium,
            _ => fit(LARGE).min(medium - 1).max(1),
        };
        let w = ((avail - gap * (cols - 1) as f32) / cols as f32).floor();
        let mut act: Option<(&'static str, String)> = None;
        for row in tiles.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for t in row {
                    if let Some(a) = self.tile(ui, e, t, w) {
                        act = Some(a);
                    }
                }
            });
            ui.add_space(gap);
        }
        match act {
            Some(("open", n)) => {
                if let Err(m) = crate::rdp::open(&n, e.config.rdp_wide.contains(&n)) {
                    e.note = m;
                }
            }
            Some(("trim", user)) => self.run_action("trim", ui.ctx(), move |e| e.trim_user(&user)),
            _ => {}
        }
        ui.add_space(12.0);
        self.ram_card(ui, e);
    }

    /// RAM over the last while: the whole PC and Roblox.
    fn ram_card(&self, ui: &mut Ui, e: &Engine) {
        let (used, total) = e.memory;
        let roblox: u64 = e.clients.iter().map(|c| c.ram).sum();
        let t = total.max(1) as f32;
        let pc: Vec<f32> = e.history.iter().map(|s| s.pc_used as f32 / t).chain([used as f32 / t]).collect();
        let rb: Vec<f32> = e.history.iter().map(|s| s.roblox as f32 / t).chain([roblox as f32 / t]).collect();
        let span = e.history.first().map_or(0, |s| e.history.last().map_or(0, |l| l.at - s.at));
        let note = if pc.len() < 3 { "Fills in while Potato runs".to_string() } else { format!("Last {} min", span / 60 + 1) };
        card(ui, "RAM Over Time", &note, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    legend(ui, FAINT, "Whole PC");
                    ui.add_space(10.0);
                    legend(ui, TEXT, "Roblox");
                });
                ui.add_space(6.0);
                chart(ui, vec2(ui.available_width(), 130.0), &[(&pc, FAINT), (&rb, TEXT)]);
            });
        });
    }

    /// One live tile. Returns what was clicked: ("open", alt) or ("trim", user).
    fn tile(&mut self, ui: &mut Ui, e: &Engine, t: &Tile, w: f32) -> Option<(&'static str, String)> {
        let mut act = None;
        let pids: Vec<u32> = t.user.as_ref().map(|u| e.clients.iter().filter(|c| c.user.eq_ignore_ascii_case(u)).map(|c| c.pid).collect()).unwrap_or_default();
        let ram: u64 = e.clients.iter().filter(|c| pids.contains(&c.pid)).map(|c| c.ram).sum();
        let cpu: f32 = pids.iter().filter_map(|p| e.cpu_pct.get(p)).sum::<f32>().max(0.0) + 0.0;
        let pic = self.avatars.texture(ui.ctx(), &t.name);
        Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(6)).show(ui, |ui| ui.vertical(|ui| {
            let inner = w - 14.0;
            ui.set_width(inner);
            let (rect, resp) = ui.allocate_exact_size(vec2(inner, inner * 9.0 / 16.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, format!("Picture Of {}", t.name)));
            ui.painter().rect_filled(rect, 6.0, CANVAS);
            match &t.win {
                Some(win) if !win.minimized => {
                    let ppp = ui.ctx().pixels_per_point();
                    let px = |r: egui::Rect| [(r.left() * ppp) as i32, (r.top() * ppp) as i32, (r.right() * ppp) as i32, (r.bottom() * ppp) as i32];
                    if !self.thumbs.show(win.hwnd, px(rect.shrink(1.0)), px(ui.clip_rect())) {
                        centered_note(ui, rect, "Live Picture", "Shows here on Windows");
                    }
                }
                Some(_) => centered_note(ui, rect, "Minimized", "Click to show it"),
                None if t.rdp.is_some() => centered_note(ui, rect, "Not Open", "Click to open it"),
                None => centered_note(ui, rect, "Not Open", ""),
            }
            if t.win.is_some() || t.rdp.is_some() {
                let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.1));
                ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, LINE.lerp_to_gamma(TEXT, hover * 0.6)), egui::StrokeKind::Inside);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    match (&t.win, &t.rdp) {
                        (Some(win), _) => crate::preview::focus(win.hwnd),
                        (None, Some(n)) => act = Some(("open", n.clone())),
                        _ => {}
                    }
                }
            }
            let narrow = inner < 250.0;   // Small: the picture is the button, so the name gets the room
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                avatar(ui, pic.as_ref(), &t.name, 28.0, !pids.is_empty(), CARD);
                ui.vertical(|ui| {
                    ui.set_max_width((inner - if narrow { 50.0 } else { 140.0 }).max(60.0));   // room for the button, never under it
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(semi(&t.name, 13.5).color(TEXT)).truncate());
                    let meta = if pids.is_empty() { "Roblox closed".to_string() } else { format!("{} · {cpu:.0}% CPU", gb(ram)) };
                    ui.add(egui::Label::new(small(meta)).truncate());
                });
                if narrow {
                    return;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(n) = &t.rdp && t.win.is_none() {
                        if primary(ui, "Open").clicked() {
                            act = Some(("open", n.clone()));
                        }
                    } else if let Some(u) = &t.user && !pids.is_empty() && action(ui, "Free RAM", false).on_hover_text("Frees the RAM this Roblox isn't using right now").clicked() {
                        act = Some(("trim", u.clone()));
                    }
                });
            });
            ui.add_space(4.0);
        }));
        act
    }

    /// Every Windows account with Roblox, one row each: who's playing, what they use, how they're set up.
    fn accounts_page(&mut self, ui: &mut Ui, e: &Engine) {
        if e.profiles.is_empty() {
            card(ui, "", "", |ui| {
                Frame::new().inner_margin(Margin::same(28)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(semi("No Account With Roblox Yet", 14.0).color(TEXT));
                        ui.label(small("Install Roblox on a Windows account and it shows up here."));
                    });
                });
            });
            return;
        }
        let full = ui.available_width() >= 720.0;
        let fixed = if full { [130.0, 90.0, 70.0, 120.0, 80.0] } else { [120.0, 80.0, 0.0, 0.0, 70.0] };
        let name_w = ui.available_width() - 32.0 - fixed.iter().sum::<f32>() - 30.0;
        let heads = ["Status", "RAM", "CPU", "Setup", "FPS Cap"];
        card(ui, "Accounts", "Every Windows account on this PC with Roblox. Click one to change how Potato runs it", |ui| {
            Frame::new().inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 6 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    cell(ui, name_w, |ui| { ui.label(RichText::new("Account").size(12.0).color(FAINT)); });
                    for (h, w) in heads.iter().zip(fixed) {
                        if w > 0.0 {
                            cell(ui, w, |ui| { ui.label(RichText::new(*h).size(12.0).color(FAINT)); });
                        }
                    }
                });
            });
            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, LINE));
            let n = e.profiles.len();
            for (i, p) in e.profiles.iter().enumerate() {
                let a = e.config.account(&p.sid);
                let clients: Vec<_> = e.clients.iter().filter(|c| c.sid == p.sid).collect();
                let ram: u64 = clients.iter().map(|c| c.ram).sum();
                let cpu: f32 = clients.iter().filter_map(|c| e.cpu_pct.get(&c.pid)).sum::<f32>().max(0.0) + 0.0;
                let pic = self.avatars.texture(ui.ctx(), &p.name);
                let username = self.avatars.username(&p.name);
                let resp = click_row(ui, ui.scope_id().with(("account", &p.sid)), &p.name, i + 1 == n, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        cell(ui, name_w, |ui| {
                            avatar(ui, pic.as_ref(), &p.name, 32.0, !clients.is_empty(), CARD);
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.add(egui::Label::new(semi(&p.name, 13.5).color(TEXT)).truncate());
                                ui.add(egui::Label::new(small(username.as_ref().map_or("Roblox account not found".into(), |u| format!("@{u}")))).truncate());
                            });
                        });
                        let state = if !a.on { "Potato off".to_string() } else if clients.is_empty() { "Closed".into() } else if clients.len() == 1 { "Playing".into() } else { format!("{} open", clients.len()) };
                        cell(ui, fixed[0], |ui| status(ui, &state, a.on && !clients.is_empty()));
                        let num = |ui: &mut Ui, s: String| { ui.label(RichText::new(s).size(13.0).color(if clients.is_empty() { FAINT } else { DIM })); };
                        cell(ui, fixed[1], |ui| num(ui, if clients.is_empty() { "-".into() } else { gb(ram) }));
                        if full {
                            cell(ui, fixed[2], |ui| num(ui, if clients.is_empty() { "-".into() } else { format!("{cpu:.0}%") }));
                            cell(ui, fixed[3], |ui| { ui.label(RichText::new(Preset::of(&a).map_or("Custom", Preset::label)).size(13.0).color(DIM)); });
                        }
                        cell(ui, fixed[4], |ui| { ui.label(RichText::new(a.plan.fps_cap.map_or("None".to_string(), |f| f.to_string())).size(13.0).color(DIM)); });
                        ui.label(RichText::new("›").size(16.0).color(OFF));
                    });
                });
                if resp.clicked() {
                    self.page = Page::Account(p.sid.clone());
                }
            }
        });
    }

    fn account_page(&mut self, ui: &mut Ui, e: &mut Engine, sid: &str) {
        let Some(profile) = e.profiles.iter().find(|p| p.sid == sid).cloned() else {
            self.page = Page::Accounts;
            return;
        };
        let mut a = e.config.account(sid);
        let running = e.clients.iter().filter(|c| c.sid == sid).count();
        let pic = self.avatars.texture(ui.ctx(), &profile.name);
        ui.horizontal(|ui| {
            avatar(ui, pic.as_ref(), &profile.name, 52.0, running > 0, PANEL);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(semi(&profile.name, 20.0).color(TEXT));
                let who = self.avatars.username(&profile.name).map_or("Roblox account not found".into(), |u| format!("Roblox: @{u}"));
                let playing = match running {
                    0 => "Roblox closed".to_string(),
                    1 => "Playing now".to_string(),
                    n => format!("{n} Roblox open"),
                };
                let state = match e.states.get(sid) {
                    Some(State::Applied) => "Settings in use",
                    Some(State::Waiting) => "Settings start next time Roblox opens",
                    Some(State::Failed) => "Couldn't change Roblox's settings",
                    None => "Potato is off",
                };
                ui.label(small(format!("{who} · {playing} · {state}")));
            });
        });
        ui.add_space(20.0);
        card(ui, "Setup", "", |ui| {
            switch_row(ui, "Let Potato Run This Account", "Off leaves this account's Roblox alone", false, &mut a.on);
            ui.add_enabled_ui(a.on, |ui| {
                stack_row(ui, "Quick Setup", Preset::of(&a).map_or("Custom: you changed something below", Preset::note), true, |ui, id| {
                    let mut pick = Preset::of(&a);
                    let opts: Vec<(Option<Preset>, &str)> = Preset::ALL.iter().map(|p| (Some(*p), p.label())).collect();
                    if segmented(ui, id.with("preset"), &mut pick, &opts) && let Some(p) = pick {
                        p.apply(&mut a);
                        self.fps_text.remove(sid);
                    }
                });
            });
        });
        ui.add_enabled_ui(a.on, |ui| {
            columns(ui, |cols| {
                card(col(cols, 0), "Graphics", "", |ui| {
                    row(ui, "FPS Cap", "Most frames a second Roblox draws. Empty for no cap", false, |ui, id| {
                        let text = self.fps_text.entry(sid.to_string()).or_insert_with(|| a.plan.fps_cap.map_or(String::new(), |f| f.to_string()));
                        ui.label(RichText::new("FPS").color(FAINT));
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(64.0).hint_text("No Cap").align(egui::Align2::CENTER_CENTER)).labelled_by(id);
                        if r.changed() {
                            text.retain(|c| c.is_ascii_digit());   // "60fps", "60 FPS" and "60" all work
                            text.truncate(3);
                            a.plan.fps_cap = text.parse::<u32>().ok().filter(|f| *f > 0);
                        }
                    });
                    switch_row(ui, "Lowest Graphics", "Roblox's graphics slider all the way down", false, &mut a.plan.lowest_graphics);
                    switch_row(ui, "Even Lower Graphics", "Blurry textures, no grass, no shadows, a plain grey sky", false, &mut a.plan.ultra_low);
                    switch_row(ui, "Open Maximized", "Roblox fills the screen but keeps its title bar", false, &mut a.plan.maximized);
                    switch_row(ui, "Mute", "No sound from this account's Roblox", true, &mut a.mute);
                });
                card(col(cols, 1), "CPU", "", |ui| {
                    row(ui, "Priority", "Low keeps the rest of your PC smooth", false, |ui, id| {
                        segmented(ui, id, &mut a.priority, &Priority::ALL.map(|p| (p, p.label())));
                    });
                    row(ui, "Max CPU", "Most of the PC's CPU one Roblox can use. Empty for no limit", false, |ui, id| {
                        number_box(ui, id, &mut a.cpu_limit, 100, "%", "None");
                    });
                    switch_row(ui, "Efficiency Mode", "Windows 11 runs Roblox slower to save CPU. Can lower FPS, so it's best for alts", true, &mut a.efficiency);
                });
                card(col(cols, 1), "When Things Go Wrong", "", |ui| {
                    let mut one = a.max_clients > 0;
                    if switch_row(ui, "Only One Roblox At A Time", "If a second Roblox opens on this account, the older one closes", false, &mut one) {
                        a.max_clients = one as u32;
                    }
                    row(ui, "Restart If It Uses Too Much RAM", "Restarts Roblox into the same server past this much. Empty for never", false, |ui, id| {
                        number_box(ui, id, &mut a.ram_limit_mb, 99_999, "MB", "Never");
                    });
                    switch_row(ui, "Rejoin After A Crash", "Opens Roblox again in the same server if it closes by itself. Leave off if a macro does this", true, &mut a.rejoin_crashed);
                });
            });
        });
        if e.profiles.len() > 1 && action(ui, "Copy These Settings To Every Account", false).clicked() {
            for p in e.profiles.clone() {
                *e.config.account_mut(&p.sid) = a.clone();
                self.fps_text.remove(&p.sid);
            }
            e.note = "Copied To Every Account".into();
        }
        if a != e.config.account(sid) {
            *e.config.account_mut(sid) = a;
        }
    }

    /// Stop Windows Update: one row, used on Performance and RDPs (an update can turn off Multiple RDPs). Potato keeps it stopped while it's on, since Windows turns its update service back on by itself.
    fn updates_row(&mut self, ui: &mut Ui, e: &mut Engine, last: bool) {
        let power = *self.power.lock().unwrap();
        let busy = *self.busy.lock().unwrap();
        let off = power.is_some_and(|p| p.updates_off);
        let note = if off {
            "On. Windows won't update or restart for updates. The Microsoft Store (and Roblox from the Store) can't install or update apps until you undo this"
        } else {
            "Windows stops updating and restarting by itself. The Microsoft Store can't install or update apps while it's on"
        };
        let mut click = false;
        row(ui, "Stop Windows Updates", note, last, |ui, id| {
            ui.add_enabled_ui(busy.is_none() && power.is_some(), |ui| {
                click = action(ui, if off { "Undo" } else { "Stop Updates" }, busy == Some("power")).labelled_by(id).clicked();
            });
        });
        if click {
            e.config.block_updates = !off;
            self.apply_power(ui.ctx(), vec![(Item::UpdatesOff, !off)]);
        }
    }

    /// One Optimize card (every PC setting Potato can turn on, one switch each and one button for all), the
    /// Windows tools, then the settings to fine-tune.
    fn performance_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let power = *self.power.lock().unwrap();
        let busy = *self.busy.lock().unwrap();
        let cores = e.cores as u32;
        enum Fix { Power(Item), Trim, HelperCores }
        let p = power.unwrap_or_default();
        // (on, title, note, what its switch does)
        let items = [
            (p.high_performance, "High Performance Power Plan", "Windows stops slowing your CPU down to save power. Off goes back to Balanced", Fix::Power(Item::HighPerformance)),
            (p.boost_off, "Turn Off CPU Boost", "Your CPU stays at its normal speed, so it runs cooler with fewer lag spikes", Fix::Power(Item::BoostOff)),
            (p.fast_startup_off, "Turn Off Fast Startup", "Shutting down really shuts down, so your PC starts fresh every time", Fix::Power(Item::FastStartupOff)),
            (e.config.trim_minutes > 0, "Free Roblox RAM Every 5 Minutes", "Keeps every Roblox's RAM low while you play", Fix::Trim),
            (e.config.helper_cores > 0, "Separate CPU Cores For RDPs", "RDPs and macros get the last 2 cores, so they never slow Roblox down", Fix::HelperCores),
            (p.smooth_rdp, "Smoother RDPs", "Fixes laggy, low FPS RDPs: medium picture quality and a steadier connection. Open RDPs need to reconnect", Fix::Power(Item::SmoothRdp)),
        ];
        let done = items.iter().filter(|i| i.0).count();
        let mut run: Vec<(&Fix, bool)> = Vec::new();
        card(ui, "Optimize", &format!("{done} of {} on", items.len()), |ui| {
            Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 12 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 180.0).max(120.0));
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(small(if done == items.len() { "Your PC is fully optimized" } else { "Turns on everything below in one click. You can turn any of them off again" }));
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, LINE_HI);
                        let share = ui.ctx().animate_value_with_time(egui::Id::unique("optimized"), done as f32 / items.len() as f32, dur(0.4));
                        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * share, 4.0)), 2.0, GOOD);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(done < items.len() && busy.is_none() && power.is_some(), |ui| {
                            if primary(ui, "Optimize Now").clicked() {
                                run = items.iter().filter(|i| !i.0).map(|i| (&i.3, true)).collect();
                            }
                        });
                    });
                });
            });
            let r = ui.min_rect();
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, LINE));
            for (i, (on, title, note, fix)) in items.iter().enumerate() {
                let waiting = matches!(fix, Fix::Power(_)) && (power.is_none() || busy.is_some());
                ui.add_enabled_ui(!waiting, |ui| {
                    let mut v = *on;
                    if switch_row(ui, title, note, i + 1 == items.len(), &mut v) {
                        run.push((fix, v));
                    }
                });
            }
        });
        let power_items: Vec<(Item, bool)> = run.iter().filter_map(|(f, on)| if let Fix::Power(it) = f { Some((*it, *on)) } else { None }).collect();
        for (f, on) in &run {
            match f {
                Fix::Trim => e.config.trim_minutes = if *on { 5 } else { 0 },
                Fix::HelperCores => e.config.helper_cores = if *on { 2.min(cores.saturating_sub(2)) } else { 0 },
                Fix::Power(_) => {}
            }
        }
        if !power_items.is_empty() {
            self.apply_power(ui.ctx(), power_items);
        }
        self.windows_card(ui, e);

        let trim_note = match e.last_trim_freed {
            Some(f) => {
                let ago = match e.last_trim.elapsed().as_secs() / 60 { 0 => "just now".to_string(), 1 => "a minute ago".into(), m => format!("{m} min ago") };
                if f == 0 { format!("Last ran {ago}: Roblox had nothing to free") } else { format!("Last ran {ago} and freed {}. Roblox takes some back as it plays", gb(f)) }
            }
            None => "Frees the RAM Roblox isn't using right now".into(),
        };
        let off_or = |n: u32, f: &dyn Fn(u32) -> String| if n == 0 { "Off".to_string() } else { f(n) };
        let c = &mut e.config;
        let shutdown_text = &mut self.shutdown_text;
        columns(ui, |cols| {
            card(col(cols, 0), "RAM", "", |ui| {
                let list = [0, 1, 2, 5, 10, 15, 30].map(|n| (n, off_or(n, &|n| format!("Every {n} Min"))));
                row(ui, "Free Roblox RAM", &trim_note, false, |ui, id| { dropdown(ui, "trim", id, &mut c.trim_minutes, &list); });
                row(ui, "Free PC RAM When It's This Full", "Clears files Windows keeps in RAM just in case. Empty for never", true, |ui, id| {
                    ui.label(RichText::new("%").color(FAINT));
                    let mut text = if c.clean_above == 0 { String::new() } else { c.clean_above.to_string() };
                    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0).hint_text("Never").align(egui::Align2::CENTER_CENTER)).labelled_by(id).changed() {
                        text.retain(|ch| ch.is_ascii_digit());
                        c.clean_above = text.parse::<u32>().unwrap_or(0).min(99);
                    }
                });
            });
            card(col(cols, 0), "Internet", "", |ui| {
                row(ui, "Limit Roblox's Upload Speed", "For slow internet, so Roblox doesn't use it all. Empty for no limit", true, |ui, id| {
                    number_box(ui, id, &mut c.upload_kbps, 1_000_000, "kbit/s", "None");
                });
            });
            card(col(cols, 1), "CPU Cores", &format!("{cores} on this PC"), |ui| {
                let list: Vec<(u32, String)> = [0, 1, 2, 3, 4, 6, 8].into_iter().filter(|n| *n <= cores).map(|n| (n, off_or(n, &|n| format!("{n} Each")))).collect();
                row(ui, "Cores For Each Roblox", "Each Roblox gets cores of its own, so they don't fight over them", false, |ui, id| { dropdown(ui, "cores", id, &mut c.cores_per_client, &list); });
                let list: Vec<(u32, String)> = [0, 2, 3].into_iter().filter(|n| *n < cores).map(|n| (n, off_or(n, &|n| format!("Last {n} Cores")))).collect();
                row(ui, "Cores For RDPs And Macros", "RDPs and AutoHotkey run here, and Roblox stays off them", true,
                    |ui, id| { dropdown(ui, "helper-cores", id, &mut c.helper_cores, &list); });
            });
            card(col(cols, 1), "Restarts", "", |ui| {
                let list = [0, 1, 2, 4, 6, 12, 24].map(|n| (n, off_or(n, &|n| format!("Every {n} {}", if n == 1 { "Hour" } else { "Hours" }))));
                row(ui, "Restart Roblox", "Closes every Roblox and opens each one in the same server", false, |ui, id| { dropdown(ui, "restart", id, &mut c.restart_hours, &list); });
                row(ui, "Shut Down The PC At", "Every day at this time, like 6:30 or 6pm. Empty for never", true, |ui, id| {
                    let text = shutdown_text.get_or_insert_with(|| c.shutdown_at.clone());
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(84.0).hint_text("Never").align(egui::Align2::CENTER_CENTER)).labelled_by(id);
                    let parsed = crate::config::parse_time(text);
                    if r.changed() {
                        text.truncate(8);
                        if text.trim().is_empty() {
                            c.shutdown_at.clear();
                        } else if let Some(t) = parsed {
                            c.shutdown_at = crate::config::format_time(t);
                        }
                    }
                    if r.lost_focus() && let Some(t) = parsed {
                        *text = crate::config::format_time(t);   // show how it was understood
                    }
                    if !r.has_focus() && !text.trim().is_empty() && parsed.is_none() {   // not while still typing
                        ui.label(RichText::new("Like 6:30 or 6pm").size(12.5).color(BAD));
                    }
                });
            });
        });
    }

    /// Stopping updates and Windows' own clean-up tools, in one card.
    fn windows_card(&mut self, ui: &mut Ui, e: &mut Engine) {
        let busy = *self.busy.lock().unwrap();
        let (mut point, mut debloat, mut shutup) = (false, false, false);
        let armed = self.confirm_debloat.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        card(ui, "Windows", "", |ui| {
            self.updates_row(ui, e, false);
            row(ui, "Make A Restore Point", "Lets you undo the two tools below with System Restore. Windows makes one a day at most", false, |ui, id| {
                ui.add_enabled_ui(busy.is_none(), |ui| point = action(ui, if busy == Some("point") { "Making" } else { "Make One" }, busy == Some("point")).labelled_by(id).clicked());
            });
            row(ui, "Remove Windows Bloat", "Opens Chris Titus Tech's tool. Picking the wrong things can break Windows Update or the Store", false, |ui, id| {
                debloat = action(ui, if armed { "Click Again" } else { "Open" }, false).labelled_by(id).clicked();
            });
            row(ui, "Turn Off Windows Tracking", "Opens the O&O ShutUp10++ download. Some of its switches turn off Windows Update or Defender", true, |ui, id| {
                shutup = action(ui, "Open", false).labelled_by(id).clicked();
            });
        });
        if armed {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if point {
            self.run_action("point", ui.ctx(), |e| e.restore_point());
        }
        if debloat {
            self.confirm_debloat = if armed {
                if !crate::system::console("irm https://christitus.com/win | iex") {
                    e.note = "Couldn't Open The Windows Tool".into();
                }
                None
            } else {
                Some(Instant::now())
            };
        }
        if shutup {
            crate::system::launch("https://www.oo-software.com/en/shutup10");
        }
    }

    /// Notifications, the Discord report and Potato itself.
    fn settings_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let (mut test, mut restore) = (false, false);
        let c = &mut e.config;
        let webhook_text = &mut self.webhook_text;
        columns(ui, |cols| {
            card(col(cols, 1), "Notifications", "", |ui| {
                switch_row(ui, "Tell Me When RAM Is Freed", "A Windows notification each time Potato frees RAM", true, &mut c.notify);
            });
            card(col(cols, 0), "Discord", "", |ui| {
                stack_row(ui, "Webhook", "Sends an update with a picture of every alt each hour. Get the link in Discord: channel settings, Integrations, Webhooks, Copy Webhook URL", false, |ui, id| {
                    let text = webhook_text.get_or_insert_with(|| c.discord.webhook.clone());
                    let hide = !text.is_empty();   // the link is a password for the channel
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width().min(420.0)).hint_text("https://discord.com/api/webhooks/...").password(hide)).labelled_by(id);
                    if r.changed() {
                        *text = text.trim().to_string();
                        if text.is_empty() || crate::config::Discord::valid_webhook(text) {
                            c.discord.webhook = text.clone();
                        }
                    }
                    if !text.is_empty() && !crate::config::Discord::valid_webhook(text) {
                        ui.label(RichText::new("Not A Discord Webhook").size(12.5).color(BAD));
                    }
                });
                switch_row(ui, "Include Screenshots", "A picture of each alt's Roblox. Minimized or disconnected alts can't be pictured", false, &mut c.discord.screenshots);
                row(ui, "Send One Now", "To check it works", true, |ui, id| {
                    ui.add_enabled_ui(!c.discord.webhook.is_empty(), |ui| test = action(ui, "Send", false).labelled_by(id).clicked());
                });
            });
            card(col(cols, 1), "Potato", "", |ui| {
                row(ui, "Undo Everything Potato Changed", "Puts every Roblox file back the way it was and turns Potato off on every account", false, |ui, id| {
                    restore = action(ui, "Undo All", false).labelled_by(id).clicked();
                });
                row(ui, "Quit Potato", "Closing the window keeps Potato running in the tray", true, |ui, id| {
                    if action(ui, "Quit", false).labelled_by(id).clicked() {
                        std::process::exit(0);
                    }
                });
            });
        });
        if test {
            e.send_report_now();
            e.note = "Sending An Update To Discord".into();
        }
        if restore {
            e.restore();
            self.fps_text.clear();
        }
    }

    /// RDP alts: how many this PC can run, Multiple RDPs, each alt, and adding new ones.
    fn rdp_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let r = e.rdp.lock().unwrap().clone();
        let busy = r.busy;
        let (mut patch, mut undo, mut add, mut open, mut show, mut open_all, mut restart) = (false, false, false, None, None, false, false);
        let (mut startup, mut shortcut, mut wide, mut remove) = (None, None, None, None);
        let (mut change, mut expire, mut repair) = (None, false, false);
        let armed = self.confirm_remove.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(4)).map(|(n, _)| n.clone());
        let ram_gb = (e.memory.1 as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0;   // what Task Manager shows: 7.99 is 8.0 GB
        let (by_cpu, by_ram) = crate::rdp::capacity(e.physical_cores, e.cores, ram_gb);
        let can = by_cpu.min(by_ram);
        let open_now = self.wins.iter().filter(|w| matches!(w.kind, Kind::Rdp(_))).count();
        card(ui, "How Many RDPs This PC Can Run", "Worked out from your CPU and RAM", |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(semi(&format!("About {can}"), 24.0).color(if can == 0 { BAD } else { TEXT }));
                    ui.add_space(6.0);
                    ui.label(small(format!("{} open now", open_now)));
                });
                ui.add_space(4.0);
                bar(ui, ui.available_width(), if can == 0 { 1.0 } else { open_now as f32 / can as f32 });
                ui.add_space(8.0);
                let why = if by_ram <= by_cpu {
                    format!("Your RAM is the limit: you have {ram_gb:.0} GB, and each RDP needs about 4 GB once Windows has its share. Your CPU could run {by_cpu}.")
                } else {
                    format!("Your CPU is the limit: {} cores and {} threads, and each RDP needs about 2 threads. Your RAM could run {by_ram}.", e.physical_cores, e.cores)
                };
                ui.label(small(why));
                if e.base_mhz > 0 {
                    let ghz = e.base_mhz as f32 / 1000.0;
                    ui.label(small(if ghz >= 2.5 { format!("Your CPU runs at {ghz:.1} GHz, which is fast enough.") } else { format!("Your CPU runs at {ghz:.1} GHz. Under 2.5 GHz, expect fewer RDPs than this.") }));
                }
            });
        });
        card(ui, "RDP Status", "What Windows says right now", |ui| {
            let running = r.service == "Running";
            row(ui, "Windows", &r.windows, false, |_, _| {});
            row(ui, "RDP Service", if running { "" } else { "Alts can't connect while it's stopped. Restart it below" }, false, |ui, _| {
                status(ui, if r.service.is_empty() { "Missing" } else { &r.service }, running);
            });
            row(ui, "Multiple RDPs", "", false, |ui, _| status(ui, match r.patch { Patch::Patched => "On", Patch::Unknown => "Checking", _ => "Off" }, r.patch == Patch::Patched));
            row(ui, "Listening", if r.listening || r.port.is_empty() { "" } else { "Nothing can connect. Restarting the RDP service usually fixes it" }, true, |ui, _| {
                status(ui, &if r.listening { format!("On Port {}", r.port) } else { "Not Listening".into() }, r.listening);
            });
        });
        card(ui, "RDP Settings", "", |ui| {
            let (note, can_patch) = match r.patch {
                Patch::Patched => ("On. All your RDPs can be signed in at once. Potato warns you if a Windows update turns it off", false),
                Patch::Unpatched => ("Windows only lets one RDP be signed in at a time. This lets all of them, and keeps working after Windows updates. Open RDPs disconnect for a second, and Roblox keeps running", true),
                Patch::Unsupported => ("This only works on PCs with an Intel or AMD CPU, so only one RDP can be signed in at a time", false),
                Patch::Unknown => ("Checking Windows", false),
            };
            row(ui, "Allow Multiple RDPs", note, false, |ui, id| {
                if can_patch {
                    ui.add_enabled_ui(busy.is_none(), |ui| patch = button(ui, if busy == Some("patch") { "Turning On" } else { "Turn On" }, busy == Some("patch"), true).labelled_by(id).clicked());
                } else if r.patch == Patch::Patched && r.can_undo {
                    ui.add_enabled_ui(busy.is_none(), |ui| undo = action(ui, if busy == Some("undo") { "Turning Off" } else { "Turn Off" }, busy == Some("undo")).labelled_by(id).clicked());
                }
            });
            self.updates_row(ui, e, false);
            row(ui, "Restart The RDP Service", "Fixes alts that won't connect or are stuck on a black screen. Open RDPs disconnect for a moment, and Roblox keeps running", false, |ui, id| {
                ui.add_enabled_ui(busy.is_none(), |ui| restart = action(ui, if busy == Some("restart") { "Restarting" } else { "Restart" }, busy == Some("restart")).labelled_by(id).clicked());
            });
            row(ui, "Make Every Account Never Expire", "Windows locks local accounts out after 42 days unless they're set to never expire. Potato does this for alts already", false, |ui, id| {
                ui.add_enabled_ui(busy.is_none(), |ui| expire = action(ui, if busy == Some("expire") { "Fixing" } else { "Fix" }, busy == Some("expire")).labelled_by(id).clicked());
            });
            if r.patch != Patch::Unknown && r.service.is_empty() {
                row(ui, "Repair The RDP Service", "Windows' Remote Desktop service is missing, so no alt can connect. This puts it back", false, |ui, id| {
                    ui.add_enabled_ui(busy.is_none(), |ui| repair = primary(ui, if busy == Some("repair") { "Repairing" } else { "Repair" }).labelled_by(id).clicked());
                });
            }
            row(ui, "Max CPU For Each RDP", "Stops one RDP from hogging the CPU. Too low and alts freeze or disconnect. Empty for no limit", true, |ui, id| {
                number_box(ui, id, &mut e.config.rdp_cpu_limit, 100, "%", "None");
            });
        });
        card(ui, "Lag And Picture", "For every RDP on this PC. Reconnect your RDPs after a change", |ui| {
            ui.add_enabled_ui(busy.is_none(), |ui| {
                let mut tcp = r.tcp_only;
                if switch_row(ui, "Use TCP Instead Of UDP", "Fixes RDPs that freeze, lag or go black for a few seconds", false, &mut tcp) {
                    change = Some((crate::rdp::Setting::TcpOnly, tcp.then_some(1)));
                }
                let mut fps = r.fps60;
                if switch_row(ui, "60 FPS In RDPs", "Windows caps RDPs at 30 FPS. 60 looks smoother but uses more CPU. Restart the PC to use it", false, &mut fps) {
                    change = Some((crate::rdp::Setting::Fps60, fps.then_some(15)));
                }
                row(ui, "Picture Quality", "Lower uses less CPU for every RDP", false, |ui, id| {
                    let mut q = r.quality;
                    let options = [(None, "Windows Default".to_string()), (Some(4), "Low".into()), (Some(3), "Medium".into()), (Some(2), "High".into()), (Some(1), "Lossless".into())];
                    if dropdown(ui, "rdp-quality", id, &mut q, &options) {
                        change = Some((crate::rdp::Setting::Quality, q));
                    }
                });
                row(ui, "Compression", "Your RDPs never leave this PC, so Off saves CPU and loses nothing", false, |ui, id| {
                    let mut c = r.compression;
                    let options = [(None, "Windows Default".to_string()), (Some(0), "Off".into()), (Some(1), "Less Memory".into()), (Some(2), "Balanced".into()), (Some(3), "Less Bandwidth".into())];
                    if dropdown(ui, "rdp-compression", id, &mut c, &options) {
                        change = Some((crate::rdp::Setting::Compression, c));
                    }
                });
                let mut hide = r.hide_alts;
                if switch_row(ui, "Hide Alts On The Sign-In Screen", "Your alts stop showing when Windows starts. They still open in RDP, and you're never hidden", true, &mut hide) {
                    change = Some((crate::rdp::Setting::HideAlts, hide.then_some(0)));
                }
            });
        });
        card(ui, "Your Alts", "Each RDP opens with no sound, no drives and no wallpaper, and Windows keys stay on this PC", |ui| {
            if r.accounts.is_empty() {
                Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| { ui.label(small("No alts yet. Add one below.")); });
            }
            let n = r.accounts.len();
            let closed = r.accounts.iter().filter(|a| !self.wins.iter().any(|w| w.kind == Kind::Rdp((*a).clone()))).count();
            if n > 1 {
                row(ui, "Open Every Alt", &if closed == 0 { "They're all open".to_string() } else { format!("Opens the {} not open yet", plural(closed, "alt", "alts")) }, false, |ui, id| {
                    ui.add_enabled_ui(closed > 0, |ui| open_all = primary(ui, "Open All").labelled_by(id).clicked());
                });
            }
            for (i, name) in r.accounts.iter().enumerate() {
                let win = self.wins.iter().find(|w| w.kind == Kind::Rdp(name.clone())).map(|w| w.hwnd);
                let is_wide = e.config.rdp_wide.contains(name);
                let pic = self.avatars.texture(ui.ctx(), name);
                let fr = Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let id = ui.horizontal(|ui| {
                        avatar(ui, pic.as_ref(), name, 34.0, win.is_some(), CARD);
                        ui.add_space(4.0);
                        let id = ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let id = ui.label(semi(name, 13.5).color(TEXT)).id;
                            ui.label(small(format!("{} · {}", if win.is_some() { "Open" } else { "Closed" }, crate::rdp::address(name))));
                            id
                        }).inner;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            match win {
                                Some(h) => if action(ui, "Show", false).labelled_by(id).on_hover_text("Goes to this alt's window").clicked() { show = Some(h); },
                                None => if primary(ui, "Open").labelled_by(id).clicked() { open = Some(name.clone()); },
                            }
                            let sure = armed.as_deref() == Some(name.as_str());
                            ui.add_enabled_ui(busy.is_none(), |ui| {
                                if action(ui, if sure { "Click Again To Remove" } else if busy == Some("remove") { "Removing" } else { "Remove" }, false)
                                    .on_hover_text("Signs the alt out and deletes its Windows account and files, Roblox in it too").clicked() {
                                    remove = Some(name.clone());
                                }
                            });
                            if action(ui, "Desktop Shortcut", false).on_hover_text("Puts a shortcut on your desktop that opens this alt with these settings, even without Potato").clicked() {
                                shortcut = Some(name.clone());
                            }
                        });
                        id
                    }).inner;
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(42.0);
                        let mut wd = is_wide;
                        if segmented(ui, id.with("size"), &mut wd, &[(false, "Normal 1366x768"), (true, "Wide 1800x768")]) {
                            wide = Some((name.clone(), wd));
                        }
                        ui.add_space(8.0);
                        let mut on = e.config.rdp_startup.contains(name);
                        if ui.checkbox(&mut on, "Open When I Sign In To Windows").changed() {
                            startup = Some((name.clone(), on));
                        }
                    });
                });
                if i + 1 < n {
                    let rect = fr.response.rect;
                    ui.painter().hline(rect.x_range().shrink(16.0), rect.bottom(), Stroke::new(1.0, LINE));
                }
            }
        });
        let ok_name = crate::rdp::valid_name(&self.new_name);
        card(ui, "Add An Alt", "Makes a Windows account for an alt, or type an existing one's name and password", |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let name_id = ui.label(RichText::new("Name").size(13.0).color(FAINT)).id;
                    let r = ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(190.0).hint_text("Alt3").char_limit(20)).labelled_by(name_id);
                    ui.add_space(6.0);
                    let pass_id = ui.label(RichText::new("Password").size(13.0).color(FAINT)).id;
                    ui.add(egui::TextEdit::singleline(&mut self.new_pass).desired_width(190.0).password(true)).labelled_by(pass_id);
                    ui.add_space(6.0);
                    let ready = ok_name && !self.new_pass.is_empty() && busy.is_none();
                    ui.add_enabled_ui(ready, |ui| add = primary(ui, if busy == Some("add") { "Adding" } else { "Add Alt" }).clicked());
                    if !self.new_name.is_empty() && !ok_name && !r.has_focus() {
                        ui.label(RichText::new("Names can't have / \\ [ ] : ; | = , + * ? < > @").size(12.5).color(BAD));
                    }
                });
            });
        });
        if patch {
            engine::refresh_rdp(e.rdp.clone(), Some("patch"), crate::rdp::patch);
        }
        if restart {
            engine::refresh_rdp(e.rdp.clone(), Some("restart"), crate::rdp::restart_service);
        }
        if let Some((setting, value)) = change {
            let alts = r.accounts.clone();
            engine::refresh_rdp(e.rdp.clone(), Some("setting"), move || crate::rdp::set(setting, value, &alts));
        }
        if expire {
            engine::refresh_rdp(e.rdp.clone(), Some("expire"), crate::rdp::never_expire);
        }
        if repair {
            engine::refresh_rdp(e.rdp.clone(), Some("repair"), crate::rdp::repair_service);
        }
        if undo {
            e.config.rdp_patched = false;   // you turned it off, not Windows: no warning
            engine::refresh_rdp(e.rdp.clone(), Some("undo"), crate::rdp::undo);
        }
        if add {
            let (name, pass) = (self.new_name.trim().to_string(), std::mem::take(&mut self.new_pass));
            self.new_name.clear();
            engine::refresh_rdp(e.rdp.clone(), Some("add"), move || crate::rdp::add_account(&name, &pass));
        }
        if let Some((name, w)) = wide {
            e.config.rdp_wide.retain(|n| *n != name);
            if w {
                e.config.rdp_wide.push(name.clone());
            }
            let _ = crate::rdp::write_file(&name, w);   // a desktop shortcut opens the file: keep it in step
        }
        if let Some(name) = shortcut {
            e.note = match crate::rdp::desktop_shortcut(&name, e.config.rdp_wide.contains(&name)) {
                Ok(()) => format!("Put {name} On Your Desktop"),
                Err(m) => m,
            };
        }
        if let Some((name, on)) = startup {
            e.config.rdp_startup.retain(|n| *n != name);
            if on {
                e.config.rdp_startup.push(name);
            }
            if !crate::rdp::save_startup(!e.config.rdp_startup.is_empty()) {
                e.note = "Couldn't Set It To Open When You Sign In".into();
            }
        }
        if armed.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if let Some(name) = remove {
            if armed.as_deref() == Some(name.as_str()) {
                self.confirm_remove = None;
                e.config.rdp_startup.retain(|n| *n != name);
                e.config.rdp_wide.retain(|n| *n != name);
                crate::rdp::save_startup(!e.config.rdp_startup.is_empty());
                engine::refresh_rdp(e.rdp.clone(), Some("remove"), move || crate::rdp::remove_account(&name));
            } else {
                self.confirm_remove = Some((name, Instant::now()));
            }
        }
        if let Some(h) = show {
            crate::preview::focus(h);
        }
        if open_all {
            for name in r.accounts.iter().filter(|a| !self.wins.iter().any(|w| w.kind == Kind::Rdp((*a).clone()))) {
                if let Err(m) = crate::rdp::open(name, e.config.rdp_wide.contains(name)) {
                    e.note = m;
                }
            }
        }
        if let Some(name) = open && let Err(m) = crate::rdp::open(&name, e.config.rdp_wide.contains(&name)) {
            e.note = m;
        }
    }
}
