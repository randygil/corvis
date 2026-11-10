//! Source checks of the authentication review: the threat model page names tests that exist, its dependency inventory
//! matches `Cargo.lock` and `deny.toml`, and the authentication modules draw their secrets from the operating system
//! generator only. Text scans, no network, no feature gate: tripwires, not proofs (an alias would slip through, a comment
//! is skipped on purpose), and the review reads the real call sites.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn read(path: &str) -> String {
    // Line endings are normalized: a checkout that converts them (`core.autocrlf`) hands the files over with CRLF.
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}")).replace("\r\n", "\n")
}

const PAGE: &str = "docs/security/authentication-threat-model.md";

// ---- the test files the page names exist ----

/// The repository paths of test files in `text`: `crates/*/tests/**.rs` and `examples/*/tests/**.rs`.
fn test_paths(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for prefix in ["crates/", "examples/"] {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            let tail = &rest[at..];
            let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || "/_.-".contains(c))).unwrap_or(tail.len());
            let candidate = &tail[..end];
            if candidate.contains("/tests/") && candidate.ends_with(".rs") {
                found.push(candidate.to_string());
            }
            rest = &tail[prefix.len()..];
        }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_test_file_the_threat_model_names_exists() {
    for page in [PAGE, "docs/es/security/authentication-threat-model.md"] {
        let text = read(page);
        let paths = test_paths(&text);
        assert!(paths.len() >= 20, "{page} names {} test files", paths.len());
        let missing: Vec<&String> = paths.iter().filter(|p| !root().join(p).is_file()).collect();
        assert!(missing.is_empty(), "{page} names test files that do not exist: {missing:?}");
    }
}

#[test]
fn the_page_references_are_parsed_as_expected() {
    let paths =
        test_paths("see `crates/corvis/tests/auth_jwt.rs` (`name`) and examples/cats-app/tests/login.rs, not crates/corvis/src/jwt.rs");
    assert_eq!(paths, ["crates/corvis/tests/auth_jwt.rs", "examples/cats-app/tests/login.rs"]);
}

// ---- the dependency inventory ----

