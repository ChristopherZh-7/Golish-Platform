use super::*;

#[test]
fn hidden_files_excluded_by_default() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 100);

    // Should not contain hidden files/dirs
    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(!names.contains(&".hidden_dir/"));
    assert!(!names.contains(&".hidden_file"));
}

#[test]
fn hidden_files_included_when_prefix_starts_with_dot() {
    let temp = setup_test_dir();
    let response = compute_path_completions(".", temp.path(), 100);

    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&".hidden_dir/"));
    assert!(names.contains(&".hidden_file"));
}

#[test]
fn fuzzy_matching_filters_results() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Do", temp.path(), 100);

    // Should match Documents, Downloads (both start with "Do")
    // Note: With fuzzy matching, Desktop could also match (D...o) but with lower score
    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"Documents/"));
    assert!(names.contains(&"Downloads/"));
    // Documents and Downloads should rank higher than Desktop due to closer character positions
    if names.contains(&"Desktop/") {
        let docs_idx = names.iter().position(|n| *n == "Documents/").unwrap();
        let desk_idx = names.iter().position(|n| *n == "Desktop/").unwrap();
        assert!(
            docs_idx < desk_idx,
            "Documents should rank higher than Desktop for 'Do' query"
        );
    }
}

#[test]
#[cfg(target_os = "macos")]
fn case_insensitive_matching_on_macos() {
    let temp = setup_test_dir();
    let response = compute_path_completions("do", temp.path(), 100);

    // On macOS, should match Documents/Downloads even with lowercase prefix
    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"Documents/"));
    assert!(names.contains(&"Downloads/"));
}
