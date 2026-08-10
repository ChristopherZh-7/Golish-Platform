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
fn regex_extraction_marks_source_regex() {
    let eps = extract_from_source("a.js", "fetch('/api/me')");
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].source, EndpointSource::Regex);
}

#[test]
fn endpoint_source_defaults_to_regex_on_old_json() {
    // Persisted js_analysis_results rows written before the `source` field
    // existed must still deserialize (defaulting to Regex), not error.
    let old = r#"{"method":"GET","path":"/a","auth":"none","source_file":"a.js","line":1,"confidence":1.0,"kind":"fetch","url_kind":"literal","has_path_params":false,"id_param_position":null}"#;
    let ep: Endpoint = serde_json::from_str(old).expect("old endpoint JSON deserializes");
    assert_eq!(ep.source, EndpointSource::Regex);
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
fn candidate_api_preserves_custom_client_receiver_and_exact_span() {
    let src = "const admin = axios.create({ baseURL: '/admin-api' });admin.get('/users');";

    let candidates = extract_candidates_from_source("app.js", src);

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.endpoint.method, "GET");
    assert_eq!(candidate.endpoint.path, "/users");
    assert_eq!(candidate.call.callee, "admin.get");
    assert_eq!(candidate.call.receiver.as_deref(), Some("admin"));
    assert_eq!(candidate.call.span.line, 1);
    assert_eq!(candidate.call.span.column, src.find("admin.get").unwrap());
    assert_eq!(
        &src[candidate.call.span.start_byte..candidate.call.span.end_byte],
        "admin.get('/users')"
    );
}

#[test]
fn candidate_api_distinguishes_minified_same_line_calls_by_byte_span() {
    let src = "admin.get('/users');open.get('/users');";

    let candidates = extract_candidates_from_source("app.js", src);

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].call.receiver.as_deref(), Some("admin"));
    assert_eq!(candidates[1].call.receiver.as_deref(), Some("open"));
    assert_eq!(candidates[0].call.span.line, 1);
    assert_eq!(candidates[1].call.span.line, 1);
    assert_ne!(
        candidates[0].call.span.start_byte,
        candidates[1].call.span.start_byte
    );
    assert_eq!(
        &src[candidates[0].call.span.start_byte..candidates[0].call.span.end_byte],
        "admin.get('/users')"
    );
    assert_eq!(
        &src[candidates[1].call.span.start_byte..candidates[1].call.span.end_byte],
        "open.get('/users')"
    );
}

#[test]
fn candidate_file_report_keeps_occurrences_while_legacy_report_stays_compatible() {
    let files = [
        ("a.js", "admin.get('/users')"),
        ("b.js", "open.get('/users')"),
    ];

    let candidate_report =
        extract_candidates_from_files(files.iter().map(|(path, source)| (*path, *source)));
    let legacy_report = extract_from_files(files.iter().map(|(path, source)| (*path, *source)));

    assert_eq!(candidate_report.candidates.len(), 2);
    assert_eq!(candidate_report.unique, 1);
    assert_eq!(legacy_report.endpoints.len(), 2);
    assert_eq!(legacy_report.unique, 1);
    assert_eq!(
        candidate_report
            .candidates
            .iter()
            .map(|candidate| &candidate.endpoint.path)
            .collect::<Vec<_>>(),
        legacy_report
            .endpoints
            .iter()
            .map(|endpoint| &endpoint.path)
            .collect::<Vec<_>>()
    );
}

#[test]
fn candidate_api_preserves_full_member_chain_without_changing_legacy_api() {
    let source = concat!(
        "const api=axios.create({baseURL:'/local'});",
        "this.api.get('/wrong');services.api.post('/also-wrong');",
        "api.get('/right');"
    );

    let candidates = extract_candidates_from_source("app.js", source);
    let legacy = extract_from_source("app.js", source);

    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].call.receiver.as_deref(), Some("this.api"));
    assert_eq!(candidates[0].call.callee, "this.api.get");
    assert_eq!(candidates[0].endpoint.path, "/wrong");
    assert_eq!(candidates[1].call.receiver.as_deref(), Some("services.api"));
    assert_eq!(candidates[1].call.callee, "services.api.post");
    assert_eq!(candidates[1].endpoint.path, "/also-wrong");
    assert_eq!(candidates[2].call.receiver.as_deref(), Some("api"));
    assert_eq!(candidates[2].endpoint.path, "/right");
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].path, "/right");
}

#[test]
fn candidate_api_keeps_optional_member_chain_opaque() {
    let source = concat!(
        "const api=axios.create({baseURL:'/local'});",
        "this?.api.get('/wrong');api.get('/right');"
    );

    let candidates = extract_candidates_from_source("app.js", source);
    let legacy = extract_from_source("app.js", source);

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].call.receiver.as_deref(), Some("this?.api"));
    assert_eq!(candidates[0].call.callee, "this?.api.get");
    assert_eq!(candidates[1].call.receiver.as_deref(), Some("api"));
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].path, "/right");
}

