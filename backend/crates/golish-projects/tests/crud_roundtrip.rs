//! Integration tests for project CRUD operations.
//!
//! These tests redirect the project registry to a tempdir via the
//! `GOLISH_PROJECTS_DIR` env var so they don't touch the user's real
//! `~/.golish/projects/` directory. They are marked `#[serial]` because
//! the env var is process-global.

use serial_test::serial;
use std::path::PathBuf;
use tempfile::TempDir;

use golish_projects::{
    delete_project, list_projects, load_project, load_workspace, save_project, save_workspace,
    ProjectConfig,
};

struct TestEnv {
    _registry: TempDir,
    project_root: TempDir,
}

impl TestEnv {
    fn new() -> Self {
        let registry = tempfile::tempdir().expect("create registry tempdir");
        let project_root = tempfile::tempdir().expect("create project root tempdir");
        std::env::set_var("GOLISH_PROJECTS_DIR", registry.path());
        Self {
            _registry: registry,
            project_root,
        }
    }

    fn make_config(&self, name: &str) -> ProjectConfig {
        ProjectConfig {
            name: name.to_string(),
            root_path: self.project_root.path().to_path_buf(),
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        std::env::remove_var("GOLISH_PROJECTS_DIR");
    }
}

#[tokio::test]
#[serial]
async fn save_then_load_returns_same_config() {
    let env = TestEnv::new();
    let cfg = env.make_config("Demo Project");

    save_project(&cfg).await.expect("save");
    let loaded = load_project("Demo Project")
        .await
        .expect("load")
        .expect("Some");

    assert_eq!(loaded.name, cfg.name);
    assert_eq!(loaded.root_path, cfg.root_path);
}

#[tokio::test]
#[serial]
async fn list_projects_returns_saved_entries_sorted() {
    let env = TestEnv::new();
    let a = env.make_config("zeta-app");
    let b = env.make_config("alpha-app");

    save_project(&a).await.unwrap();
    save_project(&b).await.unwrap();

    let projects = list_projects().await.expect("list");
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha-app", "zeta-app"]);
}

#[tokio::test]
#[serial]
async fn delete_project_removes_registry_entry() {
    let env = TestEnv::new();
    let cfg = env.make_config("Disposable");

    save_project(&cfg).await.unwrap();
    assert!(load_project("Disposable").await.unwrap().is_some());

    let removed = delete_project("Disposable").await.expect("delete");
    assert!(removed);
    assert!(load_project("Disposable").await.unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn delete_project_returns_false_when_missing() {
    let _env = TestEnv::new();
    let removed = delete_project("does-not-exist").await.expect("delete");
    assert!(!removed);
}

#[tokio::test]
#[serial]
async fn save_workspace_then_load_returns_same_json() {
    let env = TestEnv::new();
    let cfg = env.make_config("Workspace Demo");
    save_project(&cfg).await.unwrap();

    let payload = r#"{"sessions":[],"open_panels":["git"]}"#;
    save_workspace("Workspace Demo", payload).await.unwrap();

    let loaded = load_workspace("Workspace Demo")
        .await
        .expect("load")
        .expect("Some");
    assert_eq!(loaded, payload);
}

#[tokio::test]
#[serial]
async fn save_project_creates_dot_golish_directory() {
    let env = TestEnv::new();
    let cfg = env.make_config("With Subdirs");

    save_project(&cfg).await.unwrap();

    let dot = cfg.root_path.join(".golish");
    assert!(dot.exists(), ".golish/ should be created");
    assert!(
        dot.join("project.json").exists(),
        "project.json should be initialized"
    );
}

#[tokio::test]
#[serial]
async fn load_workspace_returns_none_for_missing_project() {
    let _env = TestEnv::new();
    assert!(load_workspace("ghost").await.unwrap().is_none());
}

// Regression: paths with spaces/non-alphanumerics should slugify cleanly.
#[tokio::test]
#[serial]
async fn slugified_directory_is_used_on_disk() {
    let env = TestEnv::new();
    let cfg = env.make_config("My Cool Project!");
    save_project(&cfg).await.unwrap();

    // Look for the slug "my-cool-project" inside the registry tempdir.
    let registry: PathBuf = std::env::var("GOLISH_PROJECTS_DIR").unwrap().into();
    assert!(registry.join("my-cool-project").join("config.toml").exists());
}
