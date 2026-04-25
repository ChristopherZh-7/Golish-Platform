use std::path::{Path, PathBuf};

/// Check if a path is within an allowed temporary directory.
pub fn is_in_temp_dir(path: &Path) -> bool {
    let tmp = std::env::temp_dir();
    if let Ok(canonical_tmp) = tmp.canonicalize() {
        if let Ok(canonical_path) = path.canonicalize() {
            return canonical_path.starts_with(&canonical_tmp);
        }
        if let Some(parent) = path.parent() {
            if let Ok(canonical_parent) = parent.canonicalize() {
                return canonical_parent.starts_with(&canonical_tmp);
            }
        }
    }
    path.starts_with("/tmp") || path.starts_with("/var/folders")
}

/// Check if a resolved path is within the workspace or an allowed temp directory.
pub fn is_within_workspace(resolved: &Path, workspace: &Path) -> bool {
    if is_in_temp_dir(resolved) {
        return true;
    }
    match (resolved.canonicalize(), workspace.canonicalize()) {
        (Ok(r), Ok(w)) => r.starts_with(w),
        _ => false,
    }
}

/// Resolve a path relative to workspace.
///
/// Absolute paths are returned as-is; relative paths are joined to `workspace`.
pub fn join_workspace(path_str: &str, workspace: &Path) -> PathBuf {
    let path = Path::new(path_str);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

/// Resolve a path relative to workspace and ensure it's within the workspace
/// or an allowed temporary directory.
///
/// For paths that don't yet exist, walks up to find an existing ancestor and
/// verifies that ancestor is within the workspace.
pub fn resolve_path_checked(path_str: &str, workspace: &Path) -> Result<PathBuf, String> {
    let resolved = join_workspace(path_str, workspace);

    if is_in_temp_dir(&resolved) {
        if resolved.exists() {
            return resolved
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e));
        }
        return Ok(resolved);
    }

    let workspace_canonical = workspace
        .canonicalize()
        .map_err(|e| format!("Cannot resolve workspace path: {}", e))?;

    let canonical = if resolved.exists() {
        resolved
            .canonicalize()
            .map_err(|e| format!("Cannot resolve path: {}", e))?
    } else {
        let mut check_path = resolved.as_path();
        let mut non_existent_parts: Vec<&std::ffi::OsStr> = Vec::new();

        while !check_path.exists() {
            if let Some(name) = check_path.file_name() {
                non_existent_parts.push(name);
            }
            match check_path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    check_path = parent;
                }
                _ => {
                    check_path = workspace;
                    break;
                }
            }
        }

        let canonical_ancestor = check_path
            .canonicalize()
            .map_err(|e| format!("Cannot resolve path: {}", e))?;

        if !canonical_ancestor.starts_with(&workspace_canonical) {
            return Err(format!(
                "Path '{}' is outside workspace (workspace: {})",
                path_str,
                workspace.display()
            ));
        }

        non_existent_parts.reverse();
        let mut result = canonical_ancestor;
        for part in non_existent_parts {
            result = result.join(part);
        }
        result
    };

    if !canonical.starts_with(&workspace_canonical) {
        return Err(format!(
            "Path '{}' is outside workspace (workspace: {})",
            path_str,
            workspace.display()
        ));
    }

    Ok(canonical)
}
