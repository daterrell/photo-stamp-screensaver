//! The caption editor (`photo-stamp-captions.exe`): the photos of the screensaver's folder (or
//! another, or just those dropped on it) as thumbnails on the left; on the right the chosen one as
//! the screensaver shows it, its caption drawn the same way, and a box to rewrite the caption in,
//! one line a row. Saving keeps it in the photo and in the folder's cache (`Captions::save`).
//!
//! The UI thread only draws and handles input. A worker reads, looks up and decodes the chosen
//! photo and saves edits; another makes thumbnails, newest asked for first. Their results come
//! back as window messages carrying a `Box`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
};
use windows::core::{HSTRING, Interface, w};
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, COLOR_WINDOW, CreateDIBSection, CreateFontIndirectW,
    DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, GetSysColorBrush, HBITMAP, HBRUSH, HDC,
    HFONT, HGDIOBJ, InvalidateRect, ReleaseDC, SetBkMode, TRANSPARENT, ValidateRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    EM_GETMODIFY, EM_SETMODIFY, HIMAGELIST, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES, ILC_COLOR32,
    INITCOMMONCONTROLSEX, ImageList_Add, ImageList_Create, ImageList_Destroy, ImageList_Replace,
    InitCommonControlsEx, LVIF_IMAGE, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_ENSUREVISIBLE,
    LVM_REDRAWITEMS, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETICONSPACING, LVM_SETIMAGELIST, LVM_SETITEMCOUNT,
    LVM_SETITEMSTATE, LVN_GETDISPINFOW, LVN_ITEMCHANGED, LVS_AUTOARRANGE, LVS_EX_DOUBLEBUFFER, LVS_ICON,
    LVS_OWNERDATA, LVS_SHOWSELALWAYS, LVS_SINGLESEL, LVSIL_NORMAL, NMHDR, NMLISTVIEW, NMLVDISPINFOW,
    SetWindowTheme, WC_LISTVIEWW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, SetFocus, VK_CONTROL,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN,
    GetClientRect, GetMessageW, GetWindowTextLengthW, GetWindowTextW, HMENU, ICON_BIG, ICON_SMALL, IDC_ARROW,
    IDNO, IDYES, IMAGE_ICON, IsDialogMessageW, LR_DEFAULTCOLOR, LoadCursorW, LoadImageW, MB_ICONQUESTION,
    MB_YESNO, MB_YESNOCANCEL, MINMAXINFO, MSG, MessageBoxW, NONCLIENTMETRICSW, PostMessageW, PostQuitMessage,
    RegisterClassW, SPI_GETNONCLIENTMETRICS, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageW,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_ERASEBKGND, WM_GETMINMAXINFO,
    WM_KEYDOWN, WM_NOTIFY, WM_PAINT, WM_SETFONT, WM_SETICON, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_ACCEPTFILES, WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

use super::decode::{Decoder, Pixels};
use super::draw::{self, color, upload};
use super::saver::{data_dir, towns_path};
use super::{folder, resource, store, wide};
use crate::exif::Info;
use crate::library::{Captions, Ready, Source};
use crate::photos::{self, Photo};
use crate::playlist::Entry;
use crate::settings::Settings;

const TITLE: &str = "Photo Stamp captions";

/// A chosen photo, read and decoded: `Box<Loaded>`.
const WM_LOADED: u32 = WM_APP + 1;
/// An edit kept: `Box<SavedMsg>`.
const WM_SAVED: u32 = WM_APP + 2;
/// A thumbnail: `Box<Thumb>`.
const WM_THUMB: u32 = WM_APP + 3;
/// The photos to list: `Box<Listed>`.
const WM_LISTED: u32 = WM_APP + 4;

const ID_LIST: i32 = 100;
const ID_FOLDER: i32 = 101;
const ID_WHERE: i32 = 102;
const ID_NAME: i32 = 103;
const ID_EDIT: i32 = 104;
const ID_SAVE: i32 = 105;
const ID_RESET: i32 = 106;
const ID_STATUS: i32 = 107;
const ID_HINT: i32 = 108;
const ID_PREVIEW: i32 = 109;

/// Thumbnails' side at 96 DPI.
const THUMB: i32 = 112;
/// Thumbnails kept at once; older ones are made again when scrolled back to.
const THUMB_SLOTS: usize = 400;
/// Thumbnails waiting to be made: beyond this, those asked for longest ago (scrolled past) go.
const THUMB_QUEUE: usize = 200;

/// Owns a `Box<T>` posted to the window; dropped if the window has gone.
fn post<T>(hwnd: isize, msg: u32, value: T) {
    let raw = Box::into_raw(Box::new(value));
    if unsafe { PostMessageW(hwnd as HWND, msg, 0, raw as isize) } == 0 {
        drop(unsafe { Box::from_raw(raw) });
    }
}

// ---- Work off the UI thread ----

enum Job {
    /// Read, caption and decode a photo for a preview of `size` pixels.
    Load { list: u64, index: usize, photo: Photo, size: (u32, u32) },
    /// Keep `lines` (`None`: back to automatic) for a photo.
    Save { list: u64, index: usize, photo: Photo, lines: Option<Vec<String>> },
}

struct Loaded {
    list: u64,
    index: usize,
    /// `Err`: why it can't be shown.
    pixels: Result<Pixels, String>,
    lines: Vec<String>,
    source: Source,
    automatic: Vec<String>,
}

struct SavedMsg {
    list: u64,
    index: usize,
    message: String,
    /// The photo's caption is edited now (not back to automatic, and kept somewhere).
    edited: bool,
}

struct Thumb {
    list: u64,
    index: usize,
    pixels: Pixels,
}

struct Listed {
    photos: Vec<Photo>,
    title: String,
}

