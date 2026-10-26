//! The caption editor: photos dropped on it (or its shortcut), else the screensaver's folder.

// A window, no console.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    photo_stamp_screensaver::win::captions::run(std::env::args_os().skip(1).map(Into::into).collect());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("photo-stamp-captions runs on Windows only");
    std::process::exit(1);
}
