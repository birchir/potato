//! The parts of preview.rs that are plain maths and text, shared with the demo build and tested anywhere.

/// A rectangle in physical pixels: left, top, right, bottom.
pub type Px = [i32; 4];

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// a Roblox client's own window on this desktop
    Roblox,
    /// a Remote Desktop window, and the account it's signed in to
    Rdp(String),
}

#[derive(Clone, Debug)]
pub struct Win {
    pub hwnd: isize,
    pub pid: u32,
    pub kind: Kind,
    pub minimized: bool,
}

/// The title Potato gives an account's Remote Desktop window.
pub fn rdp_title(name: &str) -> String {
    format!("{name} - Remote Desktop")
}

/// Which account a Remote Desktop window is signed in to, from its title: Potato's "<name> - Remote Desktop",
/// RDP Plus's `/title:"<name>"`, or Windows' own "<address> - Remote Desktop Connection" ("<file> - <address> - ..."
/// when opened from an .rdp file).
pub fn rdp_account(title: &str, accounts: &[String]) -> Option<String> {
    let parts: Vec<&str> = title.split(" - ").map(str::trim).collect();
    accounts.iter().find(|n| parts.iter().any(|p| p.eq_ignore_ascii_case(n) || *p == crate::rdp::address(n))).cloned()
}

/// What to call a Remote Desktop window no account matches: its title without Windows' " - Remote Desktop Connection".
pub fn rdp_label(title: &str) -> String {
    let t = title.trim_end_matches(" - Remote Desktop Connection").trim();
    if t.is_empty() { "Remote Desktop".into() } else { t.to_string() }
}

/// Fit a `src` sized picture inside `tile` (keeping its shape, centered), then cut it to `clip`. Returns where it
/// goes on screen and the matching part of the source, or None when none of it is on screen.
pub fn fit(tile: Px, clip: Px, src: (i32, i32)) -> Option<(Px, Px)> {
    let (sw, sh) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (tw, th) = ((tile[2] - tile[0]) as f32, (tile[3] - tile[1]) as f32);
    let k = (tw / sw).min(th / sh);
    let (w, h) = (sw * k, sh * k);
    let (x0, y0) = (tile[0] as f32 + (tw - w) / 2.0, tile[1] as f32 + (th - h) / 2.0);
    let d = [x0.max(clip[0] as f32), y0.max(clip[1] as f32), (x0 + w).min(clip[2] as f32), (y0 + h).min(clip[3] as f32)];
    if d[2] - d[0] < 1.0 || d[3] - d[1] < 1.0 {
        return None;
    }
    let s = [(d[0] - x0) / k, (d[1] - y0) / k, (d[2] - x0) / k, (d[3] - y0) / k];
    Some((d.map(|v| v.round() as i32), s.map(|v| v.round() as i32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_and_clips() {
        // a 1366x768 window in a 400x300 tile: full width, centered top to bottom
        let (d, s) = fit([0, 0, 400, 300], [0, 0, 2000, 2000], (1366, 768)).unwrap();
        assert_eq!(d, [0, 38, 400, 262]);
        assert_eq!(s, [0, 0, 1366, 768]);
        // the page scrolled so only the bottom half of the tile shows: only the bottom part of the window is drawn
        let (d, s) = fit([0, 0, 400, 300], [0, 150, 2000, 2000], (1366, 768)).unwrap();
        assert_eq!((d[1], d[3]), (150, 262));
        assert_eq!((s[1], s[3]), (384, 768));
        assert!(fit([0, 0, 400, 300], [0, 400, 2000, 2000], (1366, 768)).is_none());
    }

    #[test]
    fn finds_the_account() {
        let names = vec!["Alt1".to_string(), "Alt2".to_string()];
        let addr = crate::rdp::address("Alt2");
        assert_eq!(rdp_account(&format!("{addr} - Remote Desktop Connection"), &names), Some("Alt2".into()));
        assert_eq!(rdp_account("Alt1 - Remote Desktop", &names), Some("Alt1".into()));
        assert_eq!(rdp_account(&format!("Alt2 - {addr} - Remote Desktop Connection"), &names), Some("Alt2".into()), "opened from Alt2.rdp");
        assert_eq!(rdp_account("192.168.1.5 - Remote Desktop Connection", &names), None);
        assert_eq!(rdp_account("Alt2", &names), Some("Alt2".into()), "RDP Plus /title:\"Alt2\"");
        assert_eq!(rdp_account("alt1", &names), Some("Alt1".into()));
        assert_eq!(rdp_account("Alt10", &names), None, "not a different account's name");
        assert_eq!(rdp_label("127.0.0.9 - Remote Desktop Connection"), "127.0.0.9");
    }
}
