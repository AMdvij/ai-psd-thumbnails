//! PSD (version 1) and PSB (version 2) thumbnail decoder.
//!
//! Section length-field widths — only the Layer & Mask Info section widens in PSB:
//!   - File Header:             identical layout for PSD and PSB (version field differs)
//!   - Color Mode Data length:  4 bytes in BOTH PSD and PSB
//!   - Image Resources length:  4 bytes in BOTH PSD and PSB
//!   - Layer & Mask Info length: 4 bytes (PSD) / 8 bytes (PSB)
//!   - Image Data:              NO length field (2-byte compression word, then pixel data to EOF)
//!
//! Fast path: walk Image Resources to find resource id 1036 (RGB JPEG thumbnail).
//! Fallback:  resource id 1033 (BGR JPEG, legacy Photoshop 4.0) with B↔R swap.
//! Composite fallback: RAW or RLE/PackBits pixel data (size-capped; skip ZIP).
//! Never allocates proportional to full file size on the fast path.

use std::io::{Cursor, Read, Seek, SeekFrom};

use crate::{CoreError, DecodedImage, Ext};

// Maximum pixel dimensions for the composite fallback to avoid OOM on huge PSBs.
const COMPOSITE_MAX_DIM: u32 = 4096;

// ── Public entry ──────────────────────────────────────────────────────────────

pub fn decode(data: &[u8], ext: Ext) -> Result<DecodedImage, CoreError> {
    let mut cur = Cursor::new(data);
    let is_psb = parse_header(&mut cur, ext)?;
    skip_color_mode_data(&mut cur)?;
    decode_from_image_resources(&mut cur, is_psb, data)
}

// ── Header ────────────────────────────────────────────────────────────────────

/// Parse the 26-byte file header; return `true` if this is PSB (version=2).
fn parse_header(cur: &mut Cursor<&[u8]>, ext: Ext) -> Result<bool, CoreError> {
    let mut sig = [0u8; 4];
    cur.read_exact(&mut sig).map_err(io_err)?;
    if &sig != b"8BPS" {
        return Err(CoreError::Decode("not a PSD/PSB file (bad signature)".into()));
    }

    let version = read_u16_be(cur)?;
    let is_psb = match version {
        1 => false,
        2 => true,
        v => return Err(CoreError::Decode(format!("unknown PSD version: {v}"))),
    };

    // Validate that the ext hint matches the on-disk version.
    match (ext, is_psb) {
        (Ext::Psb, false) => log::warn!("file has PSD version=1 but .psb extension"),
        (Ext::Psd, true) => log::warn!("file has PSB version=2 but .psd extension"),
        _ => {}
    }

    // Reserved: 6 zero bytes
    cur.seek(SeekFrom::Current(6)).map_err(io_err)?;
    // Channels (2), height (4), width (4), depth (2), color mode (2) = 14 bytes
    cur.seek(SeekFrom::Current(14)).map_err(io_err)?;

    Ok(is_psb)
}

// ── Color Mode Data (4-byte length in BOTH PSD and PSB) ──────────────────────

fn skip_color_mode_data(cur: &mut Cursor<&[u8]>) -> Result<(), CoreError> {
    let len = read_u32_be(cur)? as u64;
    cur.seek(SeekFrom::Current(len as i64)).map_err(io_err)?;
    Ok(())
}

// ── Image Resources (4-byte length in BOTH PSD and PSB) ──────────────────────

