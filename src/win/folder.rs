//! The folder picker: Windows' own Open dialog in folder mode (it reaches network shares and
//! mapped drives, unlike the old folder tree).

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, w};

/// The folder chosen, starting from `current`; `None` if canceled. COM must be initialized
/// (apartment-threaded) on this thread.
pub fn pick(owner: windows_sys::Win32::Foundation::HWND, current: &str) -> Option<String> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(w!("Choose the folder of photos"));
        if !current.is_empty()
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(current), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // Canceling is an error here.
        dialog.Show(Some(HWND(owner))).ok()?;
        let path = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let chosen = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        chosen
    }
}
