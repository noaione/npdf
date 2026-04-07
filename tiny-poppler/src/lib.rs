//! Tiny interface for rendering PDF pages to PNG/JPEG via Poppler's Splash backend.
//!
//! This crate offers a small API so the CLI can convert PDF pages to PNG/JPEG files
//! without linking against Poppler's GLib/Cairo layers. Under the hood we build
//! Poppler (with Splash) as part of the Cargo build.

mod ffi;
mod helpers;
mod sink;

use ffi::get_poppler_version;
pub use ffi::{
    CcittParams, ColorMode, ExportedImage, ImageColorSpace, ImageExportExtension,
    ImageExportFormat, ImageExportRequest, ImageExportSelector, ImageExportType, ImageInfo,
    ImageType, PageInfo, PdfCropMode, PdfImageColorSpace, PdfMatrix, PdfPoint, PdfRect,
    ZeroWidthLineMode,
};
use png::Encoder;
pub use png::{BitDepth as PngBitDepth, ColorType as PngColorType};
use rayon::iter::{IndexedParallelIterator, IntoParallelRefMutIterator, ParallelIterator};
use rayon::slice::{ParallelSlice, ParallelSliceMut};
pub use simple_jpegli_enc::ColorSpace as JpegColorType;
use simple_jpegli_enc::{JpegEncoder, JpegError};
pub use sink::{
    EncodedExportedImage, ImageSinkError, ImageSinkOptions, PngCompression, TiffCompression,
    TiffDeflateLevel, sink_exported_image,
};

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

/// Output mode for rendering PDF pages.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq)]
pub enum OutputMode {
    /// PNG/JPG encoded image bytes.
    #[default]
    Encoded,
    /// Raw bitmap data, this basically an interleaved pixel buffer.
    RawBitmap,
}

/// Configuration for a render operation.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub dpi: f64,
    pub color_mode: ColorMode,
    pub crop_mode: PdfCropMode,
    pub jpeg_quality: Option<u8>,
    pub output_mode: OutputMode,
    pub zero_width_line_mode: ZeroWidthLineMode,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            color_mode: ColorMode::Rgb8,
            crop_mode: PdfCropMode::CropBox,
            jpeg_quality: Some(95),
            output_mode: OutputMode::Encoded,
            zero_width_line_mode: ZeroWidthLineMode::Default,
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
    /// PNG format.
    Png,
    /// JPEG format.
    Jpeg,
    /// Raw bitmap data.
    Raw,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Raw => "bin",
        }
    }
}