/// Captions for each top folder, opened as photos from it come up.
struct Roots {
    settings: Settings,
    open: HashMap<(PathBuf, bool), Arc<Captions>>,
}

impl Roots {
    fn get(&mut self, photo: &Photo) -> Arc<Captions> {
        let key = (photo.root.clone(), photo.cached);
        let settings = &self.settings;
        self.open
            .entry(key)
            .or_insert_with(|| {
                Arc::new(Captions::open(
                    &photo.root,
                    settings,
                    &data_dir(),
                    towns_path().as_deref(),
                    photo.cached,
                ))
            })
            .clone()
    }
}

/// The worker: one job at a time; of the photos to load, only the newest matters.
fn work(hwnd: isize, jobs: Receiver<Job>, settings: Settings) {
    let decoder = Decoder::new().map_err(|e| crate::log::line(&format!("no image decoder: {e}"))).ok();
    let mut roots = Roots { settings, open: HashMap::new() };
    let mut pending: VecDeque<Job> = VecDeque::new();
    loop {
        if pending.is_empty() {
            match jobs.recv() {
                Ok(job) => pending.push_back(job),
                Err(_) => return,
            }
        }
        pending.extend(jobs.try_iter());
        // Saves in order; a load only if no newer load is waiting.
        let job = pending.pop_front().unwrap();
        if matches!(job, Job::Load { .. }) && pending.iter().any(|j| matches!(j, Job::Load { .. })) {
            continue;
        }
        match job {
            Job::Load { list, index, photo, size } => {
                let captions = roots.get(&photo);
                let loaded = load(&captions, decoder.as_ref(), list, index, &photo, size);
                post(hwnd, WM_LOADED, loaded);
            }
            Job::Save { list, index, photo, lines } => {
                let captions = roots.get(&photo);
                let saved = captions.save(&photo.rel, &photo.file, lines.as_deref());
                let edited = lines.is_some() && (saved.in_photo.is_ok() || saved.in_cache.is_ok());
                crate::log::line(&format!("{}: {saved:?}", photo.file.display()));
                post(
                    hwnd,
                    WM_SAVED,
                    SavedMsg { list, index, message: saved.message(lines.is_none()), edited },
                );
            }
        }
    }
}

fn ready(photo: &Photo, bytes: Vec<u8>, info: Info) -> Ready {
    let meta = std::fs::metadata(&photo.file).ok();
    let entry = Entry {
        path: photo.rel.clone(),
        size: meta.as_ref().map_or(0, |m| m.len()),
        mtime: meta.as_ref().map_or(0, crate::playlist::mtime),
    };
    Ready { entry, bytes, info, caption: Vec::new() }
}

fn load(
    captions: &Captions,
    decoder: Option<&Decoder>,
    list: u64,
    index: usize,
    photo: &Photo,
    size: (u32, u32),
) -> Loaded {
    let failed = |why: String| Loaded {
        list,
        index,
        pixels: Err(why),
        lines: Vec::new(),
        source: Source::Automatic,
        automatic: Vec::new(),
    };
    let bytes = match std::fs::read(&photo.file) {
        Ok(b) => b,
        Err(e) => return failed(format!("Can't read this photo: {e}")),
    };
    let meta = std::fs::metadata(&photo.file).ok();
    let info = captions.info(
        &photo.rel,
        meta.as_ref().map_or(0, |m| m.len()),
        meta.as_ref().map_or(0, crate::playlist::mtime),
        &bytes,
    );
    let (lines, source) = captions.shown(&photo.rel, &info);
    let automatic = if info.stamped { Vec::new() } else { captions.automatic(&info) };
    let ready = ready(photo, bytes, info);
    let pixels = match decoder {
        Some(d) => d.decode(&ready, size).map(|d| d.photo).map_err(|e| format!("Can't show this photo: {e}")),
        None => Err("Can't show photos on this PC.".into()),
    };
    Loaded { list, index, pixels, lines, source, automatic }
}

/// Thumbnails asked for, newest last.
#[derive(Default)]
struct ThumbQueue {
    list: u64,
    wanted: VecDeque<(usize, PathBuf)>,
}

type Thumbs = Arc<(Mutex<ThumbQueue>, Condvar)>;

fn make_thumbs(hwnd: isize, queue: Thumbs, side: Arc<std::sync::atomic::AtomicU32>) {
    unsafe {
        let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let decoder = Decoder::new().ok();
    loop {
        let (list, index, file) = {
            let (lock, ready) = &*queue;
            let mut q = lock.lock().unwrap();
            loop {
                if let Some((i, f)) = q.wanted.pop_back() {
                    break (q.list, i, f);
                }
                q = ready.wait(q).unwrap();
            }
        };
        let side = side.load(std::sync::atomic::Ordering::Relaxed);
        let pixels =
            shell_thumbnail(&file, side).or_else(|| decoded_thumbnail(decoder.as_ref()?, &file, side));
        let pixels = pixels.map(|p| fit(&p, side)).unwrap_or_else(|| blank(side));
        post(hwnd, WM_THUMB, Thumb { list, index, pixels });
    }
}

/// Windows' own thumbnail (cached by Explorer, so quick), as premultiplied BGRA.
fn shell_thumbnail(file: &Path, side: u32) -> Option<Pixels> {
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(file.as_os_str()), None).ok()?;
        let size = windows::Win32::Foundation::SIZE { cx: side as i32, cy: side as i32 };
        let bitmap = factory.GetImage(size, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let hbm = bitmap.0 as HBITMAP;
        let pixels = bitmap_pixels(hbm);
        DeleteObject(hbm as HGDIOBJ);
        pixels
    }
}

