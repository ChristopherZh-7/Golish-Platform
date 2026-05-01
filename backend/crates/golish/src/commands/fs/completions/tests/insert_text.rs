use super::*;

#[test]
fn empty_input_inserts_name() {
    let temp = setup_test_dir();
    let response = compute_path_completions("", temp.path(), 100);

    let docs = response
        .completions
        .iter()
        .find(|c| c.name == "Documents/")
        .unwrap();
    assert_eq!(docs.insert_text, "Documents/");
}

#[test]
fn prefix_input_inserts_name() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Doc", temp.path(), 100);

    let docs = response
        .completions
        .iter()
        .find(|c| c.name == "Documents/")
        .unwrap();
    assert_eq!(docs.insert_text, "Documents/");
}

#[test]
fn path_with_slash_preserves_prefix() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Documents/", temp.path(), 100);

    let work = response
        .completions
        .iter()
        .find(|c| c.name == "work/")
        .unwrap();
    assert_eq!(work.insert_text, "Documents/work/");
}

#[test]
fn partial_path_replaces_last_component() {
    let temp = setup_test_dir();
    let response = compute_path_completions("Documents/wo", temp.path(), 100);

    let work = response
        .completions
        .iter()
        .find(|c| c.name == "work/")
        .unwrap();
    assert_eq!(work.insert_text, "Documents/work/");
}
