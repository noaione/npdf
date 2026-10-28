//! PDF object plumbing shared by the `fix-*` commands.
//!
//! Everything here works on `lopdf` objects directly. There is no abstraction over the
//! document: callers get a `&Document` or `&mut Document` and reach into `doc.objects`
//! through the helpers below.

pub mod colorspace;
pub mod function;
pub mod prepress_black;

use lopdf::{Dictionary, Document, Object};

/// A `Reference` chain resolved to whatever it finally points at.
///
/// `lopdf` keeps references in the object map rather than transparently resolving them, so
/// almost every read has to go through one of these first.
pub(crate) fn dereference_reference(
    doc: &Document,
    object: (u32, u16),
) -> Result<Object, lopdf::Error> {
    match doc.get_object(object)? {
        Object::Reference(ob) => dereference_reference(doc, *ob),
        Object::Array(ob) => Ok(Object::Array(dereference_array(doc, ob)?)),
        Object::Dictionary(ob) => Ok(Object::Dictionary(dereference_dictionary(doc, ob)?)),
        other => Ok(other.clone()),
    }
}

pub(crate) fn dereference_array(
    doc: &Document,
    array: &Vec<Object>,
) -> Result<Vec<Object>, lopdf::Error> {
    let mut new_array = Vec::with_capacity(array.len());
    for item in array {
        let deref_item = match item {
            Object::Reference(ob) => dereference_reference(doc, *ob)?,
            Object::Array(array) => Object::Array(dereference_array(doc, array)?),
            Object::Dictionary(dict) => Object::Dictionary(dereference_dictionary(doc, dict)?),
            other => other.clone(),
        };
        new_array.push(deref_item);
    }
    Ok(new_array)
}

pub(crate) fn dereference_dictionary(
    doc: &Document,
    resources: &Dictionary,
) -> Result<Dictionary, lopdf::Error> {
    let mut new_dict = Dictionary::new();
    // When found Reference, dereference it
    for (key, value) in resources.iter() {
        let deref_value = match value {
            Object::Reference(ob) => dereference_reference(doc, *ob)?,
            Object::Array(arr) => Object::Array(dereference_array(doc, arr)?),
            Object::Dictionary(dict) => Object::Dictionary(dereference_dictionary(doc, dict)?),
            other => other.clone(),
        };

        new_dict.set(key.to_vec(), deref_value);
    }
    Ok(new_dict)
}

/// Helper: Resolves an Object (which might be a Reference) to a Dictionary
pub(crate) fn resolve_to_dict<'a>(
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

pub(crate) fn resolve_to_stream<'a>(
    doc: &'a Document,
    object: &'a Object,
) -> Result<&'a lopdf::Stream, lopdf::Error> {
    match object {
        Object::Stream(stream) => Ok(stream),
        Object::Reference(id) => doc.get_object(*id)?.as_stream(),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Stream or Reference",
            found: object.enum_variant(),
        }),
    }
}

pub(crate) fn resolve_to_real(object: &Object) -> Result<f32, lopdf::Error> {
    match object {
        Object::Real(val) => Ok(*val),
        Object::Integer(val) => Ok(*val as f32),
        _ => Err(lopdf::Error::ObjectType {
            expected: "Real or Integer",
            found: object.enum_variant(),
        }),
    }
}

/// The referenced object, or `None` when the reference is dangling or nested too deeply.
///
/// Unlike [`dereference_reference`] this never clones: the point is to look at an object far
/// more often than it is to own it.
pub(crate) fn look_through<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    // A reference chain longer than this is a cycle in a broken file, not a real chain.
    for _ in 0..32 {
        match current {
            Object::Reference(id) => current = doc.objects.get(id)?,
            other => return Some(other),
        }
    }
    None
}

/// A number read from a dictionary entry, in the shapes PDF actually stores numbers in.
pub(crate) fn dict_number(dict: &Dictionary, key: &[u8]) -> Option<f64> {
    match dict.get(key).ok()? {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

/// A numeric array read from a dictionary entry, with `default` when the key is absent.
pub(crate) fn dict_numbers(dict: &Dictionary, key: &[u8], default: &[f64]) -> Vec<f64> {
    let Some(Object::Array(items)) = dict.get(key).ok() else {
        return default.to_vec();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Object::Integer(v) => Some(*v as f64),
            Object::Real(v) => Some(f64::from(*v)),
            _ => None,
        })
        .collect()
}

/// A name object as text without the leading slash, e.g. `Separation`.
pub(crate) fn name_text(object: &Object) -> Option<String> {
    let name = object.as_name().ok()?;
    Some(String::from_utf8_lossy(name).into_owned())
}

/// `round(x, 1)`, matching the prototype's reporting precision.
pub(crate) fn round_1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn a_reference_chain_is_followed_to_the_end() {
        let mut doc = Document::with_version("1.7");
        let inner = doc.add_object(Object::Integer(7));
        let middle = doc.add_object(Object::Reference(inner));
        let outer = Object::Reference(middle);
        assert_eq!(
            look_through(&doc, &outer),
            Some(&Object::Integer(7)),
            "three links should resolve"
        );
    }

    #[test]
    fn a_dangling_reference_resolves_to_nothing() {
        let doc = Document::with_version("1.7");
        assert!(look_through(&doc, &Object::Reference((99, 0))).is_none());
    }

    #[test]
    fn numbers_come_back_as_floats_from_either_integer_or_real() {
        let dict = dictionary! {
            "int" => 3,
            "real" => Object::Real(2.5),
            "text" => Object::string_literal("no"),
        };
        assert_eq!(dict_number(&dict, b"int"), Some(3.0));
        assert_eq!(dict_number(&dict, b"real"), Some(2.5));
        assert_eq!(dict_number(&dict, b"text"), None);
        assert_eq!(dict_number(&dict, b"missing"), None);
    }

    #[test]
    fn a_number_array_falls_back_to_the_default() {
        let dict = dictionary! {
            "both" => vec![Object::Real(0.0), Object::Integer(1)],
        };
        assert_eq!(dict_numbers(&dict, b"both", &[9.0]), vec![0.0, 1.0]);
        assert_eq!(dict_numbers(&dict, b"absent", &[9.0]), vec![9.0]);
    }

    #[test]
    fn values_round_to_one_decimal_place() {
        assert_eq!(round_1(34.567), 34.6);
        assert_eq!(round_1(35.0), 35.0);
    }
}
