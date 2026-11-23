//! Everything Potato does, on its own thread, so it keeps working while the window is in the tray. The window
//! (app.rs) only shows this state and changes the settings.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{self, Account, Config, Priority};
use crate::files;
use crate::helper::{self, Maximizer};
use crate::rejoin;
use crate::system::{self, Client, Cpu, Profile};
use crate::tweaks::{core_masks, Plan};
use crate::version::Release;

const TICK: Duration = Duration::from_secs(2);
/// how often Windows' Remote Desktop file is checked for an update that undid the patch
const RDP_EVERY: Duration = Duration::from_secs(10 * 60);
const UPDATE_EVERY: Duration = Duration::from_secs(10 * 60);   // GitHub allows 60 signed-out checks an hour per PC
#[cfg(windows)]
const RELEASES: &str = "https://api.github.com/repos/birchir/potato/releases/latest";

/// Potato's settings file, readable by every Windows user (the helpers read it from their own sessions).
/// What the guide puts on its own cores, apart from Roblox: Remote Desktop windows and AutoHotkey (every build of it).
const HELPER_EXES: [&str; 7] = ["mstsc.exe", "AutoHotkey.exe", "AutoHotkey64.exe", "AutoHotkey32.exe", "AutoHotkeyU64.exe", "AutoHotkeyU32.exe", "AutoHotkeyA32.exe"];

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Potato").join("settings.json")
}

/// Potato's own copy next to its settings, which the logon tasks run (helper.rs install keeps it current).
pub fn installed_exe() -> PathBuf {
    config_path().with_file_name("Potato.exe")
}

/// Where an account's settings stand.
#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Applied,
    /// their Roblox is open; Potato writes the moment it closes
    Waiting,
    Failed,
}

/// Where a newer release stands. Shared with the threads that check for it and install it.
#[derive(Clone, Default)]
#[cfg_attr(not(windows), allow(dead_code))]   // only Windows checks for updates
pub enum UpdateState {
    #[default]
    None,
    Available(Release),
    Installing(String),
    Failed(String),
}

pub type Update = Arc<Mutex<UpdateState>>;

