//! Project configuration schema.
//!
//! Schema E (2026-05-17): the `mode` field and `ProjectMode` enum were retired.
//! Every project now relies on the implicit-organization model — pentest
//! projects auto-create a single root organization (named after the project)
//! and red-team projects keep growing the organization tree on top of it. The
//! "is this a pentest or a red-team project?" question is now answered by
//! inspecting the organization tree shape at the UI layer (`orgs.length === 1
//! && root has no children` ⇒ pentest-like UI), not by a persisted enum.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a single project/codebase.
///
/// `#[serde(default)]` on extra fields keeps **legacy `config.toml`** files
/// (written before / after the `mode` field existed) loading without errors.
/// Unknown fields like a leftover `mode = "redteam"` are silently dropped by
/// `toml::from_str`, so no migration step is needed for on-disk configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Display name for the project.
    pub name: String,

    /// Root path to the main project directory.
    pub root_path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_loads_without_mode_field() {
        let legacy = r#"name = "Old Project"
root_path = "/tmp/old"
"#;
        let cfg: ProjectConfig = toml::from_str(legacy).expect("parse legacy");
        assert_eq!(cfg.name, "Old Project");
        assert_eq!(cfg.root_path, PathBuf::from("/tmp/old"));
    }

    #[test]
    fn config_with_extra_mode_field_still_parses() {
        // Old configs may still carry `mode = "redteam"` from Schema D.
        // Schema E silently drops unknown fields rather than rejecting them,
        // so existing projects keep loading after upgrade.
        let pre_e = r#"name = "HVV"
root_path = "/tmp/hvv"
mode = "redteam"
"#;
        let cfg: ProjectConfig = toml::from_str(pre_e).expect("parse pre-E config");
        assert_eq!(cfg.name, "HVV");
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = ProjectConfig {
            name: "SRC".into(),
            root_path: PathBuf::from("/tmp/src"),
        };
        let s = toml::to_string(&cfg).expect("serialize");
        assert!(!s.contains("mode"), "Schema E must not emit a mode field: {s}");
        let back: ProjectConfig = toml::from_str(&s).expect("parse");
        assert_eq!(back.name, "SRC");
    }
}
