//! Open Maximized. Windows only lets a program move windows in its own login session, so the main app does
//! its own session and a hidden copy of Potato (`Potato.exe --helper`) runs in every other one, RDP logins
//! included. A scheduled task starts that copy whenever any user logs on.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenMutexW, OpenProcess, PROCESS_NAME_WIN32, SYNCHRONIZATION_SYNCHRONIZE, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, IsZoomed, SW_MAXIMIZE, ShowWindow,
};
use windows::core::{BOOL, PWSTR, w};

const TASK: &str = "Potato Helper";
/// a new Roblox window is kept maximized this long (Roblox resizes itself while it loads), then left alone
const SETTLE: Duration = Duration::from_secs(30);

/// Remembers when each Roblox window was first seen.
#[derive(Default)]
pub struct Maximizer {
    seen: HashMap<isize, Instant>,
}

impl Maximizer {
    /// Maximize Roblox windows in this session that opened in the last few seconds and aren't maximized.
    /// Minimized windows are skipped, and older windows are never touched, so people can still resize them.
    pub fn tick(&mut self) {
        let mut windows: Vec<isize> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize));
        }
        let now = Instant::now();
        self.seen.retain(|h, _| windows.contains(h));
        for h in windows {
            let first = *self.seen.entry(h).or_insert(now);
            let hwnd = HWND(h as *mut _);
            if now - first < SETTLE && unsafe { !IsZoomed(hwnd).as_bool() && !IsIconic(hwnd).as_bool() } {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                }
            }
        }
    }
}

unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> BOOL {
    let out = unsafe { &mut *(out.0 as *mut Vec<isize>) };
    if unsafe { IsWindowVisible(hwnd).as_bool() } && is_roblox(hwnd) {
        out.push(hwnd.0 as isize);
    }
    BOOL(1)
}

fn is_roblox(hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
    let _ = unsafe { CloseHandle(h) };
    ok && String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase().ends_with("\\robloxplayerbeta.exe")
}

