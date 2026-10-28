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
fn a_text_page_reports_that_it_draws_text() {
    let mut doc = open_document("font_cid_1.pdf");
    let pages = doc.page_info().expect("Failed to get page info");
    assert!(
        pages[0].has_text,
        "font_cid_1.pdf stores a BT/TJ text run, so the collector should have seen it"
    );
}

#[test]
fn an_image_only_page_reports_no_text() {
    let mut doc = open_document("image_luma8.pdf");
    let pages = doc.page_info().expect("Failed to get page info");
    assert!(
        !pages[0].has_text,
        "an image-only page draws no text, so the DPI text floor must not apply"
    );
}

#[test]
fn an_image_carries_the_clip_and_opacity_it_was_drawn_with() {
    let mut doc = open_document("image_rgb8.pdf");
    let page = doc
        .images()
        .expect("Failed to get page images")
        .into_iter()
        .next()
        .expect("image_rgb8.pdf draws one image");
    assert!(
        page.opacity > 0.0 && page.opacity <= 1.0,
        "opacity should be a 0-1 fraction, got {}",
        page.opacity
    );
    if let Some(clip) = page.clip_box {
        let bounds = page.matrix.get_image_aabb();
        assert!(
            clip.x1 <= bounds.x1 && clip.y1 <= bounds.y1,
            "the clip must not start inside the image: clip {clip:?} vs image {bounds:?}"
        );
        assert!(clip.x2 >= bounds.x2 && clip.y2 >= bounds.y2);
    }
}

// #[test]
// fn test_multiple_colorspaces() {
//     let mut doc = open_document("colorspaces_multi.pdf");
//     let pages = doc.page_info().expect("Failed to get page info");
//     let page_1 = &pages[0];

//     assert!(
//         page_1.colorspaces.contains_key("CS0"),
//         "Expected to find CS0 colorspace entry"
//     );
//     assert!(
//         page_1.colorspaces.contains_key("PureBlackCS"),
//         "Expected to find PureBlackCS colorspace entry"
//     );

//     let cs0 = page_1
//         .colorspaces
//         .get("CS0")
//         .expect("Missing CS0 colorspace");
//     match cs0 {
//         tiny_poppler::PdfImageColorSpace::ICC { alternate } => {
//             assert_eq!(
//                 alternate.get_type(),
//                 tiny_poppler::ImageColorSpace::DeviceRgb,
//                 "Unexpected alternate colorspace type for CS0"
//             );
//         }
//         _ => panic!("Expected CS0 to be an ICC colorspace"),
//     }

//     let pbcs = page_1
//         .colorspaces
//         .get("PureBlackCS")
//         .expect("Missing CS0 colorspace");
//     match pbcs {
//         tiny_poppler::PdfImageColorSpace::Separation { name, alternate } => {
//             assert_eq!(name, "All", "Unexpected Separation name for PureBlackCS");
//             assert_eq!(
//                 alternate.get_type(),
//                 tiny_poppler::ImageColorSpace::DeviceGray,
//                 "Unexpected alternate colorspace type for PureBlackCS"
//             );
//         }
//         _ => panic!("Expected PureBlackCS to be a Separation colorspace"),
//     }
// }
