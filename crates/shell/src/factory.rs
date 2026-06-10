//! IClassFactory + DllGetClassObject / DllCanUnloadNow.
//!
//! CLSID_THUMB is the fixed identity of this thumbnail provider — generated
//! 2026-06-10, recorded in plan.md.  It must match the InprocServer32 registry
//! entry exactly.
//!
//! Ownership-transfer contract (windows-rs issue #1819):
//!   DllGetClassObject builds a `Factory`, converts it to `IClassFactory`, then
//!   calls `Interface::query(riid, ppv)`.  The query method calls QueryInterface
//!   which AddRefs into *ppv and the local `IClassFactory` is then dropped —
//!   the net result is exactly one reference counted to the caller.
//!
//!   CreateInstance follows the same pattern: reject aggregation, build
//!   ThumbProvider, convert to IUnknown, QueryInterface to the requested riid.

use std::sync::atomic::{AtomicI32, Ordering};

use windows::core::{implement, Interface, IUnknown, GUID, HRESULT};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

use crate::thumbnail::ThumbProvider;

// ── CLSID-A — {90E3822E-5F70-40BF-884E-77544797AD41} ─────────────────────────

/// Thumbnail provider class ID — generated 2026-06-10, fixed for the lifetime
/// of the product.  Must match the InprocServer32 and ShellEx registry values.
pub const CLSID_THUMB: GUID = GUID::from_values(
    0x90E3822E,
    0x5F70,
    0x40BF,
    [0x88, 0x4E, 0x77, 0x54, 0x47, 0x97, 0xAD, 0x41],
);

// ── Global COM counters ───────────────────────────────────────────────────────

/// Count of live ThumbProvider objects created by this DLL.
/// Incremented in ThumbProvider::new(), decremented in Drop.
pub(crate) static OBJ_COUNT: AtomicI32 = AtomicI32::new(0);

/// Count of outstanding IClassFactory::LockServer(TRUE) calls.
static LOCK_COUNT: AtomicI32 = AtomicI32::new(0);

// ── IClassFactory ─────────────────────────────────────────────────────────────

#[implement(IClassFactory)]
pub(crate) struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows::core::Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        // Aggregation is not supported — COM shell never aggregates thumbnail providers.
        if !punkouter.is_null() {
            return Err(windows::core::Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        if ppvobject.is_null() || riid.is_null() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }

        // Build the provider, convert to IUnknown, QueryInterface to the requested
        // interface.  The caller receives one AddRef'd pointer; the local IUnknown drops.
        let provider = ThumbProvider::new();
        let unknown: IUnknown = provider.into();
        unsafe { unknown.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, flock: windows::core::BOOL) -> windows::core::Result<()> {
        if flock.as_bool() {
            LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCK_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

// ── DLL exports ───────────────────────────────────────────────────────────────

/// Called by COM to obtain a class factory.
///
/// Ownership transfer: Factory → IClassFactory → query(riid, ppv).
/// The query call AddRefs into *ppv; the local IClassFactory is dropped.
/// Net effect: exactly one reference delivered to the caller, none leaked.
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() || rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = std::ptr::null_mut();

        if *rclsid != CLSID_THUMB {
            return CLASS_E_CLASSNOTAVAILABLE;
        }

        let factory = Factory;
        let cf: IClassFactory = factory.into();
        cf.query(riid, ppv)
    }
}

/// Returns S_OK when no live objects and no LockServer calls are outstanding,
/// meaning the DLL can safely be unloaded.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    let can_unload =
        OBJ_COUNT.load(Ordering::SeqCst) == 0 && LOCK_COUNT.load(Ordering::SeqCst) == 0;
    if can_unload { S_OK } else { S_FALSE }
}
