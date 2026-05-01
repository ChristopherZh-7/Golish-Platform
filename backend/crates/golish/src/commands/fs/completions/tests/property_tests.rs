use super::*;
use proptest::prelude::*;

/// Strategy for generating valid filesystem-safe names.
fn valid_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,15}".prop_map(|s| s)
}

/// Strategy for generating a list of directory/file names.
fn name_list_strategy() -> impl Strategy<Value = Vec<(String, bool)>> {
    prop::collection::vec((valid_name_strategy(), any::<bool>()), 0..10)
}

proptest! {
    /// Property: The number of completions never exceeds the limit.
    #[test]
    fn completions_respect_limit(
        limit in 1usize..50,
        names in name_list_strategy(),
    ) {
        let temp = TempDir::new().unwrap();

        // Create the directory structure
        for (name, is_dir) in &names {
            let path = temp.path().join(name);
            if *is_dir {
                let _ = fs::create_dir(&path);
            } else {
                let _ = File::create(&path);
            }
        }

        let response = compute_path_completions("", temp.path(), limit);

        prop_assert!(response.completions.len() <= limit,
            "Got {} completions but limit was {}", response.completions.len(), limit);
    }

    /// Property: Directories always have trailing slash in name.
    #[test]
    fn directories_always_have_trailing_slash(
        names in name_list_strategy(),
    ) {
        let temp = TempDir::new().unwrap();

        for (name, is_dir) in &names {
            let path = temp.path().join(name);
            if *is_dir {
                let _ = fs::create_dir(&path);
            } else {
                let _ = File::create(&path);
            }
        }

        let response = compute_path_completions("", temp.path(), 100);

        for completion in &response.completions {
            match completion.entry_type {
                PathEntryType::Directory => {
                    prop_assert!(completion.name.ends_with('/'),
                        "Directory '{}' should end with /", completion.name);
                }
                PathEntryType::File | PathEntryType::Symlink => {
                    prop_assert!(!completion.name.ends_with('/'),
                        "File/symlink '{}' should not end with /", completion.name);
                }
            }
        }
    }

    /// Property: Completions are sorted (directories first, then alphabetical when scores are equal).
    #[test]
    fn completions_are_properly_sorted(
        names in name_list_strategy(),
    ) {
        let temp = TempDir::new().unwrap();

        for (name, is_dir) in &names {
            let path = temp.path().join(name);
            if *is_dir {
                let _ = fs::create_dir(&path);
            } else {
                let _ = File::create(&path);
            }
        }

        let response = compute_path_completions("", temp.path(), 100);

        // When no prefix (empty query), all scores are 0, so directories come first
        let mut seen_file = false;
        for completion in &response.completions {
            if matches!(completion.entry_type, PathEntryType::File | PathEntryType::Symlink) {
                seen_file = true;
            } else if seen_file {
                prop_assert!(false,
                    "Directory '{}' found after file", completion.name);
            }
        }

        // Check alphabetical within each type (when scores are equal)
        let dirs: Vec<_> = response.completions.iter()
            .filter(|c| matches!(c.entry_type, PathEntryType::Directory))
            .collect();
        let files: Vec<_> = response.completions.iter()
            .filter(|c| !matches!(c.entry_type, PathEntryType::Directory))
            .collect();

        for window in dirs.windows(2) {
            prop_assert!(window[0].name.to_lowercase() <= window[1].name.to_lowercase(),
                "Directories not sorted: '{}' should come before '{}'",
                window[0].name, window[1].name);
        }

        for window in files.windows(2) {
            prop_assert!(window[0].name.to_lowercase() <= window[1].name.to_lowercase(),
                "Files not sorted: '{}' should come before '{}'",
                window[0].name, window[1].name);
        }
    }

    /// Property: Hidden files only appear when prefix starts with dot.
    #[test]
    fn hidden_files_visibility(
        prefix in prop::option::of("[.a-zA-Z][a-zA-Z0-9]*"),
    ) {
        let temp = TempDir::new().unwrap();

        // Create both hidden and visible items
        fs::create_dir(temp.path().join(".hidden")).unwrap();
        fs::create_dir(temp.path().join("visible")).unwrap();
        File::create(temp.path().join(".hidden_file")).unwrap();
        File::create(temp.path().join("visible_file")).unwrap();

        let prefix_str = prefix.unwrap_or_default();
        let response = compute_path_completions(&prefix_str, temp.path(), 100);

        let has_hidden = response.completions.iter().any(|c|
            c.name.starts_with('.') || c.name.starts_with("./"));
        let prefix_starts_with_dot = prefix_str.starts_with('.');

        if prefix_starts_with_dot {
            // Hidden files may or may not be present depending on prefix match
            // but if they match the prefix, they should be included
        } else {
            prop_assert!(!has_hidden,
                "Hidden files should not appear without dot prefix, got: {:?}",
                response.completions.iter().map(|c| &c.name).collect::<Vec<_>>());
        }
    }

    /// Property: Tilde expansion produces valid paths.
    #[test]
    fn tilde_expansion_produces_valid_path(
        // Avoid starting with / which would make it an absolute path
        suffix in "[a-zA-Z][a-zA-Z0-9_-]{0,15}",
    ) {
        let input = format!("~/{}", suffix);
        let expanded = expand_tilde(&input);

        if let Some(home) = dirs::home_dir() {
            prop_assert!(expanded.starts_with(&home.to_string_lossy().to_string()),
                "Expanded path '{}' should start with home dir", expanded);
        }
    }

    /// Property: Insert text is consistent with input patterns.
    #[test]
    fn insert_text_consistency(
        input_has_slash in any::<bool>(),
    ) {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("test_dir")).unwrap();
        File::create(temp.path().join("test_file")).unwrap();

        let input = if input_has_slash { "test_dir/" } else { "" };
        let response = compute_path_completions(input, temp.path(), 100);

        for completion in &response.completions {
            // insert_text should either:
            // 1. Be just the name (when input is empty or just a prefix)
            // 2. Preserve the path structure (when input has slashes)

            if input_has_slash && !input.is_empty() {
                // Should preserve the directory prefix
                prop_assert!(completion.insert_text.contains('/'),
                    "Insert text '{}' should contain slash when input has directory",
                    completion.insert_text);
            }

            // Insert text should contain the display name
            prop_assert!(completion.insert_text.ends_with(&completion.name) ||
                         completion.insert_text.contains(&completion.name),
                "Insert text '{}' should contain name '{}'",
                completion.insert_text, completion.name);
        }
    }

    /// Property: Total count is always >= completions length.
    #[test]
    fn total_count_gte_completions_length(
        limit in 1usize..50,
        names in name_list_strategy(),
    ) {
        let temp = TempDir::new().unwrap();

        for (name, is_dir) in &names {
            let path = temp.path().join(name);
            if *is_dir {
                let _ = fs::create_dir(&path);
            } else {
                let _ = File::create(&path);
            }
        }

        let response = compute_path_completions("", temp.path(), limit);

        prop_assert!(response.total_count >= response.completions.len(),
            "total_count {} should be >= completions.len() {}",
            response.total_count, response.completions.len());
    }
}
