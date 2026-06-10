//! HBITMAP creation from premultiplied BGRA pixel data.
//!
//! The DIB is top-down (negative biHeight) 32 bpp BI_RGB.  Explorer expects
//! premultiplied BGRA when the provider returns WTSAT_ARGB; the caller must
//! have already run `to_bgra_premul` before calling `create_hbitmap`.

use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    CreateDIBSection,
};
use windows::core::Error as WinError;

/// Create a top-down 32 bpp DIB section and copy `bgra` pixels into it.
///
/// `bgra` must be exactly `width * height * 4` bytes of premultiplied BGRA.
/// Returns the HBITMAP on success.  The HBITMAP is caller-owned and must be
/// passed back to Explorer (which frees it via DeleteObject).
pub(crate) fn create_hbitmap(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<windows::Win32::Graphics::Gdi::HBITMAP, WinError> {
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
