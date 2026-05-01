use super::*;

async fn setup_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[tokio::test]
async fn creates_directories() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    manager.ensure_dirs().await.unwrap();

    assert!(temp.path().join("artifacts/pending").exists());
    assert!(temp.path().join("artifacts/applied").exists());
}

#[tokio::test]
async fn creates_pending_artifact() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    let meta = ArtifactMeta::new(
        PathBuf::from("/project/README.md"),
        "Test artifact".to_string(),
    );
    let artifact =
        ArtifactFile::new("README.md".to_string(), meta, "# Content".to_string());

    let path = manager.create_artifact(&artifact).await.unwrap();

    assert!(path.exists());
    assert!(path.ends_with("pending/README.md"));
}

#[tokio::test]
async fn lists_pending_artifacts() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create two artifacts
    let meta1 = ArtifactMeta::new(
        PathBuf::from("/project/README.md"),
        "Artifact 1".to_string(),
    );
    let artifact1 = ArtifactFile::new("README.md".to_string(), meta1, "# 1".to_string());
    manager.create_artifact(&artifact1).await.unwrap();

    let meta2 = ArtifactMeta::new(
        PathBuf::from("/project/CLAUDE.md"),
        "Artifact 2".to_string(),
    );
    let artifact2 = ArtifactFile::new("CLAUDE.md".to_string(), meta2, "# 2".to_string());
    manager.create_artifact(&artifact2).await.unwrap();

    let pending = manager.list_pending().await.unwrap();

    assert_eq!(pending.len(), 2);
    // Sorted by filename
    assert_eq!(pending[0].filename, "CLAUDE.md");
    assert_eq!(pending[1].filename, "README.md");
}

#[tokio::test]
async fn gets_specific_pending_artifact() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    let meta = ArtifactMeta::new(
        PathBuf::from("/project/README.md"),
        "Test artifact".to_string(),
    );
    let artifact =
        ArtifactFile::new("README.md".to_string(), meta, "# Content".to_string());
    manager.create_artifact(&artifact).await.unwrap();

    let found = manager.get_pending("README.md").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().filename, "README.md");

    let not_found = manager.get_pending("NOTEXIST.md").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn discards_pending_artifact() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    let meta = ArtifactMeta::new(
        PathBuf::from("/project/README.md"),
        "Test artifact".to_string(),
    );
    let artifact =
        ArtifactFile::new("README.md".to_string(), meta, "# Content".to_string());
    manager.create_artifact(&artifact).await.unwrap();

    let discarded = manager.discard_artifact("README.md").await.unwrap();
    assert!(discarded);

    let pending = manager.list_pending().await.unwrap();
    assert!(pending.is_empty());

    // Discarding non-existent returns false
    let discarded_again = manager.discard_artifact("README.md").await.unwrap();
    assert!(!discarded_again);
}

#[tokio::test]
async fn returns_empty_list_when_no_artifacts() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    let pending = manager.list_pending().await.unwrap();
    assert!(pending.is_empty());

    let applied = manager.list_applied().await.unwrap();
    assert!(applied.is_empty());
}

#[tokio::test]
async fn generates_preview_diff() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create a "current" file in temp directory
    let target_path = temp.path().join("README.md");
    fs::write(&target_path, "# Old Title\n\nOld content.")
        .await
        .unwrap();

    let meta = ArtifactMeta::new(target_path.clone(), "Updated title".to_string());
    let artifact = ArtifactFile::new(
        "README.md".to_string(),
        meta,
        "# New Title\n\nNew content.".to_string(),
    );
    manager.create_artifact(&artifact).await.unwrap();

    let diff = manager.preview_artifact("README.md").await.unwrap();

    assert!(diff.contains("--- current"));
    assert!(diff.contains("+++ proposed"));
    assert!(diff.contains("-# Old Title"));
    assert!(diff.contains("+# New Title"));
}

#[tokio::test]
async fn generates_preview_for_new_file() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Target file does NOT exist
    let target_path = temp.path().join("NEW_FILE.md");

    let meta = ArtifactMeta::new(target_path.clone(), "New file".to_string());
    let artifact = ArtifactFile::new(
        "NEW_FILE.md".to_string(),
        meta,
        "# New File\n\nThis is brand new content.".to_string(),
    );
    manager.create_artifact(&artifact).await.unwrap();

    let diff = manager.preview_artifact("NEW_FILE.md").await.unwrap();

    // All lines should be additions
    assert!(diff.contains("--- current"));
    assert!(diff.contains("+++ proposed"));
    assert!(diff.contains("+# New File"));
    assert!(diff.contains("+This is brand new content."));
}

#[tokio::test]
async fn apply_artifact_copies_to_target() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create target directory (simulating git_root)
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();

    // Initialize a git repo for the git add command
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&git_root)
        .output();

    let target_path = git_root.join("README.md");
    let meta = ArtifactMeta::new(target_path.clone(), "Test artifact".to_string());
    let artifact = ArtifactFile::new(
        "README.md".to_string(),
        meta,
        "# Applied Content\n\nThis was applied.".to_string(),
    );
    manager.create_artifact(&artifact).await.unwrap();

    // Apply the artifact
    let result_path = manager
        .apply_artifact("README.md", &git_root)
        .await
        .unwrap();

    // Verify target file was created with correct content
    assert!(target_path.exists());
    let content = fs::read_to_string(&target_path).await.unwrap();
    assert_eq!(content, "# Applied Content\n\nThis was applied.");
    assert_eq!(result_path, target_path);
}

