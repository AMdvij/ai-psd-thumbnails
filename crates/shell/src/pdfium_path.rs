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

/// Return the DIRECTORY containing pdfium.dll, or None if it cannot be
/// determined.
///
/// IMPORTANT: this must be a directory, not the DLL file path —
/// `core::pdf::init` passes it to pdfium-render's
/// `pdfium_platform_library_name_at_path`, which appends "pdfium.dll"
/// itself. Passing a file path here makes the loader look for
/// `...\pdfium.dll\pdfium.dll` and silently disables all PDF rendering.
pub(crate) fn pdfium_dir() -> Option<PathBuf> {
    // Prefer env override (dev/test) — a directory path.
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
    Some(our_path.parent()?.to_path_buf())
}

/// Initialise the pdfium singleton from the resolved directory.
/// No-op if pdfium was already initialised.  Silently skips if the DLL
/// cannot be found (formats that don't need PDF rendering will still work).
pub(crate) fn ensure_pdfium_init() {
    let dir = match pdfium_dir() {
        Some(d) => d,
        None => return,
    };
    let dir_str = dir.to_string_lossy();
    if let Err(e) = ai_psd_thumbnails_core::pdf::init(&dir_str) {
        log::warn!("pdfium init skipped: {e}");
    }
}
