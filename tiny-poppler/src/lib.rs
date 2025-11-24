//! Tiny interface for rendering PDF pages to PNG/JPEG via Poppler's Splash backend.
//!
//! This crate offers a small API so the CLI can convert PDF pages to PNG/JPEG files
//! without linking against Poppler's GLib/Cairo layers. Under the hood we build
//! Poppler (with Splash) as part of the Cargo build.

mod ffi;
mod helpers;
mod sink;

pub use ffi::{
    CcittParams, ColorMode, ExportedImage, ImageExportExtension, ImageExportFormat,
    ImageExportRequest, ImageExportSelector, ImageExportType, ImageInfo, ImageType, PageInfo,
    PdfCropMode, PdfImageColorSpace,
};
use png::{BitDepth, ColorType, Encoder};
use simple_jpegli_enc::{ColorSpace as JpegColorType, JpegEncoder, JpegError};
pub use sink::{
    EncodedExportedImage, ImageSinkError, ImageSinkOptions, PngCompression, TiffCompression,
    TiffDeflateLevel, sink_exported_image,
};

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

/// Configuration for a render operation.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub dpi: f64,
    pub color_mode: ColorMode,
    pub crop_mode: PdfCropMode,
    pub jpeg_quality: Option<u8>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            color_mode: ColorMode::Rgb8,
            crop_mode: PdfCropMode::CropBox,
            jpeg_quality: Some(95),
        }
    }
}

/// Optional owner/user passwords used when opening encrypted PDFs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfPasswords {
    pub owner: Option<String>,
    pub user: Option<String>,
}

impl PdfPasswords {
    pub fn new(owner: Option<String>, user: Option<String>) -> Self {
        Self { owner, user }
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.owner.is_none() && self.user.is_none()
    }
}

/// Encoded image formats supported by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

/// Encoded image data plus format metadata.
#[derive(Debug, Clone)]
pub struct EncodedImage {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
}

impl EncodedImage {
    pub fn extension(&self) -> &'static str {
        self.format.extension()
    }
}

/// Combined image + page metadata snapshot returned from Poppler.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    pub images: Vec<ImageInfo>,
    pub pages: Vec<PageInfo>,
}

impl From<ffi::ImageCollection> for ImageMetadata {
    fn from(value: ffi::ImageCollection) -> Self {
        Self {
            images: value.images,
            pages: value.pages,
        }
    }
}

/// Errors surfaced by the renderer.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("poppler: {0}")]
    Poppler(String),
    #[error("png encode: {0}")]
    Png(String),
    #[error("jpeg encode: {0:?}")]
    Jpeg(#[from] JpegError),
    #[error("image conversion: {0}")]
    Image(String),
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("unsupported image layout returned from renderer")]
    UnsupportedLayout,
    #[error("unsupported color mode {0:?}")]
    UnsupportedColorMode(ColorMode),
    #[error("invalid u32 size: {0}")]
    InvalidU32Size(usize),
    #[error("invalid jpeg dimension ({side}): {value}")]
    InvalidJpegDimension { side: &'static str, value: usize },
    #[error("image encoded as {actual:?} but {expected:?} was requested")]
    UnexpectedImageFormat {
        expected: ImageFormat,
        actual: ImageFormat,
    },
}

#[derive(Debug, Error)]
pub enum ImageExportError {
    #[error("poppler: {0}")]
    Poppler(String),
    #[error(transparent)]
    Sink(#[from] ImageSinkError),
}

const DEFAULT_JPEG_QUALITY: u8 = 95;

/// Handle to an open PDF backed by Poppler's Splash renderer.
pub struct Document {
    renderer: ffi::Renderer,
}

impl Document {
    /// Open a PDF file and prepare the Splash renderer. Expensive operations (like parsing
    /// the document) happen here, so callers should reuse the returned `Document` when
    /// rendering multiple pages.
    pub fn open(pdf_path: &Path) -> Result<Self, RenderError> {
        let renderer = ffi::Renderer::open(pdf_path).map_err(RenderError::Poppler)?;
        Ok(Self { renderer })
    }

