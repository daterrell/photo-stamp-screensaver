//! Automatic updates on Windows: a scheduled task runs `photo-stamp-screensaver.scr /update` as
//! SYSTEM daily and after startup; setup registers it (`/install`) and removes it (`/uninstall`);
//! the settings dialog hands new server settings to an elevated `/updates-set <file>`.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{install_dir, store};
use crate::server::{Client, VERSION};
use crate::settings::{Updates, normalize_url};
use crate::update::{self, Outcome};

pub const TASK_NAME: &str = "Photo Stamp Screensaver update";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoStampScreensaver";
/// Downloads are ~20 MB; allow a slow line.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Don't flash a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where the updater logs (SYSTEM has no useful per-user folder).
fn log_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()))
        .join("Photo Stamp Screensaver")
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// `/update`: one check for a newer screensaver and towns database.
pub fn run() -> i32 {
    crate::log::init_named(&log_dir(), "update.log");
    let Some(updates) = store::machine_updates() else {
        crate::log::line("no place server set up for updates");
        return 0;
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            crate::log::line(&format!("where am I? {e}"));
            return 1;
        }
    };
    update::tidy(&exe);
    let client = Client::new(&updates.server_url, &updates.device_key, DOWNLOAD_TIMEOUT);
    let program = update::update_program(&client, updates.channel.server_name(), VERSION, &exe);
    let summary = match &program {
        Ok(Outcome::Updated(v)) => {
            let _ = set_installed_version(v);
            format!("Updated to {v} on {}.", now())
        }
        Ok(Outcome::UpToDate | Outcome::NothingPublished) => format!("Up to date, checked {}.", now()),
        Err(e) => format!("Couldn't update on {}: {e}", now()),
    };
    crate::log::line(&summary);
    let towns = install_dir().join("towns.sqlite");
    update::tidy(&towns);
    match update::update_towns(&client, &towns) {
        Ok(Some(built)) => crate::log::line(&format!("towns database updated (built {built})")),
        Ok(None) => {}
        Err(e) => crate::log::line(&format!("towns database: {e}")),
    }
    store::record_update_check(&summary);
    i32::from(program.is_err())
}

/// Settings > Apps shows the version that's installed now.
fn set_installed_version(version: &str) -> std::io::Result<()> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, REG_SZ, RegSetKeyValueW};
    let (key, name) = (super::wide(UNINSTALL_KEY), super::wide("DisplayVersion"));
    let value: Vec<u8> = super::wide(version).iter().flat_map(|c| c.to_le_bytes()).collect();
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            value.len() as u32,
        )
    };
    if err == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(err as i32)) }
}

/// `/updates-set <file>` (elevated): the update settings the dialog wrote to `file`, machine-wide.
pub fn set_from_file(file: &Path) -> i32 {
    let parsed = std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Updates>(&b).ok())
        .and_then(|u| Some(Updates { server_url: normalize_url(&u.server_url).ok()?, ..u }));
    let _ = std::fs::remove_file(file);
    match parsed.map(|u| store::save_machine_updates(&u)) {
        Some(Ok(())) => 0,
        _ => 1,
    }
}

/// `/install` (setup, elevated): this user's server for updates, if they have one, and the task.
pub fn install() -> i32 {
    crate::log::init_named(&log_dir(), "update.log");
    if store::machine_updates().is_none()
        && let Some(u) = store::load().0.updates()
        && let Err(e) = store::save_machine_updates(&u)
    {
        crate::log::line(&format!("couldn't keep the update settings: {e}"));
    }
    match register_task() {
        Ok(()) => 0,
        Err(e) => {
            crate::log::line(&format!("couldn't schedule updates: {e}"));
            1
        }
    }
}

/// `/uninstall` (elevated): no more update task or machine-wide settings.
pub fn uninstall() -> i32 {
    let _ = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    store::forget_machine();
    0
}

/// Daily at a random time around noon (missed runs happen at the next chance) and 15 minutes after
/// startup, as SYSTEM, only with a network. Through cmd.exe: Task Scheduler won't start a `.scr`
/// itself ("file not found").
fn task_xml(exe: &Path) -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let cmd = format!(r"{system_root}\System32\cmd.exe");
    // cmd /c "<command line>": the outer quotes go, the inner ones stay.
    let args = format!("/c \"\"{}\" /update\"", exe.display());
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let (cmd, args) = (escape(&cmd), escape(&args));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Keeps the Photo Stamp screensaver and its towns database up to date through the family's place server.</Description>
  </RegistrationInfo>
  <Triggers>
    <BootTrigger><Delay>PT15M</Delay></BootTrigger>
    <CalendarTrigger>
      <StartBoundary>2026-01-01T10:00:00</StartBoundary>
      <RandomDelay>PT4H</RandomDelay>
      <ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="System"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal>
  </Principals>
  <Settings>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>true</RunOnlyIfNetworkAvailable>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT1H</ExecutionTimeLimit>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
  </Settings>
  <Actions Context="System">
    <Exec><Command>{cmd}</Command><Arguments>{args}</Arguments></Exec>
  </Actions>
</Task>
"#
    )
}

fn register_task() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    // schtasks wants the XML as UTF-16 with a byte-order mark.
    let xml: Vec<u8> =
        std::iter::once(0xfeffu16).chain(task_xml(&exe).encode_utf16()).flat_map(u16::to_le_bytes).collect();
    let file = std::env::temp_dir().join(format!("pss-task-{}.xml", std::process::id()));
    std::fs::write(&file, xml)?;
    let status = Command::new("schtasks.exe")
        .args(["/Create", "/TN", TASK_NAME, "/F", "/XML"])
        .arg(&file)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    let _ = std::fs::remove_file(&file);
    match status? {
        s if s.success() => Ok(()),
        s => Err(std::io::Error::other(format!("schtasks failed ({s})"))),
    }
}
