//! The photos the screensaver shows, and getting the next one ready while the current one is on
//! screen: its bytes, metadata (from the shared cache when it was read before) and caption (with
//! the place looked up now, the first time a spot comes up).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::cache::Cache;
use crate::caption;
use crate::exif::Info;
use crate::geo::{self, Geo};
use crate::places::Towns;
use crate::playlist::{self, Entry, Playlist, Rng, State};
use crate::server::Client;
use crate::settings::Settings;

/// A photo ready to decode and show.
pub struct Ready {
    pub entry: Entry,
    pub bytes: Vec<u8>,
    pub info: Info,
    /// Empty when there's nothing to say, or photo-stamp already burned it in.
    pub caption: Vec<String>,
}

pub struct Library {
    folder: PathBuf,
    playlist: Mutex<Playlist>,
    cache: Option<Arc<Cache>>,
    geo: Geo,
}

/// Unreadable photos in a row before giving up for now.
const MAX_SKIPS: usize = 20;

impl Library {
    /// Finds the photos (slow on a big network folder: call it off the UI thread). `remember`: keep
    /// where the slideshow got to (not for the preview). `Err`: what to tell the person instead of a
    /// slideshow.
    pub fn open(
        settings: &Settings,
        data_dir: &Path,
        towns: Option<&Path>,
        remember: bool,
    ) -> Result<Self, String> {
        if settings.folder.is_empty() {
            return Err("Choose a photo folder in the screensaver's settings.".into());
        }
        let folder = PathBuf::from(&settings.folder);
        if !folder.is_dir() {
            return Err(format!("Can't find {}.", settings.folder));
        }
        let entries = playlist::scan(&folder);
        if entries.is_empty() {
            return Err(format!("There are no photos in {}.", settings.folder));
        }
        crate::log::line(&format!("{} photos in {}", entries.len(), folder.display()));
        let cache = Cache::open(&folder, data_dir).map(Arc::new);
        let dates = cache.as_ref().map(|c| c.dates()).unwrap_or_default();
        let state = remember
            .then(|| {
                State::open(&data_dir.join("state.sqlite"), &folder)
                    .map_err(|e| crate::log::line(&format!("can't keep the slideshow's place: {e}")))
                    .ok()
            })
            .flatten();
        let playlist = Playlist::new(entries, settings.order, &dates, state, Rng::from_time());
        let client = (!settings.server_url.is_empty())
            .then(|| Client::new(&settings.server_url, &settings.device_key, geo::REQUEST_TIMEOUT));
        let towns = towns.and_then(|p| {
            Towns::open(p).map_err(|e| crate::log::line(&format!("towns database {}: {e}", p.display()))).ok()
        });
        Ok(Self {
            folder,
            playlist: Mutex::new(playlist),
            cache: cache.clone(),
            geo: Geo::new(client, cache, towns),
        })
    }

    pub fn len(&self) -> usize {
        self.playlist.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The next photo that can be read, with its caption.
    pub fn next(&self) -> Option<Ready> {
        for _ in 0..MAX_SKIPS {
            let entry = self.playlist.lock().unwrap().pick()?;
            match std::fs::read(self.folder.join(&entry.path)) {
                Ok(bytes) => return Some(self.describe(entry, bytes)),
                Err(e) => crate::log::line(&format!("can't read {}: {e}", entry.path)),
            }
        }
        None
    }

    fn describe(&self, entry: Entry, bytes: Vec<u8>) -> Ready {
        let cached = self.cache.as_ref().and_then(|c| c.photo(&entry.path, entry.size, entry.mtime));
        let info = cached.unwrap_or_else(|| {
            let info = read_info(&entry.path, &bytes);
            if let Some(c) = &self.cache {
                c.put_photo(&entry.path, entry.size, entry.mtime, &info);
            }
            info
        });
        let caption = if info.stamped {
            Vec::new()
        } else {
            let place = info.gps.and_then(|at| self.geo.place(at));
            caption::lines(&info, place.as_ref())
        };
        Ready { entry, bytes, info, caption }
    }
}

fn read_info(path: &str, bytes: &[u8]) -> Info {
    #[cfg(feature = "heif")]
    if crate::heif::is_heif(path) {
        let mut info = crate::heif::exif(bytes).map(|t| crate::exif::parse_tiff(&t)).unwrap_or_default();
        // libheif applies the container's rotation; EXIF's doesn't apply to HEIF.
        info.orientation = 1;
        info.stamped = crate::exif::has_marker(bytes);
        return info;
    }
    let _ = path;
    Info::from_file_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::tests::sample_tiff;

    #[test]
    fn readies_photos_with_captions_and_remembers_them() {
        let dir = std::env::temp_dir().join(format!("pss-test-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let photos = dir.join("photos");
        std::fs::create_dir_all(photos.join("trip")).unwrap();
        // A JPEG with nothing but the sample EXIF.
        let t = sample_tiff();
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend(((t.len() + 8) as u16).to_be_bytes());
        jpeg.extend(b"Exif\0\0");
        jpeg.extend(&t);
        jpeg.extend([0xff, 0xd9]);
        std::fs::write(photos.join("trip/a.jpg"), &jpeg).unwrap();

        let settings = Settings { folder: photos.to_string_lossy().into(), ..Default::default() };
        let towns = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/places.sqlite");
        let library = Library::open(&settings, &dir.join("data"), Some(&towns), true).unwrap();
        assert_eq!(library.len(), 1);
        let ready = library.next().unwrap();
        assert_eq!(ready.entry.path, "trip/a.jpg");
        assert!(ready.caption[0].starts_with("Carmel"), "{:?}", ready.caption);
        assert_eq!(ready.caption.last().unwrap(), "Tue 3 Jun 2025, 15:10 PDT");
        // The metadata is in the shared cache now, beside the photos.
        assert!(photos.join(crate::cache::FILE_NAME).is_file());

        let empty = Settings { folder: dir.join("data").to_string_lossy().into(), ..Default::default() };
        assert!(
            Library::open(&empty, &dir.join("data2"), None, true)
                .err()
                .unwrap()
                .starts_with("There are no photos")
        );
        assert!(Library::open(&Settings::default(), &dir, None, true).is_err());
    }
}
