//! thumb_viewer.dll — COM in-process thumbnail + preview shell extension.
//!
//! Exports: DllGetClassObject, DllCanUnloadNow, DllRegisterServer, DllUnregisterServer.
//! Implements:
//!   IThumbnailProvider + IInitializeWithStream (ThumbProvider, CLSID-A)
//!   IPreviewHandler + IInitializeWithStream + IObjectWithSite + IOleWindow
//!     (PreviewProvider, CLSID-B)
//! Formats: .ai / .psd / .psb / .eps / .svg

#![cfg(target_os = "windows")]

// Required so that the #[implement] proc-macro's generated ::windows_core:: paths
// resolve correctly in this crate.
extern crate windows_core;

pub(crate) mod bitmap;
pub(crate) mod factory;
pub(crate) mod pdfium_path;
pub(crate) mod preview;
pub(crate) mod preview_window;
pub(crate) mod register;
pub(crate) mod stream_util;
pub(crate) mod thumbnail;

use std::sync::OnceLock;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

// ── Module-global HMODULE ─────────────────────────────────────────────────────

/// Newtype wrapper that implements Send + Sync for HMODULE.
/// HMODULE is a process-wide constant once set; read-only after DLL_PROCESS_ATTACH.
struct SendHmodule(HMODULE);
// SAFETY: HMODULE is a raw pointer used only as an opaque handle after attach.
// We never dereference it; it is read-only after the initial write in DllMain.
unsafe impl Send for SendHmodule {}
unsafe impl Sync for SendHmodule {}

static OUR_HMODULE: OnceLock<SendHmodule> = OnceLock::new();

/// Returns the HMODULE for this DLL, stored during DllMain DLL_PROCESS_ATTACH.
pub(crate) fn our_hmodule() -> Option<HMODULE> {
    OUR_HMODULE.get().map(|s| s.0)
}

// ── DllMain ───────────────────────────────────────────────────────────────────

#[no_mangle]
#[allow(non_snake_case)]
unsafe extern "system" fn DllMain(
    hinstance: HMODULE,
    dw_reason: u32,
    _lp_reserved: *mut std::ffi::c_void,
) -> i32 {
    if dw_reason == DLL_PROCESS_ATTACH {
        let _ = OUR_HMODULE.set(SendHmodule(hinstance));
    }
    1 // TRUE
}

// ── DLL exports (forwarded from submodules) ───────────────────────────────────

pub use factory::{DllCanUnloadNow, DllGetClassObject};
pub use register::{DllRegisterServer, DllUnregisterServer};
