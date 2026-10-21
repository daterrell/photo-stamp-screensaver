//! Place names: photo-stamp's types and `/v1` protocol (its `proto` crate), and naming a place from
//! the small towns database alone, for when the place server can't be reached.

use std::path::Path;

pub use photo_stamp_proto::api::{ClientUpdate, Me, PlaceReply, PlaceRequest};
use photo_stamp_proto::places::PlacesDb;
pub use photo_stamp_proto::{GpsPoint, Place};

/// photo-stamp's towns database (`towns.sqlite`, built monthly by dave/photo-stamp-places),
/// installed beside the uninstaller and kept up to date by the updater.
pub struct Towns(PlacesDb);

impl Towns {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        PlacesDb::open(path).map(Self)
    }

    /// When it was built (YYYY-MM-DD).
    pub fn built(&self) -> Option<String> {
        self.0.built()
    }

    /// The place as photo-stamp names it from towns alone: the town (or the big city whose district
    /// it's in), then region and country.
    pub fn place(&self, at: GpsPoint) -> Option<Place> {
        let town = self.0.town(at).ok().flatten()?;
        let primary = self.0.big_city(at).ok().flatten().unwrap_or(town.town);
        // "Moscow, Russia", not "Moscow, Moscow, Russia" where the region is the city.
        let context: Vec<String> = [town.region, town.country]
            .into_iter()
            .flatten()
            .filter(|n| !n.preferred.eq_ignore_ascii_case(&primary.preferred))
            .map(|n| n.display())
            .collect();
        Some(Place {
            primary: primary.display(),
            near: None,
            context: (!context.is_empty()).then(|| context.join(", ")),
            complete: true,
        })
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
    fn towns_name_places() {
        let towns = fixture_towns();
        let pebble = towns.place(GpsPoint { lat: 36.565462, lon: -121.945278 }).unwrap();
        assert!(pebble.primary.starts_with("Carmel"), "{pebble:?}");
        assert_eq!(pebble.context.as_deref(), Some("California, United States"));
        assert!(towns.place(GpsPoint { lat: 48.8584, lon: 2.2945 }).unwrap().primary.starts_with("Paris"));
        // Mid-Pacific: nothing within 25 km.
        assert_eq!(towns.place(GpsPoint { lat: 0.0, lon: -150.0 }), None);
        assert!(towns.built().is_some());
    }
}
