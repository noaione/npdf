use std::path::PathBuf;

use clap::{Args, ValueEnum};
use color_print::{cformat, cprintln};
use qpdf::{
    ObjectStreamMode, QPdf, QPdfArray, QPdfDictionary, QPdfObject, QPdfObjectLike, QPdfObjectType,
    QPdfScalar,
};
use tiny_poppler::PdfPasswords;

use crate::common::{ensure_pdf_output, open_maybe_locked};

#[derive(Args)]
pub struct RecropArgs {
    /// Path to the PDF file to unwatermark.
    pub pdf: PathBuf,
    /// Output file path to save the unwatermarked PDF.
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    #[clap(short = 'b', long, value_enum)]
    /// Choose which box to use for recropping pages.
    pub bounding: CropChoice,
    #[arg(short = 'i', long, default_value_t = false)]
    pub describe: bool,
}

pub fn run(args: RecropArgs, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    let output = match (args.output, args.describe) {
        (Some(path), _) => {
            ensure_pdf_output(&path)?;
            Some(path)
        }
        (None, true) => None,
        (None, false) => return Err("--output is required when not using --describe".into()),
    };

    cprintln!("<m,s>Loading PDF</>: {}", args.pdf.display());
    // read PDF to bytes first to avoid multiple opens
    let pdf_bytes = std::fs::read(&args.pdf)
        .map_err(|err| format!("Failed to read PDF file {}: {}", args.pdf.display(), err))?;

    let doc = open_maybe_locked(&pdf_bytes, passwords)?;
    let total_pages = doc
        .get_num_pages()
        .map_err(|err| format!("Failed to get number of pages: {}", err))?;

    cprintln!("<m,s>Processing pages</>...");

    for pg_num in 0..total_pages {
        let page = doc
            .get_page(pg_num)
            .ok_or(format!("Page {} not found in document.", pg_num + 1))?;

        let page_box = get_page_box(&page)?;

        if args.describe {
            cprintln!(
                " > <bold>Page <cyan>{page}</cyan></bold> ┆ {crop} ┆ {media} ┆ {art} ┆ {bleed} ┆ {trim} ┆ {orig}",
                page = pg_num,
                crop = print_box("CropBox", page_box.cropbox),
                media = print_box("MediaBox", page_box.mediabox),
                art = print_box("ArtBox", page_box.artbox),
                bleed = print_box("BleedBox", page_box.bleedbox),
                trim = print_box("TrimBox", page_box.trimbox),
                orig = print_box("OriginalCropBox", page_box.old_cropbox),
            );
            continue;
        }

        match page_box.get_box(args.bounding) {
            Some(new_box) => {
                set_new_crop_box(&doc, page, new_box, &page_box).map_err(|err| {
                    format!(
                        "Failed to set new crop box for page {}: {}",
                        pg_num + 1,
                        err
                    )
                })?;
                cprintln!("<green>Recropped page</> {} to {:?}", pg_num, new_box);
            }
            None => {
                cprintln!(
                    "<yellow>Warning:</> Page {} does not have the specified box {:?}. Skipping.",
                    pg_num,
                    args.bounding
                );
            }
        }
    }

    if let Some(output_path) = output {
        cprintln!(
            "<magenta,bold>Saving output PDF</>: {}",
            output_path.display()
        );

        let pdf_version = doc.get_pdf_version();

        doc.writer()
            .static_id(false)
            .force_pdf_version(&pdf_version)
            .normalize_content(true)
            .preserve_unreferenced_objects(false)
            .compress_streams(true)
            .object_stream_mode(ObjectStreamMode::Preserve)
            .write(output_path)
            .map_err(|err| format!("Failed to save output PDF: {}", err))?;
    }

    Ok(())
}

