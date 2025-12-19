use std::path::PathBuf;

use tiny_poppler::ImageExportRequest;

const FLOAT_TOLERANCE: f64 = 1e-5;

fn open_document(sample: &str) -> tiny_poppler::Document {
    let pdf_path = PathBuf::from(format!("tests/pdf/{sample}"));
    tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document")
}

fn assert_close(actual: f64, expected: f64, label: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= FLOAT_TOLERANCE,
        "{} expected {} but got {} (delta = {})",
        label,
        expected,
        actual,
        delta
    );
}

#[test]
fn test_describe_from_rgb8() {
    let mut document = open_document("image_rgb8.pdf");
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

    // Try extracting page with describe_only
    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.stride, 600);
    assert_eq!(exported.components, 3); // R, G, B
    assert_eq!(exported.bits_per_component, 8); // 8bit
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Rgb);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
    assert_eq!(exported.width_dpi, 72.0);
    assert_eq!(exported.height_dpi, 72.0);
    assert!(exported.jbig2_globals.is_none());
    assert!(exported.ccitt_params.is_none());
}

#[test]
fn test_describe_from_jbig2_with_globals() {
    let pdf_path = PathBuf::from("tests/pdf/image_jbig2_withglobals.pdf");

    let mut document =
        tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.page, 1);
    assert_eq!(info.width, 1747);
    assert_eq!(info.height, 2554);
    assert_eq!(info.image_type, tiny_poppler::ImageType::Image);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceGray
    ));
    assert_close(info.dpi.0, 300.0, "metadata width DPI");
    assert_close(info.dpi.1, 300.0, "metadata height DPI");

    let (obj, generation) = info.xref.expect("Expected XRef for the image");
    assert_eq!(obj, 799);
    assert_eq!(generation, 0);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: obj,
                generation,
            },
            describe_only: true,
        })
        .expect("Failed to describe JBIG2 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 1747);
    assert_eq!(exported.height, 2554);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(
        exported.extension,
        tiny_poppler::ImageExportExtension::Jbig2
    );
    assert_eq!(exported.width_dpi, 72.0);
    assert_eq!(exported.height_dpi, 72.0);
    assert!(exported.ccitt_params.is_none());

    // JBIG2 globals should still be available
    let globals = exported
        .jbig2_globals
        .as_ref()
        .expect("Expected JBIG2 globals to be present");
    assert_eq!(globals.len(), 0);
}

#[test]
fn test_describe_from_ccitt_group3() {
    let pdf_path = PathBuf::from("tests/pdf/image_ccit_3.pdf");

    let mut document =
        tiny_poppler::Document::open(&pdf_path).expect("Failed to open PDF document");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.page, 1);
    assert_eq!(info.width, 1451);
    assert_eq!(info.height, 2528);
    assert_eq!(info.image_type, tiny_poppler::ImageType::Stencil);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::Unknown
    ));
    assert_close(info.dpi.0, 300.0, "metadata width DPI");
    assert_close(info.dpi.1, 300.0, "metadata height DPI");

    let (obj, generation) = info.xref.expect("Expected XRef for the image");
    assert_eq!(obj, 8);
    assert_eq!(generation, 0);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: obj,
                generation,
            },
            describe_only: true,
        })
        .expect("Failed to describe CCITT image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 1451);
    assert_eq!(exported.height, 2528);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(
        exported.extension,
        tiny_poppler::ImageExportExtension::Ccitt
    );
    assert_eq!(exported.width_dpi, 72.0);
    assert_eq!(exported.height_dpi, 72.0);
    assert!(exported.jbig2_globals.is_none());

    let params = exported
        .ccitt_params
        .as_ref()
        .expect("Expected CCITT parameters to be present");
    assert_eq!(params.encoding, -1); // Group 3 1D
    assert_eq!(params.columns, 1451);
    assert_eq!(params.rows, 2528);
    assert_eq!(params.damaged_rows_before_error, 0);
    assert!(!params.end_of_line);
    assert!(!params.byte_align);
    assert!(params.end_of_block);
    assert!(!params.black_is_one);
}

