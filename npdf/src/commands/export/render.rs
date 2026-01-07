use clap::ValueEnum;
use color_eyre::{Result, eyre::Context};
use color_print::cprintln;
use rayon::iter::{IntoParallelRefMutIterator, ParallelIterator};
use std::fs;
use std::path::PathBuf;
use tiny_poppler::{
    ColorMode, Document, DocumentFactory, ImageInfo, ImageType, PageInfo, PdfCropMode,
    PdfImageColorSpace, PdfMatrix, PdfRect, RenderOptions, ZeroWidthLineMode, cmyk2gray, cmyk2rgb,
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

#[derive(Clone)]
pub(super) struct RenderPagePlan {
    pub page_number: u32,
    pub zero_index_page: u32,
    pub total_pages: u32,
    pub output_path: Option<PathBuf>,
    request_color: ColorChoice,
    detected_color: ColorMode,
    pub options: RenderOptions,
}

pub(super) fn process_job(document: &mut Document, job: &RenderPagePlan) -> Result<()> {
    let output_path = job.output_path.as_ref().ok_or(NpdfError::OutputNotSet)?;

    let encoded = document
        .render_page_image_bytes(job.zero_index_page, &job.options)
        .context(format!("page {}", job.page_number))?;

    let (bytes_data, extension) = match encoded.format {
        tiny_poppler::ImageFormat::Png => (encoded.bytes, "png"),
        tiny_poppler::ImageFormat::Jpeg => (encoded.bytes, "png"),
        tiny_poppler::ImageFormat::Raw => {
            // For raw bitmap, currently only used for CMYK PNG export
            let want_color = if job.request_color == ColorChoice::Auto {
                job.detected_color
            } else {
                job.request_color.to_color_mode().unwrap_or(ColorMode::Rgb8)
            };

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
                    // Export as PNG, use cmyk2gray function
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

                    let as_gray = cmyk2gray(&encoded.bytes)
                        .context(format!("page {}, cmyk2gray", job.page_number))?;

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
                    // Export as PNG, convert to RGB first
                    let rgb_data = cmyk2rgb(&encoded.bytes)
                        .context(format!("page {}, cmyk2rgb", job.page_number))?;

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
        job.options.dpi,
    );
    Ok(())
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
    };

    let guessed = precalculate_auto_export_config(
        images,
        Some(page_info),
        *bounding,
        *with_cmyk,
        *dpi,
        *auto_dpi,
        *auto_dpi_ratio,
    );

    let mut options = base_options.clone();
    if *color == ColorChoice::Auto && !*cmyk_png {
        options.color_mode = guessed.color;
    }
    if auto_dpi.is_some() {
        options.dpi = guessed.dpi;
    }

    let file_name = format!("page-{output_page:04}");
    let with_output_path = output_path.as_ref().map(|o| o.join(&file_name));

    queue_job(RenderPagePlan {
        page_number: page_info.page,
        zero_index_page: page_info.page - 1,
        total_pages: factory.page_count(),
        output_path: with_output_path,
        request_color: *color,
        detected_color: guessed.color,
        options,
    })
}

struct GuessedImage {
    color: ColorMode,
    dpi: f64,
}

fn precalculate_auto_export_config(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    with_cmyk: bool,
    target_dpi: f64,
    direction: Option<AutoDPIDirection>,
    dpi_ratio: f64,
) -> GuessedImage {
    let color = determine_page_colorspace(images, page_info, with_cmyk);
    let dpi = if let Some(direction) = direction {
        determine_export_dpi(
            images, page_info, crop_mode, target_dpi, direction, dpi_ratio,
        )
    } else {
        target_dpi
    };

    GuessedImage { color, dpi }
}

fn determine_page_colorspace(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    with_cmyk: bool,
) -> ColorMode {
    if images.is_empty() {
        match page_info {
            Some(info) => determine_from_page_info(info, with_cmyk),
            _ => ColorMode::Mono1,
        }
    } else if images.iter().any(image_has_color) {
        if with_cmyk && images.iter().any(image_has_cmyk) {
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

fn determine_export_dpi(
    images: &[ImageInfo],
    page_info: Option<&PageInfo>,
    crop_mode: CropChoice,
    target_dpi: f64,
    direction: AutoDPIDirection,
    dpi_ratio: f64,
) -> f64 {
    if images.is_empty() {
        return target_dpi;
    }

    let candidates: Vec<f64> = if direction == AutoDPIDirection::Vertical {
        images
            .iter()
            .filter(|&img| matches!(img.image_type, ImageType::Stencil | ImageType::Image))
            .filter(|&img| image_intersecting_with_page(page_info, crop_mode, img.matrix))
            .filter(|&img| img.dpi.1 >= 72.0) // Filter out images with DPI smaller than 72 for some reason
            .filter(|&img| u_t_f(img.height) >= u_t_f(img.width) * dpi_ratio)
            .map(|f| f.dpi.1)
            .collect()
    } else {
        images
            .iter()
            .filter(|&img| matches!(img.image_type, ImageType::Stencil | ImageType::Image))
            .filter(|&img| image_intersecting_with_page(page_info, crop_mode, img.matrix))
            .filter(|&img| img.dpi.0 >= 72.0) // Filter out images with DPI smaller than 72 for some reason
            .filter(|&img| u_t_f(img.height) * dpi_ratio <= u_t_f(img.width))
            .map(|f| f.dpi.0)
            .collect()
    };

    if images.len() == 1 {
        let smallest = if direction == AutoDPIDirection::Vertical {
            images[0].dpi.1
        } else {
            images[0].dpi.0
        };

        nearest_5(smallest).min(target_dpi).max(72.0)
    } else if candidates.is_empty() {
        target_dpi
    } else {
        let mut smallest = candidates[0];
        for dpi in candidates {
            if dpi < smallest {
                smallest = dpi;
            }
        }

        nearest_5(smallest).min(target_dpi).max(72.0)
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
        PdfImageColorSpace::Indexed { base, .. } => colorspace_is_color(base),
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
    if let Some(info) = page_info {
        let bbox = match crop_mode {
            CropChoice::Crop => info.cropbox,
            CropChoice::Media => info.mediabox,
        };
        match bbox {
            Some(b) => image_is_intersecting(matrix, b),
            None => false,
        }
    } else {
        false
    }
}

/// Checks if the Image (defined by matrix) intersects the Page CropBox
pub fn image_is_intersecting(matrix: PdfMatrix, bbox: PdfRect) -> bool {
    let img_aabb = matrix.get_image_aabb();

    // Separating Axis Theorem (Simplified for AABB)
    // If one is to the left/right/top/bottom of the other, they do not intersect.
    if img_aabb.x2 < bbox.x1
        || img_aabb.x1 > bbox.x2
        || img_aabb.y2 < bbox.y1
        || img_aabb.y1 > bbox.y2
    {
        false
    } else {
        // Overlap detected
        true
    }
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
