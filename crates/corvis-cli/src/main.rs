//! `corvis` – the command line interface: scaffolds projects and generates modules, controllers,
//! services and the other building blocks.
//!
//! ```text
//! corvis new my-app
//! corvis generate resource cats      (alias: corvis g res cats)
//! corvis g module users | controller | service | guard | interceptor | pipe | filter | middleware | gateway
//! corvis start [--watch]
//! corvis build
//! corvis info
//! ```

mod edit;
mod names;
mod templates;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use names::Names;
use templates::render;

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") | Some("n") => cmd_new(&args[1..]),
        Some("generate") | Some("g") => cmd_generate(&args[1..]),
        Some("start") => cmd_cargo(&args[1..], "run"),
        Some("build") => cmd_cargo(&args[1..], "build"),
        Some("info") | Some("i") => {
            cmd_info();
            Ok(())
        }
        Some("--version") | Some("-v") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            print_help();
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{RED}Error:{RESET} {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        r#"Usage: corvis <command> [options]

Commands:
  new|n <name> [--corvis-path <dir>] [--corvis-version <v>] [--skip-git]
                                 Generate a new Corvis application.
  generate|g <schematic> <name> [--flat] [--path <dir>] [--no-spec]
                                 Generate a Corvis element.
  start [--watch] [--release] [-- args]
                                 Run the application (cargo run, or cargo watch -x "run ..." with --watch).
  build [--release]              Build the application (debug profile unless --release).
  info|i                         Display Corvis project details.

Schematics:
  module (mo)        controller (co)    service (s)        resource (res)
  guard (gu)         interceptor (itc)  pipe (pi)          filter (f)
  middleware (mi)    gateway (ga)
"#
    );
}

type Res = Result<(), String>;

fn write_file(path: &Path, contents: &str) -> Res {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(path, contents).map_err(|e| e.to_string())?;
    let size = contents.len();
    println!("{GREEN}CREATE{RESET} {} ({size} bytes)", path.display().to_string().replace('\\', "/"));
    Ok(())
}