#[test]
fn candidate_api_adds_relative_custom_client_paths_without_changing_legacy_api() {
    let source = "const api=axios.create({baseURL:'/v2'});api.get('users');";

    let candidates = extract_candidates_from_source("app.js", source);
    let legacy = extract_from_source("app.js", source);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].call.receiver.as_deref(), Some("api"));
    assert_eq!(candidates[0].endpoint.path, "users");
    assert!(
        legacy.is_empty(),
        "legacy Endpoint contract remains root-relative"
    );
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

#[test]
fn candidate_params_are_bound_to_exact_minified_callsite() {
    let source = concat!(
        "fetch('/one',{body:JSON.stringify({first:secretOne})});",
        "fetch('/two',{body:JSON.stringify({second:secretTwo})})"
    );

    let candidates = extract_candidates_from_source("min.js", source);

    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0]
            .parameters
            .iter()
            .map(|fact| fact.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first"]
    );
    assert_eq!(
        candidates[1]
            .parameters
            .iter()
            .map(|fact| fact.name.as_str())
            .collect::<Vec<_>>(),
        vec!["second"]
    );
    assert_eq!(
        &source[candidates[0].call.span.start_byte..candidates[0].call.span.end_byte],
        "fetch('/one',{body:JSON.stringify({first:secretOne})})"
    );
    assert_eq!(
        &source[candidates[1].call.span.start_byte..candidates[1].call.span.end_byte],
        "fetch('/two',{body:JSON.stringify({second:secretTwo})})"
    );
}

