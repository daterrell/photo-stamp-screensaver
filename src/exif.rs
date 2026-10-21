//! What the caption needs from a photo's metadata: when (camera clock, its UTC offset, the GPS
//! fix's time), where (GPS), which way up, and whether photo-stamp already burned a caption in.
//!
//! A small EXIF reader rather than exiftool (too slow to start per photo) or another crate: it finds
//! the EXIF (TIFF) block in JPEG, PNG, WebP and TIFF files; HEIC's comes from libheif.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::places::GpsPoint;

/// photo-stamp's marker: the XMP keyword it adds to every photo it stamps.
const STAMPED_MARKER: &[u8] = b"<rdf:li>photo-stamp</rdf:li>";

#[derive(Debug, Clone, Default, PartialEq)]
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

impl Info {
    /// Reads a photo's metadata from the start of its file (or all of it), for the formats that
    /// carry EXIF inline. Without EXIF, it's all unknown.
    pub fn from_file_bytes(bytes: &[u8]) -> Self {
        let mut info = tiff_block(bytes).map(parse_tiff).unwrap_or_default();
        info.stamped = has_marker(bytes);
        info
    }
}

/// Whether photo-stamp's XMP keyword is in these bytes.
pub fn has_marker(bytes: &[u8]) -> bool {
    bytes.windows(STAMPED_MARKER.len()).any(|w| w == STAMPED_MARKER)
}

/// The EXIF (TIFF) block of a JPEG, PNG, WebP or TIFF file.
pub fn tiff_block(file: &[u8]) -> Option<&[u8]> {
    if file.starts_with(b"II*\0") || file.starts_with(b"MM\0*") {
        return Some(file);
    }
    if file.starts_with(&[0xff, 0xd8]) {
        return jpeg_exif(file);
    }
    if file.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_exif(file);
    }
    if file.len() >= 12 && &file[..4] == b"RIFF" && &file[8..12] == b"WEBP" {
        return webp_exif(file);
    }
    None
}

fn strip_exif_header(data: &[u8]) -> &[u8] {
    data.strip_prefix(b"Exif\0\0").unwrap_or(data)
}

fn jpeg_exif(file: &[u8]) -> Option<&[u8]> {
    let mut at = 2;
    while at + 4 <= file.len() {
        if file[at] != 0xff {
            return None;
        }
        let marker = file[at + 1];
        // Fill bytes.
        if marker == 0xff {
            at += 1;
            continue;
        }
        // Start of scan: the metadata segments are all before it.
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = u16::from_be_bytes([file[at + 2], file[at + 3]]) as usize;
        let data = file.get(at + 4..at + 2 + len)?;
        if marker == 0xe1 && data.starts_with(b"Exif\0\0") {
            return Some(&data[6..]);
        }
        at += 2 + len;
    }
    None
}

fn png_exif(file: &[u8]) -> Option<&[u8]> {
    let mut at = 8;
    while at + 8 <= file.len() {
        let len = u32::from_be_bytes(file[at..at + 4].try_into().ok()?) as usize;
        let kind = &file[at + 4..at + 8];
        let data = file.get(at + 8..at + 8 + len)?;
        if kind == b"eXIf" {
            return Some(strip_exif_header(data));
        }
        if kind == b"IEND" {
            return None;
        }
        // Length, type, data, CRC.
        at += 12 + len;
    }
    None
}

fn webp_exif(file: &[u8]) -> Option<&[u8]> {
    let mut at = 12;
    while at + 8 <= file.len() {
        let kind = &file[at..at + 4];
        let len = u32::from_le_bytes(file[at + 4..at + 8].try_into().ok()?) as usize;
        let data = file.get(at + 8..at + 8 + len)?;
        if kind == b"EXIF" {
            return Some(strip_exif_header(data));
        }
        // Chunks are padded to an even length.
        at += 8 + len + (len & 1);
    }
    None
}

/// A TIFF structure: byte order, then IFDs of 12-byte entries.
struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    /// Where the entry's 4-byte value/offset field is.
    field: usize,
}

const ASCII: u16 = 2;
const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;