fn decode_from_image_resources(
    cur: &mut Cursor<&[u8]>,
    is_psb: bool,
    raw_data: &[u8],
) -> Result<DecodedImage, CoreError> {
    // Image Resources section length: 4 bytes in BOTH PSD and PSB.
    let section_len = read_u32_be(cur)? as u64;
    let section_start = cur.position();
    let section_end = section_start + section_len;

    let mut jpeg_1036: Option<Vec<u8>> = None;
    let mut jpeg_1033: Option<Vec<u8>> = None;

    while cur.position() < section_end {
        // 8BIM signature
        let mut sig = [0u8; 4];
        if cur.read_exact(&mut sig).is_err() {
            break;
        }
        if &sig != b"8BIM" {
            // Desynced — bail gracefully.
            break;
        }

        let resource_id = read_u16_be(cur)?;

        // Pascal name: 1-byte length prefix + that many bytes, padded to even total.
        let name_len = read_u8(cur)? as u64;
        // Total name field = 1 (len byte) + name_len, padded to even.
        let name_field_len = 1 + name_len;
        let name_field_padded = (name_field_len + 1) & !1; // round up to even
        // We already consumed the 1-byte len prefix; skip the rest.
        let skip = name_field_padded - 1;
        cur.seek(SeekFrom::Current(skip as i64)).map_err(io_err)?;

        // Resource data length: 4 bytes in BOTH PSD and PSB.
        let data_len = read_u32_be(cur)? as u64;
        // Data padded to even length.
        let data_padded = (data_len + 1) & !1;

        let data_start = cur.position();

        match resource_id {
            1036 => {
                // RGB JPEG thumbnail — preferred (no color swap needed).
                jpeg_1036 = Some(read_thumbnail_jpeg(cur, data_len)?);
            }
            1033 => {
                // BGR JPEG thumbnail — legacy Photoshop 4.0 (swap B↔R after decode).
                jpeg_1033 = Some(read_thumbnail_jpeg(cur, data_len)?);
            }
            _ => {}
        }

        // Seek to the end of this resource (handles partial reads above).
        cur.seek(SeekFrom::Start(data_start + data_padded))
            .map_err(io_err)?;
    }

    // Seek past the Image Resources section.
    cur.seek(SeekFrom::Start(section_end)).map_err(io_err)?;

    // Prefer 1036 (RGB), fallback to 1033 (BGR).
    if let Some(jpeg) = jpeg_1036 {
        return decode_jpeg_rgb(&jpeg);
    }
    if let Some(jpeg) = jpeg_1033 {
        return decode_jpeg_bgr(&jpeg);
    }

    // No embedded thumbnail — try composite pixel data fallback.
    decode_composite_fallback(cur, is_psb, raw_data)
}

// ── Thumbnail JPEG resource reading ──────────────────────────────────────────

/// Read `data_len` bytes for a resource-1036/1033 block.
/// The block starts with a 28-byte thumbnail header; the JPEG payload follows.
fn read_thumbnail_jpeg(cur: &mut Cursor<&[u8]>, data_len: u64) -> Result<Vec<u8>, CoreError> {
    if data_len < 28 {
        return Err(CoreError::Decode("thumbnail resource too short".into()));
    }
    // 28-byte header: format(4) + width(4) + height(4) + widthbytes(4) +
    //                 totalsize(4) + compressedsize(4) + bitsperpixel(2) + planes(2)
    cur.seek(SeekFrom::Current(28)).map_err(io_err)?;
    let jpeg_len = (data_len - 28) as usize;
    let mut jpeg = vec![0u8; jpeg_len];
    cur.read_exact(&mut jpeg).map_err(io_err)?;
    Ok(jpeg)
}

// ── JPEG decode helpers ───────────────────────────────────────────────────────

fn decode_jpeg_rgb(jpeg: &[u8]) -> Result<DecodedImage, CoreError> {
    let mut dec = jpeg_decoder::Decoder::new(jpeg);
    let pixels = dec.decode().map_err(|e| CoreError::Decode(format!("JPEG 1036: {e}")))?;
    let info = dec.info().ok_or_else(|| CoreError::Decode("JPEG 1036: no info".into()))?;
    let rgba = rgb24_to_rgba(&pixels);
    Ok(DecodedImage { width: info.width as u32, height: info.height as u32, rgba })
}

fn decode_jpeg_bgr(jpeg: &[u8]) -> Result<DecodedImage, CoreError> {
    let mut dec = jpeg_decoder::Decoder::new(jpeg);
    let pixels = dec.decode().map_err(|e| CoreError::Decode(format!("JPEG 1033: {e}")))?;
    let info = dec.info().ok_or_else(|| CoreError::Decode("JPEG 1033: no info".into()))?;
    // BGR → RGB swap, then add alpha.
    let rgba: Vec<u8> = pixels
        .chunks_exact(3)
        .flat_map(|p| [p[2], p[1], p[0], 255])
        .collect();
    Ok(DecodedImage { width: info.width as u32, height: info.height as u32, rgba })
}

