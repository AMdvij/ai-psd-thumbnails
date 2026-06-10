fn main() {
    let path = std::env::args().nth(1).unwrap();
    println!("Trying to load: {path}");
    unsafe {
        // Use winapi-style via raw windows call
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr());
        println!("Handle: {handle}");
        if handle == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            println!("Error: {err}");
        }
    }
}
