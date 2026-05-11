//! End-to-end integration tests against on-disk JS fixtures.
//!
//! Unlike the inline `tests` module in `src/lib.rs` (which uses tiny
//! literal source strings to verify each pattern in isolation), these
//! tests run the full extractor against realistic fixtures —
//! `realistic_app.js` (plain JS code) and `minified_webpack.js` (a
//! webpack-style minified bundle).
//!
//! Goal: catch regressions where pattern interaction or whitespace
//! handling breaks something that the unit tests don't cover.

use std::fs;
use std::path::PathBuf;

use golish_js_analyzer::{extract_from_files, extract_from_source, CallSiteKind, UrlKind};

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

fn load_fixture(name: &str) -> String {
    let path = fixture_path(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
}

#[test]
fn realistic_app_extracts_expected_count() {
    let src = load_fixture("realistic_app.js");
    let endpoints = extract_from_source("realistic_app.js", &src);

    // Expected call-sites in realistic_app.js:
    //   - 2 plain fetch literal       (/api/me, /api/health)
    //   - 2 fetch concat              (/api/orders/, /api/users/)
    //   - 2 fetch template            (/api/users/${id}/posts, /api/items/${itemId})
    //   - 3 axios verb                (get/post/delete)
    //   - 1 axios config              (/api/login)
    //   - 1 jquery ajax               (/legacy/admin/users)
    //   - 1 new Request               (/api/v2/data/...)
    // total: 12. Comments and string-assignment noise must NOT match.
    assert_eq!(
        endpoints.len(),
        12,
        "expected 12 endpoints in realistic_app.js, got {}: {:#?}",
        endpoints.len(),
        endpoints
            .iter()
            .map(|e| (&e.method, &e.path))
            .collect::<Vec<_>>()
    );

    // Spot-check kinds and url_kinds.
    let by_kind: std::collections::HashMap<_, _> = {
        let mut m: std::collections::HashMap<CallSiteKind, usize> =
            std::collections::HashMap::new();
        for ep in &endpoints {
            *m.entry(ep.kind).or_insert(0) += 1;
        }
        m.into_iter().collect()
    };
    assert_eq!(by_kind.get(&CallSiteKind::Fetch).copied(), Some(6));
    assert_eq!(by_kind.get(&CallSiteKind::AxiosVerb).copied(), Some(3));
    assert_eq!(by_kind.get(&CallSiteKind::AxiosConfig).copied(), Some(1));
    assert_eq!(by_kind.get(&CallSiteKind::JqueryAjax).copied(), Some(1));
    assert_eq!(by_kind.get(&CallSiteKind::NewRequest).copied(), Some(1));

    // Concatenated and template-literal patterns should each appear twice.
    let concat_count = endpoints
        .iter()
        .filter(|e| e.url_kind == UrlKind::Concatenated)
        .count();
    let template_count = endpoints
        .iter()
        .filter(|e| e.url_kind == UrlKind::TemplateLiteral)
        .count();
    assert_eq!(concat_count, 2, "expected 2 concat, got {}", concat_count);
    assert_eq!(
        template_count, 2,
        "expected 2 template, got {}",
        template_count
    );

    // /api/me must be flagged Bearer (Authorization header).
    let me = endpoints
        .iter()
        .find(|e| e.path == "/api/me")
        .expect("/api/me extracted");
    assert_eq!(me.auth, golish_js_analyzer::AuthHint::Bearer);

    // axios login must be flagged Cookie (withCredentials: true).
    let login = endpoints
        .iter()
        .find(|e| e.path == "/api/login")
        .expect("/api/login extracted");
    assert_eq!(login.auth, golish_js_analyzer::AuthHint::Cookie);

    // new Request to /api/v2/data/<mongo-objectid> should mark path params.
    let mongo_call = endpoints
        .iter()
        .find(|e| e.kind == CallSiteKind::NewRequest)
        .expect("new Request extracted");
    assert!(
        mongo_call.has_path_params,
        "Mongo ObjectId should mark has_path_params"
    );
}

#[test]
fn minified_webpack_still_extracts_endpoints() {
    let src = load_fixture("minified_webpack.js");
    let endpoints = extract_from_source("minified_webpack.js", &src);

    // Even minified into a single line, regex-based extraction should
    // catch all 6 distinct call sites.
    //   - 1 plain fetch          /api/users
    //   - 2 fetch concat         /api/orders/ (twice — getOrder + deleteOrder)
    //   - 1 axios.get            /api/products
    //   - 1 axios.post           /api/login
    //   - 1 axios config         /admin/dashboard
    let count = endpoints.len();
    assert!(
        count >= 6,
        "expected >=6 endpoints in minified bundle, got {}: {:#?}",
        count,
        endpoints
            .iter()
            .map(|e| (&e.method, &e.path, e.url_kind))
            .collect::<Vec<_>>()
    );

    // Verify the admin path was caught — this is a critical IDOR / unauthorized
    // access testing target downstream.
    assert!(
        endpoints.iter().any(|e| e.path == "/admin/dashboard"),
        "/admin/dashboard must be extracted from minified bundle"
    );

    // Cookie auth detection should survive minification.
    let admin_call = endpoints
        .iter()
        .find(|e| e.path == "/admin/dashboard")
        .expect("/admin/dashboard extracted");
    assert_eq!(admin_call.auth, golish_js_analyzer::AuthHint::Cookie);
}

#[test]
fn extract_from_files_aggregates_two_fixtures() {
    let realistic = load_fixture("realistic_app.js");
    let minified = load_fixture("minified_webpack.js");
    let report = extract_from_files(vec![
        ("realistic_app.js", realistic.as_str()),
        ("minified_webpack.js", minified.as_str()),
    ]);

    assert!(report.endpoints.len() >= 18, "{:#?}", report);
    // Both fixtures hit /api/login — dedupe-aware unique count must reflect that.
    let login_hits = report
        .endpoints
        .iter()
        .filter(|e| e.path == "/api/login")
        .count();
    assert_eq!(login_hits, 2, "two files both call /api/login");
    assert!(
        report.unique < report.endpoints.len(),
        "unique count must dedupe by (method, path)"
    );
}
