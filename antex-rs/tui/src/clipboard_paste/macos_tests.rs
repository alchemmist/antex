use std::io::Cursor;

use image::DynamicImage;
use image::ImageBuffer;
use image::ImageFormat;
use image::LumaA;
use pretty_assertions::assert_eq;

#[test]
fn grayscale_alpha_png_decodes_for_clipboard_fallback() {
    let source = DynamicImage::ImageLumaA8(ImageBuffer::from_pixel(3, 2, LumaA([96, 192])));
    let mut png = Vec::new();
    source
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .expect("encode grayscale-alpha PNG");

    let decoded = super::decode_public_png(&png).expect("decode grayscale-alpha PNG");

    assert_eq!(decoded.to_luma_alpha8(), source.to_luma_alpha8());
}
