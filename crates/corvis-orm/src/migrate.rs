//! `migrate deploy` / `migrate status`, compatible with the `prisma/migrations` folder
//! format and the `_prisma_migrations` table: a database migrated by the original
//! tooling of `schema.prisma` projects can be migrated by corvis-orm and the other way around.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::client::OrmClient;
use crate::db::Dialect;
use crate::error::{OrmError, Result};

/// A migration folder (`prisma/migrations/<name>/migration.sql`).
#[derive(Debug, Clone)]
pub struct MigrationInfo {
    pub name: String,
    pub path: PathBuf,
    /// sha256 (hex) of `migration.sql`, as stored in `_prisma_migrations.checksum`.
    pub checksum: String,
}

/// Result of [`OrmClient::migrate_status`].
#[derive(Debug, Clone, Default)]
pub struct MigrateStatus {
    /// Migrations found in the folder, sorted by name.
    pub local: Vec<String>,
    /// Applied (finished, not rolled back) migrations, in order.
    pub applied: Vec<String>,
    /// Local migrations not applied yet.
    pub pending: Vec<String>,
    /// Migrations that started but did not finish (and were not rolled back).
    pub failed: Vec<String>,
    /// Applied migrations missing from the folder.
    pub missing_locally: Vec<String>,
    /// Applied migrations whose file changed after being applied.
    pub modified: Vec<String>,
    pub migrations_dir: String,
}

impl MigrateStatus {
    pub fn is_up_to_date(&self) -> bool {
        self.pending.is_empty() && self.failed.is_empty()
    }
}

impl fmt::Display for MigrateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.local.len();
        writeln!(f, "{n} migration{} found in {}\n", if n == 1 { "" } else { "s" }, self.migrations_dir)?;
        if !self.failed.is_empty() {
            writeln!(f, "Following migration{} have failed:", if self.failed.len() == 1 { "" } else { "s" })?;
            for m in &self.failed {
                writeln!(f, "{m}")?;
            }
            writeln!(f)?;
        }
        if !self.missing_locally.is_empty() {
            writeln!(
                f,
                "The following migration{} applied to the database but missing from the local migrations directory:",
                if self.missing_locally.len() == 1 { " is" } else { "s are" }
            )?;
            for m in &self.missing_locally {
                writeln!(f, "{m}")?;
            }
            writeln!(f)?;
        }
        if !self.pending.is_empty() {
            writeln!(f, "Following migration{} have not yet been applied:", if self.pending.len() == 1 { "" } else { "s" })?;
            for m in &self.pending {
                writeln!(f, "{m}")?;
            }
            writeln!(f, "\nTo apply them run corvis-orm migrate deploy.")?;
        } else if self.failed.is_empty() {
            writeln!(f, "Database schema is up to date!")?;
        }
        Ok(())
    }
}

/// sha256 hex of a migration script (the `checksum` column).
pub fn checksum(script: &[u8]) -> String {
    let digest = Sha256::digest(script);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads `dir/*/migration.sql`, sorted by folder name.
pub fn read_migrations(dir: &Path) -> Result<Vec<MigrationInfo>> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| OrmError::init(format!("Could not read the migrations directory `{}`: {e}", dir.display())))?;
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let file = path.join("migration.sql");
        if path.is_dir() && file.is_file() {
            let bytes = std::fs::read(&file).map_err(|e| OrmError::init(format!("{}: {e}", file.display())))?;
            out.push(MigrationInfo { name: e.file_name().to_string_lossy().into_owned(), path: file, checksum: checksum(&bytes) });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn create_table_sql(d: Dialect, table: &str) -> String {
    let sql = match d {
        Dialect::Postgres => {
            r#"CREATE TABLE IF NOT EXISTS __TABLE__ (
    "id"                    VARCHAR(36) PRIMARY KEY NOT NULL,
    "checksum"              VARCHAR(64) NOT NULL,
    "finished_at"           TIMESTAMPTZ,
    "migration_name"        VARCHAR(255) NOT NULL,
    "logs"                  TEXT,
    "rolled_back_at"        TIMESTAMPTZ,
    "started_at"            TIMESTAMPTZ NOT NULL DEFAULT now(),
    "applied_steps_count"   INTEGER NOT NULL DEFAULT 0
);"#
        }
        Dialect::Sqlite => {
            r#"CREATE TABLE IF NOT EXISTS __TABLE__ (
    "id"                    TEXT PRIMARY KEY NOT NULL,
    "checksum"              TEXT NOT NULL,
    "finished_at"           DATETIME,
    "migration_name"        TEXT NOT NULL,
    "logs"                  TEXT,
    "rolled_back_at"        DATETIME,
    "started_at"            DATETIME NOT NULL DEFAULT current_timestamp,
    "applied_steps_count"   INTEGER UNSIGNED NOT NULL DEFAULT 0
);"#
        }
    };
    sql.replace("__TABLE__", table)
}

