//! COM smoke test: exercises the full factory → init → GetThumbnail pipeline.
//!
//! Loads thumb_viewer.dll directly (without registering it) by:
//!   1. LoadLibraryW → our DLL
//!   2. GetProcAddress("DllGetClassObject") → IClassFactory
//!   3. IClassFactory::CreateInstance → IUnknown
//!   4. QueryInterface for IInitializeWithStream
//!   5. Initialize with an in-memory IStream from SHCreateMemStream
//!   6. QueryInterface for IThumbnailProvider
//!   7. GetThumbnail(256) → assert non-null HBITMAP
//!   8. Verify DllCanUnloadNow returns S_FALSE while object is live,
//!      then S_OK after releasing all references.
//!
//! The test uses the release DLL (built by `cargo build --release`).

use std::path::PathBuf;

use windows::core::{Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, S_FALSE, S_OK};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Shell::{SHCreateMemStream, IThumbnailProvider, WTSAT_ARGB};
use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithStream;

/// CLSID_THUMB — must match factory.rs.
const CLSID_THUMB: GUID = GUID::from_values(
    0x90E3822E,
    0x5F70,
    0x40BF,
    [0x88, 0x4E, 0x77, 0x54, 0x47, 0x97, 0xAD, 0x41],
);

/// IID_IClassFactory (known constant).
const IID_ICLASSFACTORY: GUID = GUID::from_values(
    0x00000001,
    0x0000,
    0x0000,
    [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
);

type DllGetClassObjectFn = unsafe extern "system" fn(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT;

type DllCanUnloadNowFn = unsafe extern "system" fn() -> HRESULT;

/// Locate the release DLL next to the test executable or in target/release.
fn dll_path() -> PathBuf {
    // The test binary lives in target/debug/deps/; the release DLL is in target/release/.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    // crates/shell → workspace root
    p.pop(); // shell
    p.pop(); // crates
    p.push("target");
    p.push("release");
    p.push("thumb_viewer.dll");
    p
}

/// Encode a path as null-terminated UTF-16.
fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Minimal SVG bytes used as the stream payload.
/// Raw byte literal avoids Rust treating '#' as an attribute prefix in br#"..."#.
static SVG_BYTES: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\">\
      <rect width=\"64\" height=\"64\" fill=\"green\"/></svg>";

#[test]
fn smoke_factory_init_get_thumbnail_svg() {
    let path = dll_path();
    assert!(
        path.exists(),
        "Release DLL not found at {path:?}. Run `cargo build --release` first."
    );

    let path_str = path.to_string_lossy().into_owned();
    let wide = wide_null(&path_str);

    // ── Step 1: LoadLibraryW ───────────────────────────────────────────────────
    let hmodule: HMODULE = unsafe {
        LoadLibraryW(PCWSTR(wide.as_ptr())).expect("LoadLibraryW failed")
    };

    // ── Step 2: GetProcAddress(DllGetClassObject) ──────────────────────────────
    let proc = unsafe {
        GetProcAddress(hmodule, windows::core::s!("DllGetClassObject"))
            .expect("DllGetClassObject export not found")
    };
    let dll_get_class_object: DllGetClassObjectFn = unsafe { std::mem::transmute(proc) };

    // Also get DllCanUnloadNow for the leak test.
    let can_unload_proc = unsafe {
        GetProcAddress(hmodule, windows::core::s!("DllCanUnloadNow"))
            .expect("DllCanUnloadNow export not found")
    };
    let dll_can_unload_now: DllCanUnloadNowFn = unsafe { std::mem::transmute(can_unload_proc) };

    // ── Step 3: DllGetClassObject → IClassFactory ──────────────────────────────
    let factory: IClassFactory = {
        let mut ppv: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            dll_get_class_object(&CLSID_THUMB, &IID_ICLASSFACTORY, &mut ppv)
        };
        assert!(hr.is_ok(), "DllGetClassObject failed: {hr:?}");
        assert!(!ppv.is_null(), "DllGetClassObject returned null ppv");
        unsafe { IClassFactory::from_raw(ppv) }
    };

    // IClassFactory alone does not increment the object counter; DllCanUnloadNow
    // may return S_OK at this point (factory is stateless, not counted).
    // The ownership-transfer test below (S_FALSE while ThumbProvider is live,
    // S_OK after release) is the authoritative DllCanUnloadNow leak check.

    // ── Step 4: CreateInstance → IInitializeWithStream ────────────────────────
    // windows-rs high-level CreateInstance is type-parameterized: it calls
    // QueryInterface internally and returns the requested interface.
    let init_stream: IInitializeWithStream = unsafe {
        factory.CreateInstance(None).expect("CreateInstance failed")
    };

    // ── Step 5: Initialize with an in-memory IStream containing SVG bytes ─────
    let stream = unsafe {
        SHCreateMemStream(Some(SVG_BYTES)).expect("SHCreateMemStream returned null")
    };
    unsafe {
        init_stream.Initialize(&stream, 0).expect("IInitializeWithStream::Initialize failed");
    }

    // ── Step 6: QueryInterface for IThumbnailProvider ─────────────────────────
    let thumb_provider: IThumbnailProvider = init_stream
        .cast()
        .expect("QueryInterface for IThumbnailProvider failed");

    // Drop init_stream so only thumb_provider holds the object.
    drop(init_stream);

    // ── Step 7: GetThumbnail ───────────────────────────────────────────────────
    let mut hbmp = HBITMAP::default();
    let mut alpha = windows::Win32::UI::Shell::WTS_ALPHATYPE::default();
    unsafe {
        thumb_provider
            .GetThumbnail(256, &mut hbmp, &mut alpha)
            .expect("GetThumbnail failed");
    }
    assert!(!hbmp.is_invalid(), "GetThumbnail returned null HBITMAP");
    assert_eq!(alpha, WTSAT_ARGB, "Expected WTSAT_ARGB alpha type");

    // Clean up the HBITMAP (Explorer owns it in production, but here we must free it).
    unsafe { let _ = DeleteObject(hbmp.into()); }

    // ── Step 8: DllCanUnloadNow leak test ──────────────────────────────────────
    // While thumb_provider and factory are still held, DLL cannot unload.
    let can_unload = unsafe { dll_can_unload_now() };
    assert_eq!(
        can_unload, S_FALSE,
        "DllCanUnloadNow should return S_FALSE while COM objects are live"
    );

    // Release all COM references.
    drop(thumb_provider);
    drop(factory);

    // Now DllCanUnloadNow should return S_OK.
    let can_unload = unsafe { dll_can_unload_now() };
    assert_eq!(
        can_unload, S_OK,
        "DllCanUnloadNow should return S_OK after all objects are released"
    );

    // ── Cleanup ────────────────────────────────────────────────────────────────
    unsafe { let _ = FreeLibrary(hmodule); }
}