    /// Open a PDF file using the provided owner/user password combination.
    pub fn open_with_passwords(
        pdf_path: &Path,
        passwords: Option<&PdfPasswords>,
    ) -> Result<Self, RenderError> {
        let renderer = ffi::Renderer::open_with_passwords(
            pdf_path,
            passwords.and_then(|bundle| bundle.owner()),
            passwords.and_then(|bundle| bundle.user()),
        )
        .map_err(RenderError::Poppler)?;
        Ok(Self { renderer })
    }

    /// Return the number of pages in the document.
    pub fn page_count(&self) -> Result<u32, RenderError> {
        self.renderer.page_count().map_err(RenderError::Poppler)
    }

    /// Render a page and return the raw bitmap produced by Splash.
    pub fn render_page_image(
        &mut self,
        page_index: u32,
        options: &RenderOptions,
    ) -> Result<ffi::Image, RenderError> {
        self.renderer
            .render_page(
                page_index,
                options.dpi,
                options.color_mode,
                options.crop_mode,
            )
            .map_err(RenderError::Poppler)
    }

    /// Render a page to encoded image bytes (PNG for RGB/Gray, JPEG for CMYK/DeviceN).
    pub fn render_page_image_bytes(
        &mut self,
        page_index: u32,
        options: &RenderOptions,
    ) -> Result<EncodedImage, RenderError> {
        let image = self.render_page_image(page_index, options)?;
        encode_image(
            &image,
            options.jpeg_quality.unwrap_or(DEFAULT_JPEG_QUALITY),
            options.dpi,
        )
    }

    /// Render a page to PNG bytes using the configured color mode.
    pub fn render_page_png(
        &mut self,
        page_index: u32,
        options: &RenderOptions,
    ) -> Result<Vec<u8>, RenderError> {
        let encoded = self.render_page_image_bytes(page_index, options)?;
        match encoded.format {
            ImageFormat::Png => Ok(encoded.bytes),
            other => Err(RenderError::UnexpectedImageFormat {
                expected: ImageFormat::Png,
                actual: other,
            }),
        }
    }

    /// Render a page directly to a PNG file on disk.
    pub fn render_page_to_png(
        &mut self,
        page_index: u32,
        output_path: &Path,
        options: &RenderOptions,
    ) -> Result<(), RenderError> {
        fs::create_dir_all(output_path.parent().unwrap_or_else(|| Path::new(".")))?;
        let png_bytes = self.render_page_png(page_index, options)?;
        let mut file = BufWriter::new(File::create(output_path)?);
        file.write_all(&png_bytes)?;
        file.flush()?;
        Ok(())
    }

    /// Extract an embedded image using Poppler's exporter without re-encoding it.
    pub fn export_image(
        &mut self,
        request: ImageExportRequest,
    ) -> Result<ExportedImage, ImageExportError> {
        self.renderer
            .export_image(request)
            .map_err(ImageExportError::Poppler)
    }

    /// Extract an embedded image and encode it into the requested sink format.
    pub fn export_image_to_bytes(
        &mut self,
        request: ImageExportRequest,
        options: sink::ImageSinkOptions,
    ) -> Result<EncodedExportedImage, ImageExportError> {
        let exported = self.export_image(request)?;
        sink::sink_exported_image(exported, options).map_err(ImageExportError::Sink)
    }

    /// Retrieve metadata for all images embedded in the document.
    pub fn images(&mut self) -> Result<Vec<ImageInfo>, RenderError> {
        self.image_metadata().map(|meta| meta.images)
    }

    /// Retrieve metadata for images within the provided 1-based (inclusive) page range.
    pub fn images_in_range(
        &mut self,
        start_page: u32,
        end_page: u32,
    ) -> Result<Vec<ImageInfo>, RenderError> {
        self.image_metadata_in_range(start_page, end_page)
            .map(|meta| meta.images)
    }

    /// Retrieve image + page metadata for the entire document.
    pub fn image_metadata(&mut self) -> Result<ImageMetadata, RenderError> {
        self.collect_image_metadata(None)
    }

    /// Retrieve image + page metadata for the provided range.
    pub fn image_metadata_in_range(
        &mut self,
        start_page: u32,
        end_page: u32,
    ) -> Result<ImageMetadata, RenderError> {
        self.collect_image_metadata(Some((start_page, end_page)))
    }

    /// Retrieve per-page object counters for the entire document.
    pub fn page_info(&mut self) -> Result<Vec<PageInfo>, RenderError> {
        self.image_metadata().map(|meta| meta.pages)
    }

