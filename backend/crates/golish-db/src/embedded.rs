use std::path::{Path, PathBuf};
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
    /// Download (if needed), initialize, and start the embedded PostgreSQL server.
    /// On first run this downloads ~30 MB of PG binaries; subsequent starts are fast.
    pub async fn start(config: DbConfig) -> Result<Self> {
        info!(
            port = config.port,
            data_dir = %config.pg_data_dir.display(),
            "Starting embedded PostgreSQL"
        );

        std::fs::create_dir_all(&config.pg_data_dir)
            .context("Failed to create PG data directory")?;
        std::fs::create_dir_all(&config.pg_bin_cache_dir)
            .context("Failed to create PG binary cache directory")?;

        // If binaries aren't extracted in the cache yet, extract from the
        // downloaded zip before pg-embed's setup() — avoids a slow re-download.
        let cache_dir = Self::cache_dir();
        if !cache_dir.join("bin").join("initdb").exists() {
            Self::try_extract_from_cache(&config)?;
        }

        // macOS: remove quarantine BEFORE setup() — initdb and pg_ctl
        // will fail if Gatekeeper blocks execution of the unsigned binaries.
        // Binaries live in the cache dir, not the database dir.
        #[cfg(target_os = "macos")]
        Self::clear_quarantine(&Self::cache_dir());

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
            tracing::error!(error = ?e, "pg-embed setup failed");
            return Err(anyhow::anyhow!("PostgreSQL setup failed: {e:?}"));
        }

        Self::try_install_pgvector(&pg).await;

        info!("Starting PostgreSQL server on port {}...", config.port);
        if let Err(e) = pg.start_db().await {
            warn!(error = ?e, "pg-embed start_db failed, attempting manual pg_ctl start");

            match Self::manual_pg_ctl_start(&config).await {
                Ok(()) => {
                    info!("Manual pg_ctl start succeeded");
                }
                Err(manual_err) => {
                    error!(
                        pg_embed_error = ?e,
                        manual_error = %manual_err,
                        "Both pg-embed and manual pg_ctl start failed"
                    );
                    return Err(anyhow::anyhow!(
                        "Failed to start PostgreSQL: pg-embed={e:?}, manual={manual_err}"
                    ));
                }
            }
        }

        if !pg
            .database_exists(&config.database)
            .await
            .unwrap_or(false)
        {
            info!(db = %config.database, "Creating database");
            pg.create_database(&config.database)
                .await
                .context("Failed to create database")?;
        }

        info!(port = config.port, "Embedded PostgreSQL is ready");

        Ok(Self { pg, config })
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

        if cache_dir.join("bin").join("initdb").exists() {
            info!("PG binaries already extracted in cache, skipping");
            return Ok(());
        }

        info!(
            zip = %cache_zip.display(),
            "Extracting PostgreSQL binaries from cache"
        );

        let tmp = cache_dir.join(".extract_tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;

        let status = std::process::Command::new("unzip")
            .args(["-o", "-q"])
            .arg(&cache_zip)
            .arg("-d")
            .arg(&tmp)
            .status()
            .context("Failed to run unzip")?;
        if !status.success() {
            warn!("unzip failed with status {status}, skipping cache extraction");
            let _ = std::fs::remove_dir_all(&tmp);
            return Ok(());
        }

        let txz = std::fs::read_dir(&tmp)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map_or(false, |ext| ext == "txz"));

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
        let (os, arch) = platform_strings();
        let version = PG_V17.0;
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("pg-embed")
            .join(os)
            .join(arch)
            .join(version)
    }

    fn zip_filename() -> String {
        let (os, arch) = platform_strings();
        format!("{os}-{arch}-{}.zip", PG_V17.0)
    }

    #[cfg(target_os = "macos")]
    fn clear_quarantine(pg_data_dir: &Path) {
        for subdir in &["bin", "lib"] {
            let dir = pg_data_dir.join(subdir);
            if dir.exists() {
                info!(dir = %dir.display(), "Clearing macOS quarantine attributes");
                let _ = std::process::Command::new("xattr")
                    .args(["-cr", &dir.to_string_lossy()])
                    .output();
            }
        }
    }

    /// Try to find and install the pgvector extension from system paths.
    ///
    /// Searches Homebrew and common system locations for a pre-built pgvector.
    /// Copies the shared library + extension SQL/control files into a staging
    /// directory, then calls `pg.install_extension()` to deploy them into the
    /// pg-embed cache. Runs between `setup()` and `start_db()`.
    async fn try_install_pgvector(pg: &PgEmbed) {
        let cache_dir = Self::cache_dir();
        let lib_marker = if cfg!(target_os = "macos") {
            cache_dir.join("lib").join("vector.dylib")
        } else if cfg!(target_os = "windows") {
            cache_dir.join("lib").join("vector.dll")
        } else {
            cache_dir.join("lib").join("vector.so")
        };

        if lib_marker.exists() {
            info!("pgvector already installed in pg-embed cache");
            return;
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
            if let Err(e) = std::fs::copy(src, staging.join(name)) {
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
            Ok(()) => info!("pgvector extension installed successfully"),
            Err(e) => warn!(error = ?e, "Failed to install pgvector extension"),
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
        let pg_ctl = cache_dir.join("bin").join("pg_ctl");
        let lib_dir = cache_dir.join("lib");
        let log_file = config.pg_data_dir.join("server.log");

        if !pg_ctl.exists() {
            return Err(anyhow::anyhow!(
                "pg_ctl not found at {}",
                pg_ctl.display()
            ));
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
                .and_then(|content| {
                    let lines: Vec<&str> = content.lines().collect();
                    let start = lines.len().saturating_sub(10);
                    Some(lines[start..].join("\n"))
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
        info!("Stopping embedded PostgreSQL");
        if let Err(e) = self.pg.stop_db().await {
            warn!(error = %e, "Error stopping embedded PostgreSQL");
        }
    }
}

/// Search common system paths for pgvector extension files.
/// Returns paths to the shared library (.dylib/.so) and the control + SQL files.
fn find_system_pgvector() -> Vec<PathBuf> {
    let lib_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let lib_candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            // Homebrew Apple Silicon
            PathBuf::from("/opt/homebrew/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/lib/postgresql/vector.dylib"),
            // Homebrew Intel
            PathBuf::from("/usr/local/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/usr/local/opt/postgresql@17/lib/postgresql/vector.dylib"),
            // Unversioned Homebrew
            PathBuf::from("/opt/homebrew/lib/postgresql/vector.dylib"),
            PathBuf::from("/usr/local/lib/postgresql/vector.dylib"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            PathBuf::from("/usr/lib/postgresql/17/lib/vector.so"),
            PathBuf::from("/usr/lib64/pgsql/vector.so"),
        ]
    } else {
        vec![]
    };

    let ext_candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/opt/homebrew/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/share/postgresql/extension"),
            PathBuf::from("/usr/local/share/postgresql/extension"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            PathBuf::from("/usr/share/postgresql/17/extension"),
            PathBuf::from("/usr/share/pgsql/extension"),
        ]
    } else {
        vec![]
    };

    let mut files = Vec::new();

    let lib_found = lib_candidates.iter().find(|p| p.exists());
    if lib_found.is_none() {
        // Also try pg_config --pkglibdir if available
        if let Ok(output) = std::process::Command::new("pg_config")
            .arg("--pkglibdir")
            .output()
        {
            if output.status.success() {
                let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let candidate = PathBuf::from(&dir).join(format!("vector.{lib_ext}"));
                if candidate.exists() {
                    files.push(candidate);
                }
            }
        }
    } else if let Some(path) = lib_found {
        files.push(path.clone());
    }

    let ext_found = ext_candidates.iter().find(|p| p.join("vector.control").exists());
    if let Some(ext_dir) = ext_found {
        if let Ok(entries) = std::fs::read_dir(ext_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("vector") && (name_str.ends_with(".control") || name_str.ends_with(".sql")) {
                    files.push(entry.path());
                }
            }
        }
    } else {
        // Fallback: pg_config --sharedir
        if let Ok(output) = std::process::Command::new("pg_config")
            .arg("--sharedir")
            .output()
        {
            if output.status.success() {
                let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let ext_dir = PathBuf::from(&dir).join("extension");
                if ext_dir.join("vector.control").exists() {
                    if let Ok(entries) = std::fs::read_dir(&ext_dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name();
                            let name_str = name.to_string_lossy();
                            if name_str.starts_with("vector") && (name_str.ends_with(".control") || name_str.ends_with(".sql")) {
                                files.push(entry.path());
                            }
                        }
                    }
                }
            }
        }
    }

    if !files.is_empty() {
        let has_lib = files.iter().any(|f| {
            f.extension().map_or(false, |e| e == "dylib" || e == "so" || e == "dll")
        });
        let has_control = files.iter().any(|f| {
            f.extension().map_or(false, |e| e == "control")
        });
        if !has_lib || !has_control {
            info!(
                has_lib,
                has_control,
                "Incomplete pgvector installation found, skipping"
            );
            return vec![];
        }
    }

    files
}

fn platform_strings() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64v8"
    } else {
        "amd64"
    };
    (os, arch)
}

impl Drop for EmbeddedPg {
    fn drop(&mut self) {
        // pg_embed handles cleanup on drop, but we log it
        tracing::debug!("EmbeddedPg instance dropped");
    }
}
