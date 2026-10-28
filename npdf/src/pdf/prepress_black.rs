//! Rewriting baked-RGB black into linear `DeviceGray`.
//!
//! Three passes, in the order the reference implementation uses: patch the colourspaces that
//! really are a K plate, then rewrite the fills that land on that plate's curve, then snap
//! any `Indexed` palette that uses it. Each pass only touches what it can prove.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use super::colorspace::{
    AltKind, CURVE_TOLERANCE, CsPath, DEVICEN_GRID, KCurve, SpaceInfo, alt_kind, classify,
    iter_colourspaces, separation_parts,
};
use super::function::eval_function;
use super::{look_through, name_text, resolve_to_dict};

/// What a run of `fix-black` did, or would do.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub spaces: Vec<SpaceInfo>,
    /// The tint transform's output at tint 1.0, in 0..255.
    pub black_point: Option<Vec<i64>>,
    pub patched_spaces: usize,
    pub patched_ops: usize,
    pub patched_palettes: usize,
    pub notes: Vec<String>,
}

impl Report {
    pub fn changed(&self) -> bool {
        self.patched_spaces + self.patched_ops + self.patched_palettes > 0
    }

    pub fn summary(&self) -> String {
        let black = match &self.black_point {
            Some(point) => {
                let values: Vec<String> = point.iter().map(i64::to_string).collect();
                format!("black point ({})", values.join(", "))
            }
            None => "no baked RGB black".to_string(),
        };
        format!(
            "{black}; patched {} colourspace(s), {} colour op(s), {} palette(s)",
            self.patched_spaces, self.patched_ops, self.patched_palettes
        )
    }
}

/// Patches the document in place and reports what happened.
///
/// The caller decides whether to save: nothing here writes to disk, so `--dry-run` just
/// prints the report and drops the document.
pub fn patch_prepress_black(
    doc: &mut Document,
    include_cmyk: bool,
) -> Result<Report, lopdf::Error> {
    let mut report = Report::default();

    // ---- classify ----
    let mut candidates: Vec<CsPath> = Vec::new();
    let mut tables: Vec<(Vec<String>, super::function::SampleMatrix)> = Vec::new();
    for path in iter_colourspaces(doc) {
        let Some(object) = path.read(doc) else {
            continue;
        };
        let Some(object) = look_through(doc, object) else {
            continue;
        };
        let judged = classify(doc, object, include_cmyk);
        report.spaces.push(judged.info.clone());
        if judged.patchable {
            candidates.push(path);
        }
        if let Some(table) = judged.curve {
            tables.push((judged.info.colorants.clone(), table));
        }
    }

    // ---- pick the reference K curve ----
    let mut curve = None;
    if !tables.is_empty() {
        // A `/Black` colorant wins; otherwise the first one seen.
        tables.sort_by_key(|(colorants, _)| colorants.as_slice() != ["/Black".to_string()]);
        let reference = KCurve::new(tables[0].1.clone());
        report.black_point = Some(reference.black_point());
        for (colorants, table) in &tables[1..] {
            let worst = worst_difference(&reference, table);
            if worst > CURVE_TOLERANCE {
                report.notes.push(format!(
                    "K table of {} differs from the {} table; using the latter for fills",
                    colorants.join(" "),
                    tables[0].0.join(" ")
                ));
            }
        }
        curve = Some(reference);
    }

    // ---- patch the colourspaces ----
    for path in candidates {
        let Some(inputs) = path
            .read(doc)
            .and_then(|object| separation_parts(doc, object))
            .map(|(_, colorants)| colorants.len())
        else {
            continue;
        };
        let Some(transform) = gray_transform(doc, &path, curve.as_ref(), inputs) else {
            continue;
        };
        let transform_id = doc.add_object(transform);
        let Some(array) = path
            .write(doc)
            .and_then(|object| object.as_array_mut().ok())
        else {
            continue;
        };
        array[2] = Object::Name(b"DeviceGray".to_vec());
        array[3] = Object::Reference(transform_id);
        // NChannel /Attributes (/Process) would let a renderer bypass the alternate.
        array.truncate(4);
        report.patched_spaces += 1;
    }

    // ---- rewrite fills and palettes ----
    if let Some(curve) = &curve {
        let pages = doc.get_pages();
        for page_id in pages.values() {
            report.patched_ops += rewrite_page(doc, *page_id, curve);
        }
        report.patched_ops += rewrite_forms(doc, curve);
        report.patched_palettes += patch_palettes(doc, curve);
    }

    Ok(report)
}

