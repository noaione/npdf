//! PDF function evaluation, ISO 32000-1 §7.10.
//!
//! Only what tint transforms use: FunctionType 0 (sampled), 2 (exponential
//! interpolation), 3 (stitching) and 4 (PostScript calculator), plus the array-of-functions
//! form whose results are concatenated.
//!
//! Every entry point takes a whole *set* of input rows and returns a whole matrix, because
//! the callers always want a curve or a grid rather than one point. That is also what keeps
//! the PostScript interpreter affordable: it is re-run per row but parsed once.

use lopdf::{Document, Object, Stream};

use super::{dict_number, dict_numbers, look_through};

/// Max error paths a PostScript program may walk before we give up on it.
///
/// A tint transform is a straight-line computation; anything that loops is either malicious
/// or broken, and either way it must not hang the export.
const PS_STEP_LIMIT: usize = 1_000_000;

#[derive(Debug, thiserror::Error)]
pub enum FunctionError {
    #[error("object is not a function")]
    NotAFunction,
    #[error("reference does not resolve")]
    DanglingReference,
    #[error("a function dictionary is missing /{0}")]
    Missing(&'static str),
    #[error("unsupported FunctionType {0}")]
    UnsupportedType(i64),
    #[error("unsupported BitsPerSample {0}")]
    UnsupportedBits(i64),
    #[error("truncated sample table: wanted {want} values, got {got}")]
    Truncated { want: usize, got: usize },
    #[error("tint transform returns {0} values, expected {1}")]
    Arity(usize, usize),
    #[error("stream content could not be decoded")]
    Undecodable,
    #[error("PostScript program ended with an empty stack")]
    EmptyStack,
    #[error("PostScript operator {0:?} is not supported")]
    UnsupportedOperator(String),
    #[error("PostScript program runs longer than {PS_STEP_LIMIT} steps")]
    ProgramTooLong,
}

impl From<lopdf::Error> for FunctionError {
    fn from(_: lopdf::Error) -> Self {
        FunctionError::Undecodable
    }
}

/// Row-major matrix of evaluated samples: `rows` input points times `cols` outputs.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleMatrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl SampleMatrix {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![0.0; rows * cols],
        }
    }

    pub fn from_rows(rows: Vec<Vec<f64>>) -> Self {
        let row_count = rows.len();
        let cols = rows.first().map_or(0, Vec::len);
        let mut data = Vec::with_capacity(row_count * cols);
        for row in rows {
            data.extend(row);
        }
        Self {
            rows: row_count,
            cols,
            data,
        }
    }

    pub fn at(&self, row: usize, col: usize) -> f64 {
        self.data[row * self.cols + col]
    }

    pub fn row(&self, row: usize) -> &[f64] {
        &self.data[row * self.cols..(row + 1) * self.cols]
    }

    /// Appends the columns of `other` to the right, keeping this matrix' row count.
    fn extend_columns(&mut self, other: SampleMatrix) -> Result<(), FunctionError> {
        if other.rows != self.rows {
            return Err(FunctionError::Arity(other.rows, self.rows));
        }
        if self.cols == 0 {
            *self = other;
            return Ok(());
        }
        let mut data = Vec::with_capacity(self.rows * (self.cols + other.cols));
        for row in 0..self.rows {
            data.extend_from_slice(self.row(row));
            data.extend_from_slice(other.row(row));
        }
        self.cols += other.cols;
        self.data = data;
        Ok(())
    }
}

/// Evaluates a PDF function on `rows` input points.
///
/// `xs` holds `rows * n_in` inputs (0..=1 in practice, clipped to `/Domain` here),
/// row-major, and the result has one row per input point.
pub fn eval_function(
    doc: &Document,
    func: &Object,
    xs: &[f64],
    rows: usize,
) -> Result<SampleMatrix, FunctionError> {
    let n_in = xs.len().checked_div(rows).unwrap_or(0);
    eval_inner(doc, func, xs, rows, n_in)
}

