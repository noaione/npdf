use clap::{Parser, Subcommand, ValueEnum};
use crossbeam_channel::unbounded;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::thread;
use std::{collections::HashMap, fs};

use tiny_poppler::{
    ColorMode, Document, DocumentFactory, ImageInfo, ImageType, PdfCropMode, PdfImageColorSpace,
    RenderOptions,
};

fn main() {
    let cli = Cli::parse();
    if let Err(err) = execute(cli) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::List(args) => handle_list(args),
        Commands::Export(args) => handle_export(args),
    }
}

#[derive(Parser)]
#[command(name = "npdf", version, about = "PDF helper built on Poppler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List images embedded in the PDF.
    List(ListArgs),
    /// Export pages from the PDF to PNG or JPEG files.
    Export(ExportArgs),
}

#[derive(clap::Args)]
struct ListArgs {
    /// Path to the PDF file to inspect.
    pdf: PathBuf,
}

#[derive(clap::Args)]
struct ExportArgs {
    /// Path to the PDF file to export.
    pdf: PathBuf,
    /// Directory where rendered pages will be written.
    output: PathBuf,
    /// Colorspace to request from Poppler. Auto keeps the library default.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
    #[arg(long, value_enum, default_value_t = CropChoice::CropBox)]
    crop: CropChoice,
    /// DPI used when rendering the page raster.
    #[arg(long, default_value_t = 150.0)]
    dpi: f64,
    /// Auto-DPI based on image characteristics.
    ///
    /// Select the direction constraint for auto-DPI calculation.
    /// - Horizontal: x-dpi
    /// - Vertical: y-dpi (Manga/Comics/etc)
    #[arg(long, value_enum)]
    auto_dpi: Option<AutoDPIDirection>,
    /// When in Auto color mode, if we encounter RGB colorspace,
    /// do additional check whether there is CMYK content (or color with CMYK fallback).
    #[arg(long, default_value_t = false)]
    with_cmyk: bool,
    /// JPEG quality (1-100) when exporting to JPEG files.
    #[arg(long, default_value_t = 96, value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: u8,
    /// First page to export (1-based).
    #[arg(long)]
    first: Option<u32>,
    /// Last page to export (1-based).
    #[arg(long)]
    last: Option<u32>,
    /// Reverse the page order during export.
    #[arg(long, default_value_t = false)]
    reverse: bool,
    #[arg(long, default_value_t = false)]
    describe: bool,
    /// Worker threads to use during export (omit for auto).
    #[arg(long, value_parser = clap::value_parser!(NonZeroUsize))]
    threads: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum ColorChoice {
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
enum AutoDPIDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum CropChoice {
    CropBox,
    MediaBox,
    // ArtBox,
    // BleedBox,
    // TrimBox,
}

fn handle_list(args: ListArgs) -> Result<(), String> {
    let mut document = Document::open(&args.pdf).map_err(|err| err.to_string())?;
    let page_count = document.page_count().map_err(|err| err.to_string())?;
    let images = document.images().map_err(|err| err.to_string())?;

    println!("PDF: {}", args.pdf.display());
    println!("Pages: {page_count}");

    if images.is_empty() {
        println!("No embedded images found.");
        return Ok(());
    }

    for (idx, info) in images.iter().enumerate() {
        let position = idx + 1;
        let colorspace = describe_colorspace(&info.colorspace);
        let xref = match info.xref {
            Some((obj, generation)) => format!("{} {} R", obj, generation),
            None => "inline".into(),
        };
        let image_type = describe_image_type(info.image_type);
        println!(
            "{position:>4}: page {page:>4}, {image_type}, {width}x{height}px, {components} comps, {bits} bpc, {colorspace}, xref {xref}, {dpi_x} xdpi, {dpi_y} ydpi",
            page = info.page,
            width = info.width,
            height = info.height,
            components = info.components,
            bits = info.bits_per_component,
            dpi_x = fmt_dpi(info.dpi.0),
            dpi_y = fmt_dpi(info.dpi.1),
        );
    }

    Ok(())
}

fn handle_export(args: ExportArgs) -> Result<(), String> {
    if args.dpi <= 0.0 {
        return Err("dpi must be a positive number".into());
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

    println!("Loading PDF: {:#?}", &pdf);
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
            println!(
                "Will export page {} -> {} (colorspace: {:?}, crop: {:?}, dpi: {})",
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

fn determine_page_colorspace(images: &[ImageInfo], with_cmyk: bool) -> ColorMode {
    // Heuristic: fall back to grayscale unless we see any image that clearly carries color data.
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

    // This is how we do it, depending on the "direction" we would select either x-dpi or y-dpi
    // If the image is smask/mask, we ignore.
    // If the image is image/stencil, we then calculate it.
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

        // if dpi is larger than target_dpi, clamp max to there
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

fn describe_colorspace(space: &PdfImageColorSpace) -> String {
    match space {
        PdfImageColorSpace::Unknown => "None".to_string(),
        PdfImageColorSpace::DeviceGray => "DeviceGray".to_string(),
        PdfImageColorSpace::DeviceRGB => "DeviceRGB".to_string(),
        PdfImageColorSpace::DeviceCMYK => "DeviceCMYK".to_string(),
        PdfImageColorSpace::Lab { white, black, a, b } => {
            let white_box = format!("White: {:.4},{:.4},{:.4}", white.x, white.y, white.z);
            let black_box = format!("Black: {:.4},{:.4},{:.4}", black.x, black.y, black.z);
            let a_range = format!("A: {:.4} - {:.4}", a.min, a.max);
            let b_range = format!("B: {:.4} - {:.4}", b.min, b.max);

            format!("Lab[{white_box} | {black_box} | {a_range} | {b_range}]")
        }
        PdfImageColorSpace::ICC { alternate } => {
            format!("ICC({})", describe_colorspace(alternate))
        }
        PdfImageColorSpace::Indexed { hival, base } => {
            format!("Indexed({hival}, {})", describe_colorspace(base))
        }
        PdfImageColorSpace::Pattern => "Pattern".to_string(),
        PdfImageColorSpace::Separation { name, alternate } => {
            format!("Separation({name}, {})", describe_colorspace(alternate))
        }
        PdfImageColorSpace::DeviceN {
            count,
            names,
            alternate,
        } => {
            let all_names = names.join(",");
            format!(
                "DeviceN({count}, [{}], {})",
                all_names,
                describe_colorspace(alternate)
            )
        }
    }
}

fn describe_image_type(kind: ImageType) -> &'static str {
    match kind {
        ImageType::Unknown => "unknown",
        ImageType::Stencil => "stencil",
        ImageType::SoftMask => "smask",
        ImageType::Mask => "mask",
        ImageType::Image => "image",
    }
}

fn extension_for_mode(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Cmyk8 | ColorMode::DeviceN8 => "jpg",
        _ => "png",
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
            CropChoice::CropBox => PdfCropMode::CropBox,
            CropChoice::MediaBox => PdfCropMode::MediaBox,
            // CropChoice::BleedBox => PdfCropMode::BleedBox,
            // CropChoice::TrimBox => PdfCropMode::TrimBox,
            // CropChoice::ArtBox => PdfCropMode::ArtBox,
        }
    }
}

fn fmt_dpi(num: f64) -> String {
    if num.is_nan() {
        return "NaN".to_string();
    }
    if num.is_infinite() {
        return if num.is_sign_positive() {
            "inf"
        } else {
            "-inf"
        }
        .to_string();
    }

    // Handle 0.0 and -0.0 explicitly to avoid "0.000"
    if num == 0.0 {
        return "0".to_string();
    }

    if num.abs() < 1.0 {
        format!("{:.3}", num)
    } else {
        let mut formatted_str = format!("{:.1}", num);

        // Refinement: "or just XXXX"
        // If the formatted string ends with ".0", remove it.
        if formatted_str.ends_with(".0") {
            formatted_str.truncate(formatted_str.len() - 2);
        }
        formatted_str
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
    println!(
        "Spawning {worker_count} worker(s) ({} logical CPUs detected, requested: {requested_label})...",
        cpu_count,
    );

    let (sender, receiver) = unbounded::<PagePlan>();
    let mut handles = Vec::with_capacity(worker_count);

    for worker_index in 0..worker_count {
        let rx = receiver.clone();
        let factory = factory.clone();
        handles.push(thread::spawn(move || -> Result<(), String> {
            let mut document = factory
                .open()
                .map_err(|err| format!("worker {} failed to open PDF: {err}", worker_index + 1))?;
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
    println!(
        "Exported page {} -> {} ({:?}, {:?}, {} dpi)",
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
