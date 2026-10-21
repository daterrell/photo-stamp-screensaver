//! How a photo moves on screen: framing (fill or whole over a blurred copy), the slow pan/zoom, the
//! crossfade and the caption's fading. Pure arithmetic, so the Windows drawing code stays simple.

use std::time::Duration;

use crate::playlist::Rng;
use crate::settings::{Caption, Corner};

/// How long one photo dissolves into the next (shorter for short photo times).
pub fn crossfade(seconds_per_photo: u32) -> Duration {
    Duration::from_secs_f32((seconds_per_photo as f32 / 4.0).min(1.5))
}

/// Photos keeping at least this much of themselves when cropped to the screen's shape fill it;
/// others (portraits on a landscape screen) show whole over a blurred copy.
const FILL_KEEPS: f32 = 0.7;
/// How far the slow zoom goes, filling and whole.
const FILL_ZOOM: f32 = 1.12;
const FIT_ZOOM: f32 = 1.05;

pub fn should_fill((pw, ph): (u32, u32), (sw, sh): (u32, u32)) -> bool {
    if pw == 0 || ph == 0 || sw == 0 || sh == 0 {
        return false;
    }
    let (photo, screen) = (pw as f32 / ph as f32, sw as f32 / sh as f32);
    photo.min(screen) / photo.max(screen) >= FILL_KEEPS
}

/// The most a photo is ever drawn at, relative to fitting it to the screen (so decoding can stop
/// at that size).
pub fn max_scale(photo: (u32, u32), screen: (u32, u32)) -> f32 {
    let (pw, ph) = (photo.0.max(1) as f32, photo.1.max(1) as f32);
    let (sw, sh) = (screen.0 as f32, screen.1 as f32);
    if should_fill(photo, screen) {
        (sw / pw).max(sh / ph) * FILL_ZOOM
    } else {
        (sw / pw).min(sh / ph) * FIT_ZOOM
    }
}

/// One photo's pan and zoom: from start to end over its time on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub fill: bool,
    zoom: (f32, f32),
    /// Where the visible window sits within the room to pan, -1..1 on each axis.
    pan: ((f32, f32), (f32, f32)),
}

impl Motion {
    pub fn random(rng: &mut Rng, fill: bool) -> Self {
        let max = if fill { FILL_ZOOM } else { FIT_ZOOM };
        // Half zoom in, half out.
        let zoom = if rng.below(2) == 0 { (1.0, max) } else { (max, 1.0) };
        let mut point = || (rng.unit() * 2.0 - 1.0, rng.unit() * 2.0 - 1.0);
        let pan = if fill { (point(), point()) } else { ((0.0, 0.0), (0.0, 0.0)) };
        Self { fill, zoom, pan }
    }

    pub fn still(fill: bool) -> Self {
        Self { fill, zoom: (1.0, 1.0), pan: ((0.0, 0.0), (0.0, 0.0)) }
    }

