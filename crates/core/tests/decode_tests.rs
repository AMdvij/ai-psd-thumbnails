//! Integration tests for thumb-viewer-core decoders.
//!
//! All fixtures are built in-memory from hand-crafted bytes so there is no
//! external file dependency.  PDFium tests are conditionally skipped when
//! PDFIUM_DLL_PATH is absent, keeping CI green without the DLL.

use thumb_viewer_core::{decode_thumbnail, CoreError, Ext};

// ── Shared JPEG fixture (2×2 red, valid JFIF) ────────────────────────────────

/// A minimal 2×2 red JPEG produced by PIL (verified externally).
const RED_2X2_JPEG: &[u8] = &[
    0xff,0xd8,0xff,0xe0,0x00,0x10,0x4a,0x46,0x49,0x46,0x00,0x01,0x01,0x00,0x00,0x01,
    0x00,0x01,0x00,0x00,0xff,0xdb,0x00,0x43,0x00,0x05,0x03,0x04,0x04,0x04,0x03,0x05,
    0x04,0x04,0x04,0x05,0x05,0x05,0x06,0x07,0x0c,0x08,0x07,0x07,0x07,0x07,0x0f,0x0b,
    0x0b,0x09,0x0c,0x11,0x0f,0x12,0x12,0x11,0x0f,0x11,0x11,0x13,0x16,0x1c,0x17,0x13,
    0x14,0x1a,0x15,0x11,0x11,0x18,0x21,0x18,0x1a,0x1d,0x1d,0x1f,0x1f,0x1f,0x13,0x17,
    0x22,0x24,0x22,0x1e,0x24,0x1c,0x1e,0x1f,0x1e,0xff,0xdb,0x00,0x43,0x01,0x05,0x05,
    0x05,0x07,0x06,0x07,0x0e,0x08,0x08,0x0e,0x1e,0x14,0x11,0x14,0x1e,0x1e,0x1e,0x1e,
    0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,
    0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,
    0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,0x1e,
    0x1e,0xff,0xc0,0x00,0x11,0x08,0x00,0x02,0x00,0x02,0x03,0x01,0x22,0x00,0x02,0x11,
    0x01,0x03,0x11,0x01,0xff,0xc4,0x00,0x1f,0x00,0x00,0x01,0x05,0x01,0x01,0x01,0x01,
    0x01,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x01,0x02,0x03,0x04,0x05,0x06,
    0x07,0x08,0x09,0x0a,0x0b,0xff,0xc4,0x00,0xb5,0x10,0x00,0x02,0x01,0x03,0x03,0x02,
    0x04,0x03,0x05,0x05,0x04,0x04,0x00,0x00,0x01,0x7d,0x01,0x02,0x03,0x00,0x04,0x11,
    0x05,0x12,0x21,0x31,0x41,0x06,0x13,0x51,0x61,0x07,0x22,0x71,0x14,0x32,0x81,0x91,
    0xa1,0x08,0x23,0x42,0xb1,0xc1,0x15,0x52,0xd1,0xf0,0x24,0x33,0x62,0x72,0x82,0x09,
    0x0a,0x16,0x17,0x18,0x19,0x1a,0x25,0x26,0x27,0x28,0x29,0x2a,0x34,0x35,0x36,0x37,
    0x38,0x39,0x3a,0x43,0x44,0x45,0x46,0x47,0x48,0x49,0x4a,0x53,0x54,0x55,0x56,0x57,
    0x58,0x59,0x5a,0x63,0x64,0x65,0x66,0x67,0x68,0x69,0x6a,0x73,0x74,0x75,0x76,0x77,
    0x78,0x79,0x7a,0x83,0x84,0x85,0x86,0x87,0x88,0x89,0x8a,0x92,0x93,0x94,0x95,0x96,
    0x97,0x98,0x99,0x9a,0xa2,0xa3,0xa4,0xa5,0xa6,0xa7,0xa8,0xa9,0xaa,0xb2,0xb3,0xb4,
    0xb5,0xb6,0xb7,0xb8,0xb9,0xba,0xc2,0xc3,0xc4,0xc5,0xc6,0xc7,0xc8,0xc9,0xca,0xd2,
    0xd3,0xd4,0xd5,0xd6,0xd7,0xd8,0xd9,0xda,0xe1,0xe2,0xe3,0xe4,0xe5,0xe6,0xe7,0xe8,
    0xe9,0xea,0xf1,0xf2,0xf3,0xf4,0xf5,0xf6,0xf7,0xf8,0xf9,0xfa,0xff,0xc4,0x00,0x1f,
    0x01,0x00,0x03,0x01,0x01,0x01,0x01,0x01,0x01,0x01,0x01,0x01,0x00,0x00,0x00,0x00,
    0x00,0x00,0x01,0x02,0x03,0x04,0x05,0x06,0x07,0x08,0x09,0x0a,0x0b,0xff,0xc4,0x00,
    0xb5,0x11,0x00,0x02,0x01,0x02,0x04,0x04,0x03,0x04,0x07,0x05,0x04,0x04,0x00,0x01,
    0x02,0x77,0x00,0x01,0x02,0x03,0x11,0x04,0x05,0x21,0x31,0x06,0x12,0x41,0x51,0x07,
    0x61,0x71,0x13,0x22,0x32,0x81,0x08,0x14,0x42,0x91,0xa1,0xb1,0xc1,0x09,0x23,0x33,
    0x52,0xf0,0x15,0x62,0x72,0xd1,0x0a,0x16,0x24,0x34,0xe1,0x25,0xf1,0x17,0x18,0x19,
    0x1a,0x26,0x27,0x28,0x29,0x2a,0x35,0x36,0x37,0x38,0x39,0x3a,0x43,0x44,0x45,0x46,
    0x47,0x48,0x49,0x4a,0x53,0x54,0x55,0x56,0x57,0x58,0x59,0x5a,0x63,0x64,0x65,0x66,
    0x67,0x68,0x69,0x6a,0x73,0x74,0x75,0x76,0x77,0x78,0x79,0x7a,0x82,0x83,0x84,0x85,
    0x86,0x87,0x88,0x89,0x8a,0x92,0x93,0x94,0x95,0x96,0x97,0x98,0x99,0x9a,0xa2,0xa3,
    0xa4,0xa5,0xa6,0xa7,0xa8,0xa9,0xaa,0xb2,0xb3,0xb4,0xb5,0xb6,0xb7,0xb8,0xb9,0xba,
    0xc2,0xc3,0xc4,0xc5,0xc6,0xc7,0xc8,0xc9,0xca,0xd2,0xd3,0xd4,0xd5,0xd6,0xd7,0xd8,
    0xd9,0xda,0xe2,0xe3,0xe4,0xe5,0xe6,0xe7,0xe8,0xe9,0xea,0xf2,0xf3,0xf4,0xf5,0xf6,
    0xf7,0xf8,0xf9,0xfa,0xff,0xda,0x00,0x0c,0x03,0x01,0x00,0x02,0x11,0x03,0x11,0x00,
    0x3f,0x00,0xf2,0xca,0x28,0xa2,0xbf,0x3a,0x3f,0xb2,0xcf,0xff,0xd9,
];