/// `Potato.exe --helper`: no window; maximizes new Roblox windows in this session while this account has
/// Open Maximized on. Reads the settings every second, so changes in the app reach it without a restart.
pub fn run() {
    // one helper per session ("Local\\" names are per session)
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\PotatoHelper")) };
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }
    let me = crate::system::current_sid();
    let mut m = Maximizer::default();
    let mut jobs = SessionJobs::default();
    loop {
        let config = std::fs::read_to_string(crate::engine::config_path()).map(|t| crate::config::Config::from_json(&t)).unwrap_or_default();
        let a = config.account(&me);
        if a.on && a.plan.maximized {
            m.tick();
        }
        session_tick(&me, &mut jobs);
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// What only code inside a login can do for that login's Roblox: rejoins, muting, spotting freezes and
/// taking screenshots for the Discord report. Run every second by the helper (or by the app for its own login).
#[derive(Default)]
pub struct SessionJobs {
    rejoined: u64,
    /// Roblox clients in this login with no window yet, and since when (see config::stuck_clients)
    windowless: HashMap<u32, Instant>,
    muted: HashSet<u32>,
    hung: HashSet<isize>,
    freezes: u32,
    shot_done: u64,
    /// Roblox's one-client lock, held while Multiple Roblox At Once is on
    multi: Vec<isize>,
}

/// Multiple Roblox At Once, like Roblox Account Manager: a new Roblox closes the others in its login when it finds
/// its lock free, so this login keeps the lock (the old mutex and the newer event) for as long as the setting is on.
fn hold_multi(on: bool, held: &mut Vec<isize>) {
    if on == !held.is_empty() {
        return;
    }
    if on {
        unsafe {
            held.extend(CreateMutexW(None, true, w!("ROBLOX_singletonMutex")).ok().map(|h| h.0 as isize));
            held.extend(CreateEventW(None, true, false, w!("ROBLOX_singletonEvent")).ok().map(|h| h.0 as isize));
        }
    } else {
        for h in held.drain(..) {
            let _ = unsafe { CloseHandle(HANDLE(h as *mut _)) };
        }
    }
}

/// Tidy Desktop, asked for on the Remote Desktop page: run once per request in this login, as this user.
fn serve_tidy() {
    let user = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
    let Ok(id) = std::fs::read_to_string(crate::engine::config_path().with_file_name("desktop").join(format!("{user}.txt"))) else { return };
    // the request sits in Potato's shared folder, which an alt can't delete from: remember what's done in its own profile
    let done = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Potato").join("desktop-done.txt");
    if std::fs::read_to_string(&done).is_ok_and(|d| d.trim() == id.trim()) {
        return;
    }
    let _ = std::fs::create_dir_all(done.parent().unwrap());
    let _ = std::fs::write(&done, id.trim());   // first, so a script that fails can't run again every second
    let _ = crate::system::powershell(TIDY, &[]);
}

/// Dark mode, no transparency, a black background, a bare taskbar (no search, Task View, news, People or chat).
/// Restarting Explorer also brings in the pins `rdp::request_tidy` set (File Explorer and the browser).
const TIDY: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
function Set-Dword($key, $name, $value) { New-Item $key -Force:$false -ErrorAction SilentlyContinue | Out-Null; Set-ItemProperty $key $name $value -Type DWord }
$adv = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
$theme = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
Set-Dword $theme AppsUseLightTheme 0
Set-Dword $theme SystemUsesLightTheme 0
Set-Dword $theme EnableTransparency 0
Set-Dword 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Search' SearchboxTaskbarMode 0
Set-Dword $adv ShowTaskViewButton 0
Set-Dword $adv ShowCortanaButton 0
Set-Dword $adv TaskbarMn 0
Set-Dword $adv TaskbarDa 0
Set-Dword "$adv\People" PeopleBand 0
Set-Dword 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Feeds' ShellFeedsTaskbarViewMode 2
Set-ItemProperty 'HKCU:\Control Panel\Colors' Background '0 0 0'
Set-ItemProperty 'HKCU:\Control Panel\Desktop' Wallpaper ''
Add-Type -Namespace Potato -Name W -MemberDefinition '[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool SystemParametersInfo(int a, int b, string c, int d); [DllImport("user32.dll")] public static extern bool SetSysColors(int n, int[] e, int[] c);'
[Potato.W]::SystemParametersInfo(20, 0, '', 3) | Out-Null
[Potato.W]::SetSysColors(1, @(1), @(0)) | Out-Null

# clear the pins while Explorer is closed (it saves its own list when it exits), then let it start again
$session = (Get-Process -Id $PID).SessionId
Get-Process explorer | Where-Object SessionId -eq $session | Stop-Process -Force
Start-Sleep 2
Remove-Item "$env:APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\*" -Force
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Taskband' -Recurse -Force
Start-Sleep 3
if (-not (Get-Process explorer | Where-Object SessionId -eq $session)) { Start-Process explorer.exe }

"#;

/// The folder where the helpers leave freeze counts and screenshots for the report (see engine.rs).
pub fn report_dir() -> PathBuf {
    crate::engine::config_path().with_file_name("report")
}

pub fn session_tick(me: &str, jobs: &mut SessionJobs) {
    serve_rejoins(me, &mut jobs.rejoined);
    let config = std::fs::read_to_string(crate::engine::config_path()).map(|t| crate::config::Config::from_json(&t)).unwrap_or_default();
    let a = config.account(me);
    mute(a.on && a.mute, &mut jobs.muted);
    hold_multi(a.on && a.multi_roblox, &mut jobs.multi);
    serve_tidy();

    let mut windows: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize));
    }
    // Roblox that never opened a window (it started without internet) piles up in the tray and makes new
    // launches quit at once: close it once it's been stuck for a while
    if a.on {
        let shown: HashSet<u32> = windows.iter().map(|h| {
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(HWND(*h as *mut _), Some(&mut pid)) };
            pid
        }).collect();
        let session = crate::system::current_session();
        let hidden: Vec<u32> = crate::system::clients().iter().filter(|c| c.session == session && !shown.contains(&c.pid)).map(|c| c.pid).collect();
        for pid in crate::config::stuck_clients(&mut jobs.windowless, &hidden, Instant::now()) {
            crate::system::close(pid);
        }
    }
    let hung: HashSet<isize> = windows.iter().copied().filter(|h| unsafe { IsHungAppWindow(HWND(*h as *mut _)).as_bool() }).collect();
    let before = jobs.freezes;
    jobs.freezes += hung.difference(&jobs.hung).count() as u32;   // each window counts once per freeze
    jobs.hung = hung;

    let dir = report_dir();
    let asked = std::fs::read_to_string(dir.join("request.txt")).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
    let mut shots = None;
    if asked > jobs.shot_done {
        jobs.shot_done = asked;
        let _ = std::fs::create_dir_all(&dir);
        shots = Some(windows.iter().enumerate().filter(|(i, h)| capture(**h, &dir.join(format!("{me}-{i}.png")))).count());
    }
    if shots.is_some() || jobs.freezes != before {
        let _ = std::fs::create_dir_all(&dir);
        let json = serde_json::json!({ "id": jobs.shot_done, "freezes": jobs.freezes, "frozenNow": jobs.hung.len(), "shots": shots });
        let _ = std::fs::write(dir.join(format!("{me}.json")), json.to_string());
    }
}

