//! Direct2D pieces the slideshow and the caption editor share: a decoded photo onto the GPU, and
//! the caption laid out and drawn in photo-stamp's look.

use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE, ID2D1Bitmap1, ID2D1DeviceContext,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_WRAP, IDWriteFactory, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::w;
use windows_numerics::Vector2;

use super::decode::Pixels;
use crate::settings::Corner;
use crate::slide;

pub fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

pub fn upload(dc: &ID2D1DeviceContext, pixels: &Pixels) -> windows::core::Result<ID2D1Bitmap1> {
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    unsafe {
        dc.CreateBitmap(
            D2D_SIZE_U { width: pixels.width, height: pixels.height },
            Some(pixels.bgra.as_ptr().cast()),
            pixels.width * 4,
            &props,
        )
    }
}

/// The caption's text size: as photo-stamp sizes its caption, a little smaller for a screen seen
/// from afar.
pub fn caption_font(screen: (u32, u32)) -> f32 {
    (screen.0.min(screen.1) as f32 * 0.032).max(9.0)
}

/// The caption laid out for a screen of `screen` pixels, and where it goes on it.
pub fn caption_layout(
    dwrite: &IDWriteFactory,
    lines: &[String],
    screen: (u32, u32),
    corner: Corner,
) -> windows::core::Result<(IDWriteTextLayout, (f32, f32))> {
    let short = screen.0.min(screen.1) as f32;
    let margin = short * 0.04;
    let text: Vec<u16> = lines.join("\n").encode_utf16().collect();
    unsafe {
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            caption_font(screen),
            w!("en-us"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        let right = matches!(corner, Corner::BottomRight | Corner::TopRight);
        format.SetTextAlignment(if right {
            DWRITE_TEXT_ALIGNMENT_TRAILING
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        })?;
        let layout = dwrite.CreateTextLayout(&text, &format, screen.0 as f32 * 0.6, screen.1 as f32)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        // Shrink the box to the text, so right-aligned lines end at the margin.
        layout.SetMaxWidth(metrics.width)?;
        let origin = slide::caption_origin(corner, (metrics.width, metrics.height), screen, margin);
        Ok((layout, origin))
    }
}

/// White text with a dark outline and a soft shadow, like photo-stamp's, so it reads on skies, snow
/// and night shots alike.
pub fn caption(
    dc: &ID2D1DeviceContext,
    white: &ID2D1SolidColorBrush,
    black: &ID2D1SolidColorBrush,
    layout: &IDWriteTextLayout,
    (x, y): (f32, f32),
    opacity: f32,
    font: f32,
) {
    let outline = (font * 0.07).max(1.0);
    let shadow = font * 0.09;
    unsafe {
        black.SetOpacity(opacity * 0.35);
        for (dx, dy) in [(shadow, shadow), (shadow * 0.6, shadow * 1.2)] {
            dc.DrawTextLayout(Vector2 { X: x + dx, Y: y + dy }, layout, black, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        black.SetOpacity(opacity * 0.8);
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            let at = Vector2 { X: x + a.cos() * outline, Y: y + a.sin() * outline };
            dc.DrawTextLayout(at, layout, black, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        white.SetOpacity(opacity);
        dc.DrawTextLayout(Vector2 { X: x, Y: y }, layout, white, D2D1_DRAW_TEXT_OPTIONS_NONE);
    }
}
