//! Remote Desktop alts. Windows lets one person on at a time. Multiple RDPs loads TermWrap (github.com/llccd/TermWrap,
//! MIT) into the Remote Desktop service in place of termsrv.dll: it finds the session check in memory on every
//! Windows build, so updates don't undo it. Older Potatos patched the file itself, which an update puts back; that
//! is still read and undone.
//! Alts are plain Windows users in the Remote Desktop Users group (what Computer Management does by hand), each
//! opened on its own 127.x loopback address with its login saved in Windows' Credential Manager.

use std::path::{Path, PathBuf};

use crate::system;

/// `cmp [rcx+63Ch], eax` and the 6-byte jump after it: the check that turns a second user away
const SESSION_CHECK: [u8; 6] = [0x39, 0x81, 0x3C, 0x06, 0x00, 0x00];
/// `mov eax, 100h; mov [rcx+638h], eax; nop`: always room for another session
const PATCHED: [u8; 12] = [0xB8, 0x00, 0x01, 0x00, 0x00, 0x89, 0x81, 0x38, 0x06, 0x00, 0x00, 0x90];
/// TermWrap's release, pinned to the file checked here (a new version needs a new hash)
const TERMWRAP_ZIP: &str = "https://github.com/llccd/TermWrap/releases/download/v0.7/TermWrap-0.7.zip";
const TERMWRAP_SHA256: &str = "096cc075e4d2ebbf2587f19d5e9a3003181e1d3ae44c514e0532ba222f82afc4";
/// TermWrap needs Microsoft's Visual C++ runtime
const VC_REDIST: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Patch {
    /// not looked at yet, or the file couldn't be read
    #[default]
    Unknown,
    Patched,
    Unpatched,
    /// not a 64-bit Intel or AMD PC (TermWrap's x64 build is the one Potato installs)
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
    /// Windows' version and build, e.g. "22H2, build 19045.6466"
    pub windows: String,
    /// the Remote Desktop service's state ("Running", "Stopped"), empty if it's missing
    pub service: String,
    /// the port Remote Desktop listens on, and whether it is listening right now
    pub port: String,
    pub listening: bool,
    /// the lag and picture settings below, as Windows has them now
    pub tcp_only: bool,
    pub fps60: bool,
    pub hide_alts: bool,
    /// Windows' ImageQuality / MaxCompressionLevel, None when left to Windows
    pub quality: Option<u32>,
    pub compression: Option<u32>,
}

/// Group Policy's Remote Desktop key: what `gpedit` writes, so Windows reads it without gpedit (Home has none).
const POLICY: &str = r"HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services";
const WINSTATIONS: &str = r"HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations";
const SIGN_IN_LIST: &str = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList";

/// The RDP settings Potato can change, Windows-wide. Values are from Windows' own TerminalServer.admx (and
/// Microsoft's 30 FPS article for the frame rate).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Setting {
    /// "Select RDP transport protocols: Use only TCP" for this PC as host and as client
    TcpOnly,
    /// DWMFRAMEINTERVAL 15: 60 FPS instead of 30
    Fps60,
    /// each alt hidden from Windows' sign-in screen (they still sign in over RDP)
    HideAlts,
    /// ImageQuality: 1 Lossless, 2 High, 3 Medium, 4 Low
    Quality,
    /// MaxCompressionLevel: 0 Off, 1 Less Memory, 2 Balanced, 3 Less Bandwidth
    Compression,
}

/// The registry writes for a setting as `key|name|value` lines; an empty value removes it (back to Windows'
/// default). `alts` are the accounts HideAlts covers; `me` is never hidden.
pub fn writes(setting: Setting, value: Option<u32>, alts: &[String], me: &str) -> Vec<String> {
    let v = value.map(|v| v.to_string()).unwrap_or_default();
    match setting {
        Setting::TcpOnly => vec![format!("{POLICY}|SelectTransport|{v}"), format!("{POLICY}\\Client|fClientDisableUDP|{v}")],
        Setting::Fps60 => vec![format!("{WINSTATIONS}|DWMFRAMEINTERVAL|{v}")],
        Setting::Quality => vec![format!("{POLICY}|ImageQuality|{v}")],
        Setting::Compression => vec![format!("{POLICY}|MaxCompressionLevel|{v}")],
        Setting::HideAlts => alts.iter().filter(|a| !a.eq_ignore_ascii_case(me)).map(|a| format!("{SIGN_IN_LIST}|{a}|{v}")).collect(),
    }
}