fn eval_inner(
    doc: &Document,
    func: &Object,
    xs: &[f64],
    rows: usize,
    n_in: usize,
) -> Result<SampleMatrix, FunctionError> {
    let func = look_through(doc, func).ok_or(FunctionError::DanglingReference)?;

    match func {
        // The array form is a set of 1-output functions whose results are concatenated.
        Object::Array(items) => {
            let mut out = SampleMatrix::new(rows, 0);
            for item in items {
                out.extend_columns(eval_inner(doc, item, xs, rows, n_in)?)?;
            }
            Ok(out)
        }
        Object::Stream(stream) => eval_dict(doc, &stream.dict, Some(stream), xs, rows, n_in),
        Object::Dictionary(dict) => eval_dict(doc, dict, None, xs, rows, n_in),
        _ => Err(FunctionError::NotAFunction),
    }
}

fn eval_dict(
    doc: &Document,
    dict: &lopdf::Dictionary,
    stream: Option<&Stream>,
    xs: &[f64],
    rows: usize,
    n_in: usize,
) -> Result<SampleMatrix, FunctionError> {
    let function_type =
        dict_number(dict, b"FunctionType").ok_or(FunctionError::Missing("FunctionType"))? as i64;

    let default_domain: Vec<f64> = (0..n_in).flat_map(|_| [0.0, 1.0]).collect();
    let domain = dict_numbers(dict, b"Domain", &default_domain);
    let clipped = clip_inputs(xs, &domain, rows, n_in);

    let mut out = match function_type {
        0 => eval_sampled(dict, stream, &clipped, rows, n_in, &domain)?,
        2 => eval_exponential(dict, &clipped, rows)?,
        3 => eval_stitching(doc, dict, &clipped, rows, &domain, n_in)?,
        4 => eval_postscript(dict, stream, &clipped, rows)?,
        other => return Err(FunctionError::UnsupportedType(other)),
    };

    // `/Range` clamps every output of every type.
    let range = dict_numbers(dict, b"Range", &[]);
    for row in 0..out.rows {
        for col in 0..out.cols {
            let (Some(&min), Some(&max)) = (range.get(col * 2), range.get(col * 2 + 1)) else {
                continue;
            };
            let value = &mut out.data[row * out.cols + col];
            *value = value.clamp(min, max);
        }
    }

    Ok(out)
}

fn clip_inputs(xs: &[f64], domain: &[f64], rows: usize, n_in: usize) -> Vec<f64> {
    let mut clipped = xs.to_vec();
    for row in 0..rows {
        for col in 0..n_in {
            let (Some(&min), Some(&max)) = (domain.get(col * 2), domain.get(col * 2 + 1)) else {
                continue;
            };
            let slot = &mut clipped[row * n_in + col];
            *slot = slot.clamp(min, max);
        }
    }
    clipped
}

