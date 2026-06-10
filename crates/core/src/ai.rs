//! .ai file decoder.
//!
//! Modern Illustrator files are PDF 1.x/2.x — pdfium renders page 1.
//! Legacy files are PostScript (`%!PS-Adobe-*`) — check for an embedded
//! `%AI_Thumbnail` block or a DOS-EPS binary preview; fall back to the
//! optional Ghostscript feature, otherwise return `NoThumbnail`.

use crate::{detect, eps, CoreError, DecodedImage};

pub fn decode(data: &[u8], size: u32) -> Result<DecodedImage, CoreError> {
    if detect::is_pdf(data) {
        // Modern .ai: PDF-based. PDFium renders page 1.
        return crate::pdf::render_page1(data, size);
    }

    if detect::is_ps_adobe(data) || detect::is_dos_eps(data) {
        // Legacy .ai: PostScript-based. Try embedded preview paths.
        return decode_ps_ai(data);
    }

    Err(CoreError::Decode("unrecognised .ai format (not PDF or PS-Adobe)".into()))
}

/// Attempt to extract a preview from a PostScript-style .ai file.
///
/// Priority order:
///   1. DOS-EPS binary header → TIFF/WMF preview (reuses eps.rs)
///   2. `%AI_Thumbnail` ASCII-hex embedded block
///   3. Ghostscript fallback (feature `gs-fallback`)
///   4. `NoThumbnail`
fn decode_ps_ai(data: &[u8]) -> Result<DecodedImage, CoreError> {
    // 1. DOS-EPS binary header check (some .ai files have one).
    if detect::is_dos_eps(data) {
        match eps::decode(data) {
            Ok(img) => return Ok(img),
            Err(CoreError::NoThumbnail) => {} // fall through to next strategy
            Err(e) => return Err(e),
        }
    }

    // 2. Scan for `%AI_Thumbnail` embedded ASCII-hex JPEG.
    if let Some(img) = extract_ai_thumbnail(data)? {
        return Ok(img);
    }

    // 3. Optional Ghostscript fallback.
    #[cfg(feature = "gs-fallback")]
    {
        return crate::gs::render(data);
    }

    // 4. No usable preview found.
    Err(CoreError::NoThumbnail)
}

/// Scan the PostScript stream for a `%AI_Thumbnail` block and decode the
/// embedded ASCII-hex JPEG data.
///
/// The block format is:
/// ```text
/// %AI_Thumbnail: <width> <height> <bits>
/// %%BeginData: <n> Hex Bytes
/// <hex digits ...>
/// %%EndData
/// ```
fn extract_ai_thumbnail(data: &[u8]) -> Result<Option<DecodedImage>, CoreError> {
    // Work with the file as a UTF-8 lossy string for line scanning.
    // Files are ASCII PS, so this is safe.
    let text = std::str::from_utf8(data).unwrap_or("");

    let begin_marker = "%%BeginData:";
    let end_marker = "%%EndData";

    let begin_pos = match text.find(begin_marker) {
        Some(p) => p,
        None => return Ok(None),
    };

    // Find the end of the %%BeginData line to locate hex payload start.
    let payload_start = match text[begin_pos..].find('\n') {
        Some(rel) => begin_pos + rel + 1,
        None => return Ok(None),
    };

    let end_pos = match text[payload_start..].find(end_marker) {
        Some(rel) => payload_start + rel,
        None => return Ok(None),
    };

    // Decode the ASCII-hex bytes between BeginData and EndData.
    let hex_text: String = text[payload_start..end_pos]
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();

    if hex_text.len() < 2 {
        return Ok(None);
    }

    let jpeg_bytes: Vec<u8> = hex_text
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            let s = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(s, 16).ok()
        })
        .collect();

    if jpeg_bytes.len() < 3 {
        return Ok(None);
    }

    // Decode the JPEG.
    let mut decoder = jpeg_decoder::Decoder::new(jpeg_bytes.as_slice());
    let pixels = decoder
        .decode()
        .map_err(|e| CoreError::Decode(format!("AI_Thumbnail JPEG: {e}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| CoreError::Decode("AI_Thumbnail: no JPEG info".into()))?;

    let rgba = jpeg_pixels_to_rgba(&pixels, info.pixel_format);
    Ok(Some(DecodedImage {
        width: info.width as u32,
        height: info.height as u32,
        rgba,
    }))
}

/// Convert jpeg-decoder pixel output to RGBA (handles RGB and L/greyscale).
fn jpeg_pixels_to_rgba(pixels: &[u8], fmt: jpeg_decoder::PixelFormat) -> Vec<u8> {
    use jpeg_decoder::PixelFormat;
    match fmt {
        PixelFormat::RGB24 => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        PixelFormat::L8 => pixels
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect(),
        PixelFormat::L16 => pixels
            .chunks_exact(2)
            .flat_map(|p| {
                let g = p[0]; // high byte as approximation
                [g, g, g, 255]
            })
            .collect(),
        PixelFormat::CMYK32 => pixels
            .chunks_exact(4)
            .flat_map(|p| {
                // Naive CMYK→RGB: R=255*(1-C)*(1-K), etc.
                let c = p[0] as f32 / 255.0;
                let m = p[1] as f32 / 255.0;
                let y = p[2] as f32 / 255.0;
                let k = p[3] as f32 / 255.0;
                let r = ((1.0 - c) * (1.0 - k) * 255.0) as u8;
                let g = ((1.0 - m) * (1.0 - k) * 255.0) as u8;
                let b = ((1.0 - y) * (1.0 - k) * 255.0) as u8;
                [r, g, b, 255]
            })
            .collect(),
        #[allow(unreachable_patterns)]
        _ => {
            // Future jpeg-decoder pixel formats: return an opaque grey placeholder.
            let pixel_count = pixels.len() / 3;
            vec![128u8; pixel_count * 4]
        }
    }
}
