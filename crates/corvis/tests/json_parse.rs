//! `corvis::http::json_parse` against a table generated from real Node.js (V8) output
//! (`JSON.parse` results and error messages; see `fixtures/v8_json_parse.json`).

use serde_json::Value;

#[test]
fn json_parse_matches_v8() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/v8_json_parse.json")).unwrap();
    let cases = fixture["parse"].as_array().unwrap();
    assert!(cases.len() > 1000);
    let mut failures = Vec::new();
    for case in cases {
        let input = case[0].as_str().unwrap();
        let ok = case[1].as_bool().unwrap();
        let expected = case[2].as_str().unwrap();
        let got = match corvis::http::json_parse::parse(input) {
            Ok(v) => (true, serde_json::to_string(&v).unwrap()),
            Err(e) => (false, e.message),
        };
        let want = if ok {
            // Compare through serde so number formatting is not part of the comparison.
            let v: Value = serde_json::from_str(expected).unwrap();
            (true, serde_json::to_string(&v).unwrap())
        } else {
            (false, expected.to_string())
        };
        if got != want {
            failures.push(format!("{input:?}\n   want {want:?}\n    got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}
