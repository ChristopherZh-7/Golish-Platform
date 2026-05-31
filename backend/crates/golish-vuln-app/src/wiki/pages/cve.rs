//! `wiki_create_cve` — scaffold a new CVE folder with README + PoC stub.

use golish_app_core::GolishError;
use tokio::fs;

use super::super::wiki_base_dir;
use super::templates::cve_scaffold;

#[tauri::command]
pub async fn wiki_create_cve(
    cve_id: String,
    title: String,
    poc_lang: Option<String>,
) -> Result<String, GolishError> {
    let base = wiki_base_dir();
    let folder = base.join(&cve_id);
    if folder.exists() {
        return Err(GolishError::Validation(format!(
            "folder already exists: {cve_id}"
        )));
    }
    fs::create_dir_all(&folder)
        .await
        .map_err(|e| GolishError::Internal(format!("mkdir failed: {e}")))?;

    let (readme, poc_name, poc_content) = cve_scaffold(&cve_id, &title, poc_lang.as_deref());

    fs::write(folder.join("README.md"), &readme)
        .await
        .map_err(|e| GolishError::Internal(format!("write README failed: {e}")))?;

    fs::write(folder.join(&poc_name), &poc_content)
        .await
        .map_err(|e| GolishError::Internal(format!("write POC failed: {e}")))?;

    Ok(format!("{cve_id}/README.md"))
}
