//! Place names: what the place server answers, and the small towns database for when it can't be
//! reached. Copied from photo-stamp's `proto` (`lib.rs`, `api.rs`, `places.rs`), only the parts the
//! screensaver needs; keep them in step with it.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GpsPoint {
    pub lat: f64,
    pub lon: f64,
}

impl GpsPoint {
    /// Whether this is a real position on Earth (0, 0 is a camera without a fix).
    pub fn is_valid(&self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && self.lat.abs() <= 90.0
            && self.lon.abs() <= 180.0
            && !(self.lat == 0.0 && self.lon == 0.0)
    }

    /// ~11 m: photos from the same spot share an answer (as photo-stamp keys its caches).
    pub fn cache_key(&self) -> String {
        format!("{:.4},{:.4}", self.lat, self.lon)
    }
}

/// Where a photo was taken, as caption lines.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// Most specific name: the venue we're in, a landmark, or a neighborhood/town.
    pub primary: String,
    /// A famous landmark close by, when `primary` is a venue we're inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<String>,
    /// Neighborhood, town, region, country.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Place {
    pub fn lines(&self) -> Vec<String> {
        let near = self.near.as_ref().map(|n| format!("near {n}"));
        std::iter::once(self.primary.clone()).chain(near).chain(self.context.clone()).collect()
    }
}

/// `POST /v1/place`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaceRequest {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PlaceReply {
    /// `None`: nothing known there (open sea).
    pub place: Option<Place>,
    /// False when part of the lookup failed on the server: usable, but worth asking again later.
    pub complete: bool,
    /// The server's data release: cached answers from another release are asked again.
    pub data_version: String,
    /// Changes when this person's personal places change (cached answers are then asked again).
    pub rev: u64,
}

/// A town with its region and country.
type TownNames = (Name, Option<Name>, Option<Name>);

/// A name in the preferred language plus the local-language name when it differs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Name {
    pub preferred: String,
    pub local: Option<String>,
}

impl Name {
    pub fn new(preferred: Option<&str>, local: Option<&str>) -> Option<Self> {
        let preferred = preferred.or(local)?.trim().to_owned();
        if preferred.is_empty() {
            return None;
        }
        // Skip a local name that only differs in case, spacing or punctuation ("Champ-de-Mars").
        let squash = |s: &str| {
            s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>()
        };
        let local = local
            .map(str::trim)
            .filter(|l| !l.is_empty() && !squash(&preferred).contains(&squash(l)))
            .map(str::to_owned);
        Some(Self { preferred, local })
    }

    /// "Munich (München)", or just "Paris".
    pub fn display(&self) -> String {
        match &self.local {
            Some(l) => format!("{} ({l})", self.preferred),
            None => self.preferred.clone(),
        }
    }
}

/// Like Immich: the nearest populated place within this distance names the town.
const TOWN_RADIUS_KM: f64 = 25.0;
const KM_PER_DEG: f64 = 111.32;
/// Cities at least this big can name the town for photos in their districts.
const BIG_CITY_MIN_POPULATION: u64 = 40_000;

/// How far a city of this size "reaches": 6 km at 40k people, +3 km per tenfold, at most 15 km.
fn city_reach_km(population: u64) -> f64 {
    let ratio = population.max(BIG_CITY_MIN_POPULATION) as f64 / BIG_CITY_MIN_POPULATION as f64;
    (6.0 + 3.0 * ratio.log10()).min(15.0)
}

