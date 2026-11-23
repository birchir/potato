//! The window: the Overview (live previews of every Roblox and Remote Desktop window), then each account's
//! settings, Remote Desktop and Performance down the left, like Windows Settings. All the work happens in
//! engine.rs; closing the window hides it to the tray.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, Frame, Margin, RichText, Sense, Stroke, Ui};

use crate::config::Priority;
use crate::rdp::Patch;
use crate::engine::{self, Engine, State, UpdateState, gb};

const BG: Color32 = Color32::from_rgb(10, 10, 10);
const NAV: Color32 = Color32::from_rgb(0, 0, 0);
const CARD: Color32 = Color32::from_rgb(21, 21, 21);
const CARD_HI: Color32 = Color32::from_rgb(30, 30, 30);
const LINE: Color32 = Color32::from_rgb(38, 38, 38);
const TEXT: Color32 = Color32::from_rgb(242, 242, 242);
const DIM: Color32 = Color32::from_rgb(180, 180, 180);
const FAINT: Color32 = Color32::from_rgb(142, 142, 142);   // 5.6:1 on CARD
const OFF: Color32 = Color32::from_rgb(64, 64, 64);
const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
const BAD: Color32 = Color32::from_rgb(248, 113, 113);

pub fn run() -> eframe::Result {
    crate::system::enable_privileges();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Potato")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default())
            .with_inner_size([1000.0, 760.0]).with_min_inner_size([820.0, 560.0]),
        centered: true,   // Windows otherwise opens it wherever it last cascaded windows, often off to the side
        ..Default::default()
    };
    eframe::run_native("Potato", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// Windows' "Animation effects" setting: off turns every animation here off (reduced motion).
static MOTION: AtomicBool = AtomicBool::new(true);

/// An animation's length, or none when Windows has animations off. Kept short: snappy, under the 400 ms
/// people notice as waiting.
fn dur(seconds: f32) -> f32 {
    if MOTION.load(Ordering::Relaxed) { seconds } else { 0.0 }
}

#[derive(Clone, PartialEq)]
enum Page {
    Overview,
    Account(String),
    WholePc,
    RemoteDesktop,
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
    /// the header action running in the background, if any (so the window never waits on it)
    busy: Arc<Mutex<Option<&'static str>>>,
    /// the last result shown as a toast, and when it appeared (the window's own clock, so animation stays in step)
    toast: Option<(String, f64)>,
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
        App {
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
            thumbs: crate::preview::Thumbs::new(hwnd),
            wins: Vec::new(),
            wins_at: None,
            hwnd,
        }
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
        egui::Panel::left("nav").exact_size(250.0).resizable(false).frame(Frame::new().fill(NAV).inner_margin(Margin::same(14)))
            .show(ui, |ui| self.nav(ui, &e));
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(32, 24))).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("page").show(ui, |ui| {
                // setting rows don't stretch across a wide screen; the live pictures may
                ui.set_width(ui.available_width().min(if self.page == Page::Overview { 1500.0 } else { 860.0 }));
                self.banner(ui, &e);
                self.header(ui, &mut e);
                match self.page.clone() {
                    Page::Overview => self.overview_page(ui, &mut e),
                    Page::Account(sid) => self.account_page(ui, &mut e, &sid),
                    Page::WholePc => self.pc_page(ui, &mut e),
                    Page::RemoteDesktop => self.rdp_page(ui, &mut e),
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

impl App {
    fn nav(&mut self, ui: &mut Ui, e: &Engine) {
        ui.horizontal(|ui| {
            logo(ui, 1.6);
            ui.label(semi("Potato", 15.0).color(TEXT));
        });
        ui.add_space(14.0);
        let (used, total) = e.memory;
        Frame::new().fill(CARD).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // counts to the new value instead of jumping (follow through)
            let shown = ui.ctx().animate_value_with_time(egui::Id::unique("ram-used"), used as f32, dur(0.6));
            ui.label(semi(&gb(shown as u64), 22.0).color(TEXT));
            ui.label(RichText::new(format!("of {} RAM in use", gb(total))).size(12.0).color(FAINT));
            let share = if total > 0 { used as f32 / total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(share).desired_height(4.0).fill(if share > 0.85 { BAD } else { TEXT }));
            let roblox: u64 = e.clients.iter().map(|c| c.ram).sum();
            ui.label(RichText::new(format!("Roblox {} · Saved {}", gb(roblox), gb(e.freed))).size(12.0).color(FAINT));
        });
        ui.add_space(14.0);
        let mut marker = None;
        let open = e.clients.len();
        let r = nav_item(ui, "Overview", &if open == 0 { String::new() } else { format!("{open} Live") }, None, self.page == Page::Overview);
        if self.page == Page::Overview {
            marker = Some(r.rect);
        }
        if r.clicked() {
            self.page = Page::Overview;
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Accounts").size(12.0).color(FAINT));
        for p in &e.profiles {
            let ram: u64 = e.clients.iter().filter(|c| c.sid == p.sid).map(|c| c.ram).sum();
            let running = e.clients.iter().any(|c| c.sid == p.sid);
            let right = if !e.config.account(&p.sid).on { "Off".to_string() } else if running { gb(ram) } else { "Closed".into() };
            let picked = self.page == Page::Account(p.sid.clone());
            let r = nav_item(ui, &p.name, &right, Some(if running { GOOD } else { OFF }), picked);
            if picked {
                marker = Some(r.rect);
            }
            if r.clicked() {
                self.page = Page::Account(p.sid.clone());
            }
        }
        if e.profiles.is_empty() {
            ui.label(RichText::new("No account with Roblox yet").color(FAINT));
        }
        ui.add_space(14.0);
        ui.label(RichText::new("Potato").size(12.0).color(FAINT));
        let r = nav_item(ui, "Performance", "", None, self.page == Page::WholePc);
        if self.page == Page::WholePc {
            marker = Some(r.rect);
        }
        if r.clicked() {
            self.page = Page::WholePc;
        }
        let rdp = e.rdp.lock().unwrap().clone();
        let warn = rdp.reverted(e.config.rdp_patched);
        let right = match rdp.accounts.len() { 0 => String::new(), 1 => "1 Account".into(), n => format!("{n} Accounts") };
        let r = nav_item(ui, "Remote Desktop", &right, warn.then_some(BAD), self.page == Page::RemoteDesktop);
        if self.page == Page::RemoteDesktop {
            marker = Some(r.rect);
        }
        if r.clicked() {
            self.page = Page::RemoteDesktop;
        }
        // the marker slides from the old choice to the new one (post to post, slow in and out)
        if let Some(rect) = marker {
            let y = ui.ctx().animate_value_with_time(egui::Id::unique("nav-marker"), rect.top(), dur(0.2));
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), y + 11.0), egui::vec2(3.0, 18.0)), 2.0, TEXT);
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
        egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(125.0, -24.0 + (1.0 - come) * 18.0))
            .interactable(false).show(ctx, |ui| {
                ui.set_opacity(come.min(1.0) * go);
                Frame::new().fill(TEXT).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(18, 11)).show(ui, |ui| {
                    ui.add(egui::Label::new(semi(text, 13.5).color(NAV)).extend());   // one line, never squeezed by the window edge
                });
            });
        ctx.request_repaint();
    }

    /// Run a header action on its own thread, so the window answers at once (Doherty threshold).
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
        Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(semi(&title, 14.0).color(TEXT));
                    ui.label(RichText::new(note).size(12.5).color(FAINT));
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
                        if ui.button("Close").clicked() {
                            *e.update.lock().unwrap() = UpdateState::None;
                        }
                    }
                    UpdateState::None => {}
                });
            });
        });
        ui.add_space(18.0);
    }

    /// Windows undid the Remote Desktop patch (an update put its own file back): say so on every page, with the fix.
    fn rdp_banner(&mut self, ui: &mut Ui, e: &Engine) {
        let r = e.rdp.lock().unwrap().clone();
        if !r.reverted(e.config.rdp_patched) {
            return;
        }
        let fits = r.patch == Patch::Unpatched;
        let note = if fits { "Only one Remote Desktop can be on until it's patched again." } else { "Windows updated its Remote Desktop file and Potato's patch doesn't fit it yet. Only one Remote Desktop can be on." };
        Frame::new().fill(Color32::from_rgb(48, 20, 20)).stroke(Stroke::new(1.0, BAD)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
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
                    } else if self.page != Page::RemoteDesktop && ui.button("Open Remote Desktop").clicked() {
                        self.page = Page::RemoteDesktop;
                    }
                });
            });
        });
        ui.add_space(18.0);
    }

    fn header(&mut self, ui: &mut Ui, e: &mut Engine) {
        let (title, crumb) = match &self.page {
            Page::Account(sid) => {
                let name = e.profiles.iter().find(|p| &p.sid == sid).map_or("Account".to_string(), |p| p.name.clone());
                let running = match e.clients.iter().filter(|c| &c.sid == sid).count() {
                    0 => "Roblox Closed".to_string(),
                    1 => "Roblox Running".to_string(),
                    n => format!("{n} Clients Running"),
                };
                let state = match e.states.get(sid) {
                    Some(State::Applied) => "Settings Applied",
                    Some(State::Waiting) => "Applies When Roblox Restarts",
                    Some(State::Failed) => "Couldn't Write Settings",
                    None => "Potato Is Off",
                };
                (name, format!("{running} · {state}"))
            }
            Page::Overview => {
                let rdps = self.wins.iter().filter(|w| matches!(w.kind, crate::preview::Kind::Rdp(_))).count();
                let n = e.clients.len();
                ("Overview".to_string(), format!("{n} Roblox {} · {rdps} Remote {} Open", if n == 1 { "Client" } else { "Clients" }, if rdps == 1 { "Desktop" } else { "Desktops" }))
            }
            Page::WholePc => ("Performance".to_string(), "Settings for the whole PC, from the RDP optimization guides".to_string()),
            Page::RemoteDesktop => {
                let r = e.rdp.lock().unwrap();
                let n = r.accounts.len();
                let at_once = match r.patch {
                    Patch::Patched => "All On At Once",
                    Patch::Unpatched | Patch::Unsupported => "One At A Time",
                    Patch::Unknown => "Checking",
                };
                ("Remote Desktop".to_string(), format!("{n} {} · {at_once}", if n == 1 { "Account" } else { "Accounts" }))
            }
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(semi(&title, 28.0).color(TEXT));
                ui.label(RichText::new(crumb).size(13.0).color(FAINT));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = *self.busy.lock().unwrap();
                if action(ui, "Trim All Roblox", busy == Some("trim")).on_hover_text("Frees memory every Roblox client isn't using right now").clicked() {
                    self.run_action("trim", ui.ctx(), |e| e.trim_all(true));
                }
                let armed = self.confirm_restart.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                if action(ui, if armed { "Click Again To Restart" } else { "Restart All" }, busy == Some("restart"))
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
            });
        });
        ui.add_space(10.0);
    }

    fn account_page(&mut self, ui: &mut Ui, e: &mut Engine, sid: &str) {
        let mut a = e.config.account(sid);
        group(ui, "Potato", |ui| {
            switch_row(ui, "Use Potato On This Account", "Off leaves this account's Roblox alone", true, &mut a.on);
        });
        // most used first, and no group longer than four rows (Hick's and Miller's laws)
        ui.add_enabled_ui(a.on, |ui| {
            group(ui, "Game", |ui| {
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
                switch_row(ui, "Open Maximized", "Fills the screen and keeps the title bar", true, &mut a.plan.maximized);
            });
            group(ui, "CPU", |ui| {
                row(ui, "Priority", "Lower keeps your PC smooth while Roblox runs", false, |ui, id| {
                    dropdown(ui, "priority", id, &mut a.priority, &Priority::ALL.map(|p| (p, p.label().to_string())));
                });
                row(ui, "CPU Limit", "Most of the PC's CPU one client may use. Empty for no limit", false, |ui, id| {
                    number_box(ui, id, &mut a.cpu_limit, 100, "%", "None");
                });
                switch_row(ui, "Efficiency Mode", "Windows 11 runs it slower to save CPU and heat. Can lower FPS: best for alts", true, &mut a.efficiency);
            });
            group(ui, "Clients", |ui| {
                let mut one = a.max_clients > 0;
                if switch_row(ui, "One Client Only", "If a second Roblox opens on this account, the older one closes", false, &mut one) {
                    a.max_clients = one as u32;
                }
                row(ui, "Restart Memory Hogs", "A client using more than this restarts into the same server. Empty for never", false, |ui, id| {
                    number_box(ui, id, &mut a.ram_limit_mb, 99_999, "MB", "Never");
                });
                switch_row(ui, "Rejoin After A Crash", "Reopens a client that closed by itself in the same server. Leave off if a macro rejoins for you", false, &mut a.rejoin_crashed);
                switch_row(ui, "Mute", "Silences this account's Roblox", true, &mut a.mute);
            });
        });
        ui.add_space(16.0);
        if e.profiles.len() > 1 && ui.add(egui::Button::new("Use These Settings For Every Account").min_size(egui::vec2(0.0, 36.0))).clicked() {
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

    fn pc_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let cores = e.cores as u32;
        let trim_note = match e.last_trim_freed {
            Some(f) => format!("Last trim {} min ago, freed {}. Roblox takes some back as it plays", e.last_trim.elapsed().as_secs() / 60, crate::engine::gb(f)),
            None => "Moves memory Roblox isn't using to the page file".into(),
        };
        let off_or = |n: u32, f: &dyn Fn(u32) -> String| if n == 0 { "Off".to_string() } else { f(n) };
        let c = &mut e.config;
        group(ui, "Memory", |ui| {
            let list = [0, 1, 2, 5, 10, 15, 30].map(|n| (n, off_or(n, &|n| format!("Every {n} Min"))));
            row(ui, "Trim Roblox RAM", &trim_note, false, |ui, id| { dropdown(ui, "trim", id, &mut c.trim_minutes, &list); });
            row(ui, "Clean Memory", "Empties Windows' standby memory when RAM use goes over this. Empty for never", false, |ui, id| {
                ui.label(RichText::new("%").color(FAINT));
                let mut text = if c.clean_above == 0 { String::new() } else { c.clean_above.to_string() };
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(56.0).hint_text("Off").align(egui::Align2::CENTER_CENTER)).labelled_by(id).changed() {
                    text.retain(|ch| ch.is_ascii_digit());
                    c.clean_above = text.parse::<u32>().unwrap_or(0).min(99);
                }
            });
            switch_row(ui, "Notifications", "A Windows notification each time a trim or clean frees RAM", true, &mut c.notify);
        });
        group(ui, "CPU", |ui| {
            let list: Vec<(u32, String)> = [0, 1, 2, 3, 4, 6, 8].into_iter().filter(|n| *n <= cores).map(|n| (n, off_or(n, &|n| format!("{n} Each")))).collect();
            row(ui, "Own CPU Cores", &format!("Each client gets its own cores ({cores} on this PC)"), false, |ui, id| { dropdown(ui, "cores", id, &mut c.cores_per_client, &list); });
            let list: Vec<(u32, String)> = [0, 2, 3].into_iter().filter(|n| *n < cores).map(|n| (n, off_or(n, &|n| format!("Last {n} Cores")))).collect();
            row(ui, "Cores For Remote Desktop And Macros", "Remote Desktop windows and AutoHotkey run here, Roblox stays off them", true,
                |ui, id| { dropdown(ui, "helper-cores", id, &mut c.helper_cores, &list); });
        });
        group(ui, "Power", |ui| {
            switch_row(ui, "High Performance Power Plan", "Windows' High Performance plan: no slowing down to save power. Off goes back to Balanced", false, &mut c.high_performance);
            switch_row(ui, "Turn Off CPU Boost", "Keeps the CPU at its base speed: less heat and fewer CPU spikes with many clients", false, &mut c.cpu_boost_off);
            switch_row(ui, "Turn Off Fast Startup", "A real restart every time you turn the PC off, so it starts clean", true, &mut c.fast_startup_off);
        });
        group(ui, "Network", |ui| {
            row(ui, "Upload Cap", "Limits how fast all of Roblox can upload, for slow internet. Empty for no cap", true, |ui, id| {
                number_box(ui, id, &mut c.upload_kbps, 1_000_000, "kbit/s", "None");
            });
        });
        group(ui, "Clients", |ui| {
            let list = [0, 1, 2, 4, 6, 12, 24].map(|n| (n, off_or(n, &|n| format!("Every {n} {}", if n == 1 { "Hour" } else { "Hours" }))));
            row(ui, "Restart Clients", "Closes them and opens each in the same server", false, |ui, id| { dropdown(ui, "restart", id, &mut c.restart_hours, &list); });
            row(ui, "Turn PC Off At", "Every day at this time, like 6:30, 0630 or 6pm. Empty for never", true, |ui, id| {
                let text = self.shutdown_text.get_or_insert_with(|| c.shutdown_at.clone());
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
        let mut test = false;
        group(ui, "Discord Report", |ui| {
            row(ui, "Webhook", "Discord channel settings, Integrations, Webhooks, Copy Webhook URL", false, |ui, id| {
                let text = self.webhook_text.get_or_insert_with(|| c.discord.webhook.clone());
                let hide = !text.is_empty();   // the link is a password for the channel
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(260.0).hint_text("https://discord.com/api/webhooks/...").password(hide)).labelled_by(id);
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
                test = ui.add_enabled(!c.discord.webhook.is_empty(), egui::Button::new("Send Report")).labelled_by(id).clicked();
            });
        });
        if test {
            e.send_report_now();
            e.note = "Sending A Report To Discord".into();
        }
        let busy = *self.busy.lock().unwrap();
        let (mut point, mut debloat, mut shutup) = (false, false, false);
        let armed = self.confirm_debloat.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        group(ui, "Advanced (Risky)", |ui| {
            row(ui, "Make A Restore Point", "Lets System Restore undo the tools below. Windows makes one a day at most", false, |ui, id| {
                point = ui.add_enabled(busy.is_none(), egui::Button::new(if busy == Some("point") { "Making..." } else { "Make" })).labelled_by(id).clicked();
            });
            row(ui, "Remove Windows Bloat", "Chris Titus Tech's utility. Wrong picks can break Windows Update or the Store", false, |ui, id| {
                debloat = ui.button(if armed { "Click Again To Open" } else { "Open" }).labelled_by(id).clicked();
            });
            row(ui, "Turn Off Windows Tracking", "O&O ShutUp10++ download. Some switches turn off Windows Update or Defender", true, |ui, id| {
                shutup = ui.button("Open").labelled_by(id).clicked();
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
        let mut restore = false;
        group(ui, "Potato", |ui| {
            row(ui, "Restore Original Settings", "Puts back every Roblox file Potato changed and turns it off", false, |ui, id| {
                restore = ui.button("Restore").labelled_by(id).clicked();
            });
            row(ui, "Quit Potato", "Closing the window keeps Potato running in the tray", true, |ui, id| {
                if ui.button("Quit").labelled_by(id).clicked() {
                    std::process::exit(0);
                }
            });
        });
        if restore {
            e.restore();
            self.fps_text.clear();
        }
    }
}

fn semi(text: &str, size: f32) -> RichText {
    RichText::new(text).size(size).family(FontFamily::Name("semi".into()))
}

/// The pixel potato from assets/logo.txt, `px` points per pixel.
fn logo(ui: &mut Ui, px: f32) {
    let lines: Vec<&str> = include_str!("../assets/logo.txt").lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let colors: HashMap<char, Color32> = lines.iter().filter(|l| l.get(1..2) == Some("="))
        .filter_map(|l| Some((l.chars().next()?, Color32::from_hex(l.get(2..9)?).ok()?))).collect();
    let rows: Vec<&str> = lines.into_iter().filter(|l| l.get(1..2) != Some("=")).collect();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(rows[0].len() as f32 * px, rows.len() as f32 * px), Sense::hover());
    for (y, row) in rows.iter().enumerate() {
        for (x, k) in row.chars().enumerate() {
            let Some(&color) = colors.get(&k) else { continue };
            let min = rect.min + egui::vec2(x as f32 * px, y as f32 * px);
            ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::vec2(px, px)), 0.0, color);
        }
    }
}

/// One entry in the left column: an optional status dot, a name, and a note on the right. The chosen one
/// gets a lighter background (its marker is drawn by nav, so it can slide). Tall rows: easy to hit.
fn nav_item(ui: &mut Ui, name: &str, right: &str, dot: Option<Color32>, picked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
    let p = ui.painter();
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered() || picked, dur(0.12));
    if hover > 0.0 {
        p.rect_filled(rect, 6.0, if picked { CARD_HI } else { CARD.gamma_multiply(hover) });
    }
    let mut x = rect.left() + 14.0;
    if let Some(c) = dot {
        p.circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.5, c);
        x += 16.0;
    }
    p.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.0), TEXT);
    p.text(egui::pos2(rect.right() - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER, right, egui::FontId::proportional(12.5), FAINT);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, picked, name));
    resp
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
    /// Live pictures of every Remote Desktop and Roblox window, with what each one uses (RoAlt-style).
    fn overview_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        use crate::preview::Kind;
        let accounts = e.rdp.lock().unwrap().accounts.clone();
        if self.wins_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.wins = crate::preview::windows(&accounts);
            self.wins_at = Some(Instant::now());
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        // the numbers at a glance
        let (used, total) = e.memory;
        let roblox: u64 = e.clients.iter().map(|c| c.ram).sum();
        let cpu: f32 = e.cpu_pct.values().sum::<f32>().clamp(0.0, 100.0) + 0.0;   // + 0.0 turns -0 into 0
        ui.add_space(8.0);
        ui.columns(4, |cols| {
            stat(&mut cols[0], "RAM In Use", &gb(used), &format!("of {}", gb(total)), Some(if total > 0 { used as f32 / total as f32 } else { 0.0 }));
            stat(&mut cols[1], "Roblox RAM", &gb(roblox), &format!("{} {}", e.clients.len(), if e.clients.len() == 1 { "client" } else { "clients" }), None);
            stat(&mut cols[2], "Roblox CPU", &format!("{cpu:.0}%"), &format!("{} cores", e.cores), Some(cpu / 100.0));
            stat(&mut cols[3], "Saved By Potato", &gb(e.freed), "Trimmed since it started", None);
        });
        // a tile per Remote Desktop account (open or not), then each Roblox window on this desktop
        let mut tiles: Vec<Tile> = accounts.iter().map(|n| Tile {
            name: n.clone(), note: "Remote Desktop",
            win: self.wins.iter().find(|w| w.kind == Kind::Rdp(n.clone())).cloned(),
            user: Some(n.clone()), rdp: Some(n.clone()),
        }).collect();
        for w in self.wins.iter().filter(|w| w.kind == Kind::Roblox) {
            let user = e.clients.iter().find(|c| c.pid == w.pid).map(|c| c.user.clone());
            tiles.push(Tile { name: user.clone().unwrap_or("Roblox".into()), note: "Roblox On This PC", win: Some(w.clone()), user, rdp: None });
        }
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(semi("Live", 14.0).color(TEXT));
            ui.label(RichText::new("Click a picture to bring that window to the front").size(12.5).color(FAINT));
        });
        ui.add_space(6.0);
        if tiles.is_empty() {
            Frame::new().fill(CARD).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(28)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(semi("Nothing Open Yet", 16.0).color(TEXT));
                    ui.label(RichText::new("Open Roblox, or make a Remote Desktop account on the Remote Desktop page, and it shows up here live.").color(FAINT));
                });
            });
            return;
        }
        let cols = (ui.available_width() / 330.0).clamp(2.0, 4.0) as usize;
        let gap = 14.0;
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
        Frame::new().fill(CARD).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(8)).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(w - 16.0);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w - 16.0, (w - 16.0) * 9.0 / 16.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(rect, 6.0, Color32::from_rgb(6, 6, 6));
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
            if let Some(win) = &t.win && resp.clicked() {
                crate::preview::focus(win.hwnd);
            }
            if t.win.is_some() {
                let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
                ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.5, LINE.lerp_to_gamma(TEXT, hover * 0.6)), egui::StrokeKind::Inside);
                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let live = t.win.is_some();
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, if live { GOOD } else { OFF });
                ui.vertical(|ui| {
                    ui.set_max_width(w - 110.0);   // room for the button, never under it
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(semi(&t.name, 14.0).color(TEXT)).truncate());
                    let use_ = if pids.is_empty() { "No Roblox".to_string() } else { format!("{} · {cpu:.0}% CPU", gb(ram)) };
                    ui.add(egui::Label::new(RichText::new(format!("{} · {use_}", t.note)).size(12.0).color(FAINT)).truncate());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(n) = &t.rdp && t.win.is_none() {
                        if ui.button("Open").clicked() {
                            act = Some(("open", n.clone()));
                        }
                    } else if let Some(u) = &t.user && !pids.is_empty() && ui.button("Trim").on_hover_text("Frees the RAM this Roblox isn't using").clicked() {
                        act = Some(("trim", u.clone()));
                    }
                });
            });
        }));
        act
    }
}

