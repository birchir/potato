//! Remote Desktop alts. Windows Pro lets one person on at a time; the usual fix patches the session check in
//! termsrv.dll (the same bytes as the well-known PowerShell patch), and a Windows update can put the old file back.
//! Alts are plain Windows users in the Remote Desktop Users group (what Computer Management does by hand), each
//! opened on its own 127.x loopback address with its login saved in Windows' Credential Manager.

use std::path::{Path, PathBuf};

use crate::system;

/// `cmp [rcx+63Ch], eax` and the 6-byte jump after it: the check that turns a second user away
const SESSION_CHECK: [u8; 6] = [0x39, 0x81, 0x3C, 0x06, 0x00, 0x00];
/// `mov eax, 100h; mov [rcx+638h], eax; nop`: always room for another session
const PATCHED: [u8; 12] = [0xB8, 0x00, 0x01, 0x00, 0x00, 0x89, 0x81, 0x38, 0x06, 0x00, 0x00, 0x90];
/// the Remote Desktop Users group, by SID so it works in every Windows language
const RDP_USERS: &str = "S-1-5-32-555";

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Patch {
    /// not looked at yet, or the file couldn't be read
    #[default]
    Unknown,
    Patched,
    Unpatched,
    /// neither pattern: a Windows build this patch doesn't fit
    Unsupported,
}

/// What the Remote Desktop page shows. Looked up on its own thread (PowerShell takes a second).
#[derive(Clone, Default)]
pub struct Rdp {
    pub patch: Patch,
    /// accounts in Remote Desktop Users
    pub accounts: Vec<String>,
    /// the saved original matches this Windows, so the patch can be undone
    pub can_undo: bool,
    /// the action running, if any
    pub busy: Option<&'static str>,
    /// a result for the toast
    pub msg: Option<String>,
}

impl Rdp {
    /// Windows put its own file back (an update) after the patch had been on. Potato's own Undo forgets the patch
    /// first, so it never counts.
    pub fn reverted(&self, was_patched: bool) -> bool {
        was_patched && matches!(self.patch, Patch::Unpatched | Patch::Unsupported)
    }
}

pub fn patch_state(dll: &[u8]) -> Patch {
    if find(dll, &PATCHED).is_some() {
        Patch::Patched
    } else if session_check(dll).is_some() {
        Patch::Unpatched
    } else {
        Patch::Unsupported
    }
}

/// The file with the session check patched, or None if it doesn't have one.
pub fn patched(dll: &[u8]) -> Option<Vec<u8>> {
    let i = session_check(dll)?;
    let mut out = dll.to_vec();
    out[i..i + PATCHED.len()].copy_from_slice(&PATCHED);
    Some(out)
}

fn session_check(dll: &[u8]) -> Option<usize> {
    find(dll, &SESSION_CHECK).filter(|i| i + PATCHED.len() <= dll.len())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Each account's own loopback address (mstsc won't open two windows to one address, and the saved login is
/// per address). From the name, so it never changes.
pub fn address(name: &str) -> String {
    let h = name.to_lowercase().bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    format!("127.{}.{}.{}", 1 + h % 254, (h >> 8) % 256, 2 + (h >> 16) % 250)
}

/// Windows' account name rules (Computer Management refuses the rest).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 20 && !n.chars().all(|c| c == '.')
        && !n.chars().any(|c| "\"/\\[]:;|=,+*?<>@".contains(c) || c.is_control())
}

fn termsrv() -> PathBuf {
    PathBuf::from(std::env::var_os("windir").unwrap_or_else(|| "C:\\Windows".into())).join("System32").join("termsrv.dll")
}

/// Potato's folder in ProgramData (next to the settings).
fn store() -> PathBuf {
    crate::engine::config_path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir)
}

fn original() -> PathBuf {
    store().join("termsrv.dll.original")
}

