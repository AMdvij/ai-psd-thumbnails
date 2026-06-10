//! Shared helper for draining an IStream into a Vec<u8>.
//!
//! Both ThumbProvider and PreviewProvider need identical stream-draining
//! logic; this module keeps it in one place (DRY).

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::IStream;

/// Maximum stream size accepted to guard against hostile / oversized files (2 GiB).
const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Read-chunk size when draining an IStream.
const READ_CHUNK: u32 = 64 * 1024;

/// Drain an `IStream` into a `Vec<u8>`, up to `MAX_STREAM_BYTES`.
///
/// Loops until `ISequentialStream::Read` reports S_FALSE or returns 0 bytes.
/// Returns `E_FAIL` if the stream is too large or a read error occurs.
pub(crate) fn read_stream_all(stream: &IStream) -> windows::core::Result<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK as usize];

    loop {
        let mut cb_read: u32 = 0;
        let hr = unsafe {
            stream.Read(
                chunk.as_mut_ptr() as *mut std::ffi::c_void,
                READ_CHUNK,
                Some(&mut cb_read),
            )
        };

        if cb_read > 0 {
            let total = data.len() + cb_read as usize;
            if total > MAX_STREAM_BYTES {
                return Err(windows::core::Error::from_hresult(E_FAIL));
            }
            data.extend_from_slice(&chunk[..cb_read as usize]);
        }

        // S_OK (0) means more data may follow; S_FALSE (1) means end-of-stream.
        if hr.is_err() {
            return Err(windows::core::Error::from_hresult(hr));
        }
        if hr.0 != 0 || cb_read == 0 {
            break;
        }
    }

    Ok(data)
}
