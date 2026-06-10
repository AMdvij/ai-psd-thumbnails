//! CLI harness for manual visual verification.
//!
//! Usage:
//!   cargo run -p ai-psd-thumbnails-core --example dump -- <input-file> <out.png> [size]
//!
//! If PDFIUM_DLL_PATH is set, pdfium-backed formats (.ai PDF, .pdf) will render.
//! Otherwise PDF-based files return NoThumbnail and the tool exits non-zero.

use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: dump <input-file> <out.png> [size=256]");
        process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];
    let size: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(256);

    // Determine extension.
    let ext_str = Path::new(input_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let ext = match ai_psd_thumbnails_core::detect::from_extension(ext_str) {
        Some(e) => e,
        None => {
            eprintln!("Unsupported extension: {ext_str:?}");
            process::exit(2);
        }
    };

    // Initialise pdfium if DLL path is provided.
    if let Ok(dll_path) = std::env::var("PDFIUM_DLL_PATH") {
        if let Err(e) = ai_psd_thumbnails_core::pdf::init(&dll_path) {
            eprintln!("Warning: pdfium init failed: {e}");
        }
    }

    // Read input.
    let data = match std::fs::read(input_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Cannot read {input_path}: {e}");
            process::exit(3);
        }
    };

    eprintln!(
        "Decoding {} ({} bytes) as {:?}, size={size}…",
        input_path,
        data.len(),
        ext
    );

    // Decode thumbnail.
    let img = match ai_psd_thumbnails_core::decode_thumbnail(&data, ext, size) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Decode failed: {e}");
            process::exit(4);
        }
    };

    eprintln!("Decoded: {}×{} RGBA", img.width, img.height);

    // Write PNG output using the `image` crate.
    let png = image::RgbaImage::from_raw(img.width, img.height, img.rgba)
        .expect("RGBA buffer has correct size");
    if let Err(e) = png.save(output_path) {
        eprintln!("Cannot write {output_path}: {e}");
        process::exit(5);
    }

    println!("Wrote {output_path} ({}×{})", img.width, img.height);
}
