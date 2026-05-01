use super::*;

#[test]
fn fuzzy_match_returns_score_and_indices() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Doc", temp.path(), 100);

    // Should have matches with scores > 0
    let docs = response.completions.iter().find(|c| c.name == "Documents/");
    assert!(docs.is_some());
    let docs = docs.unwrap();
    assert!(docs.score > 0);
    assert!(!docs.match_indices.is_empty());
}

#[test]
fn fuzzy_match_handles_abbreviations() {
    let temp = setup_test_dir();
    // "Dcmts" should fuzzy match "Documents"
    let response = compute_path_completions("Dcmt", temp.path(), 100);

    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"Documents/"));
}

#[test]
fn empty_prefix_returns_all_with_zero_score() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 100);

    // All completions should have score 0 and empty match_indices
    for completion in &response.completions {
        assert_eq!(completion.score, 0);
        assert!(completion.match_indices.is_empty());
    }
}

#[test]
fn higher_score_sorted_first() {
    let temp = setup_test_dir();
    let response = compute_path_completions("doc", temp.path(), 100);

    // If there are multiple matches, higher scores should come first
    if response.completions.len() >= 2 {
        for window in response.completions.windows(2) {
            // Higher score or equal should come first (with dirs before files as tiebreaker)
            assert!(
                window[0].score >= window[1].score
                    || (window[0].score == window[1].score
                        && matches!(window[0].entry_type, PathEntryType::Directory))
            );
        }
    }
}