pub struct Engine {
    pub config: Config,
    pub me: String,
    pub clients: Vec<Client>,
    pub profiles: Vec<Profile>,
    pub states: HashMap<String, State>,
    pub memory: (u64, u64),
    pub freed: u64,
    pub cores: usize,
    /// physical cores and rated speed (MHz), for the guide's how-many-RDPs test
    pub physical_cores: usize,
    pub base_mhz: u32,
    pub update: Update,
    /// the Remote Desktop page's state, looked up on its own thread
    pub rdp: Arc<Mutex<crate::rdp::Rdp>>,
    pub note: String,
    /// each client's CPU use since the last look, in percent of the whole PC
    pub cpu_pct: HashMap<u32, f32>,
    /// restarts and crash rejoins per account, for the report
    pub restarts: HashMap<String, u32>,
    /// what each client's CPU settings were last set to, so they're only set again when something changes
    cpu_set: HashMap<u32, (Priority, Option<u64>, bool)>,
    /// cores set on each Remote Desktop window and macro (None: every core)
    helper_set: HashMap<u32, Option<u64>>,
    cpu_seen: HashMap<u32, (u64, Instant)>,
    /// each capped client's job and the cap it has
    jobs: HashMap<u32, (u32, isize)>,
    /// clients Potato closed itself, so they don't count as crashes
    closed_by_us: HashSet<u32>,
    /// the last clients seen: PID -> account
    last_clients: HashMap<u32, String>,
    /// where each client is, for accounts that rejoin after a crash: PID -> (server, when it was read)
    whereabouts: HashMap<u32, (rejoin::Server, Instant)>,
    /// crashed clients waiting a moment in case something else reopens them: (account, server, when)
    crashed: Vec<(String, rejoin::Server, Instant)>,
    upload_applied: Option<u32>,
    /// every account's .rdp file has been rewritten since Potato started (desktop shortcuts open them as they are)
    rdp_files_written: bool,
    shutdown_done: Option<u32>,
    /// the clock hour (0-23) the last scheduled report went out in, so each hour sends exactly one
    reported_hour: u32,
    /// a look at RAM and CPU every 30 seconds since the last report, for the report's graphs
    /// RAM and CPU every 30 s since the last report (the Overview chart and the hourly report)
    pub history: Vec<crate::report::Sample>,
    history_start: Instant,
    /// a report waiting for the helpers' screenshots: (request id, when asked)
    report_pending: Option<(u64, Instant)>,
    reported_freezes: HashMap<String, u32>,
    maximizer: Maximizer,
    helper_installed: Option<bool>,
    last_update_check: Option<Instant>,
    last_rdp_check: Option<Instant>,
    /// the Windows notification about an undone Remote Desktop patch went out (once per undo)
    rdp_warned: bool,
    last_tick: Option<Instant>,
    pub last_trim: Instant,
    /// what the latest trim freed (None until one runs)
    pub last_trim_freed: Option<u64>,
    last_clean: Instant,
    last_restart: Instant,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            config: std::fs::read_to_string(config_path()).map(|t| Config::from_json(&t)).unwrap_or_default(),
            me: system::current_sid(),
            clients: Vec::new(),
            profiles: system::profiles(),
            states: HashMap::new(),
            memory: (0, 0),
            freed: 0,
            cores: system::logical_cores(),
            physical_cores: system::physical_cores(),
            base_mhz: system::base_mhz(),
            update: Update::default(),
            rdp: Default::default(),
            note: String::new(),
            cpu_pct: HashMap::new(),
            restarts: HashMap::new(),
            cpu_set: HashMap::new(),
            helper_set: HashMap::new(),
            cpu_seen: HashMap::new(),
            jobs: HashMap::new(),
            closed_by_us: HashSet::new(),
            last_clients: HashMap::new(),
            whereabouts: HashMap::new(),
            crashed: Vec::new(),
            upload_applied: None,
            rdp_files_written: false,
            shutdown_done: None,
            reported_hour: system::local_minutes() / 60,   // the first report waits for the next full hour
            history: Vec::new(),
            history_start: Instant::now(),
            report_pending: None,
            reported_freezes: HashMap::new(),
            maximizer: Maximizer::default(),
            helper_installed: None,
            last_update_check: None,
            last_rdp_check: None,
            rdp_warned: false,
            last_tick: None,
            last_trim: Instant::now(),
            last_trim_freed: None,
            last_clean: Instant::now(),
            last_restart: Instant::now(),
        }
    }

    /// Start the engine on its own thread, plus the thread that reopens this login's clients after a restart
    /// (a helper running in this login would do that instead). Returns the shared engine for the window.
    pub fn start(on_change: impl Fn() + Send + 'static) -> Arc<Mutex<Engine>> {
        let engine = Arc::new(Mutex::new(Engine::new()));
        let e = engine.clone();
        std::thread::spawn(move || loop {
            e.lock().unwrap().tick();
            on_change();
            std::thread::sleep(Duration::from_millis(500));
        });
        let me = engine.lock().unwrap().me.clone();
        std::thread::spawn(move || {
            let mut jobs = helper::SessionJobs::default();
            loop {
                if !helper::running_here() {
                    helper::session_tick(&me, &mut jobs);   // a helper in this login would do it instead
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        engine
    }

    /// Save the settings and act on them right away.
    pub fn changed(&mut self) {
        let path = config_path();
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(path, self.config.to_json());
        self.last_tick = None;
    }

    pub fn tick(&mut self) {
        if self.last_update_check.is_none_or(|t| t.elapsed() >= UPDATE_EVERY) {
            self.last_update_check = Some(Instant::now());
            check_for_update(self.update.clone());
        }
        if self.last_rdp_check.is_none_or(|t| t.elapsed() >= RDP_EVERY) {
            self.last_rdp_check = Some(Instant::now());
            refresh_rdp(self.rdp.clone(), None, || Ok(()));
        }
        self.watch_rdp();
        if self.last_tick.is_some_and(|t| t.elapsed() < TICK) {
            return;
        }
        self.last_tick = Some(Instant::now());
        self.clients = system::clients();
        self.memory = system::memory();
        let running: HashSet<&str> = self.clients.iter().map(|c| c.sid.as_str()).collect();
        for p in &self.profiles {
            let a = self.config.account(&p.sid);
            if !a.on {
                self.states.remove(&p.sid);
                continue;
            }
            // Roblox rewrites its settings when it closes, so write while it's closed and only look while it runs
            let state = if running.contains(p.sid.as_str()) {
                match files::is_applied(&p.local, &a.plan) {
                    Ok(true) => State::Applied,
                    Ok(false) => State::Waiting,
                    Err(_) => State::Failed,
                }
            } else {
                match files::apply(&p.local, &a.plan) {
                    Ok(_) => State::Applied,
                    Err(_) => State::Failed,
                }
            };
            self.states.insert(p.sid.clone(), state);
        }
        // a Roblox installed for every user has one flag file for all of them: Ultra Low if any account on wants it,
        // written only while no client runs (one account's setting can't flip-flop with another's)
        let on: Vec<Account> = self.profiles.iter().map(|p| self.config.account(&p.sid)).filter(|a| a.on).collect();
        if running.is_empty() && !on.is_empty() {
            let plan = Plan { ultra_low: on.iter().any(|a| a.plan.ultra_low), ..on[0].plan.clone() };
            if files::apply_flags(&files::shared_flag_files(), &plan).is_err() {
                self.note = "Couldn't Change Roblox's Shared Settings".into();
            }
        }
        self.measure_cpu();
        let at = self.history_start.elapsed().as_secs();
        if self.history.last().is_none_or(|s| at >= s.at + 30) && self.history.len() < 2880 {
            self.history.push(crate::report::Sample {
                at, pc_used: self.memory.0,
                roblox: self.clients.iter().map(|c| c.ram).sum(),
                cpu: self.cpu_pct.values().sum::<f32>().min(100.0),
            });
        }
        self.watch_crashes();
        self.close_extras();
        self.restart_hogs();
        self.apply_cpu();
        self.keep_maximized();
        self.apply_upload_cap();
        let accounts = self.rdp.lock().unwrap().accounts.clone();
        crate::preview::name_rdp_windows(&accounts);
        if !self.rdp_files_written && !accounts.is_empty() {
            for n in &accounts {
                let _ = crate::rdp::write_file(n, self.config.rdp_wide.contains(n));
            }
            self.rdp_files_written = true;
        }
        self.shutdown_on_time();
        self.report();
        let hours = self.config.restart_hours;
        if hours > 0 && self.last_restart.elapsed() >= Duration::from_secs(hours as u64 * 3600) {
            self.restart_clients();
        }
        let minutes = self.config.trim_minutes;
        if minutes > 0 && self.last_trim.elapsed() >= Duration::from_secs(minutes as u64 * 60) {
            self.trim_all(false);
        }
        let (used, total) = self.memory;
        if self.config.clean_above > 0 && total > 0 && used * 100 / total >= self.config.clean_above as u64
            && self.last_clean.elapsed() >= Duration::from_secs(60) {
            self.clean(false);
        }
    }

    fn close_extras(&mut self) {
        let list: Vec<(&str, u32, u64)> = self.clients.iter().map(|c| (c.sid.as_str(), c.pid, c.started)).collect();
        let extra = config::extra_clients(&self.config, &list);
        if extra.is_empty() {
            return;
        }
        self.closed_by_us.extend(extra.iter().copied());
        let n = extra.iter().filter(|pid| system::close(**pid)).count();
        self.note = format!("Closed {n} Extra {}", if n == 1 { "Client" } else { "Clients" });
        self.clients.retain(|c| !extra.contains(&c.pid));
    }

    /// Priority, cores and Efficiency Mode for every client. Clients are taken oldest first, so each keeps
    /// its cores while others open and close.
    fn apply_cpu(&mut self) {
        let mut list: Vec<&Client> = self.clients.iter().collect();
        list.sort_by_key(|c| (c.started, c.pid));
        let (roblox_cores, helper_mask) = crate::tweaks::split_cores(self.cores, self.config.helper_cores as usize);
        let masks = if self.config.cores_per_client > 0 {
            Some(core_masks(list.len(), roblox_cores, self.config.cores_per_client as usize))
        } else {
            (helper_mask != 0).then(|| vec![crate::tweaks::first_cores(roblox_cores); list.len()])
        };
        let untouched = (Priority::Normal, None, false);
        for (i, c) in list.iter().enumerate() {
            let a = self.config.account(&c.sid);
            let want = if a.on { (a.priority, masks.as_ref().map(|m| m[i]), a.efficiency) } else { untouched };
            let had = self.cpu_set.get(&c.pid).copied().unwrap_or(untouched);
            if want != had && system::set_cpu(c.pid, &Cpu { priority: want.0, cores: want.1, efficiency: want.2 }) {
                self.cpu_set.insert(c.pid, want);
            }
        }
        let pids: HashSet<u32> = self.clients.iter().map(|c| c.pid).collect();
        self.cpu_set.retain(|pid, _| pids.contains(pid));
        // Remote Desktop windows and AutoHotkey macros on the cores Roblox doesn't use (the guide's core affinity)
        let helpers: Vec<u32> = HELPER_EXES.iter().flat_map(|exe| system::pids_named(exe)).collect();
        for &pid in &helpers {
            let want = (helper_mask != 0).then_some(helper_mask);
            if self.helper_set.get(&pid).copied().unwrap_or(None) != want
                && system::set_cpu(pid, &Cpu { priority: Priority::Normal, cores: want, efficiency: false }) {
                self.helper_set.insert(pid, want);
            }
        }
        self.helper_set.retain(|pid, _| helpers.contains(pid));
        // hard CPU caps: a job per capped client or Remote Desktop window; a lifted cap goes back to 100%
        let mut caps: Vec<(u32, u32)> = self.clients.iter().map(|c| {
            let a = self.config.account(&c.sid);
            (c.pid, if a.on { a.cpu_limit } else { 0 })
        }).collect();
        caps.extend(system::pids_named("mstsc.exe").into_iter().map(|pid| (pid, self.config.rdp_cpu_limit)));
        let pids: HashSet<u32> = caps.iter().map(|c| c.0).collect();
        for &(pid, want) in &caps {
            let had = self.jobs.get(&pid).map_or(0, |j| j.0);
            if want == had || (want == 0 && had == 100) {
                continue;
            }
            let mut job = self.jobs.get(&pid).map(|j| j.1);
            if system::cpu_cap(pid, if want == 0 { 100 } else { want }, &mut job)
                && let Some(j) = job {
                self.jobs.insert(pid, (if want == 0 { 100 } else { want }, j));
            }
        }
        for (_, j) in self.jobs.extract_if(|pid, _| !pids.contains(pid)) {
            system::drop_job(j.1);
        }
    }

    fn measure_cpu(&mut self) {
        let now = Instant::now();
        let cores = self.cores.max(1) as f32;
        self.cpu_pct.clear();
        for c in &self.clients {
            if let Some((t, at)) = self.cpu_seen.get(&c.pid) {
                let wall = now.duration_since(*at).as_nanos() as f32 / 100.0;   // in 100 ns, like cpu_time
                if wall > 0.0 {
                    self.cpu_pct.insert(c.pid, (c.cpu_time.saturating_sub(*t) as f32 / wall / cores * 100.0).min(100.0));
                }
            }
        }
        self.cpu_seen = self.clients.iter().map(|c| (c.pid, (c.cpu_time, now))).collect();
    }

    /// Notice clients that closed without Potato closing them. For accounts with Rejoin After A Crash, wait
    /// 20 seconds in case something else (like a macro) reopens them, then send them back to their server.
    fn watch_crashes(&mut self) {
        let now: HashMap<u32, String> = self.clients.iter().map(|c| (c.pid, c.sid.clone())).collect();
        for (pid, sid) in &self.last_clients {
            if now.contains_key(pid) || self.closed_by_us.remove(pid) {
                continue;
            }
            if let Some((server, _)) = self.whereabouts.remove(pid)
                && self.config.account(sid).on && self.config.account(sid).rejoin_crashed {
                self.crashed.push((sid.clone(), server, Instant::now()));
            }
        }
        self.last_clients = now;
        // keep each watched client's server fresh (its log is re-read every 30 s, so teleports are followed)
        for c in &self.clients {
            let a = self.config.account(&c.sid);
            let stale = self.whereabouts.get(&c.pid).is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(30));
            if a.on && a.rejoin_crashed && stale {
                let log = self.profiles.iter().find(|p| p.sid == c.sid).and_then(|p| files::client_log(&p.local, c.started));
                if let Some(s) = rejoin::server(system::command_line(c.pid).as_deref(), log.as_deref()) {
                    self.whereabouts.insert(c.pid, (s, Instant::now()));
                }
            }
        }
        let pids: HashSet<u32> = self.clients.iter().map(|c| c.pid).collect();
        self.whereabouts.retain(|pid, _| pids.contains(pid));
        let mut due = Vec::new();
        self.crashed.retain(|(sid, server, at)| {
            if self.clients.iter().any(|c| &c.sid == sid && c.started >= system_time_now().saturating_sub(at.elapsed().as_nanos() as u64 / 100)) {
                return false;   // a new client opened for the account: something else rejoined it
            }
            if at.elapsed() < Duration::from_secs(20) {
                return true;
            }
            due.push((sid.clone(), server.clone()));
            false
        });
        for (sid, server) in due {
            if self.send_rejoins(&[(sid.as_str(), vec![server])]) > 0 {
                *self.restarts.entry(sid).or_default() += 1;
                self.note = "Rejoined A Crashed Client".into();
            }
        }
    }

    /// Restart clients that grew past their account's memory limit.
    fn restart_hogs(&mut self) {
        let hogs: HashSet<u32> = self.clients.iter().filter(|c| {
            let a = self.config.account(&c.sid);
            a.on && a.ram_limit_mb > 0 && c.ram > a.ram_limit_mb as u64 * (1 << 20)
        }).map(|c| c.pid).collect();
        if !hogs.is_empty() {
            self.restart(|c| hogs.contains(&c.pid));
        }
    }

    /// Roblox's upload limit, through a Windows QoS policy (applied again whenever Potato starts).
    fn apply_upload_cap(&mut self) {
        let want = self.config.upload_kbps;
        if self.upload_applied == Some(want) {
            return;
        }
        self.upload_applied = Some(want);
        let mut script = "Remove-NetQosPolicy -Name Potato -PolicyStore ActiveStore -Confirm:$false -ErrorAction SilentlyContinue".to_string();
        if want > 0 {
            script += &format!("; New-NetQosPolicy -Name Potato -PolicyStore ActiveStore -AppPathNameMatchCondition RobloxPlayerBeta.exe -ThrottleRateActionBitsPerSecond {}", want as u64 * 1000);
        }
        if !system::run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]) && want > 0 {
            self.note = "Couldn't Set The Upload Cap".into();
        }
    }

    fn shutdown_on_time(&mut self) {
        let Some(at) = config::parse_time(&self.config.shutdown_at) else { return };
        let now = system::local_minutes();
        if now != at {
            self.shutdown_done = None;
        } else if self.shutdown_done.is_none() {
            self.shutdown_done = Some(at);
            system::run("shutdown", &["/s", "/t", "60", "/c", "Potato is turning the PC off in a minute (shutdown /a cancels)"]);
            self.note = "Turning The PC Off In A Minute".into();
        }
    }

    /// Every hour, post a report to the Discord webhook. With screenshots on, the helpers are asked for
    /// them first and the report goes out a few seconds later.
    fn report(&mut self) {
        let d = self.config.discord.clone();
        if !config::Discord::valid_webhook(&d.webhook) {
            return;
        }
        if let Some((id, asked)) = self.report_pending {
            if asked.elapsed() >= Duration::from_secs(6) {
                self.report_pending = None;
                self.post_report(Some(id));
            }
            return;
        }
        // on the hour (2:00, 3:00, ...), whenever Potato was started; a few minutes' grace covers a busy tick
        let now = system::local_minutes();
        if now / 60 != self.reported_hour && now % 60 < 10 {
            self.reported_hour = now / 60;
            self.send_report_now();
        }
    }

    /// Start a report right away (also the Send Test Report button).
    pub fn send_report_now(&mut self) {
        if self.config.discord.screenshots {
            let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
            let dir = helper::report_dir();
            let _ = std::fs::create_dir_all(&dir);
            if std::fs::write(dir.join("request.txt"), id.to_string()).is_ok() {
                self.report_pending = Some((id, Instant::now()));
                return;
            }
        }
        self.post_report(None);
    }

    fn post_report(&mut self, shots: Option<u64>) {
        let dir = helper::report_dir();
        let mut accounts = Vec::new();
        for p in &self.profiles {
            let a = self.config.account(&p.sid);
            let mine: Vec<&Client> = self.clients.iter().filter(|c| c.sid == p.sid).collect();
            let stats: serde_json::Value = std::fs::read_to_string(dir.join(format!("{}.json", p.sid))).ok()
                .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let total = stats["freezes"].as_u64().unwrap_or(0) as u32;
            let last = self.reported_freezes.insert(p.sid.clone(), total).unwrap_or(total);
            let images: Vec<PathBuf> = match (shots, stats["id"].as_u64()) {
                (Some(id), Some(got)) if got == id => (0..stats["shots"].as_u64().unwrap_or(0).min(4))
                    .map(|i| dir.join(format!("{}-{i}.png", p.sid))).filter(|f| f.exists()).collect(),
                _ => Vec::new(),
            };
            accounts.push(crate::report::Account {
                name: p.name.clone(),
                on: a.on,
                clients: mine.iter().map(|c| crate::report::Client {
                    ram: c.ram, cpu: self.cpu_pct.get(&c.pid).copied().unwrap_or(0.0),
                    up_minutes: system_time_now().saturating_sub(c.started) / 600_000_000,
                }).collect(),
                freezes: total.saturating_sub(last),
                frozen_now: stats["frozenNow"].as_u64().unwrap_or(0) as u32,
                restarts: self.restarts.remove(&p.sid).unwrap_or(0),
                screenshots: images,
            });
        }
        let pc = crate::report::Pc { used: self.memory.0, total: self.memory.1, freed: self.freed, version: env!("CARGO_PKG_VERSION") };
        let history = std::mem::take(&mut self.history);
        self.history_start = Instant::now();
        let card = dir.join("card.png");
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::write(&card, crate::card::render(&pc, &accounts, &history)).is_err() {
            return;
        }
        let (payload, files) = crate::report::build(&accounts, card);
        let webhook = self.config.discord.webhook.clone();
        std::thread::spawn(move || {
            let mut args: Vec<String> = vec!["-s".into(), "-m".into(), "60".into(), "--form-string".into(), format!("payload_json={payload}")];
            for (i, f) in files.iter().enumerate() {
                args.push("-F".into());
                args.push(format!("files[{i}]=@{};type=image/png;filename=shot{i}.png", f.display()));
            }
            args.push(webhook);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            system::run("curl", &args);
        });
    }

    /// This login's windows here; every other login's through the helper, installed while any account wants it.
    fn keep_maximized(&mut self) {
        let mine = self.config.account(&self.me);
        if mine.on && mine.plan.maximized {
            self.maximizer.tick();
        }
        let wanted = self.profiles.iter().map(|p| self.config.account(&p.sid)).any(|a| a.on && a.plan.maximized);
        if self.helper_installed != Some(wanted) {
            self.helper_installed = Some(wanted);
            if wanted {
                if !helper::install() {
                    self.note = "Couldn't Set Up Open Maximized For Remote Desktop Accounts".into();
                }
            } else {
                helper::uninstall();
            }
        }
    }

    /// `say` also shows the result in the window; the Windows notification goes out either way (if it's on).
    pub fn trim_all(&mut self, say: bool) {
        let (n, freed) = self.trim_clients();
        self.tell(say, freed, format!("Trimmed {n} {}, Freed {}", if n == 1 { "Client" } else { "Clients" }, gb(freed)));
    }

    fn trim_clients(&mut self) -> (usize, u64) {
        self.last_trim = Instant::now();
        let (n, freed) = self.trim_where(|_| true);
        self.last_trim_freed = Some(freed);
        (n, freed)
    }

    /// Trim the clients running as one Windows user (a tile on the Overview).
    pub fn trim_user(&mut self, user: &str) {
        let (_, freed) = self.trim_where(|c| c.user.eq_ignore_ascii_case(user));
        self.tell(true, freed.max(1), format!("Trimmed {user}, Freed {}", gb(freed)));
    }

    fn trim_where(&mut self, pick: impl Fn(&Client) -> bool) -> (usize, u64) {
        let before: u64 = self.clients.iter().map(|c| c.ram).sum();
        let n = self.clients.iter().filter(|c| pick(c) && system::trim(c.pid)).count();
        self.clients = system::clients();
        let freed = before.saturating_sub(self.clients.iter().map(|c| c.ram).sum());
        self.freed += freed;
        (n, freed)
    }

    pub fn clean(&mut self, say: bool) {
        self.last_clean = Instant::now();
        let before = system::memory().0;
        let ok = system::clean_standby();
        self.trim_clients();
        self.memory = system::memory();
        let freed = before.saturating_sub(self.memory.0);
        self.tell(say, if ok { freed } else { 1 }, if ok { format!("Cleaned Memory, Freed {}", gb(freed)) } else { "Couldn't Clean Memory".into() });
    }

    /// Nothing freed: no notification, so a timer trim of idle clients doesn't pop one up.
    fn tell(&mut self, say: bool, freed: u64, text: String) {
        if self.config.notify && freed > 0 {
            system::notify(&text);
        }
        if say {
            self.note = text;
        }
    }

    /// Close every client and reopen each in the server it was in.
    pub fn restart_clients(&mut self) {
        self.last_restart = Instant::now();
        self.restart(|_| true);
    }

    /// Hand each account's join links to its login (helper.rs), one request file per account. Returns how
    /// many accounts got one.
    fn send_rejoins(&self, servers: &[(&str, Vec<rejoin::Server>)]) -> usize {
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let dir = helper::rejoin_dir();
        let _ = std::fs::create_dir_all(&dir);
        servers.iter().filter(|(sid, list)| std::fs::write(dir.join(format!("{sid}.txt")), rejoin::request(id, list)).is_ok()).count()
    }

    /// Close the picked clients and reopen each in the server it was in. The reopening happens in each
    /// account's own login (helper.rs); clients whose server can't be read are left running.
    fn restart(&mut self, pick: impl Fn(&Client) -> bool) {
        let mut servers: HashMap<&str, Vec<rejoin::Server>> = HashMap::new();
        let (mut close, mut unknown) = (Vec::new(), 0);
        for c in self.clients.iter().filter(|c| pick(c)) {
            let log = self.profiles.iter().find(|p| p.sid == c.sid).and_then(|p| files::client_log(&p.local, c.started));
            match rejoin::server(system::command_line(c.pid).as_deref(), log.as_deref()) {
                Some(s) if !c.sid.is_empty() => {
                    servers.entry(c.sid.as_str()).or_default().push(s);
                    close.push(c.pid);
                }
                _ => unknown += 1,
            }
        }
        let list: Vec<(&str, Vec<rejoin::Server>)> = servers.into_iter().collect();
        if self.send_rejoins(&list) < list.len() {
            self.note = "Couldn't Hand Every Restart To Its Account".into();
            return;
        }
        for (sid, l) in &list {
            *self.restarts.entry(sid.to_string()).or_default() += l.len() as u32;
        }
        self.closed_by_us.extend(close.iter().copied());
        let closed = close.iter().filter(|pid| system::close(**pid)).count();
        self.note = format!("Restarting {closed} {}{}", if closed == 1 { "Client" } else { "Clients" },
            if unknown > 0 { format!(". Couldn't Find The Server For {unknown}, Left Those Open") } else { String::new() });
        self.last_tick = None;
    }

    /// A System Restore point, so the advanced tools can be undone. Windows makes at most one a day.
    /// Remember a patched Remote Desktop, and warn once each time Windows undoes it.
    fn watch_rdp(&mut self) {
        let r = self.rdp.lock().unwrap().clone();
        if r.busy.is_some() {
            return;   // mid patch or undo: the file is changing
        }
        if r.patch == crate::rdp::Patch::Patched {
            self.rdp_warned = false;
            if !self.config.rdp_patched {
                self.config.rdp_patched = true;
                self.changed();
            }
        } else if r.reverted(self.config.rdp_patched) && !self.rdp_warned {
            self.rdp_warned = true;
            system::notify("Windows undid the Remote Desktop patch, so only one Remote Desktop can be on. Patch it again on Potato's Remote Desktop page.");
        }
    }

    pub fn restore_point(&mut self) {
        let ok = system::run("powershell", &["-NoProfile", "-NonInteractive", "-Command",
            "Enable-ComputerRestore -Drive \"$env:SystemDrive\\\"; Checkpoint-Computer -Description 'Before Potato Tweaks' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop"]);
        self.note = if ok { "Made A Restore Point" } else { "Couldn't Make A Restore Point" }.into();
    }

    pub fn restore(&mut self) {
        let mut n = 0;
        for p in &self.profiles {
            if self.config.account(&p.sid).on {
                n += files::restore(&files::all_files(&p.local)).unwrap_or(0);
                self.config.account_mut(&p.sid).on = false;   // otherwise the next tick would apply everything again
            }
        }
        n += files::restore(&files::shared_flag_files()).unwrap_or(0);
        self.changed();
        self.note = format!("Restored {n} Files. Switch an account back on to use Potato again.");
    }
}