fn rgb24_to_rgba(pixels: &[u8]) -> Vec<u8> {
    pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
}

// ── Composite pixel data fallback ─────────────────────────────────────────────

/// Skip Layer & Mask Info section and attempt to decode the Image Data section.
/// Layer & Mask Info length: 4 bytes (PSD) / 8 bytes (PSB).
/// Image Data has NO length field.
fn decode_composite_fallback(
    cur: &mut Cursor<&[u8]>,
    is_psb: bool,
    raw_data: &[u8],
) -> Result<DecodedImage, CoreError> {
    // Skip Layer & Mask Info.
    if is_psb {
        let len = read_u64_be(cur)?;
        cur.seek(SeekFrom::Current(len as i64)).map_err(io_err)?;
    } else {
        let len = read_u32_be(cur)? as u64;
        cur.seek(SeekFrom::Current(len as i64)).map_err(io_err)?;
    }

    // Re-parse channel count, height, width from the fixed-offset file header.
    // Header layout (after 4-byte sig + 2-byte version + 6 reserved):
    //   channels(2) + height(4) + width(4) + depth(2) + color_mode(2) = offset 26
    if raw_data.len() < 26 {
        return Err(CoreError::NoThumbnail);
    }
    let channels = u16::from_be_bytes([raw_data[12], raw_data[13]]) as u32;
    let height = u32::from_be_bytes([raw_data[14], raw_data[15], raw_data[16], raw_data[17]]);
    let width = u32::from_be_bytes([raw_data[18], raw_data[19], raw_data[20], raw_data[21]]);
    let depth = u16::from_be_bytes([raw_data[22], raw_data[23]]) as u32;

    // Only handle 8-bit depth, RGB (3) or RGBA (4) channels.
    if depth != 8 || channels < 3 {
        log::debug!("composite fallback: unsupported depth={depth} channels={channels}");
        return Err(CoreError::NoThumbnail);
    }

    // Cap dimensions to avoid OOM on huge files.
    if width > COMPOSITE_MAX_DIM || height > COMPOSITE_MAX_DIM {
        log::debug!("composite fallback: dimensions {width}×{height} exceed cap; returning NoThumbnail");
        return Err(CoreError::NoThumbnail);
    }

    let compression = read_u16_be(cur)?;
    match compression {
        0 => decode_raw_composite(cur, width, height, channels),
        1 => decode_rle_composite(cur, width, height, channels, is_psb),
        2 | 3 => {
            // ZIP with/without prediction — too expensive for thumbnail; skip.
            log::debug!("composite fallback: ZIP compression not supported; returning NoThumbnail");
            Err(CoreError::NoThumbnail)
        }
        c => Err(CoreError::Decode(format!("unknown composite compression: {c}"))),
    }
}

/// RAW (uncompressed) composite: channels stored planar, 8 bits/channel.
fn decode_raw_composite(
    cur: &mut Cursor<&[u8]>,
    width: u32,
    height: u32,
    channels: u32,
) -> Result<DecodedImage, CoreError> {
    let plane_size = (width * height) as usize;
    let n_planes = channels.min(4) as usize; // R, G, B[, A]

    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(n_planes);
    for _ in 0..n_planes {
        let mut buf = vec![0u8; plane_size];
        cur.read_exact(&mut buf).map_err(io_err)?;
        planes.push(buf);
    }

    let rgba = interleave_planes(&planes, plane_size);
    Ok(DecodedImage { width, height, rgba })
}

/// PackBits RLE composite: row byte-counts precede all channel data.
/// PSB row byte-counts are 4 bytes; PSD are 2 bytes.
fn decode_rle_composite(
    cur: &mut Cursor<&[u8]>,
    width: u32,
    height: u32,
    channels: u32,
    is_psb: bool,
) -> Result<DecodedImage, CoreError> {
    let n_planes = channels.min(4) as usize;
    let total_rows = (height as usize) * n_planes;

    // Read row byte-count table.
    let mut row_byte_counts: Vec<u32> = Vec::with_capacity(total_rows);
    for _ in 0..total_rows {
        let count = if is_psb {
            read_u32_be(cur)?
        } else {
            read_u16_be(cur)? as u32
        };
        row_byte_counts.push(count);
    }

    let plane_size = (width * height) as usize;
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(n_planes);

    for plane in 0..n_planes {
        let mut decoded = Vec::with_capacity(plane_size);
        for row in 0..height as usize {
            let byte_count = row_byte_counts[plane * height as usize + row] as usize;
            let mut rle = vec![0u8; byte_count];
            cur.read_exact(&mut rle).map_err(io_err)?;
            packbits_decode(&rle, &mut decoded)?;
        }
        if decoded.len() < plane_size {
            return Err(CoreError::Decode("RLE decoded data too short".into()));
        }
        decoded.truncate(plane_size);
        planes.push(decoded);
    }

    let rgba = interleave_planes(&planes, plane_size);
    Ok(DecodedImage { width, height, rgba })
}

