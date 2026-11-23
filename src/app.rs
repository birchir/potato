//! The window: an icon rail on the left (Overview, Accounts, Remote Desktop, Performance, Settings), a top bar
//! with the PC's live numbers and the actions for every client, and the page in cards. The look lives in
//! widgets.rs; all the work happens in engine.rs. Closing the window hides it to the tray.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, vec2};

use crate::config::{Preset, Priority};
use crate::engine::{self, Engine, State, UpdateState, gb};
use crate::preview::Kind;
use crate::rdp::Patch;
use crate::widgets::*;

pub fn run() -> eframe::Result {
    crate::system::enable_privileges();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Potato")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default())
            .with_inner_size([1120.0, 780.0]).with_min_inner_size([860.0, 580.0]),
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

pub struct App {
    engine: Arc<Mutex<Engine>>,
    page: Page,
    /// Restart All was clicked once and asks for a second click until then
    confirm_restart: Option<Instant>,
    confirm_debloat: Option<Instant>,
    update_dismissed: bool,
    /// what's typed in each account's FPS box
    fps_text: HashMap<String, String>,
    /// what's typed in the boxes whose value only counts once it's complete (a time, a link)
    shutdown_text: Option<String>,
    webhook_text: Option<String>,
    /// the Remote Desktop page's new account boxes
    new_name: String,
    new_pass: String,
    /// the action running in the background, if any (so the window never waits on it)
    busy: Arc<Mutex<Option<&'static str>>>,
    /// the last result shown as a toast, and when it appeared (the window's own clock, so animation stays in step)
    toast: Option<(String, f64)>,
    /// Windows' power settings as last read (None until the first read is back)
    power: Arc<Mutex<Option<crate::power::State>>>,
    /// how many live pictures a row of the Overview holds
    tile_cols: usize,
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
            update_dismissed: false,
            fps_text: HashMap::new(),
            shutdown_text: None,
            webhook_text: None,
            new_name: String::new(),
            new_pass: String::new(),
            busy: Arc::default(),
            toast: None,
            power: Arc::default(),
            tile_cols: 3,
            thumbs: crate::preview::Thumbs::new(hwnd),
            wins: Vec::new(),
            wins_at: None,
            hwnd,
        };
        app.read_power(&cc.egui_ctx);
        app
    }

    /// Read Windows' power settings on a thread (powercfg takes a moment).
    fn read_power(&self, ctx: &egui::Context) {
        let (power, ctx) = (self.power.clone(), ctx.clone());
        std::thread::spawn(move || {
            *power.lock().unwrap() = Some(crate::power::read());
            ctx.request_repaint();
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
        egui::Panel::left("rail").exact_size(84.0).resizable(false).frame(Frame::new().fill(RAIL).inner_margin(Margin::symmetric(0, 14)))
            .show(ui, |ui| self.rail(ui, &e));
        egui::Panel::top("bar").exact_size(76.0).frame(Frame::new().fill(BG).inner_margin(Margin { left: 32, right: 28, top: 14, bottom: 6 }))
            .show(ui, |ui| self.top_bar(ui, &mut e));
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin { left: 32, right: 28, top: 8, bottom: 20 })).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("page").show(ui, |ui| {
                // settings don't stretch across a wide screen; the live pictures may
                let wide = matches!(self.page, Page::Overview | Page::Accounts);
                ui.set_width(ui.available_width().min(if wide { 1600.0 } else { 1180.0 }));
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

/// Two columns side by side on a wide window, one under the other on a narrow one.
fn columns(ui: &mut Ui, body: impl FnOnce(&mut [Ui])) {
    let n = if ui.available_width() >= 860.0 { 2 } else { 1 };
    ui.columns(n, |cols| {
        if n == 2 {
            for c in cols.iter_mut() {
                c.spacing_mut().item_spacing.x = 14.0;
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

impl App {
    fn rail(&mut self, ui: &mut Ui, e: &Engine) {
        ui.vertical_centered(|ui| logo(ui, 1.8));
        ui.add_space(18.0);
        let rdp_warn = e.rdp.lock().unwrap().reverted(e.config.rdp_patched);
        let pages = [
            (Icon::Grid, "Overview", Page::Overview, false),
            (Icon::People, "Accounts", Page::Accounts, false),
            (Icon::Monitor, "Remote", Page::RemoteDesktop, rdp_warn),
            (Icon::Bolt, "Performance", Page::Performance, false),
            (Icon::Sliders, "Settings", Page::Settings, false),
        ];
        let mut marker = None;
        for (icon_kind, name, page, alert) in pages {
            let picked = self.page == page || (page == Page::Accounts && matches!(self.page, Page::Account(_)));
            let r = rail_item(ui, icon_kind, name, picked, alert);
            if picked {
                marker = Some(r.rect);
            }
            if r.clicked() {
                self.page = page;
            }
        }
        // the marker slides from the old page to the new one (post to post, slow in and out)
        if let Some(rect) = marker {
            let y = ui.ctx().animate_value_with_time(egui::Id::unique("rail-marker"), rect.top(), dur(0.2));
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), y + 19.0), vec2(3.0, 24.0)), 2.0, ACCENT);
        }
    }

    /// The page's name, the PC's live numbers, and the actions for every client.
    fn top_bar(&mut self, ui: &mut Ui, e: &mut Engine) {
        let (title, crumb) = self.title(e);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(semi(&title, 24.0).color(TEXT));
                ui.label(small(crumb));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = *self.busy.lock().unwrap();
                if action(ui, "Trim All", busy == Some("trim")).on_hover_text("Frees memory every Roblox client isn't using right now").clicked() {
                    self.run_action("trim", ui.ctx(), |e| e.trim_all(true));
                }
                let armed = self.confirm_restart.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                if action(ui, if armed { "Click Again" } else { "Restart All" }, busy == Some("restart"))
                    .on_hover_text("Closes every Roblox client and opens each one again in the same server").clicked() {
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
                if action(ui, "Clean Memory", busy == Some("clean")).on_hover_text("Empties Windows' standby memory").clicked() {
                    self.run_action("clean", ui.ctx(), |e| e.clean(true));
                }
                ui.add_space(14.0);
                let cpu = (e.cpu_pct.values().sum::<f32>().clamp(0.0, 100.0) + 0.0).round();
                meter(ui, "Roblox CPU", &format!("{cpu:.0}%"), cpu / 100.0);
                let (used, total) = e.memory;
                // counts to the new value instead of jumping (follow through)
                let shown = ui.ctx().animate_value_with_time(egui::Id::unique("ram-used"), used as f32, dur(0.6));
                meter(ui, "RAM", &format!("{} / {}", gb(shown as u64), gb(total)), if total > 0 { used as f32 / total as f32 } else { 0.0 });
            });
        });
    }

    fn title(&self, e: &Engine) -> (String, String) {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match &self.page {
            Page::Overview => {
                let rdps = self.wins.iter().filter(|w| matches!(w.kind, Kind::Rdp(_))).count();
                ("Overview".into(), format!("{} · {} open", plural(e.clients.len(), "Roblox client", "Roblox clients"), plural(rdps, "Remote Desktop", "Remote Desktops")))
            }
            Page::Accounts => ("Accounts".into(), format!("{} with Roblox on this PC", plural(e.profiles.len(), "Windows account", "Windows accounts"))),
            Page::Account(sid) => {
                let name = e.profiles.iter().find(|p| &p.sid == sid).map_or("Account".to_string(), |p| p.name.clone());
                let running = match e.clients.iter().filter(|c| &c.sid == sid).count() {
                    0 => "Roblox closed".to_string(),
                    1 => "Roblox running".to_string(),
                    n => format!("{n} clients running"),
                };
                let state = match e.states.get(sid) {
                    Some(State::Applied) => "settings applied",
                    Some(State::Waiting) => "applies when Roblox restarts",
                    Some(State::Failed) => "couldn't write settings",
                    None => "Potato is off",
                };
                (name, format!("{running} · {state}"))
            }
            Page::RemoteDesktop => {
                let r = e.rdp.lock().unwrap();
                let at_once = match r.patch {
                    Patch::Patched => "all on at once",
                    Patch::Unpatched | Patch::Unsupported => "one at a time",
                    Patch::Unknown => "checking",
                };
                ("Remote Desktop".into(), format!("{} · {at_once}", plural(r.accounts.len(), "account", "accounts")))
            }
            Page::Performance => ("Performance".into(), "Everything from the RDP optimization guides, for the whole PC".into()),
            Page::Settings => ("Settings".into(), format!("Potato {}", env!("CARGO_PKG_VERSION"))),
        }
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
        egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, vec2(42.0, -24.0 + (1.0 - come) * 18.0))
            .interactable(false).show(ctx, |ui| {
                ui.set_opacity(come.min(1.0) * go);
                Frame::new().fill(TEXT).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(18, 11)).show(ui, |ui| {
                    ui.add(egui::Label::new(semi(text, 13.5).color(RAIL)).extend());   // one line, never squeezed by the window edge
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

    fn banner(&mut self, ui: &mut Ui, e: &Engine) {
        self.rdp_banner(ui, e);
        let state = e.update.lock().unwrap().clone();
        let (title, note) = match &state {
            UpdateState::None => return,
            UpdateState::Available(_) if self.update_dismissed => return,
            UpdateState::Available(r) => (format!("Potato {} Is Out", r.tag), "Installs in a few seconds and restarts by itself.".to_string()),
            UpdateState::Installing(tag) => (format!("Installing Potato {tag}"), "Downloading and checking it.".to_string()),
            UpdateState::Failed(m) => ("Update Didn't Install".to_string(), m.clone()),
        };
        Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.5))).corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(semi(&title, 14.0).color(TEXT));
                    ui.label(small(note));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match state {
                    UpdateState::Available(r) => {
                        if primary(ui, "Update Now").clicked() {
                            engine::install_update(r, e.update.clone());
                        }
                        if ui.add(egui::Button::new(RichText::new("Later").color(DIM)).frame(false)).clicked() {
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
            });
        });
        ui.add_space(14.0);
    }

    /// Windows undid the Remote Desktop patch (an update put its own file back): say so on every page, with the fix.
    fn rdp_banner(&mut self, ui: &mut Ui, e: &Engine) {
        let r = e.rdp.lock().unwrap().clone();
        if !r.reverted(e.config.rdp_patched) {
            return;
        }
        let fits = r.patch == Patch::Unpatched;
        let note = if fits { "Only one Remote Desktop can be on until it's patched again." } else { "Windows updated its Remote Desktop file and Potato's patch doesn't fit it yet. Only one Remote Desktop can be on." };
        Frame::new().fill(Color32::from_rgb(48, 20, 20)).stroke(Stroke::new(1.0, BAD)).corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(semi("Windows Undid The Remote Desktop Patch", 14.0).color(TEXT));
                    ui.label(RichText::new(note).size(12.5).color(DIM));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if fits {
                        if button(ui, if r.busy == Some("patch") { "Patching" } else { "Patch Again" }, r.busy == Some("patch"), true).clicked() {
                            engine::refresh_rdp(e.rdp.clone(), Some("patch"), crate::rdp::patch);
                        }
                    } else if self.page != Page::RemoteDesktop && action(ui, "Open Remote Desktop", false).clicked() {
                        self.page = Page::RemoteDesktop;
                    }
                });
            });
        });
        ui.add_space(14.0);
    }
}

