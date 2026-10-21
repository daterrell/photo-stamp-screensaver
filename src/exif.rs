//! What the caption needs from a photo's metadata: when (camera clock, its UTC offset, the GPS
//! fix's time), where (GPS), which way up, and whether photo-stamp already burned a caption in.
//!
//! Read with `kamadak-exif`, which finds the EXIF in JPEG, TIFF, PNG, WebP and HEIF files, rather
//! than exiftool (too slow to start per photo).

use std::io::Cursor;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use exif::{Exif, In, Reader, Tag, Value};

use crate::places::GpsPoint;

/// photo-stamp's marker: the XMP keyword it adds to every photo it stamps.
const STAMPED_MARKER: &[u8] = b"<rdf:li>photo-stamp</rdf:li>";

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// Camera clock reading when the photo was taken (no zone attached).
    pub taken: Option<NaiveDateTime>,
    /// The camera clock's UTC offset, when it recorded one (`OffsetTimeOriginal`).
    pub offset: Option<FixedOffset>,
    /// Satellite time of the GPS fix, always UTC.
    pub gps_time: Option<DateTime<Utc>>,
    pub gps: Option<GpsPoint>,
    /// EXIF orientation (1..=8); 1 when absent.
    pub orientation: u8,
    /// photo-stamp's caption is already in the pixels.
    pub stamped: bool,
}

impl Default for Info {
    fn default() -> Self {
        Self { taken: None, offset: None, gps_time: None, gps: None, orientation: 1, stamped: false }
    }
}

impl Info {
    /// Reads a photo's metadata from its file. Without EXIF, it's all unknown.
    pub fn from_file_bytes(bytes: &[u8]) -> Self {
        let mut info = Reader::new()
            .read_from_container(&mut Cursor::new(bytes))
            .map(|e| from_exif(&e))
            .unwrap_or_default();
        info.stamped = has_marker(bytes);
        info
    }
}

/// Whether photo-stamp's XMP keyword is in these bytes.
pub fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(STAMPED_MARKER.len()).any(|w| w == STAMPED_MARKER)
}

fn ascii(exif: &Exif, tag: Tag) -> Option<String> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Ascii(parts) => {
            let s = String::from_utf8_lossy(parts.first()?);
            let s = s.trim_end_matches('\0').trim();
            (!s.is_empty()).then(|| s.to_owned())
        }
        _ => None,
    }
}

fn rationals(exif: &Exif, tag: Tag) -> Option<Vec<f64>> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(v) if v.iter().all(|r| r.denom != 0) => Some(v.iter().map(|r| r.to_f64()).collect()),
        _ => None,
    }
}

fn from_exif(exif: &Exif) -> Info {
    let mut info = Info::default();
    if let Some(o) = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
    {
        info.orientation = o as u8;
    }
    // DateTimeOriginal with OffsetTimeOriginal, else DateTimeDigitized with its offset.
    for (date, offset) in
        [(Tag::DateTimeOriginal, Tag::OffsetTimeOriginal), (Tag::DateTimeDigitized, Tag::OffsetTimeDigitized)]
    {
        if let Some(taken) = ascii(exif, date).and_then(|s| parse_date_time(&s)) {
            info.taken = Some(taken);
            info.offset = ascii(exif, offset).and_then(|s| s.parse().ok());
            break;
        }
    }
    let coord = |tag, ref_tag, negative: char| {
        let dms = rationals(exif, tag).filter(|v| v.len() == 3)?;
        let value = dms[0] + dms[1] / 60.0 + dms[2] / 3600.0;
        Some(if ascii(exif, ref_tag).unwrap_or_default().starts_with(negative) { -value } else { value })
    };
    if let (Some(lat), Some(lon)) = (
        coord(Tag::GPSLatitude, Tag::GPSLatitudeRef, 'S'),
        coord(Tag::GPSLongitude, Tag::GPSLongitudeRef, 'W'),
    ) {
        // 0, 0 is a camera without a fix.
        info.gps = Some(GpsPoint { lat, lon }).filter(|g| g.is_valid() && !(g.lat == 0.0 && g.lon == 0.0));
    }
    let date = ascii(exif, Tag::GPSDateStamp).and_then(|s| NaiveDate::parse_from_str(&s, "%Y:%m:%d").ok());
    let time = rationals(exif, Tag::GPSTimeStamp)
        .filter(|v| v.len() == 3)
        .and_then(|hms| NaiveTime::from_hms_opt(hms[0] as u32, hms[1] as u32, hms[2].floor() as u32));
    if let (Some(d), Some(t)) = (date, time) {
        info.gps_time = Some(d.and_time(t).and_utc());
    }
    info
}

