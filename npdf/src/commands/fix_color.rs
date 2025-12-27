use clap::Args;
use color_print::cprintln;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::{collections::HashSet, path::PathBuf};
use tiny_poppler::PdfPasswords;

use crate::common::{ensure_pdf_output, unlock_pdf};

const CS_NAME: &[u8; 11] = b"PureBlackCS";
const CS_SEP_BASE: &str = "Separation";
const CS_SEP_ALL: &str = "All";
const CS_SEP_DGRAY: &str = "DeviceGray";

#[derive(Args)]
pub struct FixColorspaceArgs {
    /// Path to the PDF file to fix colorspaces.
    pub pdf: PathBuf,
    /// Output file path to save the fixed PDF.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

pub fn run(args: FixColorspaceArgs, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    ensure_pdf_output(&args.output)?;

    cprintln!("<magenta,bold>Loading PDF</>: {}", args.pdf.display());
    let mut doc = Document::load(args.pdf).map_err(|err| err.to_string())?;

    unlock_pdf(&doc, passwords)?;

    let tint_transform = dictionary! {
        "FunctionType" => 2,
        "Domain" => vec![0.into(), 1.into()],
        "Range" => vec![0.into(), 1.into()],
        "C0" => vec![1.0.into()],
        "C1" => vec![0.0.into()],
        "N" => 1.0
    };

    let separation_cs_array = Object::Array(vec![
        CS_SEP_BASE.into(),
        CS_SEP_ALL.into(),
        CS_SEP_DGRAY.into(),
        tint_transform.into(),
    ]);

    cprintln!("<magenta,bold>Processing pages</>...");
    let pages = doc.get_pages();
    for (page_num, object_id) in pages {
        if is_page_pure_stencil(&doc, object_id).map_err(|err| err.to_string())? {
            cprintln!(
                "<cyan>Page <bold>{}</bold></cyan>: Pure stencil page detected, applying fix...",
                page_num
            );

            inplace_fix_colorspace(&mut doc, object_id, &separation_cs_array)
                .map_err(|err| err.to_string())?;
        }
    }

    cprintln!(
        "<magenta,bold>Saving output PDF</>: {}",
        args.output.display()
    );
    doc.save(args.output)
        .map_err(|err| format!("Failed to save output PDF: {}", err))?;

    Ok(())
}

/// including those nested inside Form XObjects.
fn is_page_pure_stencil(doc: &Document, page_id: ObjectId) -> Result<bool, lopdf::Error> {
    let page = doc.get_object(page_id).and_then(|o| o.as_dict())?;

    let mut safe_stencils: HashSet<NameDash> = HashSet::new();
    let mut forbidden_objects: HashSet<NameDash> = HashSet::new();

    if let Ok(resources) = page.get(b"Resources")
        && let Ok(resources_dict) = resolve_to_dict(doc, resources)
        && let Ok(xobject) = resources_dict.get(b"XObject")
        && let Ok(xobject_dict) = resolve_to_dict(doc, xobject)
    {
        for (name, object) in xobject_dict.iter() {
            // We need the ObjectId to track visitation and for the final list
            let name_actual = NameDash::from(name);
            if let Ok(obj_stream) = resolve_to_stream(doc, object)
                && let Ok(subtype) = obj_stream.dict.get(b"Subtype")
                && let Ok(subtype_name) = subtype.as_name()
            {
                if subtype_name != b"Image" {
                    forbidden_objects.insert(name_actual);
                } else {
                    let is_img_mask = obj_stream.dict.has(b"ImageMask");
                    let has_smask = obj_stream.dict.has(b"SMask");
                    let has_mask = obj_stream.dict.has(b"Mask");

                    if is_img_mask && !has_mask && !has_smask {
                        safe_stencils.insert(name_actual);
                    } else {
                        forbidden_objects.insert(name_actual);
                    }
                }
            }
        }
    }

    // check the contents stream for any drawing operation
    let mut has_drawn_stencil = false;
    if let Ok(content) = page.get(b"Contents")
        && let Ok(content_stream) = resolve_to_stream(doc, content)
        && let Ok(content_data) = content_stream.decompressed_content()
    {
        let content_data = Content::decode(&content_data)?;

        for op in content_data.operations {
            if op.operator == "Do"
                && let Some(operand_obj) = op.operands.first()
                && let Ok(operand_name) = operand_obj.as_name()
            {
                let obj_name = NameDash::from(operand_name);
                // found forbidden, just immediately return false
                if forbidden_objects.contains(&obj_name) {
                    return Ok(false);
                }

                if safe_stencils.contains(&obj_name) {
                    has_drawn_stencil = true;
                }
            } else if op.operator == "BI" {
                // inline image found, not pure stencil, quit early
                return Ok(false);
            }
        }
    }

    Ok(has_drawn_stencil)
}

fn inplace_fix_colorspace(
    doc: &mut Document,
    page_id: ObjectId,
    colorspace: &Object,
) -> Result<(), lopdf::Error> {
    {
        // Scope the mutable borrow so it is dropped before we need other borrows of `doc`.
        let page = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut())?;
        let resources = page.get_mut(b"Resources")?.as_dict_mut()?;
        if let Ok(cs_map) = resources.get_mut(b"ColorSpace") {
            let cs_map_obj = cs_map.as_dict_mut()?;
            cs_map_obj.set(CS_NAME, colorspace.clone());
        } else {
            let mut dict = Dictionary::new();
            dict.set(CS_NAME, colorspace.clone());
            resources.set(b"ColorSpace", dict)
        }
    }