fn update_file(path: &Path, f: impl FnOnce(&str) -> Option<String>) -> Res {
    let src = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(new) = f(&src) else {
        println!("{YELLOW}SKIP{RESET} {} (could not update automatically)", path.display());
        return Ok(());
    };
    if new != src {
        fs::write(path, &new).map_err(|e| e.to_string())?;
        println!("{YELLOW}UPDATE{RESET} {} ({} bytes)", path.display().to_string().replace('\\', "/"), new.len());
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--path" || a == "--corvis-path" || a == "--corvis-version" {
            skip = true;
            continue;
        }
        if !a.starts_with("--") {
            out.push(a.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------- new

fn corvis_dependency(args: &[String]) -> Result<String, String> {
    if let Some(v) = opt(args, "--corvis-version") {
        names::validate_version_req(&v)?;
        return Ok(format!("version = \"{v}\""));
    }
    let path = opt(args, "--corvis-path").or_else(|| std::env::var("CORVIS_PATH").ok()).unwrap_or_else(|| {
        // Path of the framework this CLI was built from.
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("corvis");
        p.canonicalize().unwrap_or(p).display().to_string()
    });
    let path = path.trim_start_matches(r"\\?\").replace('\\', "/");
    // Written inside a TOML basic string.
    if path.chars().any(|c| c == '"' || c.is_control()) {
        return Err(format!("invalid --corvis-path \"{}\"", path.escape_debug()));
    }
    Ok(format!("path = \"{path}\""))
}

fn cmd_new(args: &[String]) -> Res {
    let pos = positional(args);
    let name = pos.first().ok_or("missing project name: corvis new <name>")?;
    names::validate_name(name)?;
    let dependency = corvis_dependency(args)?;
    let n = Names::new(name);
    let root = PathBuf::from(&n.kebab);
    if root.exists() {
        return Err(format!("directory {} already exists", root.display()));
    }
    println!("⚡ We will scaffold your app in a few seconds..\n");
    let cargo = render(templates::CARGO_TOML, &n).replace("__CORVIS_DEP__", &dependency);
    write_file(&root.join("Cargo.toml"), &cargo)?;
    write_file(&root.join(".gitignore"), templates::GITIGNORE)?;
    write_file(&root.join(".env"), templates::ENV)?;
    write_file(&root.join("src/main.rs"), templates::MAIN)?;
    write_file(&root.join("src/app_module.rs"), templates::APP_MODULE)?;
    write_file(&root.join("src/app_controller.rs"), templates::APP_CONTROLLER)?;
    write_file(&root.join("src/app_service.rs"), templates::APP_SERVICE)?;
    if !flag(args, "--skip-git") {
        let _ = Command::new("git").arg("init").arg("-q").current_dir(&root).status();
    }
    println!("\n🚀  Successfully created project {GREEN}{}{RESET}", n.kebab);
    println!("👉  Get started with the following commands:\n");
    println!("$ cd {}", n.kebab);
    println!("$ cargo run\n");
    Ok(())
}

// ---------------------------------------------------------------------- generate

struct Project {
    src: PathBuf,
}

fn find_project() -> Result<Project, String> {
    let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("src").exists() {
            return Ok(Project { src: dir.join("src") });
        }
        if !dir.pop() {
            return Err("no Cargo project found (run this inside a Corvis application)".into());
        }
    }
}

fn crate_root_file(src: &Path) -> PathBuf {
    if src.join("main.rs").exists() { src.join("main.rs") } else { src.join("lib.rs") }
}

/// Ensures `src/<dir>/mod.rs` exists and is declared in the crate root.
fn ensure_feature_dir(p: &Project, dir: &str) -> Res {
    let mod_file = p.src.join(dir).join("mod.rs");
    if !mod_file.exists() {
        write_file(&mod_file, "")?;
    }
    update_file(&crate_root_file(&p.src), |s| Some(edit::add_mod(s, &format!("mod {dir};"))))
}

fn cmd_generate(args: &[String]) -> Res {
    let pos = positional(args);
    let schematic = pos.first().ok_or("missing schematic: corvis g <schematic> <name>")?;
    let name = pos.get(1).ok_or("missing name: corvis g <schematic> <name>")?;
    names::validate_name(name)?;
    let n = Names::new(name);
    let dir = opt(args, "--path").unwrap_or_else(|| n.snake.clone());
    names::validate_module_dir(&dir)?;
    let p = find_project()?;
    let flat = flag(args, "--flat");
    let no_spec = flag(args, "--no-spec");

    let simple = |suffix: &str, template: &str, register: Option<&str>| -> Res {
        let file_stem = format!("{}_{suffix}", n.snake);
        let mut contents = render(template, &n);
        if no_spec && let Some(i) = contents.find("\n#[cfg(test)]") {
            contents.truncate(i + 1);
        }
        let type_name = format!("{}{}", n.pascal, capitalize(suffix));
        if flat {
            write_file(&p.src.join(format!("{file_stem}.rs")), &contents)?;
            update_file(&crate_root_file(&p.src), |s| Some(edit::add_mod(s, &format!("mod {file_stem};"))))?;
        } else {
            ensure_feature_dir(&p, &dir)?;
            write_file(&p.src.join(&dir).join(format!("{file_stem}.rs")), &contents)?;
            update_file(&p.src.join(&dir).join("mod.rs"), |s| {
                let s = edit::add_mod(s, &format!("pub mod {file_stem};"));
                Some(add_pub_use(&s, &format!("pub use {file_stem}::{type_name};")))
            })?;
        }
        // Register controllers / providers in the feature module (or AppModule).
        if let Some(key) = register {
            let use_path = if flat { format!("crate::{file_stem}::{type_name}") } else { format!("crate::{dir}::{type_name}") };
            let target = registration_target(&p.src, &dir, &n.snake, flat);
            if target.exists() {
                if !flat && target == p.src.join("app_module.rs") {
                    let others = module_files_in(&p.src.join(&dir));
                    if !others.is_empty() {
                        println!(
                            "{YELLOW}NOTE{RESET} registered in src/app_module.rs; src/{dir}/ has {} (move the registration there if that was intended)",
                            others.join(", ")
                        );
                    }
                }
                update_file(&target, |s| {
                    let s = edit::add_to_module(s, key, &type_name)?;
                    Some(edit::add_use(&s, &format!("use {use_path};")))
                })?;
            }
        }
        Ok(())
    };

    match schematic.as_str() {
        "module" | "mo" => {
            simple("module", templates::MODULE, None)?;
            register_in_app_module(&p, &dir, &format!("{}Module", n.pascal), flat, &format!("{}_module", n.snake))
        }
        "controller" | "co" => simple("controller", templates::CONTROLLER, Some("controllers")),
        "service" | "s" => simple("service", templates::SERVICE, Some("providers")),
        "guard" | "gu" => simple("guard", templates::GUARD, None),
        "interceptor" | "itc" | "in" => simple("interceptor", templates::INTERCEPTOR, None),
        "pipe" | "pi" => simple("pipe", templates::PIPE, None),
        "filter" | "f" => simple("filter", templates::FILTER, None),
        "middleware" | "mi" => simple("middleware", templates::MIDDLEWARE, None),
        "gateway" | "ga" => simple("gateway", templates::GATEWAY, Some("providers")),
        "resource" | "res" => generate_resource(&p, &n, &dir),
        other => Err(format!("unknown schematic \"{other}\"")),
    }
}

/// The module file that receives the registration of a generated element:
/// `<dir>/<name>_module.rs`, then `<dir>/<dir>_module.rs`, then `app_module.rs`.
fn registration_target(src: &Path, dir: &str, name_snake: &str, flat: bool) -> PathBuf {
    if !flat {
        let folder = src.join(dir);
        let own = folder.join(format!("{name_snake}_module.rs"));
        if own.exists() {
            return own;
        }
        let folders = folder.join(format!("{}_module.rs", dir.rsplit('/').next().unwrap_or(dir)));
        if folders.exists() {
            return folders;
        }
    }
    src.join("app_module.rs")
}

/// Names of the `*_module.rs` files directly inside `folder`, sorted.
fn module_files_in(folder: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with("_module.rs"))
        .collect();
    out.sort();
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn add_pub_use(source: &str, line: &str) -> String {
    if source.lines().any(|l| l.trim() == line) {
        return source.to_string();
    }
    let mut s = source.trim_end().to_string();
    if !s.contains("\npub use ") && !s.starts_with("pub use ") {
        s.push('\n');
    }
    s.push('\n');
    s.push_str(line);
    s.push('\n');
    s
}

fn register_in_app_module(p: &Project, dir: &str, module: &str, flat: bool, file_stem: &str) -> Res {
    let app = p.src.join("app_module.rs");
    if !app.exists() {
        return Ok(());
    }
    let use_path = if flat { format!("crate::{file_stem}::{module}") } else { format!("crate::{dir}::{module}") };
    update_file(&app, |s| {
        let s = edit::add_to_module(s, "imports", module)?;
        Some(edit::add_use(&s, &format!("use {use_path};")))
    })
}

fn generate_resource(p: &Project, n: &Names, dir: &str) -> Res {
    let base = p.src.join(dir);
    if base.join("mod.rs").exists() {
        return Err(format!("{} already exists", base.display()));
    }
    write_file(&base.join("mod.rs"), &render(templates::RES_MOD, n))?;
    write_file(&base.join(format!("{}_module.rs", n.snake)), &render(templates::RES_MODULE, n))?;
    write_file(&base.join(format!("{}_controller.rs", n.snake)), &render(templates::RES_CONTROLLER, n))?;
    write_file(&base.join(format!("{}_service.rs", n.snake)), &render(templates::RES_SERVICE, n))?;
    write_file(&base.join("dto").join("mod.rs"), &render(templates::RES_DTO, n))?;
    write_file(&base.join("entities").join("mod.rs"), &render(templates::RES_ENTITY, n))?;
    update_file(&crate_root_file(&p.src), |s| Some(edit::add_mod(s, &format!("mod {dir};"))))?;
    register_in_app_module(p, dir, &format!("{}Module", n.pascal), false, "")
}

// ---------------------------------------------------------------------- start / build / info

/// Looks for the `cargo-<name>` executable where Cargo finds external subcommands: the
/// directories of `PATH` and `$CARGO_HOME/bin` (default `~/.cargo/bin`).
fn find_cargo_subcommand(name: &str) -> Option<PathBuf> {
    let file = format!("cargo-{name}{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
        home.map(|h| PathBuf::from(h).join(".cargo"))
    });
    dirs.extend(cargo_home.map(|c| c.join("bin")));
    dirs.into_iter().map(|d| d.join(&file)).find(|f| f.is_file())
}

/// Builds the arguments given to `cargo` for `corvis start` (`sub == "run"`) or `corvis build`.
/// `--watch`, `--release` and `--prod` are read only before the first `--`; everything after it
/// belongs to the program.
fn cargo_invocation(args: &[String], sub: &str) -> Result<Vec<String>, String> {
    let (own, program) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], Some(&args[i + 1..])),
        None => (args, None),
    };
    let release = flag(own, "--release") || flag(own, "--prod");
    let watch = sub == "run" && flag(own, "--watch");
    let program = program.unwrap_or(&[]);
    if watch {
        let mut line = String::from("run");
        if release {
            line.push_str(" --release");
        }
        if !program.is_empty() {
            line.push_str(" --");
            for a in program {
                if !watch_safe(a) {
                    return Err(format!(
                        "argument \"{a}\" cannot be passed through cargo watch (it runs the command through a shell).                          Run \"corvis start\" without --watch, or run cargo watch directly with your shell's quoting."
                    ));
                }
                line.push(' ');
                line.push_str(a);
            }
        }
        return Ok(vec!["watch".into(), "-x".into(), line]);
    }
    let mut out = vec![sub.to_string()];
    if release {
        out.push("--release".into());
    }
    if args.iter().any(|a| a == "--") {
        out.push("--".into());
        out.extend(program.iter().cloned());
    }
    Ok(out)
}

