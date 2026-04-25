use crate::tools::output_parser::OutputParserConfig;

pub(super) fn app_data_dirs() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    Some((golish_core::paths::toolsconfig_dir()?, golish_core::paths::tools_dir()?))
}

pub(super) fn find_tool_json(tool_name: &str) -> Option<serde_json::Value> {
    let (config_dir, _) = app_data_dirs()?;
    if !config_dir.exists() { return None; }
    let lower = tool_name.to_lowercase();
    for entry in walkdir::WalkDir::new(&config_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                    let name = val.pointer("/tool/name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.to_lowercase() == lower {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

pub(super) fn load_tool_output_config(tool_name: &str) -> Option<OutputParserConfig> {
    let val = find_tool_json(tool_name)?;
    val.pointer("/tool/output").and_then(|o| serde_json::from_value(o.clone()).ok())
}

pub(super) async fn resolve_tool_command(bare_cmd: &str, config_manager: &golish_pentest::ConfigManager) -> String {
    let Some(val) = find_tool_json(bare_cmd) else { return bare_cmd.to_string() };

    let tool_config: golish_pentest::ToolConfig = match serde_json::from_value(val["tool"].clone()) {
        Ok(tc) => tc,
        Err(_) => return bare_cmd.to_string(),
    };

    let config = config_manager.get().await;
    let ctx = golish_pentest::CommandContext {
        tools_dir: config.tools_dir(),
        conda_base: config.conda_path(),
        nvm_path: config.nvm_path(),
    };

    match golish_pentest::build_run_command(&tool_config, "", &ctx).await {
        Ok(result) => result.command,
        Err(e) => {
            tracing::warn!("[pipeline] build_run_command failed for '{}': {e}", bare_cmd);
            bare_cmd.to_string()
        }
    }
}
