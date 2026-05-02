use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct ThemeInfo {
    pub name: String,
    pub path: String,
}

/// Get the golish themes directory path (~/.golish/themes/)
fn get_themes_dir() -> Result<PathBuf, GolishError> {
    let home_dir = dirs::home_dir().ok_or_else(|| {
        GolishError::Internal("Could not determine home directory".into())
    })?;

    let golish_dir = home_dir.join(".golish");
    let themes_dir = golish_dir.join("themes");

    fs::create_dir_all(&themes_dir).map_err(|e| {
        GolishError::Internal(format!("Failed to create themes directory: {}", e))
    })?;

    Ok(themes_dir)
}

/// List all available themes in ~/.golish/themes/
#[tauri::command]
pub async fn list_themes() -> Result<Vec<ThemeInfo>, GolishError> {
    let themes_dir = get_themes_dir()?;
    let mut themes = Vec::new();

    let entries = fs::read_dir(&themes_dir).map_err(|e| {
        GolishError::Internal(format!("Failed to read themes directory: {}", e))
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            GolishError::Internal(format!("Failed to read directory entry: {}", e))
        })?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let theme_file = path.join("theme.json");
                if theme_file.exists() {
                    themes.push(ThemeInfo {
                        name: name.to_string(),
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }

    Ok(themes)
}

/// Read a theme file from ~/.golish/themes/{theme_name}/theme.json
#[tauri::command]
pub async fn read_theme(theme_name: String) -> Result<String, GolishError> {
    let themes_dir = get_themes_dir()?;
    let theme_path = themes_dir.join(&theme_name).join("theme.json");

    if !theme_path.exists() {
        return Err(GolishError::NotFound(format!(
            "Theme '{}' not found",
            theme_name
        )));
    }

    fs::read_to_string(&theme_path)
        .map_err(|e| GolishError::Internal(format!("Failed to read theme file: {}", e)))
}

/// Save a theme to ~/.golish/themes/{theme_name}/theme.json
#[tauri::command]
pub async fn save_theme(theme_name: String, theme_data: String) -> Result<String, GolishError> {
    let themes_dir = get_themes_dir()?;
    let theme_dir = themes_dir.join(&theme_name);

    fs::create_dir_all(&theme_dir).map_err(|e| {
        GolishError::Internal(format!("Failed to create theme directory: {}", e))
    })?;

    let theme_file = theme_dir.join("theme.json");
    fs::write(&theme_file, theme_data)
        .map_err(|e| GolishError::Internal(format!("Failed to write theme file: {}", e)))?;

    Ok(theme_dir.to_string_lossy().to_string())
}

/// Delete a theme from ~/.golish/themes/{theme_name}/
#[tauri::command]
pub async fn delete_theme(theme_name: String) -> Result<(), GolishError> {
    let themes_dir = get_themes_dir()?;
    let theme_dir = themes_dir.join(&theme_name);

    if !theme_dir.exists() {
        return Err(GolishError::NotFound(format!(
            "Theme '{}' not found",
            theme_name
        )));
    }

    fs::remove_dir_all(&theme_dir)
        .map_err(|e| GolishError::Internal(format!("Failed to delete theme: {}", e)))
}

/// Save a theme asset (like background images) to ~/.golish/themes/{theme_name}/assets/{filename}
#[tauri::command]
pub async fn save_theme_asset(
    theme_name: String,
    filename: String,
    data: Vec<u8>,
) -> Result<String, GolishError> {
    let themes_dir = get_themes_dir()?;
    let assets_dir = themes_dir.join(&theme_name).join("assets");

    fs::create_dir_all(&assets_dir).map_err(|e| {
        GolishError::Internal(format!("Failed to create assets directory: {}", e))
    })?;

    let asset_path = assets_dir.join(&filename);
    fs::write(&asset_path, data)
        .map_err(|e| GolishError::Internal(format!("Failed to write asset file: {}", e)))?;

    Ok(format!("assets/{}", filename))
}

/// Get the absolute path to a theme asset
#[tauri::command]
pub async fn get_theme_asset_path(
    theme_name: String,
    asset_path: String,
) -> Result<String, GolishError> {
    let themes_dir = get_themes_dir()?;
    let full_path = themes_dir.join(&theme_name).join(&asset_path);

    if !full_path.exists() {
        return Err(GolishError::NotFound(format!(
            "Asset not found: {}",
            asset_path
        )));
    }

    Ok(format!("asset://localhost/{}", full_path.to_string_lossy()))
}