    fn collect_image_metadata(
        &mut self,
        range: Option<(u32, u32)>,
    ) -> Result<ImageMetadata, RenderError> {
        self.renderer
            .collect_images(range)
            .map(ImageMetadata::from)
            .map_err(RenderError::Poppler)
    }
}

/// Shared, clonable factory that reopens [`Document`] instances on demand while caching
/// inexpensive metadata (page count, optional image descriptors) for multi-threaded callers.
#[derive(Clone)]
pub struct DocumentFactory {
    path: Arc<PathBuf>,
    page_count: u32,
    images: Option<Arc<[ImageInfo]>>,
    pages: Option<Arc<[PageInfo]>>,
    passwords: Option<Arc<PdfPasswords>>,
}

impl DocumentFactory {
    /// Prepare a factory from the provided PDF path, optionally caching image metadata for
    /// later use (useful for extracting per-page heuristics prior to spawning threads).
    pub fn prepare(pdf_path: &Path, cache_images: bool) -> Result<Self, RenderError> {
        Self::prepare_with_passwords(pdf_path, cache_images, None)
    }

    /// Prepare a factory using the provided passwords when opening the underlying PDF.
    pub fn prepare_with_passwords(
        pdf_path: &Path,
        cache_images: bool,
        passwords: Option<PdfPasswords>,
    ) -> Result<Self, RenderError> {
        let passwords_arc = passwords.map(Arc::new);
        let mut document = Document::open_with_passwords(pdf_path, passwords_arc.as_deref())?;
        let page_count = document.page_count()?;
        let (images, pages) = if cache_images {
            let metadata = document.image_metadata()?;
            (
                Some(Arc::from(metadata.images.into_boxed_slice())),
                Some(Arc::from(metadata.pages.into_boxed_slice())),
            )
        } else {
            (None, None)
        };
        Ok(Self {
            path: Arc::new(pdf_path.to_path_buf()),
            page_count,
            images,
            pages,
            passwords: passwords_arc,
        })
    }

    /// Convenience constructor that preloads image metadata.
    pub fn with_images(pdf_path: &Path) -> Result<Self, RenderError> {
        Self::prepare_with_passwords(pdf_path, true, None)
    }

    /// Convenience constructor that preloads image metadata using optional passwords.
    pub fn with_images_with_passwords(
        pdf_path: &Path,
        passwords: Option<PdfPasswords>,
    ) -> Result<Self, RenderError> {
        Self::prepare_with_passwords(pdf_path, true, passwords)
    }

    /// Convenience constructor that skips image metadata caching.
    pub fn from_path(pdf_path: &Path) -> Result<Self, RenderError> {
        Self::prepare_with_passwords(pdf_path, false, None)
    }

    /// Convenience constructor that skips image metadata caching while supplying passwords.
    pub fn from_path_with_passwords(
        pdf_path: &Path,
        passwords: Option<PdfPasswords>,
    ) -> Result<Self, RenderError> {
        Self::prepare_with_passwords(pdf_path, false, passwords)
    }

    /// Reopen the backing PDF as a fresh [`Document`].
    pub fn open(&self) -> Result<Document, RenderError> {
        Document::open_with_passwords(self.pdf_path(), self.passwords.as_deref())
    }

    /// Cached page count collected during [`DocumentFactory::prepare`].
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// Cached image metadata, if it was requested when preparing the factory.
    pub fn images(&self) -> Option<&[ImageInfo]> {
        self.images.as_deref()
    }

    /// Cached page metadata, if it was requested when preparing the factory.
    pub fn pages(&self) -> Option<&[PageInfo]> {
        self.pages.as_deref()
    }

