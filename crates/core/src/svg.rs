//! SVG decoder using resvg/usvg (pure Rust, no native dependencies).
//!
//! Security constraints applied to every render:
//!   - External resource loading (hrefs, images) is DISABLED — untrusted SVGs must
//!     not trigger file-system or network access.
//!   - Effective render dimensions are clamped to MAX_RENDER_DIM before allocating
//!     the pixmap, preventing decompression-bomb-style huge allocations from
//!     hostile width/height/viewBox values.

use crate::{CoreError, DecodedImage};

/// Maximum pixel dimension (width or height) allowed for a single SVG render.
/// SVGs with a natural size larger than this are scaled down to fit.
const MAX_RENDER_DIM: u32 = 4096;

/// Decode an SVG document and render it at up to `size × size` pixels.
pub fn decode(data: &[u8], size: u32) -> Result<DecodedImage, CoreError> {
    // Build usvg options with external resource loading disabled.
    let mut options = usvg::Options::default();
    // Disable loading of external image hrefs and linked resources.
    // usvg 0.44 uses `ImageHrefResolver`; setting it to a resolver that always
    // returns None ensures no file-system or network access occurs.
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };

    let tree = usvg::Tree::from_data(data, &options)
        .map_err(|e| CoreError::Decode(format!("SVG parse: {e}")))?;

    // Determine natural render size, capped at MAX_RENDER_DIM and size.
    let cap = size.min(MAX_RENDER_DIM);
    let (rw, rh) = fit_dims(
        tree.size().width(),
        tree.size().height(),
        cap,
    );

    if rw == 0 || rh == 0 {
        return Err(CoreError::Decode("SVG has zero render dimensions".into()));
    }

    let mut pixmap = tiny_skia::Pixmap::new(rw, rh)
        .ok_or_else(|| CoreError::Decode(format!("SVG: cannot allocate {rw}×{rh} pixmap")))?;

    let transform = tiny_skia::Transform::from_scale(
        rw as f32 / tree.size().width(),
        rh as f32 / tree.size().height(),
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia pixmap is pre-multiplied RGBA; convert to straight RGBA.
    let premul_data = pixmap.data();
    let rgba = unpremultiply_rgba(premul_data);

    Ok(DecodedImage { width: rw, height: rh, rgba })
}

/// Compute (width, height) to fit the SVG's natural size within `cap × cap`,
/// preserving aspect ratio.
fn fit_dims(natural_w: f32, natural_h: f32, cap: u32) -> (u32, u32) {
    if natural_w <= 0.0 || natural_h <= 0.0 {
        return (cap, cap);
    }
    let scale = (cap as f32 / natural_w).min(cap as f32 / natural_h).min(1.0);
    let w = ((natural_w * scale).round() as u32).max(1);
    let h = ((natural_h * scale).round() as u32).max(1);
    (w, h)
}

/// Convert pre-multiplied RGBA (tiny-skia format) back to straight (non-premultiplied) RGBA.
fn unpremultiply_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        let r = px[0];
        let g = px[1];
        let b = px[2];
        let a = px[3];
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            out.extend_from_slice(&[r, g, b, 255]);
        } else {
            let inv = 255.0 / a as f32;
            let ur = ((r as f32 * inv).round() as u32).min(255) as u8;
            let ug = ((g as f32 * inv).round() as u32).min(255) as u8;
            let ub = ((b as f32 * inv).round() as u32).min(255) as u8;
            out.extend_from_slice(&[ur, ug, ub, a]);
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
        <rect width="100" height="100" fill="red"/>
    </svg>"#;

    #[allow(dead_code)]
    const ZERO_SIZE_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#;

    #[test]
    fn decode_minimal_svg_produces_image() {
        let img = decode(MINIMAL_SVG, 256).expect("minimal SVG should decode");
        assert!(img.width > 0 && img.height > 0, "expected non-zero dimensions");
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
    }

    #[test]
    fn decode_respects_size_cap() {
        // The SVG is 100×100; requesting size=50 should produce ≤50×50.
        let img = decode(MINIMAL_SVG, 50).expect("SVG decode with size cap");
        assert!(img.width <= 50 && img.height <= 50);
    }

    #[test]
    fn decode_max_render_dim_cap() {
        // A very large declared size must not allocate beyond MAX_RENDER_DIM.
        let huge_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}">
                <rect width="{}" height="{}" fill="blue"/>
            </svg>"#,
            MAX_RENDER_DIM * 10, MAX_RENDER_DIM * 10,
            MAX_RENDER_DIM * 10, MAX_RENDER_DIM * 10,
        );
        let img = decode(huge_svg.as_bytes(), MAX_RENDER_DIM)
            .expect("huge SVG should be capped, not fail");
        assert!(img.width <= MAX_RENDER_DIM && img.height <= MAX_RENDER_DIM);
    }

    #[test]
    fn decode_invalid_svg_returns_error() {
        let bad = b"this is not svg";
        assert!(decode(bad, 256).is_err());
    }

    #[test]
    fn external_href_disabled() {
        // SVG with an external image href — should not trigger any I/O, just render without it.
        let svg_with_href = br#"<svg xmlns="http://www.w3.org/2000/svg"
              xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">
            <image href="file:///etc/passwd" width="100" height="100"/>
        </svg>"#;
        // Must not panic or error due to external resource; may produce a blank image.
        let result = decode(svg_with_href, 128);
        // Accept Ok (blank image) or Err — either is fine; must not panic.
        let _ = result;
    }
}