unsafe fn bitmap_pixels(hbm: HBITMAP) -> Option<Pixels> {
    unsafe {
        let mut info: BITMAP = std::mem::zeroed();
        if GetObjectW(hbm as HGDIOBJ, size_of::<BITMAP>() as i32, &mut info as *mut _ as *mut _) == 0 {
            return None;
        }
        let (width, height) = (info.bmWidth.max(1) as u32, info.bmHeight.unsigned_abs().max(1));
        let mut header = bitmap_info(width as i32, height as i32);
        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        let dc = GetDC(null_mut());
        let rows = GetDIBits(dc, hbm, 0, height, bgra.as_mut_ptr().cast(), &mut header, DIB_RGB_COLORS);
        ReleaseDC(null_mut(), dc);
        if rows == 0 {
            return None;
        }
        // A photo's thumbnail often comes without alpha: opaque, then.
        if bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            bgra.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        }
        Some(Pixels { width, height, bgra })
    }
}

/// For photos Windows can't make a thumbnail of (HEIC without Microsoft's extension).
fn decoded_thumbnail(decoder: &Decoder, file: &Path, side: u32) -> Option<Pixels> {
    let bytes = std::fs::read(file).ok()?;
    let name = file.file_name()?.to_string_lossy().into_owned();
    let info = Info::from_file_bytes(&bytes);
    // libheif turns HEIF upright itself.
    #[cfg(feature = "heif")]
    let info = Info { orientation: if crate::heif::is_heif(&name) { 1 } else { info.orientation }, ..info };
    let photo =
        Photo { file: file.to_owned(), root: PathBuf::new(), rel: name, cached: false, edited: false };
    decoder.decode(&ready(&photo, bytes, info), (side, side)).ok().map(|d| d.photo)
}

/// `pixels` shrunk to fit a `side` square, centered; transparent around it.
fn fit(pixels: &Pixels, side: u32) -> Pixels {
    let scale = (side as f32 / pixels.width as f32).min(side as f32 / pixels.height as f32).min(1.0);
    let (w, h) =
        (((pixels.width as f32 * scale) as u32).max(1), ((pixels.height as f32 * scale) as u32).max(1));
    let (x0, y0) = ((side - w.min(side)) / 2, (side - h.min(side)) / 2);
    let mut out = blank(side);
    for y in 0..h.min(side) {
        let sy = ((y as f32 / scale) as u32).min(pixels.height - 1);
        for x in 0..w.min(side) {
            let sx = ((x as f32 / scale) as u32).min(pixels.width - 1);
            let from = (sy * pixels.width + sx) as usize * 4;
            let to = ((y0 + y) * side + x0 + x) as usize * 4;
            out.bgra[to..to + 4].copy_from_slice(&pixels.bgra[from..from + 4]);
        }
    }
    out
}

fn blank(side: u32) -> Pixels {
    Pixels { width: side, height: side, bgra: vec![0; side as usize * side as usize * 4] }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Top-down.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { std::mem::zeroed() }
        },
        ..unsafe { std::mem::zeroed() }
    }
}

/// A GDI bitmap of `pixels`, for the image list.
fn gdi_bitmap(pixels: &Pixels) -> HBITMAP {
    unsafe {
        let info = bitmap_info(pixels.width as i32, pixels.height as i32);
        let mut bits = null_mut();
        let hbm = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !hbm.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.bgra.as_ptr(), bits.cast(), pixels.bgra.len());
        }
        hbm
    }
}

// ---- The window ----

/// The thumbnails on hand: which photo each image-list slot shows, oldest first.
struct Slots {
    images: HIMAGELIST,
    side: u32,
    of: HashMap<usize, i32>,
    order: VecDeque<usize>,
}

/// The chosen photo's preview and its caption.
#[derive(Default)]
struct Preview {
    pixels: Option<Pixels>,
    bitmap: Option<ID2D1Bitmap1>,
    message: Option<String>,
    lines: Vec<String>,
    stamped: bool,
}

struct Gfx {
    target: ID2D1HwndRenderTarget,
    dc: ID2D1DeviceContext,
    white: ID2D1SolidColorBrush,
    black: ID2D1SolidColorBrush,
}

/// Everything the window keeps. Borrows are kept short: messages sent to the controls come back
/// into the window procedure.
struct App {
    hwnd: HWND,
    list: HWND,
    preview: HWND,
    edit: HWND,
    settings: Settings,
    font: Cell<HFONT>,
    /// The control that had the focus when the window was last switched away from.
    focus: Cell<Option<HWND>>,
    photos: RefCell<Vec<Photo>>,
    list_id: Cell<u64>,
    /// The photo in the editor, once loaded.
    current: Cell<Option<usize>>,
    /// What's shown for it: its caption's source, and the automatic one.
    loaded: RefCell<Option<(Source, Vec<String>)>>,
    jobs: Sender<Job>,
    thumbs: Thumbs,
    thumb_side: Arc<std::sync::atomic::AtomicU32>,
    slots: RefCell<Slots>,
    shown: RefCell<Preview>,
    factory: Option<(ID2D1Factory1, IDWriteFactory)>,
    gfx: RefCell<Option<Gfx>>,
}

thread_local! {
    static APP: RefCell<Option<std::rc::Rc<App>>> = const { RefCell::new(None) };
}

fn app() -> Option<std::rc::Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

fn control(parent: HWND, class: *const u16, text: &str, style: u32, id: i32) -> HWND {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            0,
            class,
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            parent,
            id as isize as HMENU,
            GetModuleHandleW(null()),
            null(),
        )
    }
}

fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
}

fn text_of(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn child(hwnd: HWND, id: i32) -> HWND {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, id) }
}

