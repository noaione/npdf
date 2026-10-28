//! Finding and judging `Separation` / `DeviceN` colourspaces, and the document's K curve.
//!
//! Nothing here relies on operand counts or thresholds applied to content-stream values: a
//! colourspace is judged by *evaluating its own tint transform* and looking at what comes
//! out. A space is patchable only when its tint transform really does run from paper to a
//! near-neutral black.

use std::collections::HashSet;

use lopdf::{Document, Object, ObjectId};

use super::function::{FunctionError, SampleMatrix, eval_function};
use super::{look_through, name_text, round_1};

/// Max channel spread (0..255) of a black-ink rendition.
///
/// Press profiles render K as anything from neutral (U.S. Web Coated SWOP: 35,31,32) to warm
/// (Japan Color 2001 Coated: 35,25,22; Japan Color 2002 Newspaper: up to 35 apart).
pub const BLACK_INK_MAX_CHROMA: f64 = 48.0;
/// Max distance (0..255, per channel) from the K curve for a baked RGB colour to be remapped.
pub const CURVE_TOLERANCE: f64 = 2.6;
/// Max luma (0..255) of tint 1.0 for a space to count as black ink.
///
/// Uncoated/newsprint profiles without black point compensation land around 80-106; a 50 %
/// gray "Black" is still rejected.
pub const BLACK_MAX_LUMA: f64 = 120.0;
/// Min luma (0..255) of tint 0.0 (paper); absolute colorimetric renders bake in off-white.
pub const PAPER_MIN_LUMA: f64 = 190.0;
/// Colorants allowed to be treated as black; anything else is reported, never patched.
pub const BLACK_COLORANTS: [&str; 3] = ["/Black", "/All", "/None"];
/// Rec. 601 luma weights, matching the reference implementation.
pub const LUMA: [f64; 3] = [0.299, 0.587, 0.114];
/// Samples taken along a 1-input tint transform.
pub const CURVE_SAMPLES: usize = 256;
/// Samples per axis when sampling an N-input tint transform.
pub const DEVICEN_AXIS_SAMPLES: usize = 5;
/// Side of the sampled tint transform emitted for an N-input space.
pub const DEVICEN_GRID: usize = 17;

const PATCHED_VERDICT: &str = "baked RGB black: patched";

/// What an alternate / base colourspace resolves to, for our purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltKind {
    Gray,
    Rgb,
    Cmyk,
    Other,
}

impl AltKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AltKind::Gray => "gray",
            AltKind::Rgb => "rgb",
            AltKind::Cmyk => "cmyk",
            AltKind::Other => "other",
        }
    }
}

/// Where a colourspace array lives, so it can be found again after the scan.
///
/// The walker rebases a path onto a new root whenever it crosses a reference, so every path
/// stays inside one indirect object and can be mutated without re-resolving anything.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CsPath {
    pub root: ObjectId,
    pub steps: Vec<CsStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CsStep {
    /// The `/ColorSpace` entry of the dictionary at the current position.
    ColorSpace,
    /// The entry `key` of the dictionary at the current position.
    Key(Vec<u8>),
    /// Element `index` of the array at the current position.
    Index(usize),
}

impl CsPath {
    fn rooted(root: ObjectId) -> Self {
        Self {
            root,
            steps: Vec::new(),
        }
    }

    fn then(mut self, step: CsStep) -> Self {
        self.steps.push(step);
        self
    }

    /// The array this path points at, if it is one.
    pub fn read<'a>(&self, doc: &'a Document) -> Option<&'a Object> {
        let mut current = doc.objects.get(&self.root)?;
        for step in &self.steps {
            current = match step {
                CsStep::ColorSpace => current.as_dict().ok()?.get(b"ColorSpace").ok()?,
                CsStep::Key(key) => current.as_dict().ok()?.get(key).ok()?,
                CsStep::Index(index) => current.as_array().ok()?.get(*index)?,
            };
        }
        Some(current)
    }

    /// The same array, mutably. Only valid when the walker kept the path inside one object.
    pub fn write<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Object> {
        let mut current = doc.objects.get_mut(&self.root)?;
        for step in &self.steps {
            current = match step {
                CsStep::ColorSpace => current.as_dict_mut().ok()?.get_mut(b"ColorSpace").ok()?,
                CsStep::Key(key) => current.as_dict_mut().ok()?.get_mut(key).ok()?,
                CsStep::Index(index) => current.as_array_mut().ok()?.get_mut(*index)?,
            };
        }
        Some(current)
    }
}

