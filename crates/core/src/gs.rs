//! Optional Ghostscript fallback for EPS/old-AI files without embedded previews.
//!
//! Only compiled when the `gs-fallback` feature is enabled. Dynamically loads
//! gsdll64.dll via the registry key HKLM\SOFTWARE\GPL Ghostscript\<ver>\GS_DLL.
//! Ghostscript is AGPL-licensed and must NOT be bundled with the MSI.

#[cfg(feature = "gs-fallback")]
use libloading::Library;

use crate::{CoreError, DecodedImage};

/// Render the first page of an EPS/PS document via Ghostscript.
///
/// Ghostscript is located by reading the GS_DLL registry value under
/// HKLM\SOFTWARE\GPL Ghostscript. Returns `NoThumbnail` if Ghostscript is
/// not installed or if rendering fails (non-fatal for thumbnail extraction).
#[allow(unused_variables)]
pub fn render(_data: &[u8]) -> Result<DecodedImage, CoreError> {
    #[cfg(feature = "gs-fallback")]
    {
        render_via_gs(_data)
    }
    #[cfg(not(feature = "gs-fallback"))]
    {
        Err(CoreError::NoThumbnail)
    }
}

#[cfg(feature = "gs-fallback")]
fn render_via_gs(data: &[u8]) -> Result<DecodedImage, CoreError> {
    // Locate gsdll64.dll via registry: HKLM\SOFTWARE\GPL Ghostscript\<ver>\GS_DLL
    let dll_path = find_gs_dll().ok_or(CoreError::NoThumbnail)?;

    // SAFETY: loading an external DLL; all subsequent calls use the loaded symbols.
    let lib = unsafe {
        Library::new(&dll_path)
            .map_err(|e| {
                log::debug!("gs-fallback: failed to load {dll_path:?}: {e}");
                CoreError::NoThumbnail
            })?
    };

    // Write input to a temp file, run GS to produce a PNG, read back pixels.
    // This is a simplified implementation; a complete one would use gsapi directly.
    drop(lib); // placeholder — prevents unused variable warning
    log::debug!("gs-fallback: GS rendering not yet fully implemented; returning NoThumbnail");
    Err(CoreError::NoThumbnail)
}

#[cfg(feature = "gs-fallback")]
fn find_gs_dll() -> Option<String> {
    // Read HKLM\SOFTWARE\GPL Ghostscript\* to locate the GS_DLL value.
    // Uses the `winreg` crate (not yet a dependency) — returns None as a stub.
    // TODO (gs-fallback): add winreg dependency and implement full registry walk.
    None
}