/// A program argument that a shell hands over unchanged: non-empty, letters, digits and `-_./:=,+@`.
fn watch_safe(arg: &str) -> bool {
    !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=,+@".contains(c))
}

fn cmd_cargo(args: &[String], sub: &str) -> Res {
    let invocation = cargo_invocation(args, sub)?;
    let watch = invocation.first().map(String::as_str) == Some("watch");
    if !watch {
        let release = invocation.iter().take_while(|a| *a != "--").any(|a| a == "--release");
        let verb = if sub == "build" { "Building" } else { "Running" };
        if release {
            println!("{verb} the release profile");
        } else {
            println!("{verb} the debug profile (use --release for an optimized build)");
        }
    } else if find_cargo_subcommand("watch").is_none() {
        return Err(format!(
            "cargo-watch is not installed: corvis start --watch runs \"cargo watch -x \\\"{}\\\"\".                 Install it with \"cargo install cargo-watch\", or run \"corvis start\" without --watch.",
            invocation[2]
        ));
    }
    let status = Command::new("cargo").args(&invocation).status().map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("cargo {sub} failed")) }
}

fn cmd_info() {
    println!(
        r#"
   ___   ___   ___  __   __ ___  ___
  / __| / _ \ | _ \ \ \ / /|_ _|/ __|
 | (__ | (_) ||   /  \ V /  | | \__ \
  \___| \___/ |_|_\   \_/  |___||___/
"#
    );
    println!("[System Information]");
    println!("OS Version     : {} {}", std::env::consts::OS, std::env::consts::ARCH);
    if let Ok(out) = Command::new("rustc").arg("--version").output() {
        print!("Rust Version   : {}", String::from_utf8_lossy(&out.stdout));
    }
    if let Ok(out) = Command::new("cargo").arg("--version").output() {
        print!("Cargo Version  : {}", String::from_utf8_lossy(&out.stdout));
    }
    println!("\n[Corvis CLI]");
    println!("Corvis CLI Version : {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str, files: &[&str]) -> PathBuf {
        let src = std::env::temp_dir().join(format!("corvis-cli-unit-{}-{tag}", std::process::id())).join("src");
        let _ = fs::remove_dir_all(src.parent().unwrap());
        for f in files {
            let path = src.join(f);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        src
    }

    fn cleanup(src: &Path) {
        let _ = fs::remove_dir_all(src.parent().unwrap());
    }

    #[test]
    fn registers_in_the_folders_module() {
        let src = tree("folder", &["app_module.rs", "users/users_module.rs"]);
        assert_eq!(registration_target(&src, "users", "profile", false), src.join("users/users_module.rs"));
        cleanup(&src);
    }

    #[test]
    fn a_module_named_after_the_element_wins() {
        let src = tree("named", &["app_module.rs", "users/users_module.rs", "users/profile_module.rs"]);
        assert_eq!(registration_target(&src, "users", "profile", false), src.join("users/profile_module.rs"));
        cleanup(&src);
    }

    #[test]
    fn falls_back_to_app_module() {
        let src = tree("none", &["app_module.rs", "users/mod.rs"]);
        assert_eq!(registration_target(&src, "users", "profile", false), src.join("app_module.rs"));
        cleanup(&src);
    }

    #[test]
    fn default_folder_and_flat() {
        let src = tree("default", &["app_module.rs", "users/users_module.rs"]);
        assert_eq!(registration_target(&src, "users", "users", false), src.join("users/users_module.rs"));
        assert_eq!(registration_target(&src, "users", "users", true), src.join("app_module.rs"));
        cleanup(&src);
    }

    #[test]
    fn finds_cargo_watch_in_cargo_home() {
        let home = std::env::temp_dir().join(format!("corvis-cli-unit-{}-home", std::process::id()));
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let file = bin.join(format!("cargo-watch{}", std::env::consts::EXE_SUFFIX));
        fs::write(&file, "").unwrap();
        // SAFETY: no other test in this binary reads or writes CARGO_HOME.
        unsafe { std::env::set_var("CARGO_HOME", &home) };
        assert_eq!(find_cargo_subcommand("watch"), Some(file));
        assert_eq!(find_cargo_subcommand("corvis-no-such-subcommand"), None);
        let _ = fs::remove_dir_all(home);
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn inv(a: &[&str], sub: &str) -> Result<Vec<String>, String> {
        cargo_invocation(&v(a), sub)
    }

    #[test]
    fn builds_start_and_build_commands() {
        assert_eq!(inv(&[], "run").unwrap(), v(&["run"]));
        assert_eq!(inv(&["--release"], "run").unwrap(), v(&["run", "--release"]));
        assert_eq!(inv(&["--prod"], "run").unwrap(), v(&["run", "--release"]));
        assert_eq!(inv(&["--", "a", "b"], "run").unwrap(), v(&["run", "--", "a", "b"]));
        assert_eq!(inv(&["--", "--release"], "run").unwrap(), v(&["run", "--", "--release"]));
        assert_eq!(inv(&["--", "--watch"], "run").unwrap(), v(&["run", "--", "--watch"]));
        assert_eq!(inv(&[], "build").unwrap(), v(&["build"]));
        assert_eq!(inv(&["--release"], "build").unwrap(), v(&["build", "--release"]));
        assert_eq!(inv(&["--", "x"], "build").unwrap(), v(&["build", "--", "x"]));
    }

    #[test]
    fn builds_watch_commands() {
        assert_eq!(inv(&["--watch"], "run").unwrap(), v(&["watch", "-x", "run"]));
        assert_eq!(inv(&["--watch", "--release"], "run").unwrap(), v(&["watch", "-x", "run --release"]));
        assert_eq!(
            inv(&["--watch", "--prod", "--", "--port", "4000"], "run").unwrap(),
            v(&["watch", "-x", "run --release -- --port 4000"])
        );
        assert_eq!(
            inv(&["--watch", "--", "--config=./dev.toml", "--host", "127.0.0.1"], "run").unwrap(),
            v(&["watch", "-x", "run -- --config=./dev.toml --host 127.0.0.1"])
        );
        assert_eq!(inv(&["--watch", "--", "http://localhost:3000/x"], "run").unwrap().len(), 3);
    }

    #[test]
    fn watch_rejects_shell_sensitive_arguments() {
        for bad in ["my app", "", "a;b", "$HOME", "%PATH%", "a&b", "a|b", "\"q\""] {
            let err = inv(&["--watch", "--", bad], "run").unwrap_err();
            assert!(err.contains(&format!("argument \"{bad}\"")) && err.contains("without --watch"), "{bad}: {err}");
        }
        assert_eq!(inv(&["--", "my app"], "run").unwrap(), v(&["run", "--", "my app"]));
    }
}