/// The photos given on the command line (dropped on the program or its shortcut), else the
/// screensaver's folder.
pub fn run(args: Vec<PathBuf>) {
    let data = data_dir();
    crate::log::init_named(&data, "captions.log");
    let mut settings = store::load().0;
    if let Some(folder) = std::env::var_os("PSS_FOLDER") {
        settings.folder = folder.to_string_lossy().into_owned();
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_LISTVIEW_CLASSES,
        };
        InitCommonControlsEx(&controls);
    }
    let Some(hwnd) = create_window() else { return };
    let factory = unsafe {
        match (
            D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None),
            DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED),
        ) {
            (Ok(f), Ok(d)) => Some((f, d)),
            (f, d) => {
                crate::log::line(&format!("no Direct2D: {:?} {:?}", f.err(), d.err()));
                None
            }
        }
    };
    let (jobs, rx) = channel();
    let thumbs: Thumbs = Default::default();
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let thumb_side = Arc::new(std::sync::atomic::AtomicU32::new(scale(THUMB, dpi) as u32));
    {
        let (settings, thumbs, side, h) =
            (settings.clone(), thumbs.clone(), thumb_side.clone(), hwnd as isize);
        std::thread::spawn(move || work(h, rx, settings));
        std::thread::spawn(move || make_thumbs(h, thumbs, side));
    }
    let list = child(hwnd, ID_LIST);
    let side = thumb_side.load(std::sync::atomic::Ordering::Relaxed);
    let app = App {
        hwnd,
        list,
        preview: child(hwnd, ID_PREVIEW),
        edit: child(hwnd, ID_EDIT),
        settings: settings.clone(),
        font: Cell::new(null_mut()),
        focus: Cell::new(None),
        photos: RefCell::new(Vec::new()),
        list_id: Cell::new(0),
        current: Cell::new(None),
        loaded: RefCell::new(None),
        jobs,
        thumbs,
        thumb_side,
        slots: RefCell::new(Slots {
            images: image_list(side),
            side,
            of: HashMap::new(),
            order: VecDeque::new(),
        }),
        shown: RefCell::new(Preview::default()),
        factory,
        gfx: RefCell::new(None),
    };
    unsafe {
        SendMessageW(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, app.slots.borrow().images as isize)
    };
    APP.with(|a| *a.borrow_mut() = Some(std::rc::Rc::new(app)));
    restyle(hwnd);
    layout(hwnd);
    set_editing(false);
    if args.is_empty() {
        if settings.folder.is_empty() {
            set_text(child(hwnd, ID_WHERE), "Drag photos or a folder here, or choose a folder.");
            show_hint(true);
        } else {
            start_listing(vec![PathBuf::from(&settings.folder)], settings.folder.clone());
        }
    } else {
        let title = if args.len() == 1 {
            args[0].display().to_string()
        } else {
            format!("{} items dropped", args.len())
        };
        start_listing(args, title);
    }
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        // Ctrl+S saves, wherever the focus is.
        if msg.message == WM_KEYDOWN
            && msg.wParam == 'S' as usize
            && unsafe { GetKeyState(VK_CONTROL as i32) } < 0
        {
            save(false);
            continue;
        }
        unsafe {
            if IsDialogMessageW(hwnd, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|a| *a.borrow_mut() = None);
}

fn scale(px: i32, dpi: u32) -> i32 {
    px * dpi as i32 / 96
}

fn image_list(side: u32) -> HIMAGELIST {
    unsafe {
        let images = ImageList_Create(side as i32, side as i32, ILC_COLOR32, 64, 64);
        // Slot 0: the placeholder while a thumbnail is made.
        let mut placeholder = blank(side);
        placeholder.bgra.as_chunks_mut::<4>().0.fill([0xe8, 0xe8, 0xe8, 0xff]);
        let hbm = gdi_bitmap(&placeholder);
        ImageList_Add(images, hbm, null_mut());
        DeleteObject(hbm as HGDIOBJ);
        images
    }
}

fn create_window() -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(null());
        let icon = LoadImageW(module, resource(1), IMAGE_ICON, 0, 0, LR_DEFAULTCOLOR);
        let class = wide("PhotoStampCaptions");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let preview_class = wide("PhotoStampCaptionsPreview");
        let pc = WNDCLASSW {
            lpfnWndProc: Some(preview_proc),
            hInstance: module,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: preview_class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&pc);
        let title = wide(TITLE);
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            scale(1100, dpi),
            scale(720, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
        );
        let big = LoadImageW(module, resource(1), IMAGE_ICON, 32, 32, LR_DEFAULTCOLOR);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        DragAcceptFiles(hwnd, 1);

        control(
            hwnd,
            wide("BUTTON").as_ptr(),
            "&Choose a folder...",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            ID_FOLDER,
        );
        control(
            hwnd,
            wide("STATIC").as_ptr(),
            "",
            0x0100 /* SS_NOTIFY */ | 0x4000, /* SS_PATHELLIPSIS */
            ID_WHERE,
        );
        let list = control(
            hwnd,
            WC_LISTVIEWW,
            "",
            LVS_ICON
                | LVS_OWNERDATA
                | LVS_SINGLESEL
                | LVS_SHOWSELALWAYS
                | LVS_AUTOARRANGE
                | WS_TABSTOP
                | WS_BORDER,
            ID_LIST,
        );
        SetWindowTheme(list, w!("Explorer").as_ptr(), null());
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_DOUBLEBUFFER as usize,
            LVS_EX_DOUBLEBUFFER as isize,
        );
        control(
            hwnd,
            wide("STATIC").as_ptr(),
            "Drag photos or folders of photos here.\nOr choose a photo on the left.",
            0x0001, /* SS_CENTER */
            ID_HINT,
        );
        CreateWindowExW(
            0,
            preview_class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            ID_PREVIEW as isize as HMENU,
            module,
            null(),
        );
        control(hwnd, wide("STATIC").as_ptr(), "", 0x4000 /* SS_PATHELLIPSIS */, ID_NAME);
        control(
            hwnd,
            wide("EDIT").as_ptr(),
            "",
            ES_MULTILINE as u32
                | ES_WANTRETURN as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL
                | WS_BORDER
                | WS_TABSTOP,
            ID_EDIT,
        );
        control(hwnd, wide("BUTTON").as_ptr(), "&Save", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, ID_SAVE);
        control(
            hwnd,
            wide("BUTTON").as_ptr(),
            "Back to &automatic",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            ID_RESET,
        );
        control(hwnd, wide("STATIC").as_ptr(), "", 0, ID_STATUS);
        Some(hwnd)
    }
}