/// Now, as a Windows file time (100 ns since 1601), to compare with client start times.
fn system_time_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64 / 100 + 116_444_736_000_000_000)
}

const GIB: f64 = (1u64 << 30) as f64;

pub fn gb(bytes: u64) -> String {
    let g = bytes as f64 / GIB;
    if bytes >= 1000 << 20 { format!("{g:.1} GB") } else { format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64) }
}

/// The demo build shows the update banner when POTATO_DEMO_UPDATE is set, so a test render can check it.
#[cfg(not(windows))]
fn check_for_update(update: Update) {
    if std::env::var_os("POTATO_DEMO_UPDATE").is_some() {
        *update.lock().unwrap() = UpdateState::Available(Release { tag: "v9.9.9".into(), exe_url: String::new(), sums_url: String::new() });
    }
}

#[cfg(not(windows))]
pub fn install_update(_: Release, _: Update) {}

/// GET a URL with curl (it ships with Windows 10 and 11), following GitHub's download redirects.
#[cfg(windows)]
fn fetch(url: &str, seconds: u32) -> Option<Vec<u8>> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("curl").args(["-sfL", "-m", &seconds.to_string(), "-H", "User-Agent: Potato", url])
        .creation_flags(0x0800_0000).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Ask GitHub for the latest release in the background.
