//! Live pictures of the Roblox and Remote Desktop windows inside Potato's own window, like RoAlt's previews.
//! Windows' compositor (DWM) draws a live thumbnail of each window straight onto ours, the same way the taskbar's
//! hover previews work: no screenshots, next to no CPU. A Roblox client in a Remote Desktop has no window on this
//! desktop, so it shows inside that Remote Desktop's window.

use std::collections::HashMap;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE, DWM_TNP_SOURCECLIENTAREAONLY,
    DWM_TNP_VISIBLE, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow, SetWindowTextW, ShowWindow,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

pub use crate::preview_shared::*;

/// Every top-level window that's a Roblox client in this login or a Remote Desktop window. Remote Desktop
/// windows are found by their window class, so ones opened by RDP Plus or any other launcher count too.
fn top_windows() -> Vec<(isize, u32, bool, String)> {
    let mut hwnds: Vec<isize> = Vec::new();
    unsafe extern "system" fn add(hwnd: HWND, out: LPARAM) -> BOOL {
        if unsafe { IsWindowVisible(hwnd).as_bool() } {
            unsafe { &mut *(out.0 as *mut Vec<isize>) }.push(hwnd.0 as isize);
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(add), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let mut exe_of: HashMap<u32, String> = HashMap::new();
    hwnds.into_iter().filter_map(|h| {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(HWND(h as *mut _), Some(&mut pid)) };
        let rdp = class(h) == RDP_CLASS;
        let exe = exe_of.entry(pid).or_insert_with(|| exe_name(pid)).clone();
        (rdp || exe == "robloxplayerbeta.exe").then(|| (h, pid, rdp, title(h)))
    }).collect()
}

fn exe_name(pid: u32) -> String {
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return String::new() };
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
    let _ = unsafe { CloseHandle(h) };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
    path.rsplit('\\').next().unwrap_or_default().to_string()
}

/// The window class of every Windows Remote Desktop window (mstsc), whatever opened it.
const RDP_CLASS: &str = "TscShellContainerClass";

fn class(h: isize) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(HWND(h as *mut _), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn title(h: isize) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(HWND(h as *mut _), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The windows to preview: each Remote Desktop window (matched to its account) and each Roblox window on this desktop.
pub fn windows(accounts: &[String]) -> Vec<Win> {
    top_windows().into_iter().map(|(hwnd, pid, rdp, t)| {
        // a Remote Desktop no account matches still shows, under its own title
        let kind = if rdp { Kind::Rdp(rdp_account(&t, accounts).unwrap_or_else(|| rdp_label(&t))) } else { Kind::Roblox };
        Win { hwnd, pid, kind, minimized: unsafe { IsIconic(HWND(hwnd as *mut _)).as_bool() } }
    }).collect()
}

/// Give each Remote Desktop window its account's name, like the guide's RDP Plus `/title:` (Windows' own
/// Remote Desktop shows only the 127.x address). Run every tick: mstsc sets its title again when it reconnects.
pub fn name_rdp_windows(accounts: &[String]) {
    for (hwnd, _, rdp, t) in top_windows() {
        // only Windows' own "<address> - ..." titles: a title someone set (RDP Plus /title:) stays
        if rdp && let Some(name) = rdp_account(&t, accounts) && t.split(" - ").any(|p| p == crate::rdp::address(&name)) {
            let wide: Vec<u16> = rdp_title(&name).encode_utf16().chain([0]).collect();
            unsafe {
                let _ = SetWindowTextW(HWND(hwnd as *mut _), PCWSTR(wide.as_ptr()));
            }
        }
    }
}

/// Bring a window to the front (restoring it if it's minimized).
pub fn focus(hwnd: isize) {
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}

/// The live thumbnails drawn on Potato's window this frame. Ones not shown again by `end_frame` are removed.
pub struct Thumbs {
    dest: Option<isize>,
    live: HashMap<isize, (isize, bool)>,
}

impl Thumbs {
    pub fn new(dest: Option<isize>) -> Thumbs {
        Thumbs { dest, live: HashMap::new() }
    }

    /// Draw `src` fitted inside `tile` (physical pixels in Potato's window), cut to `clip` (the part of the page
    /// that's on screen). Returns false when it can't be drawn (no picture to show).
    pub fn show(&mut self, src: isize, tile: Px, clip: Px) -> bool {
        let Some(dest) = self.dest else { return false };
        let id = match self.live.get_mut(&src) {
            Some(t) => {
                t.1 = true;
                t.0
            }
            None => {
                let Ok(id) = (unsafe { DwmRegisterThumbnail(HWND(dest as *mut _), HWND(src as *mut _)) }) else { return false };
                self.live.insert(src, (id, true));
                id
            }
        };
        let Ok(size) = (unsafe { DwmQueryThumbnailSourceSize(id) }) else { return false };
        let (shown, source) = match fit(tile, clip, (size.cx, size.cy)) {
            Some(p) => (true, p),
            None => (false, ([0; 4], [0; 4])),
        };
        let rect = |r: Px| RECT { left: r[0], top: r[1], right: r[2], bottom: r[3] };
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: rect(source.0),
            rcSource: rect(source.1),
            opacity: 255,
            fVisible: BOOL(shown as i32),
            fSourceClientAreaOnly: BOOL(1),
        };
        unsafe { DwmUpdateThumbnailProperties(id, &props) }.is_ok() && size.cx > 0
    }

    /// Hide every thumbnail (a popup or another page is over them).
    pub fn end_frame(&mut self) {
        self.live.retain(|_, (id, used)| {
            let keep = std::mem::take(used);
            if !keep {
                let _ = unsafe { DwmUnregisterThumbnail(*id) };
            }
            keep
        });
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        for (id, _) in self.live.values() {
            let _ = unsafe { DwmUnregisterThumbnail(*id) };
        }
    }
}