const WRITE: &str = r#"
$ErrorActionPreference = 'Stop'
foreach ($l in $env:POTATO_SET -split "`n") {
    $k, $n, $v = $l -split '\|'
    if ($v -eq '') { Remove-ItemProperty $k $n -ErrorAction SilentlyContinue; continue }
    if (-not (Test-Path $k)) { New-Item $k -Force | Out-Null }
    Set-ItemProperty $k $n ([int]$v) -Type DWord
}
"#;

/// Change one setting (None puts Windows' default back).
pub fn set(setting: Setting, value: Option<u32>, alts: &[String]) -> Result<(), String> {
    let lines = writes(setting, value, alts, &std::env::var("USERNAME").unwrap_or_default()).join("\n");
    system::powershell(WRITE, &[("POTATO_SET", &lines)]).map(|_| ()).map_err(|_| "Windows Didn't Let Potato Change It".into())
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

/// Whether the Remote Desktop service loads a wrapper (TermWrap, or RDP Wrapper someone installed) instead of
/// Windows' own termsrv.dll.
pub fn is_wrapper(service_dll: &str) -> bool {
    let s = service_dll.to_ascii_lowercase();
    s.contains("termwrap") || s.contains("rdpwrap")
}

/// The service's DLL, then each alt (Remote Desktop Users, by SID so it works in every Windows language). Every alt's password is set to never expire on the way: Windows makes local
/// passwords expire after 42 days, and an expired one locks the alt out of Remote Desktop.
const LOOK: &str = r#"
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
"WINDOWS=$($cv.DisplayVersion), build $($cv.CurrentBuild).$($cv.UBR)"
"SERVICE=" + (Get-Service TermService -ErrorAction SilentlyContinue).Status
$port = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp' -ErrorAction SilentlyContinue).PortNumber
"PORT=$port"
"LISTENING=" + [bool](Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
"SERVICEDLL=" + (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService\Parameters' -ErrorAction SilentlyContinue).ServiceDll
$pol = Get-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services' -ErrorAction SilentlyContinue
"TCPONLY=" + ($pol.SelectTransport -eq 1)
"QUALITY=$($pol.ImageQuality)"
"COMPRESSION=$($pol.MaxCompressionLevel)"
"FPS60=" + ((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations' -ErrorAction SilentlyContinue).DWMFRAMEINTERVAL -eq 15)
$hk = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList'
$hidden = Get-ItemProperty $hk -ErrorAction SilentlyContinue
$alts = @(Get-LocalGroupMember -SID S-1-5-32-555 | ForEach-Object { [pscustomobject]@{ n = $_.Name.Split('\')[-1]; local = "$($_.PrincipalSource)" -eq 'Local' } })
$hide = [bool]($alts | Where-Object { $hidden -and $hidden.($_.n) -eq 0 })
"HIDEALTS=$hide"
foreach ($a in $alts) {
    if ($a.local) { Set-LocalUser -Name $a.n -PasswordNeverExpires $true -ErrorAction SilentlyContinue }
    # once alts are hidden, a new one is hidden too (never you)
    if ($hide -and $a.local -and $a.n -ne $env:USERNAME -and $hidden.($a.n) -ne 0) { Set-ItemProperty $hk $a.n 0 -Type DWord }
    $a.n
}
"#;

/// Look at everything the page shows.
pub fn look() -> Rdp {
    let out = system::powershell(LOOK, &[]).unwrap_or_default();
    let tag = |t: &str| out.lines().find_map(|l| l.trim().strip_prefix(t)).unwrap_or_default().to_string();
    let service_dll = tag("SERVICEDLL=");
    // Windows names can't hold '=', so every tagged line is skipped
    let accounts = out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains('=')).map(String::from).collect();
    let wrapped = is_wrapper(&service_dll);
    let dll = std::fs::read(termsrv()).ok();
    let patch = match dll.as_deref().map(patch_state) {
        _ if wrapped => Patch::Patched,
        _ if std::env::var("PROCESSOR_ARCHITECTURE").is_ok_and(|a| !a.eq_ignore_ascii_case("AMD64")) => Patch::Unsupported,
        Some(Patch::Patched) => Patch::Patched,   // an older Potato's patch of the file
        Some(_) => Patch::Unpatched,
        None => Patch::Unknown,
    };
    let can_undo = wrapped || (patch == Patch::Patched
        && std::fs::read(original()).ok().and_then(|o| patched(&o)).is_some_and(|p| Some(p) == dll));
    Rdp { patch, accounts, can_undo, windows: tag("WINDOWS="), service: tag("SERVICE="), port: tag("PORT="),
          listening: tag("LISTENING=").eq_ignore_ascii_case("true"), tcp_only: tag("TCPONLY=") == "True", fps60: tag("FPS60=") == "True",
          hide_alts: tag("HIDEALTS=") == "True", quality: tag("QUALITY=").parse().ok(), compression: tag("COMPRESSION=").parse().ok(), ..Rdp::default() }
}

/// Restart Remote Desktop so it loads its DLL again: open RDPs drop for a moment and reconnect, Roblox in them
/// keeps running. The service shares nothing, but stopping it can hang, so its process is ended too.
const RESTART: &str = r#"
Stop-Service UmRdpService -Force -ErrorAction SilentlyContinue
Stop-Service TermService -Force -ErrorAction SilentlyContinue
$p = (Get-CimInstance Win32_Service -Filter "Name='TermService'").ProcessId
if ($p) { Stop-Process -Id $p -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 2
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server' fDenyTSConnections 0
Start-Service TermService
Start-Service UmRdpService -ErrorAction SilentlyContinue
"#;

/// Windows' RDP service restarted: fixes alts that won't connect.
pub fn restart_service() -> Result<(), String> {
    system::powershell(RESTART, &[]).map(|_| ()).map_err(|_| "Couldn't Restart The RDP Service".into())
}

/// Every local account's password and the account itself set to never expire (Windows makes local passwords
/// expire after 42 days, and an expired one can't sign in over RDP).
pub fn never_expire() -> Result<(), String> {
    const ALL: &str = "Get-LocalUser | Where-Object Enabled | ForEach-Object { Set-LocalUser -Name $_.Name -PasswordNeverExpires $true -AccountNeverExpires -ErrorAction SilentlyContinue }";
    system::powershell(ALL, &[]).map(|_| ()).map_err(|_| "Couldn't Change The Accounts".into())
}

/// Put back the Remote Desktop service when something deleted it: Windows' own settings for it (svchost's
/// NetworkService group, termsrv.dll), then started.
const REPAIR: &str = r#"
$ErrorActionPreference = 'Stop'
if (-not (Get-Service TermService -ErrorAction SilentlyContinue)) {
    sc.exe create TermService type= share start= demand binPath= "%SystemRoot%\System32\svchost.exe -k NetworkService" obj= "NT Authority\NetworkService" DisplayName= "@%SystemRoot%\System32\termsrv.dll,-268" depend= RPCSS | Out-Null
    $p = 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService\Parameters'
    New-Item $p -Force | Out-Null
    Set-ItemProperty $p ServiceDll '%SystemRoot%\System32\termsrv.dll' -Type ExpandString
    Set-ItemProperty $p ServiceDllUnloadOnStop 1 -Type DWord
    # its own service identity and privileges, as Windows ships it: without them it runs but never listens
    sc.exe sidtype TermService unrestricted | Out-Null
    sc.exe privs TermService SeAssignPrimaryTokenPrivilege/SeAuditPrivilege/SeChangeNotifyPrivilege/SeCreateGlobalPrivilege/SeImpersonatePrivilege/SeIncreaseQuotaPrivilege | Out-Null
    sc.exe failure TermService reset= 86400 actions= restart/60000/restart/60000//60000 | Out-Null
    sc.exe description TermService "@%SystemRoot%\System32\termsrv.dll,-267" | Out-Null
    $g = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Svchost'
    $list = @((Get-ItemProperty $g).NetworkService)
    if ($list -notcontains 'TermService') { Set-ItemProperty $g NetworkService ($list + 'TermService') -Type MultiString }
}
"#;

pub fn repair_service() -> Result<(), String> {
    system::powershell(&format!("{REPAIR}{RESTART}"), &[]).map(|_| ()).map_err(|_| "Couldn't Repair The RDP Service".into())
}

/// Microsoft's C++ runtime if it's missing (signed by Microsoft or not run), TermWrap's DLLs from the checked zip,
/// then the service pointed at TermWrap and restarted.
const INSTALL: &str = r#"
$ErrorActionPreference = 'Stop'
if ((Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64' -ErrorAction SilentlyContinue).Installed -ne 1) {
    $vc = Join-Path $env:TEMP 'potato_vc_redist.x64.exe'
    curl.exe -sfL -m 300 -o $vc $env:POTATO_VC
    $sig = Get-AuthenticodeSignature $vc
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') { throw 'not signed by Microsoft' }
    Start-Process $vc -ArgumentList '/install', '/quiet', '/norestart' -Wait
    Remove-Item $vc -Force -ErrorAction SilentlyContinue
}
$x = Join-Path $env:TEMP 'potato_termwrap'
Remove-Item $x -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive $env:POTATO_ZIP $x
$d = "$env:ProgramFiles\RDP Wrapper"
New-Item -ItemType Directory -Force $d | Out-Null
Copy-Item "$x\x64\TermWrap.dll", "$x\x64\Zydis.dll" $d -Force
Remove-Item $x -Recurse -Force -ErrorAction SilentlyContinue
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService\Parameters' ServiceDll '%ProgramFiles%\RDP Wrapper\TermWrap.dll' -Type ExpandString
"#;

const UNWRAP: &str = r#"
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService\Parameters' ServiceDll '%SystemRoot%\System32\termsrv.dll' -Type ExpandString
"#;

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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Multiple RDPs on: TermWrap downloaded, checked against its pinned hash, and loaded by Remote Desktop.
pub fn patch() -> Result<(), String> {
    let zip = crate::engine::fetch(TERMWRAP_ZIP, 120).ok_or("Couldn't Download TermWrap. Check Your Internet")?;
    if sha256_hex(&zip) != TERMWRAP_SHA256 {
        return Err("TermWrap's Download Didn't Match, So Nothing Was Changed".into());
    }
    std::fs::create_dir_all(store()).map_err(|e| e.to_string())?;
    let file = store().join("TermWrap.zip");
    std::fs::write(&file, zip).map_err(|e| e.to_string())?;
    let done = system::powershell(&format!("{INSTALL}{RESTART}"), &[("POTATO_ZIP", &file.to_string_lossy()), ("POTATO_VC", VC_REDIST)]);
    let _ = std::fs::remove_file(&file);
    done.map_err(|_| "Windows Didn't Let Potato Change It".to_string())?;
    if look().patch == Patch::Patched { Ok(()) } else { Err("Windows Didn't Let Potato Change It".into()) }
}

/// Multiple RDPs off: Remote Desktop back on Windows' own termsrv.dll (or an older Potato's patched file put back).
pub fn undo() -> Result<(), String> {
    let wrapped = is_wrapper(&system::powershell(LOOK, &[]).unwrap_or_default().lines().find_map(|l| l.trim().strip_prefix("SERVICEDLL=").map(String::from)).unwrap_or_default());
    if wrapped {
        system::powershell(&format!("{UNWRAP}{RESTART}"), &[])?;
    } else {
        swap_in(&original())?;
    }
    if look().patch == Patch::Unpatched { Ok(()) } else { Err("Couldn't Turn It Off".into()) }
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
    if system::save_rdp_login(&address(name), name, password) { Ok(()) } else { Err("Added The Alt, But Couldn't Save Its Password".into()) }
}

/// Remove an alt: signed out, its Windows account and its files (C:\Users\<name>, Roblox in it too) deleted, or just
/// taken out of Remote Desktop Users if it isn't a local account; then its saved login, .rdp file and desktop shortcut.
pub fn remove_account(name: &str) -> Result<(), String> {
    const REMOVE: &str = r#"
$n = $env:POTATO_USER
quser 2>$null | Select-String -Pattern "^\s*>?$([regex]::Escape($n))\s" | ForEach-Object {
    $id = ($_.Line.Trim() -split '\s+') | Where-Object { $_ -match '^\d+$' } | Select-Object -First 1
    if ($id) { logoff $id }
}
$ErrorActionPreference = 'Stop'   # after quser: its "no sessions" stderr would stop the script
$u = Get-LocalUser -Name $n -ErrorAction SilentlyContinue
if ($u) {
    Start-Sleep -Seconds 3
    Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $u.SID.Value } | Remove-CimInstance -ErrorAction SilentlyContinue
    Remove-LocalUser -Name $n
} else {
    Get-LocalGroupMember -SID S-1-5-32-555 | Where-Object { $_.Name.Split('\')[-1] -eq $n } | ForEach-Object { Remove-LocalGroupMember -SID S-1-5-32-555 -Member $_ }
}
cmdkey "/delete:TERMSRV/$env:POTATO_ADDR" | Out-Null
Remove-Item (Join-Path ([Environment]::GetFolderPath('Desktop')) "$n (Remote Desktop).lnk") -Force -ErrorAction SilentlyContinue
Remove-Item $env:POTATO_RDP -Force -ErrorAction SilentlyContinue
"#;
    let rdp = store().join("rdp").join(format!("{name}.rdp"));
    system::powershell(REMOVE, &[("POTATO_USER", name), ("POTATO_ADDR", &address(name)), ("POTATO_RDP", &rdp.to_string_lossy())])
        .map(|_| ()).map_err(|_| format!("Couldn't Remove {name}"))
}

/// The account's Remote Desktop settings, as the user's RDP Plus shortcuts set them up
/// (`/title:"Alt2" /nodrives /nosound /nowallpaper /o:"keyboardhook:i:0" /w:1366 /h:768`), written as an .rdp file because
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
        // less for Remote Desktop to draw: no themes, animations, font smoothing or see-through windows
        "allow font smoothing:i:0".into(),
        "allow desktop composition:i:0".into(),
        "disable full window drag:i:1".into(),
        "disable menu anims:i:1".into(),
        "disable themes:i:1".into(),
        // it's this PC talking to itself: guessing the connection speed often settles on a slow one, so say LAN
        "networkautodetect:i:0".into(),
        "bandwidthautodetect:i:0".into(),
        "connection type:i:6".into(),
        "keyboardhook:i:0".into(),        // /o:"keyboardhook:i:0": Windows key shortcuts stay on this PC, not the alt
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

/// RDP Plus's command line for the account, the user's own shortcut string with the account's address, name and
/// size (it signs in with the login Make Account saved for that address).
pub fn rdp_plus_args(name: &str, wide: bool) -> String {
    let w = if wide { 1800 } else { 1366 };
    format!(r#"/v:{} /i:{name} /title:"{name}" /nodrives /nosound /nowallpaper /o:"keyboardhook:i:0" /w:{w} /h:768"#, address(name))
}

/// A desktop shortcut that opens the account. With RDP Plus (rdp.exe in Downloads or on the Desktop, copied once into
/// Potato's folder so the shortcut keeps working if the download is moved) it runs that with the user's string;
/// without it, Windows' Remote Desktop with the .rdp file. Either works without Potato running.
pub fn desktop_shortcut(name: &str, wide: bool) -> Result<bool, String> {
    let file = write_file(name, wide)?;
    const MAKE: &str = r#"
$ErrorActionPreference = 'Stop'
$exe = Join-Path $env:POTATO_DIR 'rdp.exe'
if (-not (Test-Path $exe)) {
    $found = @("$env:USERPROFILE\Downloads\rdp.exe", ([Environment]::GetFolderPath('Desktop') + '\rdp.exe')) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($found) { Copy-Item $found $exe }
}
$lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) ($env:POTATO_NAME + ' (Remote Desktop).lnk')
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
if (Test-Path $exe) {
    $s.TargetPath = $exe
    $s.Arguments = $env:POTATO_ARGS
    $s.WorkingDirectory = $env:POTATO_DIR
    $s.IconLocation = "$exe,0"
    'plus'
} else {
    $s.TargetPath = "$env:windir\System32\mstsc.exe"
    $s.Arguments = '"' + $env:POTATO_RDP + '"'
    $s.WorkingDirectory = Split-Path $env:POTATO_RDP
    $s.IconLocation = "$env:windir\System32\mstsc.exe,0"
}
$s.Description = "Opens $env:POTATO_NAME in Remote Desktop with Potato's light settings"
$s.Save()
"#;
    let dir = store();
    system::powershell(MAKE, &[("POTATO_NAME", name), ("POTATO_RDP", &file.to_string_lossy()),
                               ("POTATO_DIR", &dir.to_string_lossy()), ("POTATO_ARGS", &rdp_plus_args(name, wide))])
        .map(|out| out.contains("plus")).map_err(|_| "Couldn't Make The Shortcut".into())
}

/// Tidy Desktop: pin only File Explorer and the alt's default browser (Windows' taskbar layout policy in the alt's
/// own registry, which needs admin, so it's done here), hide the taskbar news for the whole PC (a machine policy),
/// then ask the alt's helper to do the rest in its own login (`helper::serve_tidy`, the next time it's signed in).
pub fn request_tidy(name: &str) -> Result<(), String> {
    let dir = store().join("desktop");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = system::powershell(PINS, &[("POTATO_NAME", name), ("POTATO_DIR", &dir.to_string_lossy())]);
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    std::fs::write(dir.join(format!("{}.txt", name.to_lowercase())), id.to_string()).map_err(|e| e.to_string())
}

/// Scripts can't pin to the taskbar any more (the pin command doesn't show up for them), so the pins come from a
/// taskbar layout file, set as a policy for that one user. Explorer applies it when it next starts.
const PINS: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
New-Item 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Windows Feeds' -Force | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Windows Feeds' EnableFeeds 0 -Type DWord
$name = $env:POTATO_NAME; $dir = $env:POTATO_DIR
$sid = (New-Object Security.Principal.NTAccount($name)).Translate([Security.Principal.SecurityIdentifier]).Value
if (-not $sid) { return }
$root = "Registry::HKEY_USERS\$sid"
$loaded = Test-Path $root
if (-not $loaded) {
    $userDir = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$sid").ProfileImagePath
    if (-not $userDir) { return }   # never signed in: no registry to write to yet
    reg load 'HKU\PotatoTidy' "$userDir\NTUSER.DAT" | Out-Null
    $root = 'Registry::HKEY_USERS\PotatoTidy'
}
$exe = $null
$prog = (Get-ItemProperty "$root\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice").ProgId
if ($prog) {
    $cmd = (Get-ItemProperty "$root\Software\Classes\$prog\shell\open\command").'(default)'
    if (-not $cmd) { $cmd = (Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\$prog\shell\open\command").'(default)' }
    if ($cmd -match '^"([^"]+)"' -or $cmd -match '^(\S+\.exe)') { $exe = $Matches[1] }
}
if (-not $exe -or -not (Test-Path $exe)) { $exe = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" }
$title = (Get-Item $exe).VersionInfo.FileDescription
if (-not $title) { $title = [IO.Path]::GetFileNameWithoutExtension($exe) }
$lnk = Join-Path $dir "$title.lnk"
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk); $s.TargetPath = $exe; $s.Save()
$xml = Join-Path $dir "$name-taskbar.xml"
@"
<?xml version="1.0" encoding="utf-8"?>
<LayoutModificationTemplate xmlns="http://schemas.microsoft.com/Start/2014/LayoutModification" xmlns:defaultlayout="http://schemas.microsoft.com/Start/2014/FullDefaultLayout" xmlns:taskbar="http://schemas.microsoft.com/Start/2014/TaskbarLayout" Version="1">
  <CustomTaskbarLayoutCollection PinListPlacement="Replace">
    <defaultlayout:TaskbarLayout>
      <taskbar:TaskbarPinList>
        <taskbar:DesktopApp DesktopApplicationLinkPath="%APPDATA%\Microsoft\Windows\Start Menu\Programs\System Tools\File Explorer.lnk" />
        <taskbar:DesktopApp DesktopApplicationLinkPath="$lnk" />
      </taskbar:TaskbarPinList>
    </defaultlayout:TaskbarLayout>
  </CustomTaskbarLayoutCollection>
</LayoutModificationTemplate>
"@ | Set-Content $xml -Encoding UTF8
$k = "$root\Software\Policies\Microsoft\Windows\Explorer"
New-Item $k -Force | Out-Null
Set-ItemProperty $k StartLayoutFile $xml
Set-ItemProperty $k LockedStartLayout 1 -Type DWord
if (-not $loaded) { [GC]::Collect(); Start-Sleep 1; reg unload 'HKU\PotatoTidy' | Out-Null }
"#;

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
    fn settings_write_the_admx_values() {
        let alts = ["Alt1".to_string(), "Jay".to_string()];
        assert_eq!(writes(Setting::TcpOnly, Some(1), &[], ""), [format!("{POLICY}|SelectTransport|1"), format!("{POLICY}\\Client|fClientDisableUDP|1")]);
        assert_eq!(writes(Setting::Fps60, Some(15), &[], ""), [format!("{WINSTATIONS}|DWMFRAMEINTERVAL|15")]);
        assert_eq!(writes(Setting::Quality, None, &[], ""), [format!("{POLICY}|ImageQuality|")], "None removes it");
        assert_eq!(writes(Setting::HideAlts, Some(0), &alts, "jay"), [format!("{SIGN_IN_LIST}|Alt1|0")], "never hides you");
    }

    #[test]
    #[cfg(not(windows))]   // reads the demo Windows; a real one answers with its own state
    fn look_reads_status_and_skips_it_in_accounts() {
        let r = look();   // the demo PowerShell answers with the same lines Windows does
        assert_eq!((r.service.as_str(), r.port.as_str(), r.listening), ("Running", "3389", true));
        assert!(r.windows.contains("build"));
        assert_eq!(r.accounts, ["Alt1", "Alt2"]);
        assert!(r.tcp_only && !r.fps60 && !r.hide_alts);
        assert_eq!((r.quality, r.compression), (Some(4), None));
    }

    #[test]
    fn startup_task_only_when_picked() {
        assert_eq!(startup_task("S-1", Path::new("C:\\P\\Potato.exe"), false), None);
        let xml = startup_task("S-1-5-21-9", Path::new("C:\\P\\Potato.exe"), true).unwrap();
        assert!(xml.contains("<UserId>S-1-5-21-9</UserId><Delay>"));
        assert!(xml.contains("<Command>C:\\P\\Potato.exe</Command><Arguments>--open-rdp</Arguments>"));
    }

    #[test]
    fn rdp_plus_args_match_the_users_shortcut() {
        let a = address("Alt1");
        assert_eq!(rdp_plus_args("Alt1", false),
                   format!(r#"/v:{a} /i:Alt1 /title:"Alt1" /nodrives /nosound /nowallpaper /o:"keyboardhook:i:0" /w:1366 /h:768"#));
        assert!(rdp_plus_args("Alt1", true).contains("/w:1800 /h:768"));
    }

    #[test]
    fn rdp_file_has_every_fast_setting() {
        let f = rdp_file("Alt1", false);
        for line in ["username:s:Alt1", "desktopwidth:i:1366", "desktopheight:i:768", "audiomode:i:2", "redirectdrives:i:0",
                     "disable wallpaper:i:1", "keyboardhook:i:0", "disable themes:i:1", "connection type:i:6", "networkautodetect:i:0",
                     &format!("full address:s:{}", address("Alt1"))] {
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

#[cfg(test)]
mod wrapper_tests {
    #[test]
    fn spots_a_wrapper() {
        assert!(super::is_wrapper(r"C:\Program Files\RDP Wrapper\TermWrap.dll"));
        assert!(super::is_wrapper(r"C:\Program Files\RDP Wrapper\rdpwrap.dll"));
        assert!(!super::is_wrapper(r"C:\Windows\System32\termsrv.dll"));
        assert!(!super::is_wrapper(""));
    }
}
