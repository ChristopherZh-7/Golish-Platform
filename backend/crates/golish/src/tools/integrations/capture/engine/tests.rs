use super::*;
use base64::{engine::general_purpose, Engine};
use golish_integrations::error::IntegrationError;
use golish_integrations::schema::{CaptureRecipe, CaptureRule};
use golish_integrations::types::{CaptureState, FailedRule};

fn aqc_recipe() -> CaptureRecipe {
    CaptureRecipe {
        login_url: "https://aiqicha.baidu.com".into(),
        success_url_pattern: None,
        visit_url: None,
        instructions: None,
        timeout_secs: 60,
        rules: vec![CaptureRule::Cookie {
            domain: ".aiqicha.baidu.com".into(),
            name: "BDUSS".into(),
            target_field: "cookies.aqc".into(),
            required: true,
        }],
    }
}

#[tokio::test]
async fn register_returns_unique_session_id() {
    let eng = CaptureEngine::new();
    let h1 = eng
        .register("enscan-go".into(), "aqc".into(), aqc_recipe())
        .await
        .unwrap();
    let h2 = eng
        .register("enscan-go".into(), "tyc".into(), aqc_recipe())
        .await
        .unwrap();
    assert_ne!(h1.session_id, h2.session_id);
}

#[tokio::test]
async fn register_rejects_duplicate_tool_group() {
    let eng = CaptureEngine::new();
    let _ = eng
        .register("enscan-go".into(), "aqc".into(), aqc_recipe())
        .await
        .unwrap();
    let err = eng
        .register("enscan-go".into(), "aqc".into(), aqc_recipe())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        IntegrationError::CaptureAlreadyRunning { .. }
    ));
}

#[tokio::test]
async fn register_after_terminal_allows_restart() {
    let eng = CaptureEngine::new();
    let h = eng
        .register("enscan-go".into(), "aqc".into(), aqc_recipe())
        .await
        .unwrap();
    eng.cancel(&h.session_id).await.unwrap();
    let _ = eng
        .register("enscan-go".into(), "aqc".into(), aqc_recipe())
        .await
        .expect("should allow restart after cancel");
}

#[tokio::test]
async fn transition_to_terminal_is_idempotent() {
    let eng = CaptureEngine::new();
    let h = eng
        .register("t".into(), "g".into(), aqc_recipe())
        .await
        .unwrap();
    eng.transition(&h.session_id, CaptureState::Captured, None)
        .await
        .unwrap();
    eng.transition(
        &h.session_id,
        CaptureState::Failed,
        Some("late update".into()),
    )
    .await
    .unwrap();
    let s = h.inner.read().await;
    assert_eq!(s.state, CaptureState::Captured);
    assert!(
        s.error_message.is_none(),
        "error must not be stamped post-terminal"
    );
}

#[tokio::test]
async fn get_unknown_returns_not_found() {
    let eng = CaptureEngine::new();
    let err = eng.get("nope").await.unwrap_err();
    assert!(matches!(err, IntegrationError::CaptureSessionNotFound(_)));
}

#[tokio::test]
async fn cancel_transitions_to_cancelled() {
    let eng = CaptureEngine::new();
    let h = eng
        .register("t".into(), "g".into(), aqc_recipe())
        .await
        .unwrap();
    eng.cancel(&h.session_id).await.unwrap();
    let s = h.inner.read().await;
    assert_eq!(s.state, CaptureState::Cancelled);
    assert!(s.state.is_terminal());
}

#[tokio::test]
async fn soft_retry_rearms_waiting_login_after_empty_cookie_attempt() {
    let eng = CaptureEngine::new();
    let h = eng
        .register("t".into(), "g".into(), aqc_recipe())
        .await
        .unwrap();

    {
        let mut s = h.inner.write().await;
        s.transition(CaptureState::Extracting);
        s.failed_rules.push(FailedRule {
            rule_index: 0,
            reason: "[SOFT_RETRY] required cookies not yet present".into(),
        });
        s.captured_fields.push("cookies.aqc".into());
    }

    eng.rearm_after_soft_retry(&h).await;

    let s = h.inner.read().await;
    assert_eq!(s.state, CaptureState::WaitingLogin);
    assert!(s.failed_rules.is_empty());
    assert!(s.captured_fields.is_empty());
}

#[test]
fn required_request_header_failures_are_soft_retryable() {
    let reason = request_header_failure_reason("X-Tycid", "value was empty", true);

    assert!(
        reason.starts_with("[SOFT_RETRY]"),
        "missing required request headers should keep the capture window open"
    );
    assert!(reason.contains("request header 'X-Tycid' not observed"));
}

#[test]
fn required_cookie_failures_are_soft_retryable() {
    let reason = cookie_failure_reason(".qimai.cn", "QIMOSESSID", true);

    assert!(
        reason.starts_with("[SOFT_RETRY]"),
        "missing required cookies should keep the capture window open"
    );
    assert!(reason.contains("cookie 'QIMOSESSID' not found"));
}

#[test]
fn required_cookie_joined_empty_matches_are_soft_retryable() {
    let reason = cookie_joined_failure_reason(".riskbird.com", &[], true);

    assert!(
        reason.starts_with("[SOFT_RETRY]"),
        "empty required cookie joins should keep the capture window open"
    );
    assert!(reason.contains("no cookies matched for domain '.riskbird.com'"));
}

#[test]
fn required_cookie_joined_min_count_failures_are_soft_retryable() {
    let reason = cookie_joined_min_count_failure_reason(".qimai.cn", 1, 2, true);

    assert!(
        reason.starts_with("[SOFT_RETRY]"),
        "anonymous cookie sets should keep the capture window open"
    );
    assert!(reason.contains("got 1, need at least 2"));
}

