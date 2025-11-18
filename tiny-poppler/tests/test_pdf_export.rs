use std::path::Path;

use tiny_poppler::RenderOptions;

#[test]
fn test_render_cid_one() {
    let pdf_path = Path::new("tests/pdf/font_cid_1.pdf");

    let mut document = tiny_poppler::Document::open(pdf_path).expect("Failed to open PDF");
    let page_count = document.page_count().expect("Failed to get page count");
    assert_eq!(page_count, 1);

    // export first page
    let exported = document.render_page_png(
        0,
        &RenderOptions {
            dpi: 150.0,
            ..Default::default()
        },
    );
    assert!(exported.is_ok());

    let exported = exported.expect("Failed to export page");
    assert!(!exported.is_empty());

    // check PNG header
    let png_header = &exported[0..8];
    let expected_header = [137, 80, 78, 71, 13, 10, 26, 10];
    assert_eq!(png_header, expected_header);
}
