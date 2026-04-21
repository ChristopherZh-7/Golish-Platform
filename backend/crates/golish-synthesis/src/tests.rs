use super::*;

    use super::*;

    // -------------------------------------------------------------------------
    // SynthesisBackend tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_synthesis_backend_from_str() {
        assert_eq!(
            "template".parse::<SynthesisBackend>().unwrap(),
            SynthesisBackend::Template
        );
        assert_eq!(
            "vertex_anthropic".parse::<SynthesisBackend>().unwrap(),
            SynthesisBackend::VertexAnthropic
        );
        assert_eq!(
            "vertex".parse::<SynthesisBackend>().unwrap(),
            SynthesisBackend::VertexAnthropic
        );
        assert_eq!(
            "openai".parse::<SynthesisBackend>().unwrap(),
            SynthesisBackend::OpenAi
        );
        assert_eq!(
            "grok".parse::<SynthesisBackend>().unwrap(),
            SynthesisBackend::Grok
        );
        assert!("invalid".parse::<SynthesisBackend>().is_err());
    }

    #[test]
    fn test_synthesis_backend_display() {
        assert_eq!(SynthesisBackend::Template.to_string(), "template");
        assert_eq!(
            SynthesisBackend::VertexAnthropic.to_string(),
            "vertex_anthropic"
        );
        assert_eq!(SynthesisBackend::OpenAi.to_string(), "openai");
        assert_eq!(SynthesisBackend::Grok.to_string(), "grok");
    }

    // -------------------------------------------------------------------------
    // SynthesisInput tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_synthesis_input_creation() {
        let input = SynthesisInput::new(
            "diff content".to_string(),
            vec![PathBuf::from("src/main.rs")],
        );
        assert_eq!(input.diff, "diff content");
        assert_eq!(input.files.len(), 1);
        assert!(input.session_context.is_none());
    }

    #[test]
    fn test_synthesis_input_with_context() {
        let input = SynthesisInput::new("diff".to_string(), vec![])
            .with_context("session context".to_string());
        assert_eq!(input.session_context, Some("session context".to_string()));
    }

    #[test]
    fn test_synthesis_input_format_files() {
        let input = SynthesisInput::new(
            "".to_string(),
            vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")],
        );
        let formatted = input.format_files();
        assert!(formatted.contains("- src/main.rs"));
        assert!(formatted.contains("- src/lib.rs"));
    }

    #[test]
    fn test_synthesis_input_build_prompt() {
        let input = SynthesisInput::new(
            "+fn new() {}".to_string(),
            vec![PathBuf::from("src/lib.rs")],
        )
        .with_context("Adding a new function".to_string());

        let prompt = input.build_prompt();
        assert!(prompt.contains("Adding a new function"));
        assert!(prompt.contains("+fn new() {}"));
        assert!(prompt.contains("- src/lib.rs"));
    }

    // -------------------------------------------------------------------------
    // Template synthesizer tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_template_synthesizer_basic() {
        let synthesizer = TemplateSynthesizer::new();
        let input = SynthesisInput::new(
            "+pub fn hello() {}\n".to_string(),
            vec![PathBuf::from("src/lib.rs")],
        );

        let result = synthesizer.synthesize(&input).await.unwrap();
        assert_eq!(result.backend, "template");
        assert!(!result.regenerated);
        assert!(!result.message.is_empty());
    }

    #[test]
    fn test_template_synthesizer_backend_name() {
        let synthesizer = TemplateSynthesizer::new();
        assert_eq!(synthesizer.backend_name(), "template");
    }

    // -------------------------------------------------------------------------
    // Template-based message generation tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_generate_template_message_new_file() {
        let files = vec![PathBuf::from("src/auth.rs")];
        let diff = "new file mode 100644\n+pub fn authenticate() {}";

        let message = generate_template_message(&files, diff);
        assert!(message.contains("feat") || message.contains("add"));
        assert!(message.contains("auth"));
    }

    #[test]
    fn test_generate_template_message_modified_file() {
        let files = vec![PathBuf::from("src/lib.rs")];
        let diff = "-old line\n+new line";

        let message = generate_template_message(&files, diff);
        assert!(message.contains("update") || message.contains("fix"));
    }

    #[test]
    fn test_generate_template_message_test_file() {
        let files = vec![PathBuf::from("tests/auth_test.rs")];
        let diff = "+#[test]\n+fn test_auth() {}";

        let message = generate_template_message(&files, diff);
        assert!(message.starts_with("test"));
    }

    #[test]
    fn test_generate_template_message_docs() {
        let files = vec![PathBuf::from("README.md")];
        let diff = "+## New Section";

        let message = generate_template_message(&files, diff);
        assert!(message.starts_with("docs"));
    }

    #[test]
    fn test_generate_template_message_config() {
        let files = vec![PathBuf::from("Cargo.toml")];
        let diff = "+[dependencies]\n+tokio = \"1.0\"";

        let message = generate_template_message(&files, diff);
        assert!(message.starts_with("chore"));
    }

    #[test]
    fn test_generate_template_message_multiple_files() {
        let files = vec![
            PathBuf::from("src/lib.rs"),
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/utils.rs"),
        ];
        let diff = "+new code\n-old code";

        let message = generate_template_message(&files, diff);
        assert!(message.contains("and") || message.contains("files"));
    }

    // -------------------------------------------------------------------------
    // Change analysis tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_analyze_changes_counts_additions() {
        let files = vec![PathBuf::from("src/new.rs")];
        let diff = "new file mode 100644\n+line1\n+line2\n+line3";

        let analysis = analyze_changes(&files, diff);
        assert_eq!(analysis.files_added, 1);
        assert_eq!(analysis.lines_added, 3);
    }

    #[test]
    fn test_analyze_changes_counts_deletions() {
        let files = vec![PathBuf::from("src/old.rs")];
        let diff = "deleted file mode 100644\n-line1\n-line2";

        let analysis = analyze_changes(&files, diff);
        assert_eq!(analysis.files_deleted, 1);
        assert_eq!(analysis.lines_deleted, 2);
    }

    #[test]
    fn test_analyze_changes_detects_test_files() {
        let files = vec![PathBuf::from("tests/my_test.rs")];
        let diff = "";

        let analysis = analyze_changes(&files, diff);
        assert!(analysis.is_test);
    }

    #[test]
    fn test_analyze_changes_detects_docs() {
        let files = vec![PathBuf::from("docs/guide.md")];
        let diff = "";

        let analysis = analyze_changes(&files, diff);
        assert!(analysis.is_docs);
    }

    #[test]
    fn test_analyze_changes_detects_config() {
        let files = vec![PathBuf::from("config.yaml")];
        let diff = "";

        let analysis = analyze_changes(&files, diff);
        assert!(analysis.is_config);
    }

    // -------------------------------------------------------------------------
    // Commit type inference tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_infer_commit_type_test() {
        let analysis = ChangeAnalysis {
            is_test: true,
            ..Default::default()
        };
        assert_eq!(infer_commit_type(&analysis), "test");
    }

    #[test]
    fn test_infer_commit_type_docs() {
        let analysis = ChangeAnalysis {
            is_docs: true,
            ..Default::default()
        };
        assert_eq!(infer_commit_type(&analysis), "docs");
    }

    #[test]
    fn test_infer_commit_type_feat_new_files() {
        let analysis = ChangeAnalysis {
            files_added: 2,
            ..Default::default()
        };
        assert_eq!(infer_commit_type(&analysis), "feat");
    }

    #[test]
    fn test_infer_commit_type_refactor_large_deletion() {
        let analysis = ChangeAnalysis {
            lines_deleted: 100,
            lines_added: 10,
            ..Default::default()
        };
        assert_eq!(infer_commit_type(&analysis), "refactor");
    }

    // -------------------------------------------------------------------------
    // Scope inference tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_infer_scope_single_file() {
        let files = vec![PathBuf::from("src/auth.rs")];
        let scope = infer_scope(&files);
        assert_eq!(scope, Some("auth".to_string()));
    }

    #[test]
    fn test_infer_scope_empty_files() {
        let files: Vec<PathBuf> = vec![];
        let scope = infer_scope(&files);
        assert!(scope.is_none());
    }

    #[test]
    fn test_infer_scope_skips_src() {
        let files = vec![PathBuf::from("src/auth/login.rs")];
        let scope = infer_scope(&files);
        // Should pick "auth" not "src"
        assert_eq!(scope, Some("auth".to_string()));
    }

    // -------------------------------------------------------------------------
    // SynthesisConfig tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_synthesis_config_default() {
        let config = SynthesisConfig::default();
        assert!(config.enabled);
        assert_eq!(config.backend, SynthesisBackend::Template);
    }

    #[test]
    fn test_synthesis_config_from_sidecar_settings() {
        let settings = SidecarSettings {
            enabled: true,
            synthesis_enabled: true,
            synthesis_backend: "openai".to_string(),
            ..Default::default()
        };

        let config = SynthesisConfig::from_sidecar_settings(&settings);
        assert!(config.enabled);
        assert_eq!(config.backend, SynthesisBackend::OpenAi);
    }

    // -------------------------------------------------------------------------
    // Factory tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_create_synthesizer_template() {
        let config = SynthesisConfig::default();
        let synthesizer = create_synthesizer(&config).unwrap();
        assert_eq!(synthesizer.backend_name(), "template");
    }

    #[test]
    fn test_create_synthesizer_openai_no_key() {
        let config = SynthesisConfig {
            backend: SynthesisBackend::OpenAi,
            openai: SynthesisOpenAiSettings {
                api_key: None,
                ..Default::default()
            },
            ..Default::default()
        };
        // Should fail if no API key is set
        // This test only passes if OPENAI_API_KEY env var is not set
        if std::env::var("OPENAI_API_KEY").is_err() {
            assert!(create_synthesizer(&config).is_err());
        }
    }
}