fn distance_m(a: GpsPoint, b: GpsPoint) -> f64 {
    let (la1, la2) = (a.lat.to_radians(), b.lat.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((b.lon - a.lon).to_radians() / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * h.sqrt().asin()
}

/// A lat/lon box around `at` covering `radius_m` (good enough away from the poles).
fn bbox(at: GpsPoint, radius_m: f64) -> (f64, f64, f64, f64) {
    let dlat = radius_m / 1000.0 / KM_PER_DEG;
    let dlon = dlat / at.lat.to_radians().cos().max(0.01);
    (at.lat - dlat, at.lat + dlat, at.lon - dlon, at.lon + dlon)
}

/// photo-stamp's small towns database (`towns.sqlite`, built monthly by dave/photo-stamp-places),
/// installed beside the uninstaller.
pub struct Towns {
    conn: Mutex<Connection>,
}

impl Towns {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Fail now, not at the first photo, if this isn't the towns database.
        conn.query_row("SELECT count(*) FROM city LIMIT 1", [], |_| Ok(()))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// The place as photo-stamp names it from towns alone: the town (or the big city whose district
    /// it's in), then region and country.
    pub fn place(&self, at: GpsPoint) -> Option<Place> {
        let (town, region, country) = self.town(at).ok().flatten()?;
        let primary = self.big_city(at).ok().flatten().unwrap_or(town);
        // "Moscow, Russia", not "Moscow, Moscow, Russia" where the region is the city.
        let context: Vec<String> = [region, country]
            .into_iter()
            .flatten()
            .filter(|n| !n.preferred.eq_ignore_ascii_case(&primary.preferred))
            .map(|n| n.display())
            .collect();
        Some(Place {
            primary: primary.display(),
            near: None,
            context: (!context.is_empty()).then(|| context.join(", ")),
        })
    }

    /// The nearest town within 25 km, with its region and country.
    fn town(&self, at: GpsPoint) -> rusqlite::Result<Option<TownNames>> {
        let conn = self.conn.lock().unwrap();
        let (lat0, lat1, lon0, lon1) = bbox(at, TOWN_RADIUS_KM * 1000.0);
        let mut stmt = conn.prepare_cached(
            "SELECT c.name, c.local, c.lat, c.lon, c.country, c.region
             FROM city_idx i JOIN city c ON c.id = i.id
             WHERE i.min_lat <= ?2 AND i.max_lat >= ?1 AND i.min_lon <= ?4 AND i.max_lon >= ?3",
        )?;
        let mut best: Option<(f64, Name, String, Option<String>)> = None;
        let rows = stmt.query_map(params![lat0, lat1, lon0, lon1], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in rows {
            let (name, local, lat, lon, country, region) = row?;
            let d = distance_m(at, GpsPoint { lat, lon });
            if d <= TOWN_RADIUS_KM * 1000.0
                && best.as_ref().is_none_or(|b| d < b.0)
                && let Some(n) = Name::new(Some(&name), local.as_deref())
            {
                best = Some((d, n, country, region));
            }
        }
        let Some((_, town, country, region)) = best else { return Ok(None) };
        let region = match region.filter(|r| !r.is_empty()) {
            Some(code) => conn
                .prepare_cached("SELECT name, local FROM region WHERE country = ?1 AND code = ?2")?
                .query_row(params![country, code], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .optional()?
                .and_then(|(n, l)| Name::new(Some(&n), l.as_deref())),
            None => None,
        };
        let country = conn
            .prepare_cached("SELECT name FROM country WHERE code = ?1")?
            .query_row(params![country], |r| r.get::<_, String>(0))
            .optional()?
            // Country names are recognizable enough in English alone.
            .and_then(|n| Name::new(Some(&n), None));
        Ok(Some((town, region, country)))
    }

    /// The biggest city whose reach covers `at`, for naming the town in one of its districts.
    fn big_city(&self, at: GpsPoint) -> rusqlite::Result<Option<Name>> {
        let conn = self.conn.lock().unwrap();
        let (lat0, lat1, lon0, lon1) = bbox(at, city_reach_km(u64::MAX) * 1000.0);
        let mut stmt = conn.prepare_cached(
            "SELECT c.name, c.local, c.lat, c.lon, c.population FROM city_idx i JOIN city c ON c.id = i.id
             WHERE i.min_lat <= ?2 AND i.max_lat >= ?1 AND i.min_lon <= ?4 AND i.max_lon >= ?3 AND c.population >= ?5
             ORDER BY c.population DESC",
        )?;
        let rows = stmt.query_map(params![lat0, lat1, lon0, lon1, BIG_CITY_MIN_POPULATION as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        for row in rows {
            let (name, local, lat, lon, population) = row?;
            if distance_m(at, GpsPoint { lat, lon }) <= city_reach_km(population.max(0) as u64) * 1000.0 {
                return Ok(Name::new(Some(&name), local.as_deref()));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// photo-stamp's fixture: real data around Pebble Beach, Paris and Tokyo.
    pub fn fixture_towns() -> Towns {
        Towns::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/places.sqlite")).unwrap()
    }

    #[test]
    fn names_skip_redundant_local_forms() {
        assert_eq!(
            Name::new(Some("Champ de Mars"), Some("Champ-de-Mars")).unwrap().display(),
            "Champ de Mars"
        );
        assert_eq!(Name::new(Some("Munich"), Some("München")).unwrap().display(), "Munich (München)");
        assert!(Name::new(Some("  "), None).is_none());
    }

    #[test]
    fn place_lines() {
        let p = Place {
            primary: "Pebble Beach Golf Course".into(),
            near: Some("Lone Cypress".into()),
            context: Some("Pebble Beach, California, United States".into()),
        };
        assert_eq!(
            p.lines(),
            ["Pebble Beach Golf Course", "near Lone Cypress", "Pebble Beach, California, United States"]
        );
        assert_eq!(GpsPoint { lat: 36.565462, lon: -121.945278 }.cache_key(), "36.5655,-121.9453");
        assert!(!GpsPoint { lat: 0.0, lon: 0.0 }.is_valid());
    }

    #[test]
    fn towns_name_places() {
        let towns = fixture_towns();
        let pebble = towns.place(GpsPoint { lat: 36.565462, lon: -121.945278 }).unwrap();
        assert!(pebble.primary.starts_with("Carmel"), "{pebble:?}");
        assert_eq!(pebble.context.as_deref(), Some("California, United States"));
        assert!(towns.place(GpsPoint { lat: 48.8584, lon: 2.2945 }).unwrap().primary.starts_with("Paris"));
        // Mid-Pacific: nothing within 25 km.
        assert_eq!(towns.place(GpsPoint { lat: 0.0, lon: -150.0 }), None);
    }
}