/// The alternate (or base) colourspace family.
pub fn alt_kind(doc: &Document, alt: &Object) -> AltKind {
    let Some(alt) = look_through(doc, alt) else {
        return AltKind::Other;
    };

    if let Some(name) = name_text(alt) {
        return match name.as_str() {
            "DeviceGray" => AltKind::Gray,
            "DeviceRGB" => AltKind::Rgb,
            "DeviceCMYK" => AltKind::Cmyk,
            _ => AltKind::Other,
        };
    }

    let Object::Array(items) = alt else {
        return AltKind::Other;
    };
    if items.len() < 2 {
        return AltKind::Other;
    }

    match name_text(&items[0]).as_deref() {
        Some("ICCBased") => {
            let Some(stream) = look_through(doc, &items[1]) else {
                return AltKind::Other;
            };
            let dict = match stream {
                Object::Stream(stream) => &stream.dict,
                Object::Dictionary(dict) => dict,
                _ => return AltKind::Other,
            };
            match super::dict_number(dict, b"N") {
                Some(1.0) => AltKind::Gray,
                Some(3.0) => AltKind::Rgb,
                Some(4.0) => AltKind::Cmyk,
                _ => AltKind::Other,
            }
        }
        Some("CalRGB") => AltKind::Rgb,
        Some("CalGray") => AltKind::Gray,
        _ => AltKind::Other,
    }
}

/// The family name (`Separation` / `DeviceN`) and its colorants, slash-prefixed like the
/// report shows them.
pub fn separation_parts(doc: &Document, cs: &Object) -> Option<(String, Vec<String>)> {
    let array = cs.as_array().ok()?;
    if array.len() < 4 {
        return None;
    }

    match name_text(&array[0]).as_deref() {
        Some("Separation") => Some(("Separation".to_string(), vec![object_text(doc, &array[1])?])),
        Some("DeviceN") => {
            let names = look_through(doc, &array[1])?.as_array().ok()?;
            let colorants = names
                .iter()
                .map(|name| object_text(doc, name))
                .collect::<Option<Vec<_>>>()?;
            Some(("DeviceN".to_string(), colorants))
        }
        _ => None,
    }
}