/// FunctionType 0: a sampled table, multilinearly interpolated.
fn eval_sampled(
    dict: &lopdf::Dictionary,
    stream: Option<&Stream>,
    xs: &[f64],
    rows: usize,
    n_in: usize,
    domain: &[f64],
) -> Result<SampleMatrix, FunctionError> {
    let Object::Array(size_items) = dict
        .get(b"Size")
        .map_err(|_| FunctionError::Missing("Size"))?
    else {
        return Err(FunctionError::Missing("Size"));
    };
    let size: Vec<usize> = size_items
        .iter()
        .filter_map(|item| match item {
            Object::Integer(v) if *v > 0 => Some(*v as usize),
            _ => None,
        })
        .collect();
    if size.len() != n_in || size.is_empty() {
        return Err(FunctionError::Arity(size.len(), n_in));
    }

    let bits =
        dict_number(dict, b"BitsPerSample").ok_or(FunctionError::Missing("BitsPerSample"))? as i64;
    if bits != 8 && bits != 16 {
        return Err(FunctionError::UnsupportedBits(bits));
    }

    let range = dict_numbers(dict, b"Range", &[]);
    let n_out = range.len() / 2;
    if n_out == 0 {
        return Err(FunctionError::Missing("Range"));
    }

    let stream = stream.ok_or(FunctionError::NotAFunction)?;
    let raw = stream.decompressed_content()?;

    let count = n_out * size.iter().product::<usize>();
    let samples: Vec<f64> = if bits == 16 {
        raw.chunks_exact(2)
            .take(count)
            .map(|pair| f64::from(u16::from_be_bytes([pair[0], pair[1]])))
            .collect()
    } else {
        raw.iter()
            .take(count)
            .map(|byte| f64::from(*byte))
            .collect()
    };
    if samples.len() < count {
        return Err(FunctionError::Truncated {
            want: count,
            got: samples.len(),
        });
    }

    // The table is stored with the first input varying fastest, so input `i` has a stride of
    // `n_out` times the product of the sizes before it.
    let strides: Vec<usize> = {
        let mut strides = Vec::with_capacity(n_in);
        let mut stride = n_out;
        for axis_size in size.iter().take(n_in) {
            strides.push(stride);
            stride *= axis_size;
        }
        strides
    };

    let default_encode: Vec<f64> = size
        .iter()
        .flat_map(|&len| [0.0, (len - 1) as f64])
        .collect();
    let encode = dict_numbers(dict, b"Encode", &default_encode);
    let decode = dict_numbers(dict, b"Decode", &range);
    if decode.len() < n_out * 2 || encode.len() < n_in * 2 {
        return Err(FunctionError::Missing("Encode"));
    }

    let max_value = ((1_u32 << bits) - 1) as f64;
    let mut out = SampleMatrix::new(rows, n_out);

    for row in 0..rows {
        // Where each input lands in its axis, and the fractional part between samples.
        let mut lower = Vec::with_capacity(n_in);
        let mut fraction = Vec::with_capacity(n_in);
        for axis in 0..n_in {
            let (d0, d1) = (domain[axis * 2], domain[axis * 2 + 1]);
            let (e0, e1) = (encode[axis * 2], encode[axis * 2 + 1]);
            let span = if d1 - d0 == 0.0 { 1.0 } else { d1 - d0 };
            let value = xs[row * n_in + axis];
            let mapped = e0 + (value - d0) * (e1 - e0) / span;
            let mapped = mapped.clamp(0.0, (size[axis] - 1) as f64);
            let floor = mapped.floor();
            lower.push(floor as usize);
            fraction.push(mapped - floor);
        }

        // Multilinear interpolation over the corners of the enclosing cell.
        let mut accumulated = vec![0.0; n_out];
        for corner in 0..(1_usize << n_in) {
            let mut weight = 1.0;
            let mut offset = 0;
            for axis in 0..n_in {
                let upper = (corner >> axis) & 1 == 1;
                let position = (lower[axis] + usize::from(upper)).min(size[axis] - 1);
                offset += position * strides[axis];
                weight *= if upper {
                    fraction[axis]
                } else {
                    1.0 - fraction[axis]
                };
            }
            if weight == 0.0 {
                continue;
            }
            for col in 0..n_out {
                accumulated[col] += weight * samples[offset + col];
            }
        }

        for col in 0..n_out {
            let min = decode[col * 2];
            let max = decode[col * 2 + 1];
            out.data[row * n_out + col] = min + accumulated[col] / max_value * (max - min);
        }
    }

    Ok(out)
}

/// FunctionType 2: `y = C0 + x^N * (C1 - C0)`.
fn eval_exponential(
    dict: &lopdf::Dictionary,
    xs: &[f64],
    rows: usize,
) -> Result<SampleMatrix, FunctionError> {
    let c0 = dict_numbers(dict, b"C0", &[0.0]);
    let c1 = dict_numbers(dict, b"C1", &[1.0]);
    let exponent = dict_number(dict, b"N").ok_or(FunctionError::Missing("N"))?;
    if c0.len() != c1.len() || c0.is_empty() {
        return Err(FunctionError::Arity(c1.len(), c0.len()));
    }

    let cols = c0.len();
    let mut out = SampleMatrix::new(rows, cols);
    for (row, value) in xs.iter().take(rows).enumerate() {
        let input = value.powf(exponent);
        for col in 0..cols {
            out.data[row * cols + col] = c0[col] + input * (c1[col] - c0[col]);
        }
    }
    Ok(out)
}

