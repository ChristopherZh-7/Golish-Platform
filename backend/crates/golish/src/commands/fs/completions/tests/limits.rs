use super::*;

#[test]
fn respects_limit_parameter() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 2);

    assert!(response.completions.len() <= 2);
    // total_count should reflect the actual number of matches
    assert!(response.total_count >= response.completions.len());
}

#[test]
fn returns_all_if_under_limit() {
    let temp = setup_test_dir();
    // We have: Documents/, Downloads/, Desktop/, file.txt, data.json = 5 visible items
    let response = compute_path_completions("", temp.path(), 100);

    // Should have all non-hidden items
    assert!(response.completions.len() >= 5);
    assert_eq!(response.total_count, response.completions.len());
}

#[test]
fn total_count_reflects_all_matches() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 2);

    // total_count should be the actual number of matches, not limited
    assert!(response.total_count >= 5); // We have at least 5 visible items
    assert_eq!(response.completions.len(), 2); // But only 2 returned due to limit
}
