use clap::Args;
use std::path::PathBuf;
use tiny_poppler::{Document, ImageType, PdfImageColorSpace};

#[derive(Args)]
pub struct ListArgs {
    /// Path to the PDF file to inspect.
    pub pdf: PathBuf,
}

pub fn run(args: ListArgs) -> Result<(), String> {
    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

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

    if num == 0.0 {
        return "0".to_string();
    }

    if num.abs() < 1.0 {
        format!("{:.3}", num)
    } else {
        let mut formatted_str = format!("{:.1}", num);
        if formatted_str.ends_with(".0") {
            formatted_str.truncate(formatted_str.len() - 2);
        }
        formatted_str
    }
}
