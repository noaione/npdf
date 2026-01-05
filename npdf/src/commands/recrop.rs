use clap::{Args, ValueEnum};
use color_eyre::{Result, eyre::Context};
use color_print::{cformat, cprintln};
use lopdf::{Document, Object, ObjectId};
use std::path::PathBuf;
use tiny_poppler::PdfPasswords;

use crate::common::{NpdfError, ensure_pdf_output, unlock_pdf};

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

pub fn run(args: RecropArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if !args.pdf.exists() {
        return Err(NpdfError::MissingPdfFile(args.pdf.display().to_string()).into());
    }

    let output = match (args.output, args.describe) {
        (Some(path), _) => {
            ensure_pdf_output(&path)?;
            Some(path)
        }
        (None, true) => None,
        (None, false) => {
            return Err(NpdfError::RequireArgumentWhen("--output", "--describe").into());
        }
    };

    cprintln!("<magenta,bold>Loading PDF</>: {}", args.pdf.display());
    let mut doc = Document::load(args.pdf)?;

    unlock_pdf(&doc, passwords)?;

    cprintln!("<magenta,bold>Processing pages</>...");
    for (page_num, object_id) in doc.get_pages() {
        let page_box = get_page_box(&doc, object_id).context(format!("page {}", page_num))?;

        if args.describe {
            cprintln!(
                " > <bold>Page <cyan>{page}</cyan></bold> ┆ {crop} ┆ {media} ┆ {art} ┆ {bleed} ┆ {trim} ┆ {orig}",
                page = page_num,
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
                set_new_crop_box(&mut doc, object_id, new_box)
                    .context(format!("page {}", page_num))?;
                cprintln!("<green>Recropped page</> {} to {:?}", page_num, new_box);
            }
            None => {
                cprintln!(
                    "<yellow>Warning:</> Page {} does not have the specified box {:?}. Skipping.",
                    page_num,
                    args.bounding
                );
            }
        }
    }

    if args.describe {
        return Ok(());
    }

    let output_path = output.as_ref().ok_or(NpdfError::OutputNotSet)?;

    cprintln!(
        "<magenta,bold>Saving output PDF</>: {}",
        output_path.display()
    );
    doc.save(output_path)?;

    cprintln!("<green,bold>Done!</>");
    Ok(())
}

struct PageBox {
    cropbox: Option<[f32; 4]>,
    mediabox: Option<[f32; 4]>,
    artbox: Option<[f32; 4]>,
    bleedbox: Option<[f32; 4]>,
    trimbox: Option<[f32; 4]>,
    old_cropbox: Option<[f32; 4]>,
}

impl PageBox {
    fn get_box(&self, choice: CropChoice) -> Option<[f32; 4]> {
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

fn get_page_box(doc: &Document, page_id: ObjectId) -> Result<PageBox, lopdf::Error> {
    let page = doc.get_object(page_id).and_then(|o| o.as_dict())?;

    let mut page_box = PageBox {
        cropbox: None,
        mediabox: None,
        artbox: None,
        bleedbox: None,
        trimbox: None,
        old_cropbox: None,
    };

    if let Ok(cropbox) = page.get(b"CropBox").and_then(Object::as_array)
        && cropbox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&cropbox[0])?,
            f32_or_i64(&cropbox[1])?,
            f32_or_i64(&cropbox[2])?,
            f32_or_i64(&cropbox[3])?,
        ];
        page_box.cropbox = Some(vals);
    }
    if let Ok(original_cropbox) = page.get(b"OriginalCropBox").and_then(Object::as_array)
        && original_cropbox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&original_cropbox[0])?,
            f32_or_i64(&original_cropbox[1])?,
            f32_or_i64(&original_cropbox[2])?,
            f32_or_i64(&original_cropbox[3])?,
        ];
        page_box.old_cropbox = Some(vals);
    }
    if let Ok(mediabox) = page.get(b"MediaBox").and_then(Object::as_array)
        && mediabox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&mediabox[0])?,
            f32_or_i64(&mediabox[1])?,
            f32_or_i64(&mediabox[2])?,
            f32_or_i64(&mediabox[3])?,
        ];
        page_box.mediabox = Some(vals);
    }
    if let Ok(artbox) = page.get(b"ArtBox").and_then(Object::as_array)
        && artbox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&artbox[0])?,
            f32_or_i64(&artbox[1])?,
            f32_or_i64(&artbox[2])?,
            f32_or_i64(&artbox[3])?,
        ];
        page_box.artbox = Some(vals);
    }
    if let Ok(bleedbox) = page.get(b"BleedBox").and_then(Object::as_array)
        && bleedbox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&bleedbox[0])?,
            f32_or_i64(&bleedbox[1])?,
            f32_or_i64(&bleedbox[2])?,
            f32_or_i64(&bleedbox[3])?,
        ];
        page_box.bleedbox = Some(vals);
    }
    if let Ok(trimbox) = page.get(b"TrimBox").and_then(Object::as_array)
        && trimbox.len() == 4
    {
        let vals: [f32; 4] = [
            f32_or_i64(&trimbox[0])?,
            f32_or_i64(&trimbox[1])?,
            f32_or_i64(&trimbox[2])?,
            f32_or_i64(&trimbox[3])?,
        ];
        page_box.trimbox = Some(vals);
    }

    Ok(page_box)
}

fn set_new_crop_box(
    doc: &mut Document,
    page_id: ObjectId,
    new_box: [f32; 4],
) -> Result<(), lopdf::Error> {
    // Get the old cropbox
    let old_cropbox = {
        let page = doc.get_object(page_id).and_then(|o| o.as_dict())?;
        if let Ok(cropbox) = page.get(b"CropBox").and_then(Object::as_array) {
            if cropbox.len() == 4 {
                Some([
                    cropbox[0].as_f32()?,
                    cropbox[1].as_f32()?,
                    cropbox[2].as_f32()?,
                    cropbox[3].as_f32()?,
                ])
            } else {
                None
            }
        } else {
            None
        }
    };

    let new_cropbox = Object::Array(vec![
        Object::Real(new_box[0]),
        Object::Real(new_box[1]),
        Object::Real(new_box[2]),
        Object::Real(new_box[3]),
    ]);

    let page = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut())?;
    page.set(b"CropBox", new_cropbox);
    // check if have OriginalCropBox
    let has_original_cropbox = page.get(b"OriginalCropBox").is_ok();
    if !has_original_cropbox && let Some(old_box) = old_cropbox {
        let original_cropbox = Object::Array(vec![
            Object::Real(old_box[0]),
            Object::Real(old_box[1]),
            Object::Real(old_box[2]),
            Object::Real(old_box[3]),
        ]);

        page.set(b"OriginalCropBox", original_cropbox);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum CropChoice {
    Crop,
    Media,
    Art,
    Bleed,
    Trim,
}

fn f32_or_i64(obj: &Object) -> Result<f32, lopdf::Error> {
    match obj {
        Object::Real(val) => Ok(*val),
        Object::Integer(val) => Ok(*val as f32),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Real or Integer",
            found: obj.enum_variant(),
        }),
    }
}

fn print_box(box_name: &str, box_values: Option<[f32; 4]>) -> String {
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
