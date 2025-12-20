use clap::Args;
use color_print::cprintln;
use lopdf::{
    Object,
    content::{Content, Operation},
};
use qpdf::{
    ObjectStreamMode, QPdfArray, QPdfDictionary, QPdfObject, QPdfObjectLike, QPdfObjectType,
    QPdfStream, StreamDecodeLevel,
};
use std::{collections::HashSet, path::PathBuf};
use tiny_poppler::PdfPasswords;

use crate::common::open_maybe_locked;

const CS_NAME: &str = "/PureBlackCS";

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

    cprintln!("<m,s>Loading PDF</>: {}", args.pdf.display());
    // read PDF to bytes first to avoid multiple opens
    let pdf_bytes = std::fs::read(&args.pdf)
        .map_err(|err| format!("Failed to read PDF file {}: {}", args.pdf.display(), err))?;

    let doc = open_maybe_locked(&pdf_bytes, passwords)?;
    let total_pages = doc
        .get_num_pages()
        .map_err(|err| format!("Failed to get number of pages: {}", err))?;

    // Make the colorspace
    let tint_func_ty = doc.new_integer(2);
    let tint_domain = doc.new_array_from(vec![
        QPdfObject::from(doc.new_integer(0)),
        QPdfObject::from(doc.new_integer(1)),
    ]);
    let tint_range = doc.new_array_from(vec![
        QPdfObject::from(doc.new_integer(0)),
        QPdfObject::from(doc.new_integer(1)),
    ]);
    let tint_c0 = doc.new_array_from(vec![QPdfObject::from(doc.new_real(1.0, 1))]);
    let tint_c1 = doc.new_array_from(vec![QPdfObject::from(doc.new_real(0.0, 1))]);
    let tint_n = doc.new_real(1.0, 1);

    let tint_transform = doc.new_dictionary_from(vec![
        ("/FunctionType", QPdfObject::from(tint_func_ty)),
        ("/Domain", QPdfObject::from(tint_domain)),
        ("/Range", QPdfObject::from(tint_range)),
        ("/C0", QPdfObject::from(tint_c0)),
        ("/C1", QPdfObject::from(tint_c1)),
        ("/N", QPdfObject::from(tint_n)),
    ]);

    let separation_color = doc.new_array_from(vec![
        doc.new_name("/Separation"),
        doc.new_name("/All"),
        doc.new_name("/DeviceGray"),
        QPdfObject::from(tint_transform),
    ]);

    cprintln!("<m,s>Processing pages</>...");
    for pg_num in 0..total_pages {
        let page = doc
            .get_page(pg_num)
            .ok_or(format!("Page {} not found in document.", pg_num + 1))?;

        if is_pure_stencil_page(&page)? {
            cprintln!(
                "<cyan>Page <bold>{}</bold></cyan>: Pure stencil page detected, applying fix...",
                pg_num + 1
            );
            inplace_fix_colorspace(&doc, page, &separation_color)?;
        }
    }

    cprintln!(
        "<magenta,bold>Saving output PDF</>: {}",
        args.output.display()
    );

    let pdf_version = doc.get_pdf_version();

    doc.writer()
        .static_id(false)
        .force_pdf_version(&pdf_version)
        .normalize_content(true)
        .preserve_unreferenced_objects(false)
        .compress_streams(true)
        .object_stream_mode(ObjectStreamMode::Preserve)
        .write(&args.output)
        .map_err(|err| format!("Failed to save output PDF: {}", err))?;

    Ok(())
}

fn is_pure_stencil_page(page: &QPdfDictionary) -> Result<bool, String> {
    let mut safe_stencils: HashSet<NameDash> = HashSet::new();
    let mut forbidden_objects: HashSet<NameDash> = HashSet::new();

    if let Some(resources_obj) = page.get("/Resources")
        && let Ok(resources_dict) = object_to_dictionary(resources_obj)
        && let Some(xobjects_obj) = resources_dict.get("/XObject")
        && let Ok(xobjects_dict) = object_to_dictionary(xobjects_obj)
    {
        for name in xobjects_dict.keys() {
            let name = NameDash::from(name);
            if let Some(xobj) = xobjects_dict.get(name.as_ref())
                && let Ok(xobj_dict) = xobject_to_dictionary(xobj)
                && let Some(subtype_obj) = xobj_dict.get("/Subtype")
                && let Ok(subtype_name) = object_to_name(subtype_obj)
            {
                if subtype_name != "/Image" {
                    forbidden_objects.insert(name);
                } else {
                    let is_image_mask = xobj_dict.has("/ImageMask");
                    let has_smask = xobj_dict.has("/SMask");
                    let has_mask = xobj_dict.has("/Mask");

                    if is_image_mask && !has_smask && !has_mask {
                        safe_stencils.insert(name);
                    } else {
                        forbidden_objects.insert(name);
                    }
                }
            }
        }
    }

    // load the contents
    if let Some(contents_obj) = page.get("/Contents")
        && let Ok(contents_stream) = object_to_stream(contents_obj)
        && let Ok(content_data) = contents_stream.get_data(StreamDecodeLevel::All)
    {
        let content_data = Content::decode(&content_data)
            .map_err(|err| format!("Failed to decode content stream: {}", err))?;

        let mut has_drawn_stencil = false;
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

        Ok(has_drawn_stencil)
    } else {
        // failed to parse or no contents, assume not pure stencil
        Ok(false)
    }
}

