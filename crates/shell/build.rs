// build.rs — links exports.def to guarantee undecorated DLL entry points.
// Without the .def file, MSVC may mangle or omit exports, causing regsvr32 to
// fail with "entry point not found" at runtime.
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let def_path = format!("{manifest_dir}\\exports.def");
    println!("cargo:rustc-cdylib-link-arg=/DEF:{def_path}");
}
