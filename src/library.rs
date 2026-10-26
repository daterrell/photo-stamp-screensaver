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
use crate::playlist::{self, Entry, Playlist, State};
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
    captions: Captions,
}

/// Where the caption shown comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Worked out from the photo's metadata and the place server.
    Automatic,
    /// Edited by hand, kept in the photo.
    Photo,
    /// Edited by hand, kept in the folder's cache.
    Cache,
    /// photo-stamp already burned one in: none shown.
    Stamped,
}

/// Where an edited caption was kept.
#[derive(Debug)]
pub struct Saved {
    pub in_photo: Result<(), String>,
    pub in_cache: Result<(), String>,
}

impl Saved {
    /// What to tell the person. `reset`: they went back to the automatic caption.
    pub fn message(&self, reset: bool) -> String {
        match (&self.in_photo, &self.in_cache, reset) {
            (Ok(()), Ok(()), false) => "Saved, in the photo and for the screensaver.".into(),
            (Ok(()), Ok(()), true) => "Back to the automatic caption.".into(),
            (Ok(()), Err(_), false) => {
                "Saved in the photo. The screensaver shows it once the photo is in its folder.".into()
            }
            (Ok(()), Err(_), true) => "Back to the automatic caption.".into(),
            (Err(why), Ok(()), false) => {
                format!("Saved for the screensaver, but not in the photo: {why}.")
            }
            (Err(why), Ok(()), true) => {
                format!("Back to the automatic caption for the screensaver, but not in the photo: {why}.")
            }
            (Err(photo), Err(cache), _) => format!("Couldn't save it: {photo}; {cache}."),
        }
    }
}

/// Captions for the photos under one top folder: what the screensaver shows, and what the caption
/// editor starts from. An edit in the shared cache comes first, then one kept in the photo, then
/// the automatic caption.
pub struct Captions {
    cache: Option<Arc<Cache>>,
    geo: Geo,
}

impl Captions {
    /// For the photos under `folder`, with the settings' place server. `cache`: keep what's learned
    /// in the folder's cache (or the per-PC copy in `data_dir`).
    pub fn open(
        folder: &Path,
        settings: &Settings,
        data_dir: &Path,
        towns: Option<&Path>,
        cache: bool,
    ) -> Self {
        let cache = cache.then(|| Cache::open(folder, data_dir)).flatten().map(Arc::new);
        let client = (!settings.server_url.is_empty())
            .then(|| Client::new(&settings.server_url, &settings.device_key, geo::REQUEST_TIMEOUT));
        let towns = towns.and_then(|p| {
            Towns::open(p).map_err(|e| crate::log::line(&format!("towns database {}: {e}", p.display()))).ok()
        });
        Self { cache: cache.clone(), geo: Geo::new(client, cache, towns) }
    }

    pub fn cache(&self) -> Option<&Cache> {
        self.cache.as_deref()
    }

    /// A photo's metadata: remembered from before, or read from `bytes` (and remembered). `path`:
    /// relative to the top folder, with `/`.
    pub fn info(&self, path: &str, size: u64, mtime: i64, bytes: &[u8]) -> Info {
        let cached = self.cache.as_ref().and_then(|c| c.photo(path, size, mtime));
        cached.unwrap_or_else(|| {
            let info = read_info(path, bytes);
            if let Some(c) = &self.cache {
                c.put_photo(path, size, mtime, &info);
            }
            info
        })
    }

    /// The caption worked out from the metadata, the place looked up the first time.
    pub fn automatic(&self, info: &Info) -> Vec<String> {
        let place = info.gps.and_then(|at| self.geo.place(at));
        caption::lines(info, place.as_ref())
    }

    /// Keeps an edited caption (`None`: back to the automatic one) for the photo at `path` (`file`
    /// on disk) in the photo, when its kind can hold one, and in the cache. Each `Err` says why
    /// it couldn't be kept there.
    pub fn save(&self, path: &str, file: &Path, lines: Option<&[String]>) -> Saved {
        let in_photo = if crate::xmp::can_write(file) {
            crate::xmp::write(file, lines)
        } else {
            Err("this kind of photo can't hold a caption".into())
        };
        // Its modified time is kept, so a same-length change wouldn't otherwise be noticed.
        if in_photo.is_ok()
            && let (Some(cache), Ok(bytes), Ok(meta)) =
                (&self.cache, std::fs::read(file), std::fs::metadata(file))
        {
            cache.put_photo(path, meta.len(), crate::playlist::mtime(&meta), &read_info(path, &bytes));
        }
        let in_cache = match &self.cache {
            Some(cache) => cache.put_edit(path, lines).map_err(|e| e.to_string()),
            None => Err("the photo isn't in the screensaver's folder".into()),
        };
        Saved { in_photo, in_cache }
    }

