use clap::Args;
use color_eyre::{Result, eyre::Context};
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

use crate::common::{NpdfError, ensure_pdf_output, unlock_pdf};

const CS_NAME: &[u8; 11] = b"PureBlackCS";
const CS_SEP_BASE: &str = "Separation";
const CS_SEP_ALL: &str = "All";
const CS_SEP_DGRAY: &str = "DeviceGray";

const C0: f32 = 1.0;
const C1: f32 = 0.0;
const N: f32 = 1.0;

#[derive(Args)]
pub struct FixColorspaceArgs {
    /// Path to the PDF file to fix colorspaces.
    pub pdf: PathBuf,
    /// Output file path to save the fixed PDF.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

pub fn run(args: FixColorspaceArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if !args.pdf.exists() {
        return Err(NpdfError::MissingPdfFile(args.pdf.display().to_string()).into());
    }

    ensure_pdf_output(&args.output)?;

    cprintln!("<magenta,bold>Loading PDF</>: {}", args.pdf.display());
    let mut doc = Document::load(args.pdf)?;

    unlock_pdf(&doc, passwords)?;

    let tint_transform = dictionary! {
        "FunctionType" => 2,
        "Domain" => vec![0.into(), 1.into()],
        "Range" => vec![0.into(), 1.into()],
        "C0" => vec![C0.into()],
        "C1" => vec![C1.into()],
        "N" => N
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
        if is_page_pure_stencil(&doc, object_id).context(format!("page {}", page_num))? {
            cprintln!(
                "<cyan>Page <bold>{}</bold></cyan>: Pure stencil page detected, applying fix...",
                page_num
            );

            inplace_fix_colorspace(&mut doc, object_id, &separation_cs_array)
                .context(format!("page {}", page_num))?;
        } else {
            cprintln!(
                "<cyan>Page <bold>{}</bold></cyan>: No pure stencil detected, skipping.",
                page_num
            );
        }
    }

    cprintln!(
        "<magenta,bold>Saving output PDF</>: {}",
        args.output.display()
    );
    doc.save(args.output)?;

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

    // Get the colorspace object for later use in tint normalization
    let colorspaces = {
        let page = doc.get_object(page_id)?.as_dict()?;
        let resources = page.get(b"Resources")?.as_dict()?;
        let cs_map = resources.get(b"ColorSpace")?.as_dict()?;

        // Deep cloning to dereference any References
        dereference_dictionary(doc, cs_map)?
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

    let mut new_operations = Vec::new();

    let mut current_fill_cs: Option<Object> = None;
    let mut current_stroke_cs: Option<Object> = None;

    for op in content_data.operations {
        if op.operator == "cs" || op.operator == "CS" {
            // Get the old operator
            let cs_operand = op.operands.first().ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty operands index 0",
                found: "Empty operands index 0",
            })?;
            let cs_name = cs_operand.as_name()?;
            let cs_obj = colorspaces.get(cs_name)?;
            if op.operator == "cs" {
                current_fill_cs = Some(cs_obj.clone());
            } else {
                current_stroke_cs = Some(cs_obj.clone());
            }
            // Replace the operand with our new colorspace
            let new_op = Operation::new(&op.operator, vec![Object::Name(CS_NAME.to_vec())]);
            new_operations.push(new_op);
        } else if op.operator == "scn" || op.operator == "SCN" {
            // Replace the operand with correct tint value
            let is_fill = op.operator == "scn";

            let input_cs = if is_fill {
                current_fill_cs.as_ref()
            } else {
                current_stroke_cs.as_ref()
            }
            .ok_or(lopdf::Error::ObjectType {
                expected: "Current colorspace to be set before scn/SCN",
                found: "No colorspace set",
            })?;

            let components = op
                .operands
                .iter()
                .map(resolve_to_real)
                .collect::<Result<Vec<f32>, lopdf::Error>>()?;

            let scn_value = normalize_scn_to_spot_black(&components, input_cs)?;

            let new_op = Operation::new(&op.operator, vec![Object::Real(scn_value)]); // Black is the default
            new_operations.push(new_op);
        } else {
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

fn normalize_scn_to_spot_black(
    scn_values: &[f32],
    colorspace: &Object,
) -> Result<f32, lopdf::Error> {
    let gray = to_spot_black(scn_values, colorspace)?;

    let tint = (gray - C0) / (C1 - C0);
    // max(0.0, min(1.0, tint))
    let tint = tint.clamp(0.0, 1.0);

    if N != 1.0 {
        Ok(tint.powf(1.0 / N))
    } else {
        Ok(tint)
    }
}

fn to_spot_black(components: &[f32], colorspace: &Object) -> Result<f32, lopdf::Error> {
    if let Object::Array(arr) = colorspace {
        // Get first element as Name
        let first_el: NameDash = arr
            .first()
            .ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty Array index 0",
                found: "Empty Array index 0",
            })?
            .as_name()?
            .into();

        if first_el == "DeviceGray" {
            // Gray colorspace only has one component
            // Round to 0.0 or 1.0
            let gray_value = components.first().ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 0",
                found: "Empty components index 0",
            })?;

            Ok(round_1_0(*gray_value))
        } else if first_el == "DeviceRGB" {
            // Convert RGB to gray using luminosity method
            let r = components.first().ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 0",
                found: "Empty components index 0",
            })?;
            let g = components.get(1).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 1",
                found: "Empty components index 1",
            })?;
            let b = components.get(2).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 2",
                found: "Empty components index 2",
            })?;

            let gray_value = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            Ok(round_1_0(gray_value))
        } else if first_el == "DeviceCMYK" {
            // Convert CMYK to gray using formula
            let c = components.first().ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 0",
                found: "Empty components index 0",
            })?;
            let m = components.get(1).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 1",
                found: "Empty components index 1",
            })?;
            let y = components.get(2).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 2",
                found: "Empty components index 2",
            })?;
            let k = components.get(3).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 3",
                found: "Empty components index 3",
            })?;

            let gray_value =
                1.0 - (0.299 * (1.0 - c) + 0.587 * (1.0 - m) + 0.114 * (1.0 - y)) * (1.0 - k);
            Ok(round_1_0(gray_value))
        } else if first_el == "ICCBased" {
            // Check stream for alternate colorspace
            let icc_stream_obj = arr.get(1).ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty Array index 1",
                found: "Empty Array index 1",
            })?;
            let icc_stream = match icc_stream_obj {
                Object::Stream(stream) => stream,
                _ => {
                    return Err(lopdf::Error::ObjectType {
                        expected: "Stream",
                        found: icc_stream_obj.enum_variant(),
                    });
                }
            };
            let icc_dict = &icc_stream.dict;
            let alt_cs_n = icc_dict.get(b"N")?.as_i64()?;
            match alt_cs_n {
                1 => {
                    let device_gray = Object::Name(b"DeviceGray".to_vec());
                    let array_obj = Object::Array(vec![device_gray]);
                    to_spot_black(components, &array_obj)
                }
                3 => {
                    // DeviceRGB
                    let device_rgb = Object::Name(b"DeviceRGB".to_vec());
                    let array_obj = Object::Array(vec![device_rgb]);
                    to_spot_black(components, &array_obj)
                }
                4 => {
                    // DeviceCMYK
                    let device_cmyk = Object::Name(b"DeviceCMYK".to_vec());
                    let array_obj = Object::Array(vec![device_cmyk]);
                    to_spot_black(components, &array_obj)
                }
                _ => Err(lopdf::Error::ObjectType {
                    expected: "DeviceGray, DeviceRGB, or DeviceCMYK",
                    found: "Unknown ICCBased alternate colorspace",
                }),
            }
        } else if first_el == CS_SEP_BASE {
            // Separation
            // For Separation, we assume the components array has a single tint value
            let tint_value = components.first().ok_or(lopdf::Error::ObjectType {
                expected: "Non-empty components index 0",
                found: "Empty components index 0",
            })?;

            Ok(round_1_0(*tint_value)) // Already a tint value
        } else {
            Err(lopdf::Error::ObjectType {
                expected: "DeviceGray, DeviceRGB, DeviceCMYK or ICCBased",
                found: "Unknown colorspace",
            })
        }
    } else {
        Err(lopdf::Error::ObjectType {
            expected: "Array",
            found: colorspace.enum_variant(),
        })
    }
}

