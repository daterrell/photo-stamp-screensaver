//! photo-stamp-screensaver: a Windows screensaver showing a folder of photos, with where and when
//! each was taken from the family's place server. See README.md.

pub mod args;
pub mod cache;
pub mod caption;
pub mod exif;
pub mod geo;
#[cfg(feature = "heif")]
pub mod heif;
pub mod library;
pub mod log;
pub mod photos;
pub mod places;
pub mod playlist;
pub mod server;
pub mod settings;
pub mod slide;
pub mod update;
#[cfg(windows)]
pub mod win;
pub mod xmp;