/// A colorant name, rendered the way `str()` renders a `pikepdf.Name`.
fn object_text(doc: &Document, object: &Object) -> Option<String> {
    let object = look_through(doc, object)?;
    if let Some(name) = name_text(object) {
        return Some(format!("/{name}"));
    }
    match object {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

/// The FunctionType of a tint transform, including the array form, as report text.
pub fn fn_types(doc: &Document, function: &Object) -> String {
    let Some(function) = look_through(doc, function) else {
        return "?".to_string();
    };
    match function {
        Object::Array(items) => {
            let parts: Vec<String> = items.iter().map(|item| fn_types(doc, item)).collect();
            format!("[{}]", parts.join(","))
        }
        Object::Stream(stream) => stream
            .dict
            .get(b"FunctionType")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .map_or_else(|| "?".to_string(), |value| value.to_string()),
        Object::Dictionary(dict) => dict
            .get(b"FunctionType")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .map_or_else(|| "?".to_string(), |value| value.to_string()),
        _ => "?".to_string(),
    }
}

/// Every `Separation` / `DeviceN` array reachable as an indirect object or from a
/// `/ColorSpace` entry, deduped.
pub fn iter_colourspaces(doc: &Document) -> Vec<CsPath> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();

    for (id, object) in doc.objects.iter() {
        match object {
            Object::Array(_) => visit(doc, CsPath::rooted(*id), &mut found, &mut seen),
            Object::Dictionary(_) | Object::Stream(_) => {
                walk_color_space(doc, *id, &mut found, &mut seen);
            }
            _ => {}
        }
    }

    found
}

/// Visits `/ColorSpace` on the dictionary (or stream) at `id`, and on an inline `/Resources`
/// dictionary hanging off it.
fn walk_color_space(
    doc: &Document,
    id: ObjectId,
    found: &mut Vec<CsPath>,
    seen: &mut HashSet<CsPath>,
) {
    let Some(object) = doc.objects.get(&id) else {
        return;
    };
    let dict = match object {
        Object::Dictionary(dict) => dict,
        Object::Stream(stream) => &stream.dict,
        _ => return,
    };

    if let Ok(entry) = dict.get(b"ColorSpace") {
        visit_color_space_entry(doc, CsPath::rooted(id), entry, found, seen);
    }

    // A page or form whose resources are inline is not an object of its own, so the
    // reference implementation never sees it. Walking in here costs one dictionary lookup
    // and covers files that store resources on the page directly.
    if let Ok(Object::Dictionary(resources)) = dict.get(b"Resources")
        && let Ok(entry) = resources.get(b"ColorSpace")
    {
        visit_color_space_entry(
            doc,
            CsPath::rooted(id).then(CsStep::Key(b"Resources".to_vec())),
            entry,
            found,
            seen,
        );
    }
}

/// Visits one `/ColorSpace` value, rebasing the path when it is a reference.
fn visit_color_space_entry(
    doc: &Document,
    container: CsPath,
    entry: &Object,
    found: &mut Vec<CsPath>,
    seen: &mut HashSet<CsPath>,
) {
    // A reference can point either at the array itself or at a `/ColorSpace` dictionary.
    if let Object::Reference(id) = entry {
        if let Some(target) = doc.objects.get(id) {
            match target {
                Object::Array(_) => {
                    visit(doc, CsPath::rooted(*id), found, seen);
                    return;
                }
                Object::Dictionary(nested) => {
                    for key in nested
                        .iter()
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>()
                    {
                        let Ok(value) = nested.get(&key) else {
                            continue;
                        };
                        let path = match value {
                            Object::Reference(value_id) => CsPath::rooted(*value_id),
                            _ => CsPath::rooted(*id).then(CsStep::Key(key)),
                        };
                        visit(doc, path, found, seen);
                    }
                    return;
                }
                _ => return,
            }
        }
        return;
    }

    match entry {
        Object::Array(_) => {
            visit(doc, container.then(CsStep::ColorSpace), found, seen);
        }
        Object::Dictionary(nested) => {
            for key in nested
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
            {
                let Ok(value) = nested.get(&key) else {
                    continue;
                };
                let path = match value {
                    Object::Reference(id) => CsPath::rooted(*id),
                    _ => container
                        .clone()
                        .then(CsStep::ColorSpace)
                        .then(CsStep::Key(key)),
                };
                visit(doc, path, found, seen);
            }
        }
        _ => {}
    }
}

fn visit(doc: &Document, path: CsPath, found: &mut Vec<CsPath>, seen: &mut HashSet<CsPath>) {
    let Some(object) = path.read(doc) else {
        return;
    };
    let Some(object) = look_through(doc, object) else {
        return;
    };
    if !matches!(object, Object::Array(_)) {
        return;
    }
    if !seen.insert(path.clone()) {
        return;
    }

    let Ok(array) = object.as_array() else {
        return;
    };

    if separation_parts(doc, object).is_some() {
        found.push(path.clone());
        // Recurse into the alternate: an Indexed base can itself be a separation.
        visit(doc, child_path(doc, &path, 2, array), found, seen);
    } else if array.len() == 4 && name_text(&array[0]).as_deref() == Some("Indexed") {
        visit(doc, child_path(doc, &path, 1, array), found, seen);
    }
}

/// The path of element `index`, rebased when that element is a reference.
fn child_path(doc: &Document, parent: &CsPath, index: usize, array: &[Object]) -> CsPath {
    match array.get(index) {
        Some(Object::Reference(id)) if doc.objects.contains_key(id) => CsPath::rooted(*id),
        _ => parent.clone().then(CsStep::Index(index)),
    }
}

/// The classification of one colourspace, as reported to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceInfo {
    /// `Separation` or `DeviceN`.
    pub kind: String,
    /// Colorant names, slash-prefixed.
    pub colorants: Vec<String>,
    /// `gray`, `rgb`, `cmyk` or `other`.
    pub alternate: String,
    /// FunctionType text, e.g. `2` or `[0,2]`.
    pub fn_types: String,
    pub verdict: String,
    /// The tint transform's output at tint 1.0, in 0..255.
    pub black_ink: Option<Vec<i64>>,
    pub max_spread: Option<f64>,
}

impl SpaceInfo {
    /// One line, used by `--dry-run` and by the summary.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} {} -> {} (FT {}): {}",
            self.kind,
            self.colorants.join(" "),
            self.alternate,
            self.fn_types,
            self.verdict
        );
        if let Some(black) = &self.black_ink {
            let values: Vec<String> = black.iter().map(i64::to_string).collect();
            text.push_str(&format!(" [ink {}]", values.join(",")));
        }
        if let Some(spread) = self.max_spread {
            text.push_str(&format!(" [spread {spread}]"));
        }
        text
    }
}