fn get_page_box(page: &QPdfDictionary) -> Result<PageBox, String> {
    let mut page_box = PageBox {
        cropbox: None,
        mediabox: None,
        artbox: None,
        bleedbox: None,
        trimbox: None,
        old_cropbox: None,
    };

    if let Some(cropbox) = page.get("/CropBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && cropbox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(cropbox.get(0).ok_or("/CropBox missing first element")?)?,
            object_to_f64(cropbox.get(1).ok_or("/CropBox missing second element")?)?,
            object_to_f64(cropbox.get(2).ok_or("/CropBox missing third element")?)?,
            object_to_f64(cropbox.get(3).ok_or("/CropBox missing fourth element")?)?,
        ];

        page_box.cropbox = Some(vals);
    };

    if let Some(old_cropbox) = page.get("/OriginalCropBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && old_cropbox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(
                old_cropbox
                    .get(0)
                    .ok_or("/OriginalCropBox missing first element")?,
            )?,
            object_to_f64(
                old_cropbox
                    .get(1)
                    .ok_or("/OriginalCropBox missing second element")?,
            )?,
            object_to_f64(
                old_cropbox
                    .get(2)
                    .ok_or("/OriginalCropBox missing third element")?,
            )?,
            object_to_f64(
                old_cropbox
                    .get(3)
                    .ok_or("/OriginalCropBox missing fourth element")?,
            )?,
        ];

        page_box.old_cropbox = Some(vals);
    };

    if let Some(mediabox) = page.get("/MediaBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && mediabox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(mediabox.get(0).ok_or("/MediaBox missing first element")?)?,
            object_to_f64(mediabox.get(1).ok_or("/MediaBox missing second element")?)?,
            object_to_f64(mediabox.get(2).ok_or("/MediaBox missing third element")?)?,
            object_to_f64(mediabox.get(3).ok_or("/MediaBox missing fourth element")?)?,
        ];

        page_box.mediabox = Some(vals);
    };

    if let Some(artbox) = page.get("/ArtBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && artbox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(artbox.get(0).ok_or("/ArtBox missing first element")?)?,
            object_to_f64(artbox.get(1).ok_or("/ArtBox missing second element")?)?,
            object_to_f64(artbox.get(2).ok_or("/ArtBox missing third element")?)?,
            object_to_f64(artbox.get(3).ok_or("/ArtBox missing fourth element")?)?,
        ];

        page_box.artbox = Some(vals);
    };

    if let Some(bleedbox) = page.get("/BleedBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && bleedbox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(bleedbox.get(0).ok_or("/BleedBox missing first element")?)?,
            object_to_f64(bleedbox.get(1).ok_or("/BleedBox missing second element")?)?,
            object_to_f64(bleedbox.get(2).ok_or("/BleedBox missing third element")?)?,
            object_to_f64(bleedbox.get(3).ok_or("/BleedBox missing fourth element")?)?,
        ];

        page_box.bleedbox = Some(vals);
    };

    if let Some(trimbox) = page.get("/TrimBox").and_then(|x| {
        if x.get_type() == QPdfObjectType::Array {
            Some(QPdfArray::from(x))
        } else {
            None
        }
    }) && trimbox.len() == 4
    {
        let vals: [f64; 4] = [
            object_to_f64(trimbox.get(0).ok_or("/TrimBox missing first element")?)?,
            object_to_f64(trimbox.get(1).ok_or("/TrimBox missing second element")?)?,
            object_to_f64(trimbox.get(2).ok_or("/TrimBox missing third element")?)?,
            object_to_f64(trimbox.get(3).ok_or("/TrimBox missing fourth element")?)?,
        ];

        page_box.trimbox = Some(vals);
    };

    Ok(page_box)
}

fn object_to_f64(obj: QPdfObject) -> Result<f64, String> {
    match obj.get_type() {
        QPdfObjectType::Integer => {
            let int_val = QPdfScalar::from(obj).as_i64();
            Ok(int_val as f64)
        }
        QPdfObjectType::Real => {
            let real_val = QPdfScalar::from(obj).as_f64();
            Ok(real_val)
        }
        _ => Err(format!(
            "Expected numeric object (Integer or Real), found {:?}",
            obj.get_type()
        )),
    }
}

fn set_new_crop_box(
    doc: &QPdf,
    page: QPdfDictionary,
    new_box: [f64; 4],
    pagebox: &PageBox,
) -> Result<(), String> {
    let new_crop = doc.new_array_from([
        f64_to_real(doc, new_box[0]),
        f64_to_real(doc, new_box[1]),
        f64_to_real(doc, new_box[2]),
        f64_to_real(doc, new_box[3]),
    ]);

    page.set("/CropBox", new_crop);

    if pagebox.old_cropbox.is_none()
        && let Some(cropbox) = pagebox.cropbox
    {
        let original_crop = doc.new_array_from([
            f64_to_real(doc, cropbox[0]),
            f64_to_real(doc, cropbox[1]),
            f64_to_real(doc, cropbox[2]),
            f64_to_real(doc, cropbox[3]),
        ]);
        page.set("/OriginalCropBox", original_crop);
    }

    Ok(())
}

fn f64_to_real(doc: &QPdf, value: f64) -> QPdfObject {
    let dec_places = value.fract().abs().log10().ceil() as u32;
    doc.new_real(value, dec_places).into()
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CropChoice {
    Crop,
    Media,
    Art,
    Bleed,
    Trim,
}

struct PageBox {
    cropbox: Option<[f64; 4]>,
    mediabox: Option<[f64; 4]>,
    artbox: Option<[f64; 4]>,
    bleedbox: Option<[f64; 4]>,
    trimbox: Option<[f64; 4]>,
    old_cropbox: Option<[f64; 4]>,
}

impl PageBox {
    fn get_box(&self, choice: CropChoice) -> Option<[f64; 4]> {
        match choice {
            // Always use old cropbox for recropping to CropBox
            CropChoice::Crop => self.old_cropbox,
            CropChoice::Media => self.mediabox,
            CropChoice::Art => self.artbox,
            CropChoice::Bleed => self.bleedbox,
            CropChoice::Trim => self.trimbox,
        }
    }
}

fn print_box(box_name: &str, box_values: Option<[f64; 4]>) -> String {
    if let Some(values) = box_values {
        cformat!(
            "<blue>{}</>: [{:.2}, {:.2}, {:.2}, {:.2}]",
            box_name,
            values[0],
            values[1],
            values[2],
            values[3]
        )
    } else {
        cformat!("<blue>{}</>: <yellow>Not Set</>", box_name)
    }
}