/// Largest per-channel difference between two K tables, over the rows they share.
fn worst_difference(reference: &KCurve, other: &super::function::SampleMatrix) -> f64 {
    let rows = reference.table.rows.min(other.rows);
    let cols = reference.table.cols.min(other.cols);
    let mut worst: f64 = 0.0;
    for row in 0..rows {
        for col in 0..cols {
            worst = worst.max((reference.table.at(row, col) - other.at(row, col)).abs());
        }
    }
    worst
}

/// The tint transform that turns a K plate into `DeviceGray`.
fn gray_transform(
    doc: &Document,
    path: &CsPath,
    curve: Option<&KCurve>,
    inputs: usize,
) -> Option<Object> {
    if inputs <= 1 {
        // Identity on the tint axis: tint 0 is paper, tint 1 is full black.
        return Some(Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(1.0)],
            "C1" => vec![Object::Real(0.0)],
            "N" => 1,
        }));
    }

    let curve = curve?;
    let array = path.read(doc)?.as_array().ok()?;

    // A 17-per-axis lattice, stored with the first input varying fastest.
    let rows = DEVICEN_GRID.pow(inputs as u32);
    let mut xs = Vec::with_capacity(rows * inputs);
    for row in 0..rows {
        for axis in 0..inputs {
            let stride = DEVICEN_GRID.pow(axis as u32);
            xs.push(((row / stride) % DEVICEN_GRID) as f64 / (DEVICEN_GRID - 1) as f64);
        }
    }

    let evaluated = eval_function(doc, &array[3], &xs, rows).ok()?;
    let mut samples = Vec::with_capacity(rows);
    for row in 0..rows {
        let luma = super::colorspace::luma_of(evaluated.row(row), 255.0);

        let gray = 1.0 - curve.k_from_luma(luma);
        samples.push((gray * 255.0).round().clamp(0.0, 255.0) as u8);
    }

    let domain: Vec<Object> = (0..inputs)
        .flat_map(|_| [Object::Real(0.0), Object::Real(1.0)])
        .collect();
    let dict = dictionary! {
        "FunctionType" => 0,
        "Domain" => domain,
        "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
        "Size" => vec![Object::Integer(DEVICEN_GRID as i64); inputs],
        "BitsPerSample" => 8,
    };

    let mut stream = Stream::new(dict, Vec::new());
    stream.set_content(samples);
    Some(Object::Stream(stream))
}

/// Rewrites a page's contents, returning how many colour operations changed.
fn rewrite_page(doc: &mut Document, page_id: ObjectId, curve: &KCurve) -> usize {
    let content_ids = doc.get_page_contents(page_id);
    if content_ids.is_empty() {
        return 0;
    }

    // Page resources, resolved once so `cs` names can be looked up per operator.
    let resources = page_resources(doc, page_id);

    // The streams of one page form a single content sequence, so they are joined before
    // decoding; splitting them would reset the colour-space state at the join.
    let mut buffer = Vec::new();
    for id in &content_ids {
        if let Ok(stream) = doc.get_object(*id).and_then(Object::as_stream)
            && let Ok(bytes) = stream.decompressed_content()
        {
            buffer.extend_from_slice(&bytes);
            buffer.push(b'\n');
        }
    }

    let Some((changed, rewritten)) = rewrite_stream(doc, &buffer, resources.as_ref(), curve) else {
        return 0;
    };

    // A fresh stream rather than an in-place write: a page's content stream can be shared
    // with another page, and this one now holds every stream's content.
    let id = doc.add_object(Object::Stream(flate_stream(rewritten)));
    if let Ok(page) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
        page.set("Contents", Object::Reference(id));
    }
    changed
}

