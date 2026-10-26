//! An edited caption kept inside the photo: an XMP property in our own namespace, `pss:Caption`, an
//! `rdf:Seq` of lines (empty: no caption at all), in its own `rdf:Description` beside whatever XMP
//! the photo already has (photo-stamp's marker, the camera's, a photo app's).
//!
//! Written into JPEG (the APP1 XMP segment) and PNG (the `XML:com.adobe.xmp` iTXt chunk) through
//! `img-parts`. Other formats (HEIC, WebP, TIFF...) keep an edit in the shared cache only. The
//! photo is rewritten beside itself, checked, and swapped in with its modified time kept.

use std::path::Path;

use img_parts::Bytes;
use img_parts::jpeg::{Jpeg, JpegSegment, markers};
use img_parts::png::{Png, PngChunk};
use memchr::memmem;
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};

/// Our namespace.
pub const NS: &str = "https://forgejo.terrell.ltd/dave/photo-stamp-screensaver/ns/1.0/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// What starts a JPEG's XMP segment.
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// A PNG iTXt chunk's keyword for XMP, then uncompressed, no language, no translated keyword.
const PNG_XMP: &[u8] = b"XML:com.adobe.xmp\0";
const ITXT: [u8; 4] = *b"iTXt";
const IDAT: [u8; 4] = *b"IDAT";
/// A JPEG segment holds at most this much (its length field counts itself).
const MAX_SEGMENT: usize = 65533;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Jpeg,
    Png,
}

fn kind(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => Some(Kind::Jpeg),
        "png" => Some(Kind::Png),
        _ => None,
    }
}

/// Whether a caption can be kept in this kind of photo.
pub fn can_write(path: &Path) -> bool {
    kind(path).is_some()
}

/// The edited caption somewhere in a photo's bytes, if it has one (empty: no caption). Quick for
/// the photos without: nothing is parsed unless our namespace is there.
pub fn find_caption(bytes: &[u8]) -> Option<Vec<String>> {
    let at = memmem::find(bytes, NS.as_bytes())?;
    let start = memmem::rfind(&bytes[..at], b"<x:xmpmeta")?;
    let end = at + memmem::find(&bytes[at..], b"</x:xmpmeta>")? + b"</x:xmpmeta>".len();
    caption(std::str::from_utf8(&bytes[start..end]).ok()?)
}

fn is(ns: &ResolveResult, uri: &str) -> bool {
    matches!(ns, ResolveResult::Bound(Namespace(n)) if *n == uri)
}

/// An entity reference's character: `&amp;`, `&#233;`.
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// The caption lines in an XMP packet, if it has ours.
pub fn caption(packet: &str) -> Option<Vec<String>> {
    let mut reader = NsReader::from_str(packet);
    // Inside pss:Caption, and inside one of its rdf:li.
    let (mut depth, mut lines, mut line): (usize, Option<Vec<String>>, Option<String>) = (0, None, None);
    loop {
        match reader.read_resolved_event().ok()? {
            (ns, Event::Start(e)) => {
                if depth > 0 {
                    depth += 1;
                    if is(&ns, RDF) && e.local_name().as_ref() == "li" {
                        line = Some(String::new());
                    }
                } else if is(&ns, NS) && e.local_name().as_ref() == "Caption" {
                    depth = 1;
                    lines = Some(Vec::new());
                }
            }
            (ns, Event::Empty(e)) => {
                if depth > 0 && is(&ns, RDF) && e.local_name().as_ref() == "li" {
                    lines.get_or_insert_default().push(String::new());
                } else if depth == 0 && is(&ns, NS) && e.local_name().as_ref() == "Caption" {
                    return Some(Vec::new());
                }
            }
            (_, Event::Text(t)) => {
                if let Some(l) = &mut line {
                    l.push_str(&t.xml10_content());
                }
            }
            (_, Event::CData(t)) => {
                if let Some(l) = &mut line {
                    l.push_str(&t.xml10_content());
                }
            }
            (_, Event::GeneralRef(r)) => {
                if let (Some(l), Some(c)) = (&mut line, entity(&r)) {
                    l.push(c);
                }
            }
            (_, Event::End(_)) if depth > 0 => {
                if let Some(l) = line.take() {
                    lines.get_or_insert_default().push(l.trim().to_owned());
                }
                depth -= 1;
                if depth == 0 {
                    return lines;
                }
            }
            (_, Event::Eof) => return None,
            _ => {}
        }
    }
}

