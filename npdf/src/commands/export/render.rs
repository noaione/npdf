use clap::ValueEnum;
use color_eyre::{Result, eyre::Context};
use color_print::cprintln;
use fast_image_resize::images::Image as ResizeImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use rayon::iter::{IntoParallelRefMutIterator, ParallelIterator};
use std::fs;
use std::path::PathBuf;
use tiny_poppler::{
    ColorMode, Document, DocumentFactory, GlyphFillMode, ImageInfo, ImageType, PageInfo,
    PdfCropMode, PdfImageColorSpace, PdfMatrix, PdfRect, RenderOptions, ZeroWidthLineMode,
    cmyk2gray, cmyk2rgb,
};

use crate::{commands::ExportArgs, common::NpdfError};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Mono1,
    Mono8,
    Rgb8,
    Bgr8,
    Xbgr8,
    Cmyk8,
    Devicen8,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum AutoDPIDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CropChoice {
    Crop,
    Media,
    // ArtBox,
    // BleedBox,
    // TrimBox,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum ZeroWidthLineChoice {
    Default,
    Hairline,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum GlyphFillChoice {
    /// Rasterize glyphs via FreeType's own renderer (default, best antialiasing).
    Bitmap,
    /// Fill glyph outlines through Splash's own path rasterizer. Works around fonts whose
    /// self-intersecting contours FreeType mis-renders, at the cost of coarser antialiasing
    /// on every glyph -- only use this if you've hit that specific artifact.
    Path,
}

#[derive(Clone)]
pub(super) struct RenderPagePlan {
    pub page_number: u32,
    pub zero_index_page: u32,
    pub total_pages: u32,
    pub output_path: Option<PathBuf>,
    /// `--output-dpi` when it is below the chosen render DPI: the page is rendered at the
    /// higher resolution and then averaged down, which antialiases text and line art
    /// better than rasterizing straight at the lower one.
    pub output_dpi: Option<f64>,
    request_color: ColorChoice,
    detected_color: ColorMode,
    pub options: RenderOptions,
}

pub(super) fn process_job(document: &mut Document, job: &RenderPagePlan) -> Result<()> {
    let output_path = job.output_path.as_ref().ok_or(NpdfError::OutputNotSet)?;

    let rendered = document
        .render_page_image_bytes(job.zero_index_page, &job.options)
        .context(format!("page {}", job.page_number))?;
    let encoded = match job.output_dpi {
        Some(target) => {
            downscale(rendered, target).context(format!("page {}, downscale", job.page_number))?
        }
        None => rendered,
    };

    let (bytes_data, extension) = match encoded.format {
        tiny_poppler::ImageFormat::Png => (encoded.bytes, "png"),
        tiny_poppler::ImageFormat::Jpeg => (encoded.bytes, "png"),
        tiny_poppler::ImageFormat::Raw => {
            // A raw bitmap arrives either from `--cmyk-png` (always rendered as CMYK, to be
            // converted here) or from a `--output-dpi` supersample (rendered directly in
            // the colour that gets written). Only the first needs a colour conversion.
            let want_color = if job.request_color == ColorChoice::Auto {
                job.detected_color
            } else {
                job.request_color.to_color_mode().unwrap_or(ColorMode::Rgb8)
            };
            let rendered_cmyk = matches!(
                job.options.color_mode,
                ColorMode::Cmyk8 | ColorMode::DeviceN8
            );

            match want_color {
                // Still export as JPEG
                ColorMode::Cmyk8 | ColorMode::DeviceN8 => {
                    let inverted_payload = {
                        let mut buf = encoded.bytes;
                        buf.par_iter_mut().for_each(|p| *p = 255 - *p);
                        buf
                    };

                    // convert dpi f64 to u16
                    let dpi_usize: usize =
                        encoded.dpi.round().clamp(72.0, u16::MAX as f64) as usize;
                    let dpi_u16: u16 = dpi_usize
                        .try_into()
                        .context(format!("page {}, clamp u16", job.page_number))?;

                    let jpeg = tiny_poppler::encode_jpeg(
                        &inverted_payload,
                        encoded.width,
                        encoded.height,
                        tiny_poppler::JpegColorType::Cmyk,
                        job.options.jpeg_quality.unwrap_or(95),
                        dpi_u16,
                    )
                    .context(format!("page {}, encode JPEG", job.page_number))?;
                    (jpeg, "jpg")
                }
                ColorMode::Mono1 | ColorMode::Mono8 => {
                    // Export as PNG, converting from CMYK when that is what was rendered.
                    let (colorspace, depth, palette) = if want_color == ColorMode::Mono1 {
                        (
                            tiny_poppler::PngColorType::Indexed,
                            tiny_poppler::PngBitDepth::One,
                            Some(&[0x00u8, 0x00, 0x00, 0xFF, 0xFF, 0xFF] as &[u8]),
                        )
                    } else {
                        (
                            tiny_poppler::PngColorType::Grayscale,
                            tiny_poppler::PngBitDepth::Eight,
                            None,
                        )
                    };

                    let as_gray = if rendered_cmyk {
                        cmyk2gray(&encoded.bytes)
                            .context(format!("page {}, cmyk2gray", job.page_number))?
                    } else {
                        encoded.bytes
                    };

                    let png = tiny_poppler::encode_png(
                        &as_gray,
                        encoded.width,
                        encoded.height,
                        encoded.dpi,
                        colorspace,
                        depth,
                        palette,
                    )
                    .context(format!("page {}, encode PNG", job.page_number))?;

                    (png, "png")
                }
                other => {
                    // Export as PNG, converting from CMYK when that is what was rendered.
                    let rgb_data = if rendered_cmyk {
                        cmyk2rgb(&encoded.bytes)
                            .context(format!("page {}, cmyk2rgb", job.page_number))?
                    } else {
                        encoded.bytes
                    };

                    let (adjusted_rgb, colorspace) = if other == ColorMode::Xbgr8 {
                        let mut rgba = Vec::with_capacity(encoded.width * encoded.height * 4);
                        for chunk in rgb_data.chunks_exact(3) {
                            // we only want to add an opaque alpha channel
                            rgba.push(chunk[0]);
                            rgba.push(chunk[1]);
                            rgba.push(chunk[2]);
                            rgba.push(0xFF);
                        }
                        (rgba, tiny_poppler::PngColorType::Rgba)
                    } else {
                        (rgb_data, tiny_poppler::PngColorType::Rgb)
                    };

                    let png = tiny_poppler::encode_png(
                        &adjusted_rgb,
                        encoded.width,
                        encoded.height,
                        encoded.dpi,
                        colorspace,
                        tiny_poppler::PngBitDepth::Eight,
                        None,
                    )
                    .context(format!("page {}, encode PNG", job.page_number))?;

                    (png, "png")
                }
            }
        }
    };

    let output_path_with_ext = output_path.with_extension(extension);
    fs::write(output_path_with_ext, &bytes_data).context(format!("page {}", job.page_number))?;

    // pad page number depending on total pages
    let total_page = job.total_pages.to_string().len();
    let pad_page = format!("{:0width$}", job.page_number, width = total_page);
    cprintln!(
        "Rendered <m,s>page {}</m,s> -> <m,s>{}.{}</m,s> ({:?}, {:?}, {} dpi)",
        pad_page,
        output_path.display(),
        extension,
        encoded.format,
        job.options.color_mode,
        encoded.dpi,
    );
    Ok(())
}

/// Averages a supersampled raw bitmap down to `target_dpi`.
///
/// Only raw bitmaps can be resized, so `prepare_job` renders in `OutputMode::RawBitmap`
/// whenever this is going to run. Layouts the resizer cannot express, and the no-op case
/// where the size already matches, are returned untouched.
fn downscale(
    encoded: tiny_poppler::EncodedImage,
    target_dpi: f64,
) -> Result<tiny_poppler::EncodedImage> {
    if !matches!(encoded.format, tiny_poppler::ImageFormat::Raw) {
        return Ok(encoded);
    }

    let (width, height) = (encoded.width, encoded.height);
    if width == 0 || height == 0 || encoded.dpi <= 0.0 {
        return Ok(encoded);
    }

    // Trust the buffer over the reported component count: DeviceN spot channels are
    // stripped before the bitmap is handed over, but the count still reports the original.
    let components = encoded.bytes.len() / (width * height);
    let pixel_type = match components {
        1 => PixelType::U8,
        3 => PixelType::U8x3,
        4 => PixelType::U8x4,
        _ => return Ok(encoded),
    };

    let ratio = target_dpi / encoded.dpi;
    let new_width = ((width as f64) * ratio).round().max(1.0) as u32;
    let new_height = ((height as f64) * ratio).round().max(1.0) as u32;
    if (new_width as usize, new_height as usize) == (width, height) {
        return Ok(encoded);
    }

    let expected = width * height * components;
    if encoded.bytes.len() < expected {
        return Ok(encoded);
    }
    let mut bytes = encoded.bytes;
    bytes.truncate(expected);

    let source = ResizeImage::from_vec_u8(width as u32, height as u32, bytes, pixel_type)?;

    // Greyscale keeps more of its rendering with a softer kernel; colour tolerates the
    // sharper one without ringing.
    let filter = if components == 1 {
        FilterType::Mitchell
    } else {
        FilterType::Lanczos3
    };

    let mut target = ResizeImage::new(new_width, new_height, pixel_type);
    Resizer::new().resize(
        &source,
        &mut target,
        &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter)),
    )?;

    Ok(tiny_poppler::EncodedImage {
        format: tiny_poppler::ImageFormat::Raw,
        bytes: target.into_vec(),
        width: new_width as usize,
        height: new_height as usize,
        dpi: target_dpi,
        components,
    })
}

