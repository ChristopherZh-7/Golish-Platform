use super::*;

#[test]
fn nonexistent_directory_returns_empty() {
    let temp = setup_test_dir();
    let response = compute_path_completions("nonexistent/", temp.path(), 100);

    assert!(response.completions.is_empty());
    assert_eq!(response.total_count, 0);
}

#[test]
fn no_matches_returns_empty() {
    let temp = setup_test_dir();
    let response = compute_path_completions("xyz", temp.path(), 100);

    assert!(response.completions.is_empty());
    assert_eq!(response.total_count, 0);
}

#[test]
fn dot_dot_navigates_up() {
    let temp = setup_test_dir();
    let nested_dir = temp.path().join("Documents");

    // From Documents/, "../Do" should list the temp root
    let response = compute_path_completions("../Do", &nested_dir, 100);

    let names: Vec<&str> = response
        .completions
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"Documents/"));
    assert!(names.contains(&"Downloads/"));
}