/// "2024:06:12 14:32:05"; blank ("0000:00:00 00:00:00", a clock never set) is none.
fn parse_date_time(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.get(..19)?, "%Y:%m:%d %H:%M:%S").ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const ASCII: u16 = 2;
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    const RATIONAL: u16 = 5;

    type Entry = (u16, u16, u32, Vec<u8>);

    /// Builds a little-endian TIFF block from IFD0, EXIF and GPS entries: (tag, type, count, value
    /// bytes).
    pub fn tiff(ifd0: &[Entry], exif: &[Entry], gps: &[Entry]) -> Vec<u8> {
        // Layout: header, IFD0, EXIF IFD, GPS IFD, then the values that don't fit in 4 bytes.
        let ifd_len = |n: usize| 2 + n * 12 + 4;
        let mut ifd0 = ifd0.to_vec();
        if !exif.is_empty() {
            ifd0.push((0x8769, LONG, 1, vec![0; 4]));
        }
        if !gps.is_empty() {
            ifd0.push((0x8825, LONG, 1, vec![0; 4]));
        }
        let ifd0_at = 8;
        let exif_at = ifd0_at + ifd_len(ifd0.len());
        let gps_at = exif_at + if exif.is_empty() { 0 } else { ifd_len(exif.len()) };
        let mut extra_at = gps_at + if gps.is_empty() { 0 } else { ifd_len(gps.len()) };
        for e in ifd0.iter_mut() {
            match e.0 {
                0x8769 => e.3 = (exif_at as u32).to_le_bytes().to_vec(),
                0x8825 => e.3 = (gps_at as u32).to_le_bytes().to_vec(),
                _ => {}
            }
        }
        let mut out = b"II*\0".to_vec();
        out.extend((ifd0_at as u32).to_le_bytes());
        let mut extra: Vec<u8> = Vec::new();
        for ifd in [&ifd0[..], exif, gps] {
            if ifd.is_empty() {
                continue;
            }
            out.extend((ifd.len() as u16).to_le_bytes());
            for (tag, kind, count, value) in ifd {
                out.extend(tag.to_le_bytes());
                out.extend(kind.to_le_bytes());
                out.extend(count.to_le_bytes());
                if value.len() <= 4 {
                    let mut field = value.clone();
                    field.resize(4, 0);
                    out.extend(field);
                } else {
                    out.extend((extra_at as u32).to_le_bytes());
                    extra.extend(value);
                    extra_at += value.len();
                }
            }
            out.extend(0u32.to_le_bytes());
        }
        out.extend(extra);
        out
    }

    fn ascii(tag: u16, s: &str) -> Entry {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        (tag, ASCII, v.len() as u32, v)
    }

    fn rationals(tag: u16, values: &[(u32, u32)]) -> Entry {
        let v = values.iter().flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes())).collect();
        (tag, RATIONAL, values.len() as u32, v)
    }

    /// A phone photo from Pebble Beach: local time with offset, GPS, rotated.
    pub fn sample_tiff() -> Vec<u8> {
        tiff(
            &[(0x0112, SHORT, 1, 6u16.to_le_bytes().to_vec())],
            &[ascii(0x9003, "2025:06:03 15:10:42"), ascii(0x9011, "-07:00")],
            &[
                ascii(1, "N"),
                rationals(2, &[(36, 1), (33, 1), (5566, 100)]),
                ascii(3, "W"),
                rationals(4, &[(121, 1), (56, 1), (4300, 100)]),
                rationals(7, &[(22, 1), (10, 1), (42, 1)]),
                ascii(29, "2025:06:03"),
            ],
        )
    }

    /// A JPEG holding only `tiff` as its EXIF.
    pub fn jpeg_with(tiff: &[u8]) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend(((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend(b"Exif\0\0");
        jpeg.extend(tiff);
        jpeg.extend([0xff, 0xd9]);
        jpeg
    }

    fn date(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, s).unwrap()
    }

    #[test]
    fn reads_the_caption_facts() {
        for bytes in [sample_tiff(), jpeg_with(&sample_tiff())] {
            let info = Info::from_file_bytes(&bytes);
            assert_eq!(info.taken, Some(date(2025, 6, 3, 15, 10, 42)));
            assert_eq!(info.offset, FixedOffset::west_opt(7 * 3600));
            assert_eq!(info.gps_time, Some(date(2025, 6, 3, 22, 10, 42).and_utc()));
            let gps = info.gps.unwrap();
            assert!((gps.lat - 36.565461).abs() < 1e-5 && (gps.lon + 121.945278).abs() < 1e-5, "{gps:?}");
            assert_eq!(info.orientation, 6);
        }
    }

    #[test]
    fn blank_dates_zero_gps_and_no_exif() {
        let blank = tiff(&[], &[ascii(0x9003, "0000:00:00 00:00:00")], &[]);
        assert_eq!(Info::from_file_bytes(&blank).taken, None);
        let zero = tiff(
            &[],
            &[],
            &[rationals(2, &[(0, 1), (0, 1), (0, 1)]), rationals(4, &[(0, 1), (0, 1), (0, 1)])],
        );
        assert_eq!(Info::from_file_bytes(&zero).gps, None);
        assert_eq!(Info::from_file_bytes(b"GIF89a"), Info::default());
        assert_eq!(Info::from_file_bytes(&[]), Info::default());
    }

    #[test]
    fn recognizes_photo_stamps_marker() {
        let xmp =
            b"<x:xmpmeta><dc:subject><rdf:Bag><rdf:li>holiday</rdf:li><rdf:li>photo-stamp</rdf:li></rdf:Bag>";
        assert!(Info::from_file_bytes(xmp).stamped);
        assert!(!Info::from_file_bytes(b"<rdf:li>photo-stamped-not</rdf:li>").stamped);
    }
}