#[cfg(windows)]
fn check_for_update(update: Update) {
    std::thread::spawn(move || {
        let Some(body) = fetch(RELEASES, 15) else { return };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else { return };
        let mut state = update.lock().unwrap();
        if let (UpdateState::None, Some(r)) = (&*state, crate::version::newer_release(&v, env!("CARGO_PKG_VERSION"))) {
            *state = UpdateState::Available(r);
        }
    });
}

/// Download the new Potato.exe, check it against the release's SHA-256, put it in place of this one and
/// restart into it. Windows lets a running exe be renamed but not overwritten, so this one becomes
/// Potato.old (deleted on the next start).
#[cfg(windows)]
pub fn install_update(r: Release, update: Update) {
    *update.lock().unwrap() = UpdateState::Installing(r.tag.clone());
    std::thread::spawn(move || {
        let fail = |msg: &str| *update.lock().unwrap() = UpdateState::Failed(msg.to_string());
        let (Some(exe), Some(sums)) = (fetch(&r.exe_url, 300), fetch(&r.sums_url, 30)) else { return fail("Couldn't download the update") };
        if !crate::version::matches_checksum(&exe, &String::from_utf8_lossy(&sums)) {
            return fail("The download didn't match its checksum, so it wasn't installed");
        }
        let Ok(me) = std::env::current_exe() else { return fail("Couldn't find Potato.exe") };
        let old = me.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(&me, &old).is_err() {
            return fail("Couldn't replace Potato.exe");
        }
        if std::fs::write(&me, &exe).is_err() {
            let _ = std::fs::rename(&old, &me);   // put the working one back
            return fail("Couldn't replace Potato.exe");
        }
        if std::process::Command::new(&me).spawn().is_ok() {
            std::process::exit(0);
        }
        fail("Updated. Close Potato and open it again.");
    });
}

/// Run a Remote Desktop action (`busy` names it for the page) on its own thread, then look everything up again.
/// The engine isn't locked meanwhile, so the window keeps answering while Remote Desktop restarts.
pub fn refresh_rdp(rdp: Arc<Mutex<crate::rdp::Rdp>>, busy: Option<&'static str>, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
    {
        let mut r = rdp.lock().unwrap();
        if busy.is_some() && r.busy.is_some() {
            return;
        }
        r.busy = busy.or(r.busy);
    }
    std::thread::spawn(move || {
        let done = work();
        let mut fresh = crate::rdp::look();
        let mut r = rdp.lock().unwrap();
        fresh.busy = if busy.is_some() { None } else { r.busy };
        fresh.msg = match done {
            Err(e) => Some(e),
            Ok(()) => busy.map(|b| match b {
                "patch" => "Every Account Can Be On At Once Now",
                "undo" => "Put Windows' Own File Back",
                "add" => "Account Ready: Click Open",
                _ => "Done",
            }.to_string()).or(r.msg.take()),
        };
        *r = fresh;
    });
}
