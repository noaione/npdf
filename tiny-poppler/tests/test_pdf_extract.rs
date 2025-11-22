use std::path::PathBuf;

use tiny_poppler::ImageExportRequest;

#[test]
fn test_extract_from_rgb8() {
    let pdf_path = PathBuf::from("tests/pdf/image_rgb8.pdf");

    let mut document =
        tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.page, 1);
    assert_eq!(info.width, 200);
    assert_eq!(info.height, 200);
    assert_eq!(info.image_type, tiny_poppler::ImageType::Image);
    assert_eq!(info.components, 3); // R, G, B
    assert_eq!(info.bits_per_component, 8); // 8bit
    assert_eq!(info.dpi, (72.0, 72.0)); // 72 DPI
    match info.xref {
        Some((obj, generation)) => {
            assert_eq!(obj, 4);
            assert_eq!(generation, 0);
        }
        None => panic!("Expected XRef for the image"),
    };

    // Try extracting page
    let extract_page = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
        })
        .expect("Failed to extarct image");

    assert_eq!(extract_page.width, 200);
    assert_eq!(extract_page.height, 200);
    assert_eq!(extract_page.stride, 600);
    assert_eq!(extract_page.components, 3); // R, G, B
    assert_eq!(extract_page.bits_per_component, 8); // 8bit
    assert_eq!(extract_page.format, tiny_poppler::ImageExportFormat::Rgb);
    assert_eq!(
        extract_page.extension,
        tiny_poppler::ImageExportExtension::Png
    );
    assert_eq!(extract_page.width_dpi, 72.0);
    assert_eq!(extract_page.height_dpi, 72.0);
    assert!(extract_page.jbig2_globals.is_none());
    assert!(extract_page.ccitt_params.is_none());

    // Process into sink
    assert!(!extract_page.data.is_empty());

    let sink_image =
        tiny_poppler::sink_exported_image(extract_page).expect("Failed to sink exported image");

    // Peek into data, make sure PNG header is present
    let png_header = &sink_image.bytes[0..8];
    let expected_header = [137, 80, 78, 71, 13, 10, 26, 10];
    assert_eq!(png_header, expected_header);
}
