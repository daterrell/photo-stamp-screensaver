//! The screensaver: one window over every monitor (or inside Screen Saver Settings' preview), each
//! monitor with its own slideshow, the changes staggered. Each monitor has a loader thread that gets
//! the next photo ready (read, place looked up, decoded) while the current one shows; the UI thread
//! only draws: Direct2D, synced to the display.

use std::cell::Cell;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{Interface, w};
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BLACK_BRUSH, EnumDisplayMonitors, GetStockObject, HBRUSH, HDC, HMONITOR,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursor, TranslateMessage, WM_ACTIVATEAPP,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WM_SYSKEYDOWN, WNDCLASSW, WS_CHILD, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use super::decode::{Decoded, Decoder, Pixels};
use super::{store, wide};
use crate::library::Library;
use crate::playlist::Rng;
use crate::settings::{Caption, Corner, Settings};
use crate::slide::{self, Motion};

/// How far the mouse may drift (a desk bump) before the screensaver closes, in pixels.
const MOUSE_SLACK: i32 = 8;
/// Direct2D's "the GPU went away" (driver update, remote desktop): make everything again.
const D2DERR_RECREATE_TARGET: i32 = 0x8899000Cu32 as i32;

thread_local! {
    static PREVIEW: Cell<bool> = const { Cell::new(false) };
    /// `/w`: in an ordinary window that input doesn't close.
    static WINDOWED: Cell<bool> = const { Cell::new(false) };
    /// Where the mouse was first seen.
    static MOUSE_START: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

/// `/s`: fullscreen across every monitor, until there's input.
pub fn run() {
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    // Each monitor, relative to the window's top-left.
    let screens = monitors()
        .into_iter()
        .map(|r| RECT { left: r.left - x, top: r.top - y, right: r.right - x, bottom: r.bottom - y })
        .collect();
    show(WS_EX_TOPMOST, WS_POPUP | WS_VISIBLE, (x, y, w, h), null_mut(), screens);
}

/// `/w`: the slideshow in an ordinary 1280×720 window, for trying it out.
pub fn window() {
    WINDOWED.set(true);
    let (w, h) = (1280, 720);
    let style = windows_sys::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW | WS_VISIBLE;
    let mut frame = RECT { left: 0, top: 0, right: w, bottom: h };
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AdjustWindowRect(&mut frame, style, 0) };
    let outer = (frame.right - frame.left, frame.bottom - frame.top);
    show(
        0,
        style,
        (100, 100, outer.0, outer.1),
        null_mut(),
        vec![RECT { left: 0, top: 0, right: w, bottom: h }],
    );
}

/// `/p HWND`: inside the little monitor in Screen Saver Settings, until it closes.
pub fn preview(parent: isize) {
    PREVIEW.set(true);
    let parent = parent as HWND;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(parent, &mut rect) };
    show(0, WS_CHILD | WS_VISIBLE, (0, 0, rect.right, rect.bottom), parent, vec![rect]);
}

fn monitors() -> Vec<RECT> {
    unsafe extern "system" fn add(_: HMONITOR, _: HDC, rect: *mut RECT, list: LPARAM) -> i32 {
        unsafe { (*(list as *mut Vec<RECT>)).push(*rect) };
        1
    }
    let mut list: Vec<RECT> = Vec::new();
    unsafe { EnumDisplayMonitors(null_mut(), null(), Some(add), &mut list as *mut _ as LPARAM) };
    list
}

/// Where the screensaver keeps its own files: logs, where the slideshow got to, fallback caches.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PSS_DATA_DIR") {
        return dir.into();
    }
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("photo-stamp-screensaver")
}

/// photo-stamp's towns database, installed beside the uninstaller.
fn towns_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PSS_TOWNS") {
        return Some(p.into());
    }
    let dir = std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles"))?;
    Some(PathBuf::from(dir).join(r"Photo Stamp Screensaver\towns.sqlite")).filter(|p| p.is_file())
}

/// A photo ready to go on screen.
struct Prepared {
    decoded: Decoded,
    caption: Vec<String>,
}

type Status = Arc<Mutex<Option<String>>>;

