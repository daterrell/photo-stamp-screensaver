//! Which photos the caption editor lists, and the top folder each belongs to: the screensaver's
//! own folder when a photo is in it, else the nearest folder above it with a screensaver cache
//! (another PC's photo folder), else none (an edit can then only be kept in the photo).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::cache::{self, Cache};
use crate::playlist;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub file: PathBuf,
    /// The top folder: the cache's key for the photo is `rel`, relative to it.
    pub root: PathBuf,
    pub rel: String,
    /// The top folder has (or gets) a screensaver cache.
    pub cached: bool,
    /// Its caption was edited for the screensaver.
    pub edited: bool,
}

/// Windows paths ignore case.
fn same(a: Component, b: Component) -> bool {
    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
}

/// `file` relative to `root`, with `/`, the way the screensaver names it; `None` if it isn't in it.
pub fn relative(file: &Path, root: &Path) -> Option<String> {
    let mut rest = file.components();
    for c in root.components() {
        if !same(rest.next()?, c) {
            return None;
        }
    }
    let rel: Vec<_> = rest.map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!rel.is_empty()).then(|| rel.join("/"))
}

/// The top folder for something at `path`, and whether it keeps a cache.
pub fn root_for(path: &Path, screensaver_folder: &str) -> (PathBuf, bool) {
    if !screensaver_folder.is_empty() {
        let folder = Path::new(screensaver_folder);
        if path.components().count() > folder.components().count() && relative(path, folder).is_some() {
            return (folder.to_owned(), true);
        }
    }
    let parent = path.parent().unwrap_or(path);
    match parent.ancestors().find(|a| a.join(cache::FILE_NAME).is_file()) {
        Some(root) => (root.to_owned(), true),
        None => (parent.to_owned(), false),
    }
}

/// Every photo in `folder` and its subfolders, and the photos among `paths` (files or folders):
/// dropped on the editor, or given on its command line.
pub fn list(paths: &[PathBuf], screensaver_folder: &str, data_dir: &Path) -> Vec<Photo> {
    let mut photos = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let files: Vec<PathBuf> = if path.is_dir() {
            let mut entries = playlist::scan(path);
            entries.sort_by(|a, b| playlist::folder_order(&a.path, &b.path));
            entries.into_iter().map(|e| e.path.split('/').fold(path.clone(), |p, c| p.join(c))).collect()
        } else if playlist::is_photo(&path.file_name().unwrap_or_default().to_string_lossy()) {
            vec![path.clone()]
        } else {
            continue;
        };
        for file in files {
            if !seen.insert(file.to_string_lossy().to_lowercase()) {
                continue;
            }
            let (root, cached) = root_for(&file, screensaver_folder);
            let Some(rel) = relative(&file, &root) else { continue };
            photos.push(Photo { file, root, rel, cached, edited: false });
        }
    }
    mark_edited(&mut photos, data_dir);
    photos
}

/// Notes which photos' captions were edited for the screensaver, from each top folder's cache.
fn mark_edited(photos: &mut [Photo], data_dir: &Path) {
    let mut roots: Vec<PathBuf> = photos.iter().filter(|p| p.cached).map(|p| p.root.clone()).collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        let Some(edited) = Cache::open(&root, data_dir).map(|c| c.edited()) else { continue };
        for p in photos.iter_mut().filter(|p| p.root == root) {
            p.edited = edited.contains(&p.rel);
        }
    }
}

/// The caption lines typed in the editor's box: one a row, blank rows left out.
pub fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_photos_top_folder() {
        let dir = std::env::temp_dir().join(format!("pss-test-photos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mine = dir.join("Photos");
        let other = dir.join("Other");
        for d in [mine.join("2024/Trip"), other.join("x/y"), dir.join("Loose")] {
            std::fs::create_dir_all(d).unwrap();
        }
        for f in
            ["Photos/2024/Trip/b.jpg", "Photos/2024/a.JPG", "Other/x/y/c.heic", "Loose/d.png", "Loose/e.txt"]
        {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        std::fs::write(other.join(cache::FILE_NAME), b"").unwrap();
        let folder = mine.to_string_lossy().into_owned();

        let photos = list(
            &[mine.clone(), other.join("x/y/c.heic"), dir.join("Loose"), mine.join("2024/a.JPG")],
            &folder,
            &dir.join("data"),
        );
        let got: Vec<_> = photos.iter().map(|p| (p.root.clone(), p.rel.as_str(), p.cached)).collect();
        // c.heic counts only with libheif.
        let mut want = vec![(mine.clone(), "2024/a.JPG", true), (mine.clone(), "2024/Trip/b.jpg", true)];
        if cfg!(feature = "heif") {
            want.push((other.clone(), "x/y/c.heic", true));
        }
        want.push((dir.join("Loose"), "d.png", false));
        assert_eq!(got, want);
        assert_eq!(relative(Path::new("/A/b/c.jpg"), Path::new("/a/B")), Some("c.jpg".into()));
        assert_eq!(relative(Path::new("/a/c.jpg"), Path::new("/a/b")), None);
    }

    #[test]
    fn marks_edited_photos() {
        let dir = std::env::temp_dir().join(format!("pss-test-photos-edited-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["a.jpg", "b.jpg"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let cache = Cache::open(&dir, &dir.join("data")).unwrap();
        cache.put_edit("b.jpg", Some(&["Home".into()])).unwrap();
        drop(cache);
        let photos = list(std::slice::from_ref(&dir), &dir.to_string_lossy(), &dir.join("data"));
        assert_eq!(photos.iter().map(|p| p.edited).collect::<Vec<_>>(), [false, true]);
    }

    #[test]
    fn typed_lines() {
        assert_eq!(
            lines("  Grandma's garden \r\n\r\n Sun 3 Aug 1997\r\n"),
            ["Grandma's garden", "Sun 3 Aug 1997"]
        );
        assert!(lines(" \n ").is_empty());
    }
}
