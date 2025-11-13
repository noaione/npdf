//! Tiny interface for rendering PDF pages to PNG via Poppler's Splash backend.
//!
//! This crate offers a small API so the CLI can convert PDF pages to PNG files
//! without linking against Poppler's GLib/Cairo layers. Under the hood we build
//! Poppler (with Splash) as part of the Cargo build.

mod ffi;

pub use ffi::{ColorMode, ImageColorSpace, ImageInfo};
use png::{BitDepth, ColorType, Compression, Encoder};

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use thiserror::Error;

/// Configuration for a render operation.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub dpi: f64,
    pub color_mode: ColorMode,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            color_mode: ColorMode::Rgb8,
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
}

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
            .render_page(page_index, options.dpi, options.color_mode)
            .map_err(RenderError::Poppler)
    }

    /// Render a page to PNG bytes using the configured color mode.
    pub fn render_page_png(
        &mut self,
        page_index: u32,
        options: &RenderOptions,
    ) -> Result<Vec<u8>, RenderError> {
        let image = self.render_page_image(page_index, options)?;
        image_to_png(&image)
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

    /// Retrieve metadata for all images embedded in the document.
    pub fn images(&mut self) -> Result<Vec<ImageInfo>, RenderError> {
        self.renderer
            .collect_images(None)
            .map_err(RenderError::Poppler)
    }

    /// Retrieve metadata for images within the provided 1-based (inclusive) page range.
    pub fn images_in_range(
        &mut self,
        start_page: u32,
        end_page: u32,
    ) -> Result<Vec<ImageInfo>, RenderError> {
        self.renderer
            .collect_images(Some((start_page, end_page)))
            .map_err(RenderError::Poppler)
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

fn image_to_png(image: &ffi::Image) -> Result<Vec<u8>, RenderError> {
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
    let row_bytes = ((row_bits + 7) / 8).max(1);
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
            encode_png(
                &pixels,
                width,
                height,
                ColorType::Indexed,
                BitDepth::One,
                Some(&PALETTE),
            )
        }
        ColorMode::Mono8 => {
            let pixels = collect_rows(image, row_bytes)?;
            encode_png(
                &pixels,
                width,
                height,
                ColorType::Grayscale,
                BitDepth::Eight,
                None,
            )
        }
        ColorMode::Rgb8 => {
            let pixels = collect_rows(image, row_bytes)?;
            encode_png(
                &pixels,
                width,
                height,
                ColorType::Rgb,
                BitDepth::Eight,
                None,
            )
        }
        ColorMode::Bgr8 => {
            let mut pixels = collect_rows(image, row_bytes)?;
            for chunk in pixels.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            encode_png(
                &pixels,
                width,
                height,
                ColorType::Rgb,
                BitDepth::Eight,
                None,
            )
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
            encode_png(&rgba, width, height, ColorType::Rgba, BitDepth::Eight, None)
        }
        ColorMode::Cmyk8 | ColorMode::DeviceN8 => {
            Err(RenderError::UnsupportedColorMode(image.color_mode))
        }
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
        encoder.set_compression(Compression::Balanced);
        encoder.set_color(colorspace);
        encoder.set_depth(depth);
        if let Some(palette_bytes) = palette {
            encoder.set_palette(palette_bytes.to_vec());
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }

    Ok(buffer)
}