    /// Where to draw the photo (left, top, right, bottom on the screen) at `progress` 0..1 of its
    /// time on screen.
    pub fn place(&self, photo: (u32, u32), screen: (u32, u32), progress: f32) -> (f32, f32, f32, f32) {
        let p = progress.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * p;
        let (pw, ph) = (photo.0.max(1) as f32, photo.1.max(1) as f32);
        let (sw, sh) = (screen.0 as f32, screen.1 as f32);
        let base = if self.fill { (sw / pw).max(sh / ph) } else { (sw / pw).min(sh / ph) };
        let scale = base * lerp(self.zoom.0, self.zoom.1);
        let (w, h) = (pw * scale, ph * scale);
        // The room to pan is how much bigger than the screen the photo is.
        let room = (((w - sw) / 2.0).max(0.0), ((h - sh) / 2.0).max(0.0));
        let (px, py) = (lerp(self.pan.0.0, self.pan.1.0), lerp(self.pan.0.1, self.pan.1.1));
        let (cx, cy) = (sw / 2.0 + px * room.0, sh / 2.0 + py * room.1);
        (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
    }
}

/// The caption's opacity `t` into a photo's `shown` time.
pub fn caption_opacity(mode: Caption, t: Duration, shown: Duration) -> f32 {
    const DELAY: f32 = 1.0;
    const FADE: f32 = 1.0;
    let (t, shown) = (t.as_secs_f32(), shown.as_secs_f32());
    match mode {
        Caption::Always => 1.0,
        Caption::Hidden => 0.0,
        // In a moment after the photo appears, out before the next one starts.
        Caption::Fade => {
            let fade_in = ((t - DELAY) / FADE).clamp(0.0, 1.0);
            let fade_out = ((shown - FADE - t) / FADE).clamp(0.0, 1.0);
            fade_in.min(fade_out)
        }
    }
}

/// Where the caption's block goes: its top-left corner, for a block of `size` on `screen`.
pub fn caption_origin(corner: Corner, size: (f32, f32), screen: (u32, u32), margin: f32) -> (f32, f32) {
    let (sw, sh) = (screen.0 as f32, screen.1 as f32);
    let x = match corner {
        Corner::BottomLeft | Corner::TopLeft => margin,
        Corner::BottomRight | Corner::TopRight => sw - margin - size.0,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => margin,
        Corner::BottomLeft | Corner::BottomRight => sh - margin - size.1,
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (u32, u32) = (3840, 2160);

    #[test]
    fn landscape_fills_portrait_doesnt() {
        assert!(should_fill((4000, 3000), SCREEN), "4:3 keeps 75%");
        assert!(should_fill((6000, 4000), SCREEN), "3:2 keeps 84%");
        assert!(!should_fill((3000, 4000), SCREEN), "portrait");
        assert!(!should_fill((6000, 2000), SCREEN), "panorama");
        assert!(should_fill((3000, 4000), (1080, 1920)), "portrait on a portrait screen");
    }

    #[test]
    fn filling_always_covers_the_screen() {
        let mut rng = Rng::from_time();
        for _ in 0..200 {
            let m = Motion::random(&mut rng, true);
            for p in [0.0, 0.3, 1.0] {
                let (l, t, r, b) = m.place((4000, 3000), SCREEN, p);
                assert!(
                    l <= 0.01 && t <= 0.01 && r >= 3839.99 && b >= 2159.99,
                    "{m:?} at {p}: {l} {t} {r} {b}"
                );
            }
        }
    }

    #[test]
    fn whole_photos_stay_centered_and_in_bounds() {
        let m = Motion::random(&mut Rng::from_time(), false);
        let (l, t, r, b) = m.place((3000, 4000), SCREEN, 0.0);
        assert!((l + r - 3840.0).abs() < 0.01 && (t + b - 2160.0).abs() < 0.01);
        assert!(b - t <= 2160.0 * FIT_ZOOM + 0.01);
        assert!(max_scale((3000, 4000), SCREEN) >= (b - t) / 4000.0);
    }

    #[test]
    fn caption_fades() {
        let shown = Duration::from_secs(10);
        let at = |s: f32| caption_opacity(Caption::Fade, Duration::from_secs_f32(s), shown);
        assert_eq!(at(0.5), 0.0);
        assert_eq!(at(5.0), 1.0);
        assert!((at(1.5) - 0.5).abs() < 0.01);
        assert_eq!(at(9.5), 0.0);
        assert_eq!(caption_opacity(Caption::Always, Duration::ZERO, shown), 1.0);
        assert_eq!(caption_opacity(Caption::Hidden, Duration::from_secs(5), shown), 0.0);
    }

    #[test]
    fn caption_corners() {
        assert_eq!(caption_origin(Corner::BottomLeft, (400.0, 100.0), (1920, 1080), 40.0), (40.0, 940.0));
        assert_eq!(caption_origin(Corner::TopRight, (400.0, 100.0), (1920, 1080), 40.0), (1480.0, 40.0));
    }

    #[test]
    fn short_photo_times_fade_faster() {
        assert_eq!(crossfade(10), Duration::from_secs_f32(1.5));
        assert_eq!(crossfade(3), Duration::from_secs_f32(0.75));
    }
}