#[test]
fn format_joined_cookies_all_when_names_empty() {
    let cookies = vec![
        ("BDUSS".to_string(), "xx".to_string()),
        ("BAIDUID".to_string(), "yy".to_string()),
        ("PSTM".to_string(), "12345".to_string()),
    ];
    let parts = format_joined_cookies(&[], "{name}={value}", &cookies);
    assert_eq!(
        parts,
        vec!["BDUSS=xx", "BAIDUID=yy", "PSTM=12345"],
        "names=[] should take every cookie in order"
    );
}

#[test]
fn format_joined_cookies_filters_to_names_list() {
    let cookies = vec![
        ("BDUSS".to_string(), "xx".to_string()),
        ("BAIDUID".to_string(), "yy".to_string()),
        ("UNRELATED".to_string(), "drop_me".to_string()),
    ];
    let parts = format_joined_cookies(
        &["BDUSS".to_string(), "BAIDUID".to_string()],
        "{name}={value}",
        &cookies,
    );
    assert_eq!(parts, vec!["BDUSS=xx", "BAIDUID=yy"]);
}

#[test]
fn format_joined_cookies_custom_fmt_template() {
    let cookies = vec![("BDUSS".to_string(), "abc".to_string())];
    let parts = format_joined_cookies(&[], "Cookie: {name}: {value}", &cookies);
    assert_eq!(parts, vec!["Cookie: BDUSS: abc"]);
}

#[test]
fn format_joined_cookies_empty_input_returns_empty() {
    let parts = format_joined_cookies(&[], "{name}={value}", &[]);
    assert!(parts.is_empty());
}

#[test]
fn cookie_domain_matches_exact_host() {
    assert!(cookie_domain_matches("baidu.com", "baidu.com"));
    assert!(cookie_domain_matches("BAIDU.COM", "baidu.com"));
}

#[test]
fn cookie_domain_matches_strips_leading_dot() {
    // This is the case that wry 0.54's `cookies_for_url` botches:
    // NSHTTPCookie hands back `.baidu.com`, url.domain() returns
    // `baidu.com`, wry compares with `==` and drops every cookie.
    assert!(cookie_domain_matches(".baidu.com", "baidu.com"));
    assert!(cookie_domain_matches(".BAIDU.COM", "baidu.com"));
}

#[test]
fn cookie_domain_matches_subdomain_under_target() {
    // Captures aiqicha-host-only cookies when the rule asks for
    // ".baidu.com" — needed because AQC sets some session cookies
    // host-only on aiqicha.baidu.com that ENScan_GO still needs.
    assert!(cookie_domain_matches("aiqicha.baidu.com", "baidu.com"));
    assert!(cookie_domain_matches(".aiqicha.baidu.com", "baidu.com"));
}

#[test]
fn cookie_domain_matches_rejects_unrelated() {
    // Guardrail: a baidubcd.com cookie must NOT match baidu.com
    // (suffix match must be on a `.` boundary, not raw substring).
    assert!(!cookie_domain_matches("baidubcd.com", "baidu.com"));
    assert!(!cookie_domain_matches("evilbaidu.com", "baidu.com"));
    assert!(!cookie_domain_matches("notbaidu.com", "baidu.com"));
    assert!(!cookie_domain_matches("baidu.com.evil.com", "baidu.com"));
}

#[test]
fn cookie_domain_matches_empty_inputs_return_false() {
    assert!(!cookie_domain_matches("", "baidu.com"));
    assert!(!cookie_domain_matches(".", "baidu.com"));
    assert!(!cookie_domain_matches("baidu.com", ""));
}

#[test]
fn parse_js_value_title_decodes_ok_payload() {
    let payload = general_purpose::STANDARD.encode("secret-token");
    let title = format!("{JS_VALUE_TITLE_PREFIX}nonce-1:ok:{payload}");
    let (nonce, value) = parse_js_value_title(&title).expect("capture title should parse");
    assert_eq!(nonce, "nonce-1");
    assert_eq!(value.unwrap(), "secret-token");
}

#[test]
fn parse_js_value_title_decodes_error_payload() {
    let payload = general_purpose::STANDARD.encode("missing selector");
    let title = format!("{JS_VALUE_TITLE_PREFIX}nonce-2:err:{payload}");
    let (nonce, value) = parse_js_value_title(&title).expect("capture title should parse");
    assert_eq!(nonce, "nonce-2");
    assert_eq!(value.unwrap_err(), "missing selector");
}

#[tokio::test]
async fn gc_drops_only_old_terminal_sessions() {
    let eng = CaptureEngine::new();
    // Fresh terminal: should NOT be GC'd.
    let h_fresh = eng
        .register("t".into(), "fresh".into(), aqc_recipe())
        .await
        .unwrap();
    eng.cancel(&h_fresh.session_id).await.unwrap();

    // Stale terminal: backdate updated_at_ms to before the cutoff.
    let h_stale = eng
        .register("t".into(), "stale".into(), aqc_recipe())
        .await
        .unwrap();
    eng.cancel(&h_stale.session_id).await.unwrap();
    {
        let mut s = h_stale.inner.write().await;
        s.updated_at_ms = chrono::Utc::now().timestamp_millis() - GC_RETENTION_MS - 1;
    }

    // Non-terminal: should NOT be touched.
    let h_active = eng
        .register("t".into(), "active".into(), aqc_recipe())
        .await
        .unwrap();

    assert_eq!(eng.session_count().await, 3);
    eng.gc().await;
    assert_eq!(eng.session_count().await, 2, "only stale terminal removed");

    // active + fresh-cancelled should remain.
    assert!(eng.get(&h_fresh.session_id).await.is_ok());
    assert!(eng.get(&h_active.session_id).await.is_ok());
    assert!(eng.get(&h_stale.session_id).await.is_err());
}