#[test]
fn test_describe_from_rgba8_with_softmask() {
    let mut document = open_document("image_rgba8.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 2);

    let image = &images.images[0];
    assert_eq!(image.page, 1);
    assert_eq!(image.width, 200);
    assert_eq!(image.height, 200);
    assert_eq!(image.components, 3);
    assert_eq!(image.bits_per_component, 8);
    assert_eq!(image.image_type, tiny_poppler::ImageType::Image);
    assert!(matches!(
        image.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceRGB
    ));
    assert_eq!(image.xref, Some((5, 0)));

    let soft_mask = &images.images[1];
    assert_eq!(soft_mask.page, 1);
    assert_eq!(soft_mask.width, 200);
    assert_eq!(soft_mask.height, 200);
    assert_eq!(soft_mask.components, 1);
    assert_eq!(soft_mask.bits_per_component, 8);
    assert_eq!(soft_mask.image_type, tiny_poppler::ImageType::SoftMask);
    assert!(matches!(
        soft_mask.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceGray
    ));

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 5,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe RGBA8 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.stride, 600);
    assert_eq!(exported.components, 3);
    assert_eq!(exported.bits_per_component, 8);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Rgb);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
    assert_eq!(exported.width_dpi, 72.0);
    assert_eq!(exported.height_dpi, 72.0);
    assert!(exported.jbig2_globals.is_none());
    assert!(exported.ccitt_params.is_none());
}

#[test]
fn test_describe_from_rgba16_with_softmask() {
    let mut document = open_document("image_rgba16.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 2);

    let image = &images.images[0];
    assert_eq!(image.width, 200);
    assert_eq!(image.height, 200);
    assert_eq!(image.components, 3);
    assert_eq!(image.bits_per_component, 16);
    assert_eq!(image.image_type, tiny_poppler::ImageType::Image);
    assert_eq!(image.xref, Some((5, 0)));

    let soft_mask = &images.images[1];
    assert_eq!(soft_mask.image_type, tiny_poppler::ImageType::SoftMask);
    assert_eq!(soft_mask.components, 1);
    assert_eq!(soft_mask.bits_per_component, 16);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 5,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe RGBA16 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.stride, 1_200);
    assert_eq!(exported.components, 3);
    assert_eq!(exported.bits_per_component, 16);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Rgb48);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
    assert_eq!(exported.width_dpi, 72.0);
    assert_eq!(exported.height_dpi, 72.0);
}

#[test]
fn test_describe_from_rgb16() {
    let mut document = open_document("image_rgb16.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 200);
    assert_eq!(info.height, 200);
    assert_eq!(info.components, 3);
    assert_eq!(info.bits_per_component, 16);
    assert_eq!(info.image_type, tiny_poppler::ImageType::Image);
    assert_eq!(info.xref, Some((4, 0)));

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe RGB16 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.components, 3);
    assert_eq!(exported.bits_per_component, 16);
    assert_eq!(exported.stride, 1_200);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Rgb48);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
}

#[test]
fn test_describe_from_cmyk_jpeg() {
    let mut document = open_document("image_cmyk_jpg.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 200);
    assert_eq!(info.height, 200);
    assert_eq!(info.components, 4);
    assert_eq!(info.bits_per_component, 8);
    assert_eq!(info.image_type, tiny_poppler::ImageType::Image);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceCMYK
    ));
    assert_eq!(info.xref, Some((4, 0)));

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe CMYK image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.components, 4);
    assert_eq!(exported.bits_per_component, 8);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Jpeg);
}

#[test]
fn test_describe_from_luma8() {
    let mut document = open_document("image_luma8.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 200);
    assert_eq!(info.height, 200);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 8);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceGray
    ));

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe luma8 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 8);
    assert_eq!(exported.stride, 200);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Gray);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
}

#[test]
fn test_describe_from_luma16() {
    let mut document = open_document("image_luma16.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 200);
    assert_eq!(info.height, 200);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 16);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 4,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe luma16 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 200);
    assert_eq!(exported.height, 200);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 8);
    assert_eq!(exported.stride, 200);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Gray);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
}

