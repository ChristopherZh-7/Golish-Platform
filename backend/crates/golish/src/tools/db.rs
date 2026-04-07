use rusqlite::{Connection, Result as SqlResult, params};
use std::path::PathBuf;

const SCHEMA_VERSION: i32 = 1;

pub fn resolve_db_path(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("data.db");
        }
    }
    let home = dirs::home_dir().expect("cannot resolve home directory");
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    base.join("data.db")
}

/// Open (or create) the project SQLite database and ensure the schema is up to date.
pub fn open_db(project_path: Option<&str>) -> Result<Connection, String> {
    let db_path = resolve_db_path(project_path);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .map_err(|e| e.to_string())?;

    migrate(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS _meta (key TEXT PRIMARY KEY, value TEXT)")?;

    let version: i32 = conn
        .query_row(
            "SELECT value FROM _meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|v| v.parse().unwrap_or(0))
        .unwrap_or(0);

    if version < 1 {
        create_schema_v1(conn)?;
    }

    conn.execute(
        "INSERT OR REPLACE INTO _meta (key, value) VALUES ('schema_version', ?1)",
        params![SCHEMA_VERSION.to_string()],
    )?;

    Ok(())
}

fn create_schema_v1(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "
        -- Targets
        CREATE TABLE IF NOT EXISTS targets (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            target_type TEXT NOT NULL DEFAULT 'domain',
            value       TEXT NOT NULL,
            tags        TEXT NOT NULL DEFAULT '[]',
            notes       TEXT NOT NULL DEFAULT '',
            scope       TEXT NOT NULL DEFAULT 'in',
            grp         TEXT NOT NULL DEFAULT 'default',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS target_groups (
            name TEXT PRIMARY KEY
        );
        INSERT OR IGNORE INTO target_groups (name) VALUES ('default');

        -- Findings
        CREATE TABLE IF NOT EXISTS findings (
            id          TEXT PRIMARY KEY,
            title       TEXT NOT NULL,
            severity    TEXT NOT NULL DEFAULT 'info',
            cvss        REAL,
            url         TEXT NOT NULL DEFAULT '',
            target      TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            steps       TEXT NOT NULL DEFAULT '',
            remediation TEXT NOT NULL DEFAULT '',
            tags        TEXT NOT NULL DEFAULT '[]',
            tool        TEXT NOT NULL DEFAULT '',
            template    TEXT NOT NULL DEFAULT '',
            refs        TEXT NOT NULL DEFAULT '[]',
            evidence    TEXT NOT NULL DEFAULT '[]',
            status      TEXT NOT NULL DEFAULT 'open',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_findings_severity ON findings(severity);
        CREATE INDEX IF NOT EXISTS idx_findings_status ON findings(status);
        CREATE INDEX IF NOT EXISTS idx_findings_target ON findings(target);

        -- Notes
        CREATE TABLE IF NOT EXISTS notes (
            id          TEXT PRIMARY KEY,
            entity_type TEXT NOT NULL,
            entity_id   TEXT NOT NULL,
            content     TEXT NOT NULL,
            color       TEXT NOT NULL DEFAULT 'yellow',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_notes_entity ON notes(entity_type, entity_id);

        -- Audit log
        CREATE TABLE IF NOT EXISTS audit_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp   INTEGER NOT NULL,
            action      TEXT NOT NULL,
            category    TEXT NOT NULL DEFAULT 'general',
            details     TEXT NOT NULL DEFAULT '',
            entity_type TEXT,
            entity_id   TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_cat ON audit_log(category);

        -- Vault entries (values stored obfuscated)
        CREATE TABLE IF NOT EXISTS vault_entries (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            entry_type  TEXT NOT NULL DEFAULT 'password',
            value       TEXT NOT NULL DEFAULT '',
            username    TEXT NOT NULL DEFAULT '',
            notes       TEXT NOT NULL DEFAULT '',
            project     TEXT NOT NULL DEFAULT '',
            tags        TEXT NOT NULL DEFAULT '[]',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        -- Topology scans (store full JSON blob per named scan)
        CREATE TABLE IF NOT EXISTS topology_scans (
            name       TEXT PRIMARY KEY,
            data       TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        -- Methodology projects (store full JSON blob)
        CREATE TABLE IF NOT EXISTS methodology_projects (
            id         TEXT PRIMARY KEY,
            data       TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        -- Pipelines (store full JSON blob)
        CREATE TABLE IF NOT EXISTS pipelines (
            id         TEXT PRIMARY KEY,
            data       TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        ",
    )?;
    Ok(())
}
