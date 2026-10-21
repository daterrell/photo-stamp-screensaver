//! The screensaver windows. For now a placeholder: a black screen across every monitor that says
//! which folder will be shown, closing on any input like a screensaver should. The slideshow comes
//! next.

use std::cell::Cell;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BLACK_BRUSH, BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint,
    FW_NORMAL, FillRect, GetStockObject, HBRUSH, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, MSG, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursor, TranslateMessage, WM_ACTIVATEAPP,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WM_SYSKEYDOWN, WNDCLASSW, WS_CHILD, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use super::{store, wide};

/// How far the mouse may drift (a desk bump) before the screensaver closes, in pixels.
const MOUSE_SLACK: i32 = 8;

thread_local! {
    static PREVIEW: Cell<bool> = const { Cell::new(false) };
    /// Where the mouse was first seen.
    static MOUSE_START: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

/// `/s`: fullscreen across every monitor, until there's input.
pub fn run() {
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    open(WS_EX_TOPMOST, WS_POPUP | WS_VISIBLE, (x, y, w, h), null_mut());
}

/// `/p HWND`: inside the little monitor in Screen Saver Settings, until it closes.
pub fn preview(parent: isize) {
    PREVIEW.set(true);
    let parent = parent as HWND;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(parent, &mut rect) };
    open(0, WS_CHILD | WS_VISIBLE, (0, 0, rect.right, rect.bottom), parent);
}

fn open(ex_style: u32, style: u32, (x, y, w, h): (i32, i32, i32, i32), parent: HWND) {
    unsafe {
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
        let hwnd = CreateWindowExW(
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
        );
        if hwnd.is_null() {
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let preview = PREVIEW.get();
    match msg {
        WM_PAINT => {
            paint(hwnd, preview);
            return 0;
        }
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

fn paint(hwnd: HWND, preview: bool) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut client);
        FillRect(dc, &client, GetStockObject(BLACK_BRUSH) as HBRUSH);

        // The text goes in the middle of the primary monitor, whose top-left is (0, 0) on the
        // desktop; the window starts at the top-left of all monitors.
        let mut area = if preview {
            client
        } else {
            let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            RECT {
                left: -x,
                top: -y,
                right: -x + GetSystemMetrics(SM_CXSCREEN),
                bottom: -y + GetSystemMetrics(SM_CYSCREEN),
            }
        };
        let text = if preview {
            "Photo Stamp".to_owned()
        } else {
            let folder = store::load().0.folder;
            let photos = if folder.is_empty() {
                "No photo folder is chosen yet: open the screensaver's settings.".to_owned()
            } else {
                format!("Photos from {folder}")
            };
            format!("Photo Stamp\n\n{photos}\n\nThe slideshow comes in the next version.")
        };
        let height = (area.bottom - area.top) / if preview { 8 } else { 30 };
        let face = wide("Segoe UI");
        let font = CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        );
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00a0a0a0);
        let mut text = wide(&text);
        let len = text.len() as i32 - 1;
        let flags = DT_CENTER | DT_WORDBREAK | DT_NOPREFIX;
        // Measure, then center vertically.
        let mut measured = area;
        DrawTextW(dc, text.as_mut_ptr(), len, &mut measured, flags | DT_CALCRECT);
        let offset = ((area.bottom - area.top) - (measured.bottom - measured.top)) / 2;
        area.top += offset.max(0);
        DrawTextW(dc, text.as_mut_ptr(), len, &mut area, flags);
        SelectObject(dc, old);
        DeleteObject(font);
        EndPaint(hwnd, &ps);
    }
}