/// The message font at the window's DPI, on every control.
fn restyle(hwnd: HWND) {
    let Some(app) = app() else { return };
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut _ as *mut _,
            0,
            dpi,
        );
        let font = CreateFontIndirectW(&metrics.lfMessageFont);
        for id in [ID_FOLDER, ID_WHERE, ID_LIST, ID_HINT, ID_NAME, ID_EDIT, ID_SAVE, ID_RESET, ID_STATUS] {
            SendMessageW(child(hwnd, id), WM_SETFONT, font as usize, 1);
        }
        let old = app.font.replace(font);
        if !old.is_null() {
            DeleteObject(old as HGDIOBJ);
        }
        let side = scale(THUMB, dpi) as u32;
        let spacing = (scale(THUMB + 24, dpi) & 0xffff) | (scale(THUMB + 44, dpi) << 16);
        SendMessageW(app.list, LVM_SETICONSPACING, 0, spacing as isize);
        if side != app.slots.borrow().side {
            app.thumb_side.store(side, std::sync::atomic::Ordering::Relaxed);
            let images = image_list(side);
            SendMessageW(app.list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, images as isize);
            let mut slots = app.slots.borrow_mut();
            ImageList_Destroy(slots.images);
            *slots = Slots { images, side, of: HashMap::new(), order: VecDeque::new() };
        }
    }
}

/// Places the controls for the window's size.
fn layout(hwnd: HWND) {
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut client) };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let s = |px: i32| scale(px, dpi);
    let (w, h) = (client.right, client.bottom);
    let m = s(10);
    let bar = s(28);
    let left = (w * 2 / 5).max(s(260)).min(w - s(300)).max(s(120));
    let place = |id: i32, x: i32, y: i32, cw: i32, ch: i32| unsafe {
        SetWindowPos(child(hwnd, id), null_mut(), x, y, cw.max(0), ch.max(0), SWP_NOZORDER | SWP_NOACTIVATE);
    };
    place(ID_FOLDER, m, m, s(150), bar);
    place(ID_WHERE, m + s(160), m + s(6), w - s(170) - m * 2, bar - s(6));
    place(ID_LIST, m, m * 2 + bar, left - m, h - bar - m * 3);
    let x = left + m;
    let rw = w - x - m;
    let button = s(28);
    let status = s(36);
    let edit = s(84);
    let name = s(22);
    let bottom = h - m;
    let status_y = bottom - status;
    let buttons_y = status_y - m / 2 - button;
    let edit_y = buttons_y - m / 2 - edit;
    let name_y = edit_y - m / 2 - name;
    let preview_y = m * 2 + bar;
    place(ID_HINT, x, preview_y + (name_y - preview_y) / 2 - s(20), rw, s(40));
    place(ID_PREVIEW, x, preview_y, rw, name_y - m / 2 - preview_y);
    place(ID_NAME, x, name_y + s(4), rw, name - s(4));
    place(ID_EDIT, x, edit_y, rw, edit);
    place(ID_SAVE, x, buttons_y, s(100), button);
    place(ID_RESET, x + s(110), buttons_y, s(150), button);
    place(ID_STATUS, x, status_y, rw, status);
}

fn show_hint(show: bool) {
    let Some(app) = app() else { return };
    unsafe {
        ShowWindow(child(app.hwnd, ID_HINT), if show { SW_SHOW } else { 0 });
        ShowWindow(app.preview, if show { 0 } else { SW_SHOW });
    }
}

fn set_editing(on: bool) {
    let Some(app) = app() else { return };
    unsafe {
        EnableWindow(app.edit, on as i32);
        EnableWindow(child(app.hwnd, ID_SAVE), on as i32);
    }
    let reset =
        on && app.loaded.borrow().as_ref().is_some_and(|(s, _)| matches!(s, Source::Photo | Source::Cache));
    unsafe { EnableWindow(child(app.hwnd, ID_RESET), reset as i32) };
}

/// Lists the photos at `paths` (off the UI thread: a network folder can be slow).
fn start_listing(paths: Vec<PathBuf>, title: String) {
    let Some(app) = app() else { return };
    set_text(child(app.hwnd, ID_WHERE), &format!("Looking for photos in {title}…"));
    let (hwnd, folder) = (app.hwnd as isize, app.settings.folder.clone());
    std::thread::spawn(move || {
        let photos = photos::list(&paths, &folder, &data_dir());
        post(hwnd, WM_LISTED, Listed { photos, title });
    });
}

fn listed(listed: Listed) {
    let Some(app) = app() else { return };
    if !confirm_leaving(false) {
        return;
    }
    let count = listed.photos.len();
    app.list_id.set(app.list_id.get() + 1);
    {
        let (lock, _) = &*app.thumbs;
        let mut q = lock.lock().unwrap();
        q.list = app.list_id.get();
        q.wanted.clear();
    }
    {
        let mut slots = app.slots.borrow_mut();
        slots.of.clear();
        slots.order.clear();
    }
    *app.photos.borrow_mut() = listed.photos;
    app.current.set(None);
    *app.loaded.borrow_mut() = None;
    *app.shown.borrow_mut() = Preview::default();
    set_text(app.edit, "");
    set_text(child(app.hwnd, ID_NAME), "");
    set_text(child(app.hwnd, ID_STATUS), "");
    set_editing(false);
    let where_ = match count {
        0 => format!("No photos in {}.", listed.title),
        1 => format!("1 photo in {}", listed.title),
        n => format!("{n} photos in {}", listed.title),
    };
    set_text(child(app.hwnd, ID_WHERE), &where_);
    unsafe {
        SendMessageW(app.list, LVM_SETITEMCOUNT, count, 0);
        SetFocus(app.list);
    }
    show_hint(true);
    if count > 0 {
        select(0);
    }
}

