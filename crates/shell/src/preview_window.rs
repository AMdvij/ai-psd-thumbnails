//! Win32 window class registration, child-window creation, and WM_PAINT
//! rendering for the preview pane.
//!
//! The window class is registered once per process via `std::sync::Once`.
//!
//! wndproc ↔ object binding contract:
//!   `DoPreview` passes a `*mut PreviewWindowState` as `CreateWindowExW` lpParam.
//!   On WM_NCCREATE the pointer is stored in GWLP_USERDATA.
//!   Every subsequent message reads GWLP_USERDATA and null-checks before use.
//!   On WM_NCDESTROY the slot is cleared to 0 so late messages see null.
//!
//! The wndproc body is wrapped in `catch_unwind(AssertUnwindSafe(..))` so that
//! panics in paint/decode logic never unwind across the FFI boundary into the
//! Windows message pump.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Once;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, HMODULE, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, DeleteDC, EndPaint, FillRect, InvalidateRect,
    SelectObject, StretchBlt, COLOR_WINDOW, HBITMAP, HBRUSH, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    RegisterClassW, SetWindowLongPtrW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, HCURSOR, HMENU, IDC_ARROW, WINDOW_EX_STYLE, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};
use windows::Win32::UI::WindowsAndMessaging::LoadCursorW;

use crate::bitmap::BitmapData;

/// Class name for the preview child window.
const CLASS_NAME: &str = "ThumbViewerPreview";

/// One-time window class registration guard.
static REGISTER_CLASS: Once = Once::new();

// ── PreviewWindowState ────────────────────────────────────────────────────────

/// State shared between the COM object and the window proc via GWLP_USERDATA.
///
/// A raw pointer to this struct is stored in GWLP_USERDATA for the lifetime of
/// the HWND.  The COM object owns the struct and destroys the HWND before or
/// during drop, guaranteeing the pointer is valid whenever the wndproc runs.
pub(crate) struct PreviewWindowState {
    /// Decoded and cached bitmap for WM_PAINT.  None until DoPreview decodes.
    pub(crate) bitmap: Option<BitmapData>,
}

// ── Window class registration ─────────────────────────────────────────────────

/// Register the window class once.  Idempotent after the first call.
pub(crate) fn ensure_window_class_registered(hinstance: HMODULE) {
    REGISTER_CLASS.call_once(|| {
        let class_name_wide: Vec<u16> =
            CLASS_NAME.encode_utf16().chain(std::iter::once(0)).collect();

        let cursor = unsafe {
            LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default())
        };

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(preview_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: Default::default(),
            hCursor: cursor,
            // Default window background brush (COLOR_WINDOW + 1 as HBRUSH).
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
        };

        // Ignore the return value — failure here means the class is already
        // registered (another thread beat us) or the process is out of resources.
        // The Once ensures this runs at most once; subsequent CreateWindowExW
        // calls will use the class whether registered here or previously.
        unsafe { RegisterClassW(&wc) };
    });
}

// ── Child window creation ─────────────────────────────────────────────────────

/// Create the child window parented to `parent_hwnd` at `bounds`.
///
/// `state_ptr` is passed as lpParam so WM_NCCREATE binds it into GWLP_USERDATA.
/// Caller retains ownership of the pointed-to `PreviewWindowState`.
pub(crate) fn create_preview_child(
    parent_hwnd: HWND,
    bounds: RECT,
    hinstance: HMODULE,
    state_ptr: *mut PreviewWindowState,
) -> windows::core::Result<HWND> {
    let class_name_wide: Vec<u16> =
        CLASS_NAME.encode_utf16().chain(std::iter::once(0)).collect();

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name_wide.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            Some(parent_hwnd),
            Some(HMENU::default()),
            Some(hinstance.into()),
            Some(state_ptr as *const std::ffi::c_void),
        )
    }?;

    Ok(hwnd)
}

// ── Window procedure ──────────────────────────────────────────────────────────

/// The window procedure for preview child windows.
///
/// Every message except WM_NCCREATE/WM_NCDESTROY is wrapped in
/// `catch_unwind(AssertUnwindSafe(..))` — a panic maps to DefWindowProcW,
/// never unwinding across the FFI/message-pump boundary.
unsafe extern "system" fn preview_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => handle_nccreate(hwnd, wparam, lparam),
        WM_NCDESTROY => handle_ncdestroy(hwnd),
        WM_PAINT => {
            catch_unwind(AssertUnwindSafe(|| handle_paint(hwnd)))
                .unwrap_or(LRESULT(0))
        }
        WM_SIZE => {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { let _ = InvalidateRect(Some(hwnd), None, true); };
                LRESULT(0)
            }))
            .unwrap_or(LRESULT(0))
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Bind the state pointer from lpParam into GWLP_USERDATA on WM_NCCREATE.
fn handle_nccreate(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let cs = lparam.0 as *const CREATESTRUCTW;
    if !cs.is_null() {
        let state_ptr = unsafe { (*cs).lpCreateParams };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        }
    }
    // Delegate to DefWindowProc to allow default non-client creation.
    unsafe { DefWindowProcW(hwnd, WM_NCCREATE, wparam, lparam) }
}

