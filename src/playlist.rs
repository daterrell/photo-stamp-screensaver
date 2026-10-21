//! Which photo comes next: finding the photos in the folder, and the order setting (shuffled with
//! no repeats until all have shown, by date taken, or by folder). Where the slideshow got to is
//! kept per PC (`state.sqlite` in the data folder), not in the shared folder, so each PC goes
//! through the photos at its own pace.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::settings::Order;

/// A photo in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the top folder, with `/` between folders.
    pub path: String,
    pub size: u64,
    /// Modified time, as seconds.
    pub mtime: i64,
}

/// The extensions shown. HEIC needs libheif, which only the Windows build links.
pub fn is_photo(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "tif" | "tiff" | "webp" | "bmp" | "gif" => true,
        "heic" | "heif" | "hif" => cfg!(feature = "heif"),
        _ => false,
    }
}

/// Folders that only hold other programs' leftovers: recycle bins, NAS thumbnails.
fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || ["$recycle.bin", "system volume information", "@eadir", "#recycle", "#snapshot"]
            .contains(&name.to_ascii_lowercase().as_str())
}

#[cfg(windows)]
fn hidden(entry: &walkdir::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM.
    entry.metadata().is_ok_and(|m| m.file_attributes() & 0x6 != 0)
}

#[cfg(not(windows))]
fn hidden(_: &walkdir::DirEntry) -> bool {
    false
}

/// Every photo under `folder`, skipping hidden files and folders.
pub fn scan(folder: &Path) -> Vec<Entry> {
    let walk = walkdir::WalkDir::new(folder).follow_links(true).into_iter().filter_entry(|e| {
        e.depth() == 0 || {
            let name = e.file_name().to_string_lossy();
            !hidden(e) && if e.file_type().is_dir() { !skip_dir(&name) } else { !name.starts_with('.') }
        }
    });
    walk.filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && is_photo(&e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let rel = e.path().strip_prefix(folder).ok()?;
            let path =
                rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            Some(Entry { path, size: meta.len(), mtime })
        })
        .collect()
}

/// Folder by folder (case-insensitive), the photos of a folder before its subfolders'.
fn folder_order(a: &str, b: &str) -> Ordering {
    let (da, fa) = a.rsplit_once('/').unwrap_or(("", a));
    let (db, fb) = b.rsplit_once('/').unwrap_or(("", b));
    da.to_lowercase().cmp(&db.to_lowercase()).then_with(|| fa.to_lowercase().cmp(&fb.to_lowercase()))
}

/// Where the slideshow got to on this PC, per folder.
pub struct State {
    conn: Connection,
    folder: String,
}