/// Encoded image data plus format metadata.
#[derive(Debug, Clone)]
pub struct EncodedImage {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub dpi: f64,
    pub components: usize,
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
    #[error("invalid DPI value: {0}")]
    InvalidDpiValue(f64),
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
                options.zero_width_line_mode,
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
            options.output_mode,
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

fn encode_image(
    image: &ffi::Image,
    quality: u8,
    dpi: f64,
    output_mode: OutputMode,
) -> Result<EncodedImage, RenderError> {
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
            match output_mode {
                OutputMode::RawBitmap => Ok(EncodedImage {
                    format: ImageFormat::Raw,
                    bytes: pixels,
                    width,
                    height,
                    dpi,
                    components: 1,
                }),
                OutputMode::Encoded => {
                    // Poppler represents mono1 scans with 1 bit per pixel; encode using a
                    // two-entry palette (black, white) to preserve the bit-packed data.
                    const PALETTE: [u8; 6] = [0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF];
                    let bytes = encode_png(
                        &pixels,
                        width,
                        height,
                        dpi,
                        PngColorType::Indexed,
                        PngBitDepth::One,
                        Some(&PALETTE),
                    )?;
                    Ok(EncodedImage {
                        format: ImageFormat::Png,
                        bytes,
                        width,
                        height,
                        dpi,
                        components: 1,
                    })
                }
            }
        }
        ColorMode::Mono8 | ColorMode::Rgb8 => {
            let pixels = collect_rows(image, row_bytes)?;
            match output_mode {
                OutputMode::RawBitmap => Ok(EncodedImage {
                    format: ImageFormat::Raw,
                    bytes: pixels,
                    width,
                    height,
                    dpi,
                    components,
                }),
                OutputMode::Encoded => {
                    let is_rgb = matches!(image.color_mode, ColorMode::Rgb8);
                    let bytes = encode_png(
                        &pixels,
                        width,
                        height,
                        dpi,
                        if is_rgb {
                            PngColorType::Rgb
                        } else {
                            PngColorType::Grayscale
                        },
                        PngBitDepth::Eight,
                        None,
                    )?;
                    Ok(EncodedImage {
                        format: ImageFormat::Png,
                        bytes,
                        width,
                        height,
                        dpi,
                        components,
                    })
                }
            }
        }
        ColorMode::Bgr8 => {
            let mut pixels = collect_rows(image, row_bytes)?;
            for chunk in pixels.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            match output_mode {
                OutputMode::RawBitmap => Ok(EncodedImage {
                    format: ImageFormat::Raw,
                    bytes: pixels,
                    width,
                    height,
                    dpi,
                    components,
                }),
                OutputMode::Encoded => {
                    let bytes = encode_png(
                        &pixels,
                        width,
                        height,
                        dpi,
                        PngColorType::Rgb,
                        PngBitDepth::Eight,
                        None,
                    )?;
                    Ok(EncodedImage {
                        format: ImageFormat::Png,
                        bytes,
                        width,
                        height,
                        dpi,
                        components,
                    })
                }
            }
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
            match output_mode {
                OutputMode::RawBitmap => Ok(EncodedImage {
                    format: ImageFormat::Raw,
                    bytes: rgba,
                    width,
                    height,
                    dpi,
                    components,
                }),
                OutputMode::Encoded => {
                    let bytes = encode_png(
                        &rgba,
                        width,
                        height,
                        dpi,
                        PngColorType::Rgba,
                        PngBitDepth::Eight,
                        None,
                    )?;
                    Ok(EncodedImage {
                        format: ImageFormat::Png,
                        bytes,
                        width,
                        height,
                        dpi,
                        components,
                    })
                }
            }
        }
        ColorMode::Cmyk8 => encode_cmyk_like(
            image,
            (width, height),
            row_bytes,
            false,
            quality,
            dpi,
            output_mode,
        ),
        ColorMode::DeviceN8 => encode_cmyk_like(
            image,
            (width, height),
            row_bytes,
            true,
            quality,
            dpi,
            output_mode,
        ),
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

/// Encode raw pixel data into PNG format using the `png` crate.
///
/// # Errors
/// Returns `RenderError::InvalidU32Size` if the provided width or height
/// cannot be converted to `u32`.
pub fn encode_png(
    pixels: &[u8],
    width: usize,
    height: usize,
    dpi: f64,
    colorspace: PngColorType,
    depth: PngBitDepth,
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
        encoder.set_filter(png::Filter::Adaptive);
        encoder.set_deflate_compression(png::DeflateCompression::Level(9));
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
    sizes: (usize, usize),
    row_bytes: usize,
    drop_spot_channels: bool,
    quality: u8,
    dpi: f64,
    output_mode: OutputMode,
) -> Result<EncodedImage, RenderError> {
    if image.bits_per_component != 8 {
        return Err(RenderError::UnsupportedLayout);
    }

    let (width, height) = sizes;

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

    match output_mode {
        OutputMode::RawBitmap => Ok(EncodedImage {
            format: ImageFormat::Raw,
            bytes: payload.to_vec(),
            width,
            height,
            dpi,
            components,
        }),
        OutputMode::Encoded => {
            // invert CMYK to match JPEGli expectations
            let inverted_payload = {
                let mut buf = payload.to_vec();
                buf.par_iter_mut().for_each(|p| *p = 255 - *p);
                buf
            };

            // convert dpi f64 to u16
            let dpi_usize: usize = dpi.round().clamp(72.0, u16::MAX as f64) as usize;
            let dpi_u16: u16 = dpi_usize
                .try_into()
                .map_err(|_| RenderError::InvalidDpiValue(dpi))?;

            let jpeg = encode_jpeg(
                &inverted_payload,
                width,
                height,
                JpegColorType::Cmyk,
                quality,
                dpi_u16,
            )?;
            Ok(EncodedImage {
                format: ImageFormat::Jpeg,
                bytes: jpeg,
                width,
                height,
                dpi,
                components,
            })
        }
    }
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

/// Encode raw pixel data into JPEG format using the `simple_jpegli_enc` crate.
///
/// # Errors
/// Returns `RenderError::InvalidJpegDimension` if the provided width or height
/// cannot be converted to `u16`.
pub fn encode_jpeg(
    pixels: &[u8],
    width: usize,
    height: usize,
    colorspace: JpegColorType,
    quality: u8,
    dpi: u16,
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
    let buffer = encoder.encode(pixels, width_u16, height_u16, colorspace, Some((dpi, dpi)))?;
    Ok(buffer)
}

/// A fast CMYK -> RGB conversion function based on Python Pillow's implementation.
///
/// Some stuff are "guaranteed" by the caller (e.g., input length is multiple of 4, etc).
#[inline(always)]
fn cmyk2rgb_pix_fast(c: u8, m: u8, y: u8, k: u8) -> (u8, u8, u8) {
    // We use u32 to prevent overflow during multiplication, but we know
    // the result fits in u8, so we can skip saturating_sub and min.

    // nk = 255 - k
    let nk = 255 - k as u32;

    // The logic: result = nk - ((color * nk + 128) / 255)
    // The fast division (x + 128) / 255 is approx ((x + 128) * 257) >> 16
    // Or the shift trick you used: t = v + 128; (t + (t >> 8)) >> 8

    // R
    let r_target = c as u32 * nk + 128;
    let r_div = (r_target + (r_target >> 8)) >> 8;
    let r = (nk - r_div) as u8;

    // G
    let g_target = m as u32 * nk + 128;
    let g_div = (g_target + (g_target >> 8)) >> 8;
    let g = (nk - g_div) as u8;

    // B
    let b_target = y as u32 * nk + 128;
    let b_div = (b_target + (b_target >> 8)) >> 8;
    let b = (nk - b_div) as u8;

    (r, g, b)
}

/// A parallel CMYK -> RGB conversion function based on Python Pillow's implementation.
///
/// Some stuff are "guaranteed" by the caller (e.g., input length is multiple of 4, etc).
///
/// # Errors
/// Returns `RenderError::UnsupportedLayout` if the input pixel buffer length is not
/// a multiple of 4.
///
/// # Example
/// ```rust
/// let cmyk_pixels = vec![0u8, 255, 255, 0, 255, 0, 255, 0]; // Two CMYK pixels
/// let rgb_pixels = tiny_poppler::cmyk2rgb(&cmyk_pixels).unwrap();
/// assert_eq!(rgb_pixels, vec![255, 0, 0, 0, 255, 0]); // Corresponding RGB pixels
/// ```
pub fn cmyk2rgb(pixels: &[u8]) -> Result<Vec<u8>, RenderError> {
    // check dimension that each row has multiple of 4 bytes
    if !pixels.len().is_multiple_of(4) {
        return Err(RenderError::UnsupportedLayout);
    }

    let num_pixels = pixels.len() / 4;

    // pre-allocate output buffer
    let mut results = vec![0u8; num_pixels * 3];

    // parallel convert
    results
        .par_chunks_exact_mut(3) // Output RGB slice
        .zip(pixels.par_chunks_exact(4)) // Input CMYK slice
        .for_each(|(out_pixel, in_pixel)| {
            // no need for bounds checking, chunks_exact ensures correct sizes
            let (r, g, b) = cmyk2rgb_pix_fast(in_pixel[0], in_pixel[1], in_pixel[2], in_pixel[3]);

            out_pixel[0] = r;
            out_pixel[1] = g;
            out_pixel[2] = b;
        });

    Ok(results)
}

/// Do a fast RGB -> Gray conversion using ITU-R BT.709 luminance approximation.
///
/// # Errors
/// Returns `RenderError::UnsupportedLayout` if the input pixel buffer length is not
/// a multiple of 3.
///
/// # Example
/// ```rust
/// let rgb_pixels = vec![255u8, 0, 0, 0, 255, 0]; // Two RGB pixels (red, green)
/// let gray_pixels = tiny_poppler::rgb2gray(&rgb_pixels).unwrap();
/// assert_eq!(gray_pixels, vec![53, 182]); // Corresponding grayscale values
/// ```
pub fn rgb2gray(pixels: &[u8]) -> Result<Vec<u8>, RenderError> {
    // Check dimension that each row has multiple of 3 bytes
    if !pixels.len().is_multiple_of(3) {
        return Err(RenderError::UnsupportedLayout);
    }

    let result: Vec<u8> = pixels
        .par_chunks_exact(3)
        .map(|chunk| {
            let r = chunk[0] as u16;
            let g = chunk[1] as u16;
            let b = chunk[2] as u16;

            // Fast approx of ITU-R BT.709 luminance
            // 0.2126 * R + 0.7152 * G + 0.0722 * B
            // Using fast approximation with integer math:
            ((r * 54 + g * 183 + b * 19) >> 8) as u8
        })
        .collect();

    Ok(result)
}

/// Do a fast CMYK -> Gray conversion without using RGB as an intermediate.
///
/// Note: This is a really rough approximation and may not yield visually accurate results.
///
/// # Errors
/// Returns `RenderError::UnsupportedLayout` if the input pixel buffer length is not
/// a multiple of 4.
///
/// # Example
/// ```rust
/// let cmyk_pixels = vec![0u8, 255, 255, 0, 255, 0, 255, 0]; // Two CMYK pixels
/// let gray_pixels = tiny_poppler::cmyk2gray(&cmyk_pixels).unwrap();
/// assert_eq!(gray_pixels, vec![77, 151]); // Corresponding grayscale values
/// ```
pub fn cmyk2gray(pixels: &[u8]) -> Result<Vec<u8>, RenderError> {
    // Check dimension that each row has multiple of 4 bytes
    if !pixels.len().is_multiple_of(4) {
        return Err(RenderError::UnsupportedLayout);
    }

    let result: Vec<u8> = pixels
        .par_chunks_exact(4)
        .map(|chunk| {
            let c = chunk[0] as u16;
            let m = chunk[1] as u16;
            let y = chunk[2] as u16;
            let k = chunk[3];

            // Fast approx of CMYK to Gray
            // >> gray = K + int(0.3 * C) + int(0.59 * M) + int(0.11 * Y)
            // >> gray = max(0, min(255, gray))
            let cmy_part = (c * 77 + m * 151 + y * 28) >> 8;
            let gray = k.saturating_add(cmy_part as u8);
            // invert to get final gray value
            255 - gray
        })
        .collect();

    Ok(result)
}

#[derive(Debug, Clone)]
pub struct VersionInfo<'a> {
    version: (u32, u32, u32),
    sha: Option<&'a str>,
}

impl<'a> VersionInfo<'a> {
    pub fn version_string(&self) -> String {
        format!("{}.{}.{}", self.version.0, self.version.1, self.version.2)
    }

    pub fn git_sha(&self) -> Option<&'a str> {
        self.sha
    }
}

/// Get the Poppler library version as (major, minor, patch).
///
/// # Example
/// ```rust
/// let version = tiny_poppler::get_version();
///
/// assert_eq!(version.version_string(), "26.4.90"); // Example version
/// ```
pub fn get_version() -> VersionInfo<'static> {
    let version = get_poppler_version();
    let sha = option_env!("POPPLER_COMMIT_SHA");
    VersionInfo { version, sha }
}
