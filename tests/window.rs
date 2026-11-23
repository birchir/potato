//! Draws the real window with fake accounts (src/system_demo.rs) and saves target/window-*.png, so the layout
//! can be checked without Windows: an account page at the default size (and its Trim All result) and maximized
//! on a 1080p screen, and the Whole PC page. Needs a GPU or a software Vulkan driver (lavapipe): cargo test --test window -- --ignored
#![cfg(not(windows))]

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Every test run gets its own empty settings folder (the demo's ProgramData), so what the clicks save can be checked.
fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("test-programdata");
        let _ = std::fs::remove_dir_all(&d);
        unsafe { std::env::set_var("ProgramData", &d) };   // before any engine starts: the tests share it
        d
    }).clone()
}

fn harness(size: [f32; 2]) -> Harness<'static, potato::app::App> {
    data_dir();
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(2.0).with_step_dt(1.0 / 60.0).build_eframe(|cc| potato::app::App::new(cc));
    h.ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    h.run_steps(6);
    h
}

fn save(h: &mut Harness<'static, potato::app::App>, name: &str) {
    let image = h.render().expect("render");
    image.save(format!("{}/target/window-{name}.png", env!("CARGO_MANIFEST_DIR"))).expect("save");
}

#[test]
#[ignore]
fn draw_the_window() {
    let mut h = harness([1000.0, 760.0]);
    save(&mut h, "default");
    h.get_by_label("Trim All").click();
    for _ in 0..40 {   // the trim runs on its own thread, then its result floats up
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.run_steps(1);
    }
    save(&mut h, "toast");
    save(&mut harness([1920.0, 1000.0]), "maximized");
    let mut h = harness([1000.0, 1300.0]);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Performance").click();
    h.run_steps(4);
    save(&mut h, "whole-pc");
    let mut h = harness([1000.0, 1100.0]);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Remote").click();
    h.run_steps(4);
    save(&mut h, "rdp");
}

fn steps(h: &mut Harness<'static, potato::app::App>, n: usize) {
    for _ in 0..n {
        std::thread::sleep(std::time::Duration::from_millis(30));
        h.run_steps(1);
    }
}

fn saved() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(data_dir().join("Potato").join("settings.json")).expect("settings saved")).unwrap()
}

/// Clicks and types through every control like a person would, then checks what was saved.
#[test]
#[ignore]
fn click_and_type_everywhere() {
    use egui::accesskit::Role;
    let mut h = harness([1400.0, 1100.0]);
    // Overview: trim one live tile, change the tile size
    h.get_all_by_label("Trim").next().expect("a tile with Trim").click();
    let mut said = false;
    for _ in 0..160 {   // the trim runs on its own thread: wait up to ~5 s for its result
        steps(&mut h, 1);
        if h.query_by_label_contains("Trimmed Alt1").is_some() {
            said = true;
            break;
        }
    }
    assert!(said, "the tile's trim says what it freed");
    h.get_by_role_and_label(Role::RadioButton, "Large").click();
    steps(&mut h, 4);
    save(&mut h, "clicked-overview");

    // Accounts: open Alt1's card, pick a preset, then type an FPS cap over it
    h.get_by_role_and_label(Role::Tab, "Accounts").click();
    steps(&mut h, 4);
    save(&mut h, "clicked-accounts");
    h.get_by_role_and_label(Role::Button, "Alt1").click();
    steps(&mut h, 4);
    h.get_by_role_and_label(Role::RadioButton, "Max Savings").click();
    steps(&mut h, 4);
    let a = &saved()["accounts"]["S-1-5-21-3"];
    assert_eq!((a["ultraLow"].as_bool(), a["fpsCap"].as_u64(), a["priority"].as_str()), (Some(true), Some(30), Some("belowNormal")), "Max Savings");
    let fps = h.get_by_label("FPS Cap");
    fps.focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("FPS Cap").type_text("45 fps");
    steps(&mut h, 4);
    assert_eq!(saved()["accounts"]["S-1-5-21-3"]["fpsCap"], 45, "typing 45 fps saves 45");
    h.get_by_role_and_label(Role::RadioButton, "Low").click();   // priority
    steps(&mut h, 4);
    assert_eq!(saved()["accounts"]["S-1-5-21-3"]["priority"], "idle");
    save(&mut h, "clicked-account");

    // Performance: the guide checklist's one button, then the core split by hand
    h.get_by_role_and_label(Role::Tab, "Performance").click();
    steps(&mut h, 10);
    h.get_by_role_and_label(Role::Button, "Apply Recommended").click();
    steps(&mut h, 30);
    assert_eq!(saved()["helperCores"], 2, "Apply Recommended put Remote Desktops on their own cores");
    let p = potato::power::read();
    assert!(p.high_performance && p.boost_off && p.fast_startup_off, "and applied every power setting");
    assert!(h.get_all_by_role_and_label(Role::Button, "Undo").count() >= 4, "done items offer Undo");
    h.get_by_label("Cores For Remote Desktop And Macros").click();
    steps(&mut h, 4);
    h.get_by_label("Last 3 Cores").click();
    steps(&mut h, 4);
    assert_eq!(saved()["helperCores"], 3);
    save(&mut h, "clicked-performance");

    // Remote Desktop: boosting size, a desktop shortcut, a new account typed in
    h.get_by_role_and_label(Role::Tab, "Remote").click();
    steps(&mut h, 6);
    h.get_all_by_role_and_label(Role::RadioButton, "1800x768 Boosting").next().expect("Alt1's size").click();
    steps(&mut h, 4);
    assert_eq!(saved()["rdpWide"], serde_json::json!(["Alt1"]));
    let file = std::fs::read_to_string(data_dir().join("Potato").join("rdp").join("Alt1.rdp")).expect("Alt1.rdp written");
    for line in ["desktopwidth:i:1800", "audiomode:i:2", "redirectdrives:i:0", "disable wallpaper:i:1", "keyboardhook:i:0"] {
        assert!(file.contains(line), "{line}");
    }
    h.get_all_by_label("Desktop Shortcut").next().unwrap().click();
    steps(&mut h, 6);
    assert!(h.query_by_label_contains("On Your Desktop").is_some(), "the shortcut says where it went");
    h.get_by_role_and_label(Role::TextInput, "Name").focus();
    steps(&mut h, 2);
    h.get_by_role_and_label(Role::TextInput, "Name").type_text("Alt3");
    h.get_by_role_and_label(Role::PasswordInput, "Password").focus();
    steps(&mut h, 2);
    h.get_by_role_and_label(Role::PasswordInput, "Password").type_text("Potato123!");
    steps(&mut h, 2);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Name").value().as_deref(), Some("Alt3"), "the name was typed");
    steps(&mut h, 4);
    h.get_by_role_and_label(Role::Button, "Make It").click();
    steps(&mut h, 30);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Name").value().as_deref(), Some(""), "the name box clears once the account is made");
    save(&mut h, "clicked-rdp");

    // Settings: a wrong webhook is caught as it's typed
    h.get_by_role_and_label(Role::Tab, "Settings").click();
    steps(&mut h, 4);
    h.get_by_label("Webhook").focus();
    steps(&mut h, 2);
    h.get_by_label("Webhook").type_text("https://example.com/not-discord");
    steps(&mut h, 4);
    save(&mut h, "clicked-settings");
    assert!(h.query_by_label("Not a Discord webhook").is_some());
}
