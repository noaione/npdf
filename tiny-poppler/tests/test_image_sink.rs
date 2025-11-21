use tiny_poppler::{
    ExportedImage, ImageExportExtension, ImageExportFormat, ImageExportType, sink_exported_image,
};

fn base_image() -> ExportedImage {
    ExportedImage {
        data: Vec::new(),
        width: 0,
        height: 0,
        stride: 0,
        components: 0,
        bits_per_component: 0,
        width_dpi: 72.0,
        height_dpi: 72.0,
        format: ImageExportFormat::Unknown,
        image_type: ImageExportType::Image,
        extension: ImageExportExtension::Png,
        jbig2_globals: None,
        ccitt_params: None,
    }
}

#[test]
fn sink_png_includes_phys_chunk() {
    let mut image = base_image();
    image.data = vec![255, 0, 0, 0, 255, 0];
    image.width = 2;
    image.height = 1;
    image.stride = 6;
    image.components = 3;
    image.bits_per_component = 8;
    image.format = ImageExportFormat::Rgb;
    image.extension = ImageExportExtension::Png;

    let encoded = sink_exported_image(image).expect("png sink failed");
    assert_eq!(&encoded.bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    assert!(
        encoded.bytes.windows(4).any(|chunk| chunk == b"pHYs"),
        "missing pHYs chunk"
    );
    assert_eq!(encoded.file_extension(), "png");
}

#[test]
fn sink_tiff_writes_header() {
    let mut image = base_image();
    image.data = vec![0, 0, 0, 0];
    image.width = 1;
    image.height = 1;
    image.stride = 4;
    image.components = 4;
    image.bits_per_component = 8;
    image.format = ImageExportFormat::Cmyk;
    image.extension = ImageExportExtension::Tiff;

    let encoded = sink_exported_image(image).expect("tiff sink failed");
    assert!(encoded.bytes.len() > 4);
    assert_eq!(&encoded.bytes[..4], b"II*\0");
    assert_eq!(encoded.file_extension(), "tiff");
}

#[test]
fn sink_pnm_bitmap_layout() {
    let mut image = base_image();
    image.data = vec![0b1010_0000];
    image.width = 8;
    image.height = 1;
    image.stride = 1;
    image.components = 1;
    image.bits_per_component = 1;
    image.format = ImageExportFormat::Monochrome;
    image.extension = ImageExportExtension::Pnm;

    let encoded = sink_exported_image(image).expect("pnm sink failed");
    let header = b"P4\n8 1\n";
    assert!(encoded.bytes.starts_with(header));
    assert_eq!(&encoded.bytes[header.len()..], &[0b1010_0000]);
    assert_eq!(encoded.file_extension(), "pbm");
}