/// Whether a rendered page may be averaged down to a lower DPI.
fn can_downscale(options: &RenderOptions) -> bool {
    // Bit-packed 1-bit output cannot be resized as bytes, and thresholding an averaged
    // grey back to one bit throws away exactly the detail the supersample bought. The
    // BGR/XBGR modes are left out as well, since the raw encoder assumes RGB channel
    // order.
    matches!(
        options.color_mode,
        ColorMode::Mono8 | ColorMode::Rgb8 | ColorMode::Cmyk8 | ColorMode::DeviceN8
    )
}

pub(super) fn prepare_job(
    output_page: u32,
    page_info: &PageInfo,
    images: &[ImageInfo],
    args: &ExportArgs,
    output_path: Option<&PathBuf>,
    factory: &DocumentFactory,
    // callback to add the job to a queue
    queue_job: &mut dyn FnMut(RenderPagePlan),
) {
    let ExportArgs {
        color,
        dpi,
        bounding,
        auto_dpi,
        with_cmyk,
        quality,
        auto_dpi_ratio,
        cmyk_png,
        ..
    } = args;

    let base_options = RenderOptions {
        dpi: *dpi,
        crop_mode: bounding.to_crop_mode(),
        color_mode: if *cmyk_png {
            ColorMode::Cmyk8
        } else {
            color.to_color_mode().unwrap_or(ColorMode::Rgb8)
        },
        jpeg_quality: Some(*quality),
        output_mode: if *cmyk_png {
            tiny_poppler::OutputMode::RawBitmap
        } else {
            tiny_poppler::OutputMode::Encoded
        },
        zero_width_line_mode: args.zero_width_line.to_zero_width_line_mode(),
        glyph_fill_mode: args.glyph_fill.to_glyph_fill_mode(),
        min_line_width: args.min_line_width,
    };

    let guessed = precalculate_auto_export_config(
        images,
        Some(page_info),
        *bounding,
        *with_cmyk,
        DpiConfig {
            target_dpi: *dpi,
            direction: *auto_dpi,
            ratio: *auto_dpi_ratio,
            min_coverage: args.min_image_coverage / 100.0,
            min_image_dim: args.min_image_dim,
            color_export_dpi: args.color_export_dpi,
        },
    );

    let mut options = base_options.clone();
    if *color == ColorChoice::Auto && !*cmyk_png {
        options.color_mode = guessed.color;
    }
    if auto_dpi.is_some() || args.color_export_dpi.is_some() {
        options.dpi = guessed.dpi;
    }

    // Supersample only when the final resolution is actually lower, and only for pixels
    // that can be averaged. Otherwise `--output-dpi` just becomes the render resolution.
    let mut output_dpi = args.output_dpi.filter(|target| *target < options.dpi);
    if let Some(target) = output_dpi
        && !can_downscale(&options)
    {
        options.dpi = target;
        output_dpi = None;
    }
    if output_dpi.is_some() {
        options.output_mode = tiny_poppler::OutputMode::RawBitmap;
    }

    let file_name = format!("page-{output_page:04}");
    let with_output_path = output_path.as_ref().map(|o| o.join(&file_name));

    queue_job(RenderPagePlan {
        page_number: page_info.page,
        zero_index_page: page_info.page - 1,
        total_pages: factory.page_count(),
        output_path: with_output_path,
        output_dpi,
        request_color: *color,
        detected_color: guessed.color,
        options,
    })
}