/// Our `rdf:Description`, holding `lines`.
fn description(lines: &[String]) -> String {
    let items: String =
        lines.iter().map(|l| format!("<rdf:li>{}</rdf:li>", quick_xml::escape::escape(l.trim()))).collect();
    let seq = if items.is_empty() { "<rdf:Seq/>".to_owned() } else { format!("<rdf:Seq>{items}</rdf:Seq>") };
    format!(
        "<rdf:Description rdf:about=\"\" xmlns:pss=\"{NS}\" xmlns:rdf=\"{RDF}\"><pss:Caption>{seq}</pss:Caption></rdf:Description>"
    )
}

/// A whole packet with only our caption.
fn new_packet(lines: &[String]) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\">{}</rdf:RDF></x:xmpmeta>\
         <?xpacket end=\"w\"?>",
        description(lines)
    )
}

/// `packet` with our caption set to `lines` (`None`: removed). Everything else in it is kept as it
/// was; an earlier caption of ours goes, and so does the description that held only that.
/// `Ok(None)`: there's nothing left to write (no packet, nothing to add, or nothing left in it).
pub fn with_caption(packet: Option<&str>, lines: Option<&[String]>) -> Result<Option<String>, String> {
    let Some(packet) = packet else { return Ok(lines.map(new_packet)) };
    let mut reader = NsReader::from_str(packet);
    let mut out = String::with_capacity(packet.len() + 256);
    // Where the last event ended in `packet`: everything is copied through byte for byte, apart
    // from what's left out.
    let mut copied = 0;
    // Depth in the document; where a skipped element started, and its depth.
    let mut depth = 0usize;
    let mut skip: Option<usize> = None;
    // An rdf:Description under rdf:RDF, held back until we know whether anything but our caption
    // is in it: its text so far and whether it has more.
    let mut held: Option<(String, bool)> = None;
    let mut rdf_depth: Option<usize> = None;
    let mut inserted = false;
    // Whether anything but ours is in rdf:RDF.
    let mut kept = false;
    let emit = |out: &mut String, held: &mut Option<(String, bool)>, s: &str| match held {
        Some((text, _)) => text.push_str(s),
        None => out.push_str(s),
    };
    loop {
        let (ns, event) = reader.read_resolved_event().map_err(|e| format!("its XMP is broken: {e}"))?;
        let ns_is_rdf = is(&ns, RDF);
        let ns_is_ours = is(&ns, NS);
        let end = reader.buffer_position() as usize;
        let raw = &packet[copied..end];
        match event {
            Event::Eof => {
                emit(&mut out, &mut held, raw);
                break;
            }
            _ if skip.is_some() => match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if Some(depth) == skip {
                        skip = None;
                    }
                }
                _ => {}
            },
            Event::Start(e) => {
                let name = e.local_name();
                if ns_is_ours && name.as_ref() == "Caption" {
                    skip = Some(depth);
                    depth += 1;
                } else if rdf_depth == Some(depth) && ns_is_rdf && name.as_ref() == "Description" {
                    held = Some((raw.to_owned(), has_properties(&e)));
                    depth += 1;
                } else {
                    if let Some(h) = &mut held {
                        h.1 = true;
                    }
                    kept |= rdf_depth == Some(depth);
                    if ns_is_rdf && name.as_ref() == "RDF" && rdf_depth.is_none() {
                        rdf_depth = Some(depth + 1);
                    }
                    emit(&mut out, &mut held, raw);
                    depth += 1;
                }
            }
            Event::Empty(e) => {
                if !(ns_is_ours && e.local_name().as_ref() == "Caption") {
                    if let Some(h) = &mut held {
                        h.1 = true;
                    }
                    kept |= rdf_depth == Some(depth);
                    emit(&mut out, &mut held, raw);
                }
            }
            Event::End(e) => {
                depth -= 1;
                if held.is_some() && rdf_depth == Some(depth) {
                    let (mut text, keep) = held.take().unwrap();
                    if keep {
                        text.push_str(raw);
                        out.push_str(&text);
                        kept = true;
                    }
                } else {
                    if rdf_depth == Some(depth + 1) && ns_is_rdf && e.local_name().as_ref() == "RDF" {
                        // Ours goes last in rdf:RDF.
                        let (before, close) = raw.split_at(raw.rfind("</").unwrap_or(0));
                        out.push_str(before);
                        if let Some(lines) = lines {
                            out.push_str(&description(lines));
                        }
                        out.push_str(close);
                        inserted = true;
                    } else {
                        emit(&mut out, &mut held, raw);
                    }
                }
            }
            _ => emit(&mut out, &mut held, raw),
        }
        copied = end;
    }
    if !inserted {
        return Err("its XMP has no rdf:RDF".into());
    }
    Ok((lines.is_some() || kept).then_some(out))
}

