use clap::{Args, ValueEnum};
use color_print::{cformat, cprintln};
use crossbeam_channel::unbounded;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::thread;
use std::{collections::HashMap, fs};
use tiny_poppler::{
    ColorMode, Document, DocumentFactory, ImageInfo, ImageType, PdfCropMode, PdfImageColorSpace,
    RenderOptions,
};

#[derive(Args)]
pub struct ExportArgs {
    /// Path to the PDF file to export.
    pub pdf: PathBuf,
    /// Directory where rendered pages will be written.
    pub output: PathBuf,
    /// Colorspace to request from Poppler. Auto keeps the library default.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
    #[arg(long, value_enum, default_value_t = CropChoice::CropBox)]
    pub crop: CropChoice,
    /// DPI used when rendering the page raster.
    #[arg(long, default_value_t = 150.0)]
    pub dpi: f64,
    /// Auto-DPI based on image characteristics.
    ///
    /// Select the direction constraint for auto-DPI calculation.
    /// - Horizontal: x-dpi
    /// - Vertical: y-dpi (Manga/Comics/etc)
    #[arg(long, value_enum)]
    pub auto_dpi: Option<AutoDPIDirection>,
    /// When in Auto color mode, if we encounter RGB colorspace,
    /// do additional check whether there is CMYK content (or color with CMYK fallback).
    #[arg(long, default_value_t = false)]
    pub with_cmyk: bool,
    /// JPEG quality (1-100) when exporting to JPEG files.
    #[arg(long, default_value_t = 96, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub quality: u8,
    /// First page to export (1-based).
    #[arg(long)]
    pub first: Option<u32>,
    /// Last page to export (1-based).
    #[arg(long)]
    pub last: Option<u32>,
    /// Reverse the page order during export.
    #[arg(long, default_value_t = false)]
    pub reverse: bool,
    #[arg(long, default_value_t = false)]
    pub describe: bool,
    /// Worker threads to use during export (omit for auto).
    #[arg(long, value_parser = clap::value_parser!(NonZeroUsize))]
    pub threads: Option<NonZeroUsize>,
}

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
    CropBox,
    MediaBox,
    // ArtBox,
    // BleedBox,
    // TrimBox,
}

pub fn run(args: ExportArgs) -> Result<(), String> {
    if args.dpi <= 0.0 {
        return Err("dpi must be a positive number".into());
    }

    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    let ExportArgs {
        pdf,
        output,
        color,
        dpi,
        first,
        last,
        crop,
        describe,
        auto_dpi,
        with_cmyk,
        reverse,
        quality,
        threads,
    } = args;

    cprintln!("Loading PDF: <m,s>{:#?}</m,s>", &pdf);
    let factory = DocumentFactory::with_images(&pdf).map_err(|err| err.to_string())?;
    let page_count = factory.page_count();

    let first_page = first.unwrap_or(1);
    if first_page == 0 || first_page > page_count {
        return Err(format!("first page must be between 1 and {page_count}"));
    }

    let last_page = last.unwrap_or(page_count);
    if last_page == 0 || last_page > page_count {
        return Err(format!("last page must be between 1 and {page_count}"));
    }
    if last_page < first_page {
        return Err("last page must be greater than or equal to first page".into());
    }

    if !describe && let Err(err) = fs::create_dir_all(&output) {
        return Err(format!("failed to create output directory: {err}"));
    }

    println!("Preloading images...");
    let images_metadata = factory
        .images()
        .map(|slice| slice.to_vec())
        .unwrap_or_default();

    let base_options = {
        let mut opts = RenderOptions::default();
        opts.dpi = dpi;
        opts.crop_mode = crop.to_crop_mode();
        opts.jpeg_quality = Some(quality);
        if let Some(mode) = color.to_color_mode() {
            opts.color_mode = mode;
        }
        opts
    };

    let mut images_mappings: HashMap<u32, Vec<ImageInfo>> = HashMap::new();
    for item in images_metadata {
        images_mappings.entry(item.page).or_default().push(item);
    }
    println!("Precalculating export settings...");
    let mut precalculated_settings: HashMap<u32, GuessedImage> = HashMap::new();
    for (page, images) in &images_mappings {
        let guessed = precalculate_auto_export_config(images, with_cmyk, dpi, auto_dpi);
        precalculated_settings.insert(*page, guessed);
    }

    println!("Starting export...");

    let pages: Vec<u32> = if reverse {
        (first_page..=last_page).rev().collect()
    } else {
        (first_page..=last_page).collect()
    };

    let jobs: Vec<PagePlan> = pages
        .into_iter()
        .enumerate()
        .map(|(idx, page)| {
            let mut per_page = base_options.clone();
            if color == ColorChoice::Auto {
                per_page.color_mode = precalculated_settings
                    .get(&page)
                    .map(|pre| pre.color)
                    .unwrap_or(ColorMode::Mono8);
            }
            if auto_dpi.is_some() {
                per_page.dpi = precalculated_settings
                    .get(&page)
                    .map(|pre| pre.dpi)
                    .unwrap_or(dpi);
            }
            let file_number = idx + 1;
            let extension = extension_for_mode(per_page.color_mode);
            let file_name = format!("page-{file_number:04}.{extension}");
            let output_path = output.join(file_name);
            PagePlan {
                page_number: page,
                zero_index_page: page - 1,
                output_path,
                options: per_page,
            }
        })
        .collect();

    if describe {
        for job in &jobs {
            cprintln!(
                "Will export page <m,s>{}</m,s> -> <m,s>{}</m,s> (colorspace: {:?}, crop: {:?}, dpi: {})",
                job.page_number,
                job.output_path.display(),
                job.options.color_mode,
                job.options.crop_mode,
                job.options.dpi,
            );
        }
        return Ok(());
    }

    run_export_jobs(factory, jobs, threads.map(NonZeroUsize::get))
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
            CropChoice::CropBox => PdfCropMode::CropBox,
            CropChoice::MediaBox => PdfCropMode::MediaBox,
        }
    }
}

