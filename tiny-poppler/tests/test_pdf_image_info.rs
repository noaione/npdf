use std::path::PathBuf;

fn open_document(sample: &str) -> tiny_poppler::Document {
    let pdf_path = PathBuf::from(format!("tests/pdf/{sample}"));
    tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document")
}

#[test]
fn test_single_colorspace() {
    let mut doc = open_document("single_colorspace.pdf");
    let pages = doc.page_info().expect("Failed to get page info");

    let page_1 = pages
        .iter()
        .find(|p| p.page == 1)
        .expect("Missing page 1 info");
    assert!(
        page_1.colorspaces.contains_key("CS0"),
        "Expected to find CS0 colorspace entry"
    );

    let cs_space = page_1
        .colorspaces
        .get("CS0")
        .expect("Missing CS0 colorspace");
    match cs_space {
        tiny_poppler::PdfImageColorSpace::Separation { name, alternate } => {
            assert_eq!(name, "All", "Unexpected Separation name");
            assert_eq!(
                alternate.get_type(),
                tiny_poppler::ImageColorSpace::DeviceGray,
                "Unexpected alternate colorspace type"
            );
        }
        _ => panic!("Expected CS0 to be a Separation colorspace"),
    }
}