fn select(index: usize) {
    let Some(app) = app() else { return };
    let mut item: LVITEMW = unsafe { std::mem::zeroed() };
    item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
    item.state = LVIS_SELECTED | LVIS_FOCUSED;
    unsafe {
        SendMessageW(app.list, LVM_SETITEMSTATE, index, &item as *const _ as isize);
        SendMessageW(app.list, LVM_ENSUREVISIBLE, index, 0);
    }
}

/// The photo at `index` goes in the editor.
fn open(index: usize) {
    let Some(app) = app() else { return };
    let Some(photo) = app.photos.borrow().get(index).cloned() else { return };
    app.current.set(Some(index));
    *app.loaded.borrow_mut() = None;
    set_editing(false);
    set_text(child(app.hwnd, ID_NAME), &format!("{} — loading…", photo.rel));
    set_text(child(app.hwnd, ID_STATUS), "");
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(app.preview, &mut rect) };
    let size = (rect.right.max(320) as u32, rect.bottom.max(240) as u32);
    let _ = app.jobs.send(Job::Load { list: app.list_id.get(), index, photo, size });
}

fn loaded(loaded: Loaded) {
    let Some(app) = app() else { return };
    if loaded.list != app.list_id.get() || Some(loaded.index) != app.current.get() {
        return;
    }
    let Some(photo) = app.photos.borrow().get(loaded.index).cloned() else { return };
    let stamped = loaded.source == Source::Stamped;
    let about = match loaded.source {
        Source::Automatic => "the caption the screensaver works out",
        Source::Photo => "a caption edited by hand, kept in the photo",
        Source::Cache => "a caption edited by hand",
        Source::Stamped => "photo-stamp already put the caption into this photo",
    };
    set_text(child(app.hwnd, ID_NAME), &format!("{}: {about}", photo.rel));
    let text = loaded.lines.join("\r\n");
    *app.loaded.borrow_mut() = Some((loaded.source, loaded.automatic));
    {
        let mut shown = app.shown.borrow_mut();
        shown.bitmap = None;
        shown.stamped = stamped;
        match loaded.pixels {
            Ok(p) => {
                shown.pixels = Some(p);
                shown.message = None;
            }
            Err(why) => {
                shown.pixels = None;
                shown.message = Some(why);
            }
        }
        shown.lines = loaded.lines;
    }
    set_text(app.edit, &text);
    unsafe { SendMessageW(app.edit, EM_SETMODIFY, 0, 0) };
    show_hint(false);
    set_editing(!stamped);
    if stamped {
        set_text(
            child(app.hwnd, ID_STATUS),
            "Its caption is part of the picture itself, so the screensaver doesn't add another.",
        );
    } else if loaded.source == Source::Automatic && text.is_empty() {
        set_text(
            child(app.hwnd, ID_STATUS),
            "This photo doesn't say where or when it was taken. Type a caption above.",
        );
    }
    unsafe { InvalidateRect(app.preview, null(), 0) };
}

/// Saves the caption in the box (`reset`: goes back to the automatic one).
fn save(reset: bool) {
    let Some(app) = app() else { return };
    let (Some(index), Some((source, automatic))) = (app.current.get(), app.loaded.borrow().clone()) else {
        return;
    };
    if source == Source::Stamped {
        return;
    }
    let lines = photos::lines(&text_of(app.edit));
    if !reset && source == Source::Automatic && lines == automatic {
        set_text(
            child(app.hwnd, ID_STATUS),
            "That's the automatic caption already; there's nothing to save.",
        );
        unsafe { SendMessageW(app.edit, EM_SETMODIFY, 0, 0) };
        return;
    }
    save_photo(index, (!reset).then_some(lines));
}

fn save_photo(index: usize, lines: Option<Vec<String>>) {
    let Some(app) = app() else { return };
    let Some(photo) = app.photos.borrow().get(index).cloned() else { return };
    set_text(child(app.hwnd, ID_STATUS), "Saving…");
    unsafe { SendMessageW(app.edit, EM_SETMODIFY, 0, 0) };
    let _ = app.jobs.send(Job::Save { list: app.list_id.get(), index, photo, lines });
}

fn saved(saved: SavedMsg) {
    let Some(app) = app() else { return };
    if saved.list != app.list_id.get() {
        return;
    }
    if let Some(p) = app.photos.borrow_mut().get_mut(saved.index) {
        p.edited = saved.edited;
    }
    unsafe { SendMessageW(app.list, LVM_REDRAWITEMS, saved.index, saved.index as isize) };
    if app.current.get() == Some(saved.index) {
        // Shows where it's kept now (and the automatic one again, after a reset).
        open(saved.index);
    }
    set_text(child(app.hwnd, ID_STATUS), &saved.message);
}

/// Before the photo in the editor goes: offers to save a changed caption. `false`: stay (only
/// when `can_stay`).
fn confirm_leaving(can_stay: bool) -> bool {
    let Some(app) = app() else { return true };
    let Some(index) = app.current.get() else { return true };
    if unsafe { SendMessageW(app.edit, EM_GETMODIFY, 0, 0) } == 0 || app.loaded.borrow().is_none() {
        return true;
    }
    let name = app.photos.borrow().get(index).map(|p| p.rel.clone()).unwrap_or_default();
    let question = wide(&format!("Save the caption you changed for {name}?"));
    let title = wide(TITLE);
    let buttons = if can_stay { MB_YESNOCANCEL } else { MB_YESNO };
    match unsafe { MessageBoxW(app.hwnd, question.as_ptr(), title.as_ptr(), buttons | MB_ICONQUESTION) } {
        IDYES => {
            save(false);
            true
        }
        IDNO => {
            unsafe { SendMessageW(app.edit, EM_SETMODIFY, 0, 0) };
            true
        }
        _ => false,
    }
}