impl<'a> Tiff<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        let little = match data.get(..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        Some(Self { data, little })
    }

    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    fn ifd(&self, offset: usize) -> Vec<Entry> {
        let Some(n) = self.u16(offset) else { return Vec::new() };
        (0..n as usize)
            .map_while(|i| {
                let at = offset + 2 + i * 12;
                Some(Entry {
                    tag: self.u16(at)?,
                    kind: self.u16(at + 2)?,
                    count: self.u32(at + 4)?,
                    field: at + 8,
                })
            })
            .collect()
    }

    /// Where an entry's value is: in the field itself when it fits, else where the field points.
    fn value_at(&self, e: &Entry) -> Option<usize> {
        let size = match e.kind {
            1 | ASCII | 6 | 7 => 1,
            SHORT | 8 => 2,
            LONG | 9 | 11 => 4,
            RATIONAL | 10 | 12 => 8,
            _ => return None,
        } * e.count as usize;
        let at = if size <= 4 { e.field } else { self.u32(e.field)? as usize };
        (at.checked_add(size)? <= self.data.len()).then_some(at)
    }

    fn ascii(&self, e: &Entry) -> Option<String> {
        (e.kind == ASCII).then_some(())?;
        let at = self.value_at(e)?;
        let s = String::from_utf8_lossy(&self.data[at..at + e.count as usize]);
        let s = s.trim_end_matches('\0').trim();
        (!s.is_empty()).then(|| s.to_owned())
    }

    fn uint(&self, e: &Entry) -> Option<u32> {
        match e.kind {
            SHORT => self.u16(self.value_at(e)?).map(u32::from),
            LONG => self.u32(self.value_at(e)?),
            _ => None,
        }
    }

    fn rationals(&self, e: &Entry) -> Option<Vec<f64>> {
        (e.kind == RATIONAL).then_some(())?;
        let start = self.value_at(e)?;
        (0..e.count as usize)
            .map(|i| {
                let (n, d) = (self.u32(start + i * 8)?, self.u32(start + i * 8 + 4)?);
                (d != 0).then(|| n as f64 / d as f64)
            })
            .collect()
    }
}

fn find(entries: &[Entry], tag: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.tag == tag)
}