struct GuessedImage {
    color: ColorMode,
    dpi: f64,
}

/// Everything the DPI rules need from the command line, gathered once per page.
#[derive(Clone, Copy)]
struct DpiConfig {
    /// `--dpi`: the ceiling every rule may only lower.
    target_dpi: f64,
    direction: Option<AutoDPIDirection>,
    /// `--auto-dpi-ratio`: the height/width ratio from which an image counts as tall.
    ratio: f64,
    /// `--min-image-coverage`, as a fraction (0.0-1.0).
    min_coverage: f64,
    min_image_dim: Option<u32>,
    /// `--color-export-dpi`: a lower ceiling for colour pages that carry nothing, or
    /// several things.
    color_export_dpi: Option<f64>,
}

fn precalculate_auto_export_config(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    with_cmyk: bool,
    cfg: DpiConfig,
) -> GuessedImage {
    let color = determine_page_colorspace(images, page_info, crop_mode, with_cmyk);
    let notable = notable_images(images, page_info, crop_mode, cfg);

    let mut dpi = match cfg.direction {
        Some(direction) => determine_export_dpi(&notable, direction, cfg),
        None => cfg.target_dpi,
    };

    // A colour page with no notable image, or with several, has no single piece of
    // imagery to resolve for, so it gets its own lower ceiling. Pages whose DPI already
    // came in under it keep their value.
    if let Some(color_dpi) = cfg.color_export_dpi
        && is_color_mode(color)
        && (notable.is_empty() || notable.len() > 1)
        && dpi > color_dpi
    {
        dpi = color_dpi;
    }

    GuessedImage { color, dpi }
}

fn is_color_mode(mode: ColorMode) -> bool {
    !matches!(mode, ColorMode::Mono1 | ColorMode::Mono8)
}

fn determine_page_colorspace(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    with_cmyk: bool,
) -> ColorMode {
    let relevant_images: Vec<&ImageInfo> = images
        .iter()
        .filter(|img| image_intersecting_with_page(page_info, crop_mode, img.matrix))
        .collect();

    if relevant_images.is_empty() {
        match page_info {
            Some(info) => determine_from_page_info(info, with_cmyk),
            _ => ColorMode::Mono1,
        }
    } else if relevant_images.iter().any(|img| image_has_color(img)) {
        if with_cmyk && relevant_images.iter().any(|img| image_has_cmyk(img)) {
            ColorMode::Cmyk8
        } else {
            ColorMode::Rgb8
        }
    } else {
        ColorMode::Mono8
    }
}