/// The list asks what to show for a photo: its name and thumbnail (asked for if not made yet).
fn display_info(info: &mut NMLVDISPINFOW) {
    let Some(app) = app() else { return };
    let index = info.item.iItem as usize;
    let photos = app.photos.borrow();
    let Some(photo) = photos.get(index) else { return };
    if info.item.mask & LVIF_TEXT != 0 && !info.item.pszText.is_null() {
        let name = photo.file.file_name().unwrap_or_default().to_string_lossy();
        let label = if photo.edited { format!("✎ {name}") } else { name.into_owned() };
        let text: Vec<u16> =
            label.encode_utf16().take(info.item.cchTextMax.max(1) as usize - 1).chain([0]).collect();
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), info.item.pszText, text.len()) };
    }
    if info.item.mask & LVIF_IMAGE != 0 {
        let slot = app.slots.borrow().of.get(&index).copied();
        info.item.iImage = slot.unwrap_or_else(|| {
            let (lock, ready) = &*app.thumbs;
            let mut q = lock.lock().unwrap();
            if !q.wanted.iter().any(|(i, _)| *i == index) {
                q.wanted.push_back((index, photo.file.clone()));
                if q.wanted.len() > THUMB_QUEUE {
                    q.wanted.pop_front();
                }
                ready.notify_one();
            }
            0
        });
    }
}

fn thumb(thumb: Thumb) {
    let Some(app) = app() else { return };
    if thumb.list != app.list_id.get() {
        return;
    }
    {
        let mut slots = app.slots.borrow_mut();
        if thumb.pixels.width != slots.side || slots.of.contains_key(&thumb.index) {
            return;
        }
        let hbm = gdi_bitmap(&thumb.pixels);
        let slot = if slots.order.len() >= THUMB_SLOTS {
            let oldest = slots.order.pop_front().unwrap();
            let slot = slots.of.remove(&oldest).unwrap();
            unsafe { ImageList_Replace(slots.images, slot, hbm, null_mut()) };
            slot
        } else {
            unsafe { ImageList_Add(slots.images, hbm, null_mut()) }
        };
        unsafe { DeleteObject(hbm as HGDIOBJ) };
        if slot < 0 {
            return;
        }
        slots.of.insert(thumb.index, slot);
        slots.order.push_back(thumb.index);
    }
    unsafe { SendMessageW(app.list, LVM_REDRAWITEMS, thumb.index, thumb.index as isize) };
}

fn dropped(drop: HDROP) {
    let mut paths = Vec::new();
    unsafe {
        let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
        for i in 0..count {
            let len = DragQueryFileW(drop, i, null_mut(), 0);
            let mut buf = vec![0u16; len as usize + 1];
            DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
        }
        DragFinish(drop);
    }
    if paths.is_empty() {
        return;
    }
    let title = if paths.len() == 1 {
        paths[0].display().to_string()
    } else {
        format!("the {} items dropped", paths.len())
    };
    start_listing(paths, title);
}

fn choose_folder() {
    let Some(app) = app() else { return };
    let current = app
        .photos
        .borrow()
        .first()
        .map(|p| p.root.display().to_string())
        .unwrap_or(app.settings.folder.clone());
    if let Some(folder) = folder::pick(app.hwnd, &current) {
        start_listing(vec![PathBuf::from(&folder)], folder);
    }
}

// ---- The preview ----

fn make_gfx(app: &App) -> windows::core::Result<Gfx> {
    let (factory, _) = app
        .factory
        .as_ref()
        .ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005u32 as i32)))?;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(app.preview, &mut rect) };
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
            hwnd: WinHwnd(app.preview),
            pixelSize: D2D_SIZE_U { width: rect.right.max(1) as u32, height: rect.bottom.max(1) as u32 },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let target = factory.CreateHwndRenderTarget(&props, &hwnd_props)?;
        let dc: ID2D1DeviceContext = target.cast()?;
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let white = dc.CreateSolidColorBrush(&color(1.0, 1.0, 1.0, 1.0), None)?;
        let black = dc.CreateSolidColorBrush(&color(0.0, 0.0, 0.0, 1.0), None)?;
        Ok(Gfx { target, dc, white, black })
    }
}

