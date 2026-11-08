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

fn scaffold(tag: &str) -> (PathBuf, PathBuf) {
    let dir = sandbox(tag);
    let work = dir.join("work");
    let (ok, out) = corvis(&work, &["new", "app", "--skip-git", "--corvis-version", "0.1"]);
    assert!(ok, "{out}");
    (dir, work.join("app"))
}

/// Before: `g controller profile --path users` looked for `users/profile_module.rs` and registered in `AppModule`.
#[test]
fn path_registers_in_the_folders_module() {
    let (dir, app) = scaffold("path-module");
    let (ok, out) = corvis(&app, &["g", "module", "users"]);
    assert!(ok, "{out}");
    let (ok, out) = corvis(&app, &["g", "controller", "profile", "--path", "users"]);
    assert!(ok, "{out}");
    assert!(out.contains("src/users/users_module.rs"), "{out}");
    let users = std::fs::read_to_string(app.join("src/users/users_module.rs")).unwrap();
    assert!(users.contains("ProfileController"), "{users}");
    let app_module = std::fs::read_to_string(app.join("src/app_module.rs")).unwrap();
    assert!(!app_module.contains("ProfileController"), "{app_module}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_to_app_module_names_the_folders_modules() {
    let (dir, app) = scaffold("path-fallback");
    // A folder with a module that is not named after the folder or the element.
    std::fs::create_dir_all(app.join("src/users")).unwrap();
    std::fs::write(app.join("src/users/mod.rs"), "").unwrap();
    std::fs::write(app.join("src/users/other_module.rs"), "").unwrap();
    let (ok, out) = corvis(&app, &["g", "service", "profile", "--path", "users"]);
    assert!(ok, "{out}");
    assert!(out.contains("NOTE") && out.contains("other_module.rs"), "{out}");
    let app_module = std::fs::read_to_string(app.join("src/app_module.rs")).unwrap();
    assert!(app_module.contains("ProfileService"), "{app_module}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_folder_registration_is_unchanged() {
    let (dir, app) = scaffold("default-folder");
    let (ok, out) = corvis(&app, &["g", "module", "users"]);
    assert!(ok, "{out}");
    let (ok, out) = corvis(&app, &["g", "service", "users"]);
    assert!(ok, "{out}");
    let users = std::fs::read_to_string(app.join("src/users/users_module.rs")).unwrap();
    assert!(users.contains("UsersService"), "{users}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn start_watch_without_cargo_watch_explains_how_to_install_it() {
    let dir = sandbox("watch");
    let work = dir.join("work");
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_corvis"))
        .args(["start", "--watch"])
        .current_dir(&work)
        .env("PATH", &empty)
        .env("CARGO_HOME", &empty)
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("cargo install cargo-watch") && text.contains("without --watch"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
