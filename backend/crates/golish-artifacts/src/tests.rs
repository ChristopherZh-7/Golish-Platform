use super::*;
use tempfile::TempDir;

    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // ArtifactMeta Tests
    // -------------------------------------------------------------------------

    mod artifact_meta {
        use super::*;

        #[test]
        fn creates_new_metadata() {
            let meta = ArtifactMeta::new(
                PathBuf::from("/path/to/README.md"),
                "Added authentication".to_string(),
            );

            assert_eq!(meta.target, PathBuf::from("/path/to/README.md"));
            assert_eq!(meta.reason, "Added authentication");
            assert!(meta.based_on_patches.is_empty());
        }

        #[test]
        fn creates_metadata_with_patches() {
            let meta = ArtifactMeta::with_patches(
                PathBuf::from("/path/to/README.md"),
                "Added auth".to_string(),
                vec![1, 2, 3],
            );

            assert_eq!(meta.based_on_patches, vec![1, 2, 3]);
        }

        #[test]
        fn formats_header_without_patches() {
            let meta = ArtifactMeta {
                target: PathBuf::from("/path/to/README.md"),
                created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                reason: "Added authentication feature".to_string(),
                based_on_patches: Vec::new(),
            };

            let header = meta.to_header();

            assert!(header.starts_with("<!--"));
            assert!(header.ends_with("-->"));
            assert!(header.contains("Target: /path/to/README.md"));
            assert!(header.contains("Created: 2025-12-10 14:30"));
            assert!(header.contains("Reason: Added authentication feature"));
            assert!(!header.contains("Based on patches"));
        }

        #[test]
        fn formats_header_with_patches() {
            let meta = ArtifactMeta {
                target: PathBuf::from("/path/to/README.md"),
                created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                reason: "Added authentication".to_string(),
                based_on_patches: vec![1, 2],
            };

            let header = meta.to_header();

            assert!(header.contains("Based on patches: 0001, 0002"));
        }

        #[test]
        fn parses_header_without_patches() {
            let header = r#"<!--
Target: /path/to/README.md
Created: 2025-12-10 14:30
Reason: Added authentication feature
-->"#;

            let meta = ArtifactMeta::from_header(header).unwrap();

            assert_eq!(meta.target, PathBuf::from("/path/to/README.md"));
            assert_eq!(meta.reason, "Added authentication feature");
            assert!(meta.based_on_patches.is_empty());
        }

        #[test]
        fn parses_header_with_patches() {
            let header = r#"<!--
Target: /path/to/CLAUDE.md
Created: 2025-12-10 15:00
Reason: Updated conventions
Based on patches: 0001, 0002, 0003
-->"#;

            let meta = ArtifactMeta::from_header(header).unwrap();

            assert_eq!(meta.target, PathBuf::from("/path/to/CLAUDE.md"));
            assert_eq!(meta.based_on_patches, vec![1, 2, 3]);
        }

        #[test]
        fn roundtrip_header() {
            let original = ArtifactMeta {
                target: PathBuf::from("/home/user/project/README.md"),
                created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                reason: "Added new feature".to_string(),
                based_on_patches: vec![1, 5, 10],
            };

            let header = original.to_header();
            let parsed = ArtifactMeta::from_header(&header).unwrap();

            assert_eq!(original.target, parsed.target);
            assert_eq!(original.reason, parsed.reason);
            assert_eq!(original.based_on_patches, parsed.based_on_patches);
            // Note: created_at might differ slightly due to formatting precision
        }

        #[test]
        fn returns_error_for_missing_delimiters() {
            let result = ArtifactMeta::from_header("No delimiters here");
            assert!(result.is_err());
        }

        #[test]
        fn returns_error_for_missing_target() {
            let header = r#"<!--
Created: 2025-12-10 14:30
Reason: Some reason
-->"#;

            let result = ArtifactMeta::from_header(header);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Target"));
        }

        #[test]
        fn returns_error_for_missing_created() {
            let header = r#"<!--
Target: /path/to/file.md
Reason: Some reason
-->"#;

            let result = ArtifactMeta::from_header(header);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Created"));
        }

        #[test]
        fn returns_error_for_missing_reason() {
            let header = r#"<!--
Target: /path/to/file.md
Created: 2025-12-10 14:30
-->"#;

            let result = ArtifactMeta::from_header(header);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Reason"));
        }
    }

    // -------------------------------------------------------------------------
    // ArtifactFile Tests
    // -------------------------------------------------------------------------

    mod artifact_file {
        use super::*;

        #[test]
        fn creates_artifact_file() {
            let meta = ArtifactMeta::new(
                PathBuf::from("/path/to/README.md"),
                "Added feature".to_string(),
            );

            let artifact = ArtifactFile::new(
                "README.md".to_string(),
                meta,
                "# Project\n\nDescription here.".to_string(),
            );

            assert_eq!(artifact.filename, "README.md");
            assert!(artifact.content.contains("# Project"));
        }

        #[test]
        fn formats_full_file_content() {
            let meta = ArtifactMeta {
                target: PathBuf::from("/path/to/README.md"),
                created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                reason: "Initial creation".to_string(),
                based_on_patches: Vec::new(),
            };

            let artifact = ArtifactFile::new(
                "README.md".to_string(),
                meta,
                "# My Project\n\nWelcome!".to_string(),
            );

            let content = artifact.to_file_content();

            assert!(content.starts_with("<!--"));
            assert!(content.contains("Target: /path/to/README.md"));
            assert!(content.contains("# My Project"));
            assert!(content.contains("Welcome!"));
        }

        #[test]
        fn parses_file_content() {
            let content = r#"<!--
Target: /path/to/CLAUDE.md
Created: 2025-12-10 14:30
Reason: Updated conventions
-->

# CLAUDE.md

Instructions for the AI assistant.

## Commands
- `cargo test` - Run tests"#;

            let artifact = ArtifactFile::from_file_content("CLAUDE.md", content).unwrap();

            assert_eq!(artifact.filename, "CLAUDE.md");
            assert_eq!(artifact.meta.target, PathBuf::from("/path/to/CLAUDE.md"));
            assert!(artifact.content.starts_with("# CLAUDE.md"));
            assert!(artifact.content.contains("## Commands"));
        }

        #[test]
        fn roundtrip_file_content() {
            let meta = ArtifactMeta {
                target: PathBuf::from("/project/README.md"),
                created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                reason: "Test roundtrip".to_string(),
                based_on_patches: vec![1, 2],
            };

            let original = ArtifactFile::new(
                "README.md".to_string(),
                meta,
                "# Title\n\nContent here.".to_string(),
            );

            let file_content = original.to_file_content();
            let parsed = ArtifactFile::from_file_content("README.md", &file_content).unwrap();

            assert_eq!(original.filename, parsed.filename);
            assert_eq!(original.meta.target, parsed.meta.target);
            assert_eq!(original.meta.reason, parsed.meta.reason);
            assert_eq!(original.content, parsed.content);
        }

        #[test]
        fn returns_error_for_missing_header() {
            let content = "# Just content, no header";
            let result = ArtifactFile::from_file_content("file.md", content);
            assert!(result.is_err());
        }
    }

    // -------------------------------------------------------------------------
    // ArtifactManager Tests
    // -------------------------------------------------------------------------

    mod artifact_manager {
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
    }

    // -------------------------------------------------------------------------
    // Rule-Based Generation Tests
    // -------------------------------------------------------------------------

    mod generation {
        use super::*;

        #[test]
        fn generates_readme_with_changes() {
            let current = "# Project\n\nA cool project.";
            let context = "Goal: Add authentication";
            let patches = vec!["feat(auth): add login".to_string()];

            let result = generate_readme_update(current, context, &patches);

            assert!(result.contains("# Project"));
            assert!(result.contains("## Recent Changes"));
            assert!(result.contains("feat(auth): add login"));
        }

        #[test]
        fn generates_readme_without_changes() {
            let current = "# Project\n\nA cool project.";
            let context = "Goal: Review code";
            let patches: Vec<String> = vec![];

            let result = generate_readme_update(current, context, &patches);

            assert_eq!(result, current);
        }

        #[test]
        fn generates_claude_md_with_changes() {
            let current = "# CLAUDE.md\n\nInstructions.";
            let context = "Session context here";
            let patches = vec!["Added new convention".to_string()];

            let result = generate_claude_md_update(current, context, &patches);

            assert!(result.contains("# CLAUDE.md"));
            assert!(result.contains("## Session Notes"));
        }
    }

    // -------------------------------------------------------------------------
    // Helper Function Tests
    // -------------------------------------------------------------------------

    mod helpers {
        use super::*;

        #[test]
        fn generates_simple_diff() {
            let old = "line1\nline2\nline3";
            let new = "line1\nmodified\nline3";

            let diff = generate_simple_diff(old, new);

            assert!(diff.contains("--- current"));
            assert!(diff.contains("+++ proposed"));
            assert!(diff.contains(" line1"));
            assert!(diff.contains("-line2"));
            assert!(diff.contains("+modified"));
        }

        #[test]
        fn generates_diff_for_added_lines() {
            let old = "line1";
            let new = "line1\nline2\nline3";

            let diff = generate_simple_diff(old, new);

            assert!(diff.contains("+line2"));
            assert!(diff.contains("+line3"));
        }

        #[test]
        fn generates_diff_for_removed_lines() {
            let old = "line1\nline2\nline3";
            let new = "line1";

            let diff = generate_simple_diff(old, new);

            assert!(diff.contains("-line2"));
            assert!(diff.contains("-line3"));
        }
    }

    // -------------------------------------------------------------------------
    // Artifact Synthesis Backend Tests
    // -------------------------------------------------------------------------

    mod synthesis_backend {
        use super::*;

        #[test]
        fn backend_from_str_template() {
            let backend: ArtifactSynthesisBackend = "template".parse().unwrap();
            assert_eq!(backend, ArtifactSynthesisBackend::Template);
        }

        #[test]
        fn backend_from_str_vertex() {
            let backend: ArtifactSynthesisBackend = "vertex_anthropic".parse().unwrap();
            assert_eq!(backend, ArtifactSynthesisBackend::VertexAnthropic);

            // Short form
            let backend: ArtifactSynthesisBackend = "vertex".parse().unwrap();
            assert_eq!(backend, ArtifactSynthesisBackend::VertexAnthropic);
        }

        #[test]
        fn backend_from_str_openai() {
            let backend: ArtifactSynthesisBackend = "openai".parse().unwrap();
            assert_eq!(backend, ArtifactSynthesisBackend::OpenAi);
        }

        #[test]
        fn backend_from_str_grok() {
            let backend: ArtifactSynthesisBackend = "grok".parse().unwrap();
            assert_eq!(backend, ArtifactSynthesisBackend::Grok);
        }

        #[test]
        fn backend_from_str_invalid() {
            let result: Result<ArtifactSynthesisBackend, _> = "invalid".parse();
            assert!(result.is_err());
        }

        #[test]
        fn backend_display() {
            assert_eq!(ArtifactSynthesisBackend::Template.to_string(), "template");
            assert_eq!(
                ArtifactSynthesisBackend::VertexAnthropic.to_string(),
                "vertex_anthropic"
            );
            assert_eq!(ArtifactSynthesisBackend::OpenAi.to_string(), "openai");
            assert_eq!(ArtifactSynthesisBackend::Grok.to_string(), "grok");
        }

        #[test]
        fn config_default_is_template() {
            let config = ArtifactSynthesisConfig::default();
            assert_eq!(config.backend, ArtifactSynthesisBackend::Template);
            assert!(!config.uses_llm());
        }

        #[test]
        fn config_uses_llm_when_not_template() {
            let mut config = ArtifactSynthesisConfig {
                backend: ArtifactSynthesisBackend::OpenAi,
                ..Default::default()
            };
            assert!(config.uses_llm());

            config.backend = ArtifactSynthesisBackend::VertexAnthropic;
            assert!(config.uses_llm());

            config.backend = ArtifactSynthesisBackend::Grok;
            assert!(config.uses_llm());
        }
    }

    // -------------------------------------------------------------------------
    // Artifact Synthesis Input Tests
    // -------------------------------------------------------------------------

    mod synthesis_input {
        use super::*;

        #[test]
        fn creates_synthesis_input() {
            let input = ArtifactSynthesisInput::new(
                "# README".to_string(),
                vec!["feat: add feature".to_string()],
                "Goal: Add new feature".to_string(),
            );

            assert_eq!(input.existing_content, "# README");
            assert_eq!(input.patches_summary.len(), 1);
            assert_eq!(input.session_context, "Goal: Add new feature");
        }

        #[test]
        fn builds_readme_prompt_with_patches() {
            let input = ArtifactSynthesisInput::new(
                "# My Project".to_string(),
                vec!["feat: add login".to_string(), "fix: fix bug".to_string()],
                "Session context".to_string(),
            );

            let prompt = input.build_readme_prompt();

            assert!(prompt.contains("# My Project"));
            assert!(prompt.contains("1. feat: add login"));
            assert!(prompt.contains("2. fix: fix bug"));
            assert!(prompt.contains("Session context"));
        }

        #[test]
        fn builds_readme_prompt_without_patches() {
            let input = ArtifactSynthesisInput::new(
                "# My Project".to_string(),
                vec![],
                "Session context".to_string(),
            );

            let prompt = input.build_readme_prompt();

            assert!(prompt.contains("# My Project"));
            assert!(prompt.contains("No patches available"));
        }

        #[test]
        fn builds_claude_md_prompt_with_patches() {
            let input = ArtifactSynthesisInput::new(
                "# CLAUDE.md\n\nInstructions".to_string(),
                vec!["refactor: update structure".to_string()],
                "Goal: Refactor".to_string(),
            );

            let prompt = input.build_claude_md_prompt();

            assert!(prompt.contains("# CLAUDE.md"));
            assert!(prompt.contains("1. refactor: update structure"));
            assert!(prompt.contains("Goal: Refactor"));
        }
    }

    // -------------------------------------------------------------------------
    // Artifact Synthesis Result Tests
    // -------------------------------------------------------------------------

    mod synthesis_result {
        use super::*;

        #[test]
        fn creates_synthesis_result() {
            let result = ArtifactSynthesisResult {
                content: "# Updated README".to_string(),
                backend: "template".to_string(),
            };

            assert_eq!(result.content, "# Updated README");
            assert_eq!(result.backend, "template");
        }

        #[test]
        fn synthesis_result_serializes() {
            let result = ArtifactSynthesisResult {
                content: "# Content".to_string(),
                backend: "openai".to_string(),
            };

            let json = serde_json::to_string(&result).unwrap();
            assert!(json.contains("\"content\":\"# Content\""));
            assert!(json.contains("\"backend\":\"openai\""));
        }
    }

    // -------------------------------------------------------------------------
    // Template Synthesis Tests (synchronous, no API calls)
    // -------------------------------------------------------------------------

    mod template_synthesis {
        use super::*;

        #[tokio::test]
        async fn synthesize_readme_with_template_backend() {
            let config = ArtifactSynthesisConfig::default();
            let input = ArtifactSynthesisInput::new(
                "# Project".to_string(),
                vec!["feat: new feature".to_string()],
                "Goal: Add feature".to_string(),
            );

            let result = synthesize_readme(&config, &input).await.unwrap();

            assert_eq!(result.backend, "template");
            assert!(result.content.contains("# Project"));
            assert!(result.content.contains("## Recent Changes"));
        }

        #[tokio::test]
        async fn synthesize_claude_md_with_template_backend() {
            let config = ArtifactSynthesisConfig::default();
            let input = ArtifactSynthesisInput::new(
                "# CLAUDE.md\n\nInstructions here.".to_string(),
                vec!["refactor: update structure".to_string()],
                "Session: Refactor codebase".to_string(),
            );

            let result = synthesize_claude_md(&config, &input).await.unwrap();

            assert_eq!(result.backend, "template");
            assert!(result.content.contains("# CLAUDE.md"));
            assert!(result.content.contains("## Session Notes"));
        }

        #[tokio::test]
        async fn synthesize_readme_no_changes_when_no_patches() {
            let config = ArtifactSynthesisConfig::default();
            let input = ArtifactSynthesisInput::new(
                "# Project\n\nExisting content.".to_string(),
                vec![],
                "No-op session".to_string(),
            );

            let result = synthesize_readme(&config, &input).await.unwrap();

            // Template returns content unchanged when no patches
            assert_eq!(result.content, "# Project\n\nExisting content.");
        }
    }
}
