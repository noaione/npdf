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
use tiny_poppler::{Document, DocumentFactory, ImageInfo, ImageType, PageInfo, PdfPasswords};

mod extract;
mod render;

use crate::commands::export::extract::{ExtractPagePlan, PrerenderPlan, describe_component};
use crate::commands::export::render::{AutoDPIDirection, ColorChoice, CropChoice, RenderPagePlan};
use crate::commands::export::render::{GlyphFillChoice, ZeroWidthLineChoice};
use crate::common::NpdfError;

pub type PagePairings = HashMap<PageInfo, Vec<ImageInfo>>;

/// `--min-image-coverage` default, in percent, matching the sandbox prototype.
const DEFAULT_MIN_IMAGE_COVERAGE: f64 = 15.0;

/// How much of the page a stored image must cover before its bytes can stand in for the
/// whole page. Slightly under 1.0 to allow for bleed past the crop box.
const PRERENDERED_MIN_COVERAGE: f64 = 0.9;

#[derive(Args, Debug, PartialEq)]
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
    /// Ignore images covering less than this percentage of the visible page when
    /// choosing an export DPI.
    ///
    /// A page is set by its dominant imagery, so a small low-resolution ornament
    /// should not drag the whole page down. Set to 0 to weigh every image.
    #[arg(long, default_value_t = DEFAULT_MIN_IMAGE_COVERAGE)]
    pub min_image_coverage: f64,
    /// Ignore images narrower or shorter than this many pixels when choosing an
    /// export DPI.
    ///
    /// Below 11 px both axes must fall short, since a small legitimate mark is
    /// likelier than a thumbnail.
    #[arg(long)]
    pub min_image_dim: Option<u32>,
    /// Lower DPI ceiling for colour pages that have no notable images, or several.
    ///
    /// Such a page has no single piece of imagery to resolve for, so it does not need
    /// the resolution a full-page image calls for.
    #[arg(long)]
    pub color_export_dpi: Option<f64>,
    /// Let a lowered DPI stand on pages that carry text.
    ///
    /// Text is rasterized at the page DPI too, so a page whose imagery lowered the
    /// resolution would render its text soft. By default such a page is put back to
    /// `--dpi` (grayscale) or `--color-export-dpi` (colour); pass this to keep the
    /// imagery's verdict instead.
    #[arg(long, default_value_t = false)]
    pub skip_text_dpi_floor: bool,
    /// Final output DPI. Pages chosen at a higher DPI are rendered at that resolution
    /// and then averaged down.
    ///
    /// Supersampling this way antialiases text and line art better than rasterizing
    /// straight at the lower DPI. Ignored for 1-bit output, which is rendered at this
    /// resolution directly.
    #[arg(long)]
    pub output_dpi: Option<f64>,
    /// When in Auto color mode, if we encounter RGB colorspace,
    /// do additional check whether there is CMYK content (or color with CMYK fallback).
    #[arg(long, default_value_t = false)]
    pub with_cmyk: bool,
    /// Skip the ML colour-page check during auto colour detection.
    ///
    /// For pages whose colourspaces say colour, the check renders a small preview and
    /// asks a manga-trained classifier whether the page is visually black and white,
    /// which catches RGB scans of grey originals. It needs a build with the model
    /// weights embedded (`OGSOX_WEIGHTS_PATH` at build time) and silently does nothing
    /// without them.
    #[arg(long, default_value_t = false)]
    pub skip_ml_color_check: bool,
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
    /// Skip the pre-rendered page check.
    ///
    /// A page that is nothing but one stored image covering it is copied out as-is
    /// instead of being re-rendered, which avoids re-encoding the scan a second time.
    /// Pass this to always render.
    #[arg(long, default_value_t = false)]
    pub skip_prerendered_check: bool,
    /// Export CCITT as TIFF images where applicable.
    #[arg(long, default_value_t = false)]
    pub ccitt_as_tiff: bool,
    /// Zero-width line rendering mode.
    #[arg(short = 'z', long, value_enum, default_value_t = ZeroWidthLineChoice::Default)]
    pub zero_width_line: ZeroWidthLineChoice,
    /// Glyph fill rendering mode. `path` works around fonts with self-intersecting glyph
    /// contours that FreeType mis-renders, at the cost of coarser text antialiasing.
    #[arg(long, value_enum, default_value_t = GlyphFillChoice::Bitmap)]
    pub glyph_fill: GlyphFillChoice,
    /// Minimum stroke width in device pixels (0 disables). Widens strokes thinner than this
    /// after DPI scaling, which avoids sub-pixel gaps at sharp corners of very thin strokes
    /// (e.g. decorative fonts drawn in outline/stroke text mode).
    #[arg(long, default_value_t = 0.0)]
    pub min_line_width: f64,
}

