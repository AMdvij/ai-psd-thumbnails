//! DOS-EPS binary header preview extraction.
//!
//! The DOS-EPS format prepends a 30-byte binary header (magic 0xC5D0D3C6) that
//! gives byte offsets and lengths for the PostScript data, a WMF preview, and
//! a TIFF preview.  We prefer the TIFF preview (decoded via the `tiff` crate);
//! WMF is deferred to `NoThumbnail`.  Files without the DOS header are pure PS
//! and cannot be previewed without Ghostscript.

use std::io::Cursor;

use crate::{CoreError, DecodedImage};

// DOS-EPS binary header magic (little-endian u32 = 0xC5D0D3C6).
const DOS_EPS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];

/// Decode a preview from a DOS-EPS file.
///
/// Returns `CoreError::NoThumbnail` when:
///   - The file lacks a DOS-EPS binary header.
///   - The header has no TIFF preview offset/length.
///   - The TIFF data is unsupported or corrupt (`tiff` crate returns an error).
pub fn decode(data: &[u8]) -> Result<DecodedImage, CoreError> {
    if data.len() < 30 || data[..4] != DOS_EPS_MAGIC {
        return Err(CoreError::NoThumbnail);
    }

    // DOS-EPS 30-byte header layout (all little-endian u32):
    //   [0..4]   magic
    //   [4..8]   PS offset
    //   [8..12]  PS length
    //   [12..16] WMF offset
    //   [16..20] WMF length
    //   [20..24] TIFF offset
    //   [24..28] TIFF length
    //   [28..30] checksum (u16, ignored)
    let tiff_offset = read_u32_le(&data[20..24]) as usize;
    let tiff_len = read_u32_le(&data[24..28]) as usize;

    if tiff_offset == 0 || tiff_len == 0 {
        // No TIFF preview; WMF-only → NoThumbnail.
        return Err(CoreError::NoThumbnail);
    }

    // Bounds-check the declared offset + length against file size.
    let end = tiff_offset.checked_add(tiff_len).ok_or_else(|| {
        CoreError::Decode("DOS-EPS TIFF offset+length overflow".into())
    })?;
    if end > data.len() {
        return Err(CoreError::Decode(format!(
            "DOS-EPS TIFF region [{tiff_offset}..{end}] exceeds file size {}",
            data.len()
        )));
    }

    let tiff_bytes = &data[tiff_offset..end];
    decode_tiff_preview(tiff_bytes)
}

/// Decode a TIFF preview blob via the `tiff` crate.
/// Any unsupported variant or decode error maps to `NoThumbnail` (graceful fallback).
fn decode_tiff_preview(tiff_bytes: &[u8]) -> Result<DecodedImage, CoreError> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::ColorType;

    let cursor = Cursor::new(tiff_bytes);
    let mut decoder = Decoder::new(cursor).map_err(|e| {
        log::debug!("DOS-EPS TIFF init error: {e}");
        CoreError::NoThumbnail
    })?;

    let (width, height) = decoder.dimensions().map_err(|e| {
        log::debug!("DOS-EPS TIFF dimensions error: {e}");
        CoreError::NoThumbnail
    })?;

    // Cap dimensions before allocating.
    if width > 8192 || height > 8192 {
        log::debug!("DOS-EPS TIFF dimensions {width}×{height} exceed safety cap");
        return Err(CoreError::NoThumbnail);
    }

    let color_type = decoder.colortype().map_err(|e| {
        log::debug!("DOS-EPS TIFF color type error: {e}");
        CoreError::NoThumbnail
    })?;

    let result = decoder.read_image().map_err(|e| {
        log::debug!("DOS-EPS TIFF decode error: {e}");
        CoreError::NoThumbnail
    })?;

    let rgba = match (color_type, result) {
        (ColorType::RGB(8), DecodingResult::U8(pixels)) => {
            pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        (ColorType::RGBA(8), DecodingResult::U8(pixels)) => pixels,
        (ColorType::Gray(8), DecodingResult::U8(pixels)) => {
            pixels.iter().flat_map(|&g| [g, g, g, 255]).collect()
        }
        (ColorType::GrayA(8), DecodingResult::U8(pixels)) => {
            pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        (ct, _) => {
            log::debug!("DOS-EPS TIFF: unsupported color type {ct:?}");
            return Err(CoreError::NoThumbnail);
        }
    };

    Ok(DecodedImage { width, height, rgba })
}

fn read_u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_dos_header_returns_no_thumbnail() {
        let data = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n";
        assert!(matches!(decode(data), Err(CoreError::NoThumbnail)));
    }

    #[test]
    fn truncated_dos_header_returns_no_thumbnail() {
        // Only the magic, not a full 30-byte header.
        let data = [0xC5, 0xD0, 0xD3, 0xC6, 0x00, 0x00];
        assert!(matches!(decode(&data), Err(CoreError::NoThumbnail)));
    }

    #[test]
    fn dos_header_zero_tiff_offset_returns_no_thumbnail() {
        // Build a 30-byte header with TIFF offset=0, length=0.
        let mut hdr = [0u8; 30];
        hdr[..4].copy_from_slice(&DOS_EPS_MAGIC);
        // All offsets/lengths zero → no TIFF preview.
        assert!(matches!(decode(&hdr), Err(CoreError::NoThumbnail)));
    }

    #[test]
    fn dos_header_out_of_bounds_tiff_returns_decode_error() {
        let mut hdr = [0u8; 30];
        hdr[..4].copy_from_slice(&DOS_EPS_MAGIC);
        // TIFF offset=20, length=9999 — extends beyond the 30-byte buffer.
        hdr[20..24].copy_from_slice(&20u32.to_le_bytes());
        hdr[24..28].copy_from_slice(&9999u32.to_le_bytes());
        assert!(matches!(decode(&hdr), Err(CoreError::Decode(_))));
    }
}
