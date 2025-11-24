use simple_jpegli_enc::{ColorSpace, JpegEncoder};

fn make_cmyk_pixels(width: u16, height: u16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let c = (x * 255 / width) as u8;
            let m = (y * 255 / height) as u8;
            let yv = c.wrapping_add(m) / 2;
            let k = 0u8; // keep black low for simplicity
            buf.push(c);
            buf.push(m);
            buf.push(yv);
            buf.push(k);
        }
    }
    buf
}

fn find_components(jpeg: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i + 10 < jpeg.len() {
        if jpeg[i] == 0xFF && jpeg[i + 1] >= 0xC0 && jpeg[i + 1] <= 0xC3 {
            return Some(jpeg[i + 9]);
        }
        i += 1;
    }
    None
}

#[test]
fn encode_cmyk_baseline() {
    let (w, h) = (16u16, 10u16);
    let pixels = make_cmyk_pixels(w, h);
    let enc = JpegEncoder::new();
    let out = enc
        .encode(&pixels, w, h, ColorSpace::Cmyk, None)
        .expect("encode cmyk baseline");
    assert!(out.starts_with(&[0xFF, 0xD8]));
    let comps = find_components(&out).expect("SOF present");
    assert_eq!(comps, 4, "CMYK should have 4 components");
}

#[test]
fn encode_cmyk_quality_variation() {
    let (w, h) = (24u16, 18u16);
    let pixels = make_cmyk_pixels(w, h);
    let q40 = JpegEncoder::new()
        .quality(40)
        .encode(&pixels, w, h, ColorSpace::Cmyk, None)
        .unwrap();
    let q95 = JpegEncoder::new()
        .quality(95)
        .encode(&pixels, w, h, ColorSpace::Cmyk, None)
        .unwrap();
    assert!(
        q40.len() < q95.len(),
        "Lower quality should reduce size: {} < {}",
        q40.len(),
        q95.len()
    );
}

#[test]
fn encode_cmyk_progressive_toggle() {
    let (w, h) = (32u16, 20u16);
    let pixels = make_cmyk_pixels(w, h);
    let progressive = JpegEncoder::new()
        .progressive(true)
        .encode(&pixels, w, h, ColorSpace::Cmyk, None)
        .unwrap();
    let baseline = JpegEncoder::new()
        .progressive(false)
        .encode(&pixels, w, h, ColorSpace::Cmyk, None)
        .unwrap();
    // Allow either ordering; just ensure they differ in some manner (often size or header).
    assert!(
        progressive != baseline,
        "Progressive and baseline encodes should differ"
    );
}