#[derive(Clone)]
struct PagePlan {
    page_number: u32,
    zero_index_page: u32,
    output_path: PathBuf,
    options: RenderOptions,
}

fn run_export_jobs(
    factory: DocumentFactory,
    jobs: Vec<PagePlan>,
    threads: Option<usize>,
) -> Result<(), String> {
    if jobs.is_empty() {
        return Ok(());
    }

    let cpu_count = num_cpus::get().max(1);
    let (desired, requested_label) = match threads {
        Some(value) => (value, value.to_string()),
        None => (cpu_count, "auto".into()),
    };
    let worker_count = desired.max(1).min(jobs.len().max(1));
    cprintln!(
        "Spawning <m,s>{worker_count}</m,s> worker(s) (<c,s>{}</c,s> logical CPUs detected, requested: <c,s>{requested_label}</c,s>)...",
        cpu_count,
    );

    let (sender, receiver) = unbounded::<PagePlan>();
    let mut handles = Vec::with_capacity(worker_count);

    for worker_index in 0..worker_count {
        let rx = receiver.clone();
        let factory = factory.clone();
        handles.push(thread::spawn(move || -> Result<(), String> {
            let mut document = factory.open().map_err(|err| {
                cformat!(
                    "worker <c,s>{}</c,s> failed to open PDF: <m,s>{err}</m,s>",
                    worker_index + 1
                )
            })?;
            while let Ok(job) = rx.recv() {
                if let Err(err) = process_job(&mut document, job) {
                    return Err(err);
                }
            }
            Ok(())
        }));
    }

    drop(receiver);

    for job in jobs {
        sender
            .send(job)
            .map_err(|_| "render queue closed unexpectedly".to_string())?;
    }
    drop(sender);

    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err("worker thread panicked".into()),
        }
    }

    Ok(())
}

fn process_job(document: &mut Document, job: PagePlan) -> Result<(), String> {
    let encoded = document
        .render_page_image_bytes(job.zero_index_page, &job.options)
        .map_err(|err| format!("failed to export page {}: {err}", job.page_number))?;
    fs::write(&job.output_path, &encoded.bytes)
        .map_err(|err| format!("failed to write {}: {err}", job.output_path.display()))?;
    cprintln!(
        "Exported <m,s>page {}</m,s> -> <m,s>{}</m,s> ({:?}, {:?}, {} dpi)",
        job.page_number,
        job.output_path.display(),
        encoded.format,
        job.options.color_mode,
        job.options.dpi,
    );
    Ok(())
}

struct GuessedImage {
    color: ColorMode,
    dpi: f64,
}

fn precalculate_auto_export_config(
    images: &[ImageInfo],
    with_cmyk: bool,
    target_dpi: f64,
    direction: Option<AutoDPIDirection>,
) -> GuessedImage {
    let color = determine_page_colorspace(images, with_cmyk);
    let dpi = if let Some(direction) = direction {
        determine_export_dpi(images, target_dpi, direction)
    } else {
        target_dpi
    };

    GuessedImage { color, dpi }
}

fn determine_page_colorspace(images: &[ImageInfo], with_cmyk: bool) -> ColorMode {
    if images.iter().any(image_has_color) {
        if with_cmyk && images.iter().any(image_has_cmyk) {
            ColorMode::Cmyk8
        } else {
            ColorMode::Rgb8
        }
    } else {
        ColorMode::Mono8
    }
}

fn determine_export_dpi(images: &[ImageInfo], target_dpi: f64, direction: AutoDPIDirection) -> f64 {
    if images.is_empty() {
        return target_dpi;
    }

    let candidates: Vec<f64> = if direction == AutoDPIDirection::Vertical {
        images
            .iter()
            .filter(|&img| matches!(img.image_type, ImageType::Stencil | ImageType::Image))
            .filter(|&img| img.dpi.1 >= img.dpi.0 * 1.25)
            .map(|f| f.dpi.1)
            .collect()
    } else {
        images
            .iter()
            .filter(|&img| matches!(img.image_type, ImageType::Stencil | ImageType::Image))
            .filter(|&img| img.dpi.1 * 1.25 <= img.dpi.0)
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

fn nearest_5(dpi: f64) -> f64 {
    (dpi / 5.0).round() * 5.0
}

fn image_has_color(image: &ImageInfo) -> bool {
    if !matches!(image.image_type, ImageType::Image | ImageType::Stencil) {
        return false;
    }
    colorspace_contains_color(&image.colorspace, image.components)
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

fn colorspace_contains_color(space: &PdfImageColorSpace, components: u32) -> bool {
    match space {
        PdfImageColorSpace::DeviceGray => false,
        PdfImageColorSpace::Unknown => components > 1,
        PdfImageColorSpace::DeviceRGB
        | PdfImageColorSpace::DeviceCMYK
        | PdfImageColorSpace::Lab { .. }
        | PdfImageColorSpace::ICC { .. }
        | PdfImageColorSpace::Pattern => true,
        PdfImageColorSpace::Indexed { base, .. } => colorspace_contains_color(base, components),
        PdfImageColorSpace::Separation { alternate, .. } => {
            colorspace_contains_color(alternate, components)
        }
        PdfImageColorSpace::DeviceN { alternate, .. } => {
            colorspace_contains_color(alternate, components)
        }
    }
}

fn extension_for_mode(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Cmyk8 | ColorMode::DeviceN8 => "jpg",
        _ => "png",
    }
}
