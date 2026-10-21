//! photo-stamp-screensaver: a Windows screensaver showing a folder of photos, with where and when
//! each was taken from the family's place server. See README.md.

pub mod args;
pub mod server;
pub mod settings;
#[cfg(windows)]
pub mod win;
