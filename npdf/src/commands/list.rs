use clap::Args;
use color_eyre::Result;
use color_print::{cformat, cprintln};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tiny_poppler::{Document, ImageInfo, ImageType, PdfImageColorSpace, PdfPasswords};

use crate::common::NpdfError;

#[derive(Args, Debug, PartialEq)]
pub struct ListArgs {
    /// Path to the PDF file to inspect.
    pub pdf: PathBuf,
    /// Dump page info too
    #[clap(short, long)]
    pub page_info: bool,
}

pub fn run(args: ListArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if !args.pdf.exists() {
        return Err(NpdfError::MissingPdfFile(args.pdf.display().to_string()).into());
    }

    cprintln!("Loading PDF: <m,s>{:#?}</m,s>", &args.pdf);
    let mut document = Document::open_with_passwords(&args.pdf, passwords)?;
    let page_count = document.page_count()?;
    let images = document.images()?;
    let pages_infos = document.page_info()?;

    cprintln!("<magenta,bold>PDF</>: {}", args.pdf.display());
    cprintln!("<magenta,bold>Pages</>: {page_count}");

    if images.is_empty() {
        println!("No embedded images found.");
        return Ok(());
    }

    cprintln!("<magenta,bold>Images</>:\n");

    let mut pages: BTreeMap<u32, Vec<&ImageInfo>> = BTreeMap::new();
    for info in &images {
        pages.entry(info.page).or_default().push(info);
    }

    let header = format!(
        "  {page:>6} ┆ {idx:>4} ┆ {kind:<8} ┆ {size:<15} ┆ {comp:<11} ┆ {color:<20} ┆ {xref:<12} ┆ {dpi:<17}",
        page = "Page",
        idx = "#",
        kind = "Type",
        size = "Size (W×H)",
        comp = "Comp/BPC",
        color = "Colorspace",
        xref = "XRef",
        dpi = "DPI (X × Y)",
    );
    println!("{header}");
    println!("  {}", "-".repeat(header.len().saturating_sub(2)));

    for page in 1..=page_count {
        if let Some(entries) = pages.get(&page) {
            for (idx, &info) in entries.iter().enumerate() {
                let kind = describe_image_type(info.image_type);
                let size = format!("{}×{}", info.width, info.height);
                let comp = format!("{}c/{}bpc", info.components, info.bits_per_component);
                let color = describe_colorspace(&info.colorspace);
                let xref = match info.xref {
                    Some((obj, generation)) => format!("{} {} R", obj, generation),
                    None => "inline".into(),
                };
                let dpi = format!("{} × {}", fmt_dpi(info.dpi.0), fmt_dpi(info.dpi.1));
                let page_cell = format!("{:>6}", page);

                if idx == 0 {
                    cprintln!(
                        "  <bold><cyan>{page}</cyan></bold> ┆ {idx:>4} ┆ {kind:<8} ┆ {size:<15} ┆ {comp:<11} ┆ {color:<20} ┆ {xref:<12} ┆ {dpi:<17}",
                        page = page_cell,
                        idx = idx + 1,
                        kind = kind,
                        size = size,
                        comp = comp,
                        color = color,
                        xref = xref,
                        dpi = dpi,
                    );
                } else {
                    println!(
                        "  {page:>6} ┆ {idx:>4} ┆ {kind:<8} ┆ {size:<15} ┆ {comp:<11} ┆ {color:<20} ┆ {xref:<12} ┆ {dpi:<17}",
                        page = page_cell,
                        idx = idx + 1,
                        kind = kind,
                        size = size,
                        comp = comp,
                        color = color,
                        xref = xref,
                        dpi = dpi,
                    );
                }
            }
        } else {
            let page_cell = format!("{:>6}", page);
            cprintln!(
                "  <bold><cyan>{page}</cyan></bold> ┆ <magenta>{note}</magenta>",
                page = page_cell,
                note = "(no embedded images)"
            );
        }
    }

    // Dump page info
    if args.page_info {
        cprintln!("\n<magenta,bold>Page Info</>:\n");

        let (max_mediabox_len, max_cropbox_len) = test_longest_rectbox(&pages_infos);

        let header = format!(
            "  {page:>6} ┆ {idx:>4} ┆ {img_count:>7} ┆ {obj_count:>8} ┆ {mediabox}{mb_space} ┆ {cropbox}{cb_space} ┆ {color:<20}",
            page = "Page",
            idx = "#",
            img_count = "Image #",
            obj_count = "Object #",
            mediabox = "MediaBox",
            mb_space = " ".repeat(max_mediabox_len.saturating_sub(8) as usize),
            cb_space = " ".repeat(max_cropbox_len.saturating_sub(7) as usize),
            cropbox = "CropBox",
            color = "Colorspace",
        );
        println!("{header}");
        println!("  {}", "-".repeat(header.len().saturating_sub(2)));

        for page_info in &pages_infos {
            // Since colorspace can be multiple entries, we just need to repeat the row for each colorspace
            // If no colorspace, just print one row with "None"
            let mb_info = describe_rectbox(&page_info.mediabox);
            let cb_info = describe_rectbox(&page_info.cropbox);
            let mb_space =
                " ".repeat(max_mediabox_len.saturating_sub(mb_info.len() as u32) as usize);
            let cb_space =
                " ".repeat(max_cropbox_len.saturating_sub(cb_info.len() as u32) as usize);

            if page_info.colorspaces.is_empty() {
                cprintln!(
                    "  <bold><cyan>{page}</cyan></bold> ┆ {idx:>4} ┆ {img_count:>7} ┆ {obj_count:>8} ┆ {mediabox}{mb_space} ┆ {cropbox}{cb_space} ┆ {color:<20}",
                    page = format!("{:>6}", page_info.page),
                    idx = 1,
                    img_count = page_info.image_count,
                    obj_count = page_info.object_count,
                    mediabox = &mb_info,
                    mb_space = &mb_space,
                    cropbox = &cb_info,
                    cb_space = &cb_space,
                    color = "None",
                );
            } else {
                for (idx, (name, space)) in page_info.colorspaces.iter().enumerate() {
                    let color = cformat!("<y,s>{}</y,s>: {}", name, describe_colorspace(space));
                    if idx == 0 {
                        cprintln!(
                            "  <bold><cyan>{page}</cyan></bold> ┆ {idx:>4} ┆ {img_count:>7} ┆ {obj_count:>8} ┆ {mediabox}{mb_space} ┆ {cropbox}{cb_space} ┆ {color:<20}",
                            page = format!("{:>6}", page_info.page),
                            idx = idx + 1,
                            img_count = page_info.image_count,
                            obj_count = page_info.object_count,
                            mediabox = &mb_info,
                            mb_space = &mb_space,
                            cropbox = &cb_info,
                            cb_space = &cb_space,
                            color = color,
                        );
                    } else {
                        println!(
                            "  {page:>6} ┆ {idx:>4} ┆ {color:<20}",
                            page = format!("{:>6}", page_info.page),
                            idx = idx + 1,
                            color = color,
                        );
                    }
                }
            }
        }
    }

    cprintln!("\nTotal embedded images: <bold>{}</bold>", images.len());
    cprintln!(
        "<magenta,bold>Hint:</magenta,bold> a <cyan,bold>highlighted</cyan,bold> page number indicates new page."
    );

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
            let a_range = format!("A: {:.4}–{:.4}", a.min, a.max);
            let b_range = format!("B: {:.4}–{:.4}", b.min, b.max);

            format!("Lab[{white_box} / {black_box} / {a_range} / {b_range}]")
        }
        PdfImageColorSpace::ICC { alternate } => {
            format!("ICC({})", describe_colorspace(alternate))
        }
        PdfImageColorSpace::Indexed {
            hival,
            base,
            is_achromatic,
        } => {
            let suffix = if *is_achromatic { ", achromatic" } else { "" };
            format!("Indexed({hival}, {}{suffix})", describe_colorspace(base))
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

fn describe_rectbox(box_opt: &Option<tiny_poppler::PdfRect>) -> String {
    match box_opt {
        Some(rect) => format!(
            "[{:.2}, {:.2}, {:.2}, {:.2}]",
            rect.x1, rect.x2, rect.y1, rect.y2
        ),
        None => "None".to_string(),
    }
}

fn test_longest_rectbox(boxes: &[tiny_poppler::PageInfo]) -> (u32, u32) {
    let mut max_mediabox = 20u32;
    let mut max_cropbox = 20u32;

    for page in boxes {
        let mb_desc = describe_rectbox(&page.mediabox);
        if mb_desc.len() as u32 > max_mediabox {
            max_mediabox = mb_desc.len() as u32;
        }
        let cb_desc = describe_rectbox(&page.cropbox);
        if cb_desc.len() as u32 > max_cropbox {
            max_cropbox = cb_desc.len() as u32;
        }
    }

    (max_mediabox, max_cropbox)
}
