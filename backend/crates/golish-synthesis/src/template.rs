use std::path::PathBuf;

/// Generate a commit message using template/rule-based approach
pub fn generate_template_message(files: &[PathBuf], diff: &str) -> String {
    // Analyze the changes
    let analysis = analyze_changes(files, diff);

    // Determine commit type
    let commit_type = infer_commit_type(&analysis);

    // Determine scope
    let scope = infer_scope(files);

    // Generate subject line
    let subject = generate_subject(&analysis, commit_type);

    // Generate body (optional, for more complex changes)
    let body = generate_body(&analysis);

    // Format the message
    if let Some(s) = scope {
        if body.is_empty() {
            format!("{}({}): {}", commit_type, s, subject)
        } else {
            format!("{}({}): {}\n\n{}", commit_type, s, subject, body)
        }
    } else if body.is_empty() {
        format!("{}: {}", commit_type, subject)
    } else {
        format!("{}: {}\n\n{}", commit_type, subject, body)
    }
}

/// Analysis of changes for template-based generation
#[derive(Debug, Default)]
struct ChangeAnalysis {
    /// Number of files added
    files_added: usize,
    /// Number of files modified
    files_modified: usize,
    /// Number of files deleted
    files_deleted: usize,
    /// Number of lines added
    lines_added: usize,
    /// Number of lines deleted
    lines_deleted: usize,
    /// Whether changes appear to be tests
    is_test: bool,
    /// Whether changes appear to be documentation
    is_docs: bool,
    /// Whether changes appear to be configuration
    is_config: bool,
    /// Key file names
    key_files: Vec<String>,
}

fn analyze_changes(files: &[PathBuf], diff: &str) -> ChangeAnalysis {
    let mut analysis = ChangeAnalysis::default();

    // Count files by type
    for file in files {
        let filename = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let path_str = file.to_string_lossy().to_lowercase();

        // Check file type
        if path_str.contains("test") || path_str.contains("spec") {
            analysis.is_test = true;
        }
        if filename.ends_with(".md")
            || filename.ends_with(".txt")
            || path_str.contains("doc")
            || filename == "README"
        {
            analysis.is_docs = true;
        }
        if filename.ends_with(".toml")
            || filename.ends_with(".json")
            || filename.ends_with(".yaml")
            || filename.ends_with(".yml")
            || filename == ".env"
            || filename.starts_with(".")
        {
            analysis.is_config = true;
        }

        // Extract key file name (without extension)
        if let Some(stem) = file.file_stem().and_then(|s| s.to_str()) {
            if !analysis.key_files.contains(&stem.to_string()) && analysis.key_files.len() < 3 {
                analysis.key_files.push(stem.to_string());
            }
        }
    }

    // Analyze diff
    for line in diff.lines() {
        if line.starts_with("new file") {
            analysis.files_added += 1;
        } else if line.starts_with("deleted file") {
            analysis.files_deleted += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            analysis.lines_added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            analysis.lines_deleted += 1;
        }
    }

    // Modified = total - added - deleted
    analysis.files_modified = files
        .len()
        .saturating_sub(analysis.files_added + analysis.files_deleted);

    analysis
}

fn infer_commit_type(analysis: &ChangeAnalysis) -> &'static str {
    if analysis.is_test {
        return "test";
    }
    if analysis.is_docs {
        return "docs";
    }
    if analysis.is_config {
        return "chore";
    }
    if analysis.files_added > 0 && analysis.files_modified == 0 && analysis.files_deleted == 0 {
        return "feat";
    }
    if analysis.files_deleted > 0 && analysis.files_added == 0 {
        return "refactor";
    }
    if analysis.lines_deleted > analysis.lines_added * 2 {
        return "refactor";
    }
    if analysis.files_modified > 0 && analysis.files_added == 0 {
        // Small changes are likely fixes, larger ones are features
        if analysis.lines_added < 50 {
            return "fix";
        }
        return "feat";
    }
    "chore"
}

fn infer_scope(files: &[PathBuf]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    // Find common directory
    let first = &files[0];
    let components: Vec<_> = first.components().collect();

    // Get the file name separately (we'll skip it when looking for directories)
    let filename = first.file_name().and_then(|n| n.to_str());

    if components.len() > 1 {
        // Use the first meaningful directory component (excluding the filename)
        for component in &components {
            let name = component.as_os_str().to_string_lossy();
            // Skip src, lib, hidden dirs, and the filename itself
            if name != "src" && name != "lib" && !name.starts_with('.') && filename != Some(&*name)
            {
                // Limit scope length
                if name.len() <= 15 {
                    return Some(name.to_string());
                }
            }
        }
    }

    // Use file stem if only one file
    if files.len() == 1 {
        if let Some(stem) = first.file_stem().and_then(|s| s.to_str()) {
            if stem.len() <= 15 {
                return Some(stem.to_string());
            }
        }
    }

    None
}

fn generate_subject(analysis: &ChangeAnalysis, commit_type: &str) -> String {
    let action = match (
        analysis.files_added,
        analysis.files_modified,
        analysis.files_deleted,
    ) {
        (n, 0, 0) if n > 0 => "add",
        (0, n, 0) if n > 0 => "update",
        (0, 0, n) if n > 0 => "remove",
        (a, m, 0) if a > 0 && m > 0 => "add and update",
        (0, m, d) if m > 0 && d > 0 => "update and remove",
        _ => "update",
    };

    let target = if !analysis.key_files.is_empty() {
        if analysis.key_files.len() == 1 {
            analysis.key_files[0].clone()
        } else {
            format!(
                "{} and {} more",
                analysis.key_files[0],
                analysis.key_files.len() - 1
            )
        }
    } else {
        match commit_type {
            "test" => "tests".to_string(),
            "docs" => "documentation".to_string(),
            "chore" => "configuration".to_string(),
            _ => "files".to_string(),
        }
    };

    format!("{} {}", action, target)
}

fn generate_body(analysis: &ChangeAnalysis) -> String {
    // Only generate body for larger changes
    let total_changes = analysis.lines_added + analysis.lines_deleted;
    if total_changes < 20 {
        return String::new();
    }

    let mut parts = Vec::new();

    if analysis.files_added > 0 {
        parts.push(format!(
            "{} file{} added",
            analysis.files_added,
            if analysis.files_added == 1 { "" } else { "s" }
        ));
    }
    if analysis.files_modified > 0 {
        parts.push(format!(
            "{} file{} modified",
            analysis.files_modified,
            if analysis.files_modified == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if analysis.files_deleted > 0 {
        parts.push(format!(
            "{} file{} deleted",
            analysis.files_deleted,
            if analysis.files_deleted == 1 { "" } else { "s" }
        ));
    }

    if parts.is_empty() {
        return String::new();
    }

    format!(
        "Changes: {} (+{} -{} lines)",
        parts.join(", "),
        analysis.lines_added,
        analysis.lines_deleted
    )
}

