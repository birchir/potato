//! The tray icon next to the clock. Closing the window only hides it; Potato keeps working (engine.rs) until
//! Quit Potato in the tray menu.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow};

use crate::engine::Engine;

/// Keep the returned icon alive for as long as Potato runs; dropping it removes it from the tray.
pub fn create(hwnd: isize, ctx: egui::Context, engine: Arc<Mutex<Engine>>) -> Option<TrayIcon> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok()?;
    let (open, trim, quit) = (MenuItem::new("Open Potato", true, None), MenuItem::new("Trim All Roblox", true, None), MenuItem::new("Quit Potato", true, None));
    let menu = Menu::new();
    menu.append_items(&[&open, &trim, &quit]).ok()?;
    let tray = TrayIconBuilder::new().with_tooltip("Potato").with_menu(Box::new(menu)).with_menu_on_left_click(false)
        .with_icon(Icon::from_rgba(icon.rgba, icon.width, icon.height).ok()?).build().ok()?;
    let (open, trim, quit) = (open.id().clone(), trim.id().clone(), quit.id().clone());
    std::thread::spawn(move || loop {
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if e.id == open {
                show(hwnd, &ctx);
            } else if e.id == trim {
                engine.lock().unwrap().trim_all(true);
            } else if e.id == quit {
                std::process::exit(0);   // settings are saved the moment they change
            }
        }
        while let Ok(e) = TrayIconEvent::receiver().try_recv() {
            if matches!(e, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) {
                show(hwnd, &ctx);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    });
    Some(tray)
}

pub fn show(hwnd: isize, ctx: &egui::Context) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
    }
    ctx.request_repaint();
}

pub fn hide(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE);
    }
}

/// The name on the taskbar's right-click menu. Windows otherwise takes it from a cache of the exe's name made on first
/// run, so anyone who ran 0.1.0 (named "potato") kept seeing lowercase. Setting it on the window skips that cache.
pub fn name_taskbar(hwnd: isize) {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
    use windows::Win32::UI::Shell::SHStrDupW;
    use windows::core::{GUID, HSTRING};

    let Ok(exe) = std::env::current_exe() else { return };
    // System.AppUserModel.RelaunchCommand (2), .RelaunchDisplayNameResource (4), .ID (5); Windows wants the ID last
    let props = [(2, format!("\"{}\"", exe.display())), (4, "Potato".to_string()), (5, "birchir.Potato".to_string())];
    unsafe {
        let Ok(store) = SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd as *mut _)) else { return };
        for (pid, val) in props {
            let Ok(text) = SHStrDupW(&HSTRING::from(val)) else { return };
            let mut pv = PROPVARIANT::default();   // dropping it frees the string (PropVariantClear)
            let inner = &mut *pv.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = text;
            let _ = store.SetValue(&PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid }, &pv);
        }
        let _ = store.Commit();
    }
}