/// The photo, whole, and its caption as the screensaver would put it on a screen its shape.
fn paint() {
    let Some(app) = app() else { return };
    if app.gfx.borrow().is_none() {
        match make_gfx(&app) {
            Ok(g) => *app.gfx.borrow_mut() = Some(g),
            Err(e) => {
                crate::log::line(&format!("can't draw the preview: {e}"));
                return;
            }
        }
    }
    let gfx = app.gfx.borrow();
    let g = gfx.as_ref().unwrap();
    let mut shown = app.shown.borrow_mut();
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(app.preview, &mut rect) };
    let (w, h) = (rect.right.max(1) as f32, rect.bottom.max(1) as f32);
    let result = unsafe {
        g.dc.BeginDraw();
        g.dc.SetTransform(&Matrix3x2::identity());
        g.dc.Clear(Some(&color(0.12, 0.12, 0.12, 1.0)));
        if shown.bitmap.is_none()
            && let Some(p) = &shown.pixels
        {
            shown.bitmap = upload(&g.dc, p).ok();
        }
        if let (Some(bitmap), Some(p)) = (&shown.bitmap, &shown.pixels) {
            let s = (w / p.width as f32).min(h / p.height as f32);
            let (pw, ph) = (p.width as f32 * s, p.height as f32 * s);
            let (x, y) = ((w - pw) / 2.0, (h - ph) / 2.0);
            let at = D2D_RECT_F { left: x, top: y, right: x + pw, bottom: y + ph };
            g.dc.DrawBitmap(bitmap, Some(&at), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
            let lines = &shown.lines;
            if !shown.stamped
                && !lines.is_empty()
                && let Some((_, dwrite)) = &app.factory
            {
                let screen = (pw.max(1.0) as u32, ph.max(1.0) as u32);
                if let Ok((layout, origin)) = draw::caption_layout(dwrite, lines, screen, app.settings.corner)
                {
                    g.dc.SetTransform(&Matrix3x2::translation(x, y));
                    draw::caption(
                        &g.dc,
                        &g.white,
                        &g.black,
                        &layout,
                        origin,
                        1.0,
                        draw::caption_font(screen),
                    );
                    g.dc.SetTransform(&Matrix3x2::identity());
                }
            }
        } else if let (Some(message), Some((_, dwrite))) = (&shown.message, &app.factory) {
            draw_message(&g.dc, dwrite, &g.white, message, (w, h));
        }
        g.dc.EndDraw(None, None)
    };
    drop(shown);
    drop(gfx);
    if result.is_err() {
        // The GPU went away: made again next time, with the photo uploaded again.
        *app.gfx.borrow_mut() = None;
        app.shown.borrow_mut().bitmap = None;
    }
}

fn draw_message(
    dc: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    brush: &ID2D1SolidColorBrush,
    message: &str,
    (w, h): (f32, f32),
) {
    let text: Vec<u16> = message.encode_utf16().collect();
    unsafe {
        let Ok(format) = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            (h / 20.0).clamp(12.0, 24.0),
            w!("en-us"),
        ) else {
            return;
        };
        let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let layout: Option<IDWriteTextLayout> = dwrite.CreateTextLayout(&text, &format, w * 0.8, h).ok();
        if let Some(layout) = layout {
            brush.SetOpacity(0.7);
            dc.DrawTextLayout(Vector2 { X: w * 0.1, Y: 0.0 }, &layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            brush.SetOpacity(1.0);
        }
    }
}

unsafe extern "system" fn preview_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint();
            unsafe { ValidateRect(hwnd, null()) };
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            if let Some(app) = app()
                && let Some(g) = app.gfx.borrow().as_ref()
            {
                let size =
                    D2D_SIZE_U { width: (lparam & 0xffff) as u32, height: ((lparam >> 16) & 0xffff) as u32 };
                let _ = unsafe { g.target.Resize(&size) };
            }
            unsafe { InvalidateRect(hwnd, null(), 0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout(hwnd);
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = scale(700, dpi);
            info.ptMinTrackSize.y = scale(480, dpi);
            0
        }
        WM_DPICHANGED => {
            let r = unsafe { &*(lparam as *const RECT) };
            restyle(hwnd);
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            0
        }
        WM_COMMAND => {
            let (id, code) = ((wparam & 0xffff) as i32, ((wparam >> 16) & 0xffff) as u32);
            match (id, code) {
                (ID_FOLDER, BN_CLICKED) => choose_folder(),
                (ID_SAVE, BN_CLICKED) => save(false),
                (ID_RESET, BN_CLICKED) => save(true),
                (ID_EDIT, EN_CHANGE) => {
                    if let Some(app) = app() {
                        app.shown.borrow_mut().lines = photos::lines(&text_of(app.edit));
                        unsafe { InvalidateRect(app.preview, null(), 0) };
                    }
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            let header = unsafe { &*(lparam as *const NMHDR) };
            if header.idFrom as i32 == ID_LIST {
                match header.code {
                    LVN_GETDISPINFOW => display_info(unsafe { &mut *(lparam as *mut NMLVDISPINFOW) }),
                    LVN_ITEMCHANGED => {
                        let change = unsafe { &*(lparam as *const NMLISTVIEW) };
                        if change.uNewState & LVIS_SELECTED != 0 && change.uOldState & LVIS_SELECTED == 0 {
                            let index = change.iItem as usize;
                            if app().and_then(|a| a.current.get()) != Some(index) {
                                confirm_leaving(false);
                                open(index);
                            }
                        }
                    }
                    _ => {}
                }
            }
            0
        }
        WM_DROPFILES => {
            dropped(wparam as HDROP);
            0
        }
        // Labels on the window's own background, not a dialog's gray.
        WM_CTLCOLORSTATIC => unsafe {
            SetBkMode(wparam as HDC, TRANSPARENT as i32);
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        },
        WM_LISTED => {
            listed(*unsafe { Box::from_raw(lparam as *mut Listed) });
            0
        }
        WM_LOADED => {
            loaded(*unsafe { Box::from_raw(lparam as *mut Loaded) });
            0
        }
        WM_SAVED => {
            saved(*unsafe { Box::from_raw(lparam as *mut SavedMsg) });
            0
        }
        WM_THUMB => {
            thumb(*unsafe { Box::from_raw(lparam as *mut Thumb) });
            0
        }
        // Focus goes back where it was when the window is switched back to.
        WM_ACTIVATE => {
            if let Some(app) = app() {
                if wparam & 0xffff == WA_INACTIVE as usize {
                    let focus = unsafe { GetFocus() };
                    app.focus.set((!focus.is_null()).then_some(focus));
                } else {
                    unsafe { SetFocus(app.focus.get().unwrap_or(app.list)) };
                }
            }
            0
        }
        WM_CLOSE => {
            if confirm_leaving(true) {
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            if let Some(app) = app() {
                unsafe { ImageList_Destroy(app.slots.borrow().images) };
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