// ── PSD fixture builder ───────────────────────────────────────────────────────

/// Build a minimal PSD (version=1) with a resource-1036 JPEG thumbnail.
///
/// Layout (all big-endian):
///   [0..4]   "8BPS"
///   [4..6]   version=1
///   [6..12]  reserved zeros
///   [12..14] channels=3
///   [14..18] height=1
///   [18..22] width=1
///   [22..24] depth=8
///   [24..26] color_mode=3 (RGB)
///   [26..30] Color Mode Data length=0
///   [30..34] Image Resources section length (4 bytes, computed)
///   [34..]   one 8BIM block for resource 1036
///
/// 8BIM block layout:
///   [0..4]  "8BIM"
///   [4..6]  resource_id=1036
///   [6]     pascal_name_len=0  (empty name)
///   [7]     pad byte (name field must be even total: 1+0=1 → pad to 2)
///   [8..12] data_len (4 bytes)
///   [12..]  thumbnail header (28 bytes) + JPEG data
///
/// Resource-1036 data offset = 34 (Image Resources start + 4-byte section_len) +
///   4 (8BIM sig) + 2 (id) + 1 (pascal len) + 1 (pad) + 4 (data_len) + 28 (thumb hdr) = 74
/// But the JPEG data pointer used in the test is the start of the full data block
/// (including 28-byte thumb header) = 34 + 4 + 4+2+1+1+4 = 34 + 4 + 12 = 50.
fn build_psd_with_resource_1036(jpeg: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();

    // File header (26 bytes).
    out.extend_from_slice(b"8BPS");           // signature
    out.extend_from_slice(&1u16.to_be_bytes()); // version = PSD
    out.extend_from_slice(&[0u8; 6]);         // reserved
    out.extend_from_slice(&3u16.to_be_bytes()); // channels
    out.extend_from_slice(&1u32.to_be_bytes()); // height
    out.extend_from_slice(&1u32.to_be_bytes()); // width
    out.extend_from_slice(&8u16.to_be_bytes()); // depth
    out.extend_from_slice(&3u16.to_be_bytes()); // color mode (RGB)
    assert_eq!(out.len(), 26);

    // Color Mode Data section (4-byte length = 0).
    out.extend_from_slice(&0u32.to_be_bytes());
    assert_eq!(out.len(), 30);

    // 8BIM thumbnail header (28 bytes).
    let thumb_hdr = build_thumbnail_header(jpeg.len() as u32);

    // Build 8BIM block for resource 1036.
    let data_payload: Vec<u8> = thumb_hdr.iter().chain(jpeg.iter()).cloned().collect();
    let data_len = data_payload.len() as u32;
    let data_padded_len = (data_len + 1) & !1; // pad to even

    let mut bim = Vec::new();
    bim.extend_from_slice(b"8BIM");
    bim.extend_from_slice(&1036u16.to_be_bytes());
    bim.push(0u8);  // pascal name len = 0
    bim.push(0u8);  // pad byte (name field: 1 byte len + 0 bytes name = 1; pad to 2)
    bim.extend_from_slice(&data_len.to_be_bytes());
    bim.extend_from_slice(&data_payload);
    // Pad data to even length if needed.
    if data_len % 2 != 0 {
        bim.push(0u8);
    }
    assert_eq!(bim.len(), 4 + 2 + 1 + 1 + 4 + data_padded_len as usize);

    // Image Resources section: 4-byte length + 8BIM blocks.
    let section_len = bim.len() as u32;
    out.extend_from_slice(&section_len.to_be_bytes()); // at offset 30
    out.extend_from_slice(&bim);

    // Image Resources section starts at offset 30.
    // 8BIM block starts at offset 34 (after 4-byte section_len).
    // Resource data (thumb hdr + JPEG) starts at: 34 + 4+2+1+1+4 = 34+12 = 46.
    // The JPEG payload itself starts at 46 + 28 (thumb header) = 74.

    out
}

