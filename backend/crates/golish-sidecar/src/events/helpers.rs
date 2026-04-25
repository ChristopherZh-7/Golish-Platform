/// Parse context XML from user prompts and extract cwd.
///
/// Returns (cwd, clean_message) where clean_message has the context block removed.
pub(super) fn parse_context_xml(input: &str) -> (Option<String>, String) {
    // Look for <context>...</context> block
    let context_start = input.find("<context>");
    let context_end = input.find("</context>");

    match (context_start, context_end) {
        (Some(start), Some(end)) if start < end => {
            let context_block = &input[start..end + "</context>".len()];

            // Extract <cwd>...</cwd>
            let cwd = extract_xml_tag(context_block, "cwd");

            // Remove the context block and trim leading whitespace
            let before = &input[..start];
            let after = &input[end + "</context>".len()..];
            let clean = format!("{}{}", before, after).trim().to_string();

            (cwd, clean)
        }
        _ => (None, input.to_string()),
    }
}

/// Extract content from a simple XML tag
pub(super) fn extract_xml_tag(input: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    let start = input.find(&open)?;
    let end = input.find(&close)?;

    if start < end {
        let content_start = start + open.len();
        Some(input[content_start..end].trim().to_string())
    } else {
        None
    }
}

/// Truncate a string to a maximum length
pub(super) fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max_len.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}