fn round_1_0(value: f32) -> f32 {
    if value < 0.5 { 0.0 } else { 1.0 }
}

fn dereference_dictionary(
    doc: &Document,
    resources: &Dictionary,
) -> Result<Dictionary, lopdf::Error> {
    let mut new_dict = Dictionary::new();
    // When found Reference, dereference it
    for (key, value) in resources.iter() {
        let deref_value = match value {
            Object::Reference(ob) => dereference_reference(doc, *ob)?,
            Object::Array(arr) => {
                let deref_array = dereference_array(doc, arr)?;
                Object::Array(deref_array)
            }
            Object::Dictionary(dict) => {
                let deref_dict = dereference_dictionary(doc, dict)?;
                Object::Dictionary(deref_dict)
            }
            other => other.clone(),
        };

        new_dict.set(key.to_vec(), deref_value);
    }
    Ok(new_dict)
}

fn dereference_array(doc: &Document, array: &Vec<Object>) -> Result<Vec<Object>, lopdf::Error> {
    let mut new_array = Vec::new();
    for item in array {
        let deref_item = match item {
            Object::Reference(ob) => dereference_reference(doc, *ob)?,
            Object::Array(array) => {
                let deref_array = dereference_array(doc, array)?;
                Object::Array(deref_array)
            }
            Object::Dictionary(dict) => {
                let deref_dict = dereference_dictionary(doc, dict)?;
                Object::Dictionary(deref_dict)
            }
            other => other.clone(),
        };
        new_array.push(deref_item);
    }
    Ok(new_array)
}

fn dereference_reference(doc: &Document, object: (u32, u16)) -> Result<Object, lopdf::Error> {
    match doc.get_object(object)? {
        Object::Reference(ob) => dereference_reference(doc, *ob),
        Object::Array(ob) => {
            let deref_array = dereference_array(doc, ob)?;
            Ok(Object::Array(deref_array))
        }
        Object::Dictionary(ob) => {
            let deref_dict = dereference_dictionary(doc, ob)?;
            Ok(Object::Dictionary(deref_dict))
        }
        other => Ok(other.clone()),
    }
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

fn resolve_to_real(object: &Object) -> Result<f32, lopdf::Error> {
    match object {
        Object::Real(val) => Ok(*val),
        Object::Integer(val) => Ok(*val as f32),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Real or Integer",
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
