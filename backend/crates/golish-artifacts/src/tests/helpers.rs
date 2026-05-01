use super::*;

#[test]
fn generates_simple_diff() {
    let old = "line1\nline2\nline3";
    let new = "line1\nmodified\nline3";

    let diff = generate_simple_diff(old, new);

    assert!(diff.contains("--- current"));
    assert!(diff.contains("+++ proposed"));
    assert!(diff.contains(" line1"));
    assert!(diff.contains("-line2"));
    assert!(diff.contains("+modified"));
}

#[test]
fn generates_diff_for_added_lines() {
    let old = "line1";
    let new = "line1\nline2\nline3";

    let diff = generate_simple_diff(old, new);

    assert!(diff.contains("+line2"));
    assert!(diff.contains("+line3"));
}

#[test]
fn generates_diff_for_removed_lines() {
    let old = "line1\nline2\nline3";
    let new = "line1";

    let diff = generate_simple_diff(old, new);

    assert!(diff.contains("-line2"));
    assert!(diff.contains("-line3"));
}
