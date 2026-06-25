use super::*;

#[test]
fn fetch_post_with_init() {
    let src = r#"
            fetch('/api/users', { method: 'POST', body: JSON.stringify(payload) });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1, "should catch one fetch call");
    assert_eq!(eps[0].method, "POST");
    assert_eq!(eps[0].path, "/api/users");
    assert_eq!(eps[0].kind, CallSiteKind::Fetch);
}

#[test]
fn fetch_default_get() {
    let src = "fetch('/api/me')";
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "GET");
    assert_eq!(eps[0].path, "/api/me");
}

#[test]
fn axios_verb_helpers() {
    let src = r#"
            axios.get('/users');
            axios.post('/orders', body);
            axios.delete('/items/123');
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 3);
    assert!(eps.iter().any(|e| e.method == "GET" && e.path == "/users"));
    assert!(eps
        .iter()
        .any(|e| e.method == "POST" && e.path == "/orders"));
    assert!(eps
        .iter()
        .any(|e| e.method == "DELETE" && e.path == "/items/123"));
}

#[test]
fn custom_http_client_verb_helpers() {
    let src = r#"
            Wr.post('/system/auth/login', body);
            t3.get('/system/auth/get-permission-info');
            aa.download('/system/dict-data/export-excel', params);
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 3);
    assert!(eps.iter().any(|e| {
        e.method == "POST"
            && e.path == "/system/auth/login"
            && e.kind == CallSiteKind::HttpClientVerb
    }));
    assert!(eps.iter().any(|e| {
        e.method == "GET"
            && e.path == "/system/auth/get-permission-info"
            && e.kind == CallSiteKind::HttpClientVerb
    }));
    assert!(eps.iter().any(|e| {
        e.method == "GET"
            && e.path == "/system/dict-data/export-excel"
            && e.kind == CallSiteKind::HttpClientVerb
    }));
}

#[test]
fn axios_verb_helpers_are_not_double_emitted_by_generic_client_pattern() {
    let src = r#"
            axios.get('/api/me');
            Wr.get('/system/user/simple-list');
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 2);
    assert_eq!(
        eps.iter()
            .filter(|e| e.path == "/api/me" && e.kind == CallSiteKind::AxiosVerb)
            .count(),
        1
    );
    assert_eq!(
        eps.iter()
            .filter(
                |e| e.path == "/system/user/simple-list" && e.kind == CallSiteKind::HttpClientVerb
            )
            .count(),
        1
    );
}

#[test]
fn axios_config_object() {
    let src = r#"
            axios({ url: '/api/login', method: 'PUT', data: payload });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "PUT");
    assert_eq!(eps[0].path, "/api/login");
}

#[test]
fn jquery_ajax() {
    let src = r#"
            $.ajax({ url: '/legacy', type: 'POST' });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "POST");
    assert_eq!(eps[0].path, "/legacy");
}

#[test]
fn new_request_constructor() {
    let src = r#"
            const req = new Request('/api/v2/data', { method: 'PATCH' });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "PATCH");
    assert_eq!(eps[0].path, "/api/v2/data");
}

#[test]
fn auth_bearer_inferred() {
    let src = r#"
            fetch('/secure', {
                method: 'GET',
                headers: { Authorization: 'Bearer ' + token }
            });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].auth, AuthHint::Bearer);
}

#[test]
fn auth_cookie_inferred() {
    let src = r#"
            fetch('/with-cookie', { credentials: 'include' });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].auth, AuthHint::Cookie);
}

#[test]
fn extract_from_files_dedupes_unique_count() {
    let files = [
        ("a.js", r#"fetch('/api/x', {method:'GET'})"#),
        ("b.js", r#"fetch('/api/x', {method:'GET'})"#),
        ("c.js", r#"axios.post('/api/y', body)"#),
    ];
    let report = extract_from_files(files.iter().map(|(p, s)| (*p, *s)));
    assert_eq!(report.endpoints.len(), 3, "all 3 occurrences listed");
    assert_eq!(report.unique, 2, "only 2 unique (method, path) pairs");
}

#[test]
fn skipped_when_no_calls() {
    let report = extract_from_files(vec![("noise.js", "console.log('hi');")]);
    assert!(report.endpoints.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].reason.contains("no recognized"));
}

// ─── path-shape inference ───────────────────────────────────────────

#[test]
fn path_with_numeric_id_marks_path_params() {
    let src = r#"axios.get('/api/users/123')"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert!(eps[0].has_path_params);
    // segments: ["api", "users", "123"] — 0-based, "123" is at idx 2
    assert_eq!(eps[0].id_param_position, Some(2));
    assert_eq!(eps[0].url_kind, UrlKind::Literal);
}

#[test]
fn path_with_uuid_marks_path_params() {
    let src = r#"fetch('/items/550e8400-e29b-41d4-a716-446655440000')"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert!(eps[0].has_path_params);
    assert_eq!(eps[0].id_param_position, Some(1));
}

#[test]
fn path_with_mongo_objectid_marks_path_params() {
    let src = r#"fetch('/orders/507f1f77bcf86cd799439011')"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert!(eps[0].has_path_params);
    assert_eq!(eps[0].id_param_position, Some(1));
}

#[test]
fn path_without_id_segments_clears_flag() {
    let src = r#"fetch('/api/health')"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert!(!eps[0].has_path_params);
    assert_eq!(eps[0].id_param_position, None);
}

// ─── concatenated URLs ──────────────────────────────────────────────

#[test]
fn fetch_concat_recognized_as_concatenated() {
    let src = r#"fetch('/api/users/' + userId)"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1, "concat should not double-emit a literal too");
    assert_eq!(eps[0].url_kind, UrlKind::Concatenated);
    assert_eq!(eps[0].path, "/api/users/");
    assert!(eps[0].has_path_params);
    assert_eq!(eps[0].id_param_position, Some(2));
}

#[test]
fn fetch_concat_with_method() {
    let src = r#"
            fetch('/api/orders/' + orderId, { method: 'DELETE' });
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "DELETE");
    assert_eq!(eps[0].url_kind, UrlKind::Concatenated);
}

// ─── template literal URLs ──────────────────────────────────────────

#[test]
fn fetch_template_recognized_as_template_literal() {
    let src = r#"fetch(`/api/users/${id}/posts`)"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1, "template should not double-emit");
    assert_eq!(eps[0].url_kind, UrlKind::TemplateLiteral);
    assert!(eps[0].path.contains("${id}"));
    assert!(eps[0].has_path_params);
    // segments: ["api", "users", "${id}", "posts"] — `${` is at idx 2
    assert_eq!(eps[0].id_param_position, Some(2));
}

#[test]
fn fetch_template_with_method_in_init() {
    let src = r#"
            fetch(`/api/items/${itemId}`, { method: 'PUT' })
        "#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "PUT");
    assert_eq!(eps[0].url_kind, UrlKind::TemplateLiteral);
}

#[test]
fn plain_fetch_still_marked_literal() {
    // Sanity: existing literal call sites must keep UrlKind::Literal
    // even after the concat/template patterns are introduced.
    let src = r#"fetch('/api/me')"#;
    let eps = extract_from_source("a.js", src);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].url_kind, UrlKind::Literal);
    assert!(!eps[0].has_path_params);
}
