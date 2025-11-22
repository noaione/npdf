use crate::ffi::{
    CcittParams, ExportedImage, ImageExportExtension, ImageExportFormat, ImageExportType,
};
use png::{BitDepth, ColorType, Compression, Encoder, PixelDimensions, Unit};
use std::fmt::Write as FmtWrite;
use std::io::Cursor;
use thiserror::Error;
use tiff::encoder::Rational;
use tiff::encoder::{self, TiffEncoder, colortype};
use tiff::tags::ResolutionUnit;

#[derive(Debug, Error)]
pub enum ImageSinkError {
    #[error("unsupported sink combination: {format:?} -> {extension:?}")]
    UnsupportedCombination {
        format: ImageExportFormat,
        extension: ImageExportExtension,
    },
    #[error("unsupported raster layout")]
    UnsupportedLayout,
    #[error("png encode: {0}")]
    Png(String),
    #[error("tiff encode: {0}")]
    Tiff(String),
}

#[derive(Debug, Clone)]
pub struct EncodedExportedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub width_dpi: f64,
    pub height_dpi: f64,
    pub format: ImageExportFormat,
    pub image_type: ImageExportType,
    pub extension: ImageExportExtension,
    pub jbig2_globals: Option<Vec<u8>>,
    pub ccitt_params: Option<CcittParams>,
}

impl EncodedExportedImage {
    pub fn file_extension(&self) -> &'static str {
        match self.extension {
            ImageExportExtension::Jpeg => "jpg",
            ImageExportExtension::Jp2 => "jp2",
            ImageExportExtension::Jbig2 => "jb2",
            ImageExportExtension::Ccitt => "ccitt",
            ImageExportExtension::Png => "png",
            ImageExportExtension::Tiff => "tiff",
            ImageExportExtension::Pnm => match self.format {
                ImageExportFormat::Monochrome => "pbm",
                ImageExportFormat::Gray => "pgm",
                ImageExportFormat::Rgb | ImageExportFormat::Rgb48 => "ppm",
                _ => "pnm",
            },
        }
    }
}

pub fn sink_exported_image(image: ExportedImage) -> Result<EncodedExportedImage, ImageSinkError> {
    match image.extension {
        ImageExportExtension::Png => encode_png_image(image),
        ImageExportExtension::Tiff => encode_tiff_image(image),
        ImageExportExtension::Pnm => encode_pnm_image(image),
        _ => Ok(EncodedExportedImage::from_exported(image)),
    }
}

fn encode_png_image(image: ExportedImage) -> Result<EncodedExportedImage, ImageSinkError> {
    let ExportedImage {
        data,
        width,
        height,
        stride,
        components,
        bits_per_component,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        ..
    } = image;

    if width == 0 || height == 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }

    let (color_type, depth) = match (format, bits_per_component) {
        (ImageExportFormat::Rgb, 8) => (ColorType::Rgb, BitDepth::Eight),
        (ImageExportFormat::Rgb48, 16) => (ColorType::Rgb, BitDepth::Sixteen),
        (ImageExportFormat::Gray, 8) => (ColorType::Grayscale, BitDepth::Eight),
        (ImageExportFormat::Gray, 16) => (ColorType::Grayscale, BitDepth::Sixteen),
        (ImageExportFormat::Monochrome, 1) => (ColorType::Grayscale, BitDepth::One),
        _ => return Err(ImageSinkError::UnsupportedCombination { format, extension }),
    };

    let row_bytes = compute_row_bytes(width, components, bits_per_component)?;
    let raster = collect_rows(data, stride as usize, row_bytes, height as usize)?;
    let payload = if matches!(depth, BitDepth::Sixteen) {
        to_big_endian_words(raster)?
    } else {
        raster
    };

    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_compression(Compression::Balanced);
        encoder.set_color(color_type);
        encoder.set_depth(depth);
        if let Some(dimensions) = build_pixel_dims(width_dpi, height_dpi) {
            encoder.set_pixel_dims(Some(dimensions));
        }
        let mut writer = encoder
            .write_header()
            .map_err(|err| ImageSinkError::Png(err.to_string()))?;
        writer
            .write_image_data(&payload)
            .map_err(|err| ImageSinkError::Png(err.to_string()))?;
    }

    Ok(EncodedExportedImage {
        bytes: buffer,
        width,
        height,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        jbig2_globals: None,
        ccitt_params: None,
    })
}

