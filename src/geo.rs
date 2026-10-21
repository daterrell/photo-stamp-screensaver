//! Naming where a photo was taken, for its caption: the place server's answer, remembered in the
//! shared cache; while the server can't be reached, an older answer or the nearest town.
//!
//! Lookups happen while the previous photo is on screen, so a slow server costs nothing visible,
//! and after a failure the server isn't asked again for a while, so an offline PC doesn't wait on
//! every photo.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cache::{Answer, Cache};
use crate::places::{GpsPoint, Place, Towns};
use crate::server::{CheckError, Client};

/// After a failed request, don't ask the server again for this long.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);
/// Learn the server's data version this often, so answers from older data are noticed.
const ME_EVERY: Duration = Duration::from_secs(30 * 60);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Default)]
struct Net {
    retry_at: Option<Instant>,
    /// The server's (data_version, rev), and when it said so.
    current: Option<(String, u64, Instant)>,
}

pub struct Geo {
    client: Option<Client>,
    cache: Option<Arc<Cache>>,
    towns: Option<Towns>,
    net: Mutex<Net>,
}

impl Geo {
    pub fn new(client: Option<Client>, cache: Option<Arc<Cache>>, towns: Option<Towns>) -> Self {
        Self { client, cache, towns, net: Mutex::new(Net::default()) }
    }

    /// The place's caption lines' source; `None` when nothing at all is known there.
    pub fn place(&self, at: GpsPoint) -> Option<Place> {
        let cached = self.cache.as_ref().and_then(|c| c.answer(at));
        let Some(client) = &self.client else {
            return cached.and_then(|a| a.place).or_else(|| self.town(at));
        };
        match self.ask(client, at, cached.as_ref()) {
            Ok(place) => place,
            Err(e) => {
                crate::log::line(&format!("place server: {e}"));
                // Any remembered answer beats the nearest town, even from older data.
                cached.and_then(|a| a.place).or_else(|| self.town(at))
            }
        }
    }

    fn town(&self, at: GpsPoint) -> Option<Place> {
        self.towns.as_ref()?.place(at)
    }

    fn ask(
        &self,
        client: &Client,
        at: GpsPoint,
        cached: Option<&Answer>,
    ) -> Result<Option<Place>, CheckError> {
        if self.net.lock().unwrap().retry_at.is_some_and(|t| Instant::now() < t) {
            return Err(CheckError::Unreachable("it failed a moment ago".into()));
        }
        let failed = |e: CheckError| {
            self.net.lock().unwrap().retry_at = Some(Instant::now() + RETRY_AFTER_FAILURE);
            e
        };
        let stale =
            self.net.lock().unwrap().current.as_ref().is_none_or(|(_, _, at)| at.elapsed() >= ME_EVERY);
        if stale {
            let me = client.me().map_err(failed)?;
            self.net.lock().unwrap().current = Some((me.data_version, me.rev, Instant::now()));
        }
        if let Some(cached) = cached {
            let net = self.net.lock().unwrap();
            if net.current.as_ref().is_some_and(|(v, r, _)| *v == cached.data_version && *r == cached.rev) {
                return Ok(cached.place.clone());
            }
        }
        let reply = client.place(at).map_err(failed)?;
        // Incomplete answers are shown but asked again next time.
        if reply.complete
            && let Some(cache) = &self.cache
        {
            let answer =
                Answer { place: reply.place.clone(), data_version: reply.data_version, rev: reply.rev };
            cache.put_answer(at, &answer);
        }
        Ok(reply.place)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::places::tests::fixture_towns;
    use crate::server::tests::{ME, fake_server as server};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A place server that answers every point with `primary`, counting the place requests.
    fn fake_server(primary: &'static str, asked: Arc<AtomicUsize>) -> String {
        server("k", move |target, _, _| {
            if target.starts_with("GET /v1/me") {
                return (200, ME.into());
            }
            asked.fetch_add(1, Ordering::SeqCst);
            let reply = format!(
                r#"{{"place":{{"primary":"{primary}"}},"complete":true,"data_version":"d1","rev":1}}"#
            );
            (200, reply.into_bytes())
        })
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pss-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const PEBBLE: GpsPoint = GpsPoint { lat: 36.565462, lon: -121.945278 };

    #[test]
    fn asks_once_per_spot_then_remembers_for_every_pc() {
        let dir = temp("geo");
        let asked = Arc::new(AtomicUsize::new(0));
        let url = fake_server("Pebble Beach Golf Course", asked.clone());
        let cache = Arc::new(Cache::open(&dir, &dir).unwrap());
        let geo = Geo::new(Some(Client::new(&url, "k", REQUEST_TIMEOUT)), Some(cache.clone()), None);
        assert_eq!(geo.place(PEBBLE).unwrap().primary, "Pebble Beach Golf Course");
        assert_eq!(geo.place(PEBBLE).unwrap().primary, "Pebble Beach Golf Course");
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // Another PC sharing the folder (its own Geo) finds the answer in the cache.
        let other = Geo::new(Some(Client::new(&url, "k", REQUEST_TIMEOUT)), Some(cache), None);
        assert_eq!(other.place(PEBBLE).unwrap().primary, "Pebble Beach Golf Course");
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn offline_uses_remembered_answers_then_towns() {
        let dir = temp("geo-offline");
        let cache = Arc::new(Cache::open(&dir, &dir).unwrap());
        let paris = GpsPoint { lat: 48.8584, lon: 2.2945 };
        cache.put_answer(
            paris,
            &Answer {
                place: Some(Place { primary: "Eiffel Tower".into(), ..Default::default() }),
                data_version: "old".into(),
                rev: 0,
            },
        );
        // Nobody listening there.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let client = Client::new(&format!("http://127.0.0.1:{port}"), "k", REQUEST_TIMEOUT);
        let geo = Geo::new(Some(client), Some(cache), Some(fixture_towns()));
        assert_eq!(geo.place(paris).unwrap().primary, "Eiffel Tower");
        let started = Instant::now();
        assert!(geo.place(PEBBLE).unwrap().primary.starts_with("Carmel"));
        assert!(started.elapsed() < Duration::from_secs(1), "the failed server was asked again");
        // Without a server at all.
        let geo = Geo::new(None, None, Some(fixture_towns()));
        assert!(geo.place(PEBBLE).unwrap().primary.starts_with("Carmel"));
        assert_eq!(Geo::new(None, None, None).place(PEBBLE), None);
    }
}