/// The result of judging one colourspace.
#[derive(Debug, Clone)]
pub struct Classified {
    pub info: SpaceInfo,
    /// The K -> RGB table in 0..255, present only for patchable 1-input spaces.
    pub curve: Option<SampleMatrix>,
    pub patchable: bool,
}

/// Judges a colourspace array by evaluating its tint transform.
pub fn classify(doc: &Document, cs: &Object, include_cmyk: bool) -> Classified {
    let mut info = SpaceInfo {
        kind: String::new(),
        colorants: Vec::new(),
        alternate: String::new(),
        fn_types: String::new(),
        verdict: String::new(),
        black_ink: None,
        max_spread: None,
    };
    let reject = |info: SpaceInfo, verdict: &str| Classified {
        info: SpaceInfo {
            verdict: verdict.to_string(),
            ..info
        },
        curve: None,
        patchable: false,
    };

    let Some((family, colorants)) = separation_parts(doc, cs) else {
        return reject(info, "not a Separation/DeviceN array");
    };
    let array = match cs.as_array() {
        Ok(array) => array,
        Err(_) => return reject(info, "not a Separation/DeviceN array"),
    };

    let alternate = alt_kind(doc, &array[2]);
    info.kind = family;
    info.colorants = colorants;
    info.alternate = alternate.as_str().to_string();
    info.fn_types = fn_types(doc, &array[3]);

    match alternate {
        AltKind::Gray => return reject(info, "gray alternate: no change (render with ICC off)"),
        AltKind::Cmyk if !include_cmyk => {
            return reject(info, "cmyk alternate: no change (render with ICC off)");
        }
        AltKind::Cmyk => {}
        AltKind::Rgb => {}
        AltKind::Other => {
            return reject(info, "non-RGB/CMYK/gray alternate: left untouched");
        }
    }

    let inputs = info.colorants.len();
    if inputs > 3 {
        return reject(info, &format!("{inputs} inputs: skipped (max 3)"));
    }

    let (xs, rows) = grid(inputs);
    let evaluated = match eval_function(doc, &array[3], &xs, rows) {
        Ok(matrix) => matrix,
        Err(error) => {
            let detail = error_text(&error);
            return reject(
                info,
                &format!("tint transform not evaluable ({detail}): skipped"),
            );
        }
    };

    match alternate {
        AltKind::Rgb if evaluated.cols != 3 => {
            let cols = evaluated.cols;
            return reject(
                info,
                &format!("tint transform returns {cols} values for an RGB alternate: skipped"),
            );
        }
        AltKind::Cmyk if evaluated.cols != 4 => {
            let cols = evaluated.cols;
            return reject(
                info,
                &format!("tint transform returns {cols} values for a CMYK alternate: skipped"),
            );
        }
        _ => {}
    }

    let rgb = evaluated.scaled(255.0);
    // For a CMYK alternate only C, M and Y decide whether this is a neutral plate.
    let spread = match alternate {
        AltKind::Cmyk => rgb
            .data
            .chunks_exact(4)
            .fold(0.0_f64, |worst, row| worst.max(channel_spread(&row[..3]))),
        _ => rgb
            .data
            .chunks_exact(rgb.cols.max(1))
            .fold(0.0_f64, |worst, row| worst.max(channel_spread(row))),
    };
    info.max_spread = Some(round_1(spread));

    let black = match eval_function(doc, &array[3], &vec![1.0; inputs], 1) {
        Ok(matrix) => matrix,
        Err(error) => {
            let detail = error_text(&error);
            return reject(
                info,
                &format!("tint transform not evaluable ({detail}): skipped"),
            );
        }
    };
    info.black_ink = Some(
        black
            .row(0)
            .iter()
            .map(|value| (value * 255.0).round() as i64)
            .collect(),
    );

    if !info
        .colorants
        .iter()
        .all(|name| BLACK_COLORANTS.contains(&name.as_str()))
        || info.colorants.iter().all(|name| name == "/None")
    {
        return reject(info, "colorant is not Black/All: left untouched");
    }

    if spread > BLACK_INK_MAX_CHROMA {
        return reject(
            info,
            "too saturated for black ink (real colour): left untouched",
        );
    }

    // Paper end and black end, measured on the transform's own output.
    let paper = match eval_function(doc, &array[3], &vec![0.0; inputs], 1) {
        Ok(matrix) => matrix,
        Err(error) => {
            let detail = error_text(&error);
            return reject(
                info,
                &format!("tint transform not evaluable ({detail}): skipped"),
            );
        }
    };
    let paper_luma = match alternate {
        AltKind::Cmyk => (1.0 - paper.at(0, 3)) * 255.0,
        _ => luma_of(paper.row(0), 255.0),
    };
    let ink_luma = match alternate {
        AltKind::Cmyk => (1.0 - black.at(0, 3)) * 255.0,
        _ => luma_of(black.row(0), 255.0),
    };

    if paper_luma < PAPER_MIN_LUMA || ink_luma > BLACK_MAX_LUMA {
        return reject(
            info,
            &format!(
                "not paper->black (tint0 luma {paper_luma:.0}, tint1 luma {ink_luma:.0}): left untouched"
            ),
        );
    }

    let luma: Vec<f64> = rgb
        .data
        .chunks_exact(rgb.cols.max(1))
        .map(|row| match alternate {
            AltKind::Cmyk => (1.0 - row[3]) * 255.0,
            _ => luma_of(row, 1.0),
        })
        .collect();
    let monotone = match alternate {
        AltKind::Cmyk => evaluated
            .data
            .chunks_exact(4)
            .map(|row| row[3])
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[1] >= pair[0] - 1.0 / 255.0),
        _ => luma.windows(2).all(|pair| pair[1] - pair[0] <= 1.0),
    };
    if inputs == 1 && !monotone {
        return reject(info, "not monotone: left untouched");
    }

    // A CMYK plate is expressed as the grey it renders as, so everything downstream can
    // stay RGB-shaped.
    let curve = match alternate {
        AltKind::Cmyk => {
            let mut table = SampleMatrix::new(rgb.rows, 3);
            for row in 0..rgb.rows {
                // The K column is still 0..1 here; `rgb` is already scaled to 0..255.
                let grey = (1.0 - evaluated.at(row, 3)) * 255.0;
                for col in 0..3 {
                    table.data[row * 3 + col] = grey;
                }
            }
            Some(table)
        }
        _ => Some(rgb),
    };

    let curve = if inputs == 1 { curve } else { None };
    Classified {
        info: SpaceInfo {
            verdict: PATCHED_VERDICT.to_string(),
            ..info
        },
        curve,
        patchable: true,
    }
}