/// FunctionType 3: pick a sub-function by input range, remap through `/Encode`, delegate.
fn eval_stitching(
    doc: &Document,
    dict: &lopdf::Dictionary,
    xs: &[f64],
    rows: usize,
    domain: &[f64],
    n_in: usize,
) -> Result<SampleMatrix, FunctionError> {
    let Object::Array(functions) = dict
        .get(b"Functions")
        .map_err(|_| FunctionError::Missing("Functions"))?
    else {
        return Err(FunctionError::Missing("Functions"));
    };
    if functions.is_empty() {
        return Err(FunctionError::Missing("Functions"));
    }

    let mut bounds = vec![domain.first().copied().unwrap_or(0.0)];
    bounds.extend(dict_numbers(dict, b"Bounds", &[]));
    bounds.push(domain.get(1).copied().unwrap_or(1.0));

    let encode = dict_numbers(dict, b"Encode", &[]);
    if encode.len() < functions.len() * 2 {
        return Err(FunctionError::Missing("Encode"));
    }

    let mut out_rows = Vec::with_capacity(rows);
    for value in xs.iter().step_by(n_in.max(1)).take(rows) {
        let value = *value;
        // Everything in /Bounds less than the input, capped to the last sub-function.
        let inner = &bounds[1..bounds.len() - 1];
        let upper = inner.iter().filter(|bound| **bound <= value).count();
        let index = upper.min(functions.len() - 1);

        let (b0, b1) = (bounds[index], bounds[index + 1]);
        let (e0, e1) = (encode[index * 2], encode[index * 2 + 1]);
        let span = if b1 - b0 == 0.0 { 1.0 } else { b1 - b0 };
        let mapped = e0 + (value - b0) * (e1 - e0) / span;

        let sub = eval_function(doc, &functions[index], &[mapped], 1)?;
        out_rows.push(sub.row(0).to_vec());
    }

    Ok(SampleMatrix::from_rows(out_rows))
}

/// FunctionType 4: a PostScript calculator program, run on a stack machine.
fn eval_postscript(
    dict: &lopdf::Dictionary,
    stream: Option<&Stream>,
    xs: &[f64],
    rows: usize,
) -> Result<SampleMatrix, FunctionError> {
    let stream = stream.ok_or(FunctionError::NotAFunction)?;
    let raw = stream.decompressed_content()?;
    // Tokens are ASCII in every real tint transform, so lossy decoding cannot change them.
    let program = parse_postscript(&String::from_utf8_lossy(&raw));

    let range = dict_numbers(dict, b"Range", &[]);
    let n_out = range.len() / 2;
    if n_out == 0 {
        return Err(FunctionError::Missing("Range"));
    }

    let n_in = xs.len().checked_div(rows).unwrap_or(0);
    let mut out = SampleMatrix::new(rows, n_out);
    for row in 0..rows {
        let mut stack: Vec<PsValue> = xs[row * n_in..(row + 1) * n_in]
            .iter()
            .map(|value| PsValue::Real(*value))
            .collect();
        run_postscript(&program, &mut stack)?;
        if stack.len() < n_out {
            return Err(FunctionError::EmptyStack);
        }
        let produced = &stack[stack.len() - n_out..];
        for (col, value) in produced.iter().enumerate() {
            out.data[row * n_out + col] = value.as_real();
        }
    }

    Ok(out)
}

// ---- PostScript calculator (FunctionType 4) ----

#[derive(Debug, Clone, PartialEq)]
enum PsValue {
    Real(f64),
    Bool(bool),
    Block(Vec<PsItem>),
}

impl PsValue {
    /// The numeric reading of a value: `true`/`false` count as 1/0, as in PostScript.
    fn as_real(&self) -> f64 {
        match self {
            PsValue::Real(value) => *value,
            PsValue::Bool(flag) => f64::from(u8::from(*flag)),
            PsValue::Block(_) => 0.0,
        }
    }

    fn as_int(&self) -> i64 {
        self.as_real() as i64
    }