struct Row {
    id: String,
    name: String,
    checksum: String,
    finished: bool,
    rolled_back: bool,
    started_at: String,
}

fn ph(d: Dialect, i: usize) -> String {
    match d {
        Dialect::Postgres => format!("${i}"),
        Dialect::Sqlite => "?".into(),
    }
}

impl OrmClient {
    /// The history table, qualified with the URL schema so that recording a migration does not depend on
    /// the session's `search_path` (a migration script may change it).
    fn history_table(&self) -> String {
        crate::db::qualified(self.url_schema(), "_prisma_migrations")
    }

    async fn ensure_database(&self) -> Result<()> {
        if self.dialect() != Dialect::Postgres {
            return Ok(());
        }
        use sqlx::ConnectOptions;
        use std::str::FromStr;
        let url = self.inner.driver_url.clone();
        let opts = sqlx::postgres::PgConnectOptions::from_str(&url).map_err(|e| OrmError::init(format!("Invalid database URL: {e}")))?;
        match opts.connect().await {
            Ok(c) => {
                let _ = sqlx::Connection::close(c).await;
                Ok(())
            }
            Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("3D000") => {
                let Some(name) = opts.get_database().map(str::to_string) else { return Ok(()) };
                let admin = opts.clone().database("postgres");
                let mut c = admin.connect().await.map_err(|e| OrmError::init(e.to_string()))?;
                sqlx::Executor::execute(&mut c, format!("CREATE DATABASE \"{}\"", name.replace('"', "\"\"")).as_str())
                    .await
                    .map_err(|e| OrmError::init(e.to_string()))?;
                let _ = sqlx::Connection::close(c).await;
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }

    async fn migration_rows(&self) -> Result<Vec<Row>> {
        self.ensure_schema().await?;
        let t = self.history_table();
        self.execute_script(&create_table_sql(self.dialect(), &t)).await?;
        let rows = self
            .query_raw(
                &format!(r#"SELECT "id", "checksum", "migration_name", "finished_at" IS NOT NULL AS "finished", "rolled_back_at" IS NOT NULL AS "rolled_back", CAST("started_at" AS TEXT) AS "started_at" FROM {t} ORDER BY "started_at" ASC"#),
                vec![],
            )
            .await?;
        let truthy = |v: &Value| matches!(v, Value::Bool(true)) || v.as_i64() == Some(1);
        Ok(rows
            .into_iter()
            .map(|r| Row {
                id: r["id"].as_str().unwrap_or_default().to_string(),
                name: r["migration_name"].as_str().unwrap_or_default().to_string(),
                checksum: r["checksum"].as_str().unwrap_or_default().to_string(),
                finished: truthy(&r["finished"]),
                rolled_back: truthy(&r["rolled_back"]),
                started_at: r["started_at"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }

    /// `migrate deploy`: applies the pending `dir/*/migration.sql` in order and
    /// records them in `_prisma_migrations`. Returns the names of the applied migrations.
    ///
    /// With `?schema=name` in the URL, the schema is created when missing and the history table and
    /// the migrations live in it.
    ///
    /// Fails with `P3009` when a previous migration failed, and with `P3018` when a
    /// migration fails (its error is stored in `logs`).
    pub async fn migrate_deploy(&self, dir: impl AsRef<Path>) -> Result<Vec<String>> {
        let dir = dir.as_ref();
        let local = read_migrations(dir)?;
        self.ensure_database().await?;
        let rows = self.migration_rows().await?;
        if let Some(failed) = rows.iter().find(|r| !r.finished && !r.rolled_back) {
            return Err(OrmError::known(
                "P3009",
                format!(
                    "migrate found failed migrations in the target database, new migrations will not be applied. Read more about how to resolve migration issues in a production database: https://pris.ly/d/migrate-resolve\nThe `{}` migration started at {} failed",
                    failed.name, failed.started_at
                ),
                json!({}),
            ));
        }
        let applied: std::collections::HashSet<&str> =
            rows.iter().filter(|r| r.finished && !r.rolled_back).map(|r| r.name.as_str()).collect();
        let d = self.dialect();
        let mut done = Vec::new();
        let t = self.history_table();
        for m in local.iter().filter(|m| !applied.contains(m.name.as_str())) {
            let script = std::fs::read_to_string(&m.path).map_err(|e| OrmError::init(format!("{}: {e}", m.path.display())))?;
            let id = uuid::Uuid::new_v4().to_string();
            let now = if d == Dialect::Postgres { "now()" } else { "current_timestamp" };
            self.execute_raw(
                &format!(
                    r#"INSERT INTO {t} ("id", "checksum", "migration_name", "started_at", "applied_steps_count") VALUES ({}, {}, {}, {now}, 0)"#,
                    ph(d, 1),
                    ph(d, 2),
                    ph(d, 3)
                ),
                vec![json!(id), json!(m.checksum), json!(m.name)],
            )
            .await?;
            match self.execute_script_raw(&script).await? {
                Ok(()) => {
                    self.execute_raw(
                        &format!(
                            r#"UPDATE {t} SET "finished_at" = {now}, "applied_steps_count" = "applied_steps_count" + 1 WHERE "id" = {}"#,
                            ph(d, 1)
                        ),
                        vec![json!(id)],
                    )
                    .await?;
                    done.push(m.name.clone());
                }
                Err(e) => {
                    let (code, message) = db_error_parts(&e, self.dialect());
                    let logs = format!(
                        "{message}\n\n0: sql_schema_connector::apply_migration::apply_script\n           with migration_name=\"{}\"",
                        m.name
                    );
                    let _ = self
                        .execute_raw(
                            &format!(r#"UPDATE {t} SET "logs" = {} WHERE "id" = {}"#, ph(d, 1), ph(d, 2)),
                            vec![json!(logs), json!(id)],
                        )
                        .await;
                    return Err(OrmError::known(
                        "P3018",
                        format!(
                            "A migration failed to apply. New migrations cannot be applied before the error is recovered from. Read more about how to resolve migration issues in a production database: https://pris.ly/d/migrate-resolve\n\nMigration name: {}\n\nDatabase error code: {code}\n\nDatabase error:\n{message}\n",
                            m.name
                        ),
                        json!({ "migration_name": m.name, "database_error_code": code, "database_error": message }),
                    ));
                }
            }
        }
        Ok(done)
    }

    /// `migrate status`.
    pub async fn migrate_status(&self, dir: impl AsRef<Path>) -> Result<MigrateStatus> {
        let dir = dir.as_ref();
        let local = read_migrations(dir)?;
        self.ensure_database().await?;
        let rows = self.migration_rows().await?;
        let live: Vec<&Row> = rows.iter().filter(|r| !r.rolled_back).collect();
        let mut st = MigrateStatus {
            local: local.iter().map(|m| m.name.clone()).collect(),
            migrations_dir: dir.to_string_lossy().replace('\\', "/"),
            ..Default::default()
        };
        for r in &live {
            if r.finished {
                st.applied.push(r.name.clone());
                match local.iter().find(|m| m.name == r.name) {
                    None => st.missing_locally.push(r.name.clone()),
                    Some(m) if m.checksum != r.checksum => st.modified.push(r.name.clone()),
                    _ => {}
                }
            } else {
                st.failed.push(r.name.clone());
            }
            let _ = &r.id;
        }
        st.pending = local
            .iter()
            .filter(|m| !live.iter().any(|r| r.finished && r.name == m.name))
            .filter(|m| !st.failed.contains(&m.name))
            .map(|m| m.name.clone())
            .collect();
        Ok(st)
    }
}

/// `(sqlstate, message)` of a database error, formatted like the reference schema engine.
fn db_error_parts(e: &sqlx::Error, d: Dialect) -> (String, String) {
    match e {
        sqlx::Error::Database(db) => {
            let code = db.code().map(|c| c.to_string()).unwrap_or_default();
            let mut message = db.message().to_string();
            if d == Dialect::Postgres
                && let Some(pg) = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
            {
                message = format!("ERROR: {}", pg.message());
                if let Some(detail) = pg.detail() {
                    message.push_str(&format!(
                        "
DETAIL: {detail}"
                    ));
                }
                if let Some(hint) = pg.hint() {
                    message.push_str(&format!(
                        "
HINT: {hint}"
                    ));
                }
            }
            (code, message)
        }
        other => (String::new(), other.to_string()),
    }
}
