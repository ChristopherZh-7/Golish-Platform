use super::*;

#[test]
fn directories_come_first() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 100);

    // Find first file and first directory
    let first_file_idx = response
        .completions
        .iter()
        .position(|c| matches!(c.entry_type, PathEntryType::File));
    let last_dir_idx = response
        .completions
        .iter()
        .rposition(|c| matches!(c.entry_type, PathEntryType::Directory));

    if let (Some(file_idx), Some(dir_idx)) = (first_file_idx, last_dir_idx) {
        assert!(
            dir_idx < file_idx,
            "All directories should come before files"
        );
    }
}

#[test]
fn alphabetical_within_type() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 100);

    // Get just directory names
    let dir_names: Vec<&str> = response
        .completions
        .iter()
        .filter(|c| matches!(c.entry_type, PathEntryType::Directory))
        .map(|c| c.name.as_str())
        .collect();

    // Should be alphabetically sorted (case-insensitive)
    let mut sorted = dir_names.clone();
    sorted.sort_by_key(|a| a.to_lowercase());
    assert_eq!(dir_names, sorted);
}
