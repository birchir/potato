//! Windows settings Potato changes on a button: the guides' High Performance plan, CPU boost off and Fast Startup
//! off, and stopping Windows Update. They live in Windows, not in Potato's settings: Potato reads them from
//! Windows and changes them only when you press Apply or Undo, and they stay as they are when Potato closes.
//! (Stopping updates is the one Potato also keeps up: Windows turns its update service back on by itself.)

/// What Windows has now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub high_performance: bool,
    pub boost_off: bool,
    pub fast_startup_off: bool,
    pub updates_off: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    HighPerformance,
    BoostOff,
    FastStartupOff,
    UpdatesOff,
}

impl Item {
    pub fn is_on(self, s: &State) -> bool {
        match self {
            Item::HighPerformance => s.high_performance,
            Item::BoostOff => s.boost_off,
            Item::FastStartupOff => s.fast_startup_off,
            Item::UpdatesOff => s.updates_off,
        }
    }
}

/// Windows' built-in plans.
const HIGH: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
const ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";
#[cfg(windows)]
const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

/// Whether `powercfg /getactivescheme` names a high performance plan: Windows' own GUIDs, or a copy of one
/// (laptops hide High Performance, so Apply copies it and the copy has its own GUID and Windows' English name).
pub fn is_high_performance(active: &str) -> bool {
    let a = active.to_ascii_lowercase();
    a.contains(HIGH) || a.contains(ULTIMATE) || a.contains("(high performance)") || a.contains("(ultimate performance)")
}

/// CPU boost from `powercfg /qh SCHEME_CURRENT SUB_PROCESSOR PERFBOOSTMODE`: the plugged-in value is the first
/// "0x..." in it (every Windows language prints those the same). 0 is Disabled.
pub fn boost_is_off(qh: &str) -> Option<bool> {
    let hex = qh.lines().find_map(|l| l.split("0x").nth(1))?.trim();
    u32::from_str_radix(hex.get(..8).unwrap_or(hex), 16).ok().map(|v| v == 0)
}

/// `reg query ...\Services\wuauserv /v Start`: 4 is Disabled.
pub fn updates_are_off(query: &str) -> bool {
    query.lines().any(|l| l.contains("Start") && l.trim_end().ends_with("0x4"))
}