/// Opens the library off the UI thread, then keeps each monitor's next photo coming.
fn start_loading(
    settings: Settings,
    preview: bool,
    screens: Vec<((u32, u32), SyncSender<Prepared>)>,
    status: Status,
) {
    std::thread::spawn(move || {
        // The preview mustn't use up shuffled photos before the real thing.
        let library = match Library::open(&settings, &data_dir(), towns_path().as_deref(), !preview) {
            Ok(l) => Arc::new(l),
            Err(message) => {
                crate::log::line(&message);
                *status.lock().unwrap() = Some(message);
                return;
            }
        };
        for (size, tx) in screens {
            let (library, status) = (library.clone(), status.clone());
            std::thread::spawn(move || load(&library, size, &tx, &status));
        }
    });
}

/// One monitor's loader: always the next photo ready, until the screensaver closes.
fn load(library: &Library, size: (u32, u32), tx: &SyncSender<Prepared>, status: &Status) {
    let decoder = match Decoder::new() {
        Ok(d) => d,
        Err(e) => return crate::log::line(&format!("no image decoder: {e}")),
    };
    let mut failures = 0;
    loop {
        let Some(ready) = library.next() else {
            *status.lock().unwrap() = Some("None of the photos could be read.".into());
            return;
        };
        match decoder.decode(&ready, size) {
            Ok(decoded) => {
                failures = 0;
                // Waits here while the previous one is still to be shown; fails once we're closing.
                if tx.send(Prepared { decoded, caption: ready.caption }).is_err() {
                    return;
                }
            }
            Err(e) => {
                crate::log::line(&format!("can't show {}: {e}", ready.entry.path));
                failures += 1;
                if failures >= library.len().min(50) {
                    *status.lock().unwrap() = Some("None of the photos could be shown.".into());
                    return;
                }
            }
        }
    }
}

/// A photo on screen.
struct Shown {
    bitmap: ID2D1Bitmap1,
    size: (u32, u32),
    backdrop: Option<ID2D1Bitmap1>,
    motion: Motion,
    caption: Option<(IDWriteTextLayout, (f32, f32))>,
    since: Instant,
}

struct Screen {
    /// Within the window.
    rect: RECT,
    rx: Receiver<Prepared>,
    current: Option<Shown>,
    previous: Option<Shown>,
    change_at: Instant,
}

impl Screen {
    fn size(&self) -> (u32, u32) {
        ((self.rect.right - self.rect.left).max(1) as u32, (self.rect.bottom - self.rect.top).max(1) as u32)
    }
}

/// What Direct2D draws with; made again if the GPU goes away.
struct Target {
    // Kept alive: the device context is a view of it.
    _target: ID2D1HwndRenderTarget,
    dc: ID2D1DeviceContext,
    white: ID2D1SolidColorBrush,
    black: ID2D1SolidColorBrush,
}