#[tokio::test]
async fn apply_artifact_moves_to_applied() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create target directory
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();

    // Initialize a git repo
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&git_root)
        .output();

    let target_path = git_root.join("README.md");
    let meta = ArtifactMeta::new(target_path.clone(), "Test artifact".to_string());
    let artifact =
        ArtifactFile::new("README.md".to_string(), meta, "# Content".to_string());
    manager.create_artifact(&artifact).await.unwrap();

    // Verify artifact is in pending
    let pending_before = manager.list_pending().await.unwrap();
    assert_eq!(pending_before.len(), 1);

    // Apply the artifact
    manager
        .apply_artifact("README.md", &git_root)
        .await
        .unwrap();

    // Verify artifact moved from pending to applied
    let pending_after = manager.list_pending().await.unwrap();
    assert!(pending_after.is_empty());

    let applied = manager.list_applied().await.unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].filename, "README.md");
}

#[tokio::test]
async fn apply_all_artifacts_applies_multiple() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create target directory
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();

    // Initialize a git repo
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&git_root)
        .output();

    // Create two artifacts
    let meta1 = ArtifactMeta::new(git_root.join("README.md"), "First".to_string());
    let artifact1 =
        ArtifactFile::new("README.md".to_string(), meta1, "# README".to_string());
    manager.create_artifact(&artifact1).await.unwrap();

    let meta2 = ArtifactMeta::new(git_root.join("CLAUDE.md"), "Second".to_string());
    let artifact2 =
        ArtifactFile::new("CLAUDE.md".to_string(), meta2, "# CLAUDE".to_string());
    manager.create_artifact(&artifact2).await.unwrap();

    // Apply all
    let results = manager.apply_all_artifacts(&git_root).await.unwrap();

    assert_eq!(results.len(), 2);

    // Verify both files exist
    assert!(git_root.join("README.md").exists());
    assert!(git_root.join("CLAUDE.md").exists());

    // Verify all moved to applied
    let pending = manager.list_pending().await.unwrap();
    assert!(pending.is_empty());

    let applied = manager.list_applied().await.unwrap();
    assert_eq!(applied.len(), 2);
}

#[tokio::test]
async fn apply_artifact_returns_error_for_nonexistent() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();

    let result = manager.apply_artifact("NONEXISTENT.md", &git_root).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn regenerate_from_patches_creates_readme_artifact() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create a git root with README
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();
    fs::write(
        git_root.join("README.md"),
        "# My Project\n\nOriginal content.",
    )
    .await
    .unwrap();

    // Regenerate artifacts from patches
    let patches = vec!["feat(auth): add login".to_string()];
    let context = "Goal: Implement authentication";

    let created = manager
        .regenerate_from_patches(&git_root, &patches, context)
        .await
        .unwrap();

    // Should create one artifact for README
    assert_eq!(created.len(), 1);

    let pending = manager.list_pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].filename, "README.md");
    assert!(pending[0].content.contains("## Recent Changes"));
    assert!(pending[0].content.contains("feat(auth): add login"));
}

#[tokio::test]
async fn regenerate_from_patches_creates_both_readme_and_claude() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create a git root with both README and CLAUDE.md
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();
    fs::write(git_root.join("README.md"), "# My Project")
        .await
        .unwrap();
    fs::write(git_root.join("CLAUDE.md"), "# CLAUDE.md\n\nInstructions.")
        .await
        .unwrap();

    // Regenerate artifacts from patches
    let patches = vec!["feat: new feature".to_string()];
    let context = "Goal: Add feature";

    let created = manager
        .regenerate_from_patches(&git_root, &patches, context)
        .await
        .unwrap();

    // Should create two artifacts
    assert_eq!(created.len(), 2);

    let pending = manager.list_pending().await.unwrap();
    assert_eq!(pending.len(), 2);
}

#[tokio::test]
async fn regenerate_from_patches_no_artifacts_when_no_patches() {
    let temp = setup_test_dir().await;
    let manager = ArtifactManager::new(temp.path().to_path_buf());

    // Create a git root with README
    let git_root = temp.path().join("repo");
    fs::create_dir_all(&git_root).await.unwrap();
    fs::write(git_root.join("README.md"), "# My Project")
        .await
        .unwrap();

    // Regenerate with no patches
    let patches: Vec<String> = vec![];
    let context = "Goal: Nothing";

    let created = manager
        .regenerate_from_patches(&git_root, &patches, context)
        .await
        .unwrap();

    // Should not create any artifacts (no changes to make)
    assert!(created.is_empty());

    let pending = manager.list_pending().await.unwrap();
    assert!(pending.is_empty());
}
