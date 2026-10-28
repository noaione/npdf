//! End-to-end checks for `npdf fix-black`.
//!
//! Each test builds its own tiny PDF, runs the real binary against it, and inspects both the
//! report and the patched file. Nothing here depends on the Python fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

use lopdf::{Document, Object, Stream, dictionary};

fn npdf() -> Command {
    Command::new(env!("CARGO_BIN_EXE_npdf"))
}

/// A directory unique to one test, so parallel tests cannot collide.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("npdf-fix-black-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// The press-profile K ramp this fixture uses: paper to U.S. Web Coated black.
fn tint_transform() -> Object {
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

/// Writes a one-page PDF whose page paints with a baked-RGB-black `Separation`.
fn write_fixture(dir: &Path, name: &str) -> PathBuf {
    let mut doc = Document::with_version("1.7");

    let contents = doc.add_object(Object::Stream(Stream::new(
        lopdf::Dictionary::new(),
        b"/CS0 cs 1 scn 10 10 180 180 re f".to_vec(),
    )));
    let separation = Object::Array(vec![
        Object::Name(b"Separation".to_vec()),
        Object::Name(b"Black".to_vec()),
        Object::Name(b"DeviceRGB".to_vec()),
        tint_transform(),
    ]);
    let resources = doc.add_object(dictionary! {
        "ColorSpace" => dictionary! { "CS0" => separation },
    });
    let pages_id = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => vec![
            Object::Integer(0), Object::Integer(0),
            Object::Integer(200), Object::Integer(200),
        ],
        "Contents" => Object::Reference(contents),
        "Resources" => Object::Reference(resources),
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page)],
            "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog));

    let path = dir.join(name);
    doc.save(&path).expect("fixture should save");
    path
}

/// Runs `npdf fix-black <args>` and returns whether it succeeded plus all its output.
fn run(args: &[&str]) -> (bool, String) {
    let output = npdf()
        .arg("fix-black")
        .args(args)
        .output()
        .expect("the npdf binary should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn a_baked_black_separation_is_patched_and_the_result_is_idempotent() {
    let dir = scratch("patched");
    let input = write_fixture(&dir, "baked.pdf");

    let (ok, report) = run(&[input.to_str().unwrap(), "--dry-run"]);
    assert!(ok, "dry run should succeed: {report}");
    assert!(
        report.contains("patched 1 colourspace(s)"),
        "dry run should report one patched colourspace: {report}"
    );
    assert!(
        report.contains("black point (35, 31, 32)"),
        "the K curve should be measured from the tint transform: {report}"
    );

    let output = dir.join("fixed.pdf");
    let (ok, report) = run(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
    assert!(ok, "patching should succeed: {report}");
    assert!(output.exists(), "the patched file should be written");

    // The alternate is now DeviceGray and the tint transform is the linear one.
    let patched = Document::load(&output).expect("patched PDF should load");
    let separation = find_separation(&patched).expect("the separation should survive");
    assert_eq!(
        separation[2].as_name().ok(),
        Some(b"DeviceGray".as_slice()),
        "the alternate should have been rewritten"
    );
    assert_eq!(
        separation.len(),
        4,
        "extra entries should have been dropped"
    );

    // The new tint transform is the linear one, stored as its own indirect object.
    let transform_id = separation[3]
        .as_reference()
        .expect("the tint transform should be indirect");
    let transform = patched
        .get_object(transform_id)
        .and_then(Object::as_dict)
        .expect("the tint transform should be a dictionary");
    assert_eq!(
        transform.get(b"FunctionType").and_then(Object::as_i64).ok(),
        Some(2),
        "a linear tint transform is a FunctionType 2"
    );

    // Running it again finds nothing to do.
    let (ok, report) = run(&[output.to_str().unwrap(), "--dry-run"]);
    assert!(ok, "the second run should succeed: {report}");
    assert!(
        report.contains("no baked RGB black"),
        "the patch should be idempotent: {report}"
    );
}

#[test]
fn a_clean_document_is_not_rewritten() {
    let dir = scratch("clean");
    let mut doc = Document::with_version("1.7");
    let contents = doc.add_object(Object::Stream(Stream::new(
        lopdf::Dictionary::new(),
        b"0.5 0.5 0.5 rg 10 10 180 180 re f".to_vec(),
    )));
    let pages_id = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => vec![
            Object::Integer(0), Object::Integer(0),
            Object::Integer(200), Object::Integer(200),
        ],
        "Contents" => Object::Reference(contents),
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page)],
            "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog));

    let input = dir.join("clean.pdf");
    doc.save(&input).expect("fixture should save");

    let output = dir.join("out.pdf");
    let (ok, report) = run(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
    assert!(ok, "the run should succeed: {report}");
    assert!(
        report.contains("Nothing to patch"),
        "nothing matched, so nothing should be written: {report}"
    );
    assert!(!output.exists(), "no output file should be created");
}

#[test]
fn an_output_path_is_required_unless_dry_running() {
    let dir = scratch("no-output");
    let input = write_fixture(&dir, "baked.pdf");

    let (ok, text) = run(&[input.to_str().unwrap()]);
    assert!(!ok, "a missing --output should fail");
    assert!(
        text.contains("--output"),
        "the error should name the missing flag: {text}"
    );
}

#[test]
fn a_non_pdf_output_path_is_rejected() {
    let dir = scratch("bad-output");
    let input = write_fixture(&dir, "baked.pdf");
    let output = dir.join("fixed.png");

    let (ok, text) = run(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
    assert!(!ok, "a .png output should be rejected");
    assert!(
        text.contains("must have a .pdf extension"),
        "the error should explain the extension rule: {text}"
    );
}

/// The first `Separation` array anywhere in the document.
///
/// Colourspaces are usually inline inside a `/ColorSpace` dictionary, so this has to walk
/// into dictionaries and arrays rather than just the object table.
fn find_separation(doc: &Document) -> Option<&Vec<Object>> {
    fn search(object: &Object) -> Option<&Vec<Object>> {
        match object {
            Object::Array(items) => {
                if items.len() >= 2 && items[0].as_name().ok() == Some(b"Separation".as_slice()) {
                    return Some(items);
                }
                items.iter().find_map(search)
            }
            Object::Dictionary(dict) => dict.iter().map(|(_, value)| value).find_map(search),
            Object::Stream(stream) => stream.dict.iter().map(|(_, value)| value).find_map(search),
            _ => None,
        }
    }

    doc.objects.values().find_map(search)
}
