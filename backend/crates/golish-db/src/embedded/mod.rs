mod platform;
use platform::{copy_binary, find_system_pgvector};

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V17};
use pg_embed::postgres::{PgEmbed, PgSettings};
use tracing::{error, info, warn};

use crate::config::DbConfig;

/// Manages an embedded PostgreSQL instance that lives as long as the app.
pub struct EmbeddedPg {
    pg: PgEmbed,
    config: DbConfig,
}

impl EmbeddedPg {
    /// Tweak the process environment so the embedded PostgreSQL child
    /// processes produce stable, UTF-8-decodable output.
    ///
    /// `pg-embed` 1.0.0 strictly decodes child stdout/stderr as UTF-8.
    /// On non-English Windows installs (Chinese/Japanese/Korean) the
    /// default OEM code page is not UTF-8, so localized error messages
    /// from `initdb.exe` / `postgres.exe` make `pg-embed` blow up with
    /// `Error reading process output: stream did not contain valid
    /// UTF-8` → `PgInitFailure`. Forcing the C locale keeps postgres
    /// messages plain ASCII (always valid UTF-8) and flipping the
    /// console code page to 65001 makes any stray localized byte still
    /// land as UTF-8.
    fn configure_postgres_environment() {
        std::env::set_var("PGCLIENTENCODING", "UTF8");

        #[cfg(target_os = "windows")]
        {
            std::env::set_var("LC_ALL", "C");
            std::env::set_var("LC_CTYPE", "C");
            std::env::set_var("LC_MESSAGES", "C");
            std::env::set_var("LANG", "C");

            // Best-effort: switch the process console code page to UTF-8
            // (65001) for any postgres message that ignores LC_*. Errors
            // (e.g. no attached console under Tauri) are ignored.
            unsafe {
                extern "system" {
                    fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
                    fn SetConsoleCP(wCodePageID: u32) -> i32;
                }
                let _ = SetConsoleOutputCP(65001);
                let _ = SetConsoleCP(65001);
            }
        }
    }

    /// Download (if needed), initialize, and start the embedded PostgreSQL server.
    ///
    /// On first run this downloads ~30 MB of PG binaries; subsequent starts
    /// are fast. If the first attempt fails with a recoverable error
    /// (corrupted download, half-initialized data directory, etc.) the
    /// pg-embed binary cache and the pgdata directory are purged and the
    /// whole setup is retried once. This avoids requiring users to run a
    /// manual cleanup script every time the cache or data directory get
    /// stuck in a bad state — a common failure mode on Windows where
    /// downloads or unpacks can be interrupted.
    pub async fn start(config: DbConfig) -> Result<Self> {
        Self::configure_postgres_environment();

        info!(
            port = config.port,
            data_dir = %config.pg_data_dir.display(),
            cache_dir = %config.pg_bin_cache_dir.display(),
            "Starting embedded PostgreSQL"
        );

        std::fs::create_dir_all(&config.pg_data_dir)
            .context("Failed to create PG data directory")?;
        std::fs::create_dir_all(&config.pg_bin_cache_dir)
            .context("Failed to create PG binary cache directory")?;

        // [DIAG] Snapshot pgdata + port state BEFORE we touch anything. Helps
        // distinguish "fresh install" vs "stale postmaster.pid from a crashed
        // previous run" vs "another PG holding the port" — the three root
        // causes we keep guessing at on Windows.
        Self::log_pg_data_state("pre-setup", &config);

        match Self::try_setup_and_start(config.clone()).await {
            Ok(this) => Ok(this),
            Err(first_err) => {
                let recoverable_reason = Self::is_recoverable_setup_error(&first_err);
                if recoverable_reason.is_none() {
                    error!(
                        error = ?first_err,
                        data_dir = %config.pg_data_dir.display(),
                        "PG startup failed with a NON-recoverable error; \
                         not touching pgdata"
                    );
                    return Err(first_err);
                }

                let reason = recoverable_reason.unwrap_or("unknown");
                warn!(
                    error = ?first_err,
                    matched_pattern = reason,
                    data_dir = %config.pg_data_dir.display(),
                    "PG startup failed with a RECOVERABLE error pattern; \
                     about to purge state and retry once"
                );
                Self::log_pg_data_state("pre-purge", &config);
                Self::purge_corrupted_state(&config);
                Self::log_pg_data_state("post-purge", &config);

                match Self::try_setup_and_start(config.clone()).await {
                    Ok(this) => {
                        info!(
                            matched_pattern = reason,
                            "Embedded PostgreSQL recovered after self-heal retry"
                        );
                        Ok(this)
                    }
                    Err(retry_err) => {
                        error!(
                            first_error = ?first_err,
                            retry_error = ?retry_err,
                            matched_pattern = reason,
                            "PG startup failed even after self-heal retry"
                        );
                        Err(anyhow::anyhow!(
                            "PostgreSQL failed to start after self-heal retry.\n\
                             First attempt: {first_err}\n\
                             Retry attempt: {retry_err}\n\
                             You may need to manually delete the data directory \
                             ({}) and the pg-embed cache, or check that the \
                             download was not blocked by a firewall / anti-virus.",
                            config.pg_data_dir.display()
                        ))
                    }
                }
            }
        }
    }

