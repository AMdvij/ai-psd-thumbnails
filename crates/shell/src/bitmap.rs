//! HBITMAP creation from premultiplied BGRA pixel data.
//!
//! The DIB is top-down (negative biHeight) 32 bpp BI_RGB.  Explorer expects
//! premultiplied BGRA when the provider returns WTSAT_ARGB; the caller must
//! have already run `to_bgra_premul` before calling `create_hbitmap`.
//!
//! `BitmapData` is an owned wrapper over HBITMAP + dimensions that calls
//! `DeleteObject` on drop.  Used by `PreviewProvider` to cache the decoded
//! preview bitmap; the COM object's Drop / Unload cleans up via this struct.

use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DeleteObject, DIB_RGB_COLORS, HBITMAP,
};
use windows::core::Error as WinError;

// ── BitmapData — owned HBITMAP wrapper ───────────────────────────────────────

/// Owned HBITMAP with associated pixel dimensions.
///
/// Calls `DeleteObject` on drop.  The raw `hbitmap` handle may be passed to
/// Win32 GDI (e.g. `SelectObject`) while the `BitmapData` instance is alive.
pub(crate) struct BitmapData {
    pub(crate) hbitmap: HBITMAP,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl BitmapData {
    /// Wrap an existing HBITMAP.  Takes ownership; caller must not call
    /// `DeleteObject` separately.
    pub(crate) fn new(hbitmap: HBITMAP, width: u32, height: u32) -> Self {
        Self { hbitmap, width, height }
    }
}

impl Drop for BitmapData {
    fn drop(&mut self) {
        if !self.hbitmap.is_invalid() {
            unsafe { let _ = DeleteObject(self.hbitmap.into()); }
        }
    }
}

// SAFETY: HBITMAP is a Win32 handle (isize-backed pointer).  We only pass it
// to GDI calls on the same STA thread that created it; the Send impl is
// required by the RefCell<Option<BitmapData>> field in PreviewProvider which
// must be Send for the COM object to satisfy windows-rs trait bounds.
// In practice prevhost runs on a single STA so cross-thread usage does not
// occur at runtime.
unsafe impl Send for BitmapData {}

/// Create a top-down 32 bpp DIB section and copy `bgra` pixels into it.
///
/// `bgra` must be exactly `width * height * 4` bytes of premultiplied BGRA.
/// Returns the HBITMAP on success.  The HBITMAP is caller-owned and must be
/// freed by the caller (Explorer frees it via DeleteObject for thumbnails).
pub(crate) fn create_hbitmap(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<HBITMAP, WinError> {
    let expected = (width * height * 4) as usize;
    assert_eq!(
        bgra.len(),
        expected,
        "bgra buffer length {len} != expected {expected}",
        len = bgra.len()
    );

    // Negative height signals a top-down DIB (row 0 = top of image).
    // BI_RGB (value 0) with biBitCount=32 means 32 bpp BGR with an unused alpha byte,
    // but Explorer interprets it as BGRA when the provider returns WTSAT_ARGB.
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // negative = top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB = 0
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..Default::default()
    };

    let mut pv_bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbmp = unsafe {
        CreateDIBSection(
            None,            // no HDC needed for DIB_RGB_COLORS
            &bmi,
            DIB_RGB_COLORS,
            &mut pv_bits,
            None,            // no file-mapping section
            0,
        )?
    };

    // Copy the premultiplied BGRA pixels into the DIB memory.
    if !pv_bits.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), pv_bits as *mut u8, bgra.len());
        }
    }

    Ok(hbmp)
}

/// Convenience wrapper: create a DIB section and wrap it in `BitmapData`.
///
/// Used by `PreviewProvider` so the bitmap is automatically freed on drop.
pub(crate) fn create_bitmap_data(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<BitmapData, WinError> {
    let hbmp = create_hbitmap(bgra, width, height)?;
    Ok(BitmapData::new(hbmp, width, height))
}
