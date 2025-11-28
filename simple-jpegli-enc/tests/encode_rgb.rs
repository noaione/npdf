use simple_jpegli_enc::{ColorSpace, JpegEncoder, Subsampling};

fn make_rgb_pixels(width: u16, height: u16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255 / width) as u8).wrapping_add((y * 10) as u8);
            let g = ((y * 255 / height) as u8).wrapping_add((x * 5) as u8);
            let b = r ^ g ^ 0xAA; // some variation
            buf.push(r);
            buf.push(g);
            buf.push(b);
        }
    }
    buf
}

fn find_components(jpeg: &[u8]) -> Option<u8> {
    // Search for SOF0/1/2 marker (FFC0/FFC1/FFC2)
    let mut i = 0;
    while i + 10 < jpeg.len() {
        if jpeg[i] == 0xFF && jpeg[i + 1] >= 0xC0 && jpeg[i + 1] <= 0xC3 {
            // Baseline/progressive markers range (simplified)
            return Some(jpeg[i + 9]);
        }
        i += 1;
    }
    None
}

#[test]
fn encode_rgb_baseline() {
    let (w, h) = (16u16, 12u16);
    let pixels = make_rgb_pixels(w, h);
    let enc = JpegEncoder::new();
    let out = enc
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .expect("encode rgb baseline");
    assert!(out.starts_with(&[0xFF, 0xD8]), "must start with SOI");
    let comps = find_components(&out).expect("SOF marker present");
    assert_eq!(comps, 3, "RGB should have 3 components");
}

#[test]
fn encode_rgb_quality_variation() {
    let (w, h) = (32u16, 24u16);
    let pixels = make_rgb_pixels(w, h);
    let low = JpegEncoder::new()
        .quality(30)
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .unwrap();
    let high = JpegEncoder::new()
        .quality(90)
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .unwrap();
    assert!(
        low.len() < high.len(),
        "Lower quality should generally shrink size: {} < {}",
        low.len(),
        high.len()
    );
}

#[test]
fn encode_rgb_subsampling_none_vs_auto() {
    let (w, h) = (40u16, 30u16);
    let pixels = make_rgb_pixels(w, h);
    let auto = JpegEncoder::new()
        .quality(80)
        .subsampling(Subsampling::Auto)
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .unwrap();
    let none = JpegEncoder::new()
        .quality(80)
        .subsampling(Subsampling::None)
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .unwrap();
    // Just ensure both encodings succeed and produce non-empty JPEGs.
    assert!(auto.starts_with(&[0xFF, 0xD8]) && none.starts_with(&[0xFF, 0xD8]));
    assert!(auto.len() > 0 && none.len() > 0);
}

#[test]
fn encode_rgb_buffer_mismatch_error() {
    let (w, h) = (8u16, 8u16);
    let pixels = vec![0u8; (w as usize) * (h as usize) * 3 - 1]; // one byte short
    let enc = JpegEncoder::new();
    let err = enc
        .encode(&pixels, w, h, ColorSpace::Rgb, None)
        .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("Input buffer size mismatch"));
}