    /// PostScript truthiness: non-zero numbers and non-empty blocks are true.
    fn as_bool(&self) -> bool {
        match self {
            PsValue::Real(value) => *value != 0.0,
            PsValue::Bool(flag) => *flag,
            PsValue::Block(items) => !items.is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PsItem {
    Value(PsValue),
    Op(String),
}

/// Splits a PostScript calculator program into literals, operators and `{`-blocks.
fn parse_postscript(source: &str) -> Vec<PsItem> {
    // A program is usually a single block; unwrap it so `run` sees the body.
    let tokens = tokenize_postscript(source);
    let mut index = 0;
    let items = parse_block(&tokens, &mut index);
    match items.as_slice() {
        [PsItem::Value(PsValue::Block(inner))] => inner.clone(),
        _ => items,
    }
}

fn tokenize_postscript(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in source.chars() {
        match ch {
            '{' | '}' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_block(tokens: &[String], index: &mut usize) -> Vec<PsItem> {
    let mut items = Vec::new();
    while *index < tokens.len() {
        match tokens[*index].as_str() {
            "{" => {
                *index += 1;
                items.push(PsItem::Value(PsValue::Block(parse_block(tokens, index))));
            }
            "}" => {
                *index += 1;
                return items;
            }
            token => {
                *index += 1;
                items.push(match token.parse::<f64>() {
                    Ok(value) => PsItem::Value(PsValue::Real(value)),
                    Err(_) => PsItem::Op(token.to_string()),
                });
            }
        }
    }
    items
}

fn run_postscript(program: &[PsItem], stack: &mut Vec<PsValue>) -> Result<(), FunctionError> {
    let mut budget = PS_STEP_LIMIT;

    fn execute(
        program: &[PsItem],
        stack: &mut Vec<PsValue>,
        budget: &mut usize,
    ) -> Result<(), FunctionError> {
        for item in program {
            *budget = budget.checked_sub(1).ok_or(FunctionError::ProgramTooLong)?;

            let PsItem::Op(operator) = item else {
                match item {
                    PsItem::Value(value) => stack.push(value.clone()),
                    PsItem::Op(_) => unreachable!(),
                }
                continue;
            };

            match operator.as_str() {
                "if" => {
                    let block = pop(stack)?;
                    if pop(stack)?.as_bool()
                        && let PsValue::Block(inner) = block
                    {
                        execute(&inner, stack, budget)?;
                    }
                }
                "ifelse" => {
                    let otherwise = pop(stack)?;
                    let then = pop(stack)?;
                    let condition = pop(stack)?.as_bool();
                    let branch = if condition { then } else { otherwise };
                    if let PsValue::Block(inner) = branch {
                        execute(&inner, stack, budget)?;
                    }
                }
                "roll" => {
                    let shift = pop(stack)?.as_int();
                    let count = pop(stack)?.as_int();
                    if count > 0 {
                        roll(stack, count as usize, shift);
                    }
                }
                "index" => {
                    let depth = pop(stack)?.as_int();
                    let position = stack
                        .len()
                        .checked_sub(1 + depth.max(0) as usize)
                        .ok_or(FunctionError::EmptyStack)?;
                    stack.push(stack[position].clone());
                }
                "dup" => {
                    let top = pop(stack)?;
                    stack.push(top.clone());
                    stack.push(top);
                }
                "pop" => {
                    pop(stack)?;
                }
                "exch" => {
                    let last = pop(stack)?;
                    let previous = pop(stack)?;
                    stack.push(last);
                    stack.push(previous);
                }
                "copy" => {
                    let count = pop(stack)?.as_int().max(0) as usize;
                    let start = stack
                        .len()
                        .checked_sub(count)
                        .ok_or(FunctionError::EmptyStack)?;
                    let copied: Vec<PsValue> = stack[start..].to_vec();
                    stack.extend(copied);
                }
                "true" => stack.push(PsValue::Bool(true)),
                "false" => stack.push(PsValue::Bool(false)),
                unary if unary_operator(unary).is_some() => {
                    let value = pop(stack)?;
                    stack.push(unary_operator(unary).expect("checked")(value));
                }
                binary if binary_operator(binary).is_some() => {
                    let right = pop(stack)?;
                    let left = pop(stack)?;
                    stack.push(binary_operator(binary).expect("checked")(left, right));
                }
                other => return Err(FunctionError::UnsupportedOperator(other.to_string())),
            }
        }
        Ok(())
    }

    execute(program, stack, &mut budget)
}

fn pop(stack: &mut Vec<PsValue>) -> Result<PsValue, FunctionError> {
    stack.pop().ok_or(FunctionError::EmptyStack)
}

/// PostScript `roll`: rotates the top `count` entries by `shift`.
fn roll(stack: &mut [PsValue], count: usize, shift: i64) {
    if count == 0 || count > stack.len() {
        return;
    }
    let start = stack.len() - count;
    let shift = shift.rem_euclid(count as i64) as usize;
    if shift == 0 {
        return;
    }
    let segment: Vec<PsValue> = stack[start..].to_vec();
    let split = count - shift;
    for (offset, value) in segment[split..]
        .iter()
        .chain(segment[..split].iter())
        .enumerate()
    {
        stack[start + offset] = value.clone();
    }
}

type UnaryOp = fn(PsValue) -> PsValue;
type BinaryOp = fn(PsValue, PsValue) -> PsValue;

fn unary_operator(name: &str) -> Option<UnaryOp> {
    Some(match name {
        "neg" => |a: PsValue| PsValue::Real(-a.as_real()),
        "abs" => |a: PsValue| PsValue::Real(a.as_real().abs()),
        "sqrt" => |a: PsValue| PsValue::Real(a.as_real().max(0.0).sqrt()),
        "floor" => |a: PsValue| PsValue::Real(a.as_real().floor()),
        "ceiling" => |a: PsValue| PsValue::Real(a.as_real().ceil()),
        "round" => |a: PsValue| PsValue::Real((a.as_real() + 0.5).floor()),
        "truncate" => |a: PsValue| PsValue::Real(a.as_int() as f64),
        "cvi" => |a: PsValue| PsValue::Real(a.as_int() as f64),
        "cvr" => |a: PsValue| PsValue::Real(a.as_real()),
        "not" => |a: PsValue| match a {
            PsValue::Bool(flag) => PsValue::Bool(!flag),
            other => PsValue::Real(f64::from(!(other.as_int() as u8))),
        },
        "ln" => |a: PsValue| {
            PsValue::Real(if a.as_real() > 0.0 {
                a.as_real().ln()
            } else {
                0.0
            })
        },
        "log" => |a: PsValue| {
            PsValue::Real(if a.as_real() > 0.0 {
                a.as_real().log10()
            } else {
                0.0
            })
        },
        "sin" => |a: PsValue| PsValue::Real(a.as_real().to_radians().sin()),
        "cos" => |a: PsValue| PsValue::Real(a.as_real().to_radians().cos()),
        _ => return None,
    })
}

fn binary_operator(name: &str) -> Option<BinaryOp> {
    Some(match name {
        "add" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real() + b.as_real()),
        "sub" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real() - b.as_real()),
        "mul" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real() * b.as_real()),
        "div" => |a: PsValue, b: PsValue| {
            let divisor = b.as_real();
            PsValue::Real(if divisor != 0.0 {
                a.as_real() / divisor
            } else {
                0.0
            })
        },
        "idiv" => |a: PsValue, b: PsValue| {
            let divisor = b.as_int();
            PsValue::Real(if divisor != 0 {
                (a.as_int().div_euclid(divisor)) as f64
            } else {
                0.0
            })
        },
        "mod" => |a: PsValue, b: PsValue| {
            let divisor = b.as_int();
            PsValue::Real(if divisor != 0 {
                a.as_int().rem_euclid(divisor) as f64
            } else {
                0.0
            })
        },
        "exp" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real().powf(b.as_real())),
        "atan" => |a: PsValue, b: PsValue| {
            PsValue::Real(
                a.as_real()
                    .atan2(b.as_real())
                    .to_degrees()
                    .rem_euclid(360.0),
            )
        },
        "max" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real().max(b.as_real())),
        "min" => |a: PsValue, b: PsValue| PsValue::Real(a.as_real().min(b.as_real())),
        "eq" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() == b.as_real()),
        "ne" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() != b.as_real()),
        "gt" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() > b.as_real()),
        "ge" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() >= b.as_real()),
        "lt" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() < b.as_real()),
        "le" => |a: PsValue, b: PsValue| PsValue::Bool(a.as_real() <= b.as_real()),
        // `and`/`or`/`xor` are logical on booleans and bitwise on numbers; the left operand
        // decides, which is what the reference implementation does.
        "and" => |a: PsValue, b: PsValue| match a {
            PsValue::Bool(flag) => PsValue::Bool(flag && b.as_bool()),
            other => PsValue::Real(f64::from((other.as_int() & b.as_int()) as u32)),
        },
        "or" => |a: PsValue, b: PsValue| match a {
            PsValue::Bool(flag) => PsValue::Bool(flag || b.as_bool()),
            other => PsValue::Real(f64::from((other.as_int() | b.as_int()) as u32)),
        },
        "xor" => |a: PsValue, b: PsValue| match a {
            PsValue::Bool(flag) => PsValue::Bool(flag != b.as_bool()),
            other => PsValue::Real(f64::from((other.as_int() ^ b.as_int()) as u32)),
        },
        "bitshift" => |a: PsValue, b: PsValue| {
            let shift = b.as_int();
            let value = a.as_int();
            PsValue::Real(if shift >= 0 {
                (value << shift.min(62)) as f64
            } else {
                (value >> (-shift).min(62)) as f64
            })
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn doc_with(stream: Stream) -> (Document, Object) {
        let mut doc = Document::with_version("1.7");
        let id = doc.add_object(Object::Stream(stream));
        (doc, Object::Reference(id))
    }

    fn sampled_stream(dict: lopdf::Dictionary, bytes: &[u8]) -> Stream {
        Stream::new(dict, bytes.to_vec())
    }

    #[test]
    fn a_type_2_identity_function_returns_its_input() {
        let doc = Document::with_version("1.7");
        let function = dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.0)],
            "C1" => vec![Object::Real(1.0)],
            "N" => 1,
        };
        let out = eval_function(&doc, &Object::Dictionary(function), &[0.0, 0.5, 1.0], 3)
            .expect("identity should evaluate");
        assert_eq!(out.rows, 3);
        assert_eq!(out.cols, 1);
        assert!((out.at(0, 0) - 0.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 0.5).abs() < 1e-9);
        assert!((out.at(2, 0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_type_2_inverted_ramp_is_the_grey_transform_we_emit() {
        let doc = Document::with_version("1.7");
        let function = dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(1.0)],
            "C1" => vec![Object::Real(0.0)],
            "N" => 1,
        };
        let out = eval_function(&doc, &Object::Dictionary(function), &[0.0, 0.25, 1.0], 3)
            .expect("inverted ramp should evaluate");
        assert!((out.at(0, 0) - 1.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 0.75).abs() < 1e-9);
        assert!((out.at(2, 0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn a_type_0_table_interpolates_between_its_samples() {
        // Three samples on one axis: 0, 128, 255, decoded to 0..1.
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 0,
                "Size" => vec![Object::Integer(3)],
                "BitsPerSample" => 8,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            &[0, 128, 255],
        ));
        let out =
            eval_function(&doc, &function, &[0.0, 0.5, 1.0], 3).expect("table should evaluate");
        assert!((out.at(0, 0) - 0.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 128.0 / 255.0).abs() < 1e-9);
        assert!((out.at(2, 0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_type_0_16_bit_table_reads_big_endian() {
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 0,
                "Size" => vec![Object::Integer(2)],
                "BitsPerSample" => 16,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            &[0x00, 0x00, 0xFF, 0xFF],
        ));
        let out = eval_function(&doc, &function, &[0.0, 1.0], 2).expect("table should evaluate");
        assert!((out.at(0, 0) - 0.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_two_input_table_varies_its_first_input_fastest() {
        // Size [2 2]: (0,0) (1,0) (0,1) (1,1) in storage order.
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 0,
                "Size" => vec![Object::Integer(2), Object::Integer(2)],
                "BitsPerSample" => 8,
                "Domain" => vec![
                    Object::Real(0.0), Object::Real(1.0),
                    Object::Real(0.0), Object::Real(1.0),
                ],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            &[0, 255, 0, 255],
        ));
        // Only the first input moves, so the value must go 0 -> 1 while the second stays 0.
        let out = eval_function(&doc, &function, &[0.0, 0.0, 1.0, 0.0], 2)
            .expect("2-input table should evaluate");
        assert!((out.at(0, 0) - 0.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 1.0).abs() < 1e-9);

        // Moving only the second input must not change anything.
        let out = eval_function(&doc, &function, &[0.0, 1.0], 1).expect("should evaluate");
        assert!((out.at(0, 0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn a_truncated_table_is_an_error_rather_than_a_panic() {
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 0,
                "Size" => vec![Object::Integer(3)],
                "BitsPerSample" => 8,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            &[0, 128],
        ));
        assert!(matches!(
            eval_function(&doc, &function, &[0.0], 1),
            Err(FunctionError::Truncated { .. })
        ));
    }

    #[test]
    fn a_type_3_stitch_picks_the_branch_its_input_falls_into() {
        let doc = Document::with_version("1.7");
        let low = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.0)],
            "C1" => vec![Object::Real(0.5)],
            "N" => 1,
        });
        let high = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.5)],
            "C1" => vec![Object::Real(1.0)],
            "N" => 1,
        });
        let function = dictionary! {
            "FunctionType" => 3,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            "Bounds" => vec![Object::Real(0.5)],
            "Encode" => vec![
                Object::Real(0.0), Object::Real(1.0),
                Object::Real(0.0), Object::Real(1.0),
            ],
            "Functions" => vec![low, high],
        };
        let out = eval_function(&doc, &Object::Dictionary(function), &[0.25, 0.75], 2)
            .expect("stitch should evaluate");
        // 0.25 of the way into [0, 0.5] is halfway through the low branch; 0.75 is halfway
        // through [0.5, 1], which is halfway through the high branch' own [0, 1] encode.
        assert!((out.at(0, 0) - 0.25).abs() < 1e-9, "low branch");
        assert!((out.at(1, 0) - 0.75).abs() < 1e-9, "high branch");
    }

    #[test]
    fn a_type_4_program_with_ifelse_picks_the_right_branch() {
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 4,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(2.0)],
            },
            b"{ dup 0.5 lt { 2 mul } { 2 mul 1 add } ifelse }",
        ));
        let out = eval_function(&doc, &function, &[0.25, 0.75], 2).expect("program should run");
        // Clamped to /Range, so 1.5 stays and 2.5 becomes 2.0.
        assert!((out.at(0, 0) - 0.5).abs() < 1e-9);
        assert!((out.at(1, 0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_type_4_program_can_roll_its_stack() {
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 4,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            // [x] -> [x 0 1] -> roll the top three by one -> [1 x 0] -> sub -> x.
            // Without the roll the result would be 0 - 1.
            b"{ 0 1 3 1 roll sub }",
        ));
        let out = eval_function(&doc, &function, &[0.25], 1).expect("roll should run");
        assert!((out.at(0, 0) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn an_unknown_postscript_operator_is_reported_not_guessed() {
        let (doc, function) = doc_with(sampled_stream(
            dictionary! {
                "FunctionType" => 4,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "Range" => vec![Object::Real(0.0), Object::Real(1.0)],
            },
            b"{ frobnicate }",
        ));
        assert!(matches!(
            eval_function(&doc, &function, &[0.5], 1),
            Err(FunctionError::UnsupportedOperator(_))
        ));
    }

    #[test]
    fn an_array_of_functions_concatenates_its_columns() {
        let doc = Document::with_version("1.7");
        let ramp = |c0: f64, c1: f64| {
            Object::Dictionary(dictionary! {
                "FunctionType" => 2,
                "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
                "C0" => vec![Object::Real(c0 as f32)],
                "C1" => vec![Object::Real(c1 as f32)],
                "N" => 1,
            })
        };
        let function = Object::Array(vec![ramp(1.0, 0.0), ramp(0.0, 1.0)]);
        let out = eval_function(&doc, &function, &[0.5], 1).expect("array should evaluate");
        assert_eq!(out.cols, 2);
        assert!((out.at(0, 0) - 0.5).abs() < 1e-9);
        assert!((out.at(0, 1) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn inputs_are_clipped_to_the_domain_before_evaluation() {
        let doc = Document::with_version("1.7");
        let function = Object::Dictionary(dictionary! {
            "FunctionType" => 2,
            "Domain" => vec![Object::Real(0.0), Object::Real(1.0)],
            "C0" => vec![Object::Real(0.0)],
            "C1" => vec![Object::Real(1.0)],
            "N" => 1,
        });
        let out = eval_function(&doc, &function, &[2.0, -1.0], 2).expect("should evaluate");
        assert!((out.at(0, 0) - 1.0).abs() < 1e-9);
        assert!((out.at(1, 0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn an_unsupported_function_type_is_reported() {
        let doc = Document::with_version("1.7");
        let function = Object::Dictionary(dictionary! { "FunctionType" => 7 });
        assert!(matches!(
            eval_function(&doc, &function, &[0.0], 1),
            Err(FunctionError::UnsupportedType(7))
        ));
    }
}
