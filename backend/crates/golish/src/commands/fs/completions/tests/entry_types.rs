use super::*;

#[test]
fn directories_have_trailing_slash() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Doc", temp.path(), 100);

    let docs = response
        .completions
        .iter()
        .find(|c| c.name.starts_with("Documents"));
    assert!(docs.is_some());
    assert_eq!(docs.unwrap().name, "Documents/");
    assert_eq!(docs.unwrap().entry_type, PathEntryType::Directory);
}

#[test]
fn files_have_no_trailing_slash() {
    let temp = setup_test_dir();
    let response = compute_path_completions("file", temp.path(), 100);

    let file = response
        .completions
        .iter()
        .find(|c| c.name.starts_with("file"));
    assert!(file.is_some());
    assert_eq!(file.unwrap().name, "file.txt");
    assert_eq!(file.unwrap().entry_type, PathEntryType::File);
}
