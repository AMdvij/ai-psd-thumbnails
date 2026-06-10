//! DllRegisterServer / DllUnregisterServer — write / delete Shell registry keys.
//!
//! Registration invariants (anti-SageThumbs):
//!   - Writes ONLY to:
//!       HKCR\CLSID\{CLSID-A}\InprocServer32         (DLL path + ThreadingModel)
//!       HKCR\SystemFileAssociations\.<ext>\ShellEx\{e357fccd-...}  (5 exts)
//!   - NEVER touches HKCR\.<ext> default value, ProgID, or OpenWith.
//!   - ThreadingModel = "Apartment" (STA); pdfium process-wide serialisation
//!     is provided by pdfium-render's `thread_safe` feature.
//!
//! DllRegisterServer / DllUnregisterServer are invoked by dev-register.ps1
//! (via regsvr32).  Production registration is the MSI's responsibility.

use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, S_OK};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteKeyW, RegSetValueExW,
    HKEY, HKEY_CLASSES_ROOT, REG_SZ,
};

use crate::factory::CLSID_THUMB;

/// System-defined thumbnail handler ShellEx sub-key GUID.
const THUMB_SHELLEXT_GUID: &str = "{e357fccd-a995-4576-b01f-234630154e96}";

/// File extensions to register.
const EXTS: &[&str] = &[".ai", ".psd", ".psb", ".eps", ".svg"];

// ── Helpers ───────────────────────────────────────────────────────────────────

fn win32_to_hresult(code: u32) -> HRESULT {
    if code == ERROR_SUCCESS.0 {
        S_OK
    } else {
        HRESULT(((code & 0xFFFF) | 0x80070000) as i32)
    }
}

/// Encode a `&str` as null-terminated UTF-16.
fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode a `&str` as little-endian UTF-16 bytes (including null terminator)
/// for use as REG_SZ data in RegSetValueExW.
fn wide_bytes_sz(s: &str) -> Vec<u8> {
    wide_null(s).iter().flat_map(|c| c.to_le_bytes()).collect()
}

/// Open-or-create a key under HKCR.  Caller must RegCloseKey the result.
unsafe fn create_key(subkey: &str) -> Result<HKEY, HRESULT> {
    let w = wide_null(subkey);
    let mut hkey = HKEY::default();
    let rc = unsafe { RegCreateKeyW(HKEY_CLASSES_ROOT, PCWSTR(w.as_ptr()), &mut hkey) };
    if rc == ERROR_SUCCESS {
        Ok(hkey)
    } else {
        Err(win32_to_hresult(rc.0))
    }
}

/// Set a named REG_SZ value on an open key.
unsafe fn set_sz(hkey: HKEY, name: &str, value: &str) -> Result<(), HRESULT> {
    let wname = wide_null(name);
    let bytes = wide_bytes_sz(value);
    let rc = unsafe {
        RegSetValueExW(hkey, PCWSTR(wname.as_ptr()), None, REG_SZ, Some(bytes.as_slice()))
    };
    if rc == ERROR_SUCCESS { Ok(()) } else { Err(win32_to_hresult(rc.0)) }
}

/// Set the default (unnamed) REG_SZ value on an open key.
unsafe fn set_default(hkey: HKEY, value: &str) -> Result<(), HRESULT> {
    unsafe { set_sz(hkey, "", value) }
}

/// Delete a registry key under HKCR (non-recursive; must have no subkeys).
/// Missing keys are silently ignored (idempotent).
unsafe fn delete_key(subkey: &str) -> Result<(), HRESULT> {
    let w = wide_null(subkey);
    let rc = unsafe { RegDeleteKeyW(HKEY_CLASSES_ROOT, PCWSTR(w.as_ptr())) };
    // ERROR_FILE_NOT_FOUND (2) = already absent — treat as success for idempotency.
    if rc == ERROR_SUCCESS || rc.0 == 2 {
        Ok(())
    } else {
        Err(win32_to_hresult(rc.0))
    }
}

// ── CLSID string formatting ───────────────────────────────────────────────────

fn clsid_string() -> String {
    let d = &CLSID_THUMB;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        d.data1,
        d.data2,
        d.data3,
        d.data4[0],
        d.data4[1],
        d.data4[2],
        d.data4[3],
        d.data4[4],
        d.data4[5],
        d.data4[6],
        d.data4[7],
    )
}

// ── DllRegisterServer ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register_impl() {
        Ok(()) => S_OK,
        Err(hr) => { log::error!("DllRegisterServer failed: {hr:?}"); hr }
    }
}

fn register_impl() -> Result<(), HRESULT> {
    let clsid = clsid_string();

    // Resolve our DLL path from the stored HMODULE.
    let dll_path = crate::pdfium_path::pdfium_dll_path()
        .and_then(|p| p.parent().map(|d| d.join("thumb_viewer.dll")))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "thumb_viewer.dll".to_owned());

    // HKCR\CLSID\{CLSID-A}\InprocServer32
    let inproc_path = format!("CLSID\\{}\\InprocServer32", clsid);
    unsafe {
        let hkey = create_key(&inproc_path)?;
        let r1 = set_default(hkey, &dll_path);
        let r2 = set_sz(hkey, "ThreadingModel", "Apartment");
        let _ = RegCloseKey(hkey);
        r1?;
        r2?;
    }

    // HKCR\SystemFileAssociations\.<ext>\ShellEx\{thumbnail-handler-guid}
    for ext in EXTS {
        let key_path = format!(
            "SystemFileAssociations\\{}\\ShellEx\\{}",
            ext, THUMB_SHELLEXT_GUID
        );
        unsafe {
            let hkey = create_key(&key_path)?;
            let r = set_default(hkey, &clsid);
            let _ = RegCloseKey(hkey);
            r?;
        }
    }

    Ok(())
}

// ── DllUnregisterServer ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unregister_impl() {
        Ok(()) => S_OK,
        Err(hr) => { log::error!("DllUnregisterServer failed: {hr:?}"); hr }
    }
}

fn unregister_impl() -> Result<(), HRESULT> {
    let clsid = clsid_string();

    // Remove per-extension ShellEx subkeys (leaves parent ShellEx key intact).
    for ext in EXTS {
        let key_path = format!(
            "SystemFileAssociations\\{}\\ShellEx\\{}",
            ext, THUMB_SHELLEXT_GUID
        );
        unsafe { delete_key(&key_path)?; }
    }

    // Remove InprocServer32 then the CLSID container key.
    let inproc_path = format!("CLSID\\{}\\InprocServer32", clsid);
    unsafe { delete_key(&inproc_path)?; }

    let clsid_path = format!("CLSID\\{}", clsid);
    unsafe { delete_key(&clsid_path)?; }

    Ok(())
}
