//! Extension string → `Ext` mapping and file-magic sniff helpers.

use crate::Ext;

/// Map a lowercase file extension (without leading dot) to `Ext`.
/// Returns `None` for unrecognised extensions.
pub fn from_extension(ext: &str) -> Option<Ext> {
    match ext.to_ascii_lowercase().as_str() {
        "ai" => Some(Ext::Ai),
        "psd" => Some(Ext::Psd),
        "psb" => Some(Ext::Psb),
        "eps" => Some(Ext::Eps),
        "svg" => Some(Ext::Svg),
        _ => None,
    }
}

// ── Magic-byte helpers ────────────────────────────────────────────────────────

/// `%PDF` — PDF 1.x / 2.x header (also used by modern .ai files).
pub fn is_pdf(data: &[u8]) -> bool {
    data.starts_with(b"%PDF")
}

/// `%!PS-Adobe` — PostScript / EPS / old Illustrator.
pub fn is_ps_adobe(data: &[u8]) -> bool {
    data.starts_with(b"%!PS-Adobe")
}

/// `8BPS` — Photoshop PSD/PSB signature.
pub fn is_8bps(data: &[u8]) -> bool {
    data.starts_with(b"8BPS")
}

/// DOS-EPS binary header magic: little-endian `0xC5D0D3C6`.
pub fn is_dos_eps(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == [0xC5, 0xD0, 0xD3, 0xC6]
}

/// SVG: starts with `<svg` or `<?xml` (UTF-8 / ASCII subset only; BOM-stripped caller-side).
pub fn is_svg(data: &[u8]) -> bool {
    // Skip optional UTF-8 BOM
    let d = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    // Skip leading whitespace up to 64 bytes so `<?xml ...` files aren't missed.
    let trimmed = d.iter().position(|&b| !b.is_ascii_whitespace())
        .map(|i| &d[i..])
        .unwrap_or(d);
    trimmed.starts_with(b"<svg") || trimmed.starts_with(b"<?xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        assert_eq!(from_extension("ai"), Some(Ext::Ai));
        assert_eq!(from_extension("AI"), Some(Ext::Ai));
        assert_eq!(from_extension("psd"), Some(Ext::Psd));
        assert_eq!(from_extension("psb"), Some(Ext::Psb));
        assert_eq!(from_extension("eps"), Some(Ext::Eps));
        assert_eq!(from_extension("svg"), Some(Ext::Svg));
        assert_eq!(from_extension("txt"), None);
    }

    #[test]
    fn magic_pdf() {
        assert!(is_pdf(b"%PDF-1.6\n"));
        assert!(!is_pdf(b"8BPS"));
    }

    #[test]
    fn magic_ps_adobe() {
        assert!(is_ps_adobe(b"%!PS-Adobe-3.0 EPSF-3.0\n"));
        assert!(!is_ps_adobe(b"%PDF-1.4"));
    }

    #[test]
    fn magic_8bps() {
        assert!(is_8bps(b"8BPS\x00\x01"));
        assert!(!is_8bps(b"%PDF"));
    }

    #[test]
    fn magic_dos_eps() {
        assert!(is_dos_eps(&[0xC5, 0xD0, 0xD3, 0xC6, 0x00]));
        assert!(!is_dos_eps(b"8BPS"));
    }

    #[test]
    fn magic_svg() {
        assert!(is_svg(b"<svg xmlns="));
        assert!(is_svg(b"<?xml version=\"1.0\"?><svg"));
        assert!(is_svg(b"  <svg width=\"100\""));
        assert!(!is_svg(b"%PDF-1.4"));
    }
}
