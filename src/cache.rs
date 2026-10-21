//! What's been learned about the photos, kept beside them so every PC showing the folder shares it:
//! a hidden SQLite file in the top folder (`.photo-stamp-screensaver.sqlite`) with each photo's
//! metadata and the place server's answers. The photos themselves are never changed.
//!
//! The folder may be read-only, on a NAS that several PCs use at once, or synced by OneDrive: when
//! the file can't be opened or written there, a per-PC copy in the data folder is used instead.
//! Rollback journaling (not WAL, which needs shared memory a network share can't give) and a busy
//! timeout keep concurrent PCs from tripping over each other; a failed write only costs a repeat
//! lookup.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, params};

use crate::exif::Info;
use crate::places::{GpsPoint, Place};

pub const FILE_NAME: &str = ".photo-stamp-screensaver.sqlite";
/// Beyond this many answers the oldest are dropped.
const MAX_ANSWERS: i64 = 200_000;

/// A remembered answer from the place server, and which data it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub place: Option<Place>,
    pub data_version: String,
    pub rev: u64,
}

pub struct Cache {
    conn: Mutex<Connection>,
    /// Where it ended up: in the photo folder, or the per-PC fallback.
    pub path: PathBuf,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// FNV-1a: a stable name for a folder's per-PC cache (std's hasher may change between Rust versions).
fn folder_hash(folder: &Path) -> u64 {
    folder
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

impl Cache {
    /// The cache for `folder`: in it if possible, else in `fallback_dir`.
    pub fn open(folder: &Path, fallback_dir: &Path) -> Option<Self> {
        let shared = folder.join(FILE_NAME);
        match Self::open_at(&shared) {
            Ok(cache) => {
                hide(&shared);
                return Some(cache);
            }
            Err(e) => crate::log::line(&format!("can't keep the cache in {}: {e}", folder.display())),
        }
        let local = fallback_dir.join(format!("cache-{:016x}.sqlite", folder_hash(folder)));
        match std::fs::create_dir_all(fallback_dir)
            .map_err(|e| e.to_string())
            .and_then(|()| Self::open_at(&local).map_err(|e| e.to_string()))
        {
            Ok(cache) => Some(cache),
            Err(e) => {
                crate::log::line(&format!("can't keep a cache at all ({}): {e}", local.display()));
                None
            }
        }
    }

    fn open_at(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS photo(
               path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
               taken TEXT, offset_s INTEGER, gps_time INTEGER, lat REAL, lon REAL,
               orientation INTEGER NOT NULL, stamped INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS answer(
               key TEXT PRIMARY KEY, place TEXT, data_version TEXT NOT NULL, rev INTEGER NOT NULL,
               at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS answer_at ON answer(at);
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);",
        )?;
        // Proves it can be written, not just read.
        conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES('opened', ?1)", [now().to_string()])?;
        Ok(Self { conn: Mutex::new(conn), path: path.to_owned() })
    }

    /// A photo's metadata, if it was read before and the file hasn't changed since.
    pub fn photo(&self, path: &str, size: u64, mtime: i64) -> Option<Info> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT taken, offset_s, gps_time, lat, lon, orientation, stamped FROM photo
             WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            params![path, size as i64, mtime],
            |r| {
                let gps = match (r.get::<_, Option<f64>>(3)?, r.get::<_, Option<f64>>(4)?) {
                    (Some(lat), Some(lon)) => Some(GpsPoint { lat, lon }),
                    _ => None,
                };
                Ok(Info {
                    taken: r
                        .get::<_, Option<String>>(0)?
                        .and_then(|t| NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S").ok()),
                    offset: r.get::<_, Option<i32>>(1)?.and_then(FixedOffset::east_opt),
                    gps_time: r.get::<_, Option<i64>>(2)?.and_then(|t| DateTime::from_timestamp(t, 0)),
                    gps,
                    orientation: r.get::<_, i64>(5)? as u8,
                    stamped: r.get::<_, i64>(6)? != 0,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn put_photo(&self, path: &str, size: u64, mtime: i64, info: &Info) {
        let conn = self.conn.lock().unwrap();
        let result = conn.execute(
            "INSERT OR REPLACE INTO photo(path, size, mtime, taken, offset_s, gps_time, lat, lon, orientation, stamped)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                path,
                size as i64,
                mtime,
                info.taken.map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()),
                info.offset.map(|o| o.local_minus_utc()),
                info.gps_time.map(|t| t.timestamp()),
                info.gps.map(|g| g.lat),
                info.gps.map(|g| g.lon),
                info.orientation as i64,
                info.stamped as i64,
            ],
        );
        if let Err(e) = result {
            crate::log::line(&format!("couldn't remember {path}: {e}"));
        }
    }

    /// When each photo read so far was taken (camera clock, as seconds), for ordering by date.
    pub fn dates(&self) -> std::collections::HashMap<String, i64> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare("SELECT path, taken FROM photo WHERE taken IS NOT NULL") else {
            return Default::default();
        };
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|(p, t)| {
                        let t = NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S").ok()?;
                        Some((p, t.and_utc().timestamp()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn answer(&self, at: GpsPoint) -> Option<Answer> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT place, data_version, rev FROM answer WHERE key = ?1", [at.cache_key()], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })
        .optional()
        .ok()
        .flatten()
        .map(|(place, data_version, rev)| Answer {
            place: place.and_then(|p| serde_json::from_str(&p).ok()),
            data_version,
            rev: rev as u64,
        })
    }

    pub fn put_answer(&self, at: GpsPoint, answer: &Answer) {
        let json = answer.place.as_ref().and_then(|p| serde_json::to_string(p).ok());
        let conn = self.conn.lock().unwrap();
        let result = conn
            .execute(
                "INSERT OR REPLACE INTO answer(key, place, data_version, rev, at) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![at.cache_key(), json, answer.data_version, answer.rev as i64, now()],
            )
            .and_then(|_| {
                conn.execute(
                    "DELETE FROM answer WHERE key IN (SELECT key FROM answer ORDER BY at DESC LIMIT -1 OFFSET ?1)",
                    [MAX_ANSWERS],
                )
            });
        if let Err(e) = result {
            crate::log::line(&format!("couldn't remember the place at {}: {e}", at.cache_key()));
        }
    }
}

/// Keeps the cache out of sight in Explorer (and the photo apps that show hidden files' absence).
#[cfg(windows)]
fn hide(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_HIDDEN, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, SetFileAttributesW,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let attrs = GetFileAttributesW(wide.as_ptr());
        if attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_HIDDEN == 0 {
            SetFileAttributesW(wide.as_ptr(), attrs | FILE_ATTRIBUTE_HIDDEN);
        }
    }
}

/// Elsewhere the leading dot hides it.
#[cfg(not(windows))]
fn hide(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    pub fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pss-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remembers_photos_and_answers() {
        let dir = temp("cache");
        let cache = Cache::open(&dir, &dir.join("fallback")).unwrap();
        assert_eq!(cache.path, dir.join(FILE_NAME));
        let info = Info {
            taken: NaiveDate::from_ymd_opt(2025, 6, 3).unwrap().and_hms_opt(15, 10, 42),
            offset: FixedOffset::west_opt(7 * 3600),
            gps_time: DateTime::from_timestamp(1_748_988_642, 0),
            gps: Some(GpsPoint { lat: 36.5655, lon: -121.9453 }),
            orientation: 6,
            stamped: false,
        };
        cache.put_photo("trip/a.jpg", 100, 5, &info);
        assert_eq!(cache.photo("trip/a.jpg", 100, 5), Some(info.clone()));
        // Changed since: read it again.
        assert_eq!(cache.photo("trip/a.jpg", 101, 5), None);
        assert_eq!(cache.dates().get("trip/a.jpg"), Some(&info.taken.unwrap().and_utc().timestamp()));

        let at = GpsPoint { lat: 36.56551, lon: -121.94532 };
        let answer = Answer {
            place: Some(Place { primary: "Pebble Beach Golf Course".into(), ..Default::default() }),
            data_version: "v1".into(),
            rev: 2,
        };
        cache.put_answer(at, &answer);
        // The same spot to ~11 m.
        assert_eq!(cache.answer(GpsPoint { lat: 36.56549, lon: -121.94528 }), Some(answer));
        drop(cache);
        // Kept across runs.
        assert!(Cache::open(&dir, &dir.join("fallback")).unwrap().photo("trip/a.jpg", 100, 5).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_when_the_folder_is_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("readonly");
        let photos = dir.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::set_permissions(&photos, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores permissions, so this only proves something as a normal user.
        if std::fs::write(photos.join("probe"), b"").is_ok() {
            return;
        }
        let cache = Cache::open(&photos, &dir.join("local")).unwrap();
        assert!(cache.path.starts_with(dir.join("local")), "{}", cache.path.display());
        std::fs::set_permissions(&photos, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