    /// Absolute path to the PDF file backing this factory.
    pub fn pdf_path(&self) -> &Path {
        self.path.as_path()
    }
}

/// Render a single PDF page to a PNG file using an already opened [`Document`].
pub fn render_page_to_png(
    document: &mut Document,
    page_index: u32,
    output_path: &Path,
    options: &RenderOptions,
) -> Result<(), RenderError> {
    document.render_page_to_png(page_index, output_path, options)
}

/// Render a page to encoded image bytes (PNG/JPEG) using an already opened [`Document`].
pub fn render_page_to_image(
    document: &mut Document,
    page_index: u32,
    options: &RenderOptions,
) -> Result<EncodedImage, RenderError> {
    document.render_page_image_bytes(page_index, options)
}

/// Convenience helper that opens the PDF, renders a single page, and closes the renderer.
/// Prefer [`Document::render_page_to_png`] when rendering multiple pages to avoid the cost
/// of reloading the document.
pub fn render_single_page_to_png(
    pdf_path: &Path,
    page_index: u32,
    output_path: &Path,
    options: &RenderOptions,
) -> Result<(), RenderError> {
    let mut document = Document::open(pdf_path)?;
    document.render_page_to_png(page_index, output_path, options)
}

/// Convenience helper that returns metadata for each image embedded in the PDF.
/// Prefer [`Document::images`] when you need to query multiple things from the same
/// document to avoid reopening it repeatedly.
pub fn get_images(document: &mut Document) -> Result<Vec<ImageInfo>, RenderError> {
    document.images()
}

/// Convenience helper that returns metadata for images within the given page range.
pub fn get_images_in_range(
    document: &mut Document,
    start_page: u32,
    end_page: u32,
) -> Result<Vec<ImageInfo>, RenderError> {
    document.images_in_range(start_page, end_page)
}

/// Convenience helper that opens the PDF, collects image metadata, and then drops
/// the renderer. This is useful for single-shot queries.
pub fn get_images_single(pdf_path: &Path) -> Result<Vec<ImageInfo>, RenderError> {
    let mut document = Document::open(pdf_path)?;
    document.images()
}

fn encode_image(image: &ffi::Image, quality: u8, dpi: f64) -> Result<EncodedImage, RenderError> {
    if image.width == 0 || image.height == 0 {
        return Err(RenderError::UnsupportedLayout);
    }

    let width = image.width as usize;
    let height = image.height as usize;
    let components = image.components as usize;
    let bits_per_component = image.bits_per_component as usize;

    if components == 0 || bits_per_component == 0 {
        return Err(RenderError::UnsupportedLayout);
    }
    if bits_per_component != 1 && bits_per_component != 8 {
        return Err(RenderError::UnsupportedLayout);
    }

    let row_bits = width
        .checked_mul(components)
        .and_then(|value| value.checked_mul(bits_per_component))
        .ok_or(RenderError::UnsupportedLayout)?;
    let row_bytes = row_bits.div_ceil(8).max(1);
    let stride = image.stride as usize;
    if stride < row_bytes {
        return Err(RenderError::UnsupportedLayout);
    }
    let required = stride
        .checked_mul(height)
        .ok_or(RenderError::UnsupportedLayout)?;
    if required > image.data.len() {
        return Err(RenderError::UnsupportedLayout);
    }

    match image.color_mode {
        ColorMode::Mono1 => {
            if bits_per_component != 1 {
                return Err(RenderError::UnsupportedLayout);
            }
            let pixels = collect_rows(image, row_bytes)?;
            // Poppler represents mono1 scans with 1 bit per pixel; encode using a
            // two-entry palette (black, white) to preserve the bit-packed data.
            const PALETTE: [u8; 6] = [0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF];
            let bytes = encode_png(
                &pixels,
                width,
                height,
                dpi,
                ColorType::Indexed,
                BitDepth::One,
                Some(&PALETTE),
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Png,
                bytes,
            })
        }
        ColorMode::Mono8 => {
            let pixels = collect_rows(image, row_bytes)?;
            let bytes = encode_png(
                &pixels,
                width,
                height,
                dpi,
                ColorType::Grayscale,
                BitDepth::Eight,
                None,
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Png,
                bytes,
            })
        }
        ColorMode::Rgb8 => {
            let pixels = collect_rows(image, row_bytes)?;
            let bytes = encode_png(
                &pixels,
                width,
                height,
                dpi,
                ColorType::Rgb,
                BitDepth::Eight,
                None,
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Png,
                bytes,
            })
        }
        ColorMode::Bgr8 => {
            let mut pixels = collect_rows(image, row_bytes)?;
            for chunk in pixels.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            let bytes = encode_png(
                &pixels,
                width,
                height,
                dpi,
                ColorType::Rgb,
                BitDepth::Eight,
                None,
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Png,
                bytes,
            })
        }
        ColorMode::Xbgr8 => {
            let raw = collect_rows(image, row_bytes)?;
            let mut rgba = Vec::with_capacity(width * height * 4);
            for chunk in raw.chunks_exact(4) {
                rgba.push(chunk[3]);
                rgba.push(chunk[2]);
                rgba.push(chunk[1]);
                rgba.push(255);
            }
            let bytes = encode_png(
                &rgba,
                width,
                height,
                dpi,
                ColorType::Rgba,
                BitDepth::Eight,
                None,
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Png,
                bytes,
            })
        }
        ColorMode::Cmyk8 => encode_cmyk_like(image, width, height, row_bytes, false, quality),
        ColorMode::DeviceN8 => encode_cmyk_like(image, width, height, row_bytes, true, quality),
    }
}

