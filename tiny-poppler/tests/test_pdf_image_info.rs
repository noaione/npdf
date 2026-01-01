use std::path::PathBuf;

fn open_document(sample: &str) -> tiny_poppler::Document {
    let pdf_path = PathBuf::from(format!("tests/pdf/{sample}"));
    tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document")
}

#[test]
fn test_single_colorspace() {
    let mut doc = open_document("colorspaces_single.pdf");
    let pages = doc.page_info().expect("Failed to get page info");

    let page_1 = &pages[0];
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

#[test]
fn test_multiple_colorspaces() {
    let mut doc = open_document("colorspaces_multi.pdf");
    let pages = doc.page_info().expect("Failed to get page info");
    let page_1 = &pages[0];

    assert!(
        page_1.colorspaces.contains_key("CS0"),
        "Expected to find CS0 colorspace entry"
    );
    assert!(
        page_1.colorspaces.contains_key("PureBlackCS"),
        "Expected to find PureBlackCS colorspace entry"
    );

    let cs0 = page_1
        .colorspaces
        .get("CS0")
        .expect("Missing CS0 colorspace");
    match cs0 {
        tiny_poppler::PdfImageColorSpace::ICC { alternate } => {
            assert_eq!(
                alternate.get_type(),
                tiny_poppler::ImageColorSpace::DeviceRgb,
                "Unexpected alternate colorspace type for CS0"
            );
        }
        _ => panic!("Expected CS0 to be an ICC colorspace"),
    }

    let pbcs = page_1
        .colorspaces
        .get("PureBlackCS")
        .expect("Missing CS0 colorspace");
    match pbcs {
        tiny_poppler::PdfImageColorSpace::Separation { name, alternate } => {
            assert_eq!(name, "All", "Unexpected Separation name for PureBlackCS");
            assert_eq!(
                alternate.get_type(),
                tiny_poppler::ImageColorSpace::DeviceGray,
                "Unexpected alternate colorspace type for PureBlackCS"
            );
        }
        _ => panic!("Expected PureBlackCS to be a Separation colorspace"),
    }
}
