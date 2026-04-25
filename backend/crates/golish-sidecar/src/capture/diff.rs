//! Unified-diff generation for edit/write tool captures.


/// Generate unified diff between two strings
pub(super) fn generate_unified_diff(old: &str, new: &str, filename: &str) -> String {
    use std::fmt::Write;

    let diff = similar::TextDiff::from_lines(old, new);
    let mut output = String::new();

    writeln!(output, "--- a/{}", filename).unwrap();
    writeln!(output, "+++ b/{}", filename).unwrap();

    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        writeln!(output, "{}", hunk.header()).unwrap();
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => "-",
                similar::ChangeTag::Insert => "+",
                similar::ChangeTag::Equal => " ",
            };
            write!(output, "{}{}", sign, change.value()).unwrap();
            if !change.value().ends_with('\n') {
                writeln!(output).unwrap();
            }
        }
    }

    output
}
