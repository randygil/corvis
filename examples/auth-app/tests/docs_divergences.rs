//! The migration page lists every deliberate difference from the original libraries, and it is built from the `divergence`
//! marks of the recorded fixtures (`crates/corvis/tests/fixtures/auth/*.json`). This test fails when a fixture gets a mark
//! that the page does not cover, and when the page keeps a marker for a mark that no longer exists.
//!
//! Each row of the page carries an HTML comment, `<!-- divergence: <fixture>: <first 40 characters of the reason> -->`, so
//! the check does not depend on how the row is worded or translated.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use corvis::serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn collect(file: &str, value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            let reason = match map.get("divergence") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Object(d)) => {
                    Some(d.get("reason").and_then(Value::as_str).map_or_else(|| value_text(&map["divergence"]), str::to_string))
                }
                _ => None,
            };
            if let Some(reason) = reason {
                out.insert(format!("{file}: {}", reason.chars().take(40).collect::<String>()));
            }
            map.values().for_each(|v| collect(file, v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| collect(file, v, out)),
        _ => {}
    }
}

fn value_text(value: &Value) -> String {
    value.to_string()
}

#[test]
fn every_fixture_divergence_has_a_row_on_the_migration_page() {
    let dir = root().join("crates/corvis/tests/fixtures/auth");
    let mut marks = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.starts_with("package") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        collect(&name, &corvis::serde_json::from_str(&text).unwrap(), &mut marks);
    }
    assert!(marks.len() >= 20, "the fixtures were not read: {marks:?}");

    let page = std::fs::read_to_string(root().join("docs/recipes/passport-migration.md")).unwrap().replace("\r\n", "\n");
    let markers: BTreeSet<String> =
        page.split("<!-- divergence: ").skip(1).filter_map(|rest| rest.split_once(" -->").map(|(key, _)| key.to_string())).collect();

    let missing: Vec<_> = marks.difference(&markers).collect();
    assert!(
        missing.is_empty(),
        "fixture divergences with no row on docs/recipes/passport-migration.md (add a row with the marker):\n{missing:#?}"
    );
    let stale: Vec<_> = markers.difference(&marks).collect();
    assert!(stale.is_empty(), "markers on the page for divergences that no fixture marks any more:\n{stale:#?}");
}
