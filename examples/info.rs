//! Prints what the screensaver reads from photos: `cargo run --example info -- <files>`.

use photo_stamp_screensaver::exif::Info;

fn main() {
    for path in std::env::args().skip(1) {
        match std::fs::read(&path) {
            Ok(bytes) => println!("{path}: {:?}", Info::from_file_bytes(&bytes)),
            Err(e) => println!("{path}: {e}"),
        }
    }
}
