//! The `corvis` binary rejects names, `--path` and `--corvis-version` values that would write
//! outside the project or inject text into the generated files.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvis-cli-input-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    dir
}

fn corvis(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_corvis")).args(args).current_dir(cwd).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// Before: `corvis new ../escaped` created the project outside the current directory and
/// `--corvis-version` was copied verbatim into `Cargo.toml`.
#[test]
fn new_rejects_unsafe_names_and_versions() {
    let dir = sandbox("new");
    let work = dir.join("work");
    for name in ["../escaped", "a/b", r"a\b", "C:", "x\"y"] {
        let (ok, out) = corvis(&work, &["new", name, "--skip-git"]);
        assert!(!ok, "{name}: {out}");
        assert!(out.contains("invalid name"), "{name}: {out}");
    }
    assert!(!dir.join("escaped").exists());
    let (ok, out) = corvis(&work, &["new", "app", "--skip-git", "--corvis-version", "1\"\n[dependencies.evil]\npath = \"x"]);
    assert!(!ok && out.contains("invalid --corvis-version"), "{out}");
    assert!(!work.join("app").exists());
    let (ok, out) = corvis(&work, &["new", "my-app", "--skip-git", "--corvis-version", "^0.1"]);
    assert!(ok, "{out}");
    let cargo = std::fs::read_to_string(work.join("my-app").join("Cargo.toml")).unwrap();
    assert!(cargo.contains("version = \"^0.1\""), "{cargo}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Before: `--path ../../x` (or an absolute path) wrote the generated files outside `src/`.
#[test]
fn generate_rejects_unsafe_paths() {
    let dir = sandbox("generate");
    let work = dir.join("work");
    let (ok, out) = corvis(&work, &["new", "app", "--skip-git", "--corvis-version", "0.1"]);
    assert!(ok, "{out}");
    let app = work.join("app");
    let outside = dir.join("outside").to_string_lossy().replace('\\', "/");
    for path in ["../../outside", outside.as_str(), "a/b", "a-b"] {
        let (ok, out) = corvis(&app, &["g", "service", "cats", "--path", path]);
        assert!(!ok && out.contains("invalid --path"), "{path}: {out}");
    }
    let (ok, out) = corvis(&app, &["g", "service", "../cats"]);
    assert!(!ok && out.contains("invalid name"), "{out}");
    assert!(!dir.join("outside").exists() && !work.join("cats").exists() && !app.join("cats").exists());
    let (ok, out) = corvis(&app, &["g", "service", "cats", "--path", "animals"]);
    assert!(ok, "{out}");
    assert!(app.join("src").join("animals").join("cats_service.rs").exists());
    let _ = std::fs::remove_dir_all(dir);
}
