use clap::{Args, ValueEnum};
use color_print::{cformat, cprintln};
use crossbeam_channel::unbounded;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::thread;
use tiny_poppler::{Document, DocumentFactory, ImageInfo, PageInfo, PdfPasswords};

mod extract;
mod render;

use crate::commands::export::extract::{ExtractPagePlan, describe_component};
use crate::commands::export::render::{AutoDPIDirection, ColorChoice, CropChoice, RenderPagePlan};

pub type PagePairings = HashMap<PageInfo, Vec<ImageInfo>>;

#[derive(Args)]
pub struct ExportArgs {
    /// Path to the PDF file to export.
    pub pdf: PathBuf,
    /// Directory where rendered pages will be written.
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    /// Colorspace to request from Poppler. Auto keeps the library default.
    #[arg(short = 'c', long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
    #[arg(short = 'b', long, value_enum, default_value_t = CropChoice::Crop)]
    pub bounding: CropChoice,
    /// DPI used when rendering the page raster.
    #[arg(short = 'r', long, default_value_t = 150.0)]
    pub dpi: f64,
    /// Auto-DPI based on image characteristics.
    ///
    /// Select the direction constraint for auto-DPI calculation.
    /// - Horizontal: x-dpi
    /// - Vertical: y-dpi (Manga/Comics/etc)
    #[arg(long, value_enum)]
    pub auto_dpi: Option<AutoDPIDirection>,
    /// Width/height difference ratio threshold to consider an image
    /// as predominantly horizontal/vertical for auto-DPI calculation.
    #[arg(long, default_value_t = 1.25)]
    pub auto_dpi_ratio: f64,
    /// When in Auto color mode, if we encounter RGB colorspace,
    /// do additional check whether there is CMYK content (or color with CMYK fallback).
    #[arg(long, default_value_t = false)]
    pub with_cmyk: bool,
    /// Save CMYK images in PNG format.
    ///
    /// Since PNG does not support CMYK, we would convert it to RGB first.
    /// If you use this flag with --color auto, CMYK images will be saved depending
    /// on the detected colorspace.
    ///
    /// This would force the rendering to purely use CMYK colorspace.
    #[arg(long, default_value_t = false)]
    pub cmyk_png: bool,
    /// JPEG quality (1-100) when exporting to JPEG files.
    #[arg(short = 'q', long, default_value_t = 96, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub quality: u8,
    /// First page to export (1-based).
    #[arg(short, long)]
    pub first: Option<u32>,
    /// Last page to export (1-based).
    #[arg(short, long)]
    pub last: Option<u32>,
    /// Reverse the page order during export.
    #[arg(long, default_value_t = false)]
    pub reverse: bool,
    #[arg(short = 'i', long, default_value_t = false)]
    pub describe: bool,
    /// Worker threads to use during export (omit for auto).
    #[arg(short = 't', long, value_parser = clap::value_parser!(NonZeroUsize))]
    pub threads: Option<NonZeroUsize>,
    /// Do extraction when possible instead of rendering.
    #[arg(long, value_enum, default_value_t = ExtractionMode::None)]
    pub extract: ExtractionMode,
    /// Export CCITT as TIFF images where applicable.
    #[arg(long, default_value_t = false)]
    pub ccitt_as_tiff: bool,
}