#[test]
fn test_describe_from_one_bit_gray() {
    let mut document = open_document("image_1_bit_per_component.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 256);
    assert_eq!(info.height, 256);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceGray
    ));
    assert_close(info.dpi.0, 71.99100112485938, "metadata width dpi");
    assert_close(info.dpi.1, 71.99100112485938, "metadata height dpi");

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 6,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe 1-bit image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 256);
    assert_eq!(exported.height, 256);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.stride, 32);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Monochrome);
    assert_eq!(exported.extension, tiny_poppler::ImageExportExtension::Png);
}

#[test]
fn test_describe_from_inline_ccitt() {
    let mut document = open_document("image_inline_2.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 138);
    assert_eq!(info.height, 130);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);
    assert!(matches!(
        info.colorspace,
        tiny_poppler::PdfImageColorSpace::DeviceGray
    ));
    assert!(info.xref.is_none());

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::NthOfType { occurrence: 0 },
            describe_only: true,
        })
        .expect("Failed to describe inline CCITT image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 138);
    assert_eq!(exported.height, 130);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(
        exported.extension,
        tiny_poppler::ImageExportExtension::Ccitt
    );

    let params = exported
        .ccitt_params
        .as_ref()
        .expect("Expected CCITT params for inline image");
    assert_eq!(params.encoding, -1);
    assert_eq!(params.columns, 138);
    assert_eq!(params.rows, 130);
    assert!(params.end_of_block);
    assert!(!params.end_of_line);
    assert!(!params.byte_align);
    assert!(!params.black_is_one);
}

#[test]
fn test_describe_from_ccitt_group1() {
    let mut document = open_document("image_ccit_1.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 415);
    assert_eq!(info.height, 314);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 8,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe CCITT Group 1 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 415);
    assert_eq!(exported.height, 314);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(
        exported.extension,
        tiny_poppler::ImageExportExtension::Ccitt
    );

    let params = exported
        .ccitt_params
        .as_ref()
        .expect("Expected CCITT parameters to be present");
    assert_eq!(params.encoding, -1); // Group 1
    assert_eq!(params.columns, 415);
    assert_eq!(params.rows, 314);
    assert_eq!(params.damaged_rows_before_error, 0);
    assert!(!params.end_of_line);
    assert!(!params.byte_align);
    assert!(params.end_of_block);
    assert!(!params.black_is_one);
}

#[test]
fn test_describe_from_ccitt_group4() {
    let mut document = open_document("image_ccit_4.pdf");
    let images = document
        .image_metadata()
        .expect("Failed to get image metadata");
    assert_eq!(images.images.len(), 1);

    let info = &images.images[0];
    assert_eq!(info.width, 2336);
    assert_eq!(info.height, 2857);
    assert_eq!(info.components, 1);
    assert_eq!(info.bits_per_component, 1);

    let exported = document
        .export_image(ImageExportRequest {
            page_index: 0,
            target_type: tiny_poppler::ImageExportType::Image,
            selector: tiny_poppler::ImageExportSelector::Reference {
                object: 8,
                generation: 0,
            },
            describe_only: true,
        })
        .expect("Failed to describe CCITT Group 4 image");

    assert!(exported.data.is_empty()); // No data should be returned in describe_only mode
    assert_eq!(exported.width, 2336);
    assert_eq!(exported.height, 2857);
    assert_eq!(exported.components, 1);
    assert_eq!(exported.bits_per_component, 1);
    assert_eq!(exported.stride, 0);
    assert_eq!(exported.format, tiny_poppler::ImageExportFormat::Unknown);
    assert_eq!(
        exported.extension,
        tiny_poppler::ImageExportExtension::Ccitt
    );

    let params = exported
        .ccitt_params
        .as_ref()
        .expect("Expected CCITT parameters to be present");
    assert_eq!(params.encoding, -1); // Group 4
    assert_eq!(params.columns, 2336);
    assert_eq!(params.rows, 2857);
    assert_eq!(params.damaged_rows_before_error, 0);
    assert!(!params.end_of_line);
    assert!(!params.byte_align);
    assert!(params.end_of_block);
    assert!(!params.black_is_one);
}
