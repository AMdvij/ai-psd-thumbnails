//! Resolve pdfium.dll path relative to this DLL's own directory.
//!
//! When Explorer hosts the DLL via dllhost.exe, the working directory is
//! arbitrary.  We use GetModuleFileNameW on our own HMODULE to find the
//! directory we were loaded from, then look for pdfium.dll alongside us.
//!
//! Fallback: PDFIUM_DLL_PATH environment variable (used in dev/test runs
//! where the DLL is not co-located with the output binary).

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::System::LibraryLoader::GetModuleFileNameW;

use crate::our_hmodule;

/// Return the absolute path to pdfium.dll, or None if it cannot be determined.
pub(crate) fn pdfium_dll_path() -> Option<PathBuf> {
    // Prefer env override (dev/test).
    if let Ok(p) = std::env::var("PDFIUM_DLL_PATH") {
        return Some(PathBuf::from(p));
    }

    // Derive from our own module path.
    let hmodule = our_hmodule()?;

    let mut buf = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(Some(hmodule), &mut buf) };
    if len == 0 {
        return None;
    }

    let os = OsString::from_wide(&buf[..len as usize]);
    let our_path = PathBuf::from(os);
    let dir = our_path.parent()?;
    Some(dir.join("pdfium.dll"))
}

/// Initialise the pdfium singleton from the resolved path.
/// No-op if pdfium was already initialised.  Silently skips if the DLL
/// cannot be found (formats that don't need PDF rendering will still work).
pub(crate) fn ensure_pdfium_init() {
    let path = match pdfium_dll_path() {
        Some(p) => p,
        None => return,
    };
    let path_str = path.to_string_lossy();
    if let Err(e) = thumb_viewer_core::pdf::init(&path_str) {
        log::warn!("pdfium init skipped: {e}");
    }
}