fn encode_tiff_image(image: ExportedImage) -> Result<EncodedExportedImage, ImageSinkError> {
    let ExportedImage {
        data,
        width,
        height,
        stride,
        components,
        bits_per_component,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        ..
    } = image;

    if width == 0 || height == 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }

    let row_bytes = compute_row_bytes(width, components, bits_per_component)?;
    let raster = collect_rows(data, stride as usize, row_bytes, height as usize)?;

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder =
            TiffEncoder::new(&mut cursor).map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
        match (format, bits_per_component) {
            (ImageExportFormat::Rgb, 8) => {
                let mut image = encoder
                    .new_image::<colortype::RGB8>(width, height)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
                apply_resolution(&mut image, width_dpi, height_dpi);
                image
                    .write_data(&raster)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
            }
            (ImageExportFormat::Rgb48, 16) => {
                let samples = into_u16_samples(raster)?;
                let mut image = encoder
                    .new_image::<colortype::RGB16>(width, height)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
                apply_resolution(&mut image, width_dpi, height_dpi);
                image
                    .write_data(&samples)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
            }
            (ImageExportFormat::Gray, 8) => {
                let mut image = encoder
                    .new_image::<colortype::Gray8>(width, height)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
                apply_resolution(&mut image, width_dpi, height_dpi);
                image
                    .write_data(&raster)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
            }
            (ImageExportFormat::Gray, 16) => {
                let samples = into_u16_samples(raster)?;
                let mut image = encoder
                    .new_image::<colortype::Gray16>(width, height)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
                apply_resolution(&mut image, width_dpi, height_dpi);
                image
                    .write_data(&samples)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
            }
            (ImageExportFormat::Cmyk, 8) => {
                let mut image = encoder
                    .new_image::<colortype::CMYK8>(width, height)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
                apply_resolution(&mut image, width_dpi, height_dpi);
                image
                    .write_data(&raster)
                    .map_err(|err| ImageSinkError::Tiff(err.to_string()))?;
            }
            _ => return Err(ImageSinkError::UnsupportedCombination { format, extension }),
        }
    }

    Ok(EncodedExportedImage {
        bytes: cursor.into_inner(),
        width,
        height,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        jbig2_globals: None,
        ccitt_params: None,
    })
}

fn encode_pnm_image(image: ExportedImage) -> Result<EncodedExportedImage, ImageSinkError> {
    let ExportedImage {
        data,
        width,
        height,
        stride,
        components,
        bits_per_component,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        ..
    } = image;

    if width == 0 || height == 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }

    let row_bytes = compute_row_bytes(width, components, bits_per_component)?;
    let raster = collect_rows(data, stride as usize, row_bytes, height as usize)?;

    let mut header = String::new();
    let payload = match (format, bits_per_component) {
        (ImageExportFormat::Monochrome, 1) => {
            write!(&mut header, "P4\n{} {}\n", width, height).unwrap();
            raster
        }
        (ImageExportFormat::Gray, 8) => {
            write!(&mut header, "P5\n{} {}\n255\n", width, height).unwrap();
            raster
        }
        (ImageExportFormat::Gray, 16) => {
            write!(&mut header, "P5\n{} {}\n65535\n", width, height).unwrap();
            to_big_endian_words(raster)?
        }
        (ImageExportFormat::Rgb, 8) => {
            write!(&mut header, "P6\n{} {}\n255\n", width, height).unwrap();
            raster
        }
        (ImageExportFormat::Rgb48, 16) => {
            write!(&mut header, "P6\n{} {}\n65535\n", width, height).unwrap();
            to_big_endian_words(raster)?
        }
        _ => return Err(ImageSinkError::UnsupportedCombination { format, extension }),
    };

    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(&payload);

    Ok(EncodedExportedImage {
        bytes,
        width,
        height,
        width_dpi,
        height_dpi,
        format,
        image_type,
        extension,
        jbig2_globals: None,
        ccitt_params: None,
    })
}