fn page_resources(doc: &Document, page_id: ObjectId) -> Option<Dictionary> {
    let page = doc.get_dictionary(page_id).ok()?;
    let resources = page.get(b"Resources").ok()?;
    resolve_to_dict(doc, resources).ok().cloned()
}

/// Rewrites every `/Subtype /Form` stream, returning how many colour operations changed.
fn rewrite_forms(doc: &mut Document, curve: &KCurve) -> usize {
    let form_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, object)| match object {
            Object::Stream(stream) => match stream.dict.get(b"Subtype") {
                Ok(subtype) if subtype.as_name().ok() == Some(b"Form") => Some(*id),
                _ => None,
            },
            _ => None,
        })
        .collect();

    let mut changed = 0;
    for id in form_ids {
        let resources = {
            let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
                continue;
            };
            let Ok(entry) = stream.dict.get(b"Resources") else {
                continue;
            };
            resolve_to_dict(doc, entry).ok().cloned()
        };
        let Ok(bytes) = doc
            .get_object(id)
            .and_then(Object::as_stream)
            .and_then(Stream::decompressed_content)
        else {
            continue;
        };
        let Some((count, rewritten)) = rewrite_stream(doc, &bytes, resources.as_ref(), curve)
        else {
            continue;
        };
        if let Ok(stream) = doc.get_object_mut(id).and_then(Object::as_stream_mut) {
            stream.dict.remove(b"DecodeParms");
            stream.dict.remove(b"Filter");
            stream.dict.set("Filter", "FlateDecode");
            stream.set_content(zlib(&rewritten));
            changed += count;
        }
    }
    changed
}

/// Snaps `Indexed` palettes whose base is RGB onto the document's K curve.
fn patch_palettes(doc: &mut Document, curve: &KCurve) -> usize {
    let palette_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| match object {
            Object::Array(items) if items.len() == 4 => {
                name_text(&items[0]).as_deref() == Some("Indexed")
                    && alt_kind(doc, &items[1]) == AltKind::Rgb
            }
            _ => false,
        })
        .map(|(id, _)| *id)
        .collect();

    let mut changed = 0;
    for id in palette_ids {
        if patch_palette(doc, id, curve) {
            changed += 1;
        }
    }
    changed
}

fn patch_palette(doc: &mut Document, id: ObjectId, curve: &KCurve) -> bool {
    let lookup = {
        let Ok(Object::Array(items)) = doc.get_object(id) else {
            return false;
        };
        let Some(entry) = items.get(3) else {
            return false;
        };
        entry.clone()
    };

    let patched = |bytes: &[u8]| -> Option<Vec<u8>> {
        let mut out = bytes.to_vec();
        let mut touched = false;
        for index in (0..bytes.len().saturating_sub(2)).step_by(3) {
            let gray = curve.gray([
                f64::from(bytes[index]) / 255.0,
                f64::from(bytes[index + 1]) / 255.0,
                f64::from(bytes[index + 2]) / 255.0,
            ]);
            if let Some(gray) = gray {
                let value = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
                out[index..index + 3].copy_from_slice(&[value, value, value]);
                touched = true;
            }
        }
        touched.then_some(out)
    };

    match &lookup {
        Object::String(bytes, format) => {
            let Some(out) = patched(bytes) else {
                return false;
            };
            let format = *format;
            let Ok(Object::Array(items)) = doc.get_object_mut(id) else {
                return false;
            };
            items[3] = Object::String(out, format);
            true
        }
        Object::Reference(stream_id) => {
            let stream_id = *stream_id;
            let Ok(bytes) = doc
                .get_object(stream_id)
                .and_then(Object::as_stream)
                .and_then(Stream::decompressed_content)
            else {
                return false;
            };
            let Some(out) = patched(&bytes) else {
                return false;
            };
            let Ok(stream) = doc
                .get_object_mut(stream_id)
                .and_then(Object::as_stream_mut)
            else {
                return false;
            };
            stream.dict.remove(b"DecodeParms");
            stream.dict.remove(b"Filter");
            stream.dict.set("Filter", "FlateDecode");
            stream.set_content(zlib(&out));
            true
        }
        Object::Stream(_) => {
            let Ok(bytes) = doc
                .get_object(id)
                .and_then(Object::as_array)
                .and_then(|items| items[3].as_stream())
                .and_then(Stream::decompressed_content)
            else {
                return false;
            };
            let Some(out) = patched(&bytes) else {
                return false;
            };
            let Ok(Object::Array(items)) = doc.get_object_mut(id) else {
                return false;
            };
            let Ok(stream) = items[3].as_stream_mut() else {
                return false;
            };
            stream.dict.remove(b"DecodeParms");
            stream.dict.remove(b"Filter");
            stream.dict.set("Filter", "FlateDecode");
            stream.set_content(zlib(&out));
            true
        }
        _ => false,
    }
}