    /// One-shot attempt: download (if needed), initialize, and start the
    /// embedded PostgreSQL server. Called twice by [`Self::start`] with a
    /// state purge in between.
    async fn try_setup_and_start(config: DbConfig) -> Result<Self> {
        let cache_dir = Self::cache_dir();
        let initdb_name = if cfg!(windows) { "initdb.exe" } else { "initdb" };
        if !cache_dir.join("bin").join(initdb_name).exists() {
            Self::try_extract_from_cache(&config)?;
        }

        // macOS: remove quarantine BEFORE setup() — initdb and pg_ctl
        // will fail if Gatekeeper blocks execution of the unsigned binaries.
        // Binaries live in the cache dir, not the database dir.
        golish_platform::postgres::clear_quarantine_dirs(&cache_dir, &["bin", "lib"]);

        let pg_settings = PgSettings {
            database_dir: config.pg_data_dir.clone(),
            port: config.port,
            user: config.username.clone(),
            password: config.password.clone(),
            auth_method: PgAuthMethod::MD5,
            persistent: true,
            timeout: Some(Duration::from_secs(120)),
            migration_dir: None,
        };

        let fetch_settings = PgFetchSettings {
            version: PG_V17,
            ..Default::default()
        };

        info!("Creating PgEmbed instance...");
        let mut pg = PgEmbed::new(pg_settings, fetch_settings)
            .await
            .context("Failed to create PgEmbed instance")?;

        info!("Running pg-embed setup (download/extract/initdb)...");
        if let Err(e) = pg.setup().await {
            tracing::error!(
                error = ?e,
                data_dir = %config.pg_data_dir.display(),
                cache_dir = %Self::cache_dir().display(),
                "[PG-DIAG] pg-embed setup failed"
            );
            Self::log_pg_data_state("setup-failed", &config);
            return Err(anyhow::anyhow!("PostgreSQL setup failed: {e:?}"));
        }

        Self::try_install_pgvector(&pg).await;

        info!("Starting PostgreSQL server on port {}...", config.port);
        if let Err(e) = pg.start_db().await {
            warn!(
                error = ?e,
                "[PG-DIAG] pg-embed start_db failed, attempting manual pg_ctl start"
            );
            Self::log_pg_data_state("start_db-failed", &config);
            // Show what postgres wrote to its log just before bailing, if any.
            if let Some(tail) = Self::tail_server_log(&config, 20) {
                warn!(
                    server_log_tail = %tail,
                    "[PG-DIAG] last lines of pgdata/server.log after start_db failure"
                );
            }

            match Self::manual_pg_ctl_start(&config).await {
                Ok(()) => {
                    info!("Manual pg_ctl start succeeded");
                }
                Err(manual_err) => {
                    error!(
                        pg_embed_error = ?e,
                        manual_error = %manual_err,
                        "[PG-DIAG] Both pg-embed and manual pg_ctl start failed"
                    );
                    Self::log_pg_data_state("manual_pg_ctl-failed", &config);
                    if let Some(tail) = Self::tail_server_log(&config, 20) {
                        warn!(
                            server_log_tail = %tail,
                            "[PG-DIAG] last lines of pgdata/server.log after manual pg_ctl failure"
                        );
                    }
                    return Err(anyhow::anyhow!(
                        "Failed to start PostgreSQL: pg-embed={e:?}, manual={manual_err}"
                    ));
                }
            }
        }

        if !pg.database_exists(&config.database).await.unwrap_or(false) {
            info!(db = %config.database, "Creating database");
            pg.create_database(&config.database)
                .await
                .context("Failed to create database")?;
        }

        info!(port = config.port, "Embedded PostgreSQL is ready");

        Ok(Self { pg, config })
    }

