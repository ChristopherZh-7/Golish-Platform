//! Domain error types for the vulnerability intelligence crate.

#[derive(Debug, thiserror::Error)]
pub enum VulnIntelError {
    #[error("feed fetch: {0}")]
    FeedFetch(String),

    #[error("nuclei: {0}")]
    Nuclei(String),

    #[error("github: {0}")]
    Github(String),

    #[error("db: {0}")]
    Db(#[from] sqlx::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type VulnIntelResult<T> = Result<T, VulnIntelError>;

impl From<golish_db::DbError> for VulnIntelError {
    fn from(err: golish_db::DbError) -> Self {
        match err {
            golish_db::DbError::Sqlx(e) => Self::Db(e),
            other => Self::Other(other.into()),
        }
    }
}