/// Colour-space state carried through a content stream.
#[derive(Debug, Clone, Copy, Default)]
struct GfxColour {
    fill_rgb: bool,
    stroke_rgb: bool,
}

/// Rewrites `rg` / `RG` / `scn` / `SCN` operands that land on the K curve.
///
/// Returns the number of changed operations and the re-encoded stream, or `None` when
/// nothing changed or the stream cannot be decoded.
fn rewrite_stream(
    doc: &Document,
    data: &[u8],
    resources: Option<&Dictionary>,
    curve: &KCurve,
) -> Option<(usize, Vec<u8>)> {
    let content = Content::decode(data).ok()?;

    let mut colour = GfxColour::default();
    let mut stack: Vec<GfxColour> = Vec::new();
    let mut changed = 0;
    let mut operations = Vec::with_capacity(content.operations.len());

    for mut operation in content.operations {
        let operator = operation.operator.clone();
        match operator.as_str() {
            // Unlike the reference implementation, `q`/`Q` save and restore the state, so
            // colour set inside a block does not leak past the matching `Q`.
            "q" => stack.push(colour),
            "Q" => colour = stack.pop().unwrap_or_default(),
            "cs" | "CS" => {
                let rgb = operation
                    .operands
                    .first()
                    .map(|name| is_rgb_space(doc, resources, name))
                    .unwrap_or(false);
                if operator == "cs" {
                    colour.fill_rgb = rgb;
                } else {
                    colour.stroke_rgb = rgb;
                }
            }
            "scn" | "sc" | "SCN" | "SC" => {
                let is_fill = operator.chars().all(|ch| ch.is_lowercase());
                let active = if is_fill {
                    colour.fill_rgb
                } else {
                    colour.stroke_rgb
                };
                if active
                    && operation.operands.len() == 3
                    && let Some(gray) =
                        operands_as_rgb(&operation.operands).and_then(|rgb| curve.gray(rgb))
                {
                    operation.operands = vec![Object::Real(gray as f32); 3];
                    changed += 1;
                }
            }
            "rg" | "RG" => {
                if operation.operands.len() == 3
                    && let Some(gray) =
                        operands_as_rgb(&operation.operands).and_then(|rgb| curve.gray(rgb))
                {
                    operation.operator = if operator == "rg" { "g" } else { "G" }.to_string();
                    operation.operands = vec![Object::Real(gray as f32)];
                    changed += 1;
                }
            }
            // These switch to a non-RGB device space.
            "g" | "k" => colour.fill_rgb = false,
            "G" | "K" => colour.stroke_rgb = false,
            _ => {}
        }
        operations.push(operation);
    }

    if changed == 0 {
        return None;
    }

    let encoded = Content { operations }.encode().ok()?;
    Some((changed, encoded))
}

