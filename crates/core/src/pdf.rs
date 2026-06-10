//! PDF page-1 → RGBA via pdfium-render.
//!
//! pdfium is bound ONCE per process via `OnceLock`. The `thread_safe` feature on
//! pdfium-render wraps every FFI call in a global mutex, providing process-wide
//! call serialization required because multiple STA threads can share this instance.
//!
//! The pdfium.dll path is supplied via the `PDFIUM_DLL_PATH` environment variable
//! (tests) or injected by the shell crate at startup (co-located with the DLL).

use std::sync::OnceLock;

use pdfium_render::prelude::*;

use crate::{CoreError, DecodedImage};

// ── Process-wide pdfium singleton ────────────────────────────────────────────

static PDFIUM: OnceLock<Pdfium> = OnceLock::new();

/// Initialise the pdfium singleton from a DLL path.
///
/// Safe to call multiple times; subsequent calls with a different path are ignored
/// (the first bind wins for the lifetime of the process).
pub fn init(dll_path: &str) -> Result<(), CoreError> {
    if PDFIUM.get().is_some() {
        return Ok(());
    }
    let bindings = Pdfium::bind_to_library(
        Pdfium::pdfium_platform_library_name_at_path(dll_path),
    )
    .map_err(|e| CoreError::Pdf(format!("bind pdfium at {dll_path:?}: {e}")))?;
    // `set` returns Err if another thread already initialised; both outcomes are fine.
    let _ = PDFIUM.set(Pdfium::new(bindings));
    Ok(())
}

/// Attempt to auto-initialise from the `PDFIUM_DLL_PATH` environment variable.
/// Returns `CoreError::NoThumbnail` (not a hard error) if the variable is absent,
/// so callers can gracefully degrade when the DLL is not present.
fn ensure_init() -> Result<(), CoreError> {
    if PDFIUM.get().is_some() {
        return Ok(());
    }
    match std::env::var("PDFIUM_DLL_PATH") {
        Ok(path) => init(&path),
        Err(_) => Err(CoreError::NoThumbnail),
    }
}

// ── Public render API ─────────────────────────────────────────────────────────

/// Render page 0 (first page) of a PDF document supplied as raw bytes.
/// Returns a `DecodedImage` at the requested `size` × `size` bounding box.
pub fn render_page1(data: &[u8], size: u32) -> Result<DecodedImage, CoreError> {
    ensure_init()?;

    let pdfium = PDFIUM.get().expect("pdfium initialised above");

    let doc = pdfium
        .load_pdf_from_byte_slice(data, None)
        .map_err(|e| CoreError::Pdf(format!("load PDF: {e}")))?;

    let page = doc
        .pages()
        .get(0)
        .map_err(|e| CoreError::Pdf(format!("get page 0: {e}")))?;

    // Compute render dimensions preserving aspect ratio within the size × size square.
    let page_width = page.width().value;
    let page_height = page.height().value;
    let (rw, rh) = if page_width <= 0.0 || page_height <= 0.0 {
        (size, size)
    } else {
        let scale = (size as f32) / page_width.max(page_height);
        let w = ((page_width * scale) as u32).max(1);
        let h = ((page_height * scale) as u32).max(1);
        (w, h)
    };

    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(rw as i32)
                .set_target_height(rh as i32),
        )
        .map_err(|e| CoreError::Pdf(format!("render page: {e}")))?;

    // as_rgba_bytes() normalises BGRA/BGRx → straight RGBA regardless of
    // the internal pdfium bitmap format.
    let rgba = bitmap.as_rgba_bytes();
    let expected = (rw * rh * 4) as usize;
    if rgba.len() != expected {
        return Err(CoreError::Pdf(format!(
            "unexpected bitmap size: {} (expected {expected})",
            rgba.len(),
        )));
    }

    Ok(DecodedImage { width: rw, height: rh, rgba })
}
