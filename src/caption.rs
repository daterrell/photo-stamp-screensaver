//! The caption's lines: where (place lines) and when, as photo-stamp words them.
//!
//! The time is shown as it was *where the photo was taken*: camera clocks are often left on home
//! time when traveling, so when the metadata pins down the true instant it's converted into the
//! timezone at the GPS position (from photo-stamp's `localtime.rs`).

use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use crate::exif::Info;
use crate::places::{GpsPoint, Place};

const FORMAT: &str = "%a %-d %b %Y, %H:%M";

/// Place lines, then the time; empty when nothing is known.
pub fn lines(info: &Info, place: Option<&Place>) -> Vec<String> {
    let tz = info.gps.and_then(timezone_at);
    place.map(Place::lines).unwrap_or_default().into_iter().chain(time(info, tz)).collect()
}

/// IANA timezone at a location, from an embedded offline boundary map.
pub fn timezone_at(at: GpsPoint) -> Option<Tz> {
    static FINDER: OnceLock<tzf_rs::DefaultFinder> = OnceLock::new();
    FINDER.get_or_init(tzf_rs::DefaultFinder::new).get_tz_name(at.lon, at.lat).parse().ok()
}

/// The true moment the photo was taken, when the metadata pins it down: the camera's recorded UTC
/// offset, else the GPS fix time.
fn instant(info: &Info) -> Option<DateTime<Utc>> {
    match (info.taken, info.offset) {
        (Some(t), Some(off)) => t.and_local_timezone(off).single().map(|t| t.with_timezone(&Utc)),
        _ => info.gps_time,
    }
}

/// "Wed 12 Jun 2024, 14:32 CEST" when we know both the instant and the place's timezone,
/// otherwise the camera's own clock reading without a zone.
fn time(info: &Info, tz: Option<Tz>) -> Option<String> {
    if let (Some(tz), Some(utc)) = (tz, instant(info)) {
        let local = utc.with_timezone(&tz);
        return Some(format!("{} {}", local.format(FORMAT), zone_label(&local.format("%Z").to_string())));
    }
    info.taken.map(|t| t.format(FORMAT).to_string())
}

/// Zones without a common abbreviation come out as "+07"; make those "UTC+07".
fn zone_label(abbrev: &str) -> String {
    if abbrev.starts_with(['+', '-']) { format!("UTC{abbrev}") } else { abbrev.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    const TOKYO: GpsPoint = GpsPoint { lat: 35.6586, lon: 139.7454 };

    #[test]
    fn camera_left_on_home_time_is_corrected() {
        // A London-set camera (+01:00 in summer) shoots in Tokyo at 22:32 local.
        let info = Info {
            taken: Some(at(2024, 6, 12, 14, 32)),
            offset: FixedOffset::east_opt(3600),
            gps: Some(TOKYO),
            ..Default::default()
        };
        let place = Place {
            primary: "Tokyo Tower".into(),
            context: Some("Minato, Tokyo, Japan".into()),
            ..Default::default()
        };
        assert_eq!(
            lines(&info, Some(&place)),
            ["Tokyo Tower", "Minato, Tokyo, Japan", "Wed 12 Jun 2024, 22:32 JST"]
        );
    }

    #[test]
    fn gps_time_used_without_offset() {
        let info = Info {
            taken: Some(at(2024, 1, 5, 9, 0)),
            gps_time: Some(at(2024, 1, 5, 14, 0).and_utc()),
            gps: Some(GpsPoint { lat: 40.6892, lon: -74.0445 }),
            ..Default::default()
        };
        assert_eq!(lines(&info, None), ["Fri 5 Jan 2024, 09:00 EST"]);
    }

    #[test]
    fn numeric_zone_abbreviations_get_utc_prefix() {
        let info = Info {
            gps_time: Some(at(2024, 3, 1, 3, 0).and_utc()),
            gps: Some(GpsPoint { lat: 10.7769, lon: 106.7009 }),
            ..Default::default()
        };
        assert_eq!(lines(&info, None), ["Fri 1 Mar 2024, 10:00 UTC+07"]);
    }

    #[test]
    fn falls_back_to_camera_clock() {
        let info = Info { taken: Some(at(2021, 8, 15, 21, 10)), ..Default::default() };
        assert_eq!(lines(&info, None), ["Sun 15 Aug 2021, 21:10"]);
        assert!(lines(&Info::default(), None).is_empty());
    }
}