fn determine_from_page_info(page_info: &PageInfo, with_cmyk: bool) -> ColorMode {
    if page_info.object_count == 0 {
        // Early return for empty pages
        return ColorMode::Mono1;
    }

    // Detect via existing colorspace
    let has_color = page_info
        .colorspaces
        .iter()
        .any(|(_, space)| colorspace_is_color(space));

    if has_color {
        if with_cmyk {
            let has_cmyk = page_info
                .colorspaces
                .iter()
                .any(|(_, space)| colorspace_contains_cmyk(space, 4));
            if has_cmyk {
                ColorMode::Cmyk8
            } else {
                ColorMode::Rgb8
            }
        } else {
            ColorMode::Rgb8
        }
    } else {
        ColorMode::Mono8
    }
}

/// An image that matters for the page's DPI, with the numbers the rules need.
#[derive(Clone, Copy)]
struct NotableImage {
    dpi: f64,
    coverage: f64,
    width: u32,
    height: u32,
}

/// The images worth weighing when picking an export DPI.
///
/// An image listing also carries entries that only make up a soft mask and never show as
/// content, so anything that is not a plain image or a stencil is dropped up front -- that
/// way a full-page scan with an alpha channel is one image, not two.
fn notable_images(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    cfg: DpiConfig,
) -> Vec<NotableImage> {
    images
        .iter()
        .filter(|img| matches!(img.image_type, ImageType::Image | ImageType::Stencil))
        // A tab or registration mark that mostly bleeds off the trimmed page and only
        // pokes a sub-point sliver back in is not content, and the coverage gate alone
        // would let it through.
        .filter(|img| image_intersecting_with_page(page_info, crop_mode, img.matrix))
        .filter_map(|img| {
            let coverage = page_coverage(page_info, crop_mode, img.matrix);
            if is_too_small(img, coverage, cfg.min_coverage, cfg.min_image_dim) {
                return None;
            }
            Some(NotableImage {
                dpi: match cfg.direction {
                    Some(AutoDPIDirection::Vertical) => img.dpi.1,
                    Some(AutoDPIDirection::Horizontal) => img.dpi.0,
                    // Only used for the colour ceiling, which needs no axis.
                    None => img.dpi.0.max(img.dpi.1),
                },
                coverage,
                width: img.width,
                height: img.height,
            })
        })
        .collect()
}

/// DPI for a page that has notable images.
///
/// A single image sets the page outright, there being nothing to weigh it against. With
/// more than one, the page is set by its dominant imagery rather than its weakest element
/// (`dpi_covering` in the sandbox prototype): take the sharpest image and keep taking
/// until they cover enough of the page.
fn determine_export_dpi(
    notable: &[NotableImage],
    direction: AutoDPIDirection,
    cfg: DpiConfig,
) -> f64 {
    if notable.is_empty() {
        return cfg.target_dpi;
    }

    if notable.len() == 1 {
        return nearest_5(notable[0].dpi)
            .min(cfg.target_dpi)
            .max(MIN_EXPORT_DPI);
    }

    match dpi_covering(notable, direction, cfg.ratio, cfg.min_coverage) {
        // Not rounded to 5: this is one image's own on-page resolution, and rounding it
        // would hide which image set the page.
        Some(dpi) => dpi.min(cfg.target_dpi).max(MIN_EXPORT_DPI),
        // Notable by their bounds, but clipped or covered up to next to nothing.
        None => cfg.target_dpi,
    }
}

/// The DPI of the sharpest image that closes `min_coverage` of the page.
///
/// Images are taken sharpest first so that a few tiny high-resolution bits (logos,
/// ornaments) never count, while a cover made of many small tiles still does.
fn dpi_covering(
    notable: &[NotableImage],
    direction: AutoDPIDirection,
    ratio: f64,
    min_coverage: f64,
) -> Option<f64> {
    let mut candidates: Vec<&NotableImage> = notable
        .iter()
        .filter(|img| image_matches_direction(img, direction, ratio))
        // Too soft to say anything about how sharp the page should be exported.
        .filter(|img| img.dpi >= MIN_EXPORT_DPI)
        .collect();
    candidates.sort_by(|a, b| b.dpi.total_cmp(&a.dpi));

    let mut covered = 0.0;
    for image in candidates {
        covered += image.coverage;
        if covered >= min_coverage {
            return Some(image.dpi);
        }
    }

    None
}

fn image_matches_direction(image: &NotableImage, direction: AutoDPIDirection, ratio: f64) -> bool {
    match direction {
        AutoDPIDirection::Vertical => u_t_f(image.height) >= u_t_f(image.width) * ratio,
        AutoDPIDirection::Horizontal => u_t_f(image.height) * ratio <= u_t_f(image.width),
    }
}