/// PackBits decompression (Mac/PSD RLE).
fn packbits_decode(input: &[u8], output: &mut Vec<u8>) -> Result<(), CoreError> {
    let mut i = 0;
    while i < input.len() {
        let header = input[i] as i8;
        i += 1;
        if header >= 0 {
            // Literal run: copy (header + 1) bytes.
            let count = header as usize + 1;
            if i + count > input.len() {
                return Err(CoreError::Decode("PackBits: literal overrun".into()));
            }
            output.extend_from_slice(&input[i..i + count]);
            i += count;
        } else if header != -128 {
            // Repeat run: copy the next byte (-header + 1) times.
            let count = (-header) as usize + 1;
            if i >= input.len() {
                return Err(CoreError::Decode("PackBits: repeat overrun".into()));
            }
            let byte = input[i];
            i += 1;
            for _ in 0..count {
                output.push(byte);
            }
        }
        // header == -128 is a NOP in PackBits.
    }
    Ok(())
}

/// Interleave up to 4 planar channels into RGBA.
fn interleave_planes(planes: &[Vec<u8>], plane_size: usize) -> Vec<u8> {
    let mut rgba = vec![255u8; plane_size * 4];
    for (ch, plane) in planes.iter().enumerate().take(4) {
        for px in 0..plane_size {
            rgba[px * 4 + ch] = plane[px];
        }
    }
    rgba
}

// ── Byte-reading helpers ──────────────────────────────────────────────────────

fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8, CoreError> {
    let mut b = [0u8; 1];
    cur.read_exact(&mut b).map_err(io_err)?;
    Ok(b[0])
}

fn read_u16_be(cur: &mut Cursor<&[u8]>) -> Result<u16, CoreError> {
    let mut b = [0u8; 2];
    cur.read_exact(&mut b).map_err(io_err)?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32_be(cur: &mut Cursor<&[u8]>) -> Result<u32, CoreError> {
    let mut b = [0u8; 4];
    cur.read_exact(&mut b).map_err(io_err)?;
    Ok(u32::from_be_bytes(b))
}

fn read_u64_be(cur: &mut Cursor<&[u8]>) -> Result<u64, CoreError> {
    let mut b = [0u8; 8];
    cur.read_exact(&mut b).map_err(io_err)?;
    Ok(u64::from_be_bytes(b))
}

fn io_err(e: std::io::Error) -> CoreError {
    CoreError::Io(e)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_literal() {
        // Header 0x02 = literal run of 3 bytes.
        let input = [0x02u8, 0xAA, 0xBB, 0xCC];
        let mut out = Vec::new();
        packbits_decode(&input, &mut out).unwrap();
        assert_eq!(out, [0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn packbits_repeat() {
        // Header 0xFE = -2 as i8 → repeat next byte 3 times.
        let input = [0xFEu8, 0x55];
        let mut out = Vec::new();
        packbits_decode(&input, &mut out).unwrap();
        assert_eq!(out, [0x55, 0x55, 0x55]);
    }

    #[test]
    fn bad_signature_rejected() {
        let data = b"NOTAPSD\x00\x01";
        assert!(matches!(decode(data, Ext::Psd), Err(CoreError::Decode(_))));
    }

    #[test]
    fn truncated_header_rejected() {
        // Valid sig + version but truncated after that.
        let data = b"8BPS\x00\x01\x00\x00";
        let result = decode(data, Ext::Psd);
        assert!(matches!(result, Err(CoreError::Decode(_) | CoreError::Io(_))));
    }
}
