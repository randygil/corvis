//! The code samples of the documentation are copied from the example, and this test fails when one drifts.
//!
//! A sample is a ```rust block whose first line is a comment that names its file (`// examples/auth-app/src/...`). `// ...`
//! on a line of its own marks code left out: the block is split there, and every part must appear in the named file, as
//! consecutive lines. Blank lines and lines that are only a comment are ignored on both sides, so a translation of the
//! comments does not matter; everything else must be the same text with the same indentation level removed (`trim`).
//!
//! Both languages are checked, and they must have the same number of samples. A page with no sample would pass vacuously,
//! so each page below has a minimum.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n")
}

/// The lines that count: trimmed, not empty, not a comment line.
fn significant(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).map(str::to_string).collect()
}

struct Sample {
    file: String,
    parts: Vec<Vec<String>>,
    line: usize,
}

fn samples(page: &str) -> Vec<Sample> {
    let mut found = Vec::new();
    let mut lines = page.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        if !line.trim_end().starts_with("```rust") {
            continue;
        }
        let mut block = Vec::new();
        for (_, l) in lines.by_ref() {
            if l.trim_end() == "```" {
                break;
            }
            block.push(l);
        }
        let Some(first) = block.iter().position(|l| !l.trim().is_empty()) else { continue };
        let Some(file) = block[first].trim().strip_prefix("// examples/") else { continue };
        let mut parts: Vec<Vec<String>> = vec![Vec::new()];
        for l in &block[first + 1..] {
            if l.trim() == "// ..." {
                parts.push(Vec::new());
            } else {
                parts.last_mut().unwrap().push((*l).to_string());
            }
        }
        let parts = parts.into_iter().map(|p| significant(&p.join("\n"))).filter(|p| !p.is_empty()).collect();
        found.push(Sample { file: format!("examples/{}", file.trim()), parts, line: i + 1 });
    }
    found
}

fn contains_run(haystack: &[String], needle: &[String]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Checks every sample of one page and returns how many it checked.
fn check_page(root: &Path, page_path: &str) -> usize {
    let page = read(&root.join("docs").join(page_path));
    let found = samples(&page);
    let mut problems = Vec::new();
    for sample in &found {
        let source = significant(&read(&root.join(&sample.file)));
        for (n, part) in sample.parts.iter().enumerate() {
            if !contains_run(&source, part) {
                problems.push(format!(
                    "docs/{page_path}:{} part {} of the sample for {} is not in the file:\n    {}",
                    sample.line,
                    n + 1,
                    sample.file,
                    part.join("\n    ")
                ));
            }
        }
    }
    assert!(problems.is_empty(), "samples that drifted from the example:\n{}", problems.join("\n"));
    found.len()
}

/// (page, the fewest samples it must have)
const PAGES: &[(&str, usize)] = &[
    ("security/authentication.md", 14),
    ("security/authorization.md", 3),
    ("security/authentication-security.md", 5),
    ("recipes/passport-migration.md", 2),
];

#[test]
fn the_samples_of_the_english_pages_are_in_the_example() {
    let root = repo_root();
    for (page, at_least) in PAGES {
        let english = check_page(&root, page);
        assert!(english >= *at_least, "{page} has {english} checked samples, expected at least {at_least}");
    }
}

#[test]
fn the_spanish_pages_carry_the_same_samples() {
    let root = repo_root();
    for (page, _) in PAGES {
        let english = check_page(&root, page);
        let spanish = check_page(&root, &format!("es/{page}"));
        assert_eq!(english, spanish, "{page}: the Spanish page must carry the same samples");
    }
}

#[test]
fn the_comparison_catches_a_changed_line() {
    let file = significant("fn a() {\n    one();\n    two();\n}\n");
    assert!(contains_run(&file, &significant("one();\ntwo();")));
    assert!(!contains_run(&file, &significant("one();\nthree();")));
    // Comments and blank lines do not count, so a translated comment is fine.
    assert!(contains_run(&file, &significant("fn a() {\n  // un comentario\n\n  one();")));
}
