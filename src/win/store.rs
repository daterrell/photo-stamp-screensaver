//! Settings in `HKCU\Software\Photo Stamp Screensaver`: per user, nothing in the photo folder.
//! The device key is encrypted for this Windows user (DPAPI), so another account on the PC, or a
//! copy of the registry, can't read it.

use std::io;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_DWORD, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};

use super::wide;
use crate::settings::{self, DEFAULT_SECONDS, MAX_SECONDS, MIN_SECONDS, Settings};

const KEY: &str = r"Software\Photo Stamp Screensaver";

/// The saved settings. The first time, before anything is saved, the place server photo-stamp
/// uses on this PC is filled in (`true` says so).
pub fn load() -> (Settings, bool) {
    let mut s = Settings {
        folder: get_string("Folder").unwrap_or_default(),
        server_url: get_string("ServerUrl").unwrap_or_default(),
        device_key: get_binary("DeviceKey")
            .and_then(|b| unprotect(&b))
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default(),
        seconds_per_photo: get_dword("SecondsPerPhoto")
            .unwrap_or(DEFAULT_SECONDS)
            .clamp(MIN_SECONDS, MAX_SECONDS),
    };
    let saved_before = get_dword("SecondsPerPhoto").is_some();
    if !saved_before && let Some((url, key)) = photo_stamp_server() {
        s.server_url = url;
        s.device_key = key;
        return (s, true);
    }
    (s, false)
}

pub fn save(s: &Settings) -> io::Result<()> {
    set(REG_SZ, "Folder", &string_bytes(&s.folder))?;
    set(REG_SZ, "ServerUrl", &string_bytes(&s.server_url))?;
    if s.device_key.is_empty() {
        let (key, name) = (wide(KEY), wide("DeviceKey"));
        // Missing already is fine.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    } else {
        set(REG_BINARY, "DeviceKey", &protect(s.device_key.as_bytes())?)?;
    }
    set(REG_DWORD, "SecondsPerPhoto", &s.seconds_per_photo.to_le_bytes())
}

fn photo_stamp_server() -> Option<(String, String)> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"photo-stamp\client.json");
    settings::photo_stamp_server(&std::fs::read(path).ok()?)
}

fn string_bytes(s: &str) -> Vec<u8> {
    wide(s).iter().flat_map(|c| c.to_le_bytes()).collect()
}

fn set(kind: u32, name: &str, data: &[u8]) -> io::Result<()> {
    let (key, name) = (wide(KEY), wide(name));
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            kind,
            data.as_ptr().cast(),
            data.len() as u32,
        )
    };
    if err == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(err as i32)) }
}

/// The raw value, or `None` when it's missing or of another type.
fn get(flags: u32, name: &str) -> Option<Vec<u8>> {
    let (key, name) = (wide(KEY), wide(name));
    let mut len = 0u32;
    let query = |buf: *mut u8, len: &mut u32| unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), flags, null_mut(), buf.cast(), len)
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

fn get_string(name: &str) -> Option<String> {
    let units: Vec<u16> =
        get(RRF_RT_REG_SZ, name)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
}

fn get_dword(name: &str) -> Option<u32> {
    Some(u32::from_le_bytes(get(RRF_RT_REG_DWORD, name)?.try_into().ok()?))
}

fn get_binary(name: &str) -> Option<Vec<u8>> {
    get(RRF_RT_REG_BINARY, name)
}

fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let description = wide("Photo Stamp screensaver device key");
    let ok = unsafe {
        CryptProtectData(
            &input,
            description.as_ptr(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
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
