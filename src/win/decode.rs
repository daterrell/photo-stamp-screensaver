//! Decoding a photo for the screen with Windows' own decoders (WIC: JPEG, PNG, TIFF, WebP, BMP,
//! GIF), or libheif for HEIC: scaled down to the most it will be drawn at, turned upright, as
//! premultiplied BGRA ready for Direct2D. Runs on the loader threads, not the UI thread.

use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom,
    WICBitmapTransformFlipHorizontal, WICBitmapTransformFlipVertical, WICBitmapTransformOptions,
    WICBitmapTransformRotate0, WICBitmapTransformRotate90, WICBitmapTransformRotate180,
    WICBitmapTransformRotate270, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{Interface, Result};

use crate::library::Ready;
use crate::slide;

/// The backdrop behind whole photos is this small: drawn stretched, it's a soft blur.
const BACKDROP_SIDE: u32 = 32;
/// Direct2D's textures go to 16384 on most GPUs; stay well within.
const MAX_SIDE: u32 = 8192;

/// Premultiplied BGRA, rows packed.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub struct Decoded {
    pub photo: Pixels,
    /// For photos shown whole: a tiny copy to blur behind them.
    pub backdrop: Option<Pixels>,
    pub fill: bool,
}

pub struct Decoder {
    factory: IWICImagingFactory,
}

/// EXIF orientation as WIC's rotate-then-flip.
fn transform(orientation: u8) -> WICBitmapTransformOptions {
    match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0),
        8 => WICBitmapTransformRotate270,
        _ => WICBitmapTransformRotate0,
    }
}

impl Decoder {
    /// For this thread (it joins COM's multithreaded apartment).
    pub fn new() -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            Ok(Self { factory: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? })
        }
    }

    /// The photo for a screen of `screen` pixels.
    pub fn decode(&self, ready: &Ready, screen: (u32, u32)) -> std::result::Result<Decoded, String> {
        let source = self.source(ready).map_err(|e| e.to_string())?;
        let mut size = (0, 0);
        unsafe { source.GetSize(&mut size.0, &mut size.1) }.map_err(|e| e.to_string())?;
        let orientation = ready.info.orientation;
        let swap = (5..=8).contains(&orientation);
        let upright = if swap { (size.1, size.0) } else { size };
        let fill = slide::should_fill(upright, screen);
        let scale = slide::max_scale(upright, screen)
            .min(1.0)
            .min(MAX_SIDE as f32 / upright.0.max(upright.1).max(1) as f32);
        let photo = self.pixels(&source, upright, scale, orientation).map_err(|e| e.to_string())?;
        let backdrop = if fill {
            None
        } else {
            let tiny = BACKDROP_SIDE as f32 / upright.0.max(upright.1).max(1) as f32;
            self.pixels(&source, upright, tiny.min(1.0), orientation).ok()
        };
        Ok(Decoded { photo, backdrop, fill })
    }

    fn source(&self, ready: &Ready) -> Result<IWICBitmapSource> {
        unsafe {
            #[cfg(feature = "heif")]
            if crate::heif::is_heif(&ready.entry.path) {
                let rgba = crate::heif::decode(&ready.bytes).map_err(|e| {
                    windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), e)
                })?;
                let bitmap = self.factory.CreateBitmapFromMemory(
                    rgba.width,
                    rgba.height,
                    &windows::Win32::Graphics::Imaging::GUID_WICPixelFormat32bppRGBA,
                    rgba.width * 4,
                    &rgba.pixels,
                )?;
                return bitmap.cast();
            }
            let stream = self.factory.CreateStream()?;
            stream.InitializeFromMemory(&ready.bytes)?;
            let decoder = self.factory.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )?;
            decoder.GetFrame(0)?.cast()
        }
    }

    /// `upright`: the size once turned; `scale`: of that.
    fn pixels(
        &self,
        source: &IWICBitmapSource,
        upright: (u32, u32),
        scale: f32,
        orientation: u8,
    ) -> Result<Pixels> {
        unsafe {
            let w = ((upright.0 as f32 * scale).round() as u32).max(1);
            let h = ((upright.1 as f32 * scale).round() as u32).max(1);
            let swap = (5..=8).contains(&orientation);
            let mut current: IWICBitmapSource = source.clone();
            if scale < 1.0 {
                let scaler = self.factory.CreateBitmapScaler()?;
                let (sw, sh) = if swap { (h, w) } else { (w, h) };
                scaler.Initialize(&current, sw, sh, WICBitmapInterpolationModeHighQualityCubic)?;
                current = scaler.cast()?;
            }
            if orientation > 1 {
                let rotator = self.factory.CreateBitmapFlipRotator()?;
                rotator.Initialize(&current, transform(orientation))?;
                current = rotator.cast()?;
            }
            let converter = self.factory.CreateFormatConverter()?;
            converter.Initialize(
                &current,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let (mut width, mut height) = (0, 0);
            converter.GetSize(&mut width, &mut height)?;
            let mut bgra = vec![0u8; width as usize * height as usize * 4];
            converter.CopyPixels(std::ptr::null(), width * 4, &mut bgra)?;
            Ok(Pixels { width, height, bgra })
        }
    }
}