/// A big number with its label, like the stat tiles on battlebot.dev; `share` adds a thin bar under it.
fn stat(ui: &mut Ui, label: &str, value: &str, note: &str, share: Option<f32>) {
    Frame::new().fill(CARD).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(84.0);   // every tile one height, bar or not
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).size(12.0).color(FAINT));
        ui.label(semi(value, 24.0).color(TEXT));
        ui.label(RichText::new(note).size(12.0).color(FAINT));
        if let Some(share) = share {
            ui.add_space(6.0);
            ui.add(egui::ProgressBar::new(share.clamp(0.0, 1.0)).desired_height(4.0).fill(if share > 0.85 { BAD } else { TEXT }));
        }
    });
}

/// Two lines of text in the middle of an empty preview.
fn centered_note(ui: &Ui, rect: egui::Rect, title: &str, note: &str) {
    let p = ui.painter();
    p.text(rect.center() - egui::vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, title, egui::FontId::new(14.0, FontFamily::Name("semi".into())), DIM);
    p.text(rect.center() + egui::vec2(0.0, 11.0), egui::Align2::CENTER_CENTER, note, egui::FontId::proportional(12.0), FAINT);
}

impl App {
    /// Remote Desktop alts: let them all on at once, make accounts the way Computer Management does, open them.
    fn rdp_page(&mut self, ui: &mut Ui, e: &mut Engine) {
        let r = e.rdp.lock().unwrap().clone();
        let busy = r.busy;
        let (mut patch, mut undo, mut add, mut open) = (false, false, false, None);
        let (mut startup, mut shortcut, mut wide) = (None, None, None);
        let ram_gb = (e.memory.1 as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0;   // what Task Manager shows: 7.99 is "8.0 GB"
        let (by_cpu, by_ram) = crate::rdp::capacity(e.physical_cores, e.cores, ram_gb);
        group(ui, "This PC", |ui| {
            let n = by_cpu.min(by_ram);
            let limit = if by_cpu < by_ram { "CPU" } else { "RAM" };
            row(ui, "How Many It Can Run", &format!("The guide's test: {by_cpu} by CPU, {by_ram} by RAM. {limit} is the limit"), false, |ui, _| {
                ui.label(semi(&format!("About {n}"), 18.0).color(if n == 0 { BAD } else { TEXT }));
            });
            let ghz = e.base_mhz as f32 / 1000.0;
            let note = if e.base_mhz == 0 { "Couldn't read it".to_string() } else if ghz >= 2.5 { "Over 2.5 GHz: one Remote Desktop per 2-3 logical processors".into() } else { "Under 2.5 GHz: Remote Desktops will be slow, expect fewer".into() };
            row(ui, "CPU Speed", &note, true, |ui, _| {
                if e.base_mhz > 0 {
                    ui.label(semi(&format!("{ghz:.1} GHz"), 14.0).color(if ghz >= 2.5 { TEXT } else { BAD }));
                }
            });
        });
        group(ui, "All On At Once", |ui| {
            let (note, can) = match r.patch {
                Patch::Patched => ("On. Potato warns you if a Windows update undoes it", false),
                Patch::Unpatched => ("Windows Pro allows one at a time. Open windows disconnect for a moment; Roblox keeps running", true),
                Patch::Unsupported => ("This Windows version's file doesn't match the patch yet. Only one can be on", false),
                Patch::Unknown => ("Checking Windows' Remote Desktop file", false),
            };
            row(ui, "Let Every Account On", note, !r.can_undo, |ui, id| {
                if can {
                    patch = ui.add_enabled(busy.is_none(), egui::Button::new(if busy == Some("patch") { "Patching..." } else { "Patch" })).labelled_by(id).clicked();
                } else if r.patch == Patch::Patched {
                    ui.label(semi("On", 13.5).color(GOOD));
                }
            });
            if r.can_undo {
                row(ui, "Undo The Patch", "Puts Windows' own file back. Only one Remote Desktop can be on after", true, |ui, id| {
                    undo = ui.add_enabled(busy.is_none(), egui::Button::new(if busy == Some("undo") { "Undoing..." } else { "Undo" })).labelled_by(id).clicked();
                });
            }
        });
        group(ui, "Accounts", |ui| {
            for name in &r.accounts {
                let is_wide = e.config.rdp_wide.contains(name);
                let size = if is_wide { "1800x768" } else { "1366x768" };
                stack_row(ui, name, &format!("{} · {size}, no sound, no drives, no wallpaper", crate::rdp::address(name)), |ui, id| {
                    if ui.button("Open").labelled_by(id).clicked() {
                        open = Some(name.clone());
                    }
                    if ui.button("Desktop Shortcut").on_hover_text("Puts a shortcut on your desktop that opens this account with all these settings, even without Potato").clicked() {
                        shortcut = Some(name.clone());
                    }
                    let mut w = is_wide;
                    if dropdown(ui, &format!("size-{name}"), id, &mut w, &[(false, "1366x768".to_string()), (true, "1800x768 (Boosting)".to_string())]) {
                        wide = Some((name.clone(), w));
                    }
                    let mut on = e.config.rdp_startup.contains(name);
                    if ui.checkbox(&mut on, "Open At Sign In").on_hover_text("Opens this account when you sign in to Windows").changed() {
                        startup = Some((name.clone(), on));
                    }
                });
            }
            let ok_name = crate::rdp::valid_name(&self.new_name);
            row(ui, "New Account", "A Windows user in Remote Desktop Users", false, |ui, id| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(180.0).hint_text("Name").char_limit(20)).labelled_by(id);
                if !self.new_name.is_empty() && !ok_name && !r.has_focus() {
                    ui.label(RichText::new("Can't Use / \\ [ ] : ; | = , + * ? < > @").size(12.5).color(BAD));
                }
            });
            row(ui, "Its Password", "For an account you already have, type its password", false, |ui, id| {
                ui.add(egui::TextEdit::singleline(&mut self.new_pass).desired_width(180.0).hint_text("Password").password(true)).labelled_by(id);
            });
            row(ui, "Make It", "Saves the login, so Open signs straight in", true, |ui, id| {
                let ready = ok_name && !self.new_pass.is_empty() && busy.is_none();
                add = ui.add_enabled(ready, egui::Button::new(if busy == Some("add") { "Making..." } else { "Make Account" })).labelled_by(id).clicked();
            });
        });
        group(ui, "CPU", |ui| {
            row(ui, "Remote Desktop CPU Limit", "Caps each Remote Desktop window. Too low and sessions freeze or disconnect", true, |ui, id| {
                number_box(ui, id, &mut e.config.rdp_cpu_limit, 100, "%", "None");
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
        if let Some((name, on)) = startup {
            e.config.rdp_startup.retain(|n| *n != name);
            if on {
                e.config.rdp_startup.push(name);
            }
            if !crate::rdp::save_startup(!e.config.rdp_startup.is_empty()) {
                e.note = "Couldn't Set Up Opening At Sign In".into();
            }
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
        if let Some(name) = open && let Err(m) = crate::rdp::open(&name, e.config.rdp_wide.contains(&name)) {
            e.note = m;
        }
    }
}

fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    ui.label(semi(title, 14.0).color(TEXT));
    ui.add_space(6.0);
    Frame::new().fill(CARD).corner_radius(CornerRadius::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        body(ui);
    });
}

/// One setting: label and note on the left, its control on the right, a line under it unless it's `last`.
/// The control gets the label's id for `labelled_by`, so screen readers name it.
fn row(ui: &mut Ui, label: &str, note: &str, last: bool, control: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            let id = ui.vertical(|ui| {
                let id = ui.label(RichText::new(label).size(14.0).color(TEXT)).id;
                if !note.is_empty() {
                    ui.label(RichText::new(note).size(12.5).color(FAINT));
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
fn stack_row(ui: &mut Ui, label: &str, note: &str, controls: impl FnOnce(&mut Ui, egui::Id)) {
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        let id = ui.label(RichText::new(label).size(14.0).color(TEXT)).id;
        ui.label(RichText::new(note).size(12.5).color(FAINT));
        ui.add_space(8.0);
        ui.horizontal(|ui| controls(ui, id));
    });
    let rect = r.response.rect;
    ui.painter().hline(rect.x_range().shrink(18.0), rect.bottom(), Stroke::new(1.0, LINE));
}

/// A setting with an on/off switch where the whole row is the button (Fitts's law: a big target). Returns
/// true when it flipped.
fn switch_row(ui: &mut Ui, label: &str, note: &str, last: bool, on: &mut bool) -> bool {
    let bg = ui.painter().add(egui::Shape::Noop);   // hover tint, drawn under the row once its size is known
    let mut knob = egui::Rect::NOTHING;
    let r = Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(label).size(14.0).color(TEXT));
                if !note.is_empty() {
                    ui.label(RichText::new(note).size(12.5).color(FAINT));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                knob = ui.allocate_exact_size(egui::vec2(42.0, 22.0), Sense::hover()).0;
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
    ui.painter().set(bg, egui::Shape::rect_filled(rect, 8.0, Color32::from_white_alpha((hover * 6.0) as u8)));
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
    let knob = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(w * 2.0, h * 2.0));
    p.rect_filled(knob, h, DIM.lerp_to_gamma(NAV, fill));
}

/// A box for a whole number up to `max`, with its unit after it; empty means 0 (off).
/// A number that counts once typing is done (Enter or clicking away): while typing 2500 MB the limit would
/// otherwise be 2, 25 and 250 MB for a moment, and clients over those would restart.
fn number_box(ui: &mut Ui, label: egui::Id, value: &mut u32, max: u32, unit: &str, empty: &str) {
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

/// A dropdown of fixed choices. Returns true when the choice changed.
fn dropdown<T: PartialEq + Copy>(ui: &mut Ui, salt: &str, label: egui::Id, value: &mut T, options: &[(T, String)]) -> bool {
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

/// The one white button on a screen, for the thing to do next: the header buttons' size and press, inverted.
fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    button(ui, text, false, true)
}

/// The page header's buttons: one size and style, so they line up. They squash a little when pressed and
/// show a spinner while their work runs.
fn action(ui: &mut Ui, text: &str, busy: bool) -> egui::Response {
    button(ui, text, busy, false)
}

fn button(ui: &mut Ui, text: &str, busy: bool, primary: bool) -> egui::Response {
    let font = if primary { egui::FontId::new(13.5, FontFamily::Name("semi".into())) } else { egui::FontId::proportional(13.5) };
    let ink = if primary { NAV } else { TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, ink);
    let size = egui::vec2((galley.size().x + 32.0 + if busy { 20.0 } else { 0.0 }).max(120.0), 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let ctx = ui.ctx();
    let press = ctx.animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), dur(0.07));
    let hover = ctx.animate_bool_with_time(resp.id.with("hover"), resp.hovered(), dur(0.12));
    let rect = egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.04 * press));
    let p = ui.painter();
    if primary {
        p.rect_filled(rect, 7.0, TEXT.lerp_to_gamma(Color32::from_gray(205), hover));
    } else {
        p.rect_filled(rect, 7.0, CARD_HI.lerp_to_gamma(Color32::from_gray(44), hover));
        p.rect_stroke(rect, 7.0, Stroke::new(1.0, LINE.lerp_to_gamma(Color32::from_gray(90), hover)), egui::StrokeKind::Inside);
    }
    let mut x = rect.center().x - galley.size().x / 2.0;
    if busy {
        x += 10.0;
        egui::Spinner::new().size(14.0).color(DIM).paint_at(ui, egui::Rect::from_center_size(egui::pos2(x - 14.0, rect.center().y), egui::vec2(14.0, 14.0)));
    }
    p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    resp
}

fn style(ctx: &egui::Context) {
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
    v.extreme_bg_color = Color32::from_rgb(14, 14, 14);   // text boxes
    v.selection.bg_fill = Color32::from_rgb(70, 70, 70);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 40, 40);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(90, 90, 90));
    v.widgets.open.weak_bg_fill = Color32::from_rgb(40, 40, 40);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
    });
}
