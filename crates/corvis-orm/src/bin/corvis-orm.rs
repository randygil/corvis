//! `corvis-orm`: the command-line tool of corvis-orm (code generation from
//! `schema.prisma` files and migrations).
//!
//! ```text
//! corvis-orm generate [--schema prisma/schema.prisma] [--out src/models.rs]
//! corvis-orm migrate deploy [--schema ...] [--migrations prisma/migrations] [--url $DATABASE_URL]
//! corvis-orm migrate status [--schema ...] [--migrations prisma/migrations] [--url $DATABASE_URL]
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> &'static str {
    "corvis-orm <command>

Commands:
  generate         Generate the Rust models from schema.prisma
      --schema <path>        Schema file or folder (default: ./prisma/schema.prisma)
      --out <file.rs>        Output file (default: ./src/models.rs)
      --no-docs              Do not copy /// comments
      --derive <Trait>       Extra derive for every model and enum (repeatable)
  migrate deploy   Apply the pending prisma/migrations/*/migration.sql
  migrate status   Show applied and pending migrations
      --migrations <dir>     Migrations folder (default: <schema dir>/migrations)
      --url <url>            Database URL (default: $DATABASE_URL)
"
}

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.rest.iter().position(|a| a == name) {
            self.rest.remove(i);
            true
        } else {
            false
        }
    }
    fn value(&mut self, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        if let Some(i) = self.rest.iter().position(|a| a.starts_with(&prefix)) {
            return Some(self.rest.remove(i)[prefix.len()..].to_string());
        }
        let i = self.rest.iter().position(|a| a == name)?;
        self.rest.remove(i);
        if i < self.rest.len() { Some(self.rest.remove(i)) } else { None }
    }
    fn values(&mut self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(v) = self.value(name) {
            out.push(v);
        }
        out
    }
}

fn default_schema() -> PathBuf {
    for p in ["prisma/schema.prisma", "schema.prisma", "prisma/schema"] {
        if Path::new(p).exists() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("prisma/schema.prisma")
}

fn load_dotenv() {
    for p in [".env", "prisma/.env"] {
        if let Ok(text) = std::fs::read_to_string(p) {
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let k = k.trim().trim_start_matches("export ").trim();
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if std::env::var_os(k).is_none() {
                        // SAFETY: single-threaded at this point (before the runtime starts).
                        unsafe { std::env::set_var(k, v) };
                    }
                }
            }
        }
    }
}

fn main() -> ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let cmd = argv.remove(0);
    let mut args = Args { rest: argv };
    let result = match cmd.as_str() {
        "generate" => generate(&mut args),
        "migrate" => {
            if args.rest.is_empty() {
                Err("missing migrate subcommand (deploy | status)".into())
            } else {
                let sub = args.rest.remove(0);
                migrate(&sub, &mut args)
            }
        }
        other => Err(format!("unknown command `{other}`\n\n{}", usage())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn generate(args: &mut Args) -> Result<(), String> {
    let schema_path = args.value("--schema").map(PathBuf::from).unwrap_or_else(default_schema);
    let out = args.value("--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("src/models.rs"));
    let docs = !args.flag("--no-docs");
    let derives = args.values("--derive");
    let schema = corvis_orm::psl::parse_path(&schema_path).map_err(|e| e.to_string())?;
    let opts =
        corvis_orm::codegen::Options { extra_derives: derives, docs, source: Some(schema_path.to_string_lossy().replace('\\', "/")) };
    let code = corvis_orm::codegen::generate(&schema, &opts).map_err(|e| e.to_string())?;
    if let Some(dir) = out.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out, code).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "Generated {} models and {} enums from {} into {}",
        schema.models.iter().filter(|m| m.attribute("ignore").is_none()).count(),
        schema.enums.len(),
        schema_path.display(),
        out.display()
    );
    Ok(())
}

fn migrate(sub: &str, args: &mut Args) -> Result<(), String> {
    load_dotenv();
    let schema_path = args.value("--schema").map(PathBuf::from).unwrap_or_else(default_schema);
    let dir = args.value("--migrations").map(PathBuf::from).unwrap_or_else(|| {
        let base = if schema_path.is_dir() { schema_path.clone() } else { schema_path.parent().map(Path::to_path_buf).unwrap_or_default() };
        base.join("migrations")
    });
    let url =
        args.value("--url").or_else(|| std::env::var("DATABASE_URL").ok()).ok_or("no database URL: pass --url or set DATABASE_URL")?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let client = corvis_orm::OrmClient::new(url).map_err(|e| e.to_string())?;
        match sub {
            "deploy" => {
                let applied = client.migrate_deploy(&dir).await.map_err(|e| e.to_string())?;
                if applied.is_empty() {
                    println!("No pending migrations to apply.");
                } else {
                    println!("The following migration(s) have been applied:\n");
                    for m in &applied {
                        println!("  {m}");
                    }
                    println!("\nAll migrations have been successfully applied.");
                }
                Ok(())
            }
            "status" => {
                let st = client.migrate_status(&dir).await.map_err(|e| e.to_string())?;
                print!("{st}");
                if st.is_up_to_date() { Ok(()) } else { Err("the database is not up to date".into()) }
            }
            other => Err(format!("unknown migrate subcommand `{other}` (deploy | status)")),
        }
    })
}
