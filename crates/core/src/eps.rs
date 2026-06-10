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

    // The tiff crate refuses palette (PhotometricInterpretation=3) images in
    // colortype()/read_image(), but Illustrator/AdobeStock DOS-EPS previews
    // are commonly 8-bit palette TIFFs. Its tag reader still works, so palette
    // images take a manual strip-reading path instead.
    let color_type = match decoder.colortype() {
        Ok(ct) => ct,
        Err(e) => {
            log::debug!("DOS-EPS TIFF color type unsupported, trying palette path: {e}");
            return decode_palette_tiff(&mut decoder, tiff_bytes, width, height);
        }
    };

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
        (ColorType::CMYK(8), DecodingResult::U8(pixels)) => pixels
            .chunks_exact(4)
            .flat_map(|p| {
                let k = p[3] as u32;
                let to_rgb = |v: u8| (((255 - v as u32) * (255 - k)) / 255) as u8;
                [to_rgb(p[0]), to_rgb(p[1]), to_rgb(p[2]), 255]
            })
            .collect(),
        (ct, _) => {
            log::debug!("DOS-EPS TIFF: unsupported color type {ct:?}");
            return Err(CoreError::NoThumbnail);
        }
    };

    Ok(DecodedImage { width, height, rgba })
}

/// Manual decode path for 8-bit palette TIFFs the `tiff` crate refuses.
///
/// Uses the crate's tag reader (which works fine for palette IFDs) to locate
/// strips, decompresses them (uncompressed or PackBits only — LZW palette
/// previews are rare enough to defer), and expands indices through ColorMap.
/// All strip offsets/lengths come from a hostile file: every slice is
/// bounds-checked against the preview blob and output is capped at
/// width×height before allocation-relevant work.
fn decode_palette_tiff<R: std::io::Read + std::io::Seek>(
    decoder: &mut tiff::decoder::Decoder<R>,
    tiff_bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<DecodedImage, CoreError> {
    use tiff::tags::Tag;

    let no_thumb = |what: &str| {
        log::debug!("DOS-EPS palette TIFF: {what}");
        CoreError::NoThumbnail
    };

    let photometric = decoder
        .get_tag(Tag::PhotometricInterpretation)
        .ok()
        .and_then(|v| v.into_u16().ok());
    if photometric != Some(3) {
        return Err(no_thumb("not palette photometric"));
    }

    let bits = decoder
        .get_tag(Tag::BitsPerSample)
        .ok()
        .and_then(|v| v.into_u16().ok());
    if bits != Some(8) {
        return Err(no_thumb("bits per sample != 8"));
    }

    // Compression: absent tag defaults to 1 (uncompressed) per TIFF 6.0.
    let compression = decoder
        .get_tag(Tag::Compression)
        .ok()
        .and_then(|v| v.into_u16().ok())
        .unwrap_or(1);
    if compression != 1 && compression != 32773 {
        return Err(no_thumb("unsupported compression (only none/PackBits)"));
    }

    // Palette TIFFs may carry extra samples per pixel (Photoshop writes
    // index+alpha as SamplesPerPixel=2). The palette index is sample 0;
    // sample 1, when present, is treated as alpha. Default per TIFF 6.0 is 1.
    let spp = decoder
        .get_tag(Tag::SamplesPerPixel)
        .ok()
        .and_then(|v| v.into_u16().ok())
        .unwrap_or(1) as usize;
    if spp == 0 || spp > 4 {
        return Err(no_thumb("unsupported samples per pixel"));
    }

    let colormap = decoder
        .get_tag_u16_vec(Tag::ColorMap)
        .map_err(|_| no_thumb("missing ColorMap"))?;

    let offsets = decoder
        .get_tag(Tag::StripOffsets)
        .map_err(|_| no_thumb("missing StripOffsets"))?
        .into_u64_vec()
        .map_err(|_| no_thumb("bad StripOffsets"))?;
    let counts = decoder
        .get_tag(Tag::StripByteCounts)
        .map_err(|_| no_thumb("missing StripByteCounts"))?
        .into_u64_vec()
        .map_err(|_| no_thumb("bad StripByteCounts"))?;
    if offsets.len() != counts.len() || offsets.is_empty() {
        return Err(no_thumb("strip offset/count tables mismatched"));
    }

    let expected = (width as usize) * (height as usize) * spp;
    let mut samples: Vec<u8> = Vec::with_capacity(expected);
    for (&off, &count) in offsets.iter().zip(&counts) {
        let (start, len) = (off as usize, count as usize);
        let end = start
            .checked_add(len)
            .ok_or_else(|| no_thumb("strip range overflow"))?;
        if end > tiff_bytes.len() {
            return Err(no_thumb("strip exceeds preview blob"));
        }
        let strip = &tiff_bytes[start..end];
        let remaining = expected.saturating_sub(samples.len());
        if remaining == 0 {
            break;
        }
        if compression == 1 {
            samples.extend_from_slice(&strip[..strip.len().min(remaining)]);
        } else {
            crate::psd::packbits_decode(strip, &mut samples, remaining)?;
        }
    }

    if samples.len() < expected {
        return Err(no_thumb("strip data shorter than width×height×samples"));
    }
    samples.truncate(expected);

    let rgba = expand_palette(&samples, spp, &colormap)?;
    Ok(DecodedImage { width, height, rgba })
}

/// Expand 8-bit palette samples to RGBA using the TIFF ColorMap tag.
///
/// `samples` holds `spp` interleaved bytes per pixel: sample 0 is the palette
/// index; any extra samples are skipped (stride only). Photoshop writes
/// index+alpha palette TIFFs (SamplesPerPixel=2, ExtraSamples=associated),
/// but that alpha plane is halftone-dithered 0/255 — honouring it produces a
/// checkerboard-speckled thumbnail, so we match the Windows GDI+ reference
/// behaviour and render the palette colours opaque.
///
/// The ColorMap stores 16-bit values in three consecutive planes
/// (all R entries, then all G, then all B); each plane has 2^bits entries.
/// Out-of-range indices and short colormaps resolve to black rather than
/// erroring — the preview is best-effort.
fn expand_palette(samples: &[u8], spp: usize, colormap: &[u16]) -> Result<Vec<u8>, CoreError> {
    let plane = colormap.len() / 3;
    if plane == 0 || spp == 0 {
        log::debug!("DOS-EPS TIFF palette: empty/short ColorMap or zero spp");
        return Err(CoreError::NoThumbnail);
    }
    let component = |plane_idx: usize, i: usize| -> u8 {
        colormap
            .get(plane_idx * plane + i)
            .map(|&v| (v >> 8) as u8)
            .unwrap_or(0)
    };
    Ok(samples
        .chunks_exact(spp)
        .flat_map(|px| {
            let i = (px[0] as usize).min(plane - 1);
            [component(0, i), component(1, i), component(2, i), 255]
        })
        .collect())
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
    fn palette_expansion_maps_indices_via_colormap_planes() {
        // 2-entry colormap: index 0 → red, index 1 → blue (16-bit values,
        // high byte is the 8-bit component).
        let colormap: Vec<u16> = vec![
            0xFF00, 0x0000, // R plane
            0x0000, 0x0000, // G plane
            0x0000, 0xFF00, // B plane
        ];
        let rgba = expand_palette(&[0, 1, 1], 1, &colormap).unwrap();
        assert_eq!(rgba, [255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn palette_extra_samples_are_skipped_as_stride() {
        // Index+alpha (SamplesPerPixel=2): only sample 0 (the index) is used;
        // the dithered alpha sample is ignored and pixels render opaque.
        let colormap: Vec<u16> = vec![
            0xFF00, 0x0000, // R plane
            0x0000, 0x0000, // G plane
            0x0000, 0xFF00, // B plane
        ];
        let rgba = expand_palette(&[1, 0x80, 0, 0xFF], 2, &colormap).unwrap();
        assert_eq!(rgba, [0, 0, 255, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn palette_out_of_range_index_clamps_instead_of_panicking() {
        let colormap: Vec<u16> = vec![0x1100, 0x2200, 0x3300]; // 1 entry
        let rgba = expand_palette(&[7], 1, &colormap).unwrap();
        assert_eq!(rgba, [0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn palette_empty_colormap_returns_no_thumbnail() {
        assert!(matches!(expand_palette(&[0], 1, &[]), Err(CoreError::NoThumbnail)));
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