fn operands_as_rgb(operands: &[Object]) -> Option<[f64; 3]> {
    let mut rgb = [0.0; 3];
    for (slot, operand) in rgb.iter_mut().zip(operands) {
        *slot = match operand {
            Object::Integer(value) => *value as f64,
            Object::Real(value) => f64::from(*value),
            _ => return None,
        };
    }
    Some(rgb)
}

/// Whether a `cs` / `CS` operand names a plain RGB space.
fn is_rgb_space(doc: &Document, resources: Option<&Dictionary>, name: &Object) -> bool {
    let Some(name) = name_text(name) else {
        return false;
    };
    if name == "DeviceRGB" {
        return true;
    }
    let Some(resources) = resources else {
        return false;
    };
    let Ok(colourspaces) = resources.get(b"ColorSpace") else {
        return false;
    };
    let Some(colourspaces) = look_through(doc, colourspaces) else {
        return false;
    };
    let Ok(colourspaces) = colourspaces.as_dict() else {
        return false;
    };
    let Ok(space) = colourspaces.get(name.as_bytes()) else {
        return false;
    };
    let Some(space) = look_through(doc, space) else {
        return false;
    };
    // A named Separation/DeviceN is *not* an RGB space even when its alternate is.
    alt_kind(doc, space) == AltKind::Rgb && separation_parts(doc, space).is_none()
}

/// A zlib deflated stream object holding `content`.
fn flate_stream(content: Vec<u8>) -> Stream {
    let mut dict = Dictionary::new();
    dict.set("Filter", "FlateDecode");
    let mut stream = Stream::new(dict, Vec::new());
    stream.set_content(zlib(&content));
    stream
}

