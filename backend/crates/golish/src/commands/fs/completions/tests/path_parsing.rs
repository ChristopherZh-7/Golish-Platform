use super::*;

#[test]
fn empty_input_returns_working_dir() {
    let working_dir = PathBuf::from("/home/user");
    let (search_dir, prefix) = parse_path_input("", &working_dir);

    assert_eq!(search_dir, PathBuf::from("/home/user"));
    assert_eq!(prefix, "");
}

#[test]
fn tilde_expands_to_home() {
    let expanded = expand_tilde("~/Documents");
    let home = dirs::home_dir().unwrap();
    let expected = home.join("Documents").to_string_lossy().to_string();

    assert_eq!(expanded, expected);
}

#[test]
fn tilde_alone_expands_to_home() {
    let expanded = expand_tilde("~");
    let home = dirs::home_dir().unwrap();

    assert_eq!(expanded, home.to_string_lossy().to_string());
}

#[test]
fn absolute_path_is_preserved() {
    let working_dir = PathBuf::from("/home/user");
    let (search_dir, prefix) = parse_path_input("/usr/loc", &working_dir);

    assert_eq!(search_dir, PathBuf::from("/usr"));
    assert_eq!(prefix, "loc");
}

#[test]
fn relative_path_is_joined_with_working_dir() {
    let working_dir = PathBuf::from("/home/user");
    let (search_dir, prefix) = parse_path_input("Documents/wo", &working_dir);

    assert_eq!(search_dir, PathBuf::from("/home/user/Documents"));
    assert_eq!(prefix, "wo");
}

#[test]
fn path_ending_with_slash_searches_inside() {
    let working_dir = PathBuf::from("/home/user");
    let (search_dir, prefix) = parse_path_input("Documents/", &working_dir);

    assert_eq!(search_dir, PathBuf::from("/home/user/Documents"));
    assert_eq!(prefix, "");
}

#[test]
fn simple_prefix_searches_current_dir() {
    let working_dir = PathBuf::from("/home/user");
    let (search_dir, prefix) = parse_path_input("Doc", &working_dir);

    assert_eq!(search_dir, PathBuf::from("/home/user"));
    assert_eq!(prefix, "Doc");
}
