//! Ghostscript fallback for EPS / EPS-style .ai files without any embedded
//! preview (no DOS-EPS TIFF, no %AI_Thumbnail, no XMP thumbnail).
//!
//! Strategy: spawn the user's installed Ghostscript console executable
//! (`gswin64c.exe` / `gswin32c.exe`) as an isolated subprocess rendering to a
//! temp PPM file, then parse the PPM. Ghostscript is AGPL — it is NEVER
//! bundled, linked, or loaded in-process; we only execute a copy the user
//! already installed ("mere aggregation"). The subprocess boundary also means
//! hostile PostScript can at worst kill/hang the child (which we time out and
//! kill), never the Explorer surrogate hosting this DLL.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::{CoreError, DecodedImage};

/// Wall-clock budget for one Ghostscript render before the child is killed.
const GS_TIMEOUT: Duration = Duration::from_secs(15);

/// Target size in pixels for the longest output dimension; the caller's
/// `fit_square` (box filter) does the final downscale.
const TARGET_PX: f64 = 512.0;

/// Render the first page of an EPS/PS document via the installed Ghostscript.
///
/// Returns `NoThumbnail` when Ghostscript is not installed or rendering fails
/// — both are expected, non-fatal outcomes for the thumbnail pipeline.
pub fn render(data: &[u8]) -> Result<DecodedImage, CoreError> {
    #[cfg(not(feature = "gs-fallback"))]
    {
        let _ = data;
        Err(CoreError::NoThumbnail)
    }
    #[cfg(feature = "gs-fallback")]
    {
        render_via_gs(data)
    }
}

#[cfg(feature = "gs-fallback")]
fn render_via_gs(data: &[u8]) -> Result<DecodedImage, CoreError> {
    let gs = ghostscript_exe().ok_or(CoreError::NoThumbnail)?;

    // Unique temp file names: pid + monotonic counter (multiple shell threads
    // may render concurrently in one surrogate process).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tag = format!(
        "thumb-viewer-gs-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = std::env::temp_dir();
    let in_path = tmp.join(format!("{tag}.eps"));
    let out_path = tmp.join(format!("{tag}.ppm"));

    std::fs::write(&in_path, data)?;
    let result = run_gs(gs, &in_path, &out_path, resolution_for(data));

    // Best-effort cleanup regardless of outcome.
    let _ = std::fs::remove_file(&in_path);
    let ppm = std::fs::read(&out_path);
    let _ = std::fs::remove_file(&out_path);

    result?;
    parse_ppm_p6(&ppm.map_err(|_| CoreError::NoThumbnail)?)
}

/// Spawn Ghostscript with hardened flags and wait with a kill-on-timeout loop.
#[cfg(feature = "gs-fallback")]
fn run_gs(gs: &Path, input: &Path, output: &Path, dpi: u32) -> Result<(), CoreError> {
    let mut cmd = Command::new(gs);
    cmd.arg("-dSAFER") // sandboxed file access
        .arg("-dBATCH")
        .arg("-dNOPAUSE")
        .arg("-dNOPROMPT")
        .arg("-sDEVICE=ppmraw") // trivially parseable, no extra image deps
        .arg(format!("-r{dpi}"))
        .arg("-dEPSCrop") // crop to %%BoundingBox instead of full page
        .arg("-dFirstPage=1")
        .arg("-dLastPage=1")
        .arg("-o")
        .arg(output)
        .arg(input)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Never flash a console window inside Explorer's surrogate.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        log::debug!("gs-fallback: spawn {gs:?} failed: {e}");
        CoreError::NoThumbnail
    })?;

    let deadline = Instant::now() + GS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                log::debug!("gs-fallback: ghostscript exited {status}");
                return Err(CoreError::NoThumbnail);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    log::debug!("gs-fallback: ghostscript timed out, killed");
                    return Err(CoreError::NoThumbnail);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                log::debug!("gs-fallback: wait failed: {e}");
                return Err(CoreError::NoThumbnail);
            }
        }
    }
}

/// Pick a render resolution so the longest %%BoundingBox dimension lands near
/// `TARGET_PX` pixels. Default 96 dpi when no parseable BoundingBox exists;
/// clamped so giant canvases cannot demand multi-gigabyte renders.
#[cfg(feature = "gs-fallback")]
fn resolution_for(data: &[u8]) -> u32 {
    let head_len = data.len().min(8192);
    let head = String::from_utf8_lossy(&data[..head_len]);
    for line in head.lines() {
        if let Some(rest) = line.strip_prefix("%%BoundingBox:") {
            let nums: Vec<f64> = rest
                .split_whitespace()
                .filter_map(|t| t.parse::<f64>().ok())
                .collect();
            if nums.len() == 4 {
                let w = (nums[2] - nums[0]).abs();
                let h = (nums[3] - nums[1]).abs();
                let max_pt = w.max(h);
                if max_pt > 1.0 {
                    let dpi = (TARGET_PX * 72.0 / max_pt).round();
                    return dpi.clamp(12.0, 300.0) as u32;
                }
            }
        }
    }
    96
}

