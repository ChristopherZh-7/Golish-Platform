use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IntegrationStatus {
    NotInstalled,
    Installed {
        version: String,
    },
    Outdated {
        current: String,
        latest: String,
    },
    /// Shell integration files exist but .zshrc points to wrong path
    Misconfigured {
        expected_path: String,
        issue: String,
    },
}

pub(crate) fn get_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("golish"))
}

pub(crate) fn get_integration_path() -> Option<PathBuf> {
    get_config_dir().map(|p| p.join("integration.zsh"))
}

pub(crate) fn get_version_path() -> Option<PathBuf> {
    get_config_dir().map(|p| p.join("integration.version"))
}

pub(crate) fn get_zshrc_path() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".zshrc"))
}
