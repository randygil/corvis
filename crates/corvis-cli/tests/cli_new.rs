//! The framework dependency written by `corvis new`: the published version by default, a local checkout only
//! through `--corvis-path` or `CORVIS_PATH`, with a fixed precedence.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvis-cli-new-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the CLI with `CORVIS_PATH` set to `env_path`, or removed when `None`.
fn corvis(cwd: &Path, args: &[&str], env_path: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_corvis"));
    cmd.args(args).current_dir(cwd);
    match env_path {
        Some(p) => cmd.env("CORVIS_PATH", p),
        None => cmd.env_remove("CORVIS_PATH"),
    };
    let out = cmd.output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn manifest(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("my-app").join("Cargo.toml")).unwrap()
}

#[test]
fn default_is_the_cli_version() {
    let dir = sandbox("default");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git"], None);
    assert!(ok, "{out}");
    let cargo = manifest(&dir);
    assert!(cargo.contains(&format!("corvis = {{ version = \"{}\" }}", env!("CARGO_PKG_VERSION"))), "{cargo}");
    assert!(!cargo.contains("path ="), "{cargo}");
    assert!(!out.contains("using CORVIS_PATH"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn empty_environment_variable_counts_as_unset() {
    let dir = sandbox("empty");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git"], Some(""));
    assert!(ok, "{out}");
    assert!(!manifest(&dir).contains("path ="));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn path_from_the_flag() {
    let dir = sandbox("flag");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git", "--corvis-path", "/src/corvis/crates/corvis"], None);
    assert!(ok, "{out}");
    assert!(manifest(&dir).contains("corvis = { path = \"/src/corvis/crates/corvis\" }"));
    assert!(!out.contains("NOTE"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn path_from_the_environment_prints_a_note() {
    let dir = sandbox("env");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git"], Some("/src/corvis/crates/corvis"));
    assert!(ok, "{out}");
    assert!(manifest(&dir).contains("corvis = { path = \"/src/corvis/crates/corvis\" }"));
    assert!(out.contains("NOTE") && out.contains("using CORVIS_PATH=/src/corvis/crates/corvis"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn flags_beat_the_environment() {
    let dir = sandbox("beats");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git", "--corvis-version", "0.2"], Some("/env/corvis"));
    assert!(ok, "{out}");
    let cargo = manifest(&dir);
    assert!(cargo.contains("corvis = { version = \"0.2\" }") && !cargo.contains("path ="), "{cargo}");
    std::fs::remove_dir_all(dir.join("my-app")).unwrap();
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git", "--corvis-path", "/flag/corvis"], Some("/env/corvis"));
    assert!(ok, "{out}");
    assert!(manifest(&dir).contains("path = \"/flag/corvis\""));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn both_flags_fail_and_create_nothing() {
    let dir = sandbox("both");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git", "--corvis-path", "../corvis", "--corvis-version", "0.1"], None);
    assert!(!ok, "{out}");
    assert!(out.contains("--corvis-path") && out.contains("--corvis-version"), "{out}");
    assert!(!dir.join("my-app").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unsafe_environment_path_names_the_variable() {
    let dir = sandbox("unsafe");
    let (ok, out) = corvis(&dir, &["new", "my-app", "--skip-git"], Some("a\"b"));
    assert!(!ok && out.contains("CORVIS_PATH"), "{out}");
    assert!(!dir.join("my-app").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn help_mentions_the_default_and_the_variable() {
    let dir = sandbox("help");
    let (ok, out) = corvis(&dir, &["--help"], None);
    assert!(ok && out.contains("CORVIS_PATH") && out.contains("from crates.io"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}
