//! The settings dialog (`.scr /c`), laid out in `assets/screensaver.rc`: the photo folder, how long
//! each photo shows and in what order, whether and where the caption shows, and the place server's address and device
//! key, with a connection test.

use std::path::Path;
use std::ptr::null_mut;
use std::time::Duration;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, EM_SETCUEBANNER, EM_SETPASSWORDCHAR, ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, UDM_SETPOS32, UDM_SETRANGE32,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, DialogBoxParamW, EndDialog,
    GetDlgItem, GetDlgItemInt, GetWindowTextLengthW, GetWindowTextW, ICON_BIG, ICON_SMALL, IDCANCEL, IDOK,
    IDYES, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO,
    MessageBoxW, PostMessageW, SendDlgItemMessageW, SendMessageW, SetDlgItemTextW, WM_APP, WM_COMMAND,
    WM_INITDIALOG, WM_NEXTDLGCTL, WM_SETICON,
};

use super::{folder, resource, store, wide};
use crate::server::{self, CheckError, Me};
use crate::settings::{Caption, Corner, Field, MAX_SECONDS, MIN_SECONDS, Order, Settings, normalize_url};

// As in assets/screensaver.rc.
const IDD_SETTINGS: u16 = 101;
const IDC_FOLDER: i32 = 1001;
const IDC_BROWSE: i32 = 1002;
const IDC_SECONDS: i32 = 1003;
const IDC_SECONDS_SPIN: i32 = 1004;
const IDC_URL: i32 = 1005;
const IDC_KEY: i32 = 1006;
const IDC_SHOWKEY: i32 = 1007;
const IDC_TEST: i32 = 1008;
const IDC_STATUS: i32 = 1009;
const IDC_VERSION: i32 = 1010;
const IDC_CAPTION: i32 = 1011;
const IDC_CORNER: i32 = 1012;
const IDC_ORDER: i32 = 1013;

/// Posted by the connection test's thread; `lparam` is a `Box<Result<Me, CheckError>>`.
const WM_TESTED: u32 = WM_APP + 1;
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The device key's dots while hidden.
const PASSWORD_CHAR: usize = '●' as usize;
const TITLE: &str = "Photo Stamp screensaver";

/// Shows the dialog until OK or Cancel. `parent`: Screen Saver Settings, when it opened us.
pub fn show(parent: Option<isize>) {
    unsafe {
        // For the folder picker.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_UPDOWN_CLASS,
        };
        InitCommonControlsEx(&controls);
        let parent = parent.map_or(null_mut(), |h| h as HWND);
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            resource(IDD_SETTINGS),
            parent,
            Some(dialog_proc),
            0,
        );
    }
}

unsafe extern "system" fn dialog_proc(dlg: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            init(dlg);
            1
        }
        WM_COMMAND => {
            match (wparam & 0xffff) as i32 {
                IDC_BROWSE => browse(dlg),
                IDC_SHOWKEY => show_key(dlg),
                IDC_TEST => start_test(dlg),
                IDC_CAPTION if (wparam >> 16) as u32 == CBN_SELCHANGE => caption_changed(dlg),
                IDOK => ok(dlg),
                IDCANCEL => unsafe {
                    EndDialog(dlg, 0);
                },
                _ => return 0,
            }
            1
        }
        WM_TESTED => {
            let result = unsafe { Box::from_raw(lparam as *mut Result<Me, CheckError>) };
            set_text(dlg, IDC_STATUS, &tested_message(&result));
            unsafe { EnableWindow(GetDlgItem(dlg, IDC_TEST), 1) };
            1
        }
        _ => 0,
    }
}

fn init(dlg: HWND) {
    let (settings, prefilled) = store::load();
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (which, size) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
            let icon = LoadImageW(module, resource(1), IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
            SendMessageW(dlg, WM_SETICON, which as usize, icon as isize);
        }
        SendDlgItemMessageW(
            dlg,
            IDC_SECONDS_SPIN,
            UDM_SETRANGE32,
            MIN_SECONDS as usize,
            MAX_SECONDS as isize,
        );
        SendDlgItemMessageW(dlg, IDC_SECONDS_SPIN, UDM_SETPOS32, 0, settings.seconds_per_photo as isize);
        SendDlgItemMessageW(dlg, IDC_KEY, EM_SETPASSWORDCHAR, PASSWORD_CHAR, 0);
        let cue = wide("https://places.example.com");
        SendDlgItemMessageW(dlg, IDC_URL, EM_SETCUEBANNER, 0, cue.as_ptr() as isize);
    }
    fill_list(dlg, IDC_CAPTION, &Caption::ALL.map(|(_, label)| label), settings.caption.index());
    fill_list(dlg, IDC_CORNER, &Corner::ALL.map(|(_, label)| label), settings.corner.index());
    fill_list(dlg, IDC_ORDER, &Order::ALL.map(|(_, label)| label), settings.order.index());
    caption_changed(dlg);
    set_text(dlg, IDC_FOLDER, &settings.folder);
    set_text(dlg, IDC_URL, &settings.server_url);
    set_text(dlg, IDC_KEY, &settings.device_key);
    set_text(dlg, IDC_VERSION, &format!("Version {}", server::VERSION));
    if prefilled {
        set_text(
            dlg,
            IDC_STATUS,
            "Filled in from Photo Stamp on this PC. A separate device key lets you tell them apart on the server.",
        );
    }
}

