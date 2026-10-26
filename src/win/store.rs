//! Settings in the registry, nothing in the photo folder:
//!
//! - `HKCU\Software\Photo Stamp Screensaver`: each user's settings. The device key is encrypted for
//!   that Windows user (DPAPI), so another account on the PC, or a copy of the registry, can't read
//!   it.
//! - `HKLM\Software\Photo Stamp Screensaver\Updates`: what the updater (SYSTEM) uses. Only an
//!   administrator can write there, so nobody else can point it at another server. Its key is
//!   encrypted for the machine, since SYSTEM has to read it.

use std::io;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    CryptUnprotectData,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_BINARY, REG_DWORD, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};

use super::wide;
use crate::settings::{
    self, Caption, Channel, Corner, DEFAULT_SECONDS, MAX_SECONDS, MIN_SECONDS, Order, Settings, Updates,
};

/// A registry key the settings live in.
struct Reg {
    root: HKEY,
    path: &'static str,
}

const USER: Reg = Reg { root: HKEY_CURRENT_USER, path: r"Software\Photo Stamp Screensaver" };
const MACHINE: Reg = Reg { root: HKEY_LOCAL_MACHINE, path: r"Software\Photo Stamp Screensaver\Updates" };

/// The saved settings. The first time, before anything is saved, the place server photo-stamp
/// uses on this PC is filled in (`true` says so).
pub fn load() -> (Settings, bool) {
    let mut s = Settings {
        folder: USER.string("Folder").unwrap_or_default(),
        server_url: USER.string("ServerUrl").unwrap_or_default(),
        device_key: USER.secret("DeviceKey").unwrap_or_default(),
        seconds_per_photo: USER
            .dword("SecondsPerPhoto")
            .unwrap_or(DEFAULT_SECONDS)
            .clamp(MIN_SECONDS, MAX_SECONDS),
        caption: Caption::from_index(USER.dword("Caption").unwrap_or(0)),
        corner: Corner::from_index(USER.dword("Corner").unwrap_or(0)),
        order: Order::from_index(USER.dword("Order").unwrap_or(0)),
        channel: Channel::from_index(USER.dword("Channel").unwrap_or(0)),
    };
    let saved_before = USER.dword("SecondsPerPhoto").is_some();
    if !saved_before && let Some((url, key)) = photo_stamp_server() {
        s.server_url = url;
        s.device_key = key;
        return (s, true);
    }
    (s, false)
}

pub fn save(s: &Settings) -> io::Result<()> {
    USER.set_string("Folder", &s.folder)?;
    USER.set_string("ServerUrl", &s.server_url)?;
    USER.set_secret("DeviceKey", &s.device_key, false)?;
    USER.set_dword("Caption", s.caption.index())?;
    USER.set_dword("Corner", s.corner.index())?;
    USER.set_dword("Order", s.order.index())?;
    USER.set_dword("Channel", s.channel.index())?;
    // Last: its presence says settings were saved (see `load`).
    USER.set_dword("SecondsPerPhoto", s.seconds_per_photo)
}

/// What the updater uses, if an administrator has set it.
pub fn machine_updates() -> Option<Updates> {
    Some(Updates {
        server_url: MACHINE.string("ServerUrl").filter(|u| !u.is_empty())?,
        device_key: MACHINE.secret("DeviceKey").unwrap_or_default(),
        channel: Channel::from_index(MACHINE.dword("Channel").unwrap_or(0)),
    })
}

/// Needs administrator rights.
pub fn save_machine_updates(u: &Updates) -> io::Result<()> {
    MACHINE.set_string("ServerUrl", &u.server_url)?;
    MACHINE.set_secret("DeviceKey", &u.device_key, true)?;
    MACHINE.set_dword("Channel", u.channel.index())
}

/// Removes the machine-wide settings (uninstalling).
pub fn forget_machine() {
    let path = wide(r"Software\Photo Stamp Screensaver");
    unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, path.as_ptr()) };
}

/// The caption editor's installed version (setup's, or the last update's); it's built with the
/// screensaver but updated on its own, so it may lag behind after a failed update.
pub fn captions_version() -> Option<String> {
    MACHINE.string("CaptionsVersion")
}

