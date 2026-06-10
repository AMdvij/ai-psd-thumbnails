//! MSI custom action executable for ai-psd-thumbnails installer.
//!
//! Dispatches on argv[1]:
//!   restore-ai-progid    — reads HKCR\.ai default value; if it equals the
//!                          SageThumbs leftover string, restores it to the
//!                          standard Adobe Illustrator ProgID. No-op otherwise.
//!                          Scheduled deferred + no-impersonate (SYSTEM/elevated).
//!
//!   notify-assoc-changed — calls SHChangeNotify(SHCNE_ASSOCCHANGED) so Explorer
//!                          refreshes thumbnails and previews without a manual
//!                          cache clear. Scheduled commit + impersonated (user
//!                          session) so the broadcast reaches the running Explorer.
//!
//! Exit codes: 0 = success, 1 = usage error, 2 = action failed.
//! MSI logging: writes to stdout; MSI log captures stdout from deferred CAs.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::process;

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CLASSES_ROOT,
    KEY_READ, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: installer-ca <verb>");
        eprintln!("verbs: restore-ai-progid | notify-assoc-changed");
        process::exit(1);
    }

    match args[1].as_str() {
        "restore-ai-progid" => {
            if let Err(e) = restore_ai_progid() {
                eprintln!("restore-ai-progid failed: {e}");
                process::exit(2);
            }
        }
        "notify-assoc-changed" => {
            notify_assoc_changed();
        }
        other => {
            eprintln!("unknown verb: {other}");
            process::exit(1);
        }
    }
}

// ── restore-ai-progid ─────────────────────────────────────────────────────────

/// Reads `HKCR\.ai` default value.  If it equals the SageThumbs leftover
/// string `"SageThumbsImage.ai"`, overwrites it with `"Adobe.Illustrator.28"`.
/// Any other value (including already-correct or absent) is left untouched.
///
/// Scope: this CA runs as SYSTEM (deferred, no-impersonate), so it operates on
/// the per-machine view of HKCR (i.e. HKLM\SOFTWARE\Classes\.ai).  Per-user
/// leftovers stored in HKCU\Software\Classes\.ai or
/// HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.ai\UserChoice
/// are out of scope and left unrepaired.
///
/// Idempotent: safe to run on repair; safe when Adobe is absent.
fn restore_ai_progid() -> Result<(), Box<dyn std::error::Error>> {
    const SAGETHUMBS_LEFTOVER: &str = "SageThumbsImage.ai";
    const ADOBE_PROGID: &str = "Adobe.Illustrator.28";
    const SUBKEY: &str = ".ai\0";

    // Open HKCR\.ai with read access first to check current value.
    let subkey_wide: Vec<u16> = SUBKEY.encode_utf16().collect();
    let mut hkey = HKEY::default();

    let open_result = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
    };

    if open_result != ERROR_SUCCESS {
        // Key absent — nothing to restore.
        println!("restore-ai-progid: HKCR\\.ai not found, skipping");
        return Ok(());
    }

    // Read the default value (empty name).
    let value_name_wide: Vec<u16> = "\0".encode_utf16().collect();
    let mut buf = vec![0u16; 256];
    let mut buf_bytes = (buf.len() * 2) as u32;
    let mut kind = REG_VALUE_TYPE::default();

    let query_result = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(value_name_wide.as_ptr()),
            None,
            Some(&mut kind as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_bytes),
        )
    };

    unsafe { let _ = RegCloseKey(hkey); };

    if query_result != ERROR_SUCCESS {
        // Cannot read default value — skip.
        println!("restore-ai-progid: cannot read HKCR\\.ai default, skipping");
        return Ok(());
    }

    // Convert buffer to String (REG_SZ, null-terminated wide string).
    let char_count = (buf_bytes / 2) as usize;
    let wide_slice = &buf[..char_count];
    // Strip trailing nulls.
    let wide_trimmed: Vec<u16> = wide_slice
        .iter()
        .copied()
        .take_while(|&c| c != 0)
        .collect();
    let current_value = OsString::from_wide(&wide_trimmed)
        .into_string()
        .unwrap_or_default();

    println!("restore-ai-progid: current HKCR\\.ai default = \"{current_value}\"");

    if current_value != SAGETHUMBS_LEFTOVER {
        println!("restore-ai-progid: value is not SageThumbsImage.ai, no action taken");
        return Ok(());
    }

    // Value matches leftover — open with write access and restore.
    let mut hkey_write = HKEY::default();
    let open_write = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut hkey_write,
        )
    };

    if open_write != ERROR_SUCCESS {
        return Err(format!(
            "RegOpenKeyExW for write failed: {:?}",
            open_write
        )
        .into());
    }

    // Write restored ProgID as REG_SZ.
    let new_value_wide: Vec<u16> = format!("{ADOBE_PROGID}\0").encode_utf16().collect();
    let set_result = unsafe {
        RegSetValueExW(
            hkey_write,
            PCWSTR(value_name_wide.as_ptr()),
            Some(0),
            REG_SZ,
            Some(std::slice::from_raw_parts(
                new_value_wide.as_ptr() as *const u8,
                new_value_wide.len() * 2,
            )),
        )
    };

    unsafe { let _ = RegCloseKey(hkey_write); };

    if set_result != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW failed: {:?}", set_result).into());
    }

    println!(
        "restore-ai-progid: restored HKCR\\.ai default from \"{SAGETHUMBS_LEFTOVER}\" to \"{ADOBE_PROGID}\""
    );
    Ok(())
}

// ── notify-assoc-changed ──────────────────────────────────────────────────────

/// Broadcasts SHCNE_ASSOCCHANGED so Explorer refreshes file type associations,
/// thumbnails, and preview handler assignments without requiring a manual cache
/// clear or reboot.
///
/// This CA is scheduled as commit + impersonated so it runs in the user's
/// interactive session.  A deferred no-impersonate CA would run as SYSTEM in
/// session 0, where SHChangeNotify only broadcasts within session 0 — the user's
/// Explorer would never receive it (silent no-op).
fn notify_assoc_changed() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    println!("notify-assoc-changed: SHChangeNotify(SHCNE_ASSOCCHANGED) sent");
}