fn zlib(content: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    // Writing into a growable Vec cannot fail.
    let _ = encoder.write_all(content);
    encoder.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::Operation;
    use lopdf::dictionary;

    fn neutral_curve() -> KCurve {
        // 0..255 luma, neutral: K = 1 - v/255.
        let mut table = super::super::function::SampleMatrix::new(256, 3);
        for row in 0..256 {
            let value = 255.0 - row as f64;
            for col in 0..3 {
                table.data[row * 3 + col] = value;
            }
        }
        KCurve::new(table)
    }

    fn decode(data: &[u8]) -> Content {
        Content::decode(data).expect("re-encoded content should decode")
    }

    #[test]
    fn an_rgb_fill_on_the_curve_becomes_a_single_grey_operand() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        // 0.5 grey in 0..255 terms is 127.5, which the curve turns back into 0.5.
        let (changed, out) =
            rewrite_stream(&doc, b"0.5 0.5 0.5 rg", None, &curve).expect("should rewrite");
        assert_eq!(changed, 1);
        let content = decode(&out);
        assert_eq!(content.operations[0].operator, "g");
        assert_eq!(content.operations[0].operands.len(), 1);
    }

    #[test]
    fn an_rgb_fill_off_the_curve_is_left_alone() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        assert!(rewrite_stream(&doc, b"1 0 0 rg", None, &curve).is_none());
    }

    #[test]
    fn a_stroke_is_rewritten_to_the_uppercase_grey_operator() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        let (changed, out) =
            rewrite_stream(&doc, b"0.5 0.5 0.5 RG", None, &curve).expect("should rewrite");
        assert_eq!(changed, 1);
        assert_eq!(decode(&out).operations[0].operator, "G");
    }

    #[test]
    fn colour_set_inside_a_q_block_does_not_leak_past_the_q() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        // The fill is in a non-RGB space after `g`, but the `cs` inside the block must not
        // survive the `Q`: the trailing `scn` is in an RGB space and must be rewritten.
        let stream = b"0 0 0 g q /CS0 cs 0.5 0.5 0.5 scn Q /DeviceRGB cs 0.5 0.5 0.5 scn";
        let (changed, out) = rewrite_stream(&doc, stream, None, &curve).expect("should rewrite");
        assert_eq!(changed, 1, "only the scn outside the block is RGB");
        let content = decode(&out);
        let scn: Vec<&Operation> = content
            .operations
            .iter()
            .filter(|operation| operation.operator == "scn")
            .collect();
        assert_eq!(scn.len(), 2);
        assert_eq!(scn[0].operands.len(), 3, "inside the block, left alone");
        assert_eq!(scn[1].operands.len(), 3);
        assert!(
            scn[1]
                .operands
                .iter()
                .all(|operand| matches!(operand, Object::Real(_))),
            "the rewritten operand is a real"
        );
        let _ = doc;
    }

    #[test]
    fn a_three_operand_scn_in_a_named_rgb_space_is_rewritten() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        let resources = dictionary! {
            "ColorSpace" => dictionary! {
                "CS0" => Object::Array(vec![
                    Object::Name(b"CalRGB".to_vec()),
                    Object::Dictionary(Dictionary::new()),
                ]),
            },
        };
        let (changed, _) =
            rewrite_stream(&doc, b"/CS0 cs 0.5 0.5 0.5 scn", Some(&resources), &curve)
                .expect("CalRGB is an RGB space");
        assert_eq!(changed, 1);
    }

    #[test]
    fn a_named_separation_is_not_an_rgb_space_even_with_an_rgb_alternate() {
        let doc = Document::with_version("1.7");
        let curve = neutral_curve();
        let resources = dictionary! {
            "ColorSpace" => dictionary! {
                "CS0" => Object::Array(vec![
                    Object::Name(b"Separation".to_vec()),
                    Object::Name(b"Black".to_vec()),
                    Object::Name(b"DeviceRGB".to_vec()),
                    Object::Dictionary(Dictionary::new()),
                ]),
            },
        };
        assert!(
            rewrite_stream(&doc, b"/CS0 cs 0.5 0.5 0.5 scn", Some(&resources), &curve).is_none()
        );
    }

    #[test]
    fn a_report_with_no_patches_is_not_changed() {
        let report = Report::default();
        assert!(!report.changed());
        assert!(report.summary().contains("no baked RGB black"));
    }

    #[test]
    fn a_report_summarises_what_it_patched() {
        let report = Report {
            black_point: Some(vec![35, 31, 32]),
            patched_spaces: 1,
            patched_ops: 4,
            patched_palettes: 2,
            ..Report::default()
        };
        assert!(report.changed());
        assert_eq!(
            report.summary(),
            "black point (35, 31, 32); patched 1 colourspace(s), 4 colour op(s), 2 palette(s)"
        );
    }

    #[test]
    fn an_indexed_palette_entry_on_the_curve_is_snapped_to_grey() {
        let mut doc = Document::with_version("1.7");
        let palette = doc.add_object(Object::Array(vec![
            Object::Name(b"Indexed".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            Object::Integer(1),
            Object::String(
                vec![128, 128, 128, 12, 200, 30],
                lopdf::StringFormat::Literal,
            ),
        ]));

        let curve = neutral_curve();
        assert!(patch_palette(&mut doc, palette, &curve));
        let Object::Array(items) = doc.get_object(palette).expect("palette survives") else {
            panic!("palette should still be an array");
        };
        let Object::String(bytes, _) = &items[3] else {
            panic!("lookup should still be a string");
        };
        assert_eq!(&bytes[..3], &[128, 128, 128], "on-curve entry snapped");
        assert_eq!(&bytes[3..], &[12, 200, 30], "off-curve entry untouched");
    }

    #[test]
    fn zlib_round_trips_through_the_stream_helper() {
        let stream = flate_stream(b"1 0 0 rg".to_vec());
        assert_eq!(
            stream.decompressed_content().expect("should decompress"),
            b"1 0 0 rg"
        );
    }
}
