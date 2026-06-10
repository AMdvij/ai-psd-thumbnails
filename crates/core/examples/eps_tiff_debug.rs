//! Diagnostic harness: decode a raw TIFF preview blob (as extracted from a
//! DOS-EPS header) and print the underlying `tiff` crate error chain instead
//! of the NoThumbnail mapping. Usage: eps_tiff_debug <preview.tif>
use std::io::Cursor;

fn main() {
    let path = std::env::args().nth(1).expect("usage: eps_tiff_debug <preview.tif>");
    let data = std::fs::read(&path).expect("read input");

    let mut decoder = match tiff::decoder::Decoder::new(Cursor::new(data.as_slice())) {
        Ok(d) => d,
        Err(e) => return println!("Decoder::new error: {e}"),
    };
    match decoder.dimensions() {
        Ok((w, h)) => println!("dimensions: {w}x{h}"),
        Err(e) => return println!("dimensions error: {e}"),
    }
    match decoder.colortype() {
        Ok(ct) => println!("colortype: {ct:?}"),
        Err(e) => println!("colortype error (continuing): {e}"),
    }
    match decoder.get_tag(tiff::tags::Tag::PhotometricInterpretation) {
        Ok(v) => println!("photometric: {v:?}"),
        Err(e) => println!("photometric error: {e}"),
    }
    match decoder.get_tag_u16_vec(tiff::tags::Tag::ColorMap) {
        Ok(cm) => println!("colormap entries: {}", cm.len()),
        Err(e) => println!("colormap error: {e}"),
    }
    match decoder.read_image() {
        Ok(r) => println!("read_image ok: {:?}", match r {
            tiff::decoder::DecodingResult::U8(v) => format!("U8 len {}", v.len()),
            _ => "non-U8".into(),
        }),
        Err(e) => println!("read_image error: {e}"),
    }
}