/// Fraction of the visible page an image's bounds cover, 0.0-1.0.
///
/// The bounds are clipped to the page box first, so an image that runs off the trimmed
/// page only counts the part that shows (`get_page_coverage` in the sandbox prototype).
fn page_coverage(page_info: Option<&PageInfo>, crop_mode: CropChoice, matrix: PdfMatrix) -> f64 {
    let Some(bbox) = page_info.and_then(|info| page_box(info, crop_mode)) else {
        return 0.0;
    };

    let page_area = (bbox.x2 - bbox.x1) * (bbox.y2 - bbox.y1);
    if page_area <= 0.0 {
        return 0.0;
    }

    let bounds = matrix.get_image_aabb();
    let overlap_x = bounds.x2.min(bbox.x2) - bounds.x1.max(bbox.x1);
    let overlap_y = bounds.y2.min(bbox.y2) - bounds.y1.max(bbox.y1);
    if overlap_x <= 0.0 || overlap_y <= 0.0 {
        return 0.0;
    }

    ((overlap_x * overlap_y) / page_area).clamp(0.0, 1.0)
}

/// Whether the image is too small to have a say in the page's DPI.
fn is_too_small(
    image: &ImageInfo,
    coverage: f64,
    min_coverage: f64,
    min_image_dim: Option<u32>,
) -> bool {
    if min_coverage > 0.0 && coverage < min_coverage {
        return true;
    }

    let Some(dim) = min_image_dim else {
        return false;
    };

    // Below 11px a legitimate tiny mark is likelier than a thumbnail, so both axes have
    // to fall short before the image is dropped.
    if dim < 11 {
        image.height < dim && image.width < dim
    } else {
        image.height < dim || image.width < dim
    }
}

fn u_t_f(num: u32) -> f64 {
    num as f64
}

fn nearest_5(dpi: f64) -> f64 {
    (dpi / 5.0).round() * 5.0
}

fn image_has_color(image: &ImageInfo) -> bool {
    if !matches!(image.image_type, ImageType::Image | ImageType::Stencil) {
        return false;
    }
    if matches!(image.colorspace, PdfImageColorSpace::Unknown) && image.components > 1 {
        return true;
    }
    colorspace_is_color(&image.colorspace)
}

fn image_has_cmyk(image: &ImageInfo) -> bool {
    if !matches!(image.image_type, ImageType::Image | ImageType::Stencil) {
        return false;
    }
    colorspace_contains_cmyk(&image.colorspace, image.components)
}

fn colorspace_contains_cmyk(space: &PdfImageColorSpace, components: u32) -> bool {
    match space {
        PdfImageColorSpace::DeviceCMYK => true,
        PdfImageColorSpace::Unknown => components == 4 || components == 8,
        PdfImageColorSpace::DeviceRGB
        | PdfImageColorSpace::DeviceGray
        | PdfImageColorSpace::Pattern => false,
        PdfImageColorSpace::Lab { .. } => false,
        PdfImageColorSpace::ICC { alternate } => colorspace_contains_cmyk(alternate, components),
        PdfImageColorSpace::Indexed { base, .. } => colorspace_contains_cmyk(base, components),
        PdfImageColorSpace::Separation { alternate, .. } => {
            colorspace_contains_cmyk(alternate, components)
        }
        PdfImageColorSpace::DeviceN { alternate, .. } => {
            colorspace_contains_cmyk(alternate, components)
        }
    }
}

fn colorspace_is_color(space: &PdfImageColorSpace) -> bool {
    match space {
        PdfImageColorSpace::DeviceGray => false,
        PdfImageColorSpace::DeviceRGB
        | PdfImageColorSpace::DeviceCMYK
        | PdfImageColorSpace::Lab { .. }
        | PdfImageColorSpace::ICC { .. }
        | PdfImageColorSpace::Pattern => true,
        PdfImageColorSpace::Unknown => false, // Default to non-color
        PdfImageColorSpace::Indexed {
            base,
            is_achromatic,
            ..
        } => {
            // A CMYK/RGB-backed palette can still be entirely grayscale/black
            // (e.g. scanned line art saved through an indexed CMYK palette).
            colorspace_is_color(base) && !is_achromatic
        }
        PdfImageColorSpace::Separation {
            name, alternate, ..
        } => {
            let is_color = colorspace_is_color(alternate);
            let is_achromatic = is_achromatic_color_name(name);

            // Sometimes it fallback to DeviceCMYK but only has achromatic name
            is_color && !is_achromatic
        }
        PdfImageColorSpace::DeviceN {
            names, alternate, ..
        } => {
            let is_color = colorspace_is_color(alternate);
            let all_achromatic = names.iter().all(|name| is_achromatic_color_name(name));

            // Sometimes it fallback to DeviceCMYK but only has achromatic names
            is_color && !all_achromatic
        }
    }
}

fn is_achromatic_color_name(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "black" | "gray" | "grey" | "darkgray" | "darkgrey" | "lightgray" | "lightgrey"
    )
}

fn image_intersecting_with_page(
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    matrix: PdfMatrix,
) -> bool {
    match page_info.and_then(|info| page_box(info, crop_mode)) {
        Some(b) => image_is_intersecting(matrix, b),
        None => false,
    }
}

/// The page box an export is measured against.
fn page_box(info: &PageInfo, crop_mode: CropChoice) -> Option<PdfRect> {
    match crop_mode {
        CropChoice::Crop => info.cropbox,
        CropChoice::Media => info.mediabox,
    }
}

