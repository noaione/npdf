use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::{collections::HashMap, fs};

use tiny_poppler::{
    ColorMode, Document, ImageColorSpace, ImageInfo, ImageType, PdfCropMode, RenderOptions,
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
    /// Export pages from the PDF to PNG files.
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
    /// First page to export (1-based).
    #[arg(long)]
    first: Option<u32>,
    /// Last page to export (1-based).
    #[arg(long)]
    last: Option<u32>,
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
enum CropChoice {
    CropBox,
    MediaBox,
    ArtBox,
    BleedBox,
    TrimBox,
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
        let colorspace = describe_colorspace(info.colorspace);
        let xref = match info.xref {
            Some((obj, generation)) => format!("{} {} R", obj, generation),
            None => "inline".into(),
        };
        let image_type = describe_image_type(info.image_type);
        println!(
            "{position:>4}: page {page:>4}, {image_type}, {width}x{height}px, {components} comps, {bits} bpc, colorspace {colorspace}, xref {xref}",
            page = info.page,
            width = info.width,
            height = info.height,
            components = info.components,
            bits = info.bits_per_component
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
    } = args;

    println!("Loading PDF: {:#?}", &pdf);
    let mut document = Document::open(&pdf).map_err(|err| err.to_string())?;
    let page_count = document.page_count().map_err(|err| err.to_string())?;

    println!("Preloading images count...");
    let images_metadata = document.images().map_err(|err| err.to_string())?;

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

    if let Err(err) = fs::create_dir_all(&output) {
        return Err(format!("failed to create output directory: {err}"));
    }

    let mut options = RenderOptions::default();
    options.dpi = dpi;
    options.crop_mode = crop.to_crop_mode();
    if let Some(mode) = color.to_color_mode() {
        options.color_mode = mode;
    }

    let mut images_mappings: HashMap<u32, Vec<ImageInfo>> = HashMap::new();
    for item in images_metadata {
        images_mappings.entry(item.page).or_default().push(item);
    }

    println!("Starting export...");
    for page in first_page..=last_page {
        if color == ColorChoice::Auto {
            let image_map = images_mappings.get(&page);
            options.color_mode = match image_map {
                Some(img_map) => determine_page_colorspace(img_map),
                None => ColorMode::Mono8,
            };
        };
        let file_name = format!("page-{page:04}.png");
        let output_path = output.join(file_name);
        document
            .render_page_to_png(page - 1, &output_path, &options)
            .map_err(|err| format!("failed to export page {page}: {err}"))?;
        println!("Exported page {page} -> {}", output_path.display());
    }

    Ok(())
}

fn determine_page_colorspace(images: &[ImageInfo]) -> ColorMode {
    // TODO!
    ColorMode::Rgb8
    // let has_color = images
    //     .iter()
    //     .find(|image| {
    //         if
    //     })
}

fn describe_colorspace(space: ImageColorSpace) -> &'static str {
    match space {
        ImageColorSpace::Unknown => "Unknown",
        ImageColorSpace::DeviceGray => "DeviceGray",
        ImageColorSpace::DeviceRgb => "DeviceRGB",
        ImageColorSpace::DeviceCmyk => "DeviceCMYK",
        ImageColorSpace::Lab => "Lab",
        ImageColorSpace::Icc => "ICC",
        ImageColorSpace::Indexed => "Indexed",
        ImageColorSpace::Pattern => "Pattern",
        ImageColorSpace::Separation => "Separation",
        ImageColorSpace::DeviceN => "DeviceN",
        ImageColorSpace::Other => "Other",
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
            CropChoice::BleedBox => PdfCropMode::BleedBox,
            CropChoice::TrimBox => PdfCropMode::TrimBox,
            CropChoice::ArtBox => PdfCropMode::ArtBox,
        }
    }
}
