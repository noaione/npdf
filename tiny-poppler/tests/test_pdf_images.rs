use std::path::Path;

#[test]
fn test_load_cid_one() {
    let pdf_path = Path::new("tests/pdf/font_cid_1.pdf");

    let mut document = tiny_poppler::Document::open(pdf_path).expect("Failed to open PDF");
    let page_count = document.page_count().expect("Failed to get page count");
    assert_eq!(page_count, 1);

    let page = document
        .image_metadata()
        .expect("Failed to get image metadata");

    assert_eq!(page.images.len(), 0);
    assert_eq!(page.pages.len(), 1);
    let first_page = page.pages.first().expect("No page found");
    assert_eq!(first_page.page, 1);
    assert_eq!(first_page.image_count, 0);
    assert_eq!(first_page.object_count, 18);
}