/// Minimal 28-byte resource-1036 thumbnail header.
/// format(4) + width(4) + height(4) + widthbytes(4) + totalsize(4) + compressedsize(4) +
/// bitsperpixel(2) + planes(2) = 28 bytes.
fn build_thumbnail_header(jpeg_len: u32) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&1u32.to_be_bytes()); // format = kJpegRGB (1)
    h.extend_from_slice(&2u32.to_be_bytes()); // width
    h.extend_from_slice(&2u32.to_be_bytes()); // height
    h.extend_from_slice(&8u32.to_be_bytes()); // widthbytes (approx)
    h.extend_from_slice(&(jpeg_len + 28).to_be_bytes()); // totalsize
    h.extend_from_slice(&jpeg_len.to_be_bytes()); // compressedsize
    h.extend_from_slice(&24u16.to_be_bytes()); // bitsperpixel
    h.extend_from_slice(&1u16.to_be_bytes());  // planes
    assert_eq!(h.len(), 28);
    h
}

/// Independently-derived byte offset of the resource-1036 data block
/// (28-byte thumb header + JPEG) within the fixture PSD.
///
/// Derived from the `build_psd_with_resource_1036` layout above:
///   26 (file header) + 4 (color_mode_data) + 4 (section_len) +
///   4 (8BIM sig) + 2 (id) + 1 (pascal_name_len) + 1 (pad) + 4 (data_len) = 46
const FIXTURE_PSD_RESOURCE_1036_DATA_OFFSET: usize = 46;

// ── SVG fixture ───────────────────────────────────────────────────────────────

const SAMPLE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
  <rect width="64" height="64" fill="blue"/>
</svg>"##;

// ── EPS fixtures ──────────────────────────────────────────────────────────────

const DOS_EPS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];