impl SampleMatrix {
    fn scaled(&self, factor: f64) -> SampleMatrix {
        SampleMatrix {
            rows: self.rows,
            cols: self.cols,
            data: self.data.iter().map(|value| value * factor).collect(),
        }
    }
}

/// Rec. 601 luma of a rendition, optionally scaled (255.0 for 0..255 tables).
pub fn luma_of(row: &[f64], factor: f64) -> f64 {
    row.iter()
        .zip(LUMA.iter())
        .map(|(value, weight)| value * weight)
        .sum::<f64>()
        * factor
}

/// Max minus min of a rendition's channels, the measure of "how coloured" it is.
fn channel_spread(channels: &[f64]) -> f64 {
    let max = channels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = channels.iter().copied().fold(f64::INFINITY, f64::min);
    max - min
}

fn error_text(error: &FunctionError) -> String {
    error.to_string()
}

/// The input points for evaluating a tint transform with `n` inputs.
///
/// 1 input gets a dense 0..1 sweep; more get a 5-point lattice, with the last input varying
/// fastest, matching the reference implementation's grid.
pub fn grid(inputs: usize) -> (Vec<f64>, usize) {
    if inputs <= 1 {
        let xs: Vec<f64> = (0..CURVE_SAMPLES)
            .map(|index| index as f64 / (CURVE_SAMPLES - 1) as f64)
            .collect();
        return (xs, CURVE_SAMPLES);
    }

    let axis: Vec<f64> = (0..DEVICEN_AXIS_SAMPLES)
        .map(|index| index as f64 / (DEVICEN_AXIS_SAMPLES - 1) as f64)
        .collect();
    let rows = DEVICEN_AXIS_SAMPLES.pow(inputs as u32);
    let mut xs = Vec::with_capacity(rows * inputs);
    for row in 0..rows {
        for axis_index in 0..inputs {
            // The last axis varies fastest.
            let stride = DEVICEN_AXIS_SAMPLES.pow((inputs - 1 - axis_index) as u32);
            xs.push(axis[(row / stride) % DEVICEN_AXIS_SAMPLES]);
        }
    }
    (xs, rows)
}

