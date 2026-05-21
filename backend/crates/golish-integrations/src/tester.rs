//! Test-runner implementations.
//!
//! The trait definition lives in [`crate::traits::Tester`].
//! [`DefaultTester`] implements the recipes declared in
//! [`crate::schema::TestKind`]:
//!
//! - **`Exec`** — spawns a command via `tokio::process::Command`,
//!   substitutes `{{exec}}` with the tool's executable path (passed in
//!   by the IPC facade) and `{{value:field_key}}` with the cleartext
//!   value of each declared field. Stdout + stderr are matched
//!   against `ok_regex` / `fail_regex`.
//! - **`Http`** — issues a `reqwest` request, with the same
//!   `{{value:field_key}}` substitution applied to URL and header
//!   templates. Status code is compared against the inclusive
//!   `ok_status_range`.
//! - **`Builtin`** — returns
//!   [`crate::types::HealthStatus::Unknown`] with a note explaining
//!   the caller must wire up the provider's own `test_connection`.
//!   The IPC facade is responsible for dispatching builtin tests
//!   directly to the `IntelProvider` registry.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::IntegrationResult;
use crate::schema::{IntegrationGroup, IntegrationSchema, TestKind};
use crate::traits::Tester;
use crate::types::{HealthStatus, IntegrationHealth};

/// Callback that maps `tool_id` to the absolute path of the tool's
/// executable. Returns `None` when the tool is not installed.
pub type ExecResolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Default tester. Holds an executable-resolver callback so each
/// schema can declare `{{exec}}` in its test command without this
/// crate knowing about the tool-manager.
pub struct DefaultTester {
    /// Resolves `tool_id` → absolute path of the tool's executable.
    /// Used to substitute `{{exec}}` in `TestKind::Exec`. Return
    /// `None` if the tool is not installed; the test will fail with
    /// [`HealthStatus::Unknown`].
    exec_resolver: ExecResolver,
    http_client: reqwest::Client,
}

impl DefaultTester {
    pub fn new(exec_resolver: ExecResolver) -> IntegrationResult<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| crate::IntegrationError::Internal(format!("reqwest init: {e}")))?;
        Ok(Self {
            exec_resolver,
            http_client,
        })
    }

    fn group<'s>(
        schema: &'s IntegrationSchema,
        group_id: &str,
    ) -> IntegrationResult<&'s IntegrationGroup> {
        schema
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| {
                crate::IntegrationError::SchemaNotFound(format!(
                    "group '{group_id}' not declared in schema"
                ))
            })
    }

    /// `{{value:field_key}}` → value lookup.
    fn substitute_values(template: &str, fields: &HashMap<String, String>) -> String {
        let mut out = template.to_string();
        for (k, v) in fields {
            let pat = format!("{{{{value:{k}}}}}");
            out = out.replace(&pat, v);
        }
        out
    }

    /// Match `regex` (case-insensitive by default) against `text`.
    fn matches(regex: &str, text: &str) -> bool {
        match regex::RegexBuilder::new(regex)
            .case_insensitive(true)
            .build()
        {
            Ok(re) => re.is_match(text),
            Err(_) => false,
        }
    }
}

