use clap::Args;
use color_print::cprintln;
use lopdf::{Dictionary, Document, Object, ObjectId};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use tiny_poppler::PdfPasswords;

use crate::common::{ensure_pdf_output, unlock_pdf};

#[derive(Args)]
pub struct UnwatermarkArgs {
    /// Path to the PDF file to unwatermark.
    pub pdf: PathBuf,
    /// Output file path to save the unwatermarked PDF.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

pub fn run(args: UnwatermarkArgs, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    ensure_pdf_output(&args.output)?;

    cprintln!("<magenta,bold>Loading PDF</>: {}", args.pdf.display());
    let doc = Document::load(args.pdf).map_err(|err| err.to_string())?;

    unlock_pdf(&doc, passwords)?;

    cprintln!("<magenta,bold>Processing pages</>...");
    let pages = doc.get_pages();
    let mut similar_hashes = HashMap::new();
    for (page_num, object_id) in pages {
        let collected_images = collect_images_deep(&doc, object_id)
            .map_err(|err| format!("Error collecting images on page {}: {}", page_num, err))?;
        cprintln!(
            "<cyan>Page {}</>: Found {} images.",
            page_num,
            collected_images.len()
        );

        // for same hashes, group their image IDs for later removal
        for image_info in collected_images {
            similar_hashes
                .entry(image_info.hash.clone())
                .or_insert_with(Vec::new)
                .push((page_num, image_info));
        }
    }

    // Identify duplicates (hashes with more than one image)
    let duplicates: Vec<(&String, &Vec<(u32, SimpleImageInfo)>)> = similar_hashes
        .iter()
        .filter(|(_hash, ids)| ids.len() > 1)
        .collect();

    cprintln!(
        "<magenta,bold>Found {} duplicate images.</>",
        duplicates.len()
    );

    let mut to_be_deleted_ids: Vec<ObjectId> = vec![];
    for (hash, ids) in duplicates {
        cprintln!(
            "<yellow>Hash:</> {} - <red>Occurrences:</> {}",
            hash,
            ids.len()
        );
        for (page_num, image_info) in ids {
            cprintln!(
                "  - Page {}, Image ID {:?}, Size: {}x{}",
                page_num,
                image_info.id,
                image_info.width,
                image_info.height
            );
        }

        // ask
        let confirm = inquire::Confirm::new("Delete these images?")
            .with_default(false)
            .prompt();

        match confirm {
            Ok(true) => {
                for (_page_num, image_info) in ids {
                    to_be_deleted_ids.push(image_info.id);
                }
            }
            Ok(false) => {
                cprintln!("<green>Skipped deletion for this set.</>");
            }
            Err(err) => {
                return Err(format!("Error during confirmation prompt: {}", err));
            }
        }
    }

    if to_be_deleted_ids.is_empty() {
        cprintln!("<green>No images selected for deletion. Exiting.</>");
        return Ok(());
    }

    cprintln!(
        "<magenta,bold>Deleting {} images and saving to {}</>...",
        to_be_deleted_ids.len(),
        args.output.display()
    );
    let mut doc = doc;
    for image_id in to_be_deleted_ids {
        cprintln!("<red>Removing image ID {:?}</>", image_id);
        remove_watermark_references(&mut doc, image_id);
        doc.delete_object(image_id);
    }
    doc.prune_objects();

    doc.save(&args.output)
        .map_err(|err| format!("Error saving PDF: {}", err))?;

    Ok(())
}

/// including those nested inside Form XObjects.
fn collect_images_deep(
    doc: &Document,
    page_id: ObjectId,
) -> Result<Vec<SimpleImageInfo>, lopdf::Error> {
    let mut images = Vec::new();
    let mut visited_forms = HashSet::new();

    let page = doc.get_object(page_id).and_then(|o| o.as_dict())?;

    if let Ok(resources) = page.get(b"Resources") {
        let resources_dict = resolve_to_dict(doc, resources)?;
        scan_resources(doc, resources_dict, &mut images, &mut visited_forms)?;
    }

    Ok(images)
}

/// Helper: Recursively scans a Resources dictionary
fn scan_resources(
    doc: &Document,
    resources: &Dictionary,
    collected_images: &mut Vec<SimpleImageInfo>,
    visited_forms: &mut HashSet<ObjectId>,
) -> Result<(), lopdf::Error> {
    // Look for the "XObject" dictionary
    if let Ok(xobjects) = resources.get(b"XObject") {
        let xobjects_dict = resolve_to_dict(doc, xobjects)?;

        for (_name, object) in xobjects_dict.iter() {
            // We need the ObjectId to track visitation and for the final list
            let (obj_id, actual_object) = match object {
                Object::Reference(id) => (*id, doc.get_object(*id)?),
                // If it's an inline object (rare for XObjects but possible), we can't "collect" an ID.
                // We skip inline objects for deletion purposes since they don't have an ID to delete.
                _ => continue,
            };

            if let Ok(stream) = actual_object.as_stream() {
                // Check the Subtype
                if let Ok(subtype) = stream.dict.get(b"Subtype") {
                    let subtype_name = subtype.as_name()?;

                    if subtype_name == b"Image" {
                        // FOUND ONE: It's an image
                        let width = stream.dict.get(b"Width").and_then(|w| w.as_i64())? as u32;
                        let height = stream.dict.get(b"Height").and_then(|h| h.as_i64())? as u32;

                        let hash = {
                            let digest = Sha256::digest(&stream.content);
                            format!("{:x}", digest)
                        };

                        let image_info = SimpleImageInfo {
                            id: obj_id,
                            width,
                            height,
                            hash,
                        };

                        if !collected_images.contains(&image_info) {
                            collected_images.push(image_info);
                        }
                    } else if subtype_name == b"Form" {
                        // RECURSE: It's a Form, it might contain images inside
                        if !visited_forms.contains(&obj_id) {
                            visited_forms.insert(obj_id);

                            // Forms have their own "Resources" dictionary
                            if let Ok(form_res) = stream.dict.get(b"Resources")
                                && let Ok(form_res_dict) = resolve_to_dict(doc, form_res)
                            {
                                scan_resources(
                                    doc,
                                    form_res_dict,
                                    collected_images,
                                    visited_forms,
                                )?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Helper: Resolves an Object (which might be a Reference) to a Dictionary
fn resolve_to_dict<'a>(
    doc: &'a Document,
    object: &'a Object,
) -> Result<&'a Dictionary, lopdf::Error> {
    match object {
        Object::Dictionary(dict) => Ok(dict),
        Object::Reference(id) => doc.get_object(*id)?.as_dict(),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Dictionary or Reference",
            found: object.enum_variant(),
        }),
    }
}

fn remove_watermark_references(doc: &mut Document, watermark_id: ObjectId) {
    // We need to collect page IDs first to avoid borrowing issues while iterating
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();

    for page_id in page_ids {
        if let Ok(page_obj) = doc.get_object_mut(page_id)
            && let Ok(page_dict) = page_obj.as_dict_mut()
            && let Ok(annots) = page_dict.get_mut(b"Annots")
            && let Ok(arr) = annots.as_array_mut()
        {
            // Remove any reference that matches our watermark ID
            arr.retain(|obj| match obj.as_reference() {
                Ok(ref_id) => ref_id != watermark_id,
                _ => true,
            });
        }

        let mut names_to_remove = HashSet::new();

        if let Ok((Some(resources), _)) = doc.get_page_resources(page_id)
            && let Ok(xobjects) = resources.get(b"XObject")
            && let Ok(xobj_dict) = xobjects.as_dict()
        {
            for (name, value) in xobj_dict.iter() {
                if let Ok(ref_id) = value.as_reference()
                    && ref_id == watermark_id
                {
                    names_to_remove.insert(name.to_vec());
                }
            }
        }

        // If this page actually uses the watermark, scrub the content
        if !names_to_remove.is_empty()
            && let Ok(page_obj) = doc.get_object_mut(page_id)
        {
            let page_dict = page_obj.as_dict_mut().unwrap();
            let resources = page_dict
                .get_mut(b"Resources")
                .unwrap()
                .as_dict_mut()
                .unwrap();
            let xobjects = resources
                .get_mut(b"XObject")
                .unwrap()
                .as_dict_mut()
                .unwrap();

            for name in &names_to_remove {
                xobjects.remove(name);
            }
        }
    }
}

#[derive(Clone)]
struct SimpleImageInfo {
    id: ObjectId,
    width: u32,
    height: u32,
    hash: String,
}

impl PartialEq for SimpleImageInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
