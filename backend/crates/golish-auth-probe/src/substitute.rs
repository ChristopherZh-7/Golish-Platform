//! Path ID substitution for cross-user IDOR scenarios.
//!
//! Given a path with an ID-shaped segment (or a Concatenated /
//! TemplateLiteral path with `id_param_position`), produce a new path
//! where that segment is replaced with `new_id`.

use golish_js_analyzer::{Endpoint, UrlKind};

/// Build a probe-ready URL from the endpoint's recorded path, optionally
/// substituting `new_id` into the ID slot.
///
/// Returns:
/// - `Some(path)` — path is ready to send (substituted when needed)
/// - `None` — endpoint isn't suitable for this kind of probe
///   (e.g. cross-user scenario with no `has_path_params`)
///
/// `kind` controls behavior:
/// - `SubstituteKind::SameId`: keep the path as-is. For Concatenated
///   URLs, append the original ID (not provided here — handled by the
///   caller via `default_id`). For Template literals, leave `${...}`
///   in place (caller must replace before sending).
/// - `SubstituteKind::NewId(s)`: replace the segment at
///   `id_param_position` with `s`. For Concatenated, append `s` after
///   the prefix. For TemplateLiteral, replace the placeholder segment
///   with `s`.
pub enum SubstituteKind<'a> {
    SameId { default_id: &'a str },
    NewId { id: &'a str },
}

pub fn substitute_id(endpoint: &Endpoint, kind: SubstituteKind<'_>) -> Option<String> {
    match endpoint.url_kind {
        UrlKind::Literal => substitute_literal(endpoint, kind),
        UrlKind::Concatenated => substitute_concatenated(endpoint, kind),
        UrlKind::TemplateLiteral => substitute_template(endpoint, kind),
    }
}

fn substitute_literal(endpoint: &Endpoint, kind: SubstituteKind<'_>) -> Option<String> {
    match kind {
        SubstituteKind::SameId { .. } => Some(endpoint.path.clone()),
        SubstituteKind::NewId { id } => {
            let pos = endpoint.id_param_position?;
            let leading_slash = endpoint.path.starts_with('/');
            let trimmed = endpoint
                .path
                .trim_start_matches('/')
                .trim_end_matches('/');
            let mut segments: Vec<String> =
                trimmed.split('/').map(|s| s.to_string()).collect();
            if pos >= segments.len() {
                return None;
            }
            segments[pos] = id.to_string();
            let mut out = segments.join("/");
            if leading_slash {
                out.insert(0, '/');
            }
            // Preserve trailing slash if the original had one.
            if endpoint.path.ends_with('/') && !out.ends_with('/') {
                out.push('/');
            }
            Some(out)
        }
    }
}

fn substitute_concatenated(endpoint: &Endpoint, kind: SubstituteKind<'_>) -> Option<String> {
    match kind {
        SubstituteKind::SameId { default_id } => {
            // Concatenated path is the literal prefix; append the
            // caller-provided default id so the request is well-formed.
            let mut out = endpoint.path.clone();
            if !out.ends_with('/') && !default_id.is_empty() {
                out.push('/');
            }
            out.push_str(default_id);
            Some(out)
        }
        SubstituteKind::NewId { id } => {
            let mut out = endpoint.path.clone();
            if !out.ends_with('/') && !id.is_empty() {
                out.push('/');
            }
            out.push_str(id);
            Some(out)
        }
    }
}

fn substitute_template(endpoint: &Endpoint, kind: SubstituteKind<'_>) -> Option<String> {
    let pos = endpoint.id_param_position?;
    let leading_slash = endpoint.path.starts_with('/');
    let trimmed = endpoint
        .path
        .trim_start_matches('/')
        .trim_end_matches('/');
    let mut segments: Vec<String> = trimmed.split('/').map(|s| s.to_string()).collect();
    if pos >= segments.len() {
        return None;
    }
    let replacement = match kind {
        SubstituteKind::SameId { default_id } => default_id.to_string(),
        SubstituteKind::NewId { id } => id.to_string(),
    };
    segments[pos] = replacement;
    let mut out = segments.join("/");
    if leading_slash {
        out.insert(0, '/');
    }
    if endpoint.path.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_js_analyzer::{AuthHint, CallSiteKind, Endpoint, UrlKind};

    fn ep(path: &str, kind: UrlKind, id_pos: Option<usize>, has_params: bool) -> Endpoint {
        Endpoint {
            method: "GET".into(),
            path: path.into(),
            auth: AuthHint::None,
            source_file: "test.js".into(),
            line: 1,
            confidence: 1.0,
            kind: CallSiteKind::Fetch,
            url_kind: kind,
            has_path_params: has_params,
            id_param_position: id_pos,
        }
    }

    #[test]
    fn literal_replaces_segment() {
        let e = ep("/api/users/123", UrlKind::Literal, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert_eq!(out.as_deref(), Some("/api/users/999"));
    }

    #[test]
    fn literal_preserves_trailing_slash() {
        let e = ep("/api/users/123/", UrlKind::Literal, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert_eq!(out.as_deref(), Some("/api/users/999/"));
    }

    #[test]
    fn concat_appends_id() {
        let e = ep("/api/users/", UrlKind::Concatenated, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert_eq!(out.as_deref(), Some("/api/users/999"));
    }

    #[test]
    fn concat_without_trailing_slash_inserts_one() {
        let e = ep("/api/users", UrlKind::Concatenated, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert_eq!(out.as_deref(), Some("/api/users/999"));
    }

    #[test]
    fn template_replaces_placeholder_segment() {
        let e = ep("/api/users/${id}/posts", UrlKind::TemplateLiteral, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert_eq!(out.as_deref(), Some("/api/users/999/posts"));
    }

    #[test]
    fn same_id_is_idempotent_for_literal() {
        let e = ep("/api/users/123", UrlKind::Literal, Some(2), true);
        let out = substitute_id(&e, SubstituteKind::SameId { default_id: "" });
        assert_eq!(out.as_deref(), Some("/api/users/123"));
    }

    #[test]
    fn same_id_appends_default_for_concat() {
        let e = ep("/api/users/", UrlKind::Concatenated, Some(2), true);
        let out = substitute_id(
            &e,
            SubstituteKind::SameId {
                default_id: "abc-original",
            },
        );
        assert_eq!(out.as_deref(), Some("/api/users/abc-original"));
    }

    #[test]
    fn missing_id_position_returns_none_for_literal_replace() {
        let e = ep("/api/users/123", UrlKind::Literal, None, true);
        let out = substitute_id(&e, SubstituteKind::NewId { id: "999" });
        assert!(out.is_none());
    }
}
