// The exe's own icon and name (Explorer, taskbar, Task Manager). Only when building on Windows, where the resource compiler is.
fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // the name Windows shows (Task Manager, notifications, Startup apps); it defaults to the crate name "potato"
        winresource::WindowsResource::new().set_icon("assets/icon.ico").set("ProductName", "Potato").set("FileDescription", "Potato")
            .set("OriginalFilename", "Potato.exe").set("InternalName", "Potato").compile().expect("embed the icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
