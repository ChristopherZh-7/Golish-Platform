//! Helpers shared between the vuln-intel command modules.

use golish_vuln_intel as intel;

use golish_app_core::GolishError;
use golish_settings::SettingsManager;

/// Build a `reqwest` client + resolve the GitHub token from settings.
///
/// Used by every command in [`super::enrichment`] to talk to GitHub APIs.
pub(super) async fn github_client_from_settings(
    settings_mgr: &SettingsManager,
) -> Result<(reqwest::Client, Option<String>), GolishError> {
    let settings = settings_mgr.get().await;
    let github_token = settings
        .api_keys
        .github
        .clone()
        .or_else(|| settings.network.github_token.clone());
    let proxy_url = settings.network.proxy_url.as_deref();
    let client = intel::build_github_client(proxy_url)?;
    Ok((client, github_token))
}
