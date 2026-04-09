use std::path::PathBuf;

/// Where Golish stores its PostgreSQL data and binaries.
fn golish_data_dir() -> PathBuf {
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
    base
}

#[derive(Debug, Clone)]
pub struct DbConfig {
    pub pg_data_dir: PathBuf,
    pub pg_bin_cache_dir: PathBuf,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
}

impl Default for DbConfig {
    fn default() -> Self {
        let data = golish_data_dir();
        Self {
            pg_data_dir: data.join("pgdata"),
            pg_bin_cache_dir: data.join("pg_bin"),
            port: 15432,
            database: "golish".into(),
            username: "golish".into(),
            password: "golish_local".into(),
        }
    }
}

impl DbConfig {
    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}:{}@localhost:{}/{}",
            self.username, self.password, self.port, self.database
        )
    }
}
