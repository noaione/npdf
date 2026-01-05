use clap::{Args, ValueEnum};
use color_eyre::Result;
use color_eyre::eyre::Context;
use color_print::cprintln;
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
use crate::commands::export::render::ZeroWidthLineChoice;
use crate::commands::export::render::{AutoDPIDirection, ColorChoice, CropChoice, RenderPagePlan};
use crate::common::NpdfError;

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
    /// Zero-width line rendering mode.
    #[arg(short = 'z', long, value_enum, default_value_t = ZeroWidthLineChoice::Default)]
    pub zero_width_line: ZeroWidthLineChoice,
}

pub fn run(args: ExportArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if args.dpi <= 0.0 {
        return Err(NpdfError::InvalidDpi(args.dpi).into());
    }

    if !args.pdf.exists() {
        return Err(NpdfError::MissingPdfFile(args.pdf.display().to_string()).into());
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
        (None, false) => {
            return Err(NpdfError::RequireArgumentWhen("--output", "--describe").into());
        }
    };

    cprintln!("Loading PDF: <m,s>{:#?}</m,s>", &args.pdf);
    let factory = DocumentFactory::with_images_with_passwords(&args.pdf, passwords.cloned())?;
    let page_count = factory.page_count();

    let first_page = first.unwrap_or(1);
    if first_page == 0 || first_page > page_count {
        return Err(NpdfError::MustBetween("first page", 1, page_count as usize).into());
    }

    let last_page = last.unwrap_or(page_count);
    if last_page == 0 || last_page > page_count {
        return Err(NpdfError::MustBetween("last page", 1, page_count as usize).into());
    }
    if last_page < first_page {
        return Err(
            NpdfError::MustBetween("last page", first_page as usize, page_count as usize).into(),
        );
    }

    if let Some(output_path) = output
        && let Err(err) = fs::create_dir_all(output_path)
    {
        return Err(NpdfError::CreateOutputDirError(err).into());
    }

    println!("Preloading images...");
    let images_metadata = factory
        .images()
        .map(|slice| slice.to_vec())
        .unwrap_or_default();

    let page_stats = if let Some(pages) = factory.pages() {
        pages.to_vec()
    } else {
        let mut document = factory.open()?;
        document.page_info()?
    };

    let page_pairings = pair_page_images(&page_stats, &images_metadata);

    println!("Starting export...");
    let pages_ranges = 1..=page_count;
    let pages: Vec<u32> = if reverse {
        pages_ranges
            .rev()
            .skip((first_page - 1) as usize)
            .take((last_page - first_page + 1) as usize)
            .collect()
    } else {
        pages_ranges
            .skip((first_page - 1) as usize)
            .take((last_page - first_page + 1) as usize)
            .collect()
    };

    let mut jobs: Vec<PagePlanJob> = Vec::new();
    for (pg_idx, page) in pages.iter().enumerate() {
        if let Some((page_info, images)) = page_pairings.get_key_value(page) {
            let should_extract = should_be_extracted(page_info, images, extract);
            let output_page = (pg_idx as u32) + first_page;

            if should_extract {
                extract::prepare_job(*page, output_page, images, &args, output, &mut |job_plan| {
                    jobs.push(PagePlanJob::Extract(job_plan))
                });
            } else {
                render::prepare_job(
                    output_page,
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
        let mut document = factory.open()?;
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
) -> Result<()> {
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
        handles.push(thread::spawn(move || -> Result<()> {
            let mut document = factory.open().context(format!("worker {}", worker_index))?;
            while let Ok(job) = rx.recv() {
                let pg_num = job.page_num();
                process_job(&mut document, job)
                    .context(format!("page {}, worker {}", pg_num, worker_index))?;
            }
            Ok(())
        }));
    }

    drop(receiver);

    for job in jobs {
        sender.send(job)?;
    }
    drop(sender);

    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(color_eyre::eyre::eyre!(
                    "A worker thread panicked during exporting job."
                ));
            }
        }
    }

    Ok(())
}

fn process_job(document: &mut Document, job: PagePlanJob) -> Result<()> {
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
            paired.insert(page.clone(), images.clone());
        } else {
            paired.insert(page.clone(), Vec::new());
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
                page.is_pdf_a_compatible && images.len() == 1
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
    fn page_num(&self) -> u32 {
        match self {
            PagePlanJob::Render(render_job) => render_job.page_number,
            PagePlanJob::Extract(extract_job) => extract_job.page,
        }
    }

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