fn inplace_fix_colorspace(
    doc: &qpdf::QPdf,
    page: QPdfDictionary,
    colorspace: &QPdfArray,
) -> Result<(), String> {
    // first, inject the colorspace into the /Resources
    let resources_obj = page
        .get("/Resources")
        .ok_or("Page missing /Resources dictionary.".to_string())?;
    let resources_dict = object_to_dictionary(resources_obj)?;
    let cs_dict = if let Some(cs_obj) = resources_dict.get("/ColorSpace") {
        object_to_dictionary(cs_obj)?
    } else {
        doc.new_dictionary()
    };
    cs_dict.set(CS_NAME, colorspace);
    resources_dict.set("/ColorSpace", cs_dict);

    // Now, modify the content stream to use the new colorspace
    let contents_obj = page
        .get("/Contents")
        .ok_or("Page missing /Contents stream.".to_string())?;
    let contents_stream = object_to_stream(contents_obj)?;
    let content_data = contents_stream
        .get_data(StreamDecodeLevel::All)
        .map_err(|err| format!("Failed to get content stream data: {}", err))?;
    let content = Content::decode(&content_data)
        .map_err(|err| format!("Failed to decode content stream: {}", err))?;

    let mut is_before_cs = false;
    let mut new_operations = Vec::new();
    let cs_buffer = CS_NAME.as_bytes();
    for op in content.operations {
        if op.operator == "cs" || op.operator == "CS" {
            // Replace the operand with our new colorspace
            let new_op = Operation::new(&op.operator, vec![Object::Name(cs_buffer.to_vec())]);
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
    let encoded_content = new_content
        .encode()
        .map_err(|err| format!("Failed to encode modified content stream: {}", err))?;

    let content_dict = contents_stream.get_dictionary();
    let content_filters = content_dict.get("/Filter").unwrap_or(doc.new_null());
    let content_params = content_dict.get("/DecodeParms").unwrap_or(doc.new_null());
    contents_stream.replace_data(encoded_content, content_filters, content_params);

    // all done
    Ok(())
}

fn object_to_dictionary(obj: QPdfObject) -> Result<QPdfDictionary, String> {
    let kind = obj.get_type();
    match kind {
        QPdfObjectType::Dictionary => Ok(QPdfDictionary::from(obj)),
        _ => Err(format!("Expected dictionary object, found {:?}", kind)),
    }
}

fn object_to_stream(obj: QPdfObject) -> Result<QPdfStream, String> {
    let kind = obj.get_type();
    match kind {
        QPdfObjectType::Stream => Ok(QPdfStream::from(obj)),
        _ => Err(format!("Expected stream object, found {:?}", kind)),
    }
}

fn object_to_name(obj: QPdfObject) -> Result<NameDash, String> {
    let kind = obj.get_type();
    match kind {
        QPdfObjectType::Name => {
            let name_str = obj.as_name();
            Ok(NameDash::from(name_str))
        }
        _ => Err(format!("Expected name object, found {:?}", kind)),
    }
}

fn xobject_to_dictionary(xobj: QPdfObject) -> Result<QPdfDictionary, String> {
    let kind = xobj.get_type();
    match kind {
        QPdfObjectType::Stream => {
            let stream = QPdfStream::from(xobj);
            Ok(stream.get_dictionary())
        }
        QPdfObjectType::Dictionary => Ok(QPdfDictionary::from(xobj)),
        _ => Err(format!(
            "Expected XObject dictionary or stream, found {:?}",
            kind
        )),
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