pub fn run(args: ExportArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if args.dpi <= 0.0 {
        return Err(NpdfError::InvalidDpi(args.dpi).into());
    }

    if args.min_line_width < 0.0 {
        return Err(NpdfError::InvalidMinLineWidth(args.min_line_width).into());
    }

    if args.color == ColorChoice::Auto
        && !args.skip_ml_color_check
        && !mangacolor::ogsox_available()
    {
        cprintln!(
            "<yellow>Note</>: this build carries no ML colour weights, so pages are classified from their colourspaces alone."
        );
    }

    if !(0.0..=100.0).contains(&args.min_image_coverage) {
        return Err(NpdfError::InvalidMinImageCoverage(args.min_image_coverage).into());
    }

    if let Some(color_dpi) = args.color_export_dpi
        && color_dpi <= 0.0
    {
        return Err(NpdfError::InvalidColorExportDpi(color_dpi).into());
    }

    if let Some(output_dpi) = args.output_dpi {
        if output_dpi <= 0.0 {
            return Err(NpdfError::InvalidDpi(output_dpi).into());
        }
        if output_dpi > args.dpi {
            return Err(NpdfError::OutputDpiTooHigh(output_dpi, args.dpi).into());
        }
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
            let output_page = (pg_idx as u32) + first_page;

            // A page that is just a stored scan is copied out rather than re-rendered
            // and re-encoded, which is both faster and lossless.
            if extract == ExtractionMode::None
                && let Some(plan) =
                    prerendered_page_job(page_info, images, output_page, &args, output)
            {
                jobs.push(PagePlanJob::Prerender(plan));
                continue;
            }

            let should_extract = should_be_extracted(page_info, images, extract);

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
        PagePlanJob::Prerender(prerender_job) => {
            crate::commands::export::extract::process_prerender_job(document, &prerender_job)
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

/// Whether the page's own shape allows passthrough at all.
///
/// No text, and nothing drawn on the page besides its images: a fill, a stroke, a clip or
/// a form wrapper all bump `object_count` past the image count, and any of them means the
/// stored image is no longer the whole page.
fn page_allows_passthrough(page_info: &PageInfo) -> bool {
    !page_info.has_text && page_info.object_count <= u64::from(page_info.image_count)
}

/// Builds a passthrough job for a page that is nothing but one stored image.
fn prerendered_page_job(
    page_info: &PageInfo,
    images: &[ImageInfo],
    output_page: u32,
    args: &ExportArgs,
    output: Option<&PathBuf>,
) -> Option<PrerenderPlan> {
    if args.skip_prerendered_check || !page_allows_passthrough(page_info) {
        return None;
    }

    let plain: Vec<&ImageInfo> = images
        .iter()
        .filter(|img| matches!(img.image_type, ImageType::Image))
        .collect();
    let only_image = plain.first()?;
    if plain.len() != 1
        || render::page_coverage(Some(page_info), args.bounding, only_image.matrix)
            < PRERENDERED_MIN_COVERAGE
    {
        return None;
    }

    // A copy cannot be resized, so if `--output-dpi` would have to shrink the stored
    // image, render instead and let the downscale happen.
    if let Some(target) = args.output_dpi
        && only_image.dpi.0.max(only_image.dpi.1) > target
    {
        return None;
    }

    extract::prerender_plan(output_page, images, args, output)
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
    Prerender(PrerenderPlan),
}

impl PagePlanJob {
    fn page_num(&self) -> u32 {
        match self {
            PagePlanJob::Render(render_job) => render_job.page_number,
            PagePlanJob::Extract(extract_job) => extract_job.page,
            PagePlanJob::Prerender(prerender_job) => prerender_job.page,
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
            PagePlanJob::Prerender(prerender_job) => {
                let entry = prerender_job.entry();
                let path_display = prerender_job
                    .output_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<none>".to_string());
                cprintln!(
                    "Will copy the stored image of page <m,s>{}</m,s> to <m,s>{}</m,s> (pre-rendered page, not re-encoded)",
                    prerender_job.page,
                    path_display,
                );
                if let Ok(exported) = extract::export_image_entry(document, entry, true) {
                    describe_component(prerender_job.page, None, "", entry.info(), &exported);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn page_info(object_count: u64, image_count: u32, has_text: bool) -> PageInfo {
        PageInfo {
            page: 1,
            image_count,
            object_count,
            is_pdf_a_compatible: false,
            has_text,
            mediabox: None,
            cropbox: None,
            colorspaces: HashMap::new(),
        }
    }

    #[test]
    fn a_page_that_is_only_images_may_pass_through() {
        assert!(page_allows_passthrough(&page_info(1, 1, false)));
    }

    #[test]
    fn text_disqualifies_a_page_from_passing_through() {
        assert!(!page_allows_passthrough(&page_info(1, 1, true)));
    }

    #[test]
    fn anything_drawn_besides_the_images_disqualifies_it() {
        // A fill, a stroke, a clip or a form wrapper all bump object_count past the
        // number of images drawn.
        assert!(!page_allows_passthrough(&page_info(2, 1, false)));
    }
}