/// Mute (or unmute what Potato muted) every Roblox audio session in this login.
fn mute(on: bool, muted: &mut HashSet<u32>) {
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::core::Interface;
    if !on && muted.is_empty() {
        return;
    }
    let roblox: HashSet<u32> = crate::system::clients().iter().filter(|c| c.session == crate::system::current_session()).map(|c| c.pid).collect();
    let _ = (|| -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let manager: IAudioSessionManager2 = devices.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for i in 0..sessions.GetCount()? {
                let session = sessions.GetSession(i)?;
                let pid = session.cast::<IAudioSessionControl2>()?.GetProcessId()?;
                let volume = session.cast::<ISimpleAudioVolume>()?;
                if on && roblox.contains(&pid) && !muted.contains(&pid) {
                    volume.SetMute(true, std::ptr::null())?;
                    muted.insert(pid);
                } else if !on && muted.remove(&pid) {
                    volume.SetMute(false, std::ptr::null())?;
                }
            }
        }
        Ok(())
    })();
    muted.retain(|pid| roblox.contains(pid));
}

/// Save a Roblox window as a PNG, at most 960 px wide. Works while it's covered; a minimized window, or one in
/// a disconnected RDP login, has nothing drawn and is skipped.
fn capture(hwnd: isize, path: &std::path::Path) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    let h = HWND(hwnd as *mut _);
    let mut r = RECT::default();
    if unsafe { IsIconic(h).as_bool() || GetClientRect(h, &mut r).is_err() } {
        return false;
    }
    let (w, ht) = (r.right - r.left, r.bottom - r.top);
    if w < 50 || ht < 50 {
        return false;
    }
    let mut pixels = vec![0u8; (w * ht * 4) as usize];
    let ok = unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, ht);
        let old = SelectObject(dc, bmp.into());
        let drawn = PrintWindow(h, dc, PRINT_WINDOW_FLAGS(3)).as_bool();   // client area, full content (works for DirectX)
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -ht, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let got = GetDIBits(dc, bmp, 0, ht as u32, Some(pixels.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        drawn && got == ht
    };
    if !ok || pixels.chunks(4).all(|p| p[..3] == [0, 0, 0]) {
        return false;   // nothing drawn
    }
    let step = (w as usize).div_ceil(960).max(1);
    let (ow, oh) = ((w as usize).div_ceil(step), (ht as usize).div_ceil(step));
    let mut out = Vec::with_capacity(ow * oh * 3);
    for y in (0..ht as usize).step_by(step) {
        for x in (0..w as usize).step_by(step) {
            let p = (y * w as usize + x) * 4;
            out.extend_from_slice(&[pixels[p + 2], pixels[p + 1], pixels[p]]);   // BGRA -> RGB
        }
    }
    let Ok(file) = std::fs::File::create(path) else { return false };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), ow as u32, oh as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&out)).is_ok()
}

/// The folder where the app leaves rejoin requests, one file per account (see rejoin.rs).
pub fn rejoin_dir() -> PathBuf {
    crate::engine::config_path().with_file_name("rejoin")
}

/// If the app asked this account's clients to rejoin: wait for them to close, then open each join link, a few
/// seconds apart so Roblox isn't starting two at once. `done` is the last request handled, so each runs once.
pub fn serve_rejoins(me: &str, done: &mut u64) {
    let Ok(text) = std::fs::read_to_string(rejoin_dir().join(format!("{me}.txt"))) else { return };
    let Some((id, links)) = crate::rejoin::read_request(&text) else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    if id == *done || now.saturating_sub(id) > 10 * 60 * 1000 {
        return;   // already handled, or left over from long ago
    }
    *done = id;
    let session = crate::system::current_session();
    let start = Instant::now();
    while crate::system::clients().iter().any(|c| c.session == session) && start.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
    }
    for (i, link) in links.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(Duration::from_secs(10));
        }
        crate::system::launch(link);
    }
}

/// Whether a helper is already running in this session (it then handles this account's rejoins).
pub fn running_here() -> bool {
    unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\PotatoHelper")) }.is_ok_and(|h| {
        let _ = unsafe { CloseHandle(h) };
        true
    })
}

/// Copy Potato next to its settings and register the logon task, then start it in every session that's
/// already logged on. Returns false if Windows refused (needs admin).
pub fn install() -> bool {
    let Ok(me) = std::env::current_exe() else { return false };
    let exe = crate::engine::installed_exe();
    if std::fs::read(&me).ok() != std::fs::read(&exe).ok() {
        let _ = std::fs::create_dir_all(exe.parent().unwrap());
        let _ = std::fs::rename(&exe, exe.with_extension("old"));   // a running helper's exe can be renamed, not overwritten
        if std::fs::copy(&me, &exe).is_err() {
            return false;
        }
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    // every member of Users, at their own logon, in their own session, without admin rights
    let xml = format!(r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals><Principal id="Users"><GroupId>S-1-5-32-545</GroupId><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Hidden>true</Hidden></Settings>
  <Actions Context="Users"><Exec><Command>{}</Command><Arguments>--helper</Arguments></Exec></Actions>
</Task>"#, exe.display());
    crate::system::set_task(TASK, Some(&xml)) && crate::system::run("schtasks", &["/Run", "/TN", TASK])
}

/// Remove the logon task. Running helpers stay idle until their user logs off.
pub fn uninstall() {
    crate::system::set_task(TASK, None);
}
