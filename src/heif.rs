//! HEIC/HEIF (iPhone photos) through libheif, linked statically, so it works without Windows' paid
//! HEVC extension. libheif applies the container's rotation and mirroring while decoding.

use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

pub fn is_heif(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| matches!(ext.to_ascii_lowercase().as_str(), "heic" | "heif" | "hif"))
}

/// Decoded pixels, RGBA, rows packed.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes).map_err(|e| e.to_string())?;
    let handle = ctx.primary_image_handle().map_err(|e| e.to_string())?;
    let decoded = lib.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None).map_err(|e| e.to_string())?;
    let planes = decoded.planes();
    let plane = planes.interleaved.ok_or("libheif returned no interleaved plane")?;
    let row = plane.width as usize * 4;
    let mut pixels = Vec::with_capacity(row * plane.height as usize);
    for y in 0..plane.height as usize {
        pixels.extend_from_slice(&plane.data[y * plane.stride..][..row]);
    }
    Ok(Rgba { width: plane.width, height: plane.height, pixels })
}
