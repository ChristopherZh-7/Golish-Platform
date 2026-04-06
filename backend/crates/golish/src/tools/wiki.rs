use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

const WIKI_EXTENSIONS: &[&str] = &[
    ".md", ".txt", ".py", ".sh", ".bash", ".zsh", ".go", ".rs", ".rb", ".pl",
    ".js", ".ts", ".jsx", ".tsx", ".c", ".cpp", ".h", ".hpp", ".java", ".cs",
    ".swift", ".kt", ".lua", ".r", ".ps1", ".bat", ".cmd", ".php", ".html",
    ".css", ".xml", ".json", ".yaml", ".yml", ".toml", ".ini", ".conf", ".cfg",
    ".sql", ".graphql", ".proto", ".dockerfile", ".nse",
];

fn is_wiki_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    WIKI_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        || lower == "dockerfile"
        || lower == "makefile"
        || lower == "rakefile"
}

fn is_text_searchable(name: &str) -> bool {
    is_wiki_file(name)
}

fn wiki_base_dir() -> PathBuf {
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
    base.join("wiki")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WikiEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSearchResult {
    pub path: String,
    pub name: String,
    pub line: usize,
    pub content: String,
}

async fn build_tree(dir: &std::path::Path, prefix: &str) -> std::io::Result<Vec<WikiEntry>> {
    let mut entries = Vec::new();
    let mut rd = fs::read_dir(dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata().await?;
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        if meta.is_dir() {
            let children = Box::pin(build_tree(&entry.path(), &rel)).await?;
            entries.push(WikiEntry {
                path: rel,
                name,
                is_dir: true,
                children: Some(children),
                size: None,
                modified: None,
            });
        } else if is_wiki_file(&name) {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            entries.push(WikiEntry {
                path: rel,
                name,
                is_dir: false,
                children: None,
                size: Some(meta.len()),
                modified,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

#[tauri::command]
pub async fn wiki_list() -> Result<Vec<WikiEntry>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        fs::create_dir_all(&base)
            .await
            .map_err(|e| format!("cannot create wiki dir: {e}"))?;
        return Ok(Vec::new());
    }
    build_tree(&base, "").await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wiki_read(path: String) -> Result<String, String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Err(format!("file not found: {path}"));
    }
    fs::read_to_string(&full)
        .await
        .map_err(|e| format!("read failed: {e}"))
}

#[tauri::command]
pub async fn wiki_write(path: String, content: String) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::write(&full, &content)
        .await
        .map_err(|e| format!("write failed: {e}"))
}

#[tauri::command]
pub async fn wiki_delete(path: String) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Ok(());
    }
    let meta = fs::metadata(&full)
        .await
        .map_err(|e| format!("stat failed: {e}"))?;
    if meta.is_dir() {
        fs::remove_dir_all(&full)
            .await
            .map_err(|e| format!("rmdir failed: {e}"))
    } else {
        fs::remove_file(&full)
            .await
            .map_err(|e| format!("rm failed: {e}"))
    }
}

#[tauri::command]
pub async fn wiki_rename(old_path: String, new_path: String) -> Result<(), String> {
    let base = wiki_base_dir();
    let from = base.join(&old_path);
    let to = base.join(&new_path);
    if !from.exists() {
        return Err(format!("source not found: {old_path}"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::rename(&from, &to)
        .await
        .map_err(|e| format!("rename failed: {e}"))
}

#[tauri::command]
pub async fn wiki_create_dir(path: String) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    fs::create_dir_all(&full)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))
}

#[tauri::command]
pub async fn wiki_search(query: String) -> Result<Vec<WikiSearchResult>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        return Ok(Vec::new());
    }
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut stack = vec![base.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_text_searchable(&name) {
                continue;
            }
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if name.to_lowercase().contains(&query_lower) {
                results.push(WikiSearchResult {
                    path: rel.clone(),
                    name: name.clone(),
                    line: 0,
                    content: name.clone(),
                });
            }

            if let Ok(content) = fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&query_lower) {
                        results.push(WikiSearchResult {
                            path: rel.clone(),
                            name: name.clone(),
                            line: i + 1,
                            content: line.chars().take(200).collect(),
                        });
                        if results.len() >= 100 {
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

#[tauri::command]
pub async fn wiki_create_cve(
    cve_id: String,
    title: String,
    poc_lang: Option<String>,
) -> Result<String, String> {
    let base = wiki_base_dir();
    let folder = base.join(&cve_id);
    if folder.exists() {
        return Err(format!("folder already exists: {cve_id}"));
    }
    fs::create_dir_all(&folder)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))?;

    let readme = format!(
        "# {cve_id}: {title}\n\n\
         ## 概述\n\n\
         <!-- 漏洞描述 -->\n\n\
         ## 影响范围\n\n\
         - 产品/版本:\n\
         - CVSS:\n\
         - 类型:\n\n\
         ## 复现步骤\n\n\
         1. \n\n\
         ## POC\n\n\
         参见 `poc` 文件。\n\n\
         ## 修复建议\n\n\
         <!-- 修复方案 -->\n\n\
         ## 参考\n\n\
         - https://nvd.nist.gov/vuln/detail/{cve_id}\n"
    );
    fs::write(folder.join("README.md"), &readme)
        .await
        .map_err(|e| format!("write README failed: {e}"))?;

    let lang = poc_lang.as_deref().unwrap_or("py");
    let ext = lang;
    let poc_name = format!("poc.{ext}");
    let poc_content = match lang {
        "py" => format!(
            "#!/usr/bin/env python3\n\
             \"\"\"POC for {cve_id}: {title}\"\"\"\n\n\
             import requests\nimport sys\n\n\
             def exploit(target: str):\n\
             \x20   # TODO: implement\n\
             \x20   pass\n\n\
             if __name__ == \"__main__\":\n\
             \x20   if len(sys.argv) < 2:\n\
             \x20       print(f\"Usage: {{sys.argv[0]}} <target>\")\n\
             \x20       sys.exit(1)\n\
             \x20   exploit(sys.argv[1])\n"
        ),
        "go" => format!(
            "package main\n\n\
             // POC for {cve_id}: {title}\n\n\
             import (\n\t\"fmt\"\n\t\"net/http\"\n\t\"os\"\n)\n\n\
             func exploit(target string) error {{\n\
             \t// TODO: implement\n\
             \treturn nil\n\
             }}\n\n\
             func main() {{\n\
             \tif len(os.Args) < 2 {{\n\
             \t\tfmt.Fprintf(os.Stderr, \"Usage: %s <target>\\n\", os.Args[0])\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             \tif err := exploit(os.Args[1]); err != nil {{\n\
             \t\tfmt.Fprintln(os.Stderr, err)\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             }}\n"
        ),
        "sh" | "bash" => format!(
            "#!/usr/bin/env bash\n\
             # POC for {cve_id}: {title}\n\n\
             set -euo pipefail\n\n\
             TARGET=\"${{1:?Usage: $0 <target>}}\"\n\n\
             # TODO: implement\n\
             echo \"[*] Target: $TARGET\"\n"
        ),
        _ => format!("// POC for {cve_id}: {title}\n// TODO: implement\n"),
    };
    fs::write(folder.join(&poc_name), &poc_content)
        .await
        .map_err(|e| format!("write POC failed: {e}"))?;

    Ok(format!("{cve_id}/README.md"))
}