struct Saver {
    settings: Settings,
    hwnd: WinHwnd,
    size: (u32, u32),
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    target: Option<Target>,
    screens: Vec<Screen>,
    status: Status,
    rng: Rng,
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn show(ex_style: u32, style: u32, (x, y, w, h): (i32, i32, i32, i32), parent: HWND, rects: Vec<RECT>) {
    let preview = PREVIEW.get();
    let mut settings = store::load().0;
    // For tests: another folder than the one in the settings.
    if let Some(folder) = std::env::var_os("PSS_FOLDER") {
        settings.folder = folder.to_string_lossy().into_owned();
    }
    let hwnd = unsafe {
        let module = GetModuleHandleW(null());
        let class = wide("PhotoStampScreensaver");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let title = wide("Photo Stamp");
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            parent,
            null_mut(),
            module,
            null(),
        )
    };
    if hwnd.is_null() {
        return;
    }
    crate::log::init(&data_dir());
    let status = Arc::new(Mutex::new(Some(format!("Looking for photos in {}…", settings.folder))));
    let now = Instant::now();
    let seconds = Duration::from_secs(settings.seconds_per_photo as u64);
    let count = rects.len().max(1) as u32;
    let mut senders = Vec::new();
    let screens = rects
        .into_iter()
        .enumerate()
        .map(|(i, rect)| {
            let (tx, rx) = sync_channel(1);
            let size = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
            senders.push((size, tx));
            // Staggered: the monitors change one after another, not all at once.
            Screen { rect, rx, current: None, previous: None, change_at: now + seconds * i as u32 / count }
        })
        .collect();
    start_loading(settings.clone(), preview, senders, status.clone());
    let made = unsafe {
        (
            D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None),
            DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED),
        )
    };
    let (factory, dwrite) = match made {
        (Ok(f), Ok(d)) => (f, d),
        (f, d) => {
            crate::log::line(&format!("no Direct2D: {:?} {:?}", f.err(), d.err()));
            return run_plain();
        }
    };
    let mut saver = Saver {
        settings,
        hwnd: WinHwnd(hwnd),
        size: (w.max(1) as u32, h.max(1) as u32),
        factory,
        dwrite,
        target: None,
        screens,
        status,
        rng: Rng::from_time(),
    };
    loop {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                return;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        saver.frame(Instant::now());
        // Paces the loop to the display; without composition, about 60 a second.
        if unsafe { DwmFlush() }.is_err() {
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}

/// Without Direct2D: a black screen that still closes on input.
fn run_plain() {
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

impl Saver {
    fn seconds(&self) -> Duration {
        Duration::from_secs(self.settings.seconds_per_photo as u64)
    }

    fn make_target(&self) -> windows::core::Result<Target> {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                // Pixels, not DIPs: the manifest makes the window DPI-aware.
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: self.size.0, height: self.size.1 },
                // Paced by DwmFlush instead, once for every monitor.
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            let target = self.factory.CreateHwndRenderTarget(&props, &hwnd_props)?;
            let dc: ID2D1DeviceContext = target.cast()?;
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let white = dc.CreateSolidColorBrush(&color(1.0, 1.0, 1.0, 1.0), None)?;
            let black = dc.CreateSolidColorBrush(&color(0.0, 0.0, 0.0, 1.0), None)?;
            Ok(Target { _target: target, dc, white, black })
        }
    }

    fn frame(&mut self, now: Instant) {
        if self.target.is_none() {
            match self.make_target() {
                Ok(t) => self.target = Some(t),
                Err(e) => {
                    crate::log::line(&format!("can't draw: {e}"));
                    std::thread::sleep(Duration::from_millis(500));
                    return;
                }
            }
        }
        self.advance(now);
        let Some(t) = &self.target else { return };
        let fade = slide::crossfade(self.settings.seconds_per_photo);
        let result = unsafe {
            t.dc.BeginDraw();
            t.dc.SetTransform(&Matrix3x2::identity());
            t.dc.Clear(Some(&color(0.0, 0.0, 0.0, 1.0)));
            for screen in &self.screens {
                let size = screen.size();
                let clip = D2D_RECT_F {
                    left: screen.rect.left as f32,
                    top: screen.rect.top as f32,
                    right: screen.rect.right as f32,
                    bottom: screen.rect.bottom as f32,
                };
                t.dc.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                t.dc.SetTransform(&Matrix3x2::translation(clip.left, clip.top));
                let alpha = screen
                    .current
                    .as_ref()
                    .map_or(0.0, |c| ((now - c.since).as_secs_f32() / fade.as_secs_f32().max(0.01)).min(1.0));
                // The captions take turns: the old one goes in the first half of the crossfade, the
                // new one comes in the second, so they never overlap.
                if let Some(previous) = &screen.previous {
                    self.draw(t, previous, size, 1.0, (1.0 - alpha * 2.0).max(0.0), now);
                }
                match &screen.current {
                    Some(current) => {
                        let caption =
                            if screen.previous.is_some() { (alpha * 2.0 - 1.0).max(0.0) } else { alpha };
                        self.draw(t, current, size, alpha, caption, now);
                    }
                    None => self.draw_status(t, size),
                }
                t.dc.SetTransform(&Matrix3x2::identity());
                t.dc.PopAxisAlignedClip();
            }
            t.dc.EndDraw(None, None)
        };
        if let Err(e) = result {
            crate::log::line(&format!("drawing failed: {e}"));
            if e.code().0 == D2DERR_RECREATE_TARGET {
                // The photos' bitmaps belonged to the old target.
                self.target = None;
                for s in &mut self.screens {
                    s.current = None;
                    s.previous = None;
                }
            }
        }
    }

    /// Brings on each monitor's next photo when it's time and it's ready.
    fn advance(&mut self, now: Instant) {
        let seconds = self.seconds();
        let fade = slide::crossfade(self.settings.seconds_per_photo);
        for i in 0..self.screens.len() {
            let screen = &self.screens[i];
            if screen.current.is_none() || now >= screen.change_at {
                // Not ready yet (a slow network folder): the current photo stays a little longer.
                if let Ok(prepared) = screen.rx.try_recv() {
                    let size = screen.size();
                    match self.shown(prepared, size, now) {
                        Ok(shown) => {
                            let screen = &mut self.screens[i];
                            screen.previous = screen.current.replace(shown);
                            screen.change_at = now.max(screen.change_at) + seconds;
                        }
                        Err(e) => crate::log::line(&format!("can't put a photo on screen: {e}")),
                    }
                }
            }
            let screen = &mut self.screens[i];
            if screen.current.as_ref().is_some_and(|c| now - c.since >= fade) {
                screen.previous = None;
            }
        }
    }

    /// Puts a decoded photo on the GPU, with its pan/zoom and caption.
    fn shown(
        &mut self,
        prepared: Prepared,
        screen: (u32, u32),
        now: Instant,
    ) -> windows::core::Result<Shown> {
        let t = self.target.as_ref().expect("target made first");
        let Prepared { decoded, caption } = prepared;
        let bitmap = upload(&t.dc, &decoded.photo)?;
        let backdrop = decoded.backdrop.as_ref().map(|b| upload(&t.dc, b)).transpose()?;
        let motion = if PREVIEW.get() {
            Motion::still(decoded.fill)
        } else {
            Motion::random(&mut self.rng, decoded.fill)
        };
        let caption = if caption.is_empty() || self.settings.caption == Caption::Hidden {
            None
        } else {
            Some(self.caption_layout(&caption, screen)?)
        };
        Ok(Shown {
            bitmap,
            size: (decoded.photo.width, decoded.photo.height),
            backdrop,
            motion,
            caption,
            since: now,
        })
    }

    /// The caption laid out for this screen, and where it goes.
    fn caption_layout(
        &self,
        lines: &[String],
        screen: (u32, u32),
    ) -> windows::core::Result<(IDWriteTextLayout, (f32, f32))> {
        let short = screen.0.min(screen.1) as f32;
        let margin = short * 0.04;
        let text: Vec<u16> = lines.join("\n").encode_utf16().collect();
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                caption_font(screen),
                w!("en-us"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let right = matches!(self.settings.corner, Corner::BottomRight | Corner::TopRight);
            format.SetTextAlignment(if right {
                DWRITE_TEXT_ALIGNMENT_TRAILING
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            let layout =
                self.dwrite.CreateTextLayout(&text, &format, screen.0 as f32 * 0.6, screen.1 as f32)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            // Shrink the box to the text, so right-aligned lines end at the margin.
            layout.SetMaxWidth(metrics.width)?;
            let origin =
                slide::caption_origin(self.settings.corner, (metrics.width, metrics.height), screen, margin);
            Ok((layout, origin))
        }
    }

    /// `alpha`: the photo's opacity; `caption`: its caption's, during a crossfade.
    fn draw(&self, t: &Target, shown: &Shown, screen: (u32, u32), alpha: f32, caption: f32, now: Instant) {
        let on_screen = self.seconds() + slide::crossfade(self.settings.seconds_per_photo);
        let progress = (now - shown.since).as_secs_f32() / on_screen.as_secs_f32();
        unsafe {
            if let Some(backdrop) = &shown.backdrop {
                // Fill the screen with the tiny copy, stretched soft, then dim it.
                let size = backdrop.GetPixelSize();
                let (l, top, r, b) = Motion::still(true).place((size.width, size.height), screen, 0.0);
                let rect = D2D_RECT_F { left: l, top, right: r, bottom: b };
                t.dc.DrawBitmap(
                    backdrop,
                    Some(&rect),
                    alpha,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
                t.black.SetOpacity(alpha * 0.45);
                let all = D2D_RECT_F { left: 0.0, top: 0.0, right: screen.0 as f32, bottom: screen.1 as f32 };
                t.dc.FillRectangle(&all, &t.black);
            }
            let (l, top, r, b) = shown.motion.place(shown.size, screen, progress);
            let rect = D2D_RECT_F { left: l, top, right: r, bottom: b };
            t.dc.DrawBitmap(&shown.bitmap, Some(&rect), alpha, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            if let Some((layout, origin)) = &shown.caption {
                let opacity = caption
                    * slide::caption_opacity(self.settings.caption, now - shown.since, self.seconds());
                if opacity > 0.0 {
                    draw_caption(t, layout, *origin, opacity, caption_font(screen));
                }
            }
        }
    }

    fn draw_status(&self, t: &Target, screen: (u32, u32)) {
        let Some(message) = self.status.lock().unwrap().clone() else { return };
        let preview = PREVIEW.get();
        let text: Vec<u16> = if preview { "Photo Stamp".into() } else { message }.encode_utf16().collect();
        let size = screen.0.min(screen.1) as f32 / if preview { 8.0 } else { 30.0 };
        unsafe {
            let Ok(format) = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size.max(8.0),
                w!("en-us"),
            ) else {
                return;
            };
            let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let Ok(layout) =
                self.dwrite.CreateTextLayout(&text, &format, screen.0 as f32 * 0.8, screen.1 as f32)
            else {
                return;
            };
            t.white.SetOpacity(0.6);
            let at = Vector2 { X: screen.0 as f32 * 0.1, Y: 0.0 };
            t.dc.DrawTextLayout(at, &layout, &t.white, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }
}

/// The caption's text size: as photo-stamp sizes its caption, a little smaller for a screen seen
/// from afar.
fn caption_font(screen: (u32, u32)) -> f32 {
    (screen.0.min(screen.1) as f32 * 0.032).max(9.0)
}

/// White text with a dark outline and a soft shadow, like photo-stamp's, so it reads on skies, snow
/// and night shots alike.
unsafe fn draw_caption(t: &Target, layout: &IDWriteTextLayout, (x, y): (f32, f32), opacity: f32, font: f32) {
    let outline = (font * 0.07).max(1.0);
    let shadow = font * 0.09;
    unsafe {
        t.black.SetOpacity(opacity * 0.35);
        for (dx, dy) in [(shadow, shadow), (shadow * 0.6, shadow * 1.2)] {
            t.dc.DrawTextLayout(
                Vector2 { X: x + dx, Y: y + dy },
                layout,
                &t.black,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        t.black.SetOpacity(opacity * 0.8);
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            let at = Vector2 { X: x + a.cos() * outline, Y: y + a.sin() * outline };
            t.dc.DrawTextLayout(at, layout, &t.black, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        t.white.SetOpacity(opacity);
        t.dc.DrawTextLayout(Vector2 { X: x, Y: y }, layout, &t.white, D2D1_DRAW_TEXT_OPTIONS_NONE);
    }
}

fn upload(dc: &ID2D1DeviceContext, pixels: &Pixels) -> windows::core::Result<ID2D1Bitmap1> {
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

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Only the real screensaver closes on input and hides the mouse.
    let preview = PREVIEW.get() || WINDOWED.get();
    match msg {
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        WM_SETCURSOR if !preview => {
            unsafe { SetCursor(null_mut()) };
            return 1;
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN if !preview => close(),
        WM_ACTIVATEAPP if !preview && wparam == 0 => close(),
        WM_MOUSEMOVE if !preview => {
            let at = ((lparam & 0xffff) as i16 as i32, ((lparam >> 16) & 0xffff) as i16 as i32);
            match MOUSE_START.get() {
                None => MOUSE_START.set(Some(at)),
                Some(start)
                    if (at.0 - start.0).abs() > MOUSE_SLACK || (at.1 - start.1).abs() > MOUSE_SLACK =>
                {
                    close()
                }
                Some(_) => {}
            }
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn close() {
    unsafe { PostQuitMessage(0) };
}