/// Locate the installed Ghostscript console executable, preferring 64-bit.
/// Searched once per process and cached.
///
/// Order: `GS_PROG` env override → standard install dirs
/// (`Program Files[ (x86)]\gs\gs*\bin`) → PATH.
#[cfg(feature = "gs-fallback")]
fn ghostscript_exe() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            if let Ok(p) = std::env::var("GS_PROG") {
                let p = PathBuf::from(p);
                if p.is_file() {
                    return Some(p);
                }
            }

            let mut candidates: Vec<PathBuf> = Vec::new();
            for (root, exe) in [
                ("C:\\Program Files\\gs", "gswin64c.exe"),
                ("C:\\Program Files (x86)\\gs", "gswin32c.exe"),
                ("C:\\Program Files (x86)\\gs", "gswin64c.exe"),
            ] {
                if let Ok(entries) = std::fs::read_dir(root) {
                    for entry in entries.flatten() {
                        let exe_path = entry.path().join("bin").join(exe);
                        if exe_path.is_file() {
                            candidates.push(exe_path);
                        }
                    }
                }
            }
            // Highest version (lexicographic on "gs10.05.1"-style dir names is
            // good enough across same-width versions) and 64-bit first because
            // Program Files\gs candidates were pushed first.
            candidates.sort();
            if let Some(best) = candidates.last() {
                return Some(best.clone());
            }

            // PATH fallback.
            let path_var = std::env::var_os("PATH")?;
            for dir in std::env::split_paths(&path_var) {
                for exe in ["gswin64c.exe", "gswin32c.exe"] {
                    let p = dir.join(exe);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
            None
        })
        .as_deref()
}

/// Parse a binary PPM (P6, maxval 255) into RGBA.
#[cfg(feature = "gs-fallback")]
fn parse_ppm_p6(ppm: &[u8]) -> Result<DecodedImage, CoreError> {
    let mut pos = 0usize;

    // Token reader skipping whitespace and `#` comment lines.
    let mut next_token = |ppm: &[u8]| -> Option<String> {
        loop {
            while pos < ppm.len() && ppm[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos < ppm.len() && ppm[pos] == b'#' {
                while pos < ppm.len() && ppm[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            break;
        }
        let start = pos;
        while pos < ppm.len() && !ppm[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos > start {
            Some(String::from_utf8_lossy(&ppm[start..pos]).into_owned())
        } else {
            None
        }
    };

    let bad = || CoreError::Decode("ghostscript PPM output malformed".into());

    if next_token(ppm).as_deref() != Some("P6") {
        return Err(bad());
    }
    let width: u32 = next_token(ppm).and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    let height: u32 = next_token(ppm).and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    let maxval: u32 = next_token(ppm).and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    if maxval != 255 || width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(bad());
    }
    pos += 1; // single whitespace byte after maxval, then raw RGB data

    let need = (width as usize) * (height as usize) * 3;
    let pixels = ppm.get(pos..pos + need).ok_or_else(bad)?;

    let rgba = pixels
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    Ok(DecodedImage { width, height, rgba })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "gs-fallback"))]
mod tests {
    use super::*;

    #[test]
    fn ppm_p6_with_comment_parses() {
        // 2×1 image: red, blue. Comment line between tokens.
        let ppm = b"P6\n# created by test\n2 1\n255\n\xFF\x00\x00\x00\x00\xFF";
        let img = parse_ppm_p6(ppm).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn ppm_truncated_data_is_error() {
        let ppm = b"P6\n2 2\n255\n\xFF\x00\x00";
        assert!(parse_ppm_p6(ppm).is_err());
    }

    #[test]
    fn ppm_wrong_magic_is_error() {
        assert!(parse_ppm_p6(b"P5\n1 1\n255\n\x00").is_err());
    }

    #[test]
    fn bounding_box_drives_resolution() {
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1024 512\n";
        // 1024 pt longest side -> 512px target -> 36 dpi.
        assert_eq!(resolution_for(eps), 36);
        // No bounding box -> default.
        assert_eq!(resolution_for(b"%!PS\n"), 96);
    }

    /// End-to-end render through the real Ghostscript install; skips silently
    /// on machines without Ghostscript (same pattern as the pdfium tests).
    #[test]
    fn renders_minimal_eps_when_ghostscript_present() {
        if ghostscript_exe().is_none() {
            eprintln!("skipping: no ghostscript installed");
            return;
        }
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\n\
                    1 0 0 setrgbcolor 0 0 72 72 rectfill showpage\n";
        let img = render(eps).expect("gs render");
        assert!(img.width > 0 && img.height > 0);
        // Centre pixel should be red-ish.
        let c = (((img.height / 2) * img.width + img.width / 2) * 4) as usize;
        assert!(img.rgba[c] > 200 && img.rgba[c + 1] < 50);
    }
}
