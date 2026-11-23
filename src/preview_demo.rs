//! preview.rs faked for the demo build: two open Remote Desktops and one Roblox window, drawn as placeholders.

pub use crate::preview_shared::*;

pub fn windows(accounts: &[String]) -> Vec<Win> {
    let mut out: Vec<Win> = accounts.iter().take(2).enumerate()
        .map(|(i, n)| Win { hwnd: 10 + i as isize, pid: 2000 + i as u32, kind: Kind::Rdp(n.clone()), minimized: false }).collect();
    out.push(Win { hwnd: 99, pid: 1001, kind: Kind::Roblox, minimized: false });
    out
}

pub fn name_rdp_windows(_: &[String]) {}

pub fn focus(_: isize) {}

pub struct Thumbs;

impl Thumbs {
    pub fn new(_: Option<isize>) -> Thumbs {
        Thumbs
    }

    pub fn show(&mut self, _: isize, _: Px, _: Px) -> bool {
        false
    }

    pub fn end_frame(&mut self) {}
}
