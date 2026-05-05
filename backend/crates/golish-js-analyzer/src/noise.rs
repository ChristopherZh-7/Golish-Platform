//! Strip JS comments and string-literal contents so the regex extractor
//! doesn't pick up snippets that look like call sites but aren't.
//!
//! The substitution is **byte-preserving**: every removed character is
//! replaced by a space (or kept as `\n` for newlines) so that downstream
//! `match.start()` offsets and `line_of()` calculations still match the
//! original source. This is critical because [`crate::extract_from_source`]
//! reports `Endpoint::line` based on the byte offset.
//!
//! ## What we strip
//!
//! - `// line comments` to end-of-line
//! - `/* block comments */` (no nesting in JS, so a single `*/` ends them)
//! - Single-quote, double-quote, and backtick string literals — but
//!   crucially we leave the **opening** + **closing** quote in place
//!   together with the immediately-following content; only the *interior*
//!   characters that look like call-site noise are blanked.
//!
//! Strings need this nuance because patterns like `FETCH` (`fetch\s*\(\s*"..."`)
//! actually need the URL-literal substring to *survive* — we only want to
//! kill snippets like `const docs = "axios.get('/whatever')"` where the
//! outer string is plain documentation. The trick: only blank the
//! interior of strings whose CONTENT contains a recognised call-shape
//! (`fetch(`, `axios.`, `$.ajax`, `new Request`). Strings that are just
//! ordinary URLs / identifiers stay intact, so `fetch('/api', ...)` —
//! whose `'/api'` is itself a string — keeps working.

/// Replace all comments and call-site-shaped string contents with spaces
/// in `src`, returning a new String of the same byte length.
pub(crate) fn strip_noise(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    // Pre-fill output with original bytes; we'll overwrite the noisy ones.
    let mut out: Vec<u8> = bytes.to_vec();
    let mut i = 0usize;

    while i < len {
        let b = bytes[i];

        // ── line comment ────────────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            blank_range(&mut out, start, i);
            continue;
        }

        // ── block comment ───────────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            // Consume the closing `*/` if present.
            if i + 1 < len {
                i += 2;
            } else {
                i = len;
            }
            blank_range(&mut out, start, i);
            continue;
        }

        // ── string literal ──────────────────────────────────────────────
        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            let start = i; // position of the opening quote
            i += 1; // step past opening quote
            // Walk to the matching closing quote, honouring backslash escapes.
            // Backticks allow embedded `${...}` — we ignore those structurally
            // (they may contain nested quotes and parentheses), but treat
            // them as opaque chars; this works for the noise-stripping use
            // case because we only check the OUTER literal's content.
            while i < len {
                let c = bytes[i];
                if c == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if c == quote {
                    break;
                }
                if c == b'\n' && quote != b'`' {
                    // Single/double-quoted strings can't span lines in JS;
                    // assume the source was valid and stop here.
                    break;
                }
                i += 1;
            }
            // i is now either at the closing quote or at len.
            let end = if i < len { i + 1 } else { len };
            // Only blank the INTERIOR if it contains a call-shape we care
            // about. Otherwise leave it alone — extractors need URL literals.
            let interior_start = start + 1;
            let interior_end = end.saturating_sub(1).max(interior_start);
            if interior_end > interior_start {
                let interior = &bytes[interior_start..interior_end];
                if contains_call_shape(interior) {
                    blank_range(&mut out, interior_start, interior_end);
                }
            }
            i = end;
            continue;
        }

        i += 1;
    }

    // SAFETY: blank_range only writes ASCII space and preserves newlines;
    // the original bytes were valid UTF-8 and we never split a multi-byte
    // sequence (we only walk byte-by-byte at quote / `/` characters,
    // which are all ASCII). Thus the result is still valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Replace `out[start..end]` with spaces, except newlines are preserved
/// so line numbers downstream stay correct.
fn blank_range(out: &mut [u8], start: usize, end: usize) {
    let end = end.min(out.len());
    for b in &mut out[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Quick check: does this slice contain any of the recognised call shapes?
/// Used to decide whether a string literal's interior is "noise" (a doc
/// comment in disguise) vs an actual URL we should preserve.
fn contains_call_shape(bytes: &[u8]) -> bool {
    let lower = bytes.iter().map(|b| b.to_ascii_lowercase()).collect::<Vec<u8>>();
    contains_subseq(&lower, b"fetch(")
        || contains_subseq(&lower, b"fetch ")
        || contains_subseq(&lower, b"axios.")
        || contains_subseq(&lower, b"axios(")
        || contains_subseq(&lower, b"$.ajax")
        || contains_subseq(&lower, b"jquery.ajax")
        || contains_subseq(&lower, b"new request(")
}

fn contains_subseq(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_comment_blanked_but_newline_preserved() {
        let src = "a\n// fetch('/x')\nb";
        let out = strip_noise(src);
        assert_eq!(out.len(), src.len());
        assert!(out.contains('\n'));
        assert!(!out.contains("fetch"));
    }

    #[test]
    fn block_comment_blanked() {
        let src = "x /* fetch('/y') */ z";
        let out = strip_noise(src);
        assert_eq!(out.len(), src.len());
        assert!(!out.contains("fetch"));
    }

    #[test]
    fn string_with_call_shape_blanked() {
        let src = r#"const docs = "axios.get('/example')";"#;
        let out = strip_noise(src);
        assert_eq!(out.len(), src.len());
        assert!(!out.contains("axios"));
    }

    #[test]
    fn url_literal_in_real_call_preserved() {
        // The whole input is a real call: the URL string '/api/x' must
        // survive so the extractor's FETCH pattern still matches.
        let src = "fetch('/api/x', { method: 'GET' })";
        let out = strip_noise(src);
        assert!(out.contains("/api/x"));
        assert!(out.contains("fetch"));
    }

    #[test]
    fn ordinary_string_preserved() {
        let src = r#"const name = "Alice";"#;
        let out = strip_noise(src);
        assert!(out.contains("Alice"));
    }

    #[test]
    fn escaped_quote_inside_string_handled() {
        let src = r#"const x = "a \"fetch(\" b";"#;
        let out = strip_noise(src);
        // The string contains `fetch(`, so its interior should be blanked.
        assert_eq!(out.len(), src.len());
        assert!(!out.contains("fetch"));
    }

    #[test]
    fn unterminated_block_comment_does_not_panic() {
        let src = "x /* unterminated";
        let _ = strip_noise(src);
    }

    #[test]
    fn unterminated_string_does_not_panic() {
        let src = "const x = 'unterminated";
        let _ = strip_noise(src);
    }
}