#[cfg(windows)]
pub use imp::{apply, read, updates_off};

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::system;

    fn out(program: &str, args: &[&str]) -> String {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(program).args(args).creation_flags(0x0800_0000).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    }

    pub fn read() -> State {
        let fast = system::powershell("(Get-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Power').HiberbootEnabled", &[]);
        State {
            high_performance: is_high_performance(&out("powercfg", &["/getactivescheme"])),
            boost_off: boost_is_off(&out("powercfg", &["/qh", "SCHEME_CURRENT", "SUB_PROCESSOR", "PERFBOOSTMODE"])).unwrap_or(false),
            fast_startup_off: fast.is_ok_and(|v| v.trim() == "0"),
            updates_off: updates_off(),
        }
    }

    const WU: &str = r"HKLM\SYSTEM\CurrentControlSet\Services\wuauserv";
    const WU_POLICY: &str = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";

    pub fn updates_off() -> bool {
        updates_are_off(&out("reg", &["query", WU, "/v", "Start"]))
    }

    /// Windows Update's service off and its policy set to never update by itself. Undo is Windows' default
    /// (the service starts when needed, no policy).
    fn set_updates(off: bool) -> bool {
        if off {
            let _ = system::run("sc", &["stop", "wuauserv"]);   // fails when it isn't running, which is fine
            system::run("reg", &["add", WU_POLICY, "/v", "NoAutoUpdate", "/t", "REG_DWORD", "/d", "1", "/f"])
                && system::run("sc", &["config", "wuauserv", "start=", "disabled"])
        } else {
            let _ = system::run("reg", &["delete", WU_POLICY, "/v", "NoAutoUpdate", "/f"]);
            system::run("sc", &["config", "wuauserv", "start=", "demand"])
        }
    }

    fn set_boost(off: bool) -> bool {
        // the guide's "Attributes = 2": shows Processor performance boost mode in Power Options too
        system::run("powercfg", &["-attributes", "SUB_PROCESSOR", "PERFBOOSTMODE", "-ATTRIB_HIDE"]);
        let mode = if off { "0" } else { "2" };   // 2 is Aggressive, Windows' default
        ["/setacvalueindex", "/setdcvalueindex"].iter().all(|set| system::run("powercfg", &[set, "SCHEME_CURRENT", "SUB_PROCESSOR", "PERFBOOSTMODE", mode]))
            && system::run("powercfg", &["/setactive", "SCHEME_CURRENT"])
    }

    /// Turn one setting on (Apply) or back to Windows' default (Undo).
    pub fn apply(item: Item, on: bool) -> Result<(), String> {
        let ok = match item {
            Item::HighPerformance => {
                let boost_off = read().boost_off;   // boost is a setting of each plan: carry it to the new one
                const PLAN: &str = r#"
$g = $env:POTATO_PLAN
powercfg /setactive $g 2>$null
if ($LASTEXITCODE -ne 0) {
    # laptops and Modern Standby PCs hide High Performance: copy Windows' built-in one
    $o = powercfg -duplicatescheme $g
    if ("$o" -match '[0-9a-fA-F]{8}-[0-9a-fA-F-]{27}') { powercfg /setactive $Matches[0] } else { exit 1 }
}
"#;
                // shortcut: Undo goes to Balanced, not whatever plan was on before; remember it if people use custom plans
                system::powershell(PLAN, &[("POTATO_PLAN", if on { HIGH } else { BALANCED })]).is_ok() && (!boost_off || set_boost(true))
            }
            Item::BoostOff => set_boost(on),
            Item::FastStartupOff => system::run("reg", &["add", r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power", "/v", "HiberbootEnabled",
                                                      "/t", "REG_DWORD", "/d", if on { "0" } else { "1" }, "/f"]),
            Item::UpdatesOff => set_updates(on),
        };
        if ok { Ok(()) } else { Err("Windows Didn't Take The Change".into()) }
    }
}

/// The demo build: a PC that starts with none of them on.
#[cfg(not(windows))]
mod imp {
    use super::*;
    static NOW: std::sync::Mutex<State> = std::sync::Mutex::new(State { high_performance: false, boost_off: false, fast_startup_off: false, updates_off: false });

    pub fn read() -> State {
        *NOW.lock().unwrap()
    }

    pub fn updates_off() -> bool {
        NOW.lock().unwrap().updates_off
    }

    pub fn apply(item: Item, on: bool) -> Result<(), String> {
        let mut s = NOW.lock().unwrap();
        match item {
            Item::HighPerformance => s.high_performance = on,
            Item::BoostOff => s.boost_off = on,
            Item::FastStartupOff => s.fast_startup_off = on,
            Item::UpdatesOff => s.updates_off = on,
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub use imp::{apply, read, updates_off};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_powercfg() {
        assert!(is_high_performance("Power Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)"));
        assert!(is_high_performance("Power Scheme GUID: 1d2e3f40-0000-0000-0000-000000000000  (High performance)"), "a copy");
        assert!(!is_high_performance("Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)"));
        let qh = "Power Setting GUID: be337238 (Processor performance boost mode)\n  Possible Setting Index: 000\n  Possible Setting Friendly Name: Disabled\n    Current AC Power Setting Index: 0x00000000\n    Current DC Power Setting Index: 0x00000002\n";
        assert_eq!(boost_is_off(qh), Some(true));
        assert_eq!(boost_is_off(&qh.replace("AC Power Setting Index: 0x00000000", "AC Power Setting Index: 0x00000002")), Some(false));
        assert_eq!(boost_is_off("no setting here"), None);
        let wu = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Services\\wuauserv\r\n    Start    REG_DWORD    0x4\r\n";
        assert!(updates_are_off(wu));
        assert!(!updates_are_off(&wu.replace("0x4", "0x3")));
        assert!(!updates_are_off(""));
    }
}