#[async_trait]
impl Tester for DefaultTester {
    async fn test(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        cleartext_fields: &HashMap<String, String>,
    ) -> IntegrationResult<IntegrationHealth> {
        let group = Self::group(schema, group_id)?;
        let Some(test) = &group.test else {
            return Ok(IntegrationHealth::unknown(
                "no test recipe declared in schema",
            ));
        };
        match test {
            TestKind::Builtin => Ok(IntegrationHealth::unknown(
                "builtin test path: IPC facade must dispatch to the provider's test_connection",
            )),

            TestKind::Exec {
                cmd,
                ok_regex,
                fail_regex,
                timeout_secs,
            } => {
                let exec_path = match (self.exec_resolver)(tool_id) {
                    Some(p) => p,
                    None => {
                        return Ok(IntegrationHealth::unknown(format!(
                            "tool '{tool_id}' executable not found; install it first"
                        )))
                    }
                };
                let rendered = cmd.replace("{{exec}}", &exec_path);
                let rendered = Self::substitute_values(&rendered, cleartext_fields);
                // We deliberately use whitespace split (no shell escaping).
                // Schema-declared commands should keep args simple; if a
                // schema author needs spaces inside an argument they can
                // wrap with a wrapper script.
                let mut parts: Vec<String> =
                    rendered.split_whitespace().map(|s| s.to_string()).collect();
                if parts.is_empty() {
                    return Ok(IntegrationHealth::unknown("empty exec command"));
                }
                let program = parts.remove(0);
                let timeout = Duration::from_secs(*timeout_secs as u64);
                let mut child = tokio::process::Command::new(program);
                child.args(&parts);
                child.stdout(std::process::Stdio::piped());
                child.stderr(std::process::Stdio::piped());

                let output = match tokio::time::timeout(timeout, child.output()).await {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => {
                        return Ok(IntegrationHealth::unknown(format!("spawn failed: {e}")));
                    }
                    Err(_) => {
                        return Ok(IntegrationHealth::unknown(format!(
                            "command timed out after {}s",
                            timeout_secs
                        )));
                    }
                };
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let combined = format!("{stdout}\n{stderr}");
                let preview: String = combined.chars().take(512).collect();

                if let Some(fr) = fail_regex {
                    if Self::matches(fr, &combined) {
                        return Ok(IntegrationHealth::invalid(format!(
                            "fail_regex matched: {preview}"
                        )));
                    }
                }
                if Self::matches(ok_regex, &combined) {
                    Ok(IntegrationHealth::healthy("ok_regex matched"))
                } else {
                    Ok(IntegrationHealth::invalid(format!(
                        "ok_regex did not match (preview: {preview})"
                    )))
                }
            }

            TestKind::Http {
                method,
                url,
                headers,
                ok_status_range,
                timeout_secs,
            } => {
                let url_rendered = Self::substitute_values(url, cleartext_fields);
                let method_parsed = method
                    .parse::<reqwest::Method>()
                    .map_err(|e| crate::IntegrationError::Validation(format!("bad method: {e}")))?;
                let mut req = self.http_client.request(method_parsed, &url_rendered);
                for (k, v) in headers {
                    let rendered_v = Self::substitute_values(v, cleartext_fields);
                    req = req.header(k, rendered_v);
                }
                req = req.timeout(Duration::from_secs(*timeout_secs as u64));
                match req.send().await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        if status >= ok_status_range.0 && status <= ok_status_range.1 {
                            Ok(IntegrationHealth::healthy(format!(
                                "HTTP {status} (in {}-{})",
                                ok_status_range.0, ok_status_range.1
                            )))
                        } else if status == 401 || status == 403 {
                            Ok(IntegrationHealth::invalid(format!(
                                "HTTP {status} — credential rejected"
                            )))
                        } else if status == 429 {
                            Ok(IntegrationHealth {
                                status: HealthStatus::RateLimited,
                                message: "HTTP 429 — rate limited".into(),
                                tested_at: chrono::Utc::now(),
                            })
                        } else {
                            Ok(IntegrationHealth::invalid(format!(
                                "HTTP {status} — unexpected status (expected {}-{})",
                                ok_status_range.0, ok_status_range.1
                            )))
                        }
                    }
                    Err(e) if e.is_timeout() => Ok(IntegrationHealth::unknown("timeout")),
                    Err(e) => Ok(IntegrationHealth::unknown(format!("network error: {e}"))),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Field, FieldType, IntegrationGroup, IntegrationSchema, Storage, TestKind};

    fn schema_with_test(test: Option<TestKind>) -> IntegrationSchema {
        IntegrationSchema {
            category: "x".into(),
            display_name: "demo".into(),
            description: None,
            storage: Storage::vault_default(),
            help_url: None,
            groups: vec![IntegrationGroup {
                id: "default".into(),
                name: "Default".into(),
                description: None,
                icon: None,
                help_url: None,
                test,
                fields: vec![Field {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                }],
            }],
        }
    }

    fn tester() -> DefaultTester {
        DefaultTester::new(Box::new(|tool_id: &str| {
            // Pretend "echo" tools resolve to /bin/echo for the test
            if tool_id == "echo-ok" || tool_id == "echo-fail" {
                Some("/bin/echo".to_string())
            } else {
                None
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn builtin_returns_unknown_with_dispatch_hint() {
        let t = tester();
        let schema = schema_with_test(Some(TestKind::Builtin));
        let h = t
            .test("0.zone", "default", &schema, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.contains("builtin"));
    }

    #[tokio::test]
    async fn missing_test_returns_unknown() {
        let t = tester();
        let schema = schema_with_test(None);
        let h = t
            .test("0.zone", "default", &schema, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.contains("no test recipe"));
    }

    #[tokio::test]
    async fn exec_ok_regex_matches() {
        let t = tester();
        // We use /bin/echo (resolved via exec_resolver). The command
        // emits the value of {{value:api_key}} on stdout; ok_regex
        // matches the surfaced cleartext.
        let schema = schema_with_test(Some(TestKind::Exec {
            cmd: "{{exec}} HELLO_{{value:api_key}}".into(),
            ok_regex: "HELLO_secret_token_xyz".into(),
            fail_regex: None,
            timeout_secs: 5,
        }));
        let mut fields = HashMap::new();
        fields.insert("api_key".into(), "secret_token_xyz".into());
        let h = t
            .test("echo-ok", "default", &schema, &fields)
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Healthy, "got {h:?}");
    }

    #[tokio::test]
    async fn exec_fail_regex_short_circuits() {
        let t = tester();
        let schema = schema_with_test(Some(TestKind::Exec {
            cmd: "{{exec}} EXPIRED".into(),
            ok_regex: "EXPIRED".into(), // would otherwise match too
            fail_regex: Some("EXPIRED".into()),
            timeout_secs: 5,
        }));
        let h = t
            .test("echo-fail", "default", &schema, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Invalid);
        assert!(h.message.contains("fail_regex matched"));
    }

    #[tokio::test]
    async fn exec_unknown_tool_returns_unknown() {
        let t = tester();
        let schema = schema_with_test(Some(TestKind::Exec {
            cmd: "{{exec}} anything".into(),
            ok_regex: ".".into(),
            fail_regex: None,
            timeout_secs: 5,
        }));
        let h = t
            .test("nonexistent", "default", &schema, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.contains("executable not found"));
    }

    #[tokio::test]
    async fn exec_timeout_returns_unknown() {
        // /bin/sleep doesn't always exist on all CI runners, but
        // macOS / Linux ship it. Use timeout=1, sleep 5 → must timeout.
        let schema = schema_with_test(Some(TestKind::Exec {
            cmd: "{{exec}} 5".into(),
            ok_regex: ".".into(),
            fail_regex: None,
            timeout_secs: 1,
        }));
        let t = DefaultTester::new(Box::new(|tool_id: &str| {
            if tool_id == "sleeper" {
                Some("/bin/sleep".to_string())
            } else {
                None
            }
        }))
        .unwrap();
        let h = t
            .test("sleeper", "default", &schema, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.contains("timed out"));
    }

    #[test]
    fn substitute_values_replaces_all_occurrences() {
        let mut fields = HashMap::new();
        fields.insert("api_key".into(), "ABC".into());
        fields.insert("user".into(), "alice".into());
        let out = DefaultTester::substitute_values(
            "GET /{{value:user}}/keys/{{value:api_key}}?u={{value:user}}",
            &fields,
        );
        assert_eq!(out, "GET /alice/keys/ABC?u=alice");
    }

    #[test]
    fn case_insensitive_regex_match() {
        assert!(DefaultTester::matches("company_name", "COMPANY_NAME_xx"));
        assert!(DefaultTester::matches("(?i)company", "Co Co Company foo"));
        assert!(!DefaultTester::matches("nope", "nothing here"));
    }
}
