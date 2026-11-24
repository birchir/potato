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
        // each demo account signed in to one of Roblox's own accounts, so real avatars load
        for (name, id, user) in [("Roblox", 1, "Roblox"), ("Main", 156, "builderman"), ("Alt1", 2, "John Doe"), ("Alt2", 3, "Jane Doe")] {
            let dir = std::env::temp_dir().join("potato-demo").join(name).join("Roblox").join("LocalStorage");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("appStorage.json"), format!(r#"{{"UserId":"{id}","Username":"{user}"}}"#)).unwrap();
        }
        d
    }).clone()
}

fn harness(size: [f32; 2]) -> Harness<'static, potato::app::App> {
    data_dir();
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(2.0).with_step_dt(1.0 / 60.0).build_eframe(|cc| potato::app::App::new(cc));
    h.ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    h.run_steps(6);
    for _ in 0..60 {   // the avatars download in the background
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.run_steps(1);
    }
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
    h.get_by_label("Free Roblox RAM").click();
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
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Remote Desktop").click();
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
    // Overview: free one tile's RAM, then every picture size really changes the pictures
    h.get_all_by_label("Free RAM").next().expect("a tile with Free RAM").click();
    let mut said = false;
    for _ in 0..160 {   // it runs on its own thread: wait up to ~5 s for its result
        steps(&mut h, 1);
        if h.query_by_label_contains("From Alt1").is_some() {
            said = true;
            break;
        }
    }
    assert!(said, "the tile says what it freed");
    let width = |h: &Harness<'static, potato::app::App>| h.get_by_label("Picture Of Alt1").rect().width();
    let mut widths = Vec::new();
    for size in ["Small", "Medium", "Large"] {
        h.get_by_role_and_label(Role::RadioButton, size).click();
        steps(&mut h, 4);
        widths.push(width(&h));
    }
    assert!(widths[0] < widths[1] && widths[1] < widths[2], "Small, Medium and Large each change the pictures: {widths:?}");
    // the top bar's buttons each do something
    h.get_by_label("Free PC RAM").click();
    let mut said = false;
    for _ in 0..160 {
        steps(&mut h, 1);
        if h.query_by_label_contains("Of PC RAM").is_some() {
            said = true;
            break;
        }
    }
    assert!(said, "Free PC RAM says what it freed");
    h.get_by_label("Restart Roblox").click();
    steps(&mut h, 2);
    assert!(h.query_by_label("Click Again To Restart").is_some(), "Restart Roblox asks for a second click");
    h.get_by_label("Click Again To Restart").click();
    let mut said = false;
    for _ in 0..160 {
        steps(&mut h, 1);
        if h.query_all_by_label_contains("Couldn't Tell Which Server").count() + h.query_all_by_label_contains("Restarting").count() > 0 {
            said = true;
            break;
        }
    }
    assert!(said, "the second click restarts");
    save(&mut h, "clicked-overview");

    // Accounts: open Alt1's card, pick a preset, then type an FPS cap over it
    h.get_by_role_and_label(Role::Tab, "Accounts").click();
    steps(&mut h, 4);
    assert!(h.query_by_label("@builderman").is_some(), "each account shows its Roblox username");
    save(&mut h, "clicked-accounts");
    // the sidebar's account goes straight to its page
    h.get_by_role_and_label(Role::Tab, "Alt2").click();
    steps(&mut h, 4);
    assert!(h.query_by_role_and_label(Role::Switch, "Let Potato Run This Account").is_some(), "Alt2's page");
    h.get_by_role_and_label(Role::Button, "Accounts").click();   // the top bar's way back
    steps(&mut h, 4);
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
    h.get_by_role_and_label(Role::Switch, "Mute").click();
    steps(&mut h, 4);
    assert_eq!(saved()["accounts"]["S-1-5-21-3"]["mute"], false, "Mute switch flips");
    save(&mut h, "clicked-account");

    // Performance: the guide checklist's one button, then the core split by hand
    h.get_by_role_and_label(Role::Tab, "Performance").click();
    steps(&mut h, 10);
    h.get_by_role_and_label(Role::Button, "Turn On All Of These").click();
    steps(&mut h, 30);
    assert_eq!(saved()["helperCores"], 2, "Turn On All put Remote Desktops on their own cores");
    let p = potato::power::read();
    assert!(p.high_performance && p.boost_off && p.fast_startup_off, "and applied every power setting");
    assert!(!p.updates_off, "but never stops Windows Updates by itself");
    assert!(h.get_all_by_role_and_label(Role::Button, "Undo").count() >= 4, "done items offer Undo");
    h.get_all_by_role_and_label(Role::Button, "Undo").nth(2).unwrap().click();   // the power plan (after the RAM timer and cores)
    steps(&mut h, 30);
    assert!(!potato::power::read().high_performance, "Undo puts the power plan back");
    h.get_by_role_and_label(Role::Button, "Stop Windows Updates").click();
    steps(&mut h, 30);
    assert!(potato::power::read().updates_off && saved()["blockUpdates"] == true, "Stop Windows Updates stops them and remembers it");
    assert!(h.query_all_by_label_contains("Microsoft Store").count() >= 2, "and warns about the Microsoft Store, on the page and when it's done");
    h.get_by_label("Cores For Remote Desktop And Macros").click();
    steps(&mut h, 4);
    h.get_by_label("Last 3 Cores").click();
    steps(&mut h, 4);
    assert_eq!(saved()["helperCores"], 3);
    save(&mut h, "clicked-performance");

    // Remote Desktop: boosting size, a desktop shortcut, a new account typed in
    h.get_by_role_and_label(Role::Tab, "Remote Desktop").click();
    steps(&mut h, 6);
    h.get_by_role_and_label(Role::Button, "Stop Windows Updates").click();   // now its Undo, shown here too
    steps(&mut h, 30);
    assert!(!potato::power::read().updates_off && saved()["blockUpdates"] == false, "Undo turns updates back on");
    h.get_all_by_role_and_label(Role::RadioButton, "Wide 1800x768").next().expect("Alt1's size").click();
    steps(&mut h, 4);
    assert_eq!(saved()["rdpWide"], serde_json::json!(["Alt1"]));
    let file = std::fs::read_to_string(data_dir().join("Potato").join("rdp").join("Alt1.rdp")).expect("Alt1.rdp written");
    for line in ["desktopwidth:i:1800", "audiomode:i:2", "redirectdrives:i:0", "disable wallpaper:i:1", "keyboardhook:i:0"] {
        assert!(file.contains(line), "{line}");
    }
    h.get_all_by_label("Open When I Sign In To Windows").next().expect("Alt1's checkbox").click();
    steps(&mut h, 4);
    assert_eq!(saved()["rdpStartup"], serde_json::json!(["Alt1"]), "the sign-in checkbox saves");
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
    h.get_by_role_and_label(Role::Button, "Add Alt").click();
    steps(&mut h, 30);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Name").value().as_deref(), Some(""), "the name box clears once the account is made");
    save(&mut h, "clicked-rdp");

    // Settings: a wrong webhook is caught as it's typed
    h.get_by_role_and_label(Role::Tab, "Settings").click();
    steps(&mut h, 4);
    h.get_by_role_and_label(Role::Switch, "Tell Me When RAM Is Freed").click();
    steps(&mut h, 4);
    assert_eq!(saved()["notify"], false, "the notification switch flips");
    h.get_by_label("Webhook").focus();
    steps(&mut h, 2);
    h.get_by_label("Webhook").type_text("https://example.com/not-discord");
    steps(&mut h, 4);
    save(&mut h, "clicked-settings");
    assert!(h.query_by_label("Not A Discord Webhook").is_some());
}