pub fn run(args: ExportArgs, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    if args.dpi <= 0.0 {
        return Err("dpi must be a positive number".into());
    }

    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    let ExportArgs {
        first,
        last,
        describe,
        reverse,
        threads,
        extract,
        ..
    } = args;

    let output = match (&args.output, describe) {
        (Some(path), _) => Some(path),
        (None, true) => None,
        (None, false) => return Err("--output is required when not using --describe".into()),
    };

    cprintln!("Loading PDF: <m,s>{:#?}</m,s>", &args.pdf);
    let factory = DocumentFactory::with_images_with_passwords(&args.pdf, passwords.cloned())
        .map_err(|err| err.to_string())?;
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

    if let Some(output_path) = output
        && let Err(err) = fs::create_dir_all(output_path)
    {
        return Err(format!("failed to create output directory: {err}"));
    }

    println!("Preloading images...");
    let images_metadata = factory
        .images()
        .map(|slice| slice.to_vec())
        .unwrap_or_default();

    let page_stats = if let Some(pages) = factory.pages() {
        pages.to_vec()
    } else {
        let mut document = factory.open().map_err(|err| err.to_string())?;
        document.page_info().map_err(|err| err.to_string())?
    };

    let page_pairings = pair_page_images(&page_stats, &images_metadata);

    println!("Starting export...");
    let pages: Vec<u32> = if reverse {
        (first_page..=last_page).rev().collect()
    } else {
        (first_page..=last_page).collect()
    };

    let mut jobs: Vec<PagePlanJob> = Vec::new();
    for page in pages {
        if let Some((page_info, images)) = page_pairings.get_key_value(&page) {
            let should_extract = should_be_extracted(page_info, images, extract);

            if should_extract {
                extract::prepare_job(page, images, &args, output, &mut |job_plan| {
                    jobs.push(PagePlanJob::Extract(job_plan))
                });
            } else {
                render::prepare_job(
                    page,
                    page_info,
                    images,
                    &args,
                    output,
                    &factory,
                    &mut |job_plan| jobs.push(PagePlanJob::Render(job_plan)),
                );
            }
        }
    }

    if describe {
        let mut document = factory.open().map_err(|err| err.to_string())?;
        for job in &jobs {
            job.describe(&mut document);
        }
        return Ok(());
    }

    run_export_jobs(factory, jobs, threads.map(NonZeroUsize::get))
}

fn run_export_jobs(
    factory: DocumentFactory,
    jobs: Vec<PagePlanJob>,
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

    let (sender, receiver) = unbounded::<PagePlanJob>();
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
                process_job(&mut document, job)?;
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

fn process_job(document: &mut Document, job: PagePlanJob) -> Result<(), String> {
    match job {
        PagePlanJob::Render(render_job) => {
            crate::commands::export::render::process_job(document, &render_job)
        }
        PagePlanJob::Extract(extract_job) => {
            crate::commands::export::extract::process_job(document, &extract_job)
        }
    }
}

fn pair_page_images(pages: &[PageInfo], images: &[ImageInfo]) -> PagePairings {
    // map images to btreemap first since pages is our source of truth
    let mut image_map: BTreeMap<u32, Vec<ImageInfo>> = BTreeMap::new();
    for image in images {
        image_map.entry(image.page).or_default().push(image.clone());
    }

    let mut paired: PagePairings = PagePairings::new();
    for page in pages {
        if let Some(images) = image_map.get(&page.page) {
            paired.insert(*page, images.clone());
        } else {
            paired.insert(*page, Vec::new());
        }
    }

    paired
}

fn should_be_extracted(page: &PageInfo, images: &[ImageInfo], mode: ExtractionMode) -> bool {
    match mode {
        ExtractionMode::None => {
            // render only
            false
        }
        ExtractionMode::All => {
            // extract only
            true
        }
        ExtractionMode::Some => {
            if images.is_empty() {
                // if no images, render
                false
            } else {
                // if pdf/a compliant, extract
                page.is_pdf_a_compatible
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum ExtractionMode {
    /// Do not do extraction, only rendering.
    ///
    /// This is the default mode.
    None,
    /// Extract images when possible, otherwise render.
    ///
    /// This is useful if the document is PDF/A related.
    Some,
    /// Only extract images, skip rendering.
    ///
    /// This is useful for bulk image extraction from PDF files.
    All,
}

#[derive(Clone)]
enum PagePlanJob {
    Render(RenderPagePlan),
    Extract(ExtractPagePlan),
}

impl PagePlanJob {
    fn describe(&self, document: &mut Document) {
        match self {
            PagePlanJob::Render(render_job) => {
                let path_display = render_job
                    .output_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string());
                cprintln!(
                    "Will export page <m,s>{}</m,s> -> <m,s>{}</m,s> (colorspace: {:?}, crop: {:?}, dpi: {})",
                    render_job.page_number,
                    path_display,
                    render_job.options.color_mode,
                    render_job.options.crop_mode,
                    render_job.options.dpi,
                );
            }
            PagePlanJob::Extract(extract_job) => {
                let extract = extract::export_image_entry(document, &extract_job.entry, true);
                if let Ok(exported) = extract {
                    describe_component(
                        extract_job.page,
                        extract_job.slot,
                        extract_job.component_suffix,
                        extract_job.entry.info(),
                        &exported,
                    );
                } else if let Err(err) = extract {
                    cprintln!(
                        "Failed to prepare extraction for image on page <m,s>{}</m,s>: <m,s>{}</m,s>",
                        extract_job.page,
                        err,
                    );
                }
            }
        }
    }
}