#[test]
fn fetch_config_extracts_query_body_and_header_names_without_values() {
    let source = concat!(
        "fetch('/users?tenant=private-tenant',{method:'POST',",
        "body:JSON.stringify({displayName:'Alice',token:secretToken}),",
        "headers:{Authorization:bearerToken,'X-Trace':traceId}})"
    );

    let candidate = extract_candidates_from_source("fetch.js", source)
        .into_iter()
        .next()
        .expect("fetch candidate");

    assert_eq!(candidate.adapter, CallAdapter::Fetch);
    assert!(candidate.parameters.contains(&ParameterFact {
        name: "tenant".to_string(),
        location: ParameterLocation::Query,
        value_type: ParameterValueType::Unknown,
    }));
    assert!(candidate.parameters.contains(&ParameterFact {
        name: "displayName".to_string(),
        location: ParameterLocation::Body,
        value_type: ParameterValueType::String,
    }));
    assert!(candidate.parameters.contains(&ParameterFact {
        name: "token".to_string(),
        location: ParameterLocation::Body,
        value_type: ParameterValueType::Unknown,
    }));
    assert!(candidate.parameters.contains(&ParameterFact {
        name: "Authorization".to_string(),
        location: ParameterLocation::Header,
        value_type: ParameterValueType::Unknown,
    }));
    let serialized = serde_json::to_string(&candidate.parameters).expect("serialize facts");
    for forbidden in [
        "private-tenant",
        "Alice",
        "secretToken",
        "bearerToken",
        "traceId",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn axios_config_extracts_body_and_params_by_location() {
    let source = concat!(
        "axios.post('/orders',{sku:'sku-secret',quantity:2},",
        "{params:{dryRun:true},headers:{'X-CSRF':csrfToken}});",
        "axios({url:'/search',method:'POST',data:{term:searchTerm},params:{page:1}})"
    );

    let candidates = extract_candidates_from_source("axios.js", source);
    assert_eq!(candidates.len(), 2);
    let post = candidates
        .iter()
        .find(|candidate| candidate.endpoint.path == "/orders")
        .expect("axios verb candidate");
    assert!(post.parameters.contains(&ParameterFact {
        name: "sku".to_string(),
        location: ParameterLocation::Body,
        value_type: ParameterValueType::String,
    }));
    assert!(post.parameters.contains(&ParameterFact {
        name: "dryRun".to_string(),
        location: ParameterLocation::Query,
        value_type: ParameterValueType::Boolean,
    }));
    assert!(post.parameters.contains(&ParameterFact {
        name: "X-CSRF".to_string(),
        location: ParameterLocation::Header,
        value_type: ParameterValueType::Unknown,
    }));

    let config = candidates
        .iter()
        .find(|candidate| candidate.endpoint.path == "/search")
        .expect("axios config candidate");
    assert!(config.parameters.contains(&ParameterFact {
        name: "term".to_string(),
        location: ParameterLocation::Body,
        value_type: ParameterValueType::Unknown,
    }));
    assert!(config.parameters.contains(&ParameterFact {
        name: "page".to_string(),
        location: ParameterLocation::Query,
        value_type: ParameterValueType::Number,
    }));
}

#[test]
fn request_and_jquery_extract_form_fields() {
    let source = concat!(
        "new Request('/submit',{method:'POST',body:new URLSearchParams({email,password})});",
        "$.ajax({url:'/legacy-submit',type:'POST',data:{email,password}})"
    );

    let candidates = extract_candidates_from_source("forms.js", source);
    assert_eq!(candidates.len(), 2);
    for candidate in &candidates {
        for name in ["email", "password"] {
            assert!(candidate.parameters.contains(&ParameterFact {
                name: name.to_string(),
                location: ParameterLocation::Form,
                value_type: ParameterValueType::Unknown,
            }));
        }
    }
}

#[test]
fn graphql_extracts_operation_and_variable_names() {
    let source = concat!(
        "graphql('/graphql',{query:gql`query User($id: ID!, $filter: Filter){",
        "user(id:$id){id}}`,variables:{id,filter:buildFilter()}})"
    );

    let candidate = extract_candidates_from_source("graphql.js", source)
        .into_iter()
        .next()
        .expect("GraphQL candidate");

    assert_eq!(candidate.adapter, CallAdapter::Graphql);
    assert_eq!(
        candidate.graphql_operation,
        Some(GraphqlOperationFact {
            kind: GraphqlOperationKind::Query,
            name: Some("User".to_string()),
        })
    );
    for name in ["id", "filter"] {
        assert!(candidate.parameters.iter().any(|fact| {
            fact.name == name && fact.location == ParameterLocation::GraphqlVariable
        }));
    }
}

#[test]
fn dynamic_values_keep_field_names_and_unknown_types() {
    let source = "axios.post('/profiles',{accountId,profile:buildProfile(),active:true})";

    let candidate = extract_candidates_from_source("dynamic.js", source)
        .into_iter()
        .next()
        .expect("axios candidate");

    for name in ["accountId", "profile"] {
        assert!(candidate.parameters.contains(&ParameterFact {
            name: name.to_string(),
            location: ParameterLocation::Body,
            value_type: ParameterValueType::Unknown,
        }));
    }
    assert!(candidate.parameters.contains(&ParameterFact {
        name: "active".to_string(),
        location: ParameterLocation::Body,
        value_type: ParameterValueType::Boolean,
    }));
}

#[test]
fn two_equal_paths_keep_distinct_candidate_ids_and_spans() {
    let source = "fetch('/same');fetch('/same')";

    let candidates = extract_candidates_from_source("duplicates.js", source);

    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].candidate_id, candidates[1].candidate_id);
    assert_ne!(candidates[0].call.span, candidates[1].call.span);
    assert!(candidates
        .iter()
        .all(|candidate| candidate.candidate_id.starts_with("js-callsite-v1:")));
}

#[test]
fn legacy_endpoint_projection_remains_serde_compatible() {
    let source = concat!(
        "new WebSocket('/socket');xhr.open('POST','/xhr');",
        "axios.post('/orders',{sku:'secret'});fetch('/users');",
        "graphql('/graphql',{query:'query Ping { ping }'})"
    );

    let candidates = extract_candidates_from_source("compat.js", source);
    let legacy = extract_from_source("compat.js", source);
    assert_eq!(candidates.len(), 5);
    assert_eq!(
        legacy
            .iter()
            .map(|endpoint| (endpoint.method.as_str(), endpoint.path.as_str()))
            .collect::<Vec<_>>(),
        vec![("GET", "/users"), ("POST", "/orders")]
    );
    let legacy_json = serde_json::to_value(&legacy).expect("serialize legacy endpoints");
    let object = legacy_json[0].as_object().expect("legacy endpoint object");
    assert!(!object.contains_key("candidate_id"));
    assert!(!object.contains_key("parameters"));
    assert!(!object.contains_key("config"));

    let old_candidate_json = r#"{
        "endpoint":{"method":"GET","path":"/old","auth":"none","source_file":"old.js","line":1,"confidence":0.9,"kind":"fetch","url_kind":"literal","has_path_params":false,"id_param_position":null,"source":"regex"},
        "call":{"callee":"fetch","receiver":null,"span":{"start_byte":0,"end_byte":12,"line":1,"column":0}}
    }"#;
    let old: EndpointCandidate =
        serde_json::from_str(old_candidate_json).expect("old candidate JSON remains readable");
    assert!(old.candidate_id.is_empty());
    assert_eq!(old.adapter, CallAdapter::Raw);
    assert!(old.parameters.is_empty());
    assert!(old.config.is_empty());
}