/// The caption's facts from an EXIF (TIFF) block.
pub fn parse_tiff(data: &[u8]) -> Info {
    let mut info = Info { orientation: 1, ..Default::default() };
    let Some(t) = Tiff::new(data) else { return info };
    let Some(ifd0_at) = t.u32(4) else { return info };
    let ifd0 = t.ifd(ifd0_at as usize);
    if let Some(o) = find(&ifd0, 0x0112).and_then(|e| t.uint(e)).filter(|o| (1..=8).contains(o)) {
        info.orientation = o as u8;
    }
    if let Some(exif_at) = find(&ifd0, 0x8769).and_then(|e| t.uint(e)) {
        let exif = t.ifd(exif_at as usize);
        // DateTimeOriginal with OffsetTimeOriginal, else DateTimeDigitized with its offset.
        for (date_tag, offset_tag) in [(0x9003, 0x9011), (0x9004, 0x9012)] {
            if let Some(taken) =
                find(&exif, date_tag).and_then(|e| t.ascii(e)).and_then(|s| parse_date_time(&s))
            {
                info.taken = Some(taken);
                info.offset = find(&exif, offset_tag).and_then(|e| t.ascii(e)).and_then(|s| s.parse().ok());
                break;
            }
        }
    }
    if let Some(gps_at) = find(&ifd0, 0x8825).and_then(|e| t.uint(e)) {
        let gps = t.ifd(gps_at as usize);
        let coord = |ref_tag, tag, negative: char| {
            let dms = find(&gps, tag).and_then(|e| t.rationals(e)).filter(|v| v.len() == 3)?;
            let value = dms[0] + dms[1] / 60.0 + dms[2] / 3600.0;
            let r = find(&gps, ref_tag).and_then(|e| t.ascii(e)).unwrap_or_default();
            Some(if r.starts_with(negative) { -value } else { value })
        };
        if let (Some(lat), Some(lon)) = (coord(1, 2, 'S'), coord(3, 4, 'W')) {
            info.gps = Some(GpsPoint { lat, lon }).filter(GpsPoint::is_valid);
        }
        let date = find(&gps, 29)
            .and_then(|e| t.ascii(e))
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y:%m:%d").ok());
        let time = find(&gps, 7)
            .and_then(|e| t.rationals(e))
            .filter(|v| v.len() == 3)
            .and_then(|hms| NaiveTime::from_hms_opt(hms[0] as u32, hms[1] as u32, hms[2].floor() as u32));
        if let (Some(d), Some(tm)) = (date, time) {
            info.gps_time = Some(d.and_time(tm).and_utc());
        }
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

    /// Builds a little-endian TIFF block from IFD0, EXIF and GPS entries: (tag, type, count, value
    /// bytes).
    pub fn tiff(
        ifd0: &[(u16, u16, u32, Vec<u8>)],
        exif: &[(u16, u16, u32, Vec<u8>)],
        gps: &[(u16, u16, u32, Vec<u8>)],
    ) -> Vec<u8> {
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

    pub fn ascii(s: &str) -> (u16, u32, Vec<u8>) {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        (ASCII, v.len() as u32, v)
    }

    pub fn rationals(values: &[(u32, u32)]) -> (u16, u32, Vec<u8>) {
        let v = values.iter().flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes())).collect();
        (RATIONAL, values.len() as u32, v)
    }

    fn entry(tag: u16, (kind, count, value): (u16, u32, Vec<u8>)) -> (u16, u16, u32, Vec<u8>) {
        (tag, kind, count, value)
    }

    /// A phone photo from Pebble Beach: local time with offset, GPS, rotated.
    pub fn sample_tiff() -> Vec<u8> {
        tiff(
            &[(0x0112, SHORT, 1, 6u16.to_le_bytes().to_vec())],
            &[entry(0x9003, ascii("2025:06:03 15:10:42")), entry(0x9011, ascii("-07:00"))],
            &[
                entry(1, ascii("N")),
                entry(2, rationals(&[(36, 1), (33, 1), (5566, 100)])),
                entry(3, ascii("W")),
                entry(4, rationals(&[(121, 1), (56, 1), (4300, 100)])),
                entry(7, rationals(&[(22, 1), (10, 1), (42, 1)])),
                entry(29, ascii("2025:06:03")),
            ],
        )
    }

    fn date(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, s).unwrap()
    }

    #[test]
    fn reads_the_caption_facts() {
        let info = parse_tiff(&sample_tiff());
        assert_eq!(info.taken, Some(date(2025, 6, 3, 15, 10, 42)));
        assert_eq!(info.offset, FixedOffset::west_opt(7 * 3600));
        assert_eq!(info.gps_time, Some(date(2025, 6, 3, 22, 10, 42).and_utc()));
        let gps = info.gps.unwrap();
        assert!((gps.lat - 36.565461).abs() < 1e-5 && (gps.lon + 121.945278).abs() < 1e-5, "{gps:?}");
        assert_eq!(info.orientation, 6);
    }

    #[test]
    fn big_endian_and_blank_dates() {
        // The same facts big-endian: flip every multi-byte field by hand is tedious, so check the
        // header and a blank date instead.
        let blank = tiff(&[], &[entry(0x9003, ascii("0000:00:00 00:00:00"))], &[]);
        assert_eq!(parse_tiff(&blank).taken, None);
        let mut be = b"MM\0*".to_vec();
        be.extend(8u32.to_be_bytes());
        be.extend(1u16.to_be_bytes());
        be.extend(0x0112u16.to_be_bytes());
        be.extend(SHORT.to_be_bytes());
        be.extend(1u32.to_be_bytes());
        be.extend([0, 3, 0, 0]);
        be.extend(0u32.to_be_bytes());
        assert_eq!(parse_tiff(&be).orientation, 3);
        assert_eq!(parse_tiff(b"nonsense").orientation, 1);
        assert_eq!(parse_tiff(&[]), Info { orientation: 1, ..Default::default() });
    }

    #[test]
    fn zero_gps_is_no_gps() {
        let t = tiff(
            &[],
            &[],
            &[entry(2, rationals(&[(0, 1), (0, 1), (0, 1)])), entry(4, rationals(&[(0, 1), (0, 1), (0, 1)]))],
        );
        assert_eq!(parse_tiff(&t).gps, None);
    }

    #[test]
    fn finds_exif_in_containers() {
        let t = sample_tiff();
        // JPEG: SOI, an APP0, APP1 Exif, then the scan.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 1, 2];
        jpeg.extend([0xff, 0xe1]);
        jpeg.extend(((t.len() + 8) as u16).to_be_bytes());
        jpeg.extend(b"Exif\0\0");
        jpeg.extend(&t);
        jpeg.extend([0xff, 0xda, 0, 2]);
        assert_eq!(tiff_block(&jpeg), Some(&t[..]));

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(b"IHDR");
        png.extend([0; 13 + 4]);
        png.extend((t.len() as u32).to_be_bytes());
        png.extend(b"eXIf");
        png.extend(&t);
        png.extend([0; 4]);
        assert_eq!(tiff_block(&png), Some(&t[..]));

        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend(b"VP8X");
        webp.extend(10u32.to_le_bytes());
        webp.extend([0; 10]);
        webp.extend(b"EXIF");
        webp.extend(((t.len() + 6) as u32).to_le_bytes());
        webp.extend(b"Exif\0\0");
        webp.extend(&t);
        assert_eq!(tiff_block(&webp), Some(&t[..]));

        assert_eq!(tiff_block(&t), Some(&t[..]));
        assert_eq!(tiff_block(b"GIF89a"), None);
    }

    #[test]
    fn recognizes_photo_stamps_marker() {
        let xmp =
            b"<x:xmpmeta><dc:subject><rdf:Bag><rdf:li>holiday</rdf:li><rdf:li>photo-stamp</rdf:li></rdf:Bag>";
        assert!(Info::from_file_bytes(xmp).stamped);
        assert!(!Info::from_file_bytes(b"<rdf:li>photo-stamped-not</rdf:li>").stamped);
    }
}