    /// Decide whether an error returned by [`Self::try_setup_and_start`] is
    /// likely caused by corrupted on-disk state (in which case purging the
    /// cache + data dir and retrying makes sense) rather than a permanent
    /// configuration problem (port conflict, missing runtime, etc.).
    ///
    /// Returns `Some(pattern)` with the substring that matched (so the caller
    /// can log *why* we decided to purge) or `None` if the error looks
    /// unrelated to on-disk corruption.
    fn is_recoverable_setup_error(err: &anyhow::Error) -> Option<&'static str> {
        let msg = format!("{err:?}");
        // pg-embed wraps zip errors here when the cached download is partial.
        const PATTERNS: &[&str] = &[
            "UnpackFailure",
            "PgInitFailure",
            "PgPurgeFailure",
            "PgStartFailure",
            "invalid zip archive",
        ];
        PATTERNS.iter().copied().find(|p| msg.contains(p))
    }

    /// Delete the pg-embed binary cache and the pgdata directory so the
    /// next [`Self::try_setup_and_start`] starts from a known-clean slate.
    ///
    /// We intentionally swallow IO errors here — if a delete fails because
    /// a stale `postgres.exe` is still locking files, the next setup will
    /// surface a clearer error than what we could synthesise.
    fn purge_corrupted_state(config: &DbConfig) {
        let cache_dir = Self::cache_dir();
        if cache_dir.exists() {
            let (cache_entries, cache_total_bytes) = Self::dir_summary(&cache_dir);
            warn!(
                cache_dir = %cache_dir.display(),
                entries = cache_entries,
                total_bytes = cache_total_bytes,
                "[PURGE] Purging pg-embed binary cache (safe — only contains downloaded PG binaries)"
            );
            if let Err(e) = std::fs::remove_dir_all(&cache_dir) {
                warn!(error = %e, "Failed to fully purge pg-embed cache directory");
            }
        }

        if config.pg_data_dir.exists() {
            let (data_entries, data_total_bytes) = Self::dir_summary(&config.pg_data_dir);
            let top_level_names: Vec<String> = std::fs::read_dir(&config.pg_data_dir)
                .ok()
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            warn!(
                data_dir = %config.pg_data_dir.display(),
                entries = data_entries,
                total_bytes = data_total_bytes,
                top_level = ?top_level_names,
                "[PURGE] About to DELETE pgdata directory — this will destroy any user data \
                 written in previous sessions. Search for this log line if your data \
                 mysteriously disappeared between app restarts."
            );
            if let Err(e) = std::fs::remove_dir_all(&config.pg_data_dir) {
                warn!(error = %e, "Failed to fully purge pgdata directory");
            }
            if let Err(e) = std::fs::create_dir_all(&config.pg_data_dir) {
                warn!(error = %e, "Failed to recreate pgdata directory after purge");
            }
        }
    }

    /// Read the last `n_lines` of `pgdata/server.log` for diagnostic
    /// logging. Returns `None` if the file doesn't exist or can't be read.
    fn tail_server_log(config: &DbConfig, n_lines: usize) -> Option<String> {
        let log_file = config.pg_data_dir.join("server.log");
        let content = std::fs::read_to_string(&log_file).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(n_lines);
        Some(lines[start..].join("\n"))
    }

    /// Best-effort `(entry_count, total_bytes)` snapshot of a directory.
    /// Used only for diagnostic logging — silently returns `(0, 0)` on
    /// any IO error.
    fn dir_summary(root: &std::path::Path) -> (usize, u64) {
        fn walk(p: &std::path::Path, entries: &mut usize, bytes: &mut u64) {
            let Ok(rd) = std::fs::read_dir(p) else { return };
            for entry in rd.flatten() {
                *entries += 1;
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        *bytes += meta.len();
                    } else if meta.is_dir() {
                        walk(&entry.path(), entries, bytes);
                    }
                }
            }
        }
        let (mut entries, mut bytes) = (0_usize, 0_u64);
        walk(root, &mut entries, &mut bytes);
        (entries, bytes)
    }

    /// Diagnostic snapshot of the embedded PG data directory.
    ///
    /// Logs at INFO level the things that most often explain "data
    /// disappeared between restarts" on Windows:
    /// - whether `PG_VERSION` exists (i.e. pgdata was previously initdb'd),
    /// - whether `postmaster.pid` is present and its first line (the PID),
    /// - whether that PID is currently running,
    /// - whether port `config.port` is being held by some other process,
    /// - top-level entry count under pgdata.
    ///
    /// `phase` is a short tag like `"pre-setup"` / `"pre-purge"` /
    /// `"post-purge"` so callers can correlate snapshots across the
    /// startup pipeline.
    fn log_pg_data_state(phase: &str, config: &DbConfig) {
        let data_dir = &config.pg_data_dir;
        let pg_version = data_dir.join("PG_VERSION");
        let postmaster_pid = data_dir.join("postmaster.pid");

        let pg_version_exists = pg_version.is_file();
        let pg_version_text = std::fs::read_to_string(&pg_version)
            .map(|s| s.trim().to_string())
            .ok();

        let postmaster_first_line = std::fs::read_to_string(&postmaster_pid)
            .ok()
            .and_then(|s| s.lines().next().map(|l| l.trim().to_string()));
        let recorded_pid = postmaster_first_line.as_ref().and_then(|s| s.parse::<u32>().ok());
        let recorded_pid_alive = recorded_pid.map(golish_platform::process::is_pid_running);

        let port_holders = golish_platform::process::pids_listening_on_port(config.port);
        let (entries, total_bytes) = if data_dir.exists() {
            Self::dir_summary(data_dir)
        } else {
            (0, 0)
        };

        info!(
            phase = phase,
            data_dir = %data_dir.display(),
            data_dir_exists = data_dir.exists(),
            pg_version_exists,
            pg_version = ?pg_version_text,
            postmaster_pid_exists = postmaster_pid.is_file(),
            postmaster_recorded_pid = ?recorded_pid,
            postmaster_recorded_pid_alive = ?recorded_pid_alive,
            port = config.port,
            port_holders = ?port_holders,
            data_dir_entries = entries,
            data_dir_bytes = total_bytes,
            "[PG-DIAG] Embedded PG state snapshot"
        );
    }

    /// Locate the pg-embed binary cache zip and extract binaries into the
    /// cache directory (NOT into database_dir). pg-embed checks for
    /// `cache_dir/bin/initdb` to decide whether to download.
    fn try_extract_from_cache(_config: &DbConfig) -> Result<()> {
        let cache_dir = Self::cache_dir();
        let cache_zip = cache_dir.join(Self::zip_filename());

        if !cache_zip.exists() {
            info!("No cached PG binary found at {}", cache_zip.display());
            return Ok(());
        }

        let initdb_name = if cfg!(windows) { "initdb.exe" } else { "initdb" };
        if cache_dir.join("bin").join(initdb_name).exists() {
            info!("PG binaries already extracted in cache, skipping");
            return Ok(());
        }

        let postgres_name = if cfg!(windows) { "postgres.exe" } else { "postgres" };
        if cache_dir.join("bin").join(postgres_name).exists() {
            info!("PG binaries already present (possibly in use), skipping extraction");
            return Ok(());
        }

        info!(
            zip = %cache_zip.display(),
            "Extracting PostgreSQL binaries from cache"
        );

        let tmp = cache_dir.join(".extract_tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;

        let zip_file = std::fs::File::open(&cache_zip)
            .context("Failed to open cached zip")?;
        let mut archive = zip::ZipArchive::new(zip_file)
            .context("Failed to read zip archive")?;
        if let Err(e) = archive.extract(&tmp) {
            warn!("zip extraction failed: {e}, skipping cache extraction");
            let _ = std::fs::remove_dir_all(&tmp);
            return Ok(());
        }

        let txz = std::fs::read_dir(&tmp)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|ext| ext == "txz"));

        if let Some(txz_path) = txz {
            let status = std::process::Command::new("tar")
                .args(["xJf"])
                .arg(&txz_path)
                .arg("-C")
                .arg(&cache_dir)
                .status()
                .context("Failed to run tar")?;
            if !status.success() {
                warn!("tar extraction failed with status {status}");
            } else {
                info!("Successfully extracted PG binaries to cache");
            }
        } else {
            warn!("No .txz found in cached zip, skipping");
        }

        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }

    /// Returns the pg-embed per-version cache directory.
    fn cache_dir() -> PathBuf {
        let (os, arch) = golish_platform::postgres::pg_embed_fetch_tag();
        let version = PG_V17.0;
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("pg-embed")
            .join(os)
            .join(arch)
            .join(version)
    }

    fn zip_filename() -> String {
        let (os, arch) = golish_platform::postgres::pg_embed_fetch_tag();
        format!("{os}-{arch}-{}.zip", PG_V17.0)
    }

    /// Try to find and install the pgvector extension from system paths.
    ///
    /// Searches Homebrew and common system locations for a pre-built pgvector.
    /// Copies the shared library + extension SQL/control files into a staging
    /// directory, then calls `pg.install_extension()` to deploy them into the
    /// pg-embed cache. Runs between `setup()` and `start_db()`.
    async fn try_install_pgvector(pg: &PgEmbed) {
        let cache_dir = Self::cache_dir();
        let ext_name = golish_platform::postgres::pgvector_library_name();

        // PostgreSQL loads extension libraries from lib/postgresql/ ($libdir),
        // NOT from lib/. Check the correct directory.
        let pkglib_dir = cache_dir.join("lib").join("postgresql");
        let pkglib_marker = pkglib_dir.join(&ext_name);

        if pkglib_marker.exists() {
            info!("pgvector already installed in pg-embed cache");
            return;
        }

        // Fix up: pg.install_extension() may have placed the .dylib in lib/
        // instead of lib/postgresql/. Relocate it.
        let misplaced = cache_dir.join("lib").join(&ext_name);
        if misplaced.exists() {
            info!("pgvector .dylib found in lib/ but not lib/postgresql/, relocating");
            let _ = std::fs::create_dir_all(&pkglib_dir);
            if copy_binary(&misplaced, &pkglib_marker).is_ok() {
                info!("pgvector .dylib relocated to lib/postgresql/ successfully");
                return;
            }
        }

        let found = find_system_pgvector();
        if found.is_empty() {
            info!(
                "pgvector not found in system paths. \
                 Install with: brew install pgvector (macOS) or \
                 apt install postgresql-17-pgvector (Linux). \
                 Falling back to application-level vector search."
            );
            return;
        }

        let staging = cache_dir.join(".pgvector_staging");
        let _ = std::fs::remove_dir_all(&staging);
        if let Err(e) = std::fs::create_dir_all(&staging) {
            warn!(error = %e, "Failed to create pgvector staging directory");
            return;
        }

        for src in &found {
            let name = match src.file_name() {
                Some(n) => n,
                None => continue,
            };
            if let Err(e) = copy_binary(src, &staging.join(name)) {
                warn!(src = %src.display(), error = %e, "Failed to copy pgvector file");
                let _ = std::fs::remove_dir_all(&staging);
                return;
            }
        }

        info!(
            files = found.len(),
            "Found pgvector in system, installing into pg-embed cache"
        );

        match pg.install_extension(&staging).await {
            Ok(()) => info!("pgvector extension installed successfully via pg-embed"),
            Err(e) => warn!(error = ?e, "Failed to install pgvector extension via pg-embed"),
        }

        // pg-embed's install_extension() puts .dylib in lib/ but PostgreSQL
        // loads from lib/postgresql/. Ensure it's in the right place.
        let _ = std::fs::create_dir_all(&pkglib_dir);
        if !pkglib_marker.exists() {
            // Try from the misplaced lib/ location
            let misplaced = cache_dir.join("lib").join(&ext_name);
            let src = if misplaced.exists() {
                Some(misplaced)
            } else {
                // Last resort: from staging
                let s = staging.join(&ext_name);
                s.exists().then_some(s)
            };
            if let Some(src) = src {
                match copy_binary(&src, &pkglib_marker) {
                    Ok(()) => info!("pgvector .dylib installed to lib/postgresql/"),
                    Err(e) => {
                        warn!(error = %e, "Failed to install pgvector .dylib to lib/postgresql/")
                    }
                }
            }
        }

        let _ = std::fs::remove_dir_all(&staging);
    }

    /// Fallback: start PostgreSQL using pg_ctl directly with a log file for diagnostics.
    ///
    /// pg-embed's `start_db()` sometimes fails on macOS because it doesn't propagate
    /// DYLD_LIBRARY_PATH or mishandles piped output. Running pg_ctl manually with
    /// explicit library path and a log file is more reliable.
    async fn manual_pg_ctl_start(config: &DbConfig) -> Result<()> {
        let cache_dir = Self::cache_dir();
        let pg_ctl_name = if cfg!(windows) { "pg_ctl.exe" } else { "pg_ctl" };
        let pg_ctl = cache_dir.join("bin").join(pg_ctl_name);
        let lib_dir = cache_dir.join("lib");
        let log_file = config.pg_data_dir.join("server.log");

        if !pg_ctl.exists() {
            return Err(anyhow::anyhow!("pg_ctl not found at {}", pg_ctl.display()));
        }

        // Check if PG is already running on this port
        if Self::is_port_in_use(config.port).await {
            info!(
                port = config.port,
                "Port already in use, assuming PostgreSQL is already running"
            );
            return Ok(());
        }

        let port_arg = format!("-F -p {}", config.port);
        let output = tokio::process::Command::new(&pg_ctl)
            .args([
                "start",
                "-w",
                "-D",
                &config.pg_data_dir.to_string_lossy(),
                "-o",
                &port_arg,
                "-l",
                &log_file.to_string_lossy(),
            ])
            .env("DYLD_LIBRARY_PATH", &lib_dir)
            .env("LD_LIBRARY_PATH", &lib_dir)
            .output()
            .await
            .context("Failed to spawn pg_ctl")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Read last few lines of the log file for more details
            let log_tail = std::fs::read_to_string(&log_file)
                .ok()
                .map(|content| {
                    let lines: Vec<&str> = content.lines().collect();
                    let start = lines.len().saturating_sub(10);
                    lines[start..].join("\n")
                })
                .unwrap_or_default();

            Err(anyhow::anyhow!(
                "pg_ctl start failed (exit={})\nstdout: {}\nstderr: {}\nlog tail: {}",
                output.status,
                stdout.trim(),
                stderr.trim(),
                log_tail.trim()
            ))
        }
    }

    /// Check if a port is already in use (another PG instance or other service).
    async fn is_port_in_use(port: u16) -> bool {
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
    }

    pub fn connection_string(&self) -> String {
        self.config.connection_string()
    }

    pub fn config(&self) -> &DbConfig {
        &self.config
    }

    /// Gracefully stop the embedded PostgreSQL server.
    pub async fn stop(&mut self) {
        info!("[PG-DIAG] EmbeddedPg::stop() entered — running pg_ctl stop");
        if let Err(e) = self.pg.stop_db().await {
            warn!(error = %e, "[PG-DIAG] Error stopping embedded PostgreSQL");
        } else {
            info!("[PG-DIAG] EmbeddedPg::stop() finished — pg_ctl stop returned OK");
        }
    }
}