pub fn set_captions_version(version: &str) -> io::Result<()> {
    MACHINE.set_string("CaptionsVersion", version)
}

/// What the updater last did ("Up to date, checked 2026-10-21 14:00"), for the settings dialog.
pub fn last_update_check() -> Option<String> {
    MACHINE.string("LastCheck")
}

pub fn record_update_check(text: &str) {
    let _ = MACHINE.set_string("LastCheck", text);
}

/// Whether this user said no to using `u` for updates (so they're not asked again for the same).
pub fn declined(u: &Updates) -> bool {
    USER.string("UpdatesDeclined").as_deref() == Some(&fingerprint(u))
}

pub fn set_declined(u: &Updates) {
    let _ = USER.set_string("UpdatesDeclined", &fingerprint(u));
}

fn fingerprint(u: &Updates) -> String {
    crate::update::sha256_hex(serde_json::to_string(u).unwrap_or_default().as_bytes())
}

fn photo_stamp_server() -> Option<(String, String)> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"photo-stamp\client.json");
    settings::photo_stamp_server(&std::fs::read(path).ok()?)
}

impl Reg {
    fn set(&self, kind: u32, name: &str, data: &[u8]) -> io::Result<()> {
        let (key, name) = (wide(self.path), wide(name));
        let err = unsafe {
            RegSetKeyValueW(
                self.root,
                key.as_ptr(),
                name.as_ptr(),
                kind,
                data.as_ptr().cast(),
                data.len() as u32,
            )
        };
        if err == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(err as i32)) }
    }

    fn set_string(&self, name: &str, value: &str) -> io::Result<()> {
        let bytes: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
        self.set(REG_SZ, name, &bytes)
    }

    fn set_dword(&self, name: &str, value: u32) -> io::Result<()> {
        self.set(REG_DWORD, name, &value.to_le_bytes())
    }

    /// Encrypted for this user, or for the machine; an empty value removes it.
    fn set_secret(&self, name: &str, value: &str, machine: bool) -> io::Result<()> {
        if value.is_empty() {
            let (key, name) = (wide(self.path), wide(name));
            // Missing already is fine.
            unsafe { RegDeleteKeyValueW(self.root, key.as_ptr(), name.as_ptr()) };
            return Ok(());
        }
        self.set(REG_BINARY, name, &protect(value.as_bytes(), machine)?)
    }

    /// The raw value, or `None` when it's missing or of another type.
    fn get(&self, flags: u32, name: &str) -> Option<Vec<u8>> {
        let (key, name) = (wide(self.path), wide(name));
        let mut len = 0u32;
        let query = |buf: *mut u8, len: &mut u32| unsafe {
            RegGetValueW(self.root, key.as_ptr(), name.as_ptr(), flags, null_mut(), buf.cast(), len)
        };
        if query(null_mut(), &mut len) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        (query(buf.as_mut_ptr(), &mut len) == ERROR_SUCCESS).then(|| {
            buf.truncate(len as usize);
            buf
        })
    }

    fn string(&self, name: &str) -> Option<String> {
        let units: Vec<u16> = self
            .get(RRF_RT_REG_SZ, name)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
    }

    fn dword(&self, name: &str) -> Option<u32> {
        Some(u32::from_le_bytes(self.get(RRF_RT_REG_DWORD, name)?.try_into().ok()?))
    }

    fn secret(&self, name: &str) -> Option<String> {
        String::from_utf8(unprotect(&self.get(RRF_RT_REG_BINARY, name)?)?).ok()
    }
}

fn protect(data: &[u8], machine: bool) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let description = wide("Photo Stamp screensaver device key");
    let flags = CRYPTPROTECT_UI_FORBIDDEN | if machine { CRYPTPROTECT_LOCAL_MACHINE } else { 0 };
    let ok =
        unsafe { CryptProtectData(&input, description.as_ptr(), null(), null(), null(), flags, &mut output) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(take_blob(output))
}

fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let ok = unsafe {
        CryptUnprotectData(&input, null_mut(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
    };
    (ok != 0).then(|| take_blob(output))
}

/// Copies out a blob DPAPI allocated, and frees it.
fn take_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
    unsafe { LocalFree(blob.pbData.cast()) };
    bytes
}