fn fill_list(dlg: HWND, id: i32, labels: &[&str], selected: u32) {
    for label in labels {
        let label = wide(label);
        unsafe { SendDlgItemMessageW(dlg, id, CB_ADDSTRING, 0, label.as_ptr() as isize) };
    }
    unsafe { SendDlgItemMessageW(dlg, id, CB_SETCURSEL, selected as usize, 0) };
}

fn selected(dlg: HWND, id: i32) -> u32 {
    unsafe { SendDlgItemMessageW(dlg, id, CB_GETCURSEL, 0, 0) }.max(0) as u32
}

/// The corner only matters while the caption shows.
fn caption_changed(dlg: HWND) {
    let shown = Caption::from_index(selected(dlg, IDC_CAPTION)) != Caption::Hidden;
    unsafe { EnableWindow(GetDlgItem(dlg, IDC_CORNER), shown as i32) };
}

fn browse(dlg: HWND) {
    if let Some(chosen) = folder::pick(dlg, &text(dlg, IDC_FOLDER)) {
        set_text(dlg, IDC_FOLDER, &chosen);
    }
}

fn show_key(dlg: HWND) {
    unsafe {
        let shown = SendDlgItemMessageW(dlg, IDC_SHOWKEY, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        SendDlgItemMessageW(dlg, IDC_KEY, EM_SETPASSWORDCHAR, if shown { 0 } else { PASSWORD_CHAR }, 0);
        InvalidateRect(GetDlgItem(dlg, IDC_KEY), std::ptr::null(), 1);
    }
}

fn start_test(dlg: HWND) {
    let url = match normalize_url(&text(dlg, IDC_URL)) {
        Ok(url) => url,
        Err(message) => {
            set_text(dlg, IDC_STATUS, &message);
            return focus(dlg, IDC_URL);
        }
    };
    let key = text(dlg, IDC_KEY).trim().to_owned();
    if key.is_empty() {
        set_text(dlg, IDC_STATUS, "Enter this PC's device key first.");
        return focus(dlg, IDC_KEY);
    }
    set_text(dlg, IDC_STATUS, "Connecting…");
    unsafe { EnableWindow(GetDlgItem(dlg, IDC_TEST), 0) };
    // HWNDs aren't Send; the number is.
    let dlg = dlg as isize;
    std::thread::spawn(move || {
        let result = Box::into_raw(Box::new(server::check(&url, &key, TEST_TIMEOUT)));
        // Fails only if the dialog has closed meanwhile; then nobody wants the answer.
        if unsafe { PostMessageW(dlg as HWND, WM_TESTED, 0, result as isize) } == 0 {
            drop(unsafe { Box::from_raw(result) });
        }
    });
}

fn tested_message(result: &Result<Me, CheckError>) -> String {
    match result {
        Ok(me) => me.summary(),
        Err(e) => e.to_string(),
    }
}

fn ok(dlg: HWND) {
    let mut seconds_ok = 0;
    let seconds = unsafe { GetDlgItemInt(dlg, IDC_SECONDS, &mut seconds_ok, 0) };
    let typed = Settings {
        folder: text(dlg, IDC_FOLDER),
        server_url: text(dlg, IDC_URL),
        device_key: text(dlg, IDC_KEY),
        seconds_per_photo: if seconds_ok != 0 { seconds } else { 0 },
        caption: Caption::from_index(selected(dlg, IDC_CAPTION)),
        corner: Corner::from_index(selected(dlg, IDC_CORNER)),
        order: Order::from_index(selected(dlg, IDC_ORDER)),
    };
    let settings = match typed.validated() {
        Ok(s) => s,
        Err(invalid) => {
            message(dlg, &invalid.message, MB_OK | MB_ICONWARNING);
            let control = match invalid.field {
                Field::Folder => IDC_FOLDER,
                Field::ServerUrl => IDC_URL,
                Field::DeviceKey => IDC_KEY,
                Field::Seconds => IDC_SECONDS,
            };
            return focus(dlg, control);
        }
    };
    if !settings.folder.is_empty() && !Path::new(&settings.folder).is_dir() {
        let question = format!(
            "Windows can't find {} right now.\n\nSave it anyway? Choose Yes if it's on a drive or network share that isn't connected at the moment.",
            settings.folder
        );
        if message(dlg, &question, MB_YESNO | MB_ICONQUESTION) != IDYES {
            return focus(dlg, IDC_FOLDER);
        }
    }
    match store::save(&settings) {
        Ok(()) => unsafe {
            EndDialog(dlg, 1);
        },
        Err(e) => {
            message(dlg, &format!("Couldn't save the settings: {e}"), MB_OK | MB_ICONWARNING);
        }
    }
}

fn text(dlg: HWND, id: i32) -> String {
    unsafe {
        let control = GetDlgItem(dlg, id);
        let len = GetWindowTextLengthW(control);
        let mut buf = vec![0u16; len as usize + 1];
        let got = GetWindowTextW(control, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

fn set_text(dlg: HWND, id: i32, text: &str) {
    let text = wide(text);
    unsafe { SetDlgItemTextW(dlg, id, text.as_ptr()) };
}

/// Moves the focus to a control the way Tab would (selecting an edit box's text).
fn focus(dlg: HWND, id: i32) {
    unsafe { SendMessageW(dlg, WM_NEXTDLGCTL, GetDlgItem(dlg, id) as usize, 1) };
}

fn message(dlg: HWND, text: &str, style: u32) -> i32 {
    let (text, title) = (wide(text), wide(TITLE));
    unsafe { MessageBoxW(dlg, text.as_ptr(), title.as_ptr(), style) }
}
