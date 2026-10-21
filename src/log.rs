//! A small log for finding out why a photo didn't show: `screensaver.log` in the data folder,
//! started over once it passes 1 MB.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 1 << 20;

static FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Logs to `data_dir/screensaver.log` from now on (until then, nothing is logged).
pub fn init(data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let path = data_dir.join("screensaver.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    *FILE.lock().unwrap() = Some(path);
}

pub fn line(message: &str) {
    let file = FILE.lock().unwrap();
    let Some(path) = file.as_ref() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            f,
            "{} [{}] {message}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            std::process::id()
        );
    }
}