fn compute_row_bytes(
    width: u32,
    components: u32,
    bits_per_component: u32,
) -> Result<usize, ImageSinkError> {
    if width == 0 || components == 0 || bits_per_component == 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    let width = width as usize;
    let components = components as usize;
    let bits = bits_per_component as usize;
    let row_bits = width
        .checked_mul(components)
        .and_then(|value| value.checked_mul(bits))
        .ok_or(ImageSinkError::UnsupportedLayout)?;
    Ok((row_bits + 7) / 8)
}

fn collect_rows(
    data: Vec<u8>,
    stride: usize,
    row_bytes: usize,
    height: usize,
) -> Result<Vec<u8>, ImageSinkError> {
    if height == 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    if stride < row_bytes {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    let required = stride
        .checked_mul(height)
        .ok_or(ImageSinkError::UnsupportedLayout)?;
    if required > data.len() {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    if stride == row_bytes {
        let mut owned = data;
        owned.truncate(required);
        return Ok(owned);
    }
    let mut buffer = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        let end = start + row_bytes;
        buffer.extend_from_slice(&data[start..end]);
    }
    Ok(buffer)
}

fn to_big_endian_words(mut data: Vec<u8>) -> Result<Vec<u8>, ImageSinkError> {
    if data.len() % 2 != 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    for chunk in data.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
    Ok(data)
}

fn into_u16_samples(data: Vec<u8>) -> Result<Vec<u16>, ImageSinkError> {
    if data.len() % 2 != 0 {
        return Err(ImageSinkError::UnsupportedLayout);
    }
    let mut out = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        out.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(out)
}

fn build_pixel_dims(x_dpi: f64, y_dpi: f64) -> Option<PixelDimensions> {
    let xppu = dpi_to_pixels_per_meter(x_dpi)?;
    let yppu = dpi_to_pixels_per_meter(y_dpi)?;
    Some(PixelDimensions {
        xppu,
        yppu,
        unit: Unit::Meter,
    })
}

fn dpi_to_pixels_per_meter(value: f64) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let ppm = (value / 0.0254).round();
    if ppm < 1.0 || ppm > u32::MAX as f64 {
        return None;
    }
    Some(ppm as u32)
}

fn dpi_to_rational(value: f64) -> Option<Rational> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let scaled = (value * 1000.0).round();
    if scaled <= 0.0 || scaled > u32::MAX as f64 {
        return None;
    }
    Some(Rational {
        n: scaled as u32,
        d: 1000,
    })
}

fn apply_resolution<
    W: std::io::Write + std::io::Seek,
    C: colortype::ColorType,
    K: encoder::TiffKind,
>(
    image: &mut encoder::ImageEncoder<'_, W, C, K>,
    x_dpi: f64,
    y_dpi: f64,
) {
    image.resolution_unit(ResolutionUnit::Inch);
    if let Some(value) = dpi_to_rational(x_dpi) {
        image.x_resolution(value);
    }
    if let Some(value) = dpi_to_rational(y_dpi) {
        image.y_resolution(value);
    }
}

impl EncodedExportedImage {
    fn from_exported(image: ExportedImage) -> Self {
        Self {
            bytes: image.data,
            width: image.width,
            height: image.height,
            width_dpi: image.width_dpi,
            height_dpi: image.height_dpi,
            format: image.format,
            image_type: image.image_type,
            extension: image.extension,
            jbig2_globals: image.jbig2_globals,
            ccitt_params: image.ccitt_params,
        }
    }
}
