// A screensaver has no console.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use photo_stamp_screensaver::args::{self, Mode};
    use photo_stamp_screensaver::win;

    match args::parse(std::env::args().skip(1)) {
        Mode::Run => win::saver::run(),
        Mode::Preview { parent } => win::saver::preview(parent),
        Mode::Settings { parent } => win::dialog::show(parent),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("photo-stamp-screensaver runs on Windows only");
    std::process::exit(1);
}