/// A live number in the top bar: its name, the number, and a thin bar for how full it is.
fn meter(ui: &mut Ui, label: &str, value: &str, share: f32) {
    Frame::new().fill(CARD).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
        ui.set_width(150.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(label).size(11.5).color(FAINT));
            ui.label(semi(value, 13.5).color(TEXT));
            let (r, _) = ui.allocate_exact_size(vec2(150.0, 3.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, LINE);
            ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * share.clamp(0.0, 1.0), 3.0)), 2.0, if share > 0.85 { BAD } else { TEXT });
        });
    });
}

/// One live tile: who it is, the window to picture (None: a Remote Desktop that isn't open), and its Roblox.
struct Tile {
    name: String,
    note: &'static str,
    win: Option<crate::preview::Win>,
    /// the Windows user whose Roblox it is (for its RAM, CPU and Trim)
    user: Option<String>,
    rdp: Option<String>,
}

impl App {
    /// The usage chart, then a live picture of every Remote Desktop and Roblox window with what each one uses.
    fn overview_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let accounts = e.rdp.lock().unwrap().accounts.clone();
        let (used, total) = e.memory;
        let roblox: u64 = e.clients.iter().map(|c| c.ram).sum();
        card(ui, "", "", |ui| {
            Frame::new().inner_margin(Margin::symmetric(20, 12)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(220.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(small("RAM in use"));
                        ui.label(semi(&gb(used), 30.0).color(TEXT));
                        ui.label(small(format!("of {}", gb(total))));
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 3.0), Sense::hover());
                            ui.painter().rect_filled(r, 1.0, ACCENT);
                            ui.label(small(format!("Roblox {}", gb(roblox))));
                        });
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 3.0), Sense::hover());
                            ui.painter().rect_filled(r, 1.0, TEXT);
                            ui.label(small("Whole PC"));
                        });
                        ui.add_space(8.0);
                        ui.label(semi(&format!("{} saved", gb(e.freed)), 14.0).color(GOOD));
                        ui.label(small("by trims since Potato started"));
                    });
                    ui.vertical(|ui| {
                        let t = total.max(1) as f32;
                        let pc: Vec<f32> = e.history.iter().map(|s| s.pc_used as f32 / t).chain([used as f32 / t]).collect();
                        let rb: Vec<f32> = e.history.iter().map(|s| s.roblox as f32 / t).chain([roblox as f32 / t]).collect();
                        let span = e.history.first().map_or(0, |s| e.history.last().map_or(0, |l| l.at - s.at));
                        ui.horizontal(|ui| {
                            ui.label(small(if pc.len() < 3 { "A point every 30 seconds: the chart fills in as Potato runs".to_string() } else { format!("Last {} min", span / 60 + 1) }));
                        });
                        chart(ui, vec2(ui.available_width(), 150.0), &[(&pc, TEXT), (&rb, ACCENT)]);
                    });
                });
            });
        });
        // a tile per Remote Desktop account (open or not), each other Remote Desktop window, then each Roblox window here
        let mut tiles: Vec<Tile> = accounts.iter().map(|n| Tile {
            name: n.clone(), note: "Remote Desktop",
            win: self.wins.iter().find(|w| w.kind == Kind::Rdp(n.clone())).cloned(),
            user: Some(n.clone()), rdp: Some(n.clone()),
        }).collect();
        for w in &self.wins {
            if let Kind::Rdp(label) = &w.kind && !accounts.contains(label) {
                tiles.push(Tile { name: label.clone(), note: "Remote Desktop", win: Some(w.clone()), user: Some(label.clone()), rdp: None });
            }
        }
        for w in self.wins.iter().filter(|w| w.kind == Kind::Roblox) {
            let user = e.clients.iter().find(|c| c.pid == w.pid).map(|c| c.user.clone());
            tiles.push(Tile { name: user.clone().unwrap_or("Roblox".into()), note: "Roblox on this PC", win: Some(w.clone()), user, rdp: None });
        }
        ui.horizontal(|ui| {
            ui.label(semi("Live", 17.0).color(TEXT));
            let live = tiles.iter().filter(|t| t.win.is_some()).count();
            chip(ui, &format!("{live} open"), Some(if live > 0 { GOOD } else { OFF }));
            ui.label(small("Click a picture to bring that window to the front"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let id = ui.scope_id().with("tile-size");
                segmented(ui, id, &mut self.tile_cols, &[(4, "Small"), (3, "Medium"), (2, "Large")]);
            });
        });
        ui.add_space(10.0);
        if tiles.is_empty() {
            card(ui, "", "", |ui| {
                Frame::new().inner_margin(Margin::same(28)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(semi("Nothing Open Yet", 16.0).color(TEXT));
                        ui.label(small("Open Roblox, or make a Remote Desktop account on the Remote page, and it shows up here live."));
                    });
                });
            });
            return;
        }
        let gap = 14.0;
        let cols = self.tile_cols.min(((ui.available_width() + gap) / 260.0).max(1.0) as usize).max(1);
        let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
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
    }

    /// One live tile. Returns what was clicked: ("open", account) or ("trim", user).
    fn tile(&mut self, ui: &mut Ui, e: &Engine, t: &Tile, w: f32) -> Option<(&'static str, String)> {
        let mut act = None;
        let pids: Vec<u32> = t.user.as_ref().map(|u| e.clients.iter().filter(|c| c.user.eq_ignore_ascii_case(u)).map(|c| c.pid).collect()).unwrap_or_default();
        let ram: u64 = e.clients.iter().filter(|c| pids.contains(&c.pid)).map(|c| c.ram).sum();
        let cpu: f32 = pids.iter().filter_map(|p| e.cpu_pct.get(p)).sum::<f32>().max(0.0) + 0.0;
        Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE.gamma_multiply(0.6))).corner_radius(CornerRadius::same(14)).inner_margin(Margin::same(8)).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(w - 18.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w - 18.0, (w - 18.0) * 9.0 / 16.0), Sense::click());
            ui.painter().rect_filled(rect, 9.0, Color32::from_rgb(4, 4, 5));
            match &t.win {
                Some(win) if !win.minimized => {
                    let ppp = ui.ctx().pixels_per_point();
                    let px = |r: egui::Rect| [(r.left() * ppp) as i32, (r.top() * ppp) as i32, (r.right() * ppp) as i32, (r.bottom() * ppp) as i32];
                    if !self.thumbs.show(win.hwnd, px(rect.shrink(1.0)), px(ui.clip_rect())) {
                        centered_note(ui, rect, "Live Preview", "Shows here on Windows");
                    }
                }
                Some(_) => centered_note(ui, rect, "Minimized", "Click to show it. Minimized Remote Desktops stop drawing"),
                None => centered_note(ui, rect, "Not Open", "Open it to see it live"),
            }
            if let Some(win) = &t.win {
                let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
                ui.painter().rect_stroke(rect, 9.0, Stroke::new(1.5, LINE.lerp_to_gamma(ACCENT, hover)), egui::StrokeKind::Inside);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    crate::preview::focus(win.hwnd);
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                avatar(ui, &t.name, 30.0, t.win.is_some());
                ui.vertical(|ui| {
                    ui.set_max_width(w - 150.0);   // room for the button, never under it
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(semi(&t.name, 14.5).color(TEXT)).truncate());
                    ui.add(egui::Label::new(small(t.note)).truncate());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(n) = &t.rdp && t.win.is_none() {
                        if primary(ui, "Open").clicked() {
                            act = Some(("open", n.clone()));
                        }
                    } else if let Some(u) = &t.user && !pids.is_empty() && action(ui, "Trim", false).on_hover_text("Frees the RAM this Roblox isn't using").clicked() {
                        act = Some(("trim", u.clone()));
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if pids.is_empty() {
                    chip(ui, "No Roblox", None);
                } else {
                    chip(ui, &gb(ram), Some(ACCENT));
                    chip(ui, &format!("{cpu:.0}% CPU"), None);
                    if pids.len() > 1 {
                        chip(ui, &format!("{} clients", pids.len()), None);
                    }
                }
            });
            ui.add_space(2.0);
        }));
        act
    }

    /// Every Windows account with Roblox, as cards: who's running, what they use, which setup they're on.
    fn accounts_page(&mut self, ui: &mut Ui, e: &Engine) {
        if e.profiles.is_empty() {
            card(ui, "", "", |ui| {
                Frame::new().inner_margin(Margin::same(28)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(semi("No Account With Roblox Yet", 16.0).color(TEXT));
                        ui.label(small("Install Roblox on a Windows account and it shows up here."));
                    });
                });
            });
            return;
        }
        let gap = 14.0;
        let cols = ((ui.available_width() + gap) / 300.0).clamp(1.0, 4.0) as usize;
        let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
        for row in e.profiles.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for p in row {
                    let a = e.config.account(&p.sid);
                    let clients: Vec<_> = e.clients.iter().filter(|c| c.sid == p.sid).collect();
                    let ram: u64 = clients.iter().map(|c| c.ram).sum();
                    let r = Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE.gamma_multiply(0.6))).corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::same(16)).show(ui, |ui| ui.vertical(|ui| {
                            ui.set_width(w - 34.0);
                            ui.horizontal(|ui| {
                                avatar(ui, &p.name, 40.0, !clients.is_empty());
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(egui::Label::new(semi(&p.name, 16.0).color(TEXT)).truncate());
                                    ui.label(small(if !a.on { "Potato is off".to_string() } else if clients.is_empty() { "Roblox closed".into() } else { format!("{} running", clients.len()) }));
                                });
                            });
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if !clients.is_empty() {
                                    chip(ui, &gb(ram), Some(ACCENT));
                                }
                                chip(ui, Preset::of(&a).map_or("Custom", Preset::label), None);
                                chip(ui, &a.plan.fps_cap.map_or("No FPS cap".to_string(), |f| format!("{f} FPS")), None);
                            });
                        }));
                    let resp = ui.interact(r.response.rect, ui.scope_id().with(("account-card", &p.sid)), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &p.name));
                    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
                    ui.painter().rect_stroke(r.response.rect, 14.0, Stroke::new(1.0, ACCENT.gamma_multiply(hover)), egui::StrokeKind::Inside);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.page = Page::Account(p.sid.clone());
                    }
                }
            });
            ui.add_space(gap);
        }
    }

    fn account_page(&mut self, ui: &mut Ui, e: &mut Engine, sid: &str) {
        if ui.add(egui::Button::new(RichText::new("‹  All Accounts").color(DIM)).frame(false)).clicked() {
            self.page = Page::Accounts;
        }
        ui.add_space(6.0);
        let mut a = e.config.account(sid);
        card(ui, "Setup", Preset::of(&a).map_or("Custom: changed by hand", Preset::note), |ui| {
            Frame::new().inner_margin(Margin { left: 18, right: 18, top: 6, bottom: 12 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut pick = Preset::of(&a);
                    let opts: Vec<(Option<Preset>, &str)> = Preset::ALL.iter().map(|p| (Some(*p), p.label())).collect();
                    if segmented(ui, ui.scope_id().with("preset"), &mut pick, &opts) && let Some(p) = pick {
                        p.apply(&mut a);
                        self.fps_text.remove(sid);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut on = a.on;
                        ui.checkbox(&mut on, "Use Potato On This Account").on_hover_text("Off leaves this account's Roblox alone");
                        a.on = on;
                    });
                });
            });
        });
        ui.add_enabled_ui(a.on, |ui| {
            columns(ui, |cols| {
                card(col(cols, 0), "Game", "", |ui| {
                    row(ui, "FPS Cap", "Type a number, or leave it empty for no cap", false, |ui, id| {
                        let text = self.fps_text.entry(sid.to_string()).or_insert_with(|| a.plan.fps_cap.map_or(String::new(), |f| f.to_string()));
                        ui.label(RichText::new("FPS").color(FAINT));
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(64.0).hint_text("No Cap").align(egui::Align2::CENTER_CENTER)).labelled_by(id);
                        if r.changed() {
                            text.retain(|c| c.is_ascii_digit());   // "60fps", "60 FPS" and "60" all work
                            text.truncate(3);
                            a.plan.fps_cap = text.parse::<u32>().ok().filter(|f| *f > 0);
                        }
                    });
                    switch_row(ui, "Lowest Graphics", "Roblox's own Graphics Quality, all the way down", false, &mut a.plan.lowest_graphics);
                    switch_row(ui, "Ultra Low", "Lowest textures, no grass, blocky shapes, gray sky, frozen lighting", false, &mut a.plan.ultra_low);
                    switch_row(ui, "Open Maximized", "Fills the screen and keeps the title bar", false, &mut a.plan.maximized);
                    switch_row(ui, "Mute", "Silences this account's Roblox", true, &mut a.mute);
                });
                card(col(cols, 1), "CPU", "", |ui| {
                    row(ui, "Priority", "Lower keeps your PC smooth while Roblox runs", false, |ui, id| {
                        segmented(ui, id, &mut a.priority, &Priority::ALL.map(|p| (p, p.label())));
                    });
                    row(ui, "CPU Limit", "Most of the PC's CPU one client may use. Empty for no limit", false, |ui, id| {
                        number_box(ui, id, &mut a.cpu_limit, 100, "%", "None");
                    });
                    switch_row(ui, "Efficiency Mode", "Windows 11 runs it slower to save CPU and heat. Can lower FPS: best for alts", true, &mut a.efficiency);
                });
                card(col(cols, 1), "Clients", "", |ui| {
                    let mut one = a.max_clients > 0;
                    if switch_row(ui, "One Client Only", "If a second Roblox opens on this account, the older one closes", false, &mut one) {
                        a.max_clients = one as u32;
                    }
                    row(ui, "Restart Memory Hogs", "A client using more than this restarts into the same server. Empty for never", false, |ui, id| {
                        number_box(ui, id, &mut a.ram_limit_mb, 99_999, "MB", "Never");
                    });
                    switch_row(ui, "Rejoin After A Crash", "Reopens a client that closed by itself in the same server. Leave off if a macro rejoins for you", true, &mut a.rejoin_crashed);
                });
            });
        });
        if e.profiles.len() > 1 && action(ui, "Use These Settings For Every Account", false).clicked() {
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

    /// The guides as a checklist with one button each, then the whole-PC settings.
    fn performance_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        use crate::power::Item;
        let power = *self.power.lock().unwrap();
        let busy = *self.busy.lock().unwrap();
        let cores = e.cores as u32;
        let rdp_patch = e.rdp.lock().unwrap().patch;
        // (done, title, note, what its button does)
        enum Fix { Power(Item), Trim, HelperCores, Patch, Accounts }
        let p = power.unwrap_or_default();
        let capped = !e.profiles.is_empty() && e.profiles.iter().all(|pr| { let a = e.config.account(&pr.sid); !a.on || a.plan.fps_cap.is_some() });
        let items = [
            (capped, "Every Account Has An FPS Cap", "The guides cap Roblox's frame rate, 30 for alts", Fix::Accounts),
            (e.config.trim_minutes > 0, "Trim Roblox RAM On A Timer", "Moves memory Roblox isn't using to the page file every few minutes", Fix::Trim),
            (e.config.helper_cores > 0, "Remote Desktops On Their Own Cores", "The guide's core affinity: mstsc and macros apart from Roblox", Fix::HelperCores),
            (p.high_performance, "High Performance Power Plan", "No slowing down to save power. Undo goes back to Balanced", Fix::Power(Item::HighPerformance)),
            (p.boost_off, "CPU Boost Off", "Cores stay at base speed: less heat and fewer CPU spikes with many clients", Fix::Power(Item::BoostOff)),
            (p.fast_startup_off, "Fast Startup Off", "A real restart every time you turn the PC off, so it starts clean", Fix::Power(Item::FastStartupOff)),
            (rdp_patch == Patch::Patched, "Every Remote Desktop On At Once", "Patches Windows so more than one account can be on", Fix::Patch),
        ];
        let done = items.iter().filter(|i| i.0).count();
        let mut fix_all = false;
        let mut run: Vec<(&Fix, bool)> = Vec::new();
        card(ui, "Guide Checklist", &format!("{done} of {} done", items.len()), |ui| {
            Frame::new().inner_margin(Margin { left: 18, right: 18, top: 2, bottom: 6 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - 190.0, 6.0), Sense::hover());
                    ui.painter().rect_filled(r, 3.0, LINE);
                    let share = ui.ctx().animate_value_with_time(egui::Id::unique("guide-done"), done as f32 / items.len() as f32, dur(0.4));
                    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * share, 6.0)), 3.0, GOOD);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let left = items.iter().filter(|i| !i.0 && !matches!(i.3, Fix::Patch | Fix::Accounts)).count();
                        ui.add_enabled_ui(left > 0 && busy.is_none() && power.is_some(), |ui| {
                            fix_all = primary(ui, "Apply Recommended").on_hover_text("Turns on every item below except the patch and FPS caps, which you pick yourself").clicked();
                        });
                    });
                });
            });
            for (i, (ok, title, note, fix)) in items.iter().enumerate() {
                let r = Frame::new().inner_margin(Margin::symmetric(18, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        check(ui, *ok);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(*title).size(14.0).color(TEXT));
                            ui.label(small(*note));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = match fix {
                                Fix::Accounts => "Accounts",
                                Fix::Patch if *ok => return,
                                Fix::Patch => "Patch",
                                _ if *ok => "Undo",
                                _ => "Apply",
                            };
                            let waiting = matches!(fix, Fix::Power(_)) && power.is_none();
                            ui.add_enabled_ui(busy.is_none() && !waiting, |ui| {
                                if action(ui, label, false).clicked() {
                                    run.push((fix, !*ok));
                                }
                            });
                        });
                    });
                });
                if i + 1 < items.len() {
                    let rect = r.response.rect;
                    ui.painter().hline(rect.x_range().shrink(18.0), rect.bottom(), Stroke::new(1.0, LINE));
                }
            }
        });
        if fix_all {
            run = items.iter().filter(|i| !i.0 && !matches!(i.3, Fix::Patch | Fix::Accounts)).map(|i| (&i.3, true)).collect();
        }
        let power_items: Vec<(Item, bool)> = run.iter().filter_map(|(f, on)| if let Fix::Power(it) = f { Some((*it, *on)) } else { None }).collect();
        for (f, on) in &run {
            match f {
                Fix::Trim => e.config.trim_minutes = if *on { 5 } else { 0 },
                Fix::HelperCores => e.config.helper_cores = if *on { 2.min(cores.saturating_sub(2)) } else { 0 },
                Fix::Patch => engine::refresh_rdp(e.rdp.clone(), Some("patch"), crate::rdp::patch),
                Fix::Accounts => self.page = Page::Accounts,
                Fix::Power(_) => {}
            }
        }
        if !power_items.is_empty() {
            let power = self.power.clone();
            self.run_action("power", ui.ctx(), move |e| {
                for (item, on) in power_items {
                    if let Err(m) = crate::power::apply(item, on) {
                        e.note = m;
                    }
                }
                *power.lock().unwrap() = Some(crate::power::read());
            });
        }

        let trim_note = match e.last_trim_freed {
            Some(f) => format!("Last trim {} min ago, freed {}. Roblox takes some back as it plays", e.last_trim.elapsed().as_secs() / 60, gb(f)),
            None => "Moves memory Roblox isn't using to the page file".into(),
        };
        let off_or = |n: u32, f: &dyn Fn(u32) -> String| if n == 0 { "Off".to_string() } else { f(n) };
        let c = &mut e.config;
        let shutdown_text = &mut self.shutdown_text;
        columns(ui, |cols| {
            card(col(cols, 0), "Memory", "", |ui| {
                let list = [0, 1, 2, 5, 10, 15, 30].map(|n| (n, off_or(n, &|n| format!("Every {n} Min"))));
                row(ui, "Trim Roblox RAM", &trim_note, false, |ui, id| { dropdown(ui, "trim", id, &mut c.trim_minutes, &list); });
                row(ui, "Clean Memory", "Empties Windows' standby memory when RAM use goes over this. Empty for never", true, |ui, id| {
                    ui.label(RichText::new("%").color(FAINT));
                    let mut text = if c.clean_above == 0 { String::new() } else { c.clean_above.to_string() };
                    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(56.0).hint_text("Off").align(egui::Align2::CENTER_CENTER)).labelled_by(id).changed() {
                        text.retain(|ch| ch.is_ascii_digit());
                        c.clean_above = text.parse::<u32>().unwrap_or(0).min(99);
                    }
                });
            });
            card(col(cols, 0), "Network", "", |ui| {
                row(ui, "Upload Cap", "Limits how fast all of Roblox can upload, for slow internet. Empty for no cap", true, |ui, id| {
                    number_box(ui, id, &mut c.upload_kbps, 1_000_000, "kbit/s", "None");
                });
            });
            card(col(cols, 1), "CPU Cores", &format!("{cores} on this PC"), |ui| {
                let list: Vec<(u32, String)> = [0, 1, 2, 3, 4, 6, 8].into_iter().filter(|n| *n <= cores).map(|n| (n, off_or(n, &|n| format!("{n} Each")))).collect();
                row(ui, "Own CPU Cores", "Each Roblox client gets cores of its own", false, |ui, id| { dropdown(ui, "cores", id, &mut c.cores_per_client, &list); });
                let list: Vec<(u32, String)> = [0, 2, 3].into_iter().filter(|n| *n < cores).map(|n| (n, off_or(n, &|n| format!("Last {n} Cores")))).collect();
                row(ui, "Cores For Remote Desktop And Macros", "Remote Desktop windows and AutoHotkey run here, Roblox stays off them", true,
                    |ui, id| { dropdown(ui, "helper-cores", id, &mut c.helper_cores, &list); });
            });
            card(col(cols, 1), "Clients", "", |ui| {
                let list = [0, 1, 2, 4, 6, 12, 24].map(|n| (n, off_or(n, &|n| format!("Every {n} {}", if n == 1 { "Hour" } else { "Hours" }))));
                row(ui, "Restart Clients", "Closes them and opens each in the same server", false, |ui, id| { dropdown(ui, "restart", id, &mut c.restart_hours, &list); });
                row(ui, "Turn PC Off At", "Every day at this time, like 6:30, 0630 or 6pm. Empty for never", true, |ui, id| {
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

    /// Notifications, the Discord report, Windows' own tools and Potato itself.
    fn settings_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let busy = *self.busy.lock().unwrap();
        let (mut test, mut point, mut debloat, mut shutup, mut restore) = (false, false, false, false, false);
        let armed = self.confirm_debloat.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        let c = &mut e.config;
        let webhook_text = &mut self.webhook_text;
        columns(ui, |cols| {
            card(col(cols, 0), "Notifications", "", |ui| {
                switch_row(ui, "Notifications", "A Windows notification each time a trim or clean frees RAM", true, &mut c.notify);
            });
            card(col(cols, 0), "Discord Report", "", |ui| {
                stack_row(ui, "Webhook", "Discord channel settings, Integrations, Webhooks, Copy Webhook URL", false, |ui, id| {
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
                        ui.label(RichText::new("Not a Discord webhook").size(12.5).color(BAD));
                    }
                });
                switch_row(ui, "Screenshots", "Adds a picture of each Remote Desktop's Roblox. A minimized or disconnected one can't be captured", false, &mut c.discord.screenshots);
                row(ui, "Test It", "Sends a report now. Potato sends one by itself on every hour", true, |ui, id| {
                    ui.add_enabled_ui(!c.discord.webhook.is_empty(), |ui| test = action(ui, "Send Report", false).labelled_by(id).clicked());
                });
            });
            card(col(cols, 1), "Windows Tools", "Risky: make a restore point first", |ui| {
                row(ui, "Make A Restore Point", "Lets System Restore undo the tools below. Windows makes one a day at most", false, |ui, id| {
                    ui.add_enabled_ui(busy.is_none(), |ui| point = action(ui, if busy == Some("point") { "Making" } else { "Make" }, busy == Some("point")).labelled_by(id).clicked());
                });
                row(ui, "Remove Windows Bloat", "Chris Titus Tech's utility. Wrong picks can break Windows Update or the Store", false, |ui, id| {
                    debloat = action(ui, if armed { "Click Again" } else { "Open" }, false).labelled_by(id).clicked();
                });
                row(ui, "Turn Off Windows Tracking", "O&O ShutUp10++ download. Some switches turn off Windows Update or Defender", true, |ui, id| {
                    shutup = action(ui, "Open", false).labelled_by(id).clicked();
                });
            });
            card(col(cols, 1), "Potato", "", |ui| {
                row(ui, "Restore Original Settings", "Puts back every Roblox file Potato changed and turns it off", false, |ui, id| {
                    restore = action(ui, "Restore", false).labelled_by(id).clicked();
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
            e.note = "Sending A Report To Discord".into();
        }
        if armed {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if point {
            self.run_action("point", ui.ctx(), |e| e.restore_point());
        }
        if debloat {
            self.confirm_debloat = if armed {
                if !crate::system::console("irm https://christitus.com/win | iex") {
                    e.note = "Couldn't Open The Windows Utility".into();
                }
                None
            } else {
                Some(Instant::now())
            };
        }
        if shutup {
            crate::system::launch("https://www.oo-software.com/en/shutup10");
        }
        if restore {
            e.restore();
            self.fps_text.clear();
        }
    }

    /// Remote Desktop alts: how many this PC can run, the patch, each account as a card, and making new ones.
    fn rdp_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let r = e.rdp.lock().unwrap().clone();
        let busy = r.busy;
        let (mut patch, mut undo, mut add, mut open) = (false, false, false, None);
        let (mut startup, mut shortcut, mut wide) = (None, None, None);
        let ram_gb = (e.memory.1 as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0;   // what Task Manager shows: 7.99 is 8.0 GB
        let (by_cpu, by_ram) = crate::rdp::capacity(e.physical_cores, e.cores, ram_gb);
        let can = by_cpu.min(by_ram);
        let open_now = self.wins.iter().filter(|w| matches!(w.kind, Kind::Rdp(_))).count();
        columns(ui, |cols| {
            card(col(cols, 0), "This PC", "The guide's test", |ui| {
                Frame::new().inner_margin(Margin { left: 18, right: 18, top: 4, bottom: 12 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(semi(&format!("{open_now}"), 30.0).color(TEXT));
                        ui.label(semi(&format!("/ about {can}"), 18.0).color(if can == 0 { BAD } else { DIM }));
                        ui.label(small("open of what it can run"));
                    });
                    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                    ui.painter().rect_filled(bar, 3.0, LINE);
                    let share = if can == 0 { 1.0 } else { open_now as f32 / can as f32 };
                    ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, vec2(bar.width() * share.min(1.0), 6.0)), 3.0, if share > 1.0 { BAD } else { ACCENT });
                    ui.add_space(10.0);
                    let limit = if by_cpu < by_ram { "CPU" } else { "RAM" };
                    ui.label(small(format!("{by_cpu} by CPU ({} cores, {} logical) · {by_ram} by RAM ({ram_gb:.0} GB, 4 GB each after Windows) · {limit} is the limit", e.physical_cores, e.cores)));
                    let ghz = e.base_mhz as f32 / 1000.0;
                    if e.base_mhz > 0 {
                        ui.label(small(if ghz >= 2.5 { format!("{ghz:.1} GHz: over the guide's 2.5 GHz, one per 2-3 logical processors") } else { format!("{ghz:.1} GHz: under the guide's 2.5 GHz, expect fewer and slower") }));
                    }
                });
            });
            card(col(cols, 1), "All On At Once", "", |ui| {
                let (note, can_patch) = match r.patch {
                    Patch::Patched => ("On. Potato warns you if a Windows update undoes it", false),
                    Patch::Unpatched => ("Windows Pro allows one at a time. Open windows disconnect for a moment; Roblox keeps running", true),
                    Patch::Unsupported => ("This Windows version's file doesn't match the patch yet. Only one can be on", false),
                    Patch::Unknown => ("Checking Windows' Remote Desktop file", false),
                };
                row(ui, "Let Every Account On", note, !r.can_undo, |ui, id| {
                    if can_patch {
                        ui.add_enabled_ui(busy.is_none(), |ui| patch = primary(ui, if busy == Some("patch") { "Patching" } else { "Patch" }).labelled_by(id).clicked());
                    } else if r.patch == Patch::Patched {
                        chip(ui, "On", Some(GOOD));
                    }
                });
                if r.can_undo {
                    row(ui, "Undo The Patch", "Puts Windows' own file back. Only one Remote Desktop can be on after", true, |ui, id| {
                        ui.add_enabled_ui(busy.is_none(), |ui| undo = action(ui, if busy == Some("undo") { "Undoing" } else { "Undo" }, busy == Some("undo")).labelled_by(id).clicked());
                    });
                }
            });
            card(col(cols, 1), "CPU", "", |ui| {
                row(ui, "Remote Desktop CPU Limit", "Caps each Remote Desktop window. Too low and sessions freeze or disconnect", true, |ui, id| {
                    number_box(ui, id, &mut e.config.rdp_cpu_limit, 100, "%", "None");
                });
            });
        });
        ui.label(semi("Accounts", 17.0).color(TEXT));
        ui.add_space(8.0);
        let gap = 14.0;
        let cols = ((ui.available_width() + gap) / 380.0).clamp(1.0, 3.0) as usize;
        let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
        for chunk in r.accounts.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for name in chunk {
                    let live = self.wins.iter().any(|win| win.kind == Kind::Rdp(name.clone()));
                    let is_wide = e.config.rdp_wide.contains(name);
                    Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE.gamma_multiply(0.6))).corner_radius(CornerRadius::same(14)).inner_margin(Margin::same(16))
                        .show(ui, |ui| ui.vertical(|ui| {
                            ui.set_width(w - 34.0);
                            let id = ui.horizontal(|ui| {
                                avatar(ui, name, 36.0, live);
                                let id = ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    let id = ui.label(semi(name, 16.0).color(TEXT)).id;
                                    ui.label(small(format!("{} · {}", crate::rdp::address(name), if is_wide { "1800x768" } else { "1366x768" })));
                                    id
                                }).inner;
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| chip(ui, if live { "Open" } else { "Closed" }, Some(if live { GOOD } else { OFF })));
                                id
                            }).inner;
                            ui.add_space(10.0);
                            ui.label(small("No sound, no drives, no wallpaper, Windows keys stay on this PC"));
                            ui.add_space(10.0);
                            ui.horizontal_wrapped(|ui| {
                                if primary(ui, "Open").labelled_by(id).clicked() {
                                    open = Some(name.clone());
                                }
                                if action(ui, "Desktop Shortcut", false).on_hover_text("Puts a shortcut on your desktop that opens this account with all these settings, even without Potato").clicked() {
                                    shortcut = Some(name.clone());
                                }
                            });
                            ui.add_space(8.0);
                            ui.horizontal_wrapped(|ui| {
                                let mut wd = is_wide;
                                if segmented(ui, id.with("size"), &mut wd, &[(false, "1366x768"), (true, "1800x768 Boosting")]) {
                                    wide = Some((name.clone(), wd));
                                }
                                let mut on = e.config.rdp_startup.contains(name);
                                if ui.checkbox(&mut on, "Open At Sign In").on_hover_text("Opens this account when you sign in to Windows").changed() {
                                    startup = Some((name.clone(), on));
                                }
                            });
                        }));
                }
            });
            ui.add_space(gap);
        }
        let ok_name = crate::rdp::valid_name(&self.new_name);
        card(ui, "New Account", "A Windows user in Remote Desktop Users. For one you already have, type its password to save the login", |ui| {
            Frame::new().inner_margin(Margin { left: 18, right: 18, top: 6, bottom: 14 }).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let name_id = ui.label(RichText::new("Name").size(13.0).color(FAINT)).id;
                    let r = ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(200.0).hint_text("Name").char_limit(20)).labelled_by(name_id);
                    let pass_id = ui.label(RichText::new("Password").size(13.0).color(FAINT)).id;
                    ui.add(egui::TextEdit::singleline(&mut self.new_pass).desired_width(200.0).hint_text("Password").password(true)).labelled_by(pass_id);
                    let ready = ok_name && !self.new_pass.is_empty() && busy.is_none();
                    ui.add_enabled_ui(ready, |ui| add = primary(ui, if busy == Some("add") { "Making" } else { "Make It" }).clicked());
                    if !self.new_name.is_empty() && !ok_name && !r.has_focus() {
                        ui.label(RichText::new("Can't Use / \\ [ ] : ; | = , + * ? < > @").size(12.5).color(BAD));
                    }
                });
            });
        });
        if patch {
            engine::refresh_rdp(e.rdp.clone(), Some("patch"), crate::rdp::patch);
        }
        if undo {
            e.config.rdp_patched = false;   // you undid it, not Windows: no warning
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
                e.note = "Couldn't Set Up Opening At Sign In".into();
            }
        }
        if let Some(name) = open && let Err(m) = crate::rdp::open(&name, e.config.rdp_wide.contains(&name)) {
            e.note = m;
        }
    }
}