fn collect_rows(image: &ffi::Image, row_bytes: usize) -> Result<Vec<u8>, RenderError> {
    let stride = image.stride as usize;
    let height = image.height as usize;
    let mut buffer = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        let end = start + row_bytes;
        if end > image.data.len() {
            return Err(RenderError::UnsupportedLayout);
        }
        buffer.extend_from_slice(&image.data[start..end]);
    }
    Ok(buffer)
}

fn encode_png(
    pixels: &[u8],
    width: usize,
    height: usize,
    dpi: f64,
    colorspace: ColorType,
    depth: BitDepth,
    palette: Option<&[u8]>,
) -> Result<Vec<u8>, RenderError> {
    let width_u32: u32 = width
        .try_into()
        .map_err(|_| RenderError::InvalidU32Size(width))?;
    let height_u32: u32 = height
        .try_into()
        .map_err(|_| RenderError::InvalidU32Size(height))?;

    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width_u32, height_u32);
        encoder.set_compression(PngCompression::Balanced);
        encoder.set_color(colorspace);
        encoder.set_depth(depth);
        if let Some(palette_bytes) = palette {
            encoder.set_palette(palette_bytes.to_vec());
        }
        if let Some(pixel_dims) = helpers::build_pixel_dims_png(dpi, dpi) {
            encoder.set_pixel_dims(Some(pixel_dims));
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }

    Ok(buffer)
}

fn encode_cmyk_like(
    image: &ffi::Image,
    width: usize,
    height: usize,
    row_bytes: usize,
    drop_spot_channels: bool,
    quality: u8,
) -> Result<EncodedImage, RenderError> {
    if image.bits_per_component != 8 {
        return Err(RenderError::UnsupportedLayout);
    }

    let components = image.components as usize;
    if components < 4 {
        return Err(RenderError::UnsupportedLayout);
    }

    let pixels = collect_rows(image, row_bytes)?;
    // DeviceN buffers append spot tint components after CMYK; we drop them to fit JPEG CMYK.
    let stripped = if drop_spot_channels && components > 4 {
        Some(strip_spot_channels(&pixels, components))
    } else {
        None
    };
    let payload: &[u8] = if let Some(ref owned) = stripped {
        owned.as_slice()
    } else if components == 4 {
        &pixels
    } else {
        return Err(RenderError::UnsupportedLayout);
    };

    let jpeg = encode_jpeg(payload, width, height, JpegColorType::Cmyk, quality)?;
    Ok(EncodedImage {
        format: ImageFormat::Jpeg,
        bytes: jpeg,
    })
}

fn strip_spot_channels(pixels: &[u8], components: usize) -> Vec<u8> {
    debug_assert!(components >= 4);
    let pixel_count = pixels.len() / components;
    let mut base = Vec::with_capacity(pixel_count * 4);
    for chunk in pixels.chunks_exact(components) {
        base.extend_from_slice(&chunk[..4]);
    }
    base
}

fn encode_jpeg(
    pixels: &[u8],
    width: usize,
    height: usize,
    colorspace: JpegColorType,
    quality: u8,
) -> Result<Vec<u8>, RenderError> {
    let width_u16: u16 = width
        .try_into()
        .map_err(|_| RenderError::InvalidJpegDimension {
            side: "width",
            value: width,
        })?;
    let height_u16: u16 = height
        .try_into()
        .map_err(|_| RenderError::InvalidJpegDimension {
            side: "height",
            value: height,
        })?;

    let encoder = JpegEncoder::new().quality(quality);
    let buffer = encoder.encode(pixels, width_u16, height_u16, colorspace)?;
    Ok(buffer)
}