/// Look at everything the page shows.
pub fn look() -> Rdp {
    let dll = std::fs::read(termsrv()).ok();
    let patch = dll.as_deref().map_or(Patch::Unknown, patch_state);
    let can_undo = patch == Patch::Patched
        && std::fs::read(original()).ok().and_then(|o| patched(&o)).is_some_and(|p| Some(p) == dll);
    let accounts = system::powershell(&format!("Get-LocalGroupMember -SID {RDP_USERS} | ForEach-Object {{ $_.Name.Split('\\')[-1] }}"), &[])
        .map(|out| out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    Rdp { patch, accounts, can_undo, ..Rdp::default() }
}

/// Swap Windows' termsrv.dll for `new`. Remote Desktop stops for a moment (open windows drop and reconnect); the
/// file's owner (TrustedInstaller) and rights go back on after, and the service always starts again.
const SWAP: &str = r#"
$ErrorActionPreference = 'Stop'
$f = "$env:windir\System32\termsrv.dll"
$acl = Get-Acl $f
try {
    Stop-Service UmRdpService -Force -ErrorAction SilentlyContinue
    Stop-Service TermService -Force
    $p = (Get-CimInstance Win32_Service -Filter "Name='TermService'").ProcessId
    if ($p) { Stop-Process -Id $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 2
    takeown /f $f | Out-Null
    icacls $f /grant '*S-1-5-32-544:F' | Out-Null
    Copy-Item $env:POTATO_NEW $f -Force
} finally {
    Set-Acl $f $acl -ErrorAction SilentlyContinue
    icacls $f /setowner 'NT SERVICE\TrustedInstaller' | Out-Null
    Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server' fDenyTSConnections 0
    Start-Service TermService
    Start-Service UmRdpService -ErrorAction SilentlyContinue
}
"#;

fn swap_in(new: &Path) -> Result<(), String> {
    system::powershell(SWAP, &[("POTATO_NEW", &new.to_string_lossy())]).map(|_| ())
}

/// Let every account on at once. Keeps this Windows' original file so it can be undone.
pub fn patch() -> Result<(), String> {
    let dll = std::fs::read(termsrv()).map_err(|_| "Couldn't Read Windows' Remote Desktop File".to_string())?;
    let new = patched(&dll).ok_or("Potato's Patch Doesn't Fit This Windows Version Yet")?;
    std::fs::create_dir_all(store()).map_err(|e| e.to_string())?;
    std::fs::write(original(), &dll).map_err(|e| e.to_string())?;
    let tmp = store().join("termsrv.dll.patched");
    std::fs::write(&tmp, new).map_err(|e| e.to_string())?;
    let done = swap_in(&tmp);
    let _ = std::fs::remove_file(&tmp);
    done?;
    if look().patch == Patch::Patched { Ok(()) } else { Err("Windows Kept Its Own File".into()) }
}

/// Put Windows' own file back.
pub fn undo() -> Result<(), String> {
    swap_in(&original())?;
    if look().patch == Patch::Unpatched { Ok(()) } else { Err("Couldn't Put Windows' File Back".into()) }
}

/// Make a Windows user in Remote Desktop Users (or add an existing one) and save its login for Open.
pub fn add_account(name: &str, password: &str) -> Result<(), String> {
    const ADD: &str = r#"
$ErrorActionPreference = 'Stop'
$n = $env:POTATO_USER
if (-not (Get-LocalUser -Name $n -ErrorAction SilentlyContinue)) {
    $pw = ConvertTo-SecureString $env:POTATO_PASS -AsPlainText -Force
    New-LocalUser -Name $n -Password $pw -PasswordNeverExpires -AccountNeverExpires | Out-Null
}
if (-not (Get-LocalGroupMember -SID S-1-5-32-555 | Where-Object { $_.Name.Split('\')[-1] -eq $n })) {
    Add-LocalGroupMember -SID S-1-5-32-555 -Member $n
}
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server' fDenyTSConnections 0
"#;
    let name = name.trim();
    system::powershell(ADD, &[("POTATO_USER", name), ("POTATO_PASS", password)])?;
    if system::save_rdp_login(&address(name), name, password) { Ok(()) } else { Err("Made It, But Couldn't Save Its Login".into()) }
}

/// The account's Remote Desktop settings, as the guides set them up for RDP Plus
/// (`/nodrives /nosound /nowallpaper /o:"keyboardhook:i:1" /w:1366 /h:768`), written as an .rdp file because
/// Windows' own Remote Desktop only takes those as file settings. `wide` is the guide's 1800x768 for boosting and
/// Honeyday (movespeed correction breaks in narrower windows).
pub fn rdp_file(name: &str, wide: bool) -> String {
    let (w, h) = if wide { (1800, 768) } else { (1366, 768) };
    [
        format!("full address:s:{}", address(name)),
        format!("username:s:{name}"),
        "screen mode id:i:1".into(),
        format!("desktopwidth:i:{w}"),
        format!("desktopheight:i:{h}"),
        "audiomode:i:2".into(),           // /nosound
        "redirectdrives:i:0".into(),      // /nodrives
        "drivestoredirect:s:".into(),
        "redirectprinters:i:0".into(),
        "redirectclipboard:i:1".into(),
        "disable wallpaper:i:1".into(),   // /nowallpaper
        "keyboardhook:i:1".into(),        // /o:"keyboardhook:i:1"
        "prompt for credentials:i:0".into(),
        "authentication level:i:0".into(),
    ].join("\r\n") + "\r\n"
}

/// Where an account's .rdp file lives (Potato's folder), written fresh with its current settings.
pub fn write_file(name: &str, wide: bool) -> Result<PathBuf, String> {
    let dir = store().join("rdp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{name}.rdp"));
    std::fs::write(&file, rdp_file(name, wide)).map_err(|e| e.to_string())?;
    system::trust_rdp_file(&address(name));
    Ok(file)
}

/// Open the account's Remote Desktop window.
pub fn open(name: &str, wide: bool) -> Result<(), String> {
    let file = write_file(name, wide)?;
    std::process::Command::new("mstsc").arg(&file).spawn().map(|_| ()).map_err(|_| "Couldn't Open Remote Desktop".into())
}

/// A desktop shortcut that opens the account straight in Windows' Remote Desktop with its .rdp file (every guide
/// setting is in the file, so the shortcut works without Potato running).
pub fn desktop_shortcut(name: &str, wide: bool) -> Result<(), String> {
    let file = write_file(name, wide)?;
    const MAKE: &str = r#"
$ErrorActionPreference = 'Stop'
$lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) ($env:POTATO_NAME + ' (Remote Desktop).lnk')
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$s.TargetPath = "$env:windir\System32\mstsc.exe"
$s.Arguments = '"' + $env:POTATO_RDP + '"'
$s.WorkingDirectory = Split-Path $env:POTATO_RDP
$s.IconLocation = "$env:windir\System32\mstsc.exe,0"
$s.Description = "Opens $env:POTATO_NAME in Remote Desktop with Potato's light settings"
$s.Save()
"#;
    system::powershell(MAKE, &[("POTATO_NAME", name), ("POTATO_RDP", &file.to_string_lossy())]).map(|_| ())
        .map_err(|_| "Couldn't Make The Shortcut".into())
}

/// How many Remote Desktops the guide says this PC can run steadily: the lower of its CPU and RAM tests.
/// CPU: cores - 2, or (logical processors - 4) / 2 when those aren't one or two a core (newer Intel). RAM: 4 GB
/// each, after 4 GB for Windows (8 GB from 32 GB of RAM up), rounded down.
pub fn capacity(cores: usize, logical: usize, ram_gb: f64) -> (usize, usize) {
    // one or two threads a core: cores - 2; newer Intel (performance and efficiency cores) has neither
    let cpu = if logical == cores || logical == cores * 2 { cores.saturating_sub(2) } else { logical.saturating_sub(4) / 2 };
    let reserve = if ram_gb >= 30.0 { 8.0 } else { 4.0 };   // a 32 GB PC shows about 31 GB: the guide's own 31.3 GB example keeps 8
    let ram = ((ram_gb - reserve) / 4.0).floor().max(0.0) as usize;
    (cpu, ram)
}

const STARTUP_TASK: &str = "Potato Open Remote Desktops";

/// The logon task that opens the picked accounts (`Potato.exe --open-rdp`) when this Windows user signs in, or
/// None when no account is picked. Runs elevated (no prompt for an admin) so it can rewrite the .rdp files.
pub fn startup_task(sid: &str, exe: &Path, any: bool) -> Option<String> {
    any.then(|| format!(r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId><Delay>PT15S</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Me"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT10M</ExecutionTimeLimit></Settings>
  <Actions Context="Me"><Exec><Command>{}</Command><Arguments>--open-rdp</Arguments></Exec></Actions>
</Task>"#, exe.display()))
}

/// Keep the logon task in step with the picked accounts.
pub fn save_startup(any: bool) -> bool {
    system::set_task(STARTUP_TASK, startup_task(&system::current_sid(), &crate::engine::installed_exe(), any).as_deref())
}

/// `Potato.exe --open-rdp`, at sign in: open each picked account, a few seconds apart so mstsc isn't swamped.
pub fn open_startup() {
    let config = std::fs::read_to_string(crate::engine::config_path()).map(|t| crate::config::Config::from_json(&t)).unwrap_or_default();
    for name in &config.rdp_startup {
        let _ = open(name, config.rdp_wide.contains(name));
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// Standard base64 (for PowerShell's -EncodedCommand).
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_bytes() {
        let mut dll = vec![0u8; 40];
        dll[10..16].copy_from_slice(&SESSION_CHECK);
        dll[16..22].copy_from_slice(&[0x0F, 0x84, 0x7F, 0x2C, 0x01, 0x00]);
        assert_eq!(patch_state(&dll), Patch::Unpatched);
        let p = patched(&dll).unwrap();
        assert_eq!(&p[10..22], &PATCHED);
        assert_eq!((p.len(), &p[..10], &p[22..]), (40, &dll[..10], &dll[22..]));
        assert_eq!(patch_state(&p), Patch::Patched);
        assert_eq!(patch_state(&[0u8; 40]), Patch::Unsupported);
        assert!(patched(&dll[..20]).is_none(), "a check cut off by the end of the file isn't patched");
    }

    #[test]
    fn startup_task_only_when_picked() {
        assert_eq!(startup_task("S-1", Path::new("C:\\P\\Potato.exe"), false), None);
        let xml = startup_task("S-1-5-21-9", Path::new("C:\\P\\Potato.exe"), true).unwrap();
        assert!(xml.contains("<UserId>S-1-5-21-9</UserId><Delay>"));
        assert!(xml.contains("<Command>C:\\P\\Potato.exe</Command><Arguments>--open-rdp</Arguments>"));
    }

    #[test]
    fn rdp_file_has_every_guide_setting() {
        let f = rdp_file("Alt1", false);
        for line in ["username:s:Alt1", "desktopwidth:i:1366", "desktopheight:i:768", "audiomode:i:2", "redirectdrives:i:0",
                     "disable wallpaper:i:1", "keyboardhook:i:1", &format!("full address:s:{}", address("Alt1"))] {
            assert!(f.split("\r\n").any(|l| l == line), "{line}");
        }
        assert!(rdp_file("Alt1", true).contains("desktopwidth:i:1800\r\n"));
    }

    #[test]
    fn capacity_follows_the_guide() {
        assert_eq!(capacity(8, 16, 31.3), (6, 5));    // the guide's own examples: 8 cores, 31.3 GB
        assert_eq!(capacity(14, 20, 64.0), (8, 14));  // 20 logical, not double: (20 - 4) / 2
        assert_eq!(capacity(2, 4, 8.0), (0, 1));
        assert_eq!(capacity(4, 8, 3.0), (2, 0));
        assert_eq!(capacity(4, 4, 8.0), (2, 1), "no hyper-threading: still cores - 2");
    }

    #[test]
    fn addresses_and_names() {
        assert_eq!(address("Alt1"), address("alt1"));
        assert_ne!(address("Alt1"), address("Alt2"));
        for n in ["Alt1", "Alt2", "Main", "x"] {
            let parts: Vec<u32> = address(n).split('.').map(|p| p.parse().unwrap()).collect();
            assert_eq!(parts.len(), 4);
            assert!(parts[0] == 127 && parts[1] >= 1 && parts[3] >= 2 && parts.iter().all(|p| *p <= 255));
        }
        assert!(valid_name("Alt 1") && valid_name("alt_2"));
        assert!(!valid_name("") && !valid_name("...") && !valid_name("a/b") && !valid_name("averyveryverylongname1"));
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!([base64(b""), base64(b"f"), base64(b"fo"), base64(b"foo"), base64(b"foobar")], ["", "Zg==", "Zm8=", "Zm9v", "Zm9vYmFy"]);
    }
}