/// Minimum overlap (in PDF points) required along both axes for an image's AABB to
/// count as intersecting the page box. A plain nonzero-overlap AABB test flags images
/// that only clip the box by a hairline -- e.g. a page-edge tab/registration mark that
/// mostly bleeds off the trimmed page and only pokes a sub-point sliver back in -- as
/// full page content, which skews auto color/DPI detection. Requiring a small minimum
/// overlap filters those out while still catching genuine bleed content, which overlaps
/// by tens to hundreds of points.
const MIN_INTERSECTION_OVERLAP_PT: f64 = 2.0;

/// Below this on-page DPI an image is too soft to have a say in the export resolution.
const MIN_EXPORT_DPI: f64 = 72.0;

/// Checks if the Image (defined by matrix) intersects the Page CropBox
pub fn image_is_intersecting(matrix: PdfMatrix, bbox: PdfRect) -> bool {
    let img_aabb = matrix.get_image_aabb();

    let overlap_x = img_aabb.x2.min(bbox.x2) - img_aabb.x1.max(bbox.x1);
    let overlap_y = img_aabb.y2.min(bbox.y2) - img_aabb.y1.max(bbox.y1);

    overlap_x >= MIN_INTERSECTION_OVERLAP_PT && overlap_y >= MIN_INTERSECTION_OVERLAP_PT
}

impl ColorChoice {
    fn to_color_mode(self) -> Option<ColorMode> {
        match self {
            ColorChoice::Auto => None,
            ColorChoice::Mono1 => Some(ColorMode::Mono1),
            ColorChoice::Mono8 => Some(ColorMode::Mono8),
            ColorChoice::Rgb8 => Some(ColorMode::Rgb8),
            ColorChoice::Bgr8 => Some(ColorMode::Bgr8),
            ColorChoice::Xbgr8 => Some(ColorMode::Xbgr8),
            ColorChoice::Cmyk8 => Some(ColorMode::Cmyk8),
            ColorChoice::Devicen8 => Some(ColorMode::DeviceN8),
        }
    }
}

impl CropChoice {
    fn to_crop_mode(self) -> PdfCropMode {
        match self {
            CropChoice::Crop => PdfCropMode::CropBox,
            CropChoice::Media => PdfCropMode::MediaBox,
        }
    }
}

impl ZeroWidthLineChoice {
    fn to_zero_width_line_mode(self) -> ZeroWidthLineMode {
        match self {
            ZeroWidthLineChoice::Default => ZeroWidthLineMode::Default,
            ZeroWidthLineChoice::Hairline => ZeroWidthLineMode::Hairline,
            ZeroWidthLineChoice::None => ZeroWidthLineMode::Nothing,
        }
    }
}