    // Clone the Contents object while holding only an immutable borrow so we can later
    // acquire the mutable reference we need without overlapping borrows.
    let contents_obj = {
        let page = doc.get_object(page_id)?.as_dict()?;
        page.get(b"Contents")?.clone()
    };

    let content_stream = match contents_obj {
        Object::Stream(_) => {
            // Inline stream: re-borrow the page mutably to get the stream.
            let page = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut())?;
            match page.get_mut(b"Contents")? {
                Object::Stream(stream) => stream,
                other => {
                    return Err(lopdf::Error::ObjectType {
                        expected: "Dictionary or Reference",
                        found: other.enum_variant(),
                    });
                }
            }
        }
        Object::Reference(ref_id) => {
            // Referenced stream: get the referenced object mutably.
            doc.get_object_mut(ref_id)?.as_stream_mut()?
        }
        other => {
            return Err(lopdf::Error::ObjectType {
                expected: "Dictionary or Reference",
                found: other.enum_variant(),
            });
        }
    };

    let content_data = content_stream.decompressed_content()?;
    let content_data = Content::decode(&content_data)?;
    let mut is_before_cs = false;
    let mut new_operations = Vec::new();
    for op in content_data.operations {
        if op.operator == "cs" || op.operator == "CS" {
            // Replace the operand with our new colorspace
            let new_op = Operation::new(&op.operator, vec![Object::Name(CS_NAME.to_vec())]);
            new_operations.push(new_op);
            is_before_cs = true;
        } else if op.operator == "scn" || op.operator == "SCN" {
            // Replace the operand with correct tint value
            if !is_before_cs {
                new_operations.push(op);
                continue; // only modify if preceded by cs/CS
            }

            let new_op = Operation::new(&op.operator, vec![Object::Real(1.0)]); // Black is the default
            new_operations.push(new_op);
            is_before_cs = false;
        } else {
            is_before_cs = false;
            new_operations.push(op);
        }
    }

    let new_content = Content {
        operations: new_operations,
    };

    let encoded_content = new_content.encode()?;
    // cleanup
    content_stream.dict.remove(b"DecodeParms");
    content_stream.dict.remove(b"Filter");

    use std::io::prelude::*;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&encoded_content)?;
    let compressed = encoder.finish()?;
    content_stream.dict.set("Filter", "FlateDecode");
    content_stream.set_content(compressed);

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

fn resolve_to_stream<'a>(
    doc: &'a Document,
    object: &'a Object,
) -> Result<&'a Stream, lopdf::Error> {
    match object {
        Object::Stream(stream) => Ok(stream),
        Object::Reference(id) => doc.get_object(*id)?.as_stream(),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Stream or Reference",
            found: object.enum_variant(),
        }),
    }
}

#[derive(Debug, Clone)]
struct NameDash(String);

impl AsRef<str> for NameDash {
    fn as_ref(&self) -> &str {
        self.0.trim_start_matches('/')
    }
}

impl From<&[u8]> for NameDash {
    fn from(bytes: &[u8]) -> Self {
        let s = String::from_utf8_lossy(bytes).into_owned();
        NameDash(s)
    }
}

impl From<&str> for NameDash {
    fn from(s: &str) -> Self {
        NameDash(s.to_string())
    }
}

impl From<&Vec<u8>> for NameDash {
    fn from(bytes: &Vec<u8>) -> Self {
        let s = String::from_utf8_lossy(bytes).into_owned();
        NameDash(s)
    }
}

impl From<String> for NameDash {
    fn from(s: String) -> Self {
        NameDash(s)
    }
}

impl PartialEq for NameDash {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl Eq for NameDash {}
impl std::hash::Hash for NameDash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl PartialEq<&str> for NameDash {
    fn eq(&self, other: &&str) -> bool {
        self.as_ref() == other.trim_start_matches('/')
    }
}
