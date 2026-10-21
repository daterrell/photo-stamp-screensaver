// A screensaver has no console.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use photo_stamp_screensaver::args::{self, Mode};
    use photo_stamp_screensaver::win;

    match args::parse(std::env::args().skip(1)) {
        Mode::Run => win::saver::run(),
        Mode::Preview { parent } => win::saver::preview(parent),
        Mode::Window => win::saver::window(),
        Mode::Settings { parent } => win::dialog::show(parent),
        Mode::Update => std::process::exit(win::updater::run()),
        Mode::UpdatesSet(file) => {
            std::process::exit(win::updater::set_from_file(std::path::Path::new(&file)))
        }
        Mode::Install => std::process::exit(win::updater::install()),
        Mode::Uninstall => std::process::exit(win::updater::uninstall()),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("photo-stamp-screensaver runs on Windows only");
    std::process::exit(1);
}