impl GlyphFillChoice {
    fn to_glyph_fill_mode(self) -> GlyphFillMode {
        match self {
            GlyphFillChoice::Bitmap => GlyphFillMode::Bitmap,
            GlyphFillChoice::Path => GlyphFillMode::Path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A square page anchored at the origin.
    fn page(side: f64) -> PageInfo {
        let rect = PdfRect {
            x1: 0.0,
            y1: 0.0,
            x2: side,
            y2: side,
        };
        PageInfo {
            page: 1,
            image_count: 0,
            object_count: 0,
            is_pdf_a_compatible: false,
            mediabox: Some(rect),
            cropbox: Some(rect),
            colorspaces: HashMap::new(),
        }
    }

    /// Places an image's unit square at (x, y) with the given size in points.
    fn placed(x: f64, y: f64, width: f64, height: f64) -> PdfMatrix {
        PdfMatrix {
            a: width,
            b: 0.0,
            c: 0.0,
            d: height,
            e: x,
            f: y,
        }
    }

    fn image_of(
        image_type: ImageType,
        width: u32,
        height: u32,
        dpi: f64,
        matrix: PdfMatrix,
    ) -> ImageInfo {
        ImageInfo {
            width,
            height,
            components: 1,
            bits_per_component: 8,
            colorspace: PdfImageColorSpace::DeviceGray,
            image_type,
            page: 1,
            dpi: (dpi, dpi),
            matrix,
            xref: None,
        }
    }

    fn image(width: u32, height: u32, dpi: f64, matrix: PdfMatrix) -> ImageInfo {
        image_of(ImageType::Image, width, height, dpi, matrix)
    }

    fn notable(dpi: f64, coverage: f64) -> NotableImage {
        // Tall, so the default 1.25 vertical aspect filter keeps it.
        NotableImage {
            dpi,
            coverage,
            width: 100,
            height: 200,
        }
    }

    fn dpi_cfg(target_dpi: f64, min_coverage: f64) -> DpiConfig {
        DpiConfig {
            target_dpi,
            direction: Some(AutoDPIDirection::Vertical),
            ratio: 1.25,
            min_coverage,
            min_image_dim: None,
            color_export_dpi: None,
        }
    }

    // --- page_coverage ---

    #[test]
    fn a_full_page_image_covers_the_whole_page() {
        let p = page(100.0);
        assert_eq!(
            page_coverage(Some(&p), CropChoice::Crop, placed(0.0, 0.0, 100.0, 100.0)),
            1.0
        );
    }

    #[test]
    fn coverage_clips_an_image_running_off_the_page() {
        // Half of a 100x100 image hangs past the right and top edges.
        let p = page(100.0);
        let coverage = page_coverage(Some(&p), CropChoice::Crop, placed(50.0, 50.0, 100.0, 100.0));
        assert!((coverage - 0.25).abs() < 1e-9, "got {coverage}");
    }

    #[test]
    fn an_image_entirely_off_the_page_covers_nothing() {
        let p = page(100.0);
        assert_eq!(
            page_coverage(Some(&p), CropChoice::Crop, placed(200.0, 200.0, 10.0, 10.0)),
            0.0
        );
    }

    #[test]
    fn a_page_without_a_box_reports_no_coverage() {
        assert_eq!(
            page_coverage(None, CropChoice::Crop, placed(0.0, 0.0, 100.0, 100.0)),
            0.0
        );
    }

    // --- is_too_small ---

    #[test]
    fn the_coverage_gate_drops_sparse_images() {
        let img = image(500, 500, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        assert!(is_too_small(&img, 0.10, 0.15, None));
        assert!(!is_too_small(&img, 0.20, 0.15, None));
    }

    #[test]
    fn a_zero_coverage_gate_weighs_every_image() {
        let img = image(500, 500, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        assert!(!is_too_small(&img, 0.0, 0.0, None));
    }

    #[test]
    fn a_tiny_dimension_gate_needs_both_axes_small() {
        let gate = Some(10u32);
        let lopsided = image(8, 20, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        let tiny = image(8, 8, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        assert!(!is_too_small(&lopsided, 1.0, 0.0, gate));
        assert!(is_too_small(&tiny, 1.0, 0.0, gate));
    }

    #[test]
    fn a_larger_dimension_gate_is_satisfied_on_either_axis() {
        let gate = Some(100u32);
        let lopsided = image(50, 200, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        let big = image(200, 200, 300.0, placed(0.0, 0.0, 100.0, 100.0));
        assert!(is_too_small(&lopsided, 1.0, 0.0, gate));
        assert!(!is_too_small(&big, 1.0, 0.0, gate));
    }

    // --- notable_images ---

    #[test]
    fn a_soft_mask_entry_is_not_a_notable_image() {
        let p = page(100.0);
        let images = [
            image(1000, 1000, 300.0, placed(0.0, 0.0, 100.0, 100.0)),
            image_of(
                ImageType::SoftMask,
                1000,
                1000,
                300.0,
                placed(0.0, 0.0, 100.0, 100.0),
            ),
        ];
        let notable = notable_images(&images, Some(&p), CropChoice::Crop, dpi_cfg(600.0, 0.15));
        assert_eq!(notable.len(), 1);
    }

    // --- dpi_covering ---

    #[test]
    fn the_sharpest_image_that_closes_the_coverage_sets_the_dpi() {
        // 10% + 10% crosses the 15% gate only once the 200dpi image is counted.
        let images = [notable(400.0, 0.10), notable(200.0, 0.10)];
        let dpi = dpi_covering(&images, AutoDPIDirection::Vertical, 1.25, 0.15);
        assert_eq!(dpi, Some(200.0));
    }

    #[test]
    fn a_tiny_high_res_logo_does_not_set_the_dpi() {
        let images = [notable(600.0, 0.01), notable(150.0, 0.99)];
        let dpi = dpi_covering(&images, AutoDPIDirection::Vertical, 1.25, 0.15);
        assert_eq!(dpi, Some(150.0));
    }

    #[test]
    fn coverage_that_never_adds_up_has_no_answer() {
        let images = [notable(600.0, 0.05), notable(300.0, 0.05)];
        assert_eq!(
            dpi_covering(&images, AutoDPIDirection::Vertical, 1.25, 0.15),
            None
        );
    }

    // --- determine_export_dpi ---

    #[test]
    fn a_single_notable_image_sets_the_page() {
        let images = [notable(300.0, 1.0)];
        let dpi = determine_export_dpi(&images, AutoDPIDirection::Vertical, dpi_cfg(600.0, 0.15));
        assert_eq!(dpi, 300.0);
    }

    #[test]
    fn no_notable_images_falls_back_to_the_target() {
        let dpi = determine_export_dpi(&[], AutoDPIDirection::Vertical, dpi_cfg(600.0, 0.15));
        assert_eq!(dpi, 600.0);
    }

    #[test]
    fn the_target_dpi_is_never_exceeded() {
        let images = [notable(600.0, 1.0)];
        let dpi = determine_export_dpi(&images, AutoDPIDirection::Vertical, dpi_cfg(300.0, 0.15));
        assert_eq!(dpi, 300.0);
    }

    /// Regression: a soft mask used to make a one-image page look like a two-image page,
    /// which skipped the single-image rule and sent the page down the aspect-filtered
    /// path. A wide scan then had no candidates at all and rendered at the target DPI.
    #[test]
    fn a_soft_mask_sibling_does_not_force_the_multi_image_path() {
        let p = page(100.0);
        // 4:1 wide, so it fails the default 1.25 vertical aspect filter.
        let wide = image(2000, 500, 300.0, placed(0.0, 0.0, 100.0, 25.0));
        let masked = image_of(
            ImageType::SoftMask,
            2000,
            500,
            72.0,
            placed(0.0, 0.0, 100.0, 25.0),
        );

        let without_mask = precalculate_auto_export_config(
            std::slice::from_ref(&wide),
            Some(&p),
            CropChoice::Crop,
            false,
            dpi_cfg(600.0, 0.15),
        );
        let with_mask = precalculate_auto_export_config(
            &[wide, masked],
            Some(&p),
            CropChoice::Crop,
            false,
            dpi_cfg(600.0, 0.15),
        );

        assert_eq!(without_mask.dpi, 300.0);
        assert_eq!(
            with_mask.dpi, 300.0,
            "the soft mask must not change the answer"
        );
    }

    // --- colour ceiling ---

    #[test]
    fn the_colour_ceiling_lowers_an_image_less_page() {
        let mut p = page(100.0);
        p.object_count = 1;
        p.colorspaces
            .insert("CS0".to_string(), PdfImageColorSpace::DeviceRGB);

        let mut cfg = dpi_cfg(600.0, 0.15);
        cfg.color_export_dpi = Some(150.0);

        let guessed = precalculate_auto_export_config(&[], Some(&p), CropChoice::Crop, false, cfg);
        assert_eq!(guessed.color, ColorMode::Rgb8);
        assert_eq!(guessed.dpi, 150.0);
    }

    #[test]
    fn the_colour_ceiling_leaves_a_grayscale_page_alone() {
        let mut p = page(100.0);
        p.object_count = 1;
        p.colorspaces
            .insert("CS0".to_string(), PdfImageColorSpace::DeviceGray);

        let mut cfg = dpi_cfg(600.0, 0.15);
        cfg.color_export_dpi = Some(150.0);

        let guessed = precalculate_auto_export_config(&[], Some(&p), CropChoice::Crop, false, cfg);
        assert_eq!(guessed.dpi, 600.0);
    }

    // --- --output-dpi downscale ---

    fn raw_image(
        bytes: Vec<u8>,
        width: usize,
        height: usize,
        dpi: f64,
        components: usize,
    ) -> tiny_poppler::EncodedImage {
        tiny_poppler::EncodedImage {
            format: tiny_poppler::ImageFormat::Raw,
            bytes,
            width,
            height,
            dpi,
            components,
        }
    }

    #[test]
    fn downscaling_halves_the_pixel_dimensions() {
        let (width, height) = (40, 30);
        let encoded = raw_image(vec![128; width * height * 3], width, height, 300.0, 3);

        let out = downscale(encoded, 150.0).expect("resize");
        assert_eq!((out.width, out.height), (20, 15));
        assert_eq!(out.dpi, 150.0);
        assert_eq!(out.bytes.len(), 20 * 15 * 3);
    }

    #[test]
    fn downscaling_keeps_every_channel() {
        let (width, height) = (40, 30);
        let encoded = raw_image(vec![200; width * height * 4], width, height, 600.0, 4);

        let out = downscale(encoded, 300.0).expect("resize");
        assert_eq!((out.width, out.height), (20, 15));
        assert_eq!(out.components, 4);
        assert_eq!(out.bytes.len(), 20 * 15 * 4);
    }

    #[test]
    fn a_flat_fill_survives_the_average() {
        let (width, height) = (16, 16);
        let encoded = raw_image(vec![100; width * height], width, height, 300.0, 1);

        let out = downscale(encoded, 150.0).expect("resize");
        assert!(
            out.bytes.iter().all(|&pixel| pixel == 100),
            "a uniform page should average to itself"
        );
    }

    #[test]
    fn an_already_encoded_image_is_left_alone() {
        let mut encoded = raw_image(vec![0; 16 * 16 * 3], 16, 16, 300.0, 3);
        encoded.format = tiny_poppler::ImageFormat::Png;

        let out = downscale(encoded, 150.0).expect("no-op");
        assert_eq!((out.width, out.height, out.dpi), (16, 16, 300.0));
    }

    #[test]
    fn an_unsupported_layout_is_left_alone() {
        // Two channels per pixel: nothing Splash produces, and nothing the resizer can express.
        let (width, height) = (16, 16);
        let encoded = raw_image(vec![0; width * height * 2], width, height, 300.0, 2);

        let out = downscale(encoded, 150.0).expect("no-op");
        assert_eq!((out.width, out.height, out.dpi), (16, 16, 300.0));
    }

    #[test]
    fn downscaling_to_the_same_resolution_is_a_no_op() {
        let (width, height) = (16, 16);
        let encoded = raw_image(vec![0; width * height * 3], width, height, 150.0, 3);

        let out = downscale(encoded, 150.0).expect("no-op");
        assert_eq!((out.width, out.height, out.dpi), (16, 16, 150.0));
    }

    #[test]
    fn bit_packed_and_bgr_output_are_never_downscaled() {
        let mut options = RenderOptions::default();

        for mode in [ColorMode::Mono1, ColorMode::Bgr8, ColorMode::Xbgr8] {
            options.color_mode = mode;
            assert!(!can_downscale(&options), "{mode:?} must not be downscaled");
        }

        for mode in [
            ColorMode::Mono8,
            ColorMode::Rgb8,
            ColorMode::Cmyk8,
            ColorMode::DeviceN8,
        ] {
            options.color_mode = mode;
            assert!(can_downscale(&options), "{mode:?} should be downscaled");
        }
    }
}