/// Every version of every package in `Cargo.lock`.
fn locked() -> BTreeMap<String, Vec<String>> {
    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut name: Option<String> = None;
    for line in read("Cargo.lock").lines() {
        if let Some(n) = line.strip_prefix("name = ") {
            name = Some(n.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("version = ")
            && let Some(n) = name.take()
        {
            packages.entry(n).or_default().push(v.trim_matches('"').to_string());
        }
    }
    packages
}

/// The generation of a version: the major, or `0.minor` while the major is zero.
fn generation(version: &str) -> String {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("");
    if major == "0" { format!("0.{}", parts.next().unwrap_or("0")) } else { major.to_string() }
}

/// The rows of the dependency inventory: `(feature, crate, version, policy)`.
fn inventory(page: &str) -> Vec<(String, String, String, String)> {
    let section = page.split("\n## Dependencies").nth(1).expect("the page has a Dependencies section");
    let section = section.split("\n## ").next().unwrap();
    section
        .lines()
        .filter(|l| l.starts_with("| `"))
        .map(|l| {
            let cells: Vec<String> = l.trim_matches('|').split('|').map(|c| c.trim().trim_matches('`').to_string()).collect();
            assert!(cells.len() >= 6, "{l}");
            (cells[0].clone(), cells[1].clone(), cells[2].clone(), cells[5].clone())
        })
        .collect()
}

#[test]
fn the_inventory_matches_the_lock_file() {
    let rows = inventory(&read(PAGE));
    assert!(rows.len() >= 15, "{} rows", rows.len());
    let lock = locked();
    for (feature, name, version, _) in &rows {
        let versions = lock.get(name).unwrap_or_else(|| panic!("{name} ({feature}) is in the inventory and not in Cargo.lock"));
        assert!(
            versions.iter().any(|v| generation(v) == generation(version)),
            "{name} ({feature}): the page says {version}, Cargo.lock has {versions:?}: update the inventory of {PAGE}"
        );
    }
    // The features and the crates they pull are all there.
    for feature in ["jwt", "auth-local", "auth-api-key", "auth-session", "auth-oauth2"] {
        assert!(rows.iter().any(|r| r.0 == feature), "the inventory has no row for {feature}");
    }
}

#[test]
fn a_changed_major_is_noticed() {
    assert_eq!(generation("9.3.1"), "9");
    assert_eq!(generation("0.17.14"), "0.17");
    assert_ne!(generation("0.5.3"), generation("0.6.0"));
    assert_eq!(generation("0.10.6"), generation("0.10.9"));
}

#[test]
fn every_single_generation_crate_is_banned_from_a_second_version() {
    let deny = read("deny.toml");
    for (feature, name, _, policy) in inventory(&read(PAGE)) {
        if policy == "one generation" {
            let entry = deny
                .lines()
                .find(|l| l.contains(&format!("crate = \"{name}\"")))
                .unwrap_or_else(|| panic!("{name} ({feature}) is marked `one generation` and deny.toml has no entry for it"));
            assert!(entry.contains("deny-multiple-versions = true"), "{name}: {entry}");
        }
    }
    for banned in ["openssl", "openssl-sys", "native-tls"] {
        assert!(deny.contains(&format!("crate = \"{banned}\"")), "{banned} is not banned");
    }
    // No ignored advisory names a crate of the inventory.
    let ignored = format!("{deny}\n{}", read(".cargo/audit.toml"));
    for (_, name, _, _) in inventory(&read(PAGE)) {
        for line in ignored.lines().filter(|l| l.contains("RUSTSEC") && !l.trim_start().starts_with('#')) {
            assert!(!line.contains(&format!("`{name}`")), "an ignored advisory names {name}: {line}");
        }
    }
}

// ---- the operating system generator ----

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

const FORBIDDEN: [&str; 8] =
    ["thread_rng", "rand::random", "SmallRng", "StdRng", "seed_from_u64", "from_entropy", "ThreadRng", "rand::rng("];

/// `(file, line, text)` of every use of a generator that is not the operating system's, outside test code.
fn offenders(files: &[(String, String)]) -> Vec<(String, usize, String)> {
    let mut found = Vec::new();
    for (name, text) in files {
        for (i, line) in text.lines().enumerate() {
            // Test modules sit at the end of a file: nothing after the first `#[cfg(test)]` counts.
            if line.trim() == "#[cfg(test)]" {
                break;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            if FORBIDDEN.iter().any(|f| line.contains(f)) {
                found.push((name.clone(), i + 1, line.trim().to_string()));
            }
        }
    }
    found
}

fn authentication_sources() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    rust_files(&root().join("crates/corvis/src/auth"), &mut paths);
    paths.push(root().join("crates/corvis/src/http/signed_cookie.rs"));
    paths
        .into_iter()
        .filter(|p| {
            // Files that are test code as a whole.
            let name = p.file_name().unwrap().to_string_lossy();
            !(name.ends_with("tests.rs") || name == "fuzz.rs")
        })
        .map(|p| {
            let relative = p.strip_prefix(root()).unwrap().to_string_lossy().replace('\\', "/");
            (relative, std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n"))
        })
        .collect()
}

#[test]
fn secrets_come_from_the_operating_system_generator() {
    let files = authentication_sources();
    assert!(files.len() >= 30, "{} files scanned", files.len());
    let found = offenders(&files);
    assert!(found.is_empty(), "a generator that is not the operating system's is used: {found:?}");
    // The three places that generate secrets name it.
    for file in
        ["crates/corvis/src/auth/session/session.rs", "crates/corvis/src/auth/oauth2/state.rs", "crates/corvis/src/auth/local/hasher.rs"]
    {
        let text = &files.iter().find(|(n, _)| n == file).unwrap_or_else(|| panic!("{file} was not scanned")).1;
        assert!(text.contains("OsRng"), "{file} no longer uses OsRng");
    }
}

#[test]
fn the_scan_finds_a_thread_local_generator() {
    let files = vec![(
        "crates/corvis/src/auth/session/session.rs".to_string(),
        "fn id() {\n    // rand::thread_rng() is not allowed\n    let mut rng = rand::thread_rng();\n}\n#[cfg(test)]\nmod t { fn x() { rand::thread_rng(); } }\n"
            .to_string(),
    )];
    let found = offenders(&files);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].0.as_str(), found[0].1), ("crates/corvis/src/auth/session/session.rs", 3));
}