impl State {
    pub fn open(path: &Path, folder: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS shown(folder TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY(folder, path));
             CREATE TABLE IF NOT EXISTS last(folder TEXT NOT NULL, ord INTEGER NOT NULL, path TEXT NOT NULL,
               PRIMARY KEY(folder, ord));",
        )?;
        Ok(Self { conn, folder: folder.to_string_lossy().to_lowercase() })
    }

    fn shown(&self) -> HashSet<String> {
        let Ok(mut stmt) = self.conn.prepare("SELECT path FROM shown WHERE folder = ?1") else {
            return HashSet::new();
        };
        stmt.query_map([&self.folder], |r| r.get(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    fn mark_shown(&self, path: &str) {
        let _ = self
            .conn
            .execute("INSERT OR IGNORE INTO shown(folder, path) VALUES(?1, ?2)", params![self.folder, path]);
    }

    fn clear_shown(&self) {
        let _ = self.conn.execute("DELETE FROM shown WHERE folder = ?1", [&self.folder]);
    }

    fn last(&self, order: Order) -> Option<String> {
        self.conn
            .query_row(
                "SELECT path FROM last WHERE folder = ?1 AND ord = ?2",
                params![self.folder, order.index()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    fn set_last(&self, order: Order, path: &str) {
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO last(folder, ord, path) VALUES(?1, ?2, ?3)",
            params![self.folder, order.index(), path],
        );
    }
}

/// SplitMix64: plenty random for picking photos, with no crate needed.
pub struct Rng(u64);

impl Rng {
    pub fn from_time() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Self(nanos ^ (std::process::id() as u64).rotate_left(32))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// In `0..n` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// In `0.0..1.0`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

pub struct Playlist {
    entries: Vec<Entry>,
    order: Order,
    state: Option<State>,
    rng: Rng,
    /// Shuffled: the photos not shown yet in this round.
    unshown: Vec<usize>,
    /// In order: the next one.
    next: usize,
}

impl Playlist {
    /// `dates`: when photos were taken, where known (else their modified time is used).
    pub fn new(
        mut entries: Vec<Entry>,
        order: Order,
        dates: &HashMap<String, i64>,
        state: Option<State>,
        rng: Rng,
    ) -> Self {
        let mut next = 0;
        let mut unshown = Vec::new();
        match order {
            Order::Shuffle => {
                let shown = state.as_ref().map(State::shown).unwrap_or_default();
                unshown = (0..entries.len()).filter(|&i| !shown.contains(&entries[i].path)).collect();
            }
            Order::ByDate | Order::ByFolder => {
                if order == Order::ByDate {
                    let when = |e: &Entry| dates.get(&e.path).copied().unwrap_or(e.mtime);
                    entries
                        .sort_by(|a, b| when(a).cmp(&when(b)).then_with(|| folder_order(&a.path, &b.path)));
                } else {
                    entries.sort_by(|a, b| folder_order(&a.path, &b.path));
                }
                if let Some(last) = state.as_ref().and_then(|s| s.last(order))
                    && let Some(i) = entries.iter().position(|e| e.path == last)
                {
                    next = i + 1;
                }
            }
        }
        Self { entries, order, state, rng, unshown, next }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The next photo, remembered as shown.
    pub fn pick(&mut self) -> Option<Entry> {
        if self.entries.is_empty() {
            return None;
        }
        let entry = match self.order {
            Order::Shuffle => {
                if self.unshown.is_empty() {
                    // Every photo has had its turn: a new round.
                    if let Some(s) = &self.state {
                        s.clear_shown();
                    }
                    self.unshown = (0..self.entries.len()).collect();
                }
                let pick = self.rng.below(self.unshown.len());
                let entry = self.entries[self.unshown.swap_remove(pick)].clone();
                if let Some(s) = &self.state {
                    s.mark_shown(&entry.path);
                }
                entry
            }
            Order::ByDate | Order::ByFolder => {
                let entry = self.entries[self.next % self.entries.len()].clone();
                self.next = (self.next % self.entries.len()) + 1;
                if let Some(s) = &self.state {
                    s.set_last(self.order, &entry.path);
                }
                entry
            }
        };
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, mtime: i64) -> Entry {
        Entry { path: path.into(), size: 1, mtime }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pss-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_photos_but_not_leftovers() {
        let dir = temp("scan");
        for f in [
            "a.JPG",
            "trip/b.heic",
            "trip/day 2/c.png",
            "trip/notes.txt",
            "trip/._c.png",
            ".hidden/d.jpg",
            "@eaDir/e.jpg",
            "#recycle/f.jpg",
            "video.mp4",
        ] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let mut found: Vec<String> = scan(&dir).into_iter().map(|e| e.path).collect();
        found.sort();
        let mut want = vec!["a.JPG", "trip/day 2/c.png"];
        if cfg!(feature = "heif") {
            want.insert(1, "trip/b.heic");
        }
        assert_eq!(found, want);
        assert!(scan(&dir.join("missing")).is_empty());
    }

    #[test]
    fn shuffle_shows_everything_once_per_round_across_runs() {
        let dir = temp("shuffle");
        let entries: Vec<Entry> = (0..10).map(|i| entry(&format!("p{i}.jpg"), i)).collect();
        let state = || State::open(&dir.join("state.sqlite"), Path::new("D:\\Photos")).ok();
        let mut seen = HashSet::new();
        // First run: four photos, then the screensaver closes.
        let mut list = Playlist::new(entries.clone(), Order::Shuffle, &HashMap::new(), state(), Rng(1));
        for _ in 0..4 {
            assert!(seen.insert(list.pick().unwrap().path));
        }
        // Next run carries on with the six not shown yet.
        let mut list = Playlist::new(entries.clone(), Order::Shuffle, &HashMap::new(), state(), Rng(2));
        for _ in 0..6 {
            assert!(seen.insert(list.pick().unwrap().path), "a repeat before every photo was shown");
        }
        assert_eq!(seen.len(), 10);
        // Then a new round.
        assert!(list.pick().is_some());
    }

    #[test]
    fn in_order_picks_up_where_it_left_off() {
        let dir = temp("ordered");
        let state = || State::open(&dir.join("state.sqlite"), Path::new("D:\\Photos")).ok();
        let entries =
            vec![entry("b/2.jpg", 30), entry("a.jpg", 20), entry("b/1.jpg", 10), entry("B/0.jpg", 40)];
        let names = |list: &mut Playlist, n| (0..n).map(|_| list.pick().unwrap().path).collect::<Vec<_>>();

        let mut by_folder = Playlist::new(entries.clone(), Order::ByFolder, &HashMap::new(), state(), Rng(1));
        assert_eq!(names(&mut by_folder, 2), ["a.jpg", "B/0.jpg"]);
        let mut by_folder = Playlist::new(entries.clone(), Order::ByFolder, &HashMap::new(), state(), Rng(1));
        assert_eq!(names(&mut by_folder, 3), ["b/1.jpg", "b/2.jpg", "a.jpg"]);

        // Taken dates beat modified times where known.
        let dates = HashMap::from([("B/0.jpg".to_owned(), 5)]);
        let mut by_date = Playlist::new(entries, Order::ByDate, &dates, state(), Rng(1));
        assert_eq!(names(&mut by_date, 5), ["B/0.jpg", "b/1.jpg", "a.jpg", "b/2.jpg", "B/0.jpg"]);
    }

    #[test]
    fn works_without_state() {
        let mut list = Playlist::new(vec![entry("a.jpg", 1)], Order::Shuffle, &HashMap::new(), None, Rng(3));
        assert_eq!(list.pick().unwrap().path, "a.jpg");
        assert_eq!(list.pick().unwrap().path, "a.jpg");
        assert!(Playlist::new(Vec::new(), Order::ByDate, &HashMap::new(), None, Rng(3)).pick().is_none());
    }
}
