//! Keeping the screensaver and its towns database up to date through the place server, as
//! photo-stamp does: the server relays the builds CI publishes (`/v1/client/latest?app=screensaver`).
//!
//! It runs as SYSTEM from a scheduled task (the `.scr` is in System32), with a server address and
//! key that only an administrator can set (setup, or the settings dialog's elevated step), so
//! nobody without admin rights can point it at another server. Downloads are checked against the
//! SHA-256 the server gives, and only newer versions are installed.

use std::io;
use std::path::{Path, PathBuf};

use photo_stamp_proto::version::Version;
use sha2::{Digest, Sha256};

use crate::places::Towns;
use crate::server::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// Installed this version; it runs from the next start.
    Updated(String),
    /// The channel has no builds yet.
    NothingPublished,
}

/// Whether `latest` is a newer version than `current` (unparseable versions never are).
pub fn newer(latest: &str, current: &str) -> bool {
    match (Version::parse(latest), Version::parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Puts `bytes` in place of `target`. A running program can't be overwritten on Windows but can be
/// renamed, so the old file steps aside as `<target>.old` (removed next time, once nothing runs
/// it) and is put back if the new one can't take its place.
pub fn install(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let new = with_suffix(target, ".new");
    let old = with_suffix(target, ".old");
    std::fs::write(&new, bytes)?;
    let _ = std::fs::remove_file(&old);
    let had_old = target.exists();
    if had_old && let Err(e) = std::fs::rename(target, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&new, target) {
        if had_old {
            let _ = std::fs::rename(&old, target);
        }
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    Ok(())
}

/// Removes what earlier updates left behind (`<target>.old`), if nothing holds it any more.
pub fn tidy(target: &Path) {
    let _ = std::fs::remove_file(with_suffix(target, ".old"));
    let _ = std::fs::remove_file(with_suffix(target, ".new"));
}

/// Updates the program at `target` (running as `current`) to the newest build on `channel`.
pub fn update_program(
    client: &Client,
    channel: &str,
    current: &str,
    target: &Path,
) -> Result<Outcome, String> {
    let Some(latest) = client.latest(channel).map_err(|e| e.to_string())? else {
        return Ok(Outcome::NothingPublished);
    };
    // A server from before screensaver updates ignores `app` and offers photo-stamp's own builds.
    if !latest.path.contains("app=screensaver") {
        return Err(
            "the place server doesn't offer screensaver updates yet (it needs a newer photo-stamp-server)"
                .into(),
        );
    }
    if !newer(&latest.version, current) {
        return Ok(Outcome::UpToDate);
    }
    let bytes = client.get(&latest.path).map_err(|e| format!("downloading {}: {e}", latest.version))?;
    let sha = sha256_hex(&bytes);
    if !sha.eq_ignore_ascii_case(&latest.sha256) {
        return Err(format!(
            "the download of {} is damaged (SHA-256 {sha}, expected {})",
            latest.version, latest.sha256
        ));
    }
    if !bytes.starts_with(b"MZ") {
        return Err(format!("the download of {} isn't a Windows program", latest.version));
    }
    install(target, &bytes).map_err(|e| format!("installing {}: {e}", latest.version))?;
    Ok(Outcome::Updated(latest.version))
}

/// Replaces the towns database at `path` when the server's is newer. `Some(built)`: updated to the
/// one built then.
pub fn update_towns(client: &Client, path: &Path) -> Result<Option<String>, String> {
    let me = client.me().map_err(|e| e.to_string())?;
    let ours = Towns::open(path).ok().and_then(|t| t.built());
    let Some(theirs) = me.places_built.filter(|t| ours.as_ref().is_none_or(|o| t > o)) else {
        return Ok(None);
    };
    let bytes = client.get("/v1/towns.sqlite").map_err(|e| format!("downloading the towns database: {e}"))?;
    // Check it opens before it replaces a good one.
    let check = with_suffix(path, ".check");
    std::fs::write(&check, &bytes).map_err(|e| e.to_string())?;
    let ok = Towns::open(&check).map(|t| t.built());
    let _ = std::fs::remove_file(&check);
    let built = ok.map_err(|e| format!("the downloaded towns database doesn't open: {e:#}"))?;
    // A screensaver showing photos right now holds it open: then it's for next time.
    install(path, &bytes).map_err(|e| format!("replacing the towns database: {e}"))?;
    Ok(Some(built.unwrap_or(theirs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::fake_server;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pss-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_order() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.1.1-dev.3+gabc", "0.1.0"));
        assert!(newer("0.1.1", "0.1.1-dev.3+gabc"), "the release after its betas");
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"), "never back");
        assert!(!newer("nonsense", "0.1.0"));
    }

    #[test]
    fn installs_beside_the_old_one() {
        let dir = temp("install");
        let target = dir.join("app.scr");
        std::fs::write(&target, b"old").unwrap();
        install(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("app.scr.old")).unwrap(), b"old");
        tidy(&target);
        assert!(!dir.join("app.scr.old").exists());
        // Nothing there yet is fine too.
        install(&dir.join("fresh.scr"), b"x").unwrap();
        // Can't write there: nothing changes.
        assert!(install(&dir.join("missing/app.scr"), b"x").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    /// A server with build 0.2.0 (`body`, announced with `sha`) and towns built `towns_built`.
    fn update_server(body: &'static [u8], sha: String, towns_built: &str) -> String {
        server_announcing(body, sha, towns_built, "/v1/client/download/0.2.0?app=screensaver")
    }

    /// The same, announcing the build at `path`.
    fn server_announcing(body: &'static [u8], sha: String, towns_built: &str, path: &'static str) -> String {
        let towns_built = towns_built.to_owned();
        let towns = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/places.sqlite"))
            .unwrap();
        fake_server("k", move |target, authorized, _| {
            if !authorized {
                return (401, Vec::new());
            }
            match target {
                "GET /v1/client/latest?app=screensaver&channel=stable" => {
                    let reply = format!(r#"{{"version":"0.2.0","sha256":"{sha}","path":"{path}"}}"#);
                    (200, reply.into_bytes())
                }
                p if p.starts_with("GET /v1/client/download/0.2.0") => (200, body.to_vec()),
                "GET /v1/me" => {
                    let me = format!(
                        r#"{{"user":"dave","device":"den","history":true,"max_wait_hours":72,"rev":1,"data_version":"d","server_version":"1","places_built":"{towns_built}"}}"#
                    );
                    (200, me.into_bytes())
                }
                "GET /v1/towns.sqlite" => (200, towns.clone()),
                _ => (204, Vec::new()),
            }
        })
    }

    #[test]
    fn updates_to_newer_builds_only() {
        let dir = temp("update");
        let target = dir.join("photo-stamp-screensaver.scr");
        std::fs::write(&target, b"0.1.0").unwrap();
        let url = update_server(b"MZ build 0.2.0", sha256_hex(b"MZ build 0.2.0"), "2000-01-01");
        let client = Client::new(&url, "k", Duration::from_secs(5));
        assert_eq!(update_program(&client, "stable", "0.1.0", &target), Ok(Outcome::Updated("0.2.0".into())));
        assert_eq!(std::fs::read(&target).unwrap(), b"MZ build 0.2.0");
        assert_eq!(update_program(&client, "stable", "0.2.0", &target), Ok(Outcome::UpToDate));
        assert_eq!(update_program(&client, "beta", "0.2.0", &target), Ok(Outcome::NothingPublished));
        assert!(
            update_program(&Client::new(&url, "wrong", Duration::from_secs(5)), "stable", "0.1.0", &target)
                .is_err()
        );
    }

    #[test]
    fn refuses_damaged_downloads() {
        let dir = temp("damaged");
        let target = dir.join("photo-stamp-screensaver.scr");
        std::fs::write(&target, b"0.1.0").unwrap();
        let url = update_server(b"MZ build 0.2.0", sha256_hex(b"something else"), "2000-01-01");
        let client = Client::new(&url, "k", Duration::from_secs(5));
        let err = update_program(&client, "stable", "0.1.0", &target).unwrap_err();
        assert!(err.contains("damaged"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"0.1.0");
        // Something that isn't a program, even if the server vouches for it.
        let url = update_server(b"PK zip", sha256_hex(b"PK zip"), "2000-01-01");
        let client = Client::new(&url, "k", Duration::from_secs(5));
        let err = update_program(&client, "stable", "0.1.0", &target).unwrap_err();
        assert!(err.contains("isn't a Windows program"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"0.1.0");
    }

    #[test]
    fn an_older_server_offering_photo_stamps_builds_is_ignored() {
        let dir = temp("old-server");
        let target = dir.join("photo-stamp-screensaver.scr");
        std::fs::write(&target, b"0.1.0").unwrap();
        let body = b"MZ photo-stamp";
        let url = server_announcing(body, sha256_hex(body), "2000-01-01", "/v1/client/download/0.2.0");
        let client = Client::new(&url, "k", Duration::from_secs(5));
        let err = update_program(&client, "stable", "0.1.0", &target).unwrap_err();
        assert!(err.contains("doesn't offer screensaver updates"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"0.1.0");
    }

    #[test]
    fn towns_only_when_the_servers_are_newer() {
        let dir = temp("towns");
        let path = dir.join("towns.sqlite");
        // None yet: take the server's.
        let client =
            Client::new(&update_server(b"", String::new(), "2099-01-01"), "k", Duration::from_secs(5));
        let built = update_towns(&client, &path).unwrap().unwrap();
        assert!(Towns::open(&path).is_ok());
        // Ours is as new as theirs now (the fixture's date): nothing to do.
        let same = Client::new(&update_server(b"", String::new(), &built), "k", Duration::from_secs(5));
        assert_eq!(update_towns(&same, &path).unwrap(), None);
    }
}