/// The inverse of the document's own K -> RGB table.
#[derive(Debug, Clone)]
pub struct KCurve {
    /// `rows x 3` in 0..255.
    pub table: SampleMatrix,
    luma: Vec<f64>,
}

impl KCurve {
    pub fn new(table: SampleMatrix) -> Self {
        let luma = table
            .data
            .chunks_exact(table.cols.max(1))
            .map(|row| luma_of(row, 1.0))
            .collect();
        Self { table, luma }
    }

    /// `rgb` in 0..1 -> linear grey (1 - K) in 0..1, or `None` when off the curve.
    pub fn gray(&self, rgb: [f64; 3]) -> Option<f64> {
        let target = [rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0];
        let mut best = 0;
        let mut best_distance = f64::INFINITY;
        for row in 0..self.table.rows {
            let distance = (0..3)
                .map(|col| (self.table.at(row, col) - target[col]).abs())
                .fold(0.0_f64, f64::max);
            // Strict `<` keeps the first minimum, like `argmin`.
            if distance < best_distance {
                best_distance = distance;
                best = row;
            }
        }
        if best_distance > CURVE_TOLERANCE {
            return None;
        }
        let highest = (self.table.rows - 1).max(1) as f64;
        Some(1.0 - best as f64 / highest)
    }

    /// Monotone luma -> K inverse (the reference implementation's `np.interp` on a reversed
    /// axis).
    pub fn k_from_luma(&self, luma: f64) -> f64 {
        let count = self.luma.len();
        if count == 0 {
            return 0.0;
        }
        // K decreases luma, so the ascending x axis is the reversed luma array.
        let xs: Vec<f64> = self.luma.iter().rev().copied().collect();
        let ys: Vec<f64> = (0..count)
            .rev()
            .map(|index| index as f64 / (count - 1).max(1) as f64)
            .collect();

        if luma <= xs[0] {
            return ys[0];
        }
        if luma >= xs[xs.len() - 1] {
            return ys[ys.len() - 1];
        }
        for index in 1..xs.len() {
            if luma <= xs[index] {
                let span = xs[index] - xs[index - 1];
                if span <= 0.0 {
                    return ys[index - 1];
                }
                let fraction = (luma - xs[index - 1]) / span;
                return ys[index - 1] + fraction * (ys[index] - ys[index - 1]);
            }
        }
        ys[ys.len() - 1]
    }

