//! The Windows side: the settings dialog, where settings are kept, decoding and drawing the photos.

mod decode;
pub mod dialog;
mod folder;
pub mod saver;
pub mod store;
pub mod updater;

/// Where setup puts the uninstaller and the towns database (the `.scr` itself is in System32).
pub fn install_dir() -> std::path::PathBuf {
    let dir = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .unwrap_or_else(|| r"C:\Program Files".into());
    std::path::PathBuf::from(dir).join("Photo Stamp Screensaver")
}

/// A NUL-terminated UTF-16 copy of `s`, for Windows calls.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `MAKEINTRESOURCE`: a resource ID where Windows expects a name.
fn resource(id: u16) -> windows_sys::core::PCWSTR {
    id as usize as windows_sys::core::PCWSTR
}