/// Whether a description's own attributes say anything (properties written as attributes), beyond
/// `rdf:about` and namespace declarations.
fn has_properties(e: &quick_xml::events::BytesStart) -> bool {
    e.attributes().flatten().any(|a| {
        let key = a.key.as_ref();
        !(key == "xmlns" || key.starts_with("xmlns:") || key.ends_with(":about") || key == "about")
    })
}

fn xmp_of_jpeg(jpeg: &Jpeg) -> Option<&JpegSegment> {
    jpeg.segments().iter().find(|s| s.marker() == markers::APP1 && s.contents().starts_with(JPEG_XMP))
}

fn xmp_of_png(png: &Png) -> Option<&PngChunk> {
    png.chunks().iter().find(|c| c.kind() == ITXT && c.contents().starts_with(PNG_XMP))
}

fn text(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|_| "its XMP isn't UTF-8".to_owned())
}

/// `bytes` (a photo of `kind`) with the caption set to `lines`, or removed.
fn rewrite(kind: Kind, bytes: Vec<u8>, lines: Option<&[String]>) -> Result<Vec<u8>, String> {
    let unreadable = |e: img_parts::Error| format!("can't read the photo's structure: {e}");
    let mut out = Vec::new();
    match kind {
        Kind::Jpeg => {
            let mut jpeg = Jpeg::from_bytes(Bytes::from(bytes)).map_err(unreadable)?;
            let old = xmp_of_jpeg(&jpeg).map(|s| s.contents().slice(JPEG_XMP.len()..));
            let packet = with_caption(old.as_deref().map(text).transpose()?, lines)?;
            let at = jpeg
                .segments()
                .iter()
                .position(|s| s.marker() == markers::APP1 && s.contents().starts_with(JPEG_XMP));
            let segment = match packet {
                Some(packet) => {
                    let contents = [JPEG_XMP, packet.as_bytes()].concat();
                    if contents.len() > MAX_SEGMENT {
                        return Err("its metadata is too big to add a caption to".into());
                    }
                    Some(JpegSegment::new_with_contents(markers::APP1, Bytes::from(contents)))
                }
                None => None,
            };
            match (at, segment) {
                (Some(i), Some(segment)) => jpeg.segments_mut()[i] = segment,
                // Nothing but our caption was in it.
                (Some(i), None) => drop(jpeg.segments_mut().remove(i)),
                (None, None) => return Err("nothing to change".into()),
                (None, Some(segment)) => {
                    // After the JFIF and EXIF headers, where readers look for it.
                    let i = jpeg
                        .segments()
                        .iter()
                        .take_while(|s| matches!(s.marker(), markers::APP0 | markers::APP1))
                        .count();
                    jpeg.segments_mut().insert(i, segment);
                }
            }
            jpeg.encoder().write_to(&mut out).map_err(|e| e.to_string())?;
        }
        Kind::Png => {
            let mut png = Png::from_bytes(Bytes::from(bytes)).map_err(unreadable)?;
            let old = match xmp_of_png(&png) {
                Some(c) => {
                    let rest = &c.contents()[PNG_XMP.len()..];
                    // Compression flag and method, then language and translated keyword, both
                    // NUL-terminated.
                    if rest.first() != Some(&0) {
                        return Err("its XMP is compressed".into());
                    }
                    let mut parts = rest.get(2..).unwrap_or_default().splitn(3, |&b| b == 0);
                    let (_, _, xml) = (parts.next(), parts.next(), parts.next().unwrap_or_default());
                    Some(text(xml)?.to_owned())
                }
                None => None,
            };
            let chunk = with_caption(old.as_deref(), lines)?.map(|packet| {
                PngChunk::new(ITXT, Bytes::from([PNG_XMP, b"\0\0\0\0", packet.as_bytes()].concat()))
            });
            let chunks = png.chunks_mut();
            match (chunks.iter().position(|c| c.kind() == ITXT && c.contents().starts_with(PNG_XMP)), chunk) {
                (Some(i), Some(chunk)) => chunks[i] = chunk,
                (Some(i), None) => drop(chunks.remove(i)),
                (None, None) => return Err("nothing to change".into()),
                (None, Some(chunk)) => {
                    let i = chunks.iter().position(|c| c.kind() == IDAT).unwrap_or(1);
                    chunks.insert(i, chunk);
                }
            }
            png.encoder().write_to(&mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// Keeps `lines` in the photo at `path` (`None`: takes an edited caption out again). The rest of
/// the file is unchanged, and so is its modified time. `Err`: why it couldn't be, for the person.
pub fn write(path: &Path, lines: Option<&[String]>) -> Result<(), String> {
    let kind = kind(path).ok_or("this kind of photo can't hold a caption")?;
    let bytes = std::fs::read(path).map_err(|e| format!("can't read it: {e}"))?;
    let before = crate::exif::Info::from_file_bytes(&bytes);
    if lines.is_none() && before.caption.is_none() {
        return Ok(());
    }
    let new = rewrite(kind, bytes, lines)?;
    // Checked before it replaces anything: the photo's own metadata reads the same, and the
    // caption reads back.
    let after = crate::exif::Info::from_file_bytes(&new);
    let expected = crate::exif::Info {
        caption: lines.map(|l| l.iter().map(|s| s.trim().to_owned()).collect()),
        ..before.clone()
    };
    if after != expected {
        return Err("the rewritten photo didn't read back the same; it was left as it was".into());
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let temp = sibling(path, ".pss-new");
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, &new)?;
        file.sync_all()?;
        if let Some(t) = modified {
            file.set_modified(t)?;
        }
        Ok::<_, std::io::Error>(())
    })();
    if let Err(e) = written.and_then(|()| replace(path, &temp)) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("can't save it: {e}"));
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(suffix);
    path.with_file_name(name)
}

/// Puts `new` in place of `path`. On Windows, `ReplaceFileW` keeps the original's attributes,
/// permissions and creation time.
#[cfg(windows)]
fn replace(path: &Path, new: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (target, source) = (wide(path), wide(new));
    let ok = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            source.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(not(windows))]
fn replace(path: &Path, new: &Path) -> std::io::Result<()> {
    std::fs::rename(new, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::Info;
    use crate::exif::tests::{jpeg_with, sample_tiff};

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pss-test-xmp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// photo-stamp's marker and a photo app's description, as exiftool writes them: everything in
    /// one description.
    const FOREIGN: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>photo-stamp</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn reads_and_writes_packets() {
        let mine = lines(&["Grandma's garden", "Fish & chips <3", "Sun 3 Aug 1997"]);
        let packet = with_caption(None, Some(&mine)).unwrap().unwrap();
        assert_eq!(caption(&packet), Some(mine.clone()));
        assert_eq!(with_caption(None, None).unwrap(), None);
        // Nothing left once ours is out of a packet we made.
        assert_eq!(with_caption(Some(&packet), None).unwrap(), None);
        // An empty caption: none shown.
        assert_eq!(caption(&with_caption(None, Some(&[])).unwrap().unwrap()), Some(vec![]));

        let added = with_caption(Some(FOREIGN), Some(&mine)).unwrap().unwrap();
        assert_eq!(caption(&added), Some(mine.clone()));
        assert!(added.contains("<rdf:li>photo-stamp</rdf:li>") && added.contains(r#"xmp:Rating="4""#));
        // Changed again: still one caption, the new one.
        let changed = with_caption(Some(&added), Some(&lines(&["Home"]))).unwrap().unwrap();
        assert_eq!(caption(&changed), Some(lines(&["Home"])));
        assert_eq!(changed.matches("pss:Caption>").count(), 2, "{changed}");
        // Taken out: exactly what was there before.
        assert_eq!(with_caption(Some(&changed), None).unwrap().unwrap(), FOREIGN);
        assert_eq!(caption(FOREIGN), None);
    }

    #[test]
    fn reads_a_caption_merged_into_another_description() {
        // As exiftool leaves it after copying the tags around.
        let merged = FOREIGN.replace(
            r#"xmp:Rating="4">"#,
            &format!(
                r#"xmp:Rating="4" xmlns:pss="{NS}"><pss:Caption><rdf:Seq><rdf:li>Caf&#233; &amp; bar</rdf:li></rdf:Seq></pss:Caption>"#
            ),
        );
        assert_eq!(caption(&merged), Some(lines(&["Café & bar"])));
        // Rewriting it keeps the description (it holds more), without the old caption.
        let rewritten = with_caption(Some(&merged), Some(&lines(&["Café"]))).unwrap().unwrap();
        assert_eq!(caption(&rewritten), Some(lines(&["Café"])));
        assert!(rewritten.contains("<rdf:li>photo-stamp</rdf:li>"));
        assert!(!rewritten.contains("bar"));
    }

    #[test]
    fn keeps_a_caption_in_a_jpeg() {
        let dir = temp("jpeg");
        let path = dir.join("a.jpg");
        let original = jpeg_with(&sample_tiff());
        std::fs::write(&path, &original).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
        let before = Info::from_file_bytes(&original);

        let mine = lines(&["Pebble Beach", "with Grandpa"]);
        write(&path, Some(&mine)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(Info::from_file_bytes(&bytes), Info { caption: Some(mine), ..before.clone() });
        assert_eq!(exif_of(&bytes), exif_of(&original));
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
        assert!(!dir.join("a.jpg.pss-new").exists());

        write(&path, Some(&lines(&["Monterey"]))).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(Info::from_file_bytes(&bytes).caption, Some(lines(&["Monterey"])));
        assert_eq!(memmem::find_iter(&bytes, JPEG_XMP).count(), 1);

        // Back as it was, byte for byte.
        write(&path, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    fn exif_of(bytes: &[u8]) -> Option<Bytes> {
        use img_parts::ImageEXIF;
        Jpeg::from_bytes(Bytes::copy_from_slice(bytes)).ok()?.exif()
    }

    #[test]
    fn keeps_a_caption_in_a_png() {
        let dir = temp("png");
        let path = dir.join("a.png");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let ihdr = [1u32.to_be_bytes(), 1u32.to_be_bytes()].concat();
        for (kind, data) in [
            (*b"IHDR", [&ihdr[..], &[8, 0, 0, 0, 0]].concat()),
            (IDAT, b"x\x9cc\0\0\0\x01\0\x01".to_vec()),
            (*b"IEND", vec![]),
        ] {
            PngChunk::new(kind, Bytes::from(data)).encoder().write_to(&mut png).unwrap();
        }
        std::fs::write(&path, &png).unwrap();
        let original = png;
        write(&path, Some(&lines(&["Back yard"]))).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(Info::from_file_bytes(&bytes).caption, Some(lines(&["Back yard"])));
        let png = Png::from_bytes(Bytes::from(bytes)).unwrap();
        let kinds: Vec<_> = png.chunks().iter().map(|c| c.kind()).collect();
        assert_eq!(kinds, [*b"IHDR", ITXT, IDAT, *b"IEND"]);
        write(&path, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn other_formats_arent_written() {
        let dir = temp("heic");
        let path = dir.join("a.heic");
        std::fs::write(&path, b"not really").unwrap();
        assert!(!can_write(&path));
        assert!(write(&path, Some(&lines(&["x"]))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not really");
    }
}