    /// The tint transform's output at tint 1.0, in 0..255.
    pub fn black_point(&self) -> Vec<i64> {
        let last = self.table.rows.saturating_sub(1);
        self.table
            .row(last)
            .iter()
            .map(|value| value.round() as i64)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn doc() -> Document {
        Document::with_version("1.7")
    }

    fn ramp(c0: f64, c1: f64) -> Object {
        Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(c0 as f32), Object::Real(c0 as f32), Object::Real(c0 as f32)],
            "C1" => vec![Object::Real(c1 as f32), Object::Real(c1 as f32), Object::Real(c1 as f32)],
            "N" => 1,
        })
    }

    /// A neutral 1-input Separation from paper (249,251,249) to baked black (35,31,32).
    fn baked_black() -> Object {
        Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "Range" => vec![
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
            ],
            "C0" => vec![
                Object::Real(249.0 / 255.0),
                Object::Real(251.0 / 255.0),
                Object::Real(249.0 / 255.0),
            ],
            "C1" => vec![
                Object::Real(35.0 / 255.0),
                Object::Real(31.0 / 255.0),
                Object::Real(32.0 / 255.0),
            ],
            "N" => 1,
        })
    }

    #[test]
    fn a_device_name_maps_to_its_kind() {
        let doc = doc();
        assert_eq!(
            alt_kind(&doc, &Object::Name(b"DeviceRGB".to_vec())),
            AltKind::Rgb
        );
        assert_eq!(
            alt_kind(&doc, &Object::Name(b"DeviceGray".to_vec())),
            AltKind::Gray
        );
        assert_eq!(
            alt_kind(&doc, &Object::Name(b"DeviceCMYK".to_vec())),
            AltKind::Cmyk
        );
        assert_eq!(
            alt_kind(&doc, &Object::Name(b"Indexed".to_vec())),
            AltKind::Other
        );
    }

    #[test]
    fn an_indirect_alternate_is_followed() {
        let mut doc = doc();
        let target = doc.add_object(Object::Name(b"DeviceRGB".to_vec()));
        assert_eq!(
            alt_kind(&doc, &Object::Reference(target)),
            AltKind::Rgb,
            "the prototype does not follow references here, but lopdf files need it"
        );
    }

    #[test]
    fn an_icc_based_alternate_is_classified_by_its_channel_count() {
        let mut doc = doc();
        let profile = doc.add_object(Object::Stream(lopdf::Stream::new(
            dictionary! { "N" => 3 },
            Vec::new(),
        )));
        let alt = Object::Array(vec![
            Object::Name(b"ICCBased".to_vec()),
            Object::Reference(profile),
        ]);
        assert_eq!(alt_kind(&doc, &alt), AltKind::Rgb);
    }

    #[test]
    fn a_separation_and_a_devicen_both_expose_their_colorants() {
        let doc = doc();
        let separation = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            ramp(1.0, 0.0),
        ]);
        let (family, colorants) = separation_parts(&doc, &separation).expect("is a Separation");
        assert_eq!(family, "Separation");
        assert_eq!(colorants, vec!["/Black".to_string()]);

        let devicen = Object::Array(vec![
            Object::Name(b"DeviceN".to_vec()),
            Object::Array(vec![
                Object::Name(b"Black".to_vec()),
                Object::Name(b"None".to_vec()),
            ]),
            Object::Name(b"DeviceRGB".to_vec()),
            ramp(1.0, 0.0),
        ]);
        let (family, colorants) = separation_parts(&doc, &devicen).expect("is a DeviceN");
        assert_eq!(family, "DeviceN");
        assert_eq!(colorants, vec!["/Black".to_string(), "/None".to_string()]);
    }

    #[test]
    fn a_baked_rgb_black_separation_is_patchable() {
        let doc = doc();
        let cs = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            baked_black(),
        ]);
        let judged = classify(&doc, &cs, false);
        assert!(judged.patchable, "verdict was {:?}", judged.info.verdict);
        assert_eq!(judged.info.black_ink, Some(vec![35, 31, 32]));
        let curve = judged.curve.expect("1-input spaces carry a curve");
        assert_eq!(curve.rows, CURVE_SAMPLES);
        assert_eq!(curve.cols, 3);
        assert!((curve.at(0, 0) - 249.0).abs() < 1.0);
    }

    #[test]
    fn a_saturated_separation_is_rejected() {
        let doc = doc();
        let cyan = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.0), Object::Real(1.0), Object::Real(1.0)],
            "C1" => vec![Object::Real(1.0), Object::Real(0.0), Object::Real(0.0)],
            "N" => 1,
        });
        let cs = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Cyan".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            cyan,
        ]);
        let judged = classify(&doc, &cs, false);
        assert!(!judged.patchable);
        assert!(
            judged.info.verdict.contains("colorant is not Black/All"),
            "verdict was {:?}",
            judged.info.verdict
        );
    }

    #[test]
    fn a_half_grey_black_is_rejected_on_its_black_point() {
        let doc = doc();
        let grey50 = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.98), Object::Real(0.98), Object::Real(0.98)],
            "C1" => vec![Object::Real(0.5), Object::Real(0.5), Object::Real(0.5)],
            "N" => 1,
        });
        let cs = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            grey50,
        ]);
        let judged = classify(&doc, &cs, false);
        assert!(!judged.patchable);
        assert!(
            judged.info.verdict.contains("not paper->black"),
            "verdict was {:?}",
            judged.info.verdict
        );
    }

    #[test]
    fn cmyk_alternates_are_left_alone_unless_asked() {
        let mut doc = doc();
        let k_plate = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "Range" => vec![
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
            ],
            // Only K moves: a pure K plate.
            "C0" => vec![
                Object::Real(0.0), Object::Real(0.0),
                Object::Real(0.0), Object::Real(0.0),
            ],
            "C1" => vec![
                Object::Real(0.0), Object::Real(0.0),
                Object::Real(0.0), Object::Real(1.0),
            ],
            "N" => 1,
        });
        let id = doc.add_object(k_plate);
        let cs = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceCMYK".to_vec()),
            Object::Reference(id),
        ]);

        let plain = classify(&doc, &cs, false);
        assert!(!plain.patchable);
        assert!(plain.info.verdict.contains("cmyk alternate"));

        let included = classify(&doc, &cs, true);
        assert!(
            included.patchable,
            "verdict was {:?}",
            included.info.verdict
        );
        let curve = included.curve.expect("1-input spaces carry a curve");
        // t = 0 is paper (white), t = 1 is full K (black).
        assert!((curve.at(0, 0) - 255.0).abs() < 1.0);
        assert!(curve.at(CURVE_SAMPLES - 1, 0).abs() < 1.0);
    }

    #[test]
    fn a_one_input_grid_sweeps_the_whole_range() {
        let (xs, rows) = grid(1);
        assert_eq!(rows, CURVE_SAMPLES);
        assert_eq!(xs.len(), CURVE_SAMPLES);
        assert!((xs[0] - 0.0).abs() < 1e-9);
        assert!((xs[CURVE_SAMPLES - 1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_two_input_grid_varies_its_last_input_fastest() {
        let (xs, rows) = grid(2);
        assert_eq!(rows, 25);
        // The second input sweeps a whole row before the first one moves.
        assert_eq!((xs[0], xs[1]), (0.0, 0.0));
        assert_eq!((xs[2], xs[3]), (0.0, 0.25));
        assert_eq!((xs[8], xs[9]), (0.0, 1.0));
        assert_eq!((xs[10], xs[11]), (0.25, 0.0));
    }

    #[test]
    fn the_curve_maps_an_on_curve_colour_back_to_grey() {
        let mut table = SampleMatrix::new(CURVE_SAMPLES, 3);
        for row in 0..CURVE_SAMPLES {
            let value = 255.0 - row as f64;
            for col in 0..3 {
                table.data[row * 3 + col] = value;
            }
        }
        let curve = KCurve::new(table);
        // Exactly the first sample.
        assert_eq!(curve.gray([1.0, 1.0, 1.0]), Some(1.0));
        // Off the curve by more than the tolerance.
        assert_eq!(curve.gray([0.0, 0.0, 1.0]), None);
    }

    #[test]
    fn the_luma_inverse_follows_the_curve() {
        let mut table = SampleMatrix::new(CURVE_SAMPLES, 3);
        for row in 0..CURVE_SAMPLES {
            let value = 255.0 - row as f64;
            for col in 0..3 {
                table.data[row * 3 + col] = value;
            }
        }
        let curve = KCurve::new(table);
        assert!((curve.k_from_luma(255.0) - 0.0).abs() < 1e-6);
        assert!((curve.k_from_luma(0.0) - 1.0).abs() < 1e-6);
        assert!((curve.k_from_luma(127.5) - 0.5).abs() < 0.01);
    }

    #[test]
    fn the_black_point_is_the_top_of_the_curve() {
        let mut table = SampleMatrix::new(3, 3);
        for (row, value) in [255.0_f64, 140.0, 35.0].iter().enumerate() {
            for col in 0..3 {
                table.data[row * 3 + col] = *value;
            }
        }
        assert_eq!(KCurve::new(table).black_point(), vec![35, 35, 35]);
    }

    #[test]
    fn the_scan_finds_a_separation_hung_off_a_resource_dictionary() {
        let mut doc = doc();
        let separation = Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            baked_black(),
        ]);
        let resource_id = doc.add_object(dictionary! {
            "ColorSpace" => dictionary! { "CS0" => separation },
        });
        let _ = resource_id;
        let found = iter_colourspaces(&doc);
        assert_eq!(found.len(), 1, "the inline Separation should be found");
        let array = found[0].read(&doc).expect("path should resolve");
        assert!(matches!(array, Object::Array(_)));
    }

    #[test]
    fn the_scan_finds_an_indirect_separation_array() {
        let mut doc = doc();
        let id = doc.add_object(Object::Array(vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Black".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            baked_black(),
        ]));
        let found = iter_colourspaces(&doc);
        assert_eq!(found, vec![CsPath::rooted(id)]);
    }

    #[test]
    fn the_scan_reaches_resources_stored_inline_on_a_page() {
        let mut doc = doc();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Resources" => dictionary! {
                "ColorSpace" => dictionary! {
                    "CS0" => Object::Array(vec![
                        Object::Name(b"Separation".to_vec()),
                        Object::Name(b"Black".to_vec()),
                        Object::Name(b"DeviceRGB".to_vec()),
                        baked_black(),
                    ]),
                },
            },
        });
        let found = iter_colourspaces(&doc);
        assert_eq!(
            found,
            vec![
                CsPath::rooted(page)
                    .then(CsStep::Key(b"Resources".to_vec()))
                    .then(CsStep::ColorSpace)
                    .then(CsStep::Key(b"CS0".to_vec()))
            ]
        );
        assert!(found[0].read(&doc).is_some(), "the path should resolve");
    }
}
