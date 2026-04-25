use std::path::PathBuf;

/// Where Golish stores its PostgreSQL data and binaries.
fn golish_data_dir() -> PathBuf {
    golish_core::paths::app_data_base().expect("cannot resolve home directory")
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