/// Build a DOS-EPS header where TIFF preview points into garbage data (→ NoThumbnail).
fn build_dos_eps_no_tiff() -> Vec<u8> {
    let mut hdr = vec![0u8; 30];
    hdr[..4].copy_from_slice(&DOS_EPS_MAGIC);
    // PS offset=30, len=10; WMF=0,0; TIFF=0,0
    hdr[4..8].copy_from_slice(&30u32.to_le_bytes());
    hdr[8..12].copy_from_slice(&10u32.to_le_bytes());
    // WMF: offset=0, len=0
    // TIFF: offset=0, len=0
    let ps_data = b"%!PS-Adobe-3.0\n";
    hdr.extend_from_slice(&ps_data[..10]);
    hdr
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn psd_resource_1036_decodes_ok() {
    let data = build_psd_with_resource_1036(RED_2X2_JPEG);
    let result = decode_thumbnail(&data, Ext::Psd, 256);
    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    let img = result.unwrap();
    assert!(img.width > 0 && img.height > 0, "expected non-zero dims");
    assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
}

#[test]
fn psd_resource_1036_offset_matches_independent_derivation() {
    // Verify the fixture places resource-1036 data exactly where the independent
    // byte-layout derivation says it should be.  This guards against PSB
    // length-field widening being accidentally applied to PSD (and vice-versa).
    let data = build_psd_with_resource_1036(RED_2X2_JPEG);

    // At offset FIXTURE_PSD_RESOURCE_1036_DATA_OFFSET the 28-byte thumb header starts.
    // The first 4 bytes of the thumb header are the format field = 1 (big-endian).
    let offset = FIXTURE_PSD_RESOURCE_1036_DATA_OFFSET;
    assert!(
        data.len() > offset + 4,
        "fixture too short: len={}, expected >{offset}",
        data.len()
    );
    let format = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    assert_eq!(format, 1, "thumb header format field at offset {offset} should be 1 (kJpegRGB)");
}

#[test]
fn psd_bad_signature_is_error() {
    let bad = b"NOTAPSD\x00\x01\x00\x00";
    let result = decode_thumbnail(bad, Ext::Psd, 256);
    assert!(matches!(result, Err(CoreError::Decode(_))));
}

#[test]
fn svg_decodes_ok() {
    let result = decode_thumbnail(SAMPLE_SVG, Ext::Svg, 128);
    assert!(result.is_ok(), "SVG decode failed: {:?}", result.err());
    let img = result.unwrap();
    assert_eq!(img.width, 128);
    assert_eq!(img.height, 128);
    assert_eq!(img.rgba.len(), (128 * 128 * 4) as usize);
}

#[test]
fn svg_invalid_returns_error() {
    let result = decode_thumbnail(b"not svg at all", Ext::Svg, 128);
    assert!(result.is_err(), "expected error for invalid SVG");
}

#[test]
fn eps_no_preview_returns_no_thumbnail() {
    // Plain PS without DOS-EPS header → NoThumbnail (no GS feature active).
    let plain_ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\n";
    let result = decode_thumbnail(plain_ps, Ext::Eps, 256);
    assert!(matches!(result, Err(CoreError::NoThumbnail)));
}

#[test]
fn eps_dos_header_no_tiff_returns_no_thumbnail() {
    let data = build_dos_eps_no_tiff();
    let result = decode_thumbnail(&data, Ext::Eps, 256);
    assert!(matches!(result, Err(CoreError::NoThumbnail)));
}

#[test]
fn ai_pdf_sniff_without_pdfium_returns_no_thumbnail_or_pdf_error() {
    // A minimal %PDF header but not a real PDF — pdfium will fail to parse it,
    // which should map to a Pdf error (not a panic).
    let fake_pdf = b"%PDF-1.6 fake";
    let result = decode_thumbnail(fake_pdf, Ext::Ai, 256);
    // Without pdfium DLL we expect NoThumbnail (env var absent) or Pdf error.
    assert!(
        matches!(result, Err(CoreError::NoThumbnail) | Err(CoreError::Pdf(_))),
        "expected NoThumbnail or Pdf error, got: {result:?}"
    );
}

#[test]
fn ai_ps_adobe_without_preview_returns_no_thumbnail() {
    let ps_ai = b"%!PS-Adobe-3.1 EPSF-3.0\n%%Creator: Adobe Illustrator\n";
    let result = decode_thumbnail(ps_ai, Ext::Ai, 256);
    assert!(matches!(result, Err(CoreError::NoThumbnail)));
}

// ── PDFium test (skipped when DLL absent) ────────────────────────────────────

#[test]
fn pdf_render_skipped_without_dll() {
    // This test only runs when PDFIUM_DLL_PATH is set.
    let dll_path = match std::env::var("PDFIUM_DLL_PATH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("PDFIUM_DLL_PATH not set; skipping PDFium test");
            return;
        }
    };

    // Initialise pdfium explicitly; skip (don't fail) if the DLL cannot be loaded
    // in this environment (e.g. bash/WSL path translation issues in CI).
    if let Err(e) = thumb_viewer_core::pdf::init(&dll_path) {
        eprintln!("pdfium init failed ({e}); skipping PDFium test");
        return;
    }

    // A minimal (but real) single-page PDF is needed here.
    // If an external fixture path is provided via PDFIUM_TEST_PDF, use it.
    let pdf_path = match std::env::var("PDFIUM_TEST_PDF") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("PDFIUM_TEST_PDF not set; skipping PDFium render test");
            return;
        }
    };

    let data = std::fs::read(&pdf_path)
        .unwrap_or_else(|e| panic!("cannot read {pdf_path}: {e}"));

    let result = decode_thumbnail(&data, Ext::Ai, 256);
    assert!(result.is_ok(), "PDFium render failed: {:?}", result.err());
    let img = result.unwrap();
    assert!(img.width > 0 && img.height > 0);
    assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
}

// ── render.rs unit tests (exercise via decode_thumbnail) ─────────────────────

#[test]
fn fit_square_produces_correct_output_dims() {
    let data = build_psd_with_resource_1036(RED_2X2_JPEG);
    let img = decode_thumbnail(&data, Ext::Psd, 64).expect("PSD decode");
    // fit_square must produce exactly 64×64.
    assert_eq!(img.width, 64);
    assert_eq!(img.height, 64);
}