/// Clear GWLP_USERDATA on WM_NCDESTROY so late messages see null, not freed memory.
fn handle_ncdestroy(hwnd: HWND) -> LRESULT {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        DefWindowProcW(hwnd, WM_NCDESTROY, WPARAM(0), LPARAM(0))
    }
}

/// Paint the cached bitmap centred and fit inside the client area.
fn handle_paint(hwnd: HWND) -> LRESULT {
    // Recover state pointer — null-check mandatory (messages can arrive before
    // WM_NCCREATE binds or after WM_NCDESTROY clears).
    let state_ptr =
        unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut PreviewWindowState;

    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };

    let mut client_rect = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client_rect); }
    let client_w = (client_rect.right - client_rect.left).max(1);
    let client_h = (client_rect.bottom - client_rect.top).max(1);

    let bg_brush = HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _);

    if state_ptr.is_null() {
        // No bound state — fill background and exit paint.
        unsafe { FillRect(hdc, &client_rect, bg_brush) };
        unsafe { let _ = EndPaint(hwnd, &ps); };
        return LRESULT(0);
    }

    // Copy the HBITMAP handle and dimensions OUT of state before any GDI call.
    // Never hold a borrow across a Win32 call — the wndproc can be re-entered.
    let bmp_data: Option<(HBITMAP, u32, u32)> = unsafe {
        let state = &*state_ptr;
        state.bitmap.as_ref().map(|b| (b.hbitmap, b.width, b.height))
    };

    // Fill background first (covers margins around the image too).
    unsafe { FillRect(hdc, &client_rect, bg_brush) };

    if let Some((hbmp, img_w, img_h)) = bmp_data {
        if !hbmp.is_invalid() && img_w > 0 && img_h > 0 {
            let dst = fit_rect(img_w, img_h, client_w as u32, client_h as u32);

            let mem_dc = unsafe { CreateCompatibleDC(Some(hdc)) };
            if !mem_dc.is_invalid() {
                let old_obj = unsafe { SelectObject(mem_dc, hbmp.into()) };

                unsafe {
                    let _ = StretchBlt(
                        hdc,
                        dst.left,
                        dst.top,
                        dst.right - dst.left,
                        dst.bottom - dst.top,
                        Some(mem_dc),
                        0,
                        0,
                        img_w as i32,
                        img_h as i32,
                        SRCCOPY,
                    );
                }

                if !old_obj.is_invalid() {
                    unsafe { SelectObject(mem_dc, old_obj) };
                }
                unsafe { let _ = DeleteDC(mem_dc); };
            }
        }
    }

    unsafe { let _ = EndPaint(hwnd, &ps); };
    LRESULT(0)
}

// ── Layout helpers ────────────────────────────────────────────────────────────

/// Compute a destination RECT fitting `(img_w × img_h)` inside `(area_w × area_h)`,
/// preserving aspect ratio and centering the result.
pub(crate) fn fit_rect(img_w: u32, img_h: u32, area_w: u32, area_h: u32) -> RECT {
    if img_w == 0 || img_h == 0 {
        return RECT::default();
    }
    let scale_x = area_w as f64 / img_w as f64;
    let scale_y = area_h as f64 / img_h as f64;
    let scale = scale_x.min(scale_y);

    let dst_w = ((img_w as f64 * scale).round() as i32).max(1);
    let dst_h = ((img_h as f64 * scale).round() as i32).max(1);
    let offset_x = ((area_w as i32 - dst_w) / 2).max(0);
    let offset_y = ((area_h as i32 - dst_h) / 2).max(0);

    RECT {
        left: offset_x,
        top: offset_y,
        right: offset_x + dst_w,
        bottom: offset_y + dst_h,
    }
}

/// Destroy a child HWND created by `create_preview_child`.
///
/// Clears GWLP_USERDATA first so the WM_DESTROY/WM_NCDESTROY messages emitted
/// by DestroyWindow see a null state pointer (the PreviewProvider is already
/// executing Drop when this is called from the Drop fallback path).
pub(crate) fn destroy_preview_child(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = DestroyWindow(hwnd);
    }
}