    /// The caption to show, and where it came from.
    pub fn shown(&self, path: &str, info: &Info) -> (Vec<String>, Source) {
        if info.stamped {
            return (Vec::new(), Source::Stamped);
        }
        if let Some(lines) = self.cache.as_ref().and_then(|c| c.edit(path)) {
            return (lines, Source::Cache);
        }
        if let Some(lines) = &info.caption {
            return (lines.clone(), Source::Photo);
        }
        (self.automatic(info), Source::Automatic)
    }
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
        let captions = Captions::open(&folder, settings, data_dir, towns, true);
        let dates = captions.cache().map(|c| c.dates()).unwrap_or_default();
        let state = remember
            .then(|| {
                State::open(&data_dir.join("state.sqlite"), &folder)
                    .map_err(|e| crate::log::line(&format!("can't keep the slideshow's place: {e}")))
                    .ok()
            })
            .flatten();
        let playlist = Playlist::new(entries, settings.order, &dates, state, fastrand::Rng::new());
        Ok(Self { folder, playlist: Mutex::new(playlist), captions })
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
        let info = self.captions.info(&entry.path, entry.size, entry.mtime, &bytes);
        let (caption, _) = self.captions.shown(&entry.path, &info);
        Ready { entry, bytes, info, caption }
    }
}

fn read_info(path: &str, bytes: &[u8]) -> Info {
    #[cfg(feature = "heif")]
    if crate::heif::is_heif(path) {
        let mut info = Info::from_file_bytes(bytes);
        // libheif applies the container's rotation; EXIF's doesn't apply to HEIF.
        info.orientation = 1;
        return info;
    }
    let _ = path;
    Info::from_file_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::tests::{jpeg_with, sample_tiff};

    #[test]
    fn readies_photos_with_captions_and_remembers_them() {
        let dir = std::env::temp_dir().join(format!("pss-test-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let photos = dir.join("photos");
        std::fs::create_dir_all(photos.join("trip")).unwrap();
        let jpeg = jpeg_with(&sample_tiff());
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

    #[test]
    fn edits_come_before_the_automatic_caption() {
        let dir = std::env::temp_dir().join(format!("pss-test-edits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jpg");
        std::fs::write(&path, jpeg_with(&sample_tiff())).unwrap();
        let towns = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/places.sqlite");
        let captions = Captions::open(&dir, &Settings::default(), &dir.join("data"), Some(&towns), true);
        let shown = |c: &Captions| {
            let bytes = std::fs::read(&path).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            let info = c.info("a.jpg", meta.len(), crate::playlist::mtime(&meta), &bytes);
            c.shown("a.jpg", &info)
        };
        let (lines, source) = shown(&captions);
        assert_eq!(source, Source::Automatic);
        assert!(lines[0].starts_with("Carmel"), "{lines:?}");

        // In the photo only (another folder's cache has it).
        let mine = vec!["Pebble Beach".to_owned()];
        crate::xmp::write(&path, Some(&mine)).unwrap();
        assert_eq!(shown(&captions), (mine.clone(), Source::Photo));
        // The same length again: the modified time is kept, but it's still noticed.
        let same = vec!["Pebble Beech".to_owned()];
        let saved = captions.save("a.jpg", &path, Some(&same));
        assert!(saved.in_photo.is_ok() && saved.in_cache.is_ok(), "{saved:?}");
        assert_eq!(shown(&captions), (same.clone(), Source::Cache));
        captions.cache().unwrap().put_edit("a.jpg", None).unwrap();
        assert_eq!(shown(&captions), (same, Source::Photo));
        // No caption at all.
        captions.save("a.jpg", &path, Some(&[]));
        assert_eq!(shown(&captions), (vec![], Source::Cache));
        let saved = captions.save("a.jpg", &path, None);
        assert!(saved.in_photo.is_ok() && saved.in_cache.is_ok(), "{saved:?}");
        assert_eq!(shown(&captions), (lines, Source::Automatic));

        // Without a cache, the photo alone keeps it.
        let alone = Captions::open(&dir, &Settings::default(), &dir.join("data"), None, false);
        let saved = alone.save("a.jpg", &path, Some(&mine));
        assert!(saved.in_photo.is_ok() && saved.in_cache.is_err());
        assert!(saved.message(false).starts_with("Saved in the photo."));
        alone.save("a.jpg", &path, None);
    }
}
