# 通用凭据抓取器 (Credential Capture Engine) · 实施计划

> **面向 AI 代理的工作者**：必需子技能 `superpowers:executing-plans`——逐 task 实施本计划，每个 Phase 末尾的 Review Checkpoint 必须通过才能进入下一 Phase。

**目标**：把用户「点链接 → 打开浏览器 → 登录 → F12 拷 cookie → 粘回 Integrations」的 5 步缩成「点 ⚡ → 在弹窗里登录 → 自动填值」的 2 步；且对**任意**未来需要登录后拿凭据的 integration 通用。

**架构**：在 `IntegrationGroup` schema 上加可选 `capture: Option<CaptureRecipe>` 字段；Rust `CaptureEngine` 用 Tauri 2 `WebviewWindowBuilder` 开独立 data_directory 隔离窗口；用户在弹窗里登录 → 引擎监听 navigation → 命中 `success_url_pattern` 时抠 cookie → 直接写 vault → 前端订阅 `integration-capture` event 自动刷新表单。

**技术栈**：Rust 2021（`tauri = "2"` / `regex` / `secrecy` / `uuid` / `tokio::sync`）+ React 19（`@tauri-apps/api/event listen` / `react-query`）+ 现有 `golish-integrations` crate。

**关联设计文档**：`docs/design/2026-05-21-credential-capture-engine.md`（请先读完 14 小节再开工，本计划只是落地步骤）。

**分支**：跟随 `integrations` 继续推进，不另开新分支。

**预计总工时**：1.5-2 天（P1 MVP）。

---

## Phase 0 · 启动 spike：确认 Tauri 2 API 表面（30 分钟，必做）

**目标**：在动任何业务代码之前，先用 50 行 Rust + 1 个 binary 验证 Tauri 2 在当前锁定版本里 `WebviewWindowBuilder::data_directory` / `WebviewWindow::cookies_for_url` / `WebviewWindow::on_navigation` 三个 API 真的存在且行为符合设计文档假设。**这 30 分钟不省**——避免 Phase 2 一半才发现 API 名变了要返工。

### T0.1 · 锁定 Tauri 版本号

**文件**：只读 `backend/Cargo.toml`

**步骤**：

1. `Read backend/Cargo.toml`，找 `[workspace.dependencies]` 段里 `tauri = ...` 那行。
2. 记录精确 minor 版本号到本次会话的 progress log（如 `tauri = "2.1.0"`）。

**验证**：
```bash
cd backend && cargo tree -p golish --depth 1 | rg '^├── tauri '
```
预期：输出一行 `├── tauri v2.x.y`，与 Cargo.toml 锁定版本一致。

**提交**：无（仅记录）。

### T0.2 · spike binary 验三个 API

**文件**：新建 `backend/crates/golish/examples/capture_spike.rs`

**步骤**：

1. 创建文件 `backend/crates/golish/examples/capture_spike.rs`，写入以下代码：

```rust
//! Capture engine API spike (run via `cargo run --example capture_spike -p golish`).
//!
//! Goal: prove `WebviewWindowBuilder::data_directory`,
//! `WebviewWindow::cookies_for_url`, `WebviewWindow::on_navigation`
//! exist and behave as expected in the locked Tauri 2.x version.
//! Delete this file once Phase 2 lands.

use std::time::Duration;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap()
                .join("capture-spike-tmp");
            let _ = std::fs::create_dir_all(&data_dir);
            let win = WebviewWindowBuilder::new(
                app,
                "capture-spike",
                WebviewUrl::External("https://aiqicha.baidu.com".parse().unwrap()),
            )
            .title("Capture API Spike")
            .inner_size(900.0, 700.0)
            .data_directory(data_dir.clone())
            .on_navigation(|new_url| {
                println!("[spike] navigated to: {}", new_url);
                true
            })
            .build()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(15)).await;
                if let Some(w) = handle.get_webview_window("capture-spike") {
                    let cookies = w
                        .cookies_for_url(
                            "https://aiqicha.baidu.com".parse().unwrap(),
                        )
                        .await
                        .unwrap_or_default();
                    println!("[spike] {} cookies after 15s", cookies.len());
                    for c in &cookies {
                        println!("[spike]   {} = ****", c.name());
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("spike");
}
```

2. 运行：
```bash
cd backend && cargo run --example capture_spike -p golish
```

3. 在弹出窗口里打开 `aiqicha.baidu.com`（不需要真登录），等 15 秒。

**验证**：
- 终端输出至少一行 `[spike] navigated to: ...`（证明 `on_navigation` 回调被触发）
- 15 秒后输出 `[spike] N cookies after 15s`，N 至少 ≥ 1（证明 `cookies_for_url` 能读到 cookies）
- `data_dir` 目录下生成了 webview 缓存文件（macOS：`~/Library/Application Support/com.golish.platform/capture-spike-tmp/`）

**预期失败处理**：
- API 名不一致（如叫 `cookies_for_origin` 而不是 `cookies_for_url`）→ 编译报错；查阅 `cargo doc -p tauri --open` 找正确 API；更新设计文档第 5.2 / 5.4 节。
- Linux WebKitGTK 不支持 `data_directory` → 在 progress log 标 R1，限定 P1 仅 macOS + Windows，Linux 用 fallback 单 dir。

**提交**：无（spike 临时文件，Phase 2 末尾删除）。

---

## Phase 1 · Schema 扩展 + 类型定义（90 分钟）

**目标**：`IntegrationGroup` 加可选 `capture` 字段；`CaptureRecipe` / `CaptureRule` enum 全部定义；事件 / IPC 入参出参类型定义；`integrations_list_schemas` 返回的 schema 含 capture 段（端到端类型链通）。

**Tasks**：

### T1.1 · 在 schema.rs 加 `CaptureRecipe` + `CaptureRule` 类型

**文件**：修改 `backend/crates/golish-integrations/src/schema.rs`

**步骤**：

1. 在文件末尾（`#[cfg(test)] mod tests` 之前）追加以下代码：

```rust
// ────────────────────────────────────────────────────────────────────────
// CaptureRecipe — "click ⚡ to harvest creds from a browser session"
//
// Architecture: docs/design/2026-05-21-credential-capture-engine.md
// ────────────────────────────────────────────────────────────────────────

/// A single capture recipe describing *how* Golish opens a webview,
/// detects login success, and extracts credentials into the schema's
/// declared fields.
///
/// Attached as `IntegrationGroup.capture: Option<CaptureRecipe>`. When
/// `None`, the frontend does not render the ⚡ button and the user
/// must fill the form manually (unchanged from pre-capture behavior).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureRecipe {
    /// HTTPS URL to navigate to in the capture webview. Must parse as
    /// `http://` or `https://` (validated server-side at schema load).
    pub login_url: String,

    /// Regex applied to every navigation target URL. On match, the
    /// engine triggers rule extraction. When `None`, the engine only
    /// extracts when the user clicks the manual "complete" button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_url_pattern: Option<String>,

    /// Optional URL to navigate to *after* `success_url_pattern`
    /// matches but *before* running rules. Useful for sites that
    /// only show the API key on a settings page distinct from the
    /// login-success landing page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visit_url: Option<String>,

    /// Short Markdown / plain text shown in the confirm dialog. The
    /// frontend may fall back to the i18n key
    /// `integrations.capture.<tool>.<group>.hint` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// Hard timeout. Default 300 (5 min), max 900 (15 min). Values
    /// outside `[30, 900]` are clamped server-side at load time.
    #[serde(default = "default_capture_timeout")]
    pub timeout_secs: u32,

    /// Ordered list of extraction rules. Each rule writes into one
    /// `target_field` declared in the parent group's `fields`. Order
    /// matters: a `PageContent` rule with `wait_ms` must come before
    /// any rule that depends on its DOM state.
    pub rules: Vec<CaptureRule>,
}

fn default_capture_timeout() -> u32 {
    300
}

/// One extraction action. All variants must reference a
/// `target_field` that exists in the parent group's `fields[].key`
/// (cross-validated at schema-load time, see Phase 1.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureRule {
    /// Pull a single cookie from the webview's cookie store.
    /// P1 MVP supports this variant only; P2 adds the rest.
    Cookie {
        /// Cookie domain (with or without leading dot).
        domain: String,
        /// Cookie name (exact match, case-sensitive).
        name: String,
        /// `Field.key` to write into.
        target_field: String,
        /// When `true` and the cookie is missing, the whole capture
        /// is marked `failed`; otherwise it's marked `partial`.
        #[serde(default = "default_true")]
        required: bool,
    },

    /// Pull multiple cookies, format each via `fmt`, then join with
    /// `sep`. Used by sites like TYC that expect a manually-joined
    /// `name1=v1; name2=v2` cookie header.
    CookieJoined {
        domain: String,
        names: Vec<String>,
        #[serde(default = "default_cookie_sep")]
        sep: String,
        #[serde(default = "default_cookie_fmt")]
        fmt: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// Read `localStorage[key]` via injected JS bridge.
    LocalStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// Read `sessionStorage[key]` via injected JS bridge.
    SessionStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// Read `document.querySelector(selector).textContent` (or the
    /// `attribute` value when set).
    PageContent {
        selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
        #[serde(default = "default_wait_ms")]
        wait_ms: u32,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// Read the named query parameter from the current page URL.
    UrlQuery {
        name: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },
}

fn default_cookie_sep() -> String {
    "; ".to_string()
}

fn default_cookie_fmt() -> String {
    "{name}={value}".to_string()
}

fn default_wait_ms() -> u32 {
    3000
}
```

2. 在 `IntegrationGroup` struct 末尾（`test` 字段之后）加 `capture` 字段：

```rust
    /// Optional connectivity-test recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestKind>,

    /// Optional auto-capture recipe (one-click "harvest from browser").
    /// When `None`, the frontend ⚡ button is hidden.
    /// See `docs/design/2026-05-21-credential-capture-engine.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureRecipe>,
}
```

3. 在 `#[cfg(test)] mod tests` 内加 round-trip 单测：

```rust
    #[test]
    fn capture_recipe_round_trip_cookie_rule() {
        let raw = r#"{
            "login_url": "https://aiqicha.baidu.com",
            "success_url_pattern": "aiqicha\\.baidu\\.com/(home|company)",
            "timeout_secs": 300,
            "rules": [
                { "type": "cookie", "domain": ".aiqicha.baidu.com",
                  "name": "BDUSS", "target_field": "cookies.aqc" }
            ]
        }"#;
        let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
        assert_eq!(r.login_url, "https://aiqicha.baidu.com");
        assert_eq!(r.timeout_secs, 300);
        assert_eq!(r.rules.len(), 1);
        match &r.rules[0] {
            CaptureRule::Cookie {
                domain,
                name,
                target_field,
                required,
            } => {
                assert_eq!(domain, ".aiqicha.baidu.com");
                assert_eq!(name, "BDUSS");
                assert_eq!(target_field, "cookies.aqc");
                assert!(*required);
            }
            other => panic!("expected Cookie, got {:?}", other),
        }
        let back: CaptureRecipe = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn capture_recipe_defaults() {
        let raw = r#"{
            "login_url": "https://fofa.info",
            "rules": [
                { "type": "page_content", "selector": "#api-key",
                  "target_field": "api_key" }
            ]
        }"#;
        let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
        assert_eq!(r.timeout_secs, 300);
        match &r.rules[0] {
            CaptureRule::PageContent { wait_ms, required, .. } => {
                assert_eq!(*wait_ms, 3000);
                assert!(*required);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn integration_group_capture_optional() {
        let raw = r#"{
            "id": "default",
            "name": "API Key",
            "fields": [
                { "key": "api_key", "label": "API Key", "type": "secret_text" }
            ]
        }"#;
        let g: IntegrationGroup = serde_json::from_str(raw).unwrap();
        assert!(g.capture.is_none(), "capture defaults to None when absent");
    }
```

**验证**：
```bash
cd backend && cargo nextest run -p golish-integrations -E 'test(schema::tests::capture_recipe)' --status-level fail
cd backend && cargo nextest run -p golish-integrations -E 'test(schema::tests::integration_group_capture_optional)' --status-level fail
cd backend && cargo test -p golish-integrations --lib schema::tests::storage_external_file_yaml schema::tests::field_type_is_secret
```
预期：3 个新测试 pass；老的 schema 测试 (storage_external_file_yaml / field_type_is_secret) 仍 pass，证明 `capture` 字段加进 `IntegrationGroup` 没破坏既有序列化。

**提交**：
```bash
git add backend/crates/golish-integrations/src/schema.rs
git commit -m "feat(integrations): add CaptureRecipe / CaptureRule schema types"
```

### T1.2 · 加 `CaptureState` / `CaptureSessionInfo` / `CaptureEvent` 运行时类型

**文件**：修改 `backend/crates/golish-integrations/src/types.rs`

**步骤**：

1. 用 `Read backend/crates/golish-integrations/src/types.rs` 看现有内容；在文件末尾追加：

```rust
// ────────────────────────────────────────────────────────────────────────
// Capture runtime types
// ────────────────────────────────────────────────────────────────────────

/// State of an in-flight capture session.
///
/// State machine: see `docs/design/2026-05-21-credential-capture-engine.md`
/// section 5.1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    /// Webview opened, user has not yet logged in.
    WaitingLogin,
    /// `success_url_pattern` matched; engine is currently navigating
    /// to `visit_url` (when set) before extraction.
    Navigating,
    /// Rules are executing.
    Extracting,
    /// All `required` rules succeeded → fields written to vault.
    Captured,
    /// Some optional rules failed but partial credentials written.
    Partial,
    /// At least one `required` rule failed.
    Failed,
    /// Hit `timeout_secs` without completing.
    Timeout,
    /// User clicked cancel or closed the window manually.
    Cancelled,
}

impl CaptureState {
    /// Terminal states never transition further. The engine should
    /// drop the webview + data_directory on transition into any of
    /// these.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Captured
                | Self::Partial
                | Self::Failed
                | Self::Timeout
                | Self::Cancelled
        )
    }
}

/// Snapshot of a session, returned by `integrations_capture_start` /
/// `_status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSessionInfo {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub state: CaptureState,
    pub login_url: String,
    /// `target_field` values declared in the recipe rules, in order.
    /// UI uses this to render "we will try to harvest: X, Y, Z".
    pub expected_fields: Vec<String>,
    /// Fields actually written (subset of `expected_fields`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captured_fields: Vec<String>,
    /// Per-rule failure detail. `rule_index` is 0-based.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_rules: Vec<FailedRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix milliseconds. `None` for already-terminal sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Unix milliseconds when state last transitioned.
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedRule {
    pub rule_index: usize,
    pub reason: String,
}

/// Event payload emitted on the `"integration-capture"` channel.
/// Matches `CaptureSessionInfo` minus `expires_at` (already-known to
/// the frontend from `_start`'s response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureEventPayload {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub state: CaptureState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captured_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_rules: Vec<FailedRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}
```

2. 在文件末尾的 `#[cfg(test)] mod tests`（若不存在则创建）加测试：

```rust
#[cfg(test)]
mod capture_type_tests {
    use super::*;

    #[test]
    fn capture_state_is_terminal() {
        assert!(!CaptureState::WaitingLogin.is_terminal());
        assert!(!CaptureState::Navigating.is_terminal());
        assert!(!CaptureState::Extracting.is_terminal());
        assert!(CaptureState::Captured.is_terminal());
        assert!(CaptureState::Partial.is_terminal());
        assert!(CaptureState::Failed.is_terminal());
        assert!(CaptureState::Timeout.is_terminal());
        assert!(CaptureState::Cancelled.is_terminal());
    }

    #[test]
    fn capture_state_round_trip() {
        let s = CaptureState::WaitingLogin;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"waiting_login\"");
        let back: CaptureState = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
```

**验证**：
```bash
cd backend && cargo nextest run -p golish-integrations -E 'test(capture_type_tests)' --status-level fail
```
预期：2 个新测试 pass。

**提交**：
```bash
git add backend/crates/golish-integrations/src/types.rs
git commit -m "feat(integrations): add CaptureState / CaptureSessionInfo runtime types"
```

### T1.3 · 加 capture-specific error variants

**文件**：修改 `backend/crates/golish-integrations/src/error.rs`

**步骤**：

1. 用 `Read backend/crates/golish-integrations/src/error.rs` 查看现有 `IntegrationError` enum 定义。
2. 在 enum 末尾添加 8 个新 variant（保持现有 variant 不动）：

```rust
    // Capture engine errors — keep the prefix `Capture` for grep-ability.
    #[error("[CAPTURE_NO_RECIPE] integration group has no capture recipe declared")]
    CaptureNoRecipe,

    #[error("[CAPTURE_ALREADY_RUNNING] session already in-flight for {tool_id}/{group_id}")]
    CaptureAlreadyRunning { tool_id: String, group_id: String },

    #[error("[CAPTURE_SESSION_NOT_FOUND] session_id={0} not found or already expired")]
    CaptureSessionNotFound(String),

    #[error("[WEBVIEW_CREATE_FAILED] failed to create capture webview: {0}")]
    WebviewCreateFailed(String),

    #[error("[CAPTURE_TIMEOUT] session expired after {timeout_secs}s without completion")]
    CaptureTimeout { timeout_secs: u32 },

    #[error("[CAPTURE_RULE_FAILED] rule #{rule_index} ({rule_kind}) failed: {reason}")]
    CaptureRuleFailed {
        rule_index: usize,
        rule_kind: &'static str,
        reason: String,
    },

    #[error("[CAPTURE_INVALID_URL] login_url is not a valid http(s) URL: {0}")]
    CaptureInvalidUrl(String),

    #[error("[CAPTURE_INVALID_TARGET_FIELD] rule #{rule_index} references unknown field {field}")]
    CaptureInvalidTargetField {
        rule_index: usize,
        field: String,
    },
```

3. 加单测验证 error rendering：

```rust
#[cfg(test)]
mod capture_error_tests {
    use super::*;

    #[test]
    fn capture_no_recipe_message() {
        let e = IntegrationError::CaptureNoRecipe;
        assert!(e.to_string().contains("CAPTURE_NO_RECIPE"));
    }

    #[test]
    fn capture_already_running_message() {
        let e = IntegrationError::CaptureAlreadyRunning {
            tool_id: "enscan-go".into(),
            group_id: "aqc".into(),
        };
        let s = e.to_string();
        assert!(s.contains("CAPTURE_ALREADY_RUNNING"));
        assert!(s.contains("enscan-go/aqc"));
    }
}
```

**验证**：
```bash
cd backend && cargo nextest run -p golish-integrations -E 'test(capture_error_tests)' --status-level fail
```

**提交**：
```bash
git add backend/crates/golish-integrations/src/error.rs
git commit -m "feat(integrations): add capture-specific error variants"
```

### T1.4 · schema 加载时的 capture 字段交叉校验

**文件**：修改 `backend/crates/golish-integrations/src/resolver.rs`

**步骤**：

1. 用 `Read backend/crates/golish-integrations/src/resolver.rs` 找到 `SchemaResolver` impl（具体方法名以现状为准；该 crate 已实现 schema resolution，本步是补强）。
2. 找到 schema 加载后的归一化/校验点（通常是 `load_from_path` / `load_all` 之类的方法）。在归一化函数末尾加 `validate_capture` 调用：

```rust
/// Validate a capture recipe makes sense in the context of its
/// parent group: every rule's `target_field` exists in `fields[]`,
/// `login_url` parses as http/https, `timeout_secs` is in `[30,900]`.
fn validate_capture(group: &IntegrationGroup) -> IntegrationResult<()> {
    let Some(recipe) = group.capture.as_ref() else {
        return Ok(());
    };
    // 1. login_url scheme whitelist
    let parsed = url::Url::parse(&recipe.login_url)
        .map_err(|e| IntegrationError::CaptureInvalidUrl(format!("{}: {e}", recipe.login_url)))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IntegrationError::CaptureInvalidUrl(format!(
            "{} (scheme must be http or https, got {})",
            recipe.login_url,
            parsed.scheme()
        )));
    }
    // 2. timeout clamp (warn-don't-fail: we just clamp)
    // We choose NOT to mutate here — clamping happens in the engine.
    // The schema can declare any value; CaptureEngine::start clamps.
    let _ = recipe.timeout_secs;
    // 3. target_field cross-reference
    let known: std::collections::HashSet<&str> = group.fields.iter().map(|f| f.key.as_str()).collect();
    for (idx, rule) in recipe.rules.iter().enumerate() {
        let tf = match rule {
            CaptureRule::Cookie { target_field, .. }
            | CaptureRule::CookieJoined { target_field, .. }
            | CaptureRule::LocalStorage { target_field, .. }
            | CaptureRule::SessionStorage { target_field, .. }
            | CaptureRule::PageContent { target_field, .. }
            | CaptureRule::UrlQuery { target_field, .. } => target_field.as_str(),
        };
        if !known.contains(tf) {
            return Err(IntegrationError::CaptureInvalidTargetField {
                rule_index: idx,
                field: tf.to_string(),
            });
        }
    }
    Ok(())
}
```

3. 在 schema resolution loop 内部（每解析完一个 `IntegrationGroup`）插入：

```rust
            for group in schema.groups.iter() {
                validate_capture(group)?;
            }
```

如果具体函数签名是 `fn load(&self, path: &Path) -> IntegrationResult<IntegrationSchema>`，则在 return Ok(schema) 之前调用一次：

```rust
            for group in schema.groups.iter() {
                validate_capture(group)?;
            }
            Ok(schema)
```

4. 在文件末尾的 `#[cfg(test)] mod tests` 加：

```rust
    #[test]
    fn validate_capture_rejects_unknown_target_field() {
        let group = IntegrationGroup {
            id: "default".into(),
            name: "Test".into(),
            description: None,
            icon: None,
            help_url: None,
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
            test: None,
            capture: Some(CaptureRecipe {
                login_url: "https://example.com".into(),
                success_url_pattern: None,
                visit_url: None,
                instructions: None,
                timeout_secs: 60,
                rules: vec![CaptureRule::Cookie {
                    domain: ".example.com".into(),
                    name: "X".into(),
                    target_field: "missing_field".into(),
                    required: true,
                }],
            }),
        };
        let r = validate_capture(&group);
        assert!(matches!(
            r,
            Err(IntegrationError::CaptureInvalidTargetField { .. })
        ));
    }

    #[test]
    fn validate_capture_rejects_non_https_url() {
        let group = IntegrationGroup {
            id: "default".into(),
            name: "Test".into(),
            description: None,
            icon: None,
            help_url: None,
            fields: vec![Field {
                key: "x".into(),
                label: "X".into(),
                field_type: FieldType::SecretText,
                placeholder: None,
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            }],
            test: None,
            capture: Some(CaptureRecipe {
                login_url: "javascript:alert(1)".into(),
                success_url_pattern: None,
                visit_url: None,
                instructions: None,
                timeout_secs: 60,
                rules: vec![],
            }),
        };
        let r = validate_capture(&group);
        assert!(matches!(r, Err(IntegrationError::CaptureInvalidUrl(_))));
    }

    #[test]
    fn validate_capture_accepts_valid_recipe() {
        let group = IntegrationGroup {
            id: "default".into(),
            name: "Test".into(),
            description: None,
            icon: None,
            help_url: None,
            fields: vec![Field {
                key: "cookies.aqc".into(),
                label: "Cookie".into(),
                field_type: FieldType::SecretTextarea,
                placeholder: None,
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            }],
            test: None,
            capture: Some(CaptureRecipe {
                login_url: "https://aiqicha.baidu.com".into(),
                success_url_pattern: Some(r"aiqicha\.baidu\.com".into()),
                visit_url: None,
                instructions: None,
                timeout_secs: 300,
                rules: vec![CaptureRule::Cookie {
                    domain: ".aiqicha.baidu.com".into(),
                    name: "BDUSS".into(),
                    target_field: "cookies.aqc".into(),
                    required: true,
                }],
            }),
        };
        assert!(validate_capture(&group).is_ok());
    }
```

5. 在 `Cargo.toml` 添 `url = { workspace = true }` （如果还没添）：

```bash
cd backend && rg "^url = " crates/golish-integrations/Cargo.toml
```
如无，向 `[dependencies]` 段追加 `url = { workspace = true }`，并确保 `backend/Cargo.toml` 的 workspace.dependencies 已声明 `url`（通常已有）。

**验证**：
```bash
cd backend && cargo nextest run -p golish-integrations -E 'test(validate_capture)' --status-level fail
cd backend && cargo check -p golish-integrations
```

**提交**：
```bash
git add backend/crates/golish-integrations/src/resolver.rs backend/crates/golish-integrations/Cargo.toml
git commit -m "feat(integrations): cross-validate capture recipe at schema load"
```

### T1.5 · ts-rs 同步前端类型

**文件**：修改 `frontend/lib/api/integrations.ts`

**步骤**：

1. 在 `IntegrationGroup` interface 添加 `capture?: CaptureRecipe;` 字段：

```ts
export interface IntegrationGroup {
  id: string;
  name: string;
  description?: string;
  icon?: string;
  help_url?: string;
  fields: Field[];
  test?: TestKind;
  /** Optional auto-capture recipe; absent ⇒ no ⚡ button. */
  capture?: CaptureRecipe;
}
```

2. 在文件末尾（IPC wrapper 段之前）追加 capture 相关类型：

```ts
// ────────────────────────────────────────────────────────────────────────
// Capture types (mirrors golish-integrations/src/schema.rs CaptureRecipe + types.rs CaptureState)
// ────────────────────────────────────────────────────────────────────────

export interface CaptureRecipe {
  login_url: string;
  success_url_pattern?: string;
  visit_url?: string;
  instructions?: string;
  timeout_secs: number;
  rules: CaptureRule[];
}

export type CaptureRule =
  | {
      type: "cookie";
      domain: string;
      name: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "cookie_joined";
      domain: string;
      names: string[];
      sep?: string;
      fmt?: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "local_storage";
      key: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "session_storage";
      key: string;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "page_content";
      selector: string;
      attribute?: string;
      wait_ms?: number;
      target_field: string;
      required?: boolean;
    }
  | {
      type: "url_query";
      name: string;
      target_field: string;
      required?: boolean;
    };

export type CaptureState =
  | "waiting_login"
  | "navigating"
  | "extracting"
  | "captured"
  | "partial"
  | "failed"
  | "timeout"
  | "cancelled";

export interface FailedRule {
  rule_index: number;
  reason: string;
}

export interface CaptureSessionInfo {
  session_id: string;
  tool_id: string;
  group_id: string;
  state: CaptureState;
  login_url: string;
  expected_fields: string[];
  captured_fields?: string[];
  failed_rules?: FailedRule[];
  error_message?: string;
  expires_at?: number;
  updated_at: number;
}

export interface CaptureEventPayload {
  session_id: string;
  tool_id: string;
  group_id: string;
  state: CaptureState;
  captured_fields?: string[];
  failed_rules?: FailedRule[];
  error_message?: string;
}
```

**验证**：
```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/lib/api/integrations.ts
```
预期：两个命令都 exit 0。

**提交**：
```bash
git add frontend/lib/api/integrations.ts
git commit -m "feat(integrations): mirror CaptureRecipe types to frontend"
```

### T1.6 · 整体 Phase 1 编译 + 单测全跑

**验证**：
```bash
cd backend && cargo nextest run -p golish-integrations --status-level fail
cd backend && cargo check -p golish-integrations -p golish
```
预期：所有 schema / types / error / resolver 相关测试全绿；`golish` workspace 增量 check 通过（因为我们只加了字段、没改既有签名）。

**Review Checkpoint**：
- 用户审 `CaptureRule` enum 是否覆盖所有 P2 场景（CookieJoined / LocalStorage / SessionStorage / PageContent / UrlQuery 5 种）
- 用户拍板 `CaptureState` 8 个状态是否齐全
- 用户拍板 schema 加 `capture` 字段对老的 JSON / 测试**零回退**（所有现有 schema 反序列化默认 `capture: None`）

---

## Phase 2 · CaptureEngine 核心实现（4-5 小时）

**目标**：`CaptureEngine` 在 `golish/src/tools/integrations/capture/` 落地；P1 MVP 仅实现 Cookie rule；webview 创建 + 监听 + 抠 cookie + 写 vault + cleanup 全链路打通；用 mock 测覆盖 state machine 5 个核心 transition。

**Tasks**：

### T2.1 · 创建 capture 模块骨架

**文件**：
- 新建 `backend/crates/golish/src/tools/integrations/capture/mod.rs`
- 新建 `backend/crates/golish/src/tools/integrations/capture/engine.rs`
- 新建 `backend/crates/golish/src/tools/integrations/capture/session.rs`
- 新建 `backend/crates/golish/src/tools/integrations/capture/data_dir.rs`
- 修改 `backend/crates/golish/src/tools/integrations/mod.rs` 加 `pub mod capture;`

**步骤**：

1. 在 `mod.rs` 加：

```rust
pub mod capture;
```

2. 新建 `capture/mod.rs`：

```rust
//! Credential Capture Engine.
//!
//! See `docs/design/2026-05-21-credential-capture-engine.md`.

mod data_dir;
mod engine;
mod session;

pub use engine::CaptureEngine;
pub use session::{CaptureSession, CaptureSessionHandle};
```

3. 新建 `capture/data_dir.rs`：

```rust
//! Per-session data directory management.
//!
//! Each capture session gets an isolated WebKit/WebView data dir so
//! cookies, localStorage, IndexedDB etc. never leak between sessions
//! or into the main Golish window.

use std::path::{Path, PathBuf};

use golish_integrations::error::{IntegrationError, IntegrationResult};

/// Returns the parent dir under which all per-session capture data
/// lives. Created on first call.
///
/// macOS:   `~/Library/Application Support/com.golish.platform/capture-sessions/`
/// Linux:   `~/.local/share/com.golish.platform/capture-sessions/`
/// Windows: `%APPDATA%/com.golish.platform/capture-sessions/`
pub fn capture_root() -> IntegrationResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| IntegrationError::WebviewCreateFailed(
            "no platform data_dir".into(),
        ))?
        .join("com.golish.platform")
        .join("capture-sessions");
    std::fs::create_dir_all(&base).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir {}: {e}", base.display()))
    })?;
    Ok(base)
}

/// Returns the per-session data dir, creating it if missing.
pub fn session_dir(session_id: &str) -> IntegrationResult<PathBuf> {
    let dir = capture_root()?.join(session_id);
    std::fs::create_dir_all(&dir).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir {}: {e}", dir.display()))
    })?;
    Ok(dir)
}

/// Best-effort recursive delete. Logs but never panics on failure
/// (cleanup runs on terminal-state transition; we don't want a
/// transient FS error to mask the real outcome).
pub fn cleanup_session_dir(session_id: &str) {
    let Ok(root) = capture_root() else {
        return;
    };
    let dir = root.join(session_id);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(
                session_id = %session_id,
                dir = %dir.display(),
                error = %e,
                "capture: failed to cleanup session data dir"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_missing_dir_is_noop() {
        cleanup_session_dir("nonexistent-session-uuid-aaaa");
    }

    #[test]
    fn session_dir_creates_subdir(){
        let sid = format!("test-{}", uuid::Uuid::new_v4());
        let dir = session_dir(&sid).expect("create");
        assert!(dir.exists());
        cleanup_session_dir(&sid);
        assert!(!dir.exists());
    }
}
```

4. 新建 `capture/session.rs`：

```rust
//! In-memory representation of a capture session.

use std::sync::Arc;
use std::time::Instant;

use golish_integrations::schema::CaptureRecipe;
use golish_integrations::types::{CaptureSessionInfo, CaptureState, FailedRule};
use tokio::sync::RwLock;

/// Mutable inner state held inside an `Arc<RwLock<_>>` so the engine
/// and the webview event handlers (which run on the Tauri main loop)
/// can both poke at it without contending on a global registry.
#[derive(Debug, Clone)]
pub struct CaptureSession {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub recipe: CaptureRecipe,
    pub state: CaptureState,
    pub captured_fields: Vec<String>,
    pub failed_rules: Vec<FailedRule>,
    pub error_message: Option<String>,
    pub started_at: Instant,
    /// `started_at` as Unix milliseconds, for serialization to UI.
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    /// Effective TTL in seconds (clamped from recipe.timeout_secs).
    pub timeout_secs: u32,
}

impl CaptureSession {
    pub fn new(
        session_id: String,
        tool_id: String,
        group_id: String,
        recipe: CaptureRecipe,
    ) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let timeout_secs = recipe.timeout_secs.clamp(30, 900);
        Self {
            session_id,
            tool_id,
            group_id,
            recipe,
            state: CaptureState::WaitingLogin,
            captured_fields: Vec::new(),
            failed_rules: Vec::new(),
            error_message: None,
            started_at: Instant::now(),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
            timeout_secs,
        }
    }

    pub fn transition(&mut self, next: CaptureState) {
        self.state = next;
        self.updated_at_ms = chrono::Utc::now().timestamp_millis();
    }

    pub fn info(&self) -> CaptureSessionInfo {
        let expires_at = if self.state.is_terminal() {
            None
        } else {
            Some(self.started_at_ms + (self.timeout_secs as i64) * 1000)
        };
        CaptureSessionInfo {
            session_id: self.session_id.clone(),
            tool_id: self.tool_id.clone(),
            group_id: self.group_id.clone(),
            state: self.state,
            login_url: self.recipe.login_url.clone(),
            expected_fields: self
                .recipe
                .rules
                .iter()
                .map(|r| match r {
                    golish_integrations::schema::CaptureRule::Cookie { target_field, .. }
                    | golish_integrations::schema::CaptureRule::CookieJoined { target_field, .. }
                    | golish_integrations::schema::CaptureRule::LocalStorage { target_field, .. }
                    | golish_integrations::schema::CaptureRule::SessionStorage { target_field, .. }
                    | golish_integrations::schema::CaptureRule::PageContent { target_field, .. }
                    | golish_integrations::schema::CaptureRule::UrlQuery { target_field, .. } => {
                        target_field.clone()
                    }
                })
                .collect(),
            captured_fields: self.captured_fields.clone(),
            failed_rules: self.failed_rules.clone(),
            error_message: self.error_message.clone(),
            expires_at,
            updated_at: self.updated_at_ms,
        }
    }
}

/// Shared handle: an `Arc<RwLock<CaptureSession>>` plus the immutable
/// session_id (cached so we don't lock to read it).
#[derive(Clone)]
pub struct CaptureSessionHandle {
    pub session_id: String,
    pub inner: Arc<RwLock<CaptureSession>>,
}

impl CaptureSessionHandle {
    pub fn new(session: CaptureSession) -> Self {
        Self {
            session_id: session.session_id.clone(),
            inner: Arc::new(RwLock::new(session)),
        }
    }
}
```

**验证**：
```bash
cd backend && cargo check -p golish 2>&1 | rg -i 'capture|error'
```
预期：编译通过；如果 `golish-integrations` re-export 不全可能需要在 `lib.rs` 加 `pub use schema::CaptureRecipe;` 之类，根据具体编译错修。

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/mod.rs backend/crates/golish/src/tools/integrations/capture/
git commit -m "feat(capture): scaffold capture module with session + data_dir"
```

### T2.2 · CaptureEngine 主体（不带 webview 创建，先写状态机）

**文件**：新建 `backend/crates/golish/src/tools/integrations/capture/engine.rs`

**步骤**：

1. 写引擎主体（先把 state machine 和 session registry 实现，cookie 抓取下一个 task 再加）：

```rust
//! CaptureEngine — owns the in-flight session registry and drives
//! the state machine.

use std::collections::HashMap;
use std::sync::Arc;

use golish_integrations::error::{IntegrationError, IntegrationResult};
use golish_integrations::schema::{CaptureRecipe, CaptureRule};
use golish_integrations::types::{CaptureState, FailedRule};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::data_dir;
use super::session::{CaptureSession, CaptureSessionHandle};

/// The engine is a Tauri-managed singleton (`tauri::State<CaptureEngine>`).
///
/// All session lookups go through `sessions: RwLock<HashMap>`. Only
/// the registry itself is locked at the top level; per-session mutation
/// goes through the per-handle `RwLock<CaptureSession>`. This keeps
/// concurrent reads on different sessions parallel.
pub struct CaptureEngine {
    sessions: RwLock<HashMap<String, CaptureSessionHandle>>,
}

impl Default for CaptureEngine {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }
}

impl CaptureEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new session. Returns the handle (caller is
    /// responsible for actually creating the webview).
    ///
    /// Rejects with `CaptureAlreadyRunning` if any in-flight session
    /// exists for the same `(tool_id, group_id)` pair.
    pub async fn register(
        &self,
        tool_id: String,
        group_id: String,
        recipe: CaptureRecipe,
    ) -> IntegrationResult<CaptureSessionHandle> {
        {
            let map = self.sessions.read().await;
            for h in map.values() {
                let s = h.inner.read().await;
                if !s.state.is_terminal()
                    && s.tool_id == tool_id
                    && s.group_id == group_id
                {
                    return Err(IntegrationError::CaptureAlreadyRunning {
                        tool_id,
                        group_id,
                    });
                }
            }
        }
        let sid = Uuid::new_v4().to_string();
        let session = CaptureSession::new(sid.clone(), tool_id, group_id, recipe);
        let handle = CaptureSessionHandle::new(session);
        self.sessions
            .write()
            .await
            .insert(sid.clone(), handle.clone());
        Ok(handle)
    }

    pub async fn get(&self, session_id: &str) -> IntegrationResult<CaptureSessionHandle> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| IntegrationError::CaptureSessionNotFound(session_id.to_string()))
    }

    /// Transitions a session into a terminal state (or any state) and
    /// runs cleanup if terminal.
    pub async fn transition(
        &self,
        session_id: &str,
        next: CaptureState,
        error: Option<String>,
    ) -> IntegrationResult<()> {
        let handle = self.get(session_id).await?;
        let mut s = handle.inner.write().await;
        if s.state.is_terminal() {
            tracing::debug!(
                session_id = %session_id,
                current = ?s.state,
                next = ?next,
                "capture: ignoring transition (already terminal)"
            );
            return Ok(());
        }
        s.transition(next);
        if let Some(e) = error {
            s.error_message = Some(e);
        }
        if next.is_terminal() {
            data_dir::cleanup_session_dir(session_id);
        }
        Ok(())
    }

    /// Marks a session cancelled and runs cleanup.
    pub async fn cancel(&self, session_id: &str) -> IntegrationResult<()> {
        self.transition(session_id, CaptureState::Cancelled, None).await
    }

    /// Drops terminal sessions older than 1 hour from the registry.
    /// Called periodically by a background task (Phase 2.6).
    pub async fn gc(&self) {
        let cutoff = chrono::Utc::now().timestamp_millis() - 3600 * 1000;
        let to_remove: Vec<String> = {
            let map = self.sessions.read().await;
            let mut v = Vec::new();
            for (sid, h) in map.iter() {
                let s = h.inner.read().await;
                if s.state.is_terminal() && s.updated_at_ms < cutoff {
                    v.push(sid.clone());
                }
            }
            v
        };
        if !to_remove.is_empty() {
            let mut map = self.sessions.write().await;
            for sid in to_remove {
                map.remove(&sid);
                tracing::debug!(session_id = %sid, "capture: gc removed terminal session");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_integrations::schema::{CaptureRecipe, CaptureRule};

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
        assert!(matches!(err, IntegrationError::CaptureAlreadyRunning { .. }));
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
        eng.transition(&h.session_id, CaptureState::Captured, None).await.unwrap();
        // Second transition should be ignored, not error.
        eng.transition(&h.session_id, CaptureState::Failed, Some("late".into()))
            .await
            .unwrap();
        let s = h.inner.read().await;
        assert_eq!(s.state, CaptureState::Captured);
        assert!(s.error_message.is_none());
    }

    #[tokio::test]
    async fn get_unknown_returns_not_found() {
        let eng = CaptureEngine::new();
        let err = eng.get("nope").await.unwrap_err();
        assert!(matches!(err, IntegrationError::CaptureSessionNotFound(_)));
    }
}
```

**验证**：
```bash
cd backend && cargo nextest run -p golish -E 'test(tools::integrations::capture::engine::tests)' --status-level fail
```
预期：5 个测试 pass。

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/capture/engine.rs
git commit -m "feat(capture): engine state machine + session registry"
```

### T2.3 · webview 创建 + navigation 监听

**文件**：在 `engine.rs` 加 `start_webview` 方法

**步骤**：

1. 在 `CaptureEngine` impl 内加：

```rust
    /// Creates the Tauri webview window for an already-registered
    /// session. Wires `on_navigation` to invoke `try_extract` when
    /// `success_url_pattern` matches.
    ///
    /// **Important**: `app` is the global `AppHandle`. We clone it
    /// into the navigation callback because the callback outlives
    /// this function.
    pub fn start_webview(
        &self,
        app: &tauri::AppHandle,
        handle: &CaptureSessionHandle,
    ) -> IntegrationResult<()> {
        use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

        // 1. Lock just enough to read what we need.
        let (sid, login_url, recipe) = {
            let s = futures::executor::block_on(handle.inner.read());
            (
                s.session_id.clone(),
                s.recipe.login_url.clone(),
                s.recipe.clone(),
            )
        };

        // 2. Per-session data dir.
        let dir = data_dir::session_dir(&sid)?;

        // 3. Parse the URL (already validated at schema-load time,
        //    but Tauri requires a `Url` here so re-parse defensively).
        let url = login_url
            .parse::<url::Url>()
            .map_err(|e| IntegrationError::CaptureInvalidUrl(format!("{login_url}: {e}")))?;

        // 4. Build the window. label = `capture-<sid>` is unique.
        let label = format!("capture-{}", sid);
        let app_for_cb = app.clone();
        let sid_for_cb = sid.clone();
        let recipe_for_cb = recipe.clone();

        WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
            .title(format!("Golish · 凭据抓取: {}", url.host_str().unwrap_or("?")))
            .inner_size(900.0, 700.0)
            .center()
            .focused(true)
            .visible(true)
            .data_directory(dir.clone())
            .on_navigation(move |new_url| {
                let app = app_for_cb.clone();
                let sid = sid_for_cb.clone();
                let recipe = recipe_for_cb.clone();
                let url_str = new_url.to_string();
                tauri::async_runtime::spawn(async move {
                    on_navigation_event(&app, &sid, &recipe, &url_str).await;
                });
                true
            })
            .build()
            .map_err(|e| IntegrationError::WebviewCreateFailed(e.to_string()))?;

        Ok(())
    }
```

2. 在文件末尾（在 `#[cfg(test)]` 之前）加 `on_navigation_event` 自由函数：

```rust
/// Called for every navigation event. Checks the URL against
/// `success_url_pattern` and triggers extraction when matched.
async fn on_navigation_event(
    app: &tauri::AppHandle,
    session_id: &str,
    recipe: &CaptureRecipe,
    new_url: &str,
) {
    let Some(pat) = recipe.success_url_pattern.as_ref() else {
        return;
    };
    let re = match regex::Regex::new(pat) {
        Ok(re) => re,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                pattern = %pat,
                error = %e,
                "capture: invalid success_url_pattern regex (schema validation gap)"
            );
            return;
        }
    };
    if !re.is_match(new_url) {
        return;
    }
    tracing::info!(
        session_id = %session_id,
        url = %new_url,
        "capture: success_url_pattern matched, extracting"
    );
    let eng = app.state::<CaptureEngine>();
    if let Err(e) = eng.try_extract(app, session_id).await {
        tracing::error!(
            session_id = %session_id,
            error = %e,
            "capture: extraction failed"
        );
    }
}
```

3. 在 `Cargo.toml` 加 `futures = { workspace = true }`（用 `futures::executor::block_on` 在同步 Tauri builder callback 中读 RwLock）：

```bash
cd backend && rg "^futures = " crates/golish/Cargo.toml
```
若无则向 `[dependencies]` 追加 `futures = { workspace = true }`。

**验证**：
```bash
cd backend && cargo check -p golish 2>&1 | rg -i 'error\|warning' | head -20
```
预期：编译通过；可能有 `try_extract` 未定义 warning（下一个 task 实现）。如有 missing-method 错误，先把 `try_extract` 占位 stub 加上：

```rust
    pub async fn try_extract(
        &self,
        _app: &tauri::AppHandle,
        _session_id: &str,
    ) -> IntegrationResult<()> {
        // Implemented in T2.4
        Ok(())
    }
```

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/capture/engine.rs backend/crates/golish/Cargo.toml
git commit -m "feat(capture): create capture webview + navigation handler"
```

### T2.4 · 实现 `try_extract` — Cookie rule + vault 写入

**文件**：修改 `engine.rs`

**步骤**：

1. 删除 T2.3 的 stub，替换为完整 `try_extract` 实现：

```rust
    /// Performs the full extraction sequence for a session: visits
    /// optional `visit_url`, then runs every rule, then writes
    /// captured values to vault via the existing
    /// `IntegrationsState::set` path.
    pub async fn try_extract(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
    ) -> IntegrationResult<()> {
        use tauri::Manager;

        let handle = self.get(session_id).await?;
        // 1. Idempotency guard: if already extracting/terminal, skip.
        {
            let s = handle.inner.read().await;
            if s.state.is_terminal() || s.state == CaptureState::Extracting {
                return Ok(());
            }
        }
        self.transition(session_id, CaptureState::Extracting, None).await?;

        // 2. Resolve webview window.
        let label = format!("capture-{}", session_id);
        let win = app
            .get_webview_window(&label)
            .ok_or_else(|| IntegrationError::WebviewCreateFailed("webview vanished".into()))?;

        // 3. Run rules.
        let (rules, tool_id, group_id) = {
            let s = handle.inner.read().await;
            (s.recipe.rules.clone(), s.tool_id.clone(), s.group_id.clone())
        };

        let mut captured: HashMap<String, String> = HashMap::new();
        let mut failed: Vec<FailedRule> = Vec::new();

        for (idx, rule) in rules.iter().enumerate() {
            match extract_one(&win, rule).await {
                Ok((target_field, value)) => {
                    captured.insert(target_field, value);
                }
                Err(reason) => {
                    let is_required = match rule {
                        CaptureRule::Cookie { required, .. }
                        | CaptureRule::CookieJoined { required, .. }
                        | CaptureRule::LocalStorage { required, .. }
                        | CaptureRule::SessionStorage { required, .. }
                        | CaptureRule::PageContent { required, .. }
                        | CaptureRule::UrlQuery { required, .. } => *required,
                    };
                    failed.push(FailedRule {
                        rule_index: idx,
                        reason: reason.clone(),
                    });
                    if is_required {
                        // Required rule failed → mark failed, abort.
                        let mut s = handle.inner.write().await;
                        s.failed_rules = failed;
                        s.error_message = Some(format!(
                            "[CAPTURE_RULE_FAILED] required rule #{idx}: {reason}"
                        ));
                        s.transition(CaptureState::Failed);
                        drop(s);
                        data_dir::cleanup_session_dir(session_id);
                        let _ = win.close();
                        return Ok(());
                    }
                }
            }
        }

        // 4. Write captured values via IntegrationsState.
        if !captured.is_empty() {
            let state = app.state::<crate::tools::integrations::state::IntegrationsState>();
            if let Err(e) = state.set(&tool_id, &group_id, captured.clone()).await {
                let mut s = handle.inner.write().await;
                s.error_message = Some(format!("[STORAGE_WRITE_FAILED] {e}"));
                s.transition(CaptureState::Failed);
                drop(s);
                data_dir::cleanup_session_dir(session_id);
                let _ = win.close();
                return Err(IntegrationError::WebviewCreateFailed(format!(
                    "storage_write_failed: {e}"
                )));
            }
        }

        // 5. Final state.
        let mut s = handle.inner.write().await;
        s.captured_fields = captured.keys().cloned().collect();
        s.failed_rules = failed;
        s.transition(if s.failed_rules.is_empty() {
            CaptureState::Captured
        } else {
            CaptureState::Partial
        });
        drop(s);
        data_dir::cleanup_session_dir(session_id);
        let _ = win.close();
        Ok(())
    }
```

2. 加 `extract_one` 自由函数（P1 仅实现 Cookie）：

```rust
/// Runs one extraction rule against the live webview. Returns
/// `(target_field, value)` on success, `Err(reason)` otherwise.
async fn extract_one(
    win: &tauri::WebviewWindow,
    rule: &CaptureRule,
) -> Result<(String, String), String> {
    match rule {
        CaptureRule::Cookie {
            domain,
            name,
            target_field,
            ..
        } => {
            // Tauri's cookies_for_url expects an absolute URL whose
            // host matches the cookie domain. Strip leading dot and
            // synthesize an https:// URL with that host.
            let host = domain.trim_start_matches('.');
            let url_str = format!("https://{}/", host);
            let url = url_str
                .parse::<url::Url>()
                .map_err(|e| format!("invalid synthesized cookie URL {url_str}: {e}"))?;
            let cookies = win
                .cookies_for_url(url)
                .await
                .map_err(|e| format!("cookies_for_url failed: {e}"))?;
            let value = cookies
                .into_iter()
                .find(|c| c.name() == name.as_str())
                .ok_or_else(|| format!("cookie '{name}' not found in domain '{domain}'"))?
                .value()
                .to_string();
            Ok((target_field.clone(), value))
        }
        // P2 rules: explicitly bail with "not implemented" until later.
        CaptureRule::CookieJoined { .. }
        | CaptureRule::LocalStorage { .. }
        | CaptureRule::SessionStorage { .. }
        | CaptureRule::PageContent { .. }
        | CaptureRule::UrlQuery { .. } => {
            Err("rule type not yet implemented in P1 MVP".to_string())
        }
    }
}
```

**验证**：
```bash
cd backend && cargo check -p golish
```
预期：编译通过。

> 注：cookie extraction 的真实端到端测试在 Phase 5 手动跑（需要弹出真实窗口）。

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/capture/engine.rs
git commit -m "feat(capture): extract cookies + write to vault on success_url match"
```

### T2.5 · TTL 后台任务 + IntegrationsState::set 暴露

**文件**：
- 修改 `backend/crates/golish/src/tools/integrations/capture/engine.rs`
- 修改 `backend/crates/golish/src/tools/integrations/state.rs`

**步骤**：

1. 检查 `state.rs` 是否已暴露 `pub async fn set(&self, tool_id: &str, group_id: &str, fields: HashMap<String, String>) -> IntegrationResult<()>`。如果只在 commands.rs 里有，把核心逻辑抽到 `state.rs` 作为 `IntegrationsState` impl 的 pub 方法（commands.rs 复用之）。这是必须，因为 capture engine 不能调 IPC command，必须直接调 service 层。

2. 在 `engine.rs` 加 TTL watcher：

```rust
    /// Spawn a background task that periodically scans for sessions
    /// past their TTL and transitions them to Timeout.
    ///
    /// Called once from `app.setup` (Phase 2.6 wiring).
    pub fn spawn_ttl_watcher(self: Arc<Self>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let now_ms = chrono::Utc::now().timestamp_millis();
                let to_timeout: Vec<String> = {
                    let map = self.sessions.read().await;
                    let mut v = Vec::new();
                    for (sid, h) in map.iter() {
                        let s = h.inner.read().await;
                        if s.state.is_terminal() {
                            continue;
                        }
                        let expires = s.started_at_ms + (s.timeout_secs as i64) * 1000;
                        if now_ms > expires {
                            v.push(sid.clone());
                        }
                    }
                    v
                };
                for sid in to_timeout {
                    let _ = self
                        .transition(&sid, CaptureState::Timeout, Some(
                            "capture session expired".into(),
                        ))
                        .await;
                }
                self.gc().await;
            }
        });
    }
```

3. 同时在 `engine.rs` 把 `CaptureEngine` 改成 `Arc`-friendly:

```rust
// In Tauri::manage, register as `Arc<CaptureEngine>`.
// state.rs example:
// .manage(std::sync::Arc::new(CaptureEngine::new()))
```

4. 在 `engine.rs` 的 transition 加 event emit（Phase 3 命令也会复用）：

```rust
    /// Same as `transition`, but also emits the `integration-capture`
    /// event with the new state. Use this from any path that needs
    /// the UI to react.
    pub async fn transition_and_emit(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
        next: CaptureState,
        error: Option<String>,
    ) -> IntegrationResult<()> {
        use tauri::Emitter;
        self.transition(session_id, next, error).await?;
        let handle = self.get(session_id).await?;
        let s = handle.inner.read().await;
        let payload = golish_integrations::types::CaptureEventPayload {
            session_id: s.session_id.clone(),
            tool_id: s.tool_id.clone(),
            group_id: s.group_id.clone(),
            state: s.state,
            captured_fields: s.captured_fields.clone(),
            failed_rules: s.failed_rules.clone(),
            error_message: s.error_message.clone(),
        };
        drop(s);
        let _ = app.emit("integration-capture", payload);
        Ok(())
    }
```

5. 把 T2.4 `try_extract` 内所有 `s.transition(...)` 直接调用替换为 `self.transition_and_emit(app, session_id, ..., ...).await?`；同理 TTL watcher 也调 `transition_and_emit` 而非 `transition`。

**验证**：
```bash
cd backend && cargo check -p golish
cd backend && cargo nextest run -p golish -E 'test(tools::integrations::capture)' --status-level fail
```
预期：编译过；T2.2 的 5 个 engine 测试仍 pass（transition 测试需更新——pass `None` app handle 的测试可以删，新加 `transition_and_emit` 单独测）。

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/capture/engine.rs backend/crates/golish/src/tools/integrations/state.rs
git commit -m "feat(capture): TTL watcher + event emission for state transitions"
```

### T2.6 · 注册 CaptureEngine 到 Tauri state + 启动 TTL watcher

**文件**：修改 `backend/crates/golish/src/app/tauri_app.rs`（或具体注册 Tauri state 的入口；以现状为准）

**步骤**：

1. 找到 `tauri::Builder::default().manage(...)` 调用链。
2. 加：

```rust
        .manage::<std::sync::Arc<crate::tools::integrations::capture::CaptureEngine>>(
            std::sync::Arc::new(crate::tools::integrations::capture::CaptureEngine::new()),
        )
```

3. 在 `setup` 闭包内加：

```rust
            let engine_arc: tauri::State<std::sync::Arc<crate::tools::integrations::capture::CaptureEngine>> = app.state();
            let engine = engine_arc.inner().clone();
            engine.spawn_ttl_watcher();
```

**验证**：
```bash
cd backend && cargo check -p golish
```

**提交**：
```bash
git add backend/crates/golish/src/app/tauri_app.rs
git commit -m "feat(capture): register CaptureEngine + spawn TTL watcher at startup"
```

**Review Checkpoint**：
- 用户审引擎模块分层（engine / session / data_dir）是否合理
- 审 TTL watcher 节奏（10 秒扫一次）是否过敏感
- 删除 Phase 0 的 spike binary `backend/crates/golish/examples/capture_spike.rs`

---

## Phase 3 · IPC 命令 + 前端 API wrapper（90 分钟）

**目标**：3 个 Tauri command 注册 + frontend api 多 3 个 wrapper 函数；devtools 能 invoke 全链路通。

### T3.1 · `integrations_capture_start` 命令

**文件**：新建 `backend/crates/golish/src/tools/integrations/capture_commands.rs`（与现有 `commands.rs` 平级，避免单文件过大）；修改 `mod.rs` 加 `pub mod capture_commands;`

**步骤**：

1. 创建 `capture_commands.rs`：

```rust
//! Tauri commands for the credential capture engine.

use std::sync::Arc;

use golish_integrations::error::IntegrationError;
use golish_integrations::types::{CaptureSessionInfo, CaptureState};
use serde::Deserialize;
use tauri::Manager;

use crate::error::GolishError;
use crate::tools::integrations::capture::CaptureEngine;
use crate::tools::integrations::state::IntegrationsState;

fn to_golish(e: IntegrationError) -> GolishError {
    GolishError::Internal(e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct CaptureStartArgs {
    pub tool_id: String,
    pub group_id: String,
}

#[tauri::command]
pub async fn integrations_capture_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, IntegrationsState>,
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureStartArgs,
) -> Result<CaptureSessionInfo, GolishError> {
    let CaptureStartArgs { tool_id, group_id } = args;

    // 1. Resolve schema → group → recipe.
    let resolved = state.list_schemas().await.map_err(to_golish)?;
    let res = resolved
        .iter()
        .find(|r| r.tool_id == tool_id)
        .ok_or_else(|| GolishError::Internal(format!("[INTEGRATION_NOT_FOUND] {tool_id}")))?;
    let group = res
        .schema
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| GolishError::Internal(format!("[INTEGRATION_NOT_FOUND] {tool_id}/{group_id}")))?;
    let recipe = group
        .capture
        .clone()
        .ok_or_else(|| to_golish(IntegrationError::CaptureNoRecipe))?;

    // 2. Register session.
    let handle = engine
        .register(tool_id.clone(), group_id.clone(), recipe)
        .await
        .map_err(to_golish)?;

    // 3. Create webview.
    engine
        .start_webview(&app, &handle)
        .map_err(|e| {
            // If webview creation fails, roll back the session.
            let sid = handle.session_id.clone();
            let app_clone = app.clone();
            let engine_clone = engine.inner().clone();
            tauri::async_runtime::spawn(async move {
                let _ = engine_clone
                    .transition_and_emit(
                        &app_clone,
                        &sid,
                        CaptureState::Failed,
                        Some(format!("webview_create: {e}")),
                    )
                    .await;
            });
            to_golish(e)
        })?;

    let s = handle.inner.read().await;
    Ok(s.info())
}

#[derive(Debug, Deserialize)]
pub struct CaptureSessionArgs {
    pub session_id: String,
}

#[tauri::command]
pub async fn integrations_capture_status(
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureSessionArgs,
) -> Result<CaptureSessionInfo, GolishError> {
    let handle = engine.get(&args.session_id).await.map_err(to_golish)?;
    let s = handle.inner.read().await;
    Ok(s.info())
}

#[tauri::command]
pub async fn integrations_capture_cancel(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureSessionArgs,
) -> Result<(), GolishError> {
    engine
        .transition_and_emit(&app, &args.session_id, CaptureState::Cancelled, None)
        .await
        .map_err(to_golish)?;
    let label = format!("capture-{}", args.session_id);
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.close();
    }
    Ok(())
}
```

2. 在 `mod.rs` 加 `pub mod capture_commands;`。

3. 在 `backend/crates/golish/src/commands_facade/integrations.rs` 末尾追加：

```rust
pub use crate::tools::integrations::capture_commands::{
    integrations_capture_cancel, integrations_capture_start, integrations_capture_status,
};
```

4. 在 `backend/crates/golish/src/commands_registry.rs` 找到 `tauri::generate_handler![...]` 调用，把 3 个新命令名加入：

```rust
            integrations_capture_start,
            integrations_capture_status,
            integrations_capture_cancel,
```

**验证**：
```bash
cd backend && cargo check -p golish
cd backend && cargo nextest run -p golish -E 'test(tools::integrations::capture_commands)' --status-level fail
```
预期：编译过。

**提交**：
```bash
git add backend/crates/golish/src/tools/integrations/capture_commands.rs backend/crates/golish/src/tools/integrations/mod.rs backend/crates/golish/src/commands_facade/integrations.rs backend/crates/golish/src/commands_registry.rs
git commit -m "feat(capture): 3 Tauri commands (start / status / cancel)"
```

### T3.2 · 前端 API wrappers

**文件**：修改 `frontend/lib/api/integrations.ts`

**步骤**：

1. 在文件末尾追加：

```ts
// ────────────────────────────────────────────────────────────────────────
// Capture IPC wrappers
// ────────────────────────────────────────────────────────────────────────

/**
 * Start a capture session for `(toolId, groupId)`. Opens a Tauri
 * webview window for the user to log in. The returned session info
 * already includes `expires_at` so the UI can show a countdown
 * without polling.
 *
 * Errors:
 *  - `CAPTURE_NO_RECIPE` — schema has no `capture` field
 *  - `CAPTURE_ALREADY_RUNNING` — caller must cancel first
 *  - `INTEGRATION_NOT_FOUND` — tool / group id unknown
 *  - `WEBVIEW_CREATE_FAILED` — Tauri couldn't open the window
 */
export async function captureStart(args: {
  toolId: string;
  groupId: string;
}): Promise<CaptureSessionInfo> {
  return invoke<CaptureSessionInfo>("integrations_capture_start", {
    args: { tool_id: args.toolId, group_id: args.groupId },
  });
}

/**
 * Poll one session's current state. Prefer subscribing to the
 * `integration-capture` event for push-based updates; this is a
 * fallback for reconnect / rehydrate scenarios.
 */
export async function captureStatus(args: {
  sessionId: string;
}): Promise<CaptureSessionInfo> {
  return invoke<CaptureSessionInfo>("integrations_capture_status", {
    args: { session_id: args.sessionId },
  });
}

/**
 * Cancel an in-flight session. Closes the webview, runs cleanup,
 * and emits a final `integration-capture` event with
 * `state: "cancelled"`.
 */
export async function captureCancel(args: { sessionId: string }): Promise<void> {
  return invoke<void>("integrations_capture_cancel", {
    args: { session_id: args.sessionId },
  });
}
```

**验证**：
```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/lib/api/integrations.ts
```

**提交**：
```bash
git add frontend/lib/api/integrations.ts
git commit -m "feat(capture): frontend IPC wrappers (captureStart / status / cancel)"
```

### T3.3 · 手动 devtools 验证

**步骤**：
1. `just dev`
2. 打开 devtools console
3. 跑：
```js
await window.__TAURI_INTERNALS__.invoke("integrations_capture_start", {
  args: { tool_id: "enscan-go", group_id: "aqc" }
});
```
预期：返回 `{ session_id, state: "waiting_login", login_url: "https://aiqicha.baidu.com", ... }` 且**真的弹出一个新窗口**。

**Review Checkpoint**：
- 用户截图弹窗形态
- 关掉窗口后 devtools 跑 `await window.__TAURI_INTERNALS__.invoke("integrations_capture_status", { args: { session_id: "..." } })` 应返回 `cancelled` 或 `captured`（取决于关窗时刻）

---

## Phase 4 · 前端 UX（3-4 小时）

**目标**：⚡ 按钮 + 二次确认 dialog + 三态 toast + `useCaptureSession` hook + 三态全走 + i18n。

### T4.1 · i18n 新 key

**文件**：修改 `frontend/lib/i18n/en.json` 和 `frontend/lib/i18n/zh-CN.json`

**步骤**：

1. 在 `integrations` 段内追加（en.json）：

```json
    "capture": {
      "button": {
        "label": "Auto Capture",
        "tooltip": "Open a browser window to harvest credentials automatically"
      },
      "dialog": {
        "title": "Auto-capture credentials",
        "description": "Golish will open {{url}} in a separate window. After you log in, the following fields will be extracted: {{fields}}. The window auto-closes after {{ttl}} seconds.",
        "start": "Open browser & login",
        "cancel": "Cancel"
      },
      "toast": {
        "waitingLogin": "Waiting for login · {{remaining}}s left",
        "captured": "Captured {{count}} field(s) successfully",
        "partial": "Partial capture · {{captured}} captured, {{failed}} missing",
        "timeout": "Capture timed out without login",
        "failed": "Capture failed",
        "cancelled": "Capture cancelled"
      },
      "errors": {
        "noRecipe": "This integration does not support auto-capture",
        "alreadyRunning": "Capture already in progress — cancel first",
        "webviewFailed": "Cannot open browser window — check Golish permissions"
      }
    }
```

2. zh-CN.json 对应翻译：

```json
    "capture": {
      "button": {
        "label": "自动抓取",
        "tooltip": "打开浏览器窗口自动获取凭据"
      },
      "dialog": {
        "title": "自动抓取凭据",
        "description": "Golish 将在独立窗口中打开 {{url}}。登录后会自动提取以下字段：{{fields}}。窗口会在 {{ttl}} 秒后自动关闭。",
        "start": "打开浏览器并登录",
        "cancel": "取消"
      },
      "toast": {
        "waitingLogin": "等待登录 · 剩 {{remaining}} 秒",
        "captured": "成功抓取 {{count}} 个字段",
        "partial": "部分抓取 · {{captured}} 成功，{{failed}} 缺失",
        "timeout": "登录超时未完成抓取",
        "failed": "抓取失败",
        "cancelled": "已取消抓取"
      },
      "errors": {
        "noRecipe": "该集成不支持自动抓取",
        "alreadyRunning": "已有抓取任务进行中——请先取消",
        "webviewFailed": "无法打开浏览器窗口——请检查 Golish 权限"
      }
    }
```

**验证**：
```bash
pnpm exec tsc --noEmit
node -e 'JSON.parse(require("fs").readFileSync("frontend/lib/i18n/en.json","utf8"))'
node -e 'JSON.parse(require("fs").readFileSync("frontend/lib/i18n/zh-CN.json","utf8"))'
```

**提交**：
```bash
git add frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json
git commit -m "feat(capture): i18n keys for capture button / dialog / toast"
```

### T4.2 · `useCaptureSession` hook

**文件**：新建 `frontend/components/Settings/IntegrationsSettings/hooks/useCaptureSession.ts`

**步骤**：

1. 创建文件，内容：

```ts
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  captureCancel,
  captureStart,
  type CaptureEventPayload,
  type CaptureSessionInfo,
  type CaptureState,
} from "@/lib/api/integrations";

export interface UseCaptureSessionResult {
  /** Current state — `null` when never started this mount. */
  state: CaptureState | null;
  /** Live snapshot from the last start / event. */
  session: CaptureSessionInfo | null;
  /** Seconds remaining until TTL expires (UI countdown). */
  remainingSecs: number;
  /** Last captured / failed delta — re-set on every event. */
  lastEvent: CaptureEventPayload | null;
  /** Open the confirm dialog → if confirmed, calls captureStart. */
  start: (toolId: string, groupId: string) => Promise<void>;
  /** Cancel the current session (if any). */
  cancel: () => Promise<void>;
  /** Whether to render the confirm dialog. */
  confirmOpen: boolean;
  setConfirmOpen: (v: boolean) => void;
  /** Pending dialog request — captured between `start()` and user confirm. */
  pendingRequest: { toolId: string; groupId: string } | null;
  /** Actually fire the start IPC; called from the dialog. */
  proceedAfterConfirm: () => Promise<void>;
}

export function useCaptureSession(): UseCaptureSessionResult {
  const queryClient = useQueryClient();
  const [session, setSession] = useState<CaptureSessionInfo | null>(null);
  const [lastEvent, setLastEvent] = useState<CaptureEventPayload | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pendingRequest, setPendingRequest] = useState<{
    toolId: string;
    groupId: string;
  } | null>(null);
  const [remainingSecs, setRemainingSecs] = useState(0);
  const unlistenRef = useRef<(() => void) | null>(null);

  const start = useCallback(async (toolId: string, groupId: string) => {
    setPendingRequest({ toolId, groupId });
    setConfirmOpen(true);
  }, []);

  const proceedAfterConfirm = useCallback(async () => {
    if (!pendingRequest) return;
    setConfirmOpen(false);
    try {
      const info = await captureStart(pendingRequest);
      setSession(info);
      setLastEvent(null);
    } catch (e) {
      setSession(null);
      setLastEvent({
        session_id: "",
        tool_id: pendingRequest.toolId,
        group_id: pendingRequest.groupId,
        state: "failed",
        error_message: String(e),
      });
    } finally {
      setPendingRequest(null);
    }
  }, [pendingRequest]);

  const cancel = useCallback(async () => {
    if (!session) {
      setConfirmOpen(false);
      setPendingRequest(null);
      return;
    }
    try {
      await captureCancel({ sessionId: session.session_id });
    } catch {
      /* ignore — event listener will reflect the new state */
    }
  }, [session]);

  // Subscribe to global `integration-capture` event.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const un = await listen<CaptureEventPayload>(
        "integration-capture",
        (evt) => {
          if (cancelled) return;
          // Only react to events for our session.
          if (!session || evt.payload.session_id !== session.session_id) {
            return;
          }
          setLastEvent(evt.payload);
          setSession((prev) =>
            prev
              ? {
                  ...prev,
                  state: evt.payload.state,
                  captured_fields: evt.payload.captured_fields ?? [],
                  failed_rules: evt.payload.failed_rules ?? [],
                  error_message: evt.payload.error_message,
                  updated_at: Date.now(),
                }
              : prev,
          );
          // On a successful capture, invalidate the integration query
          // so the form re-renders with the new "configured" badge.
          if (evt.payload.state === "captured" || evt.payload.state === "partial") {
            queryClient.invalidateQueries({
              queryKey: ["integration", evt.payload.tool_id, evt.payload.group_id],
            });
          }
        },
      );
      unlistenRef.current = un;
    })();
    return () => {
      cancelled = true;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, [session, queryClient]);

  // Countdown timer.
  useEffect(() => {
    if (!session || !session.expires_at) {
      setRemainingSecs(0);
      return;
    }
    const tick = () => {
      const remaining = Math.max(
        0,
        Math.ceil((session.expires_at! - Date.now()) / 1000),
      );
      setRemainingSecs(remaining);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [session]);

  return {
    state: session?.state ?? lastEvent?.state ?? null,
    session,
    remainingSecs,
    lastEvent,
    start,
    cancel,
    confirmOpen,
    setConfirmOpen,
    pendingRequest,
    proceedAfterConfirm,
  };
}
```

**验证**：
```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/Settings/IntegrationsSettings/hooks/useCaptureSession.ts
```

**提交**：
```bash
git add frontend/components/Settings/IntegrationsSettings/hooks/useCaptureSession.ts
git commit -m "feat(capture): useCaptureSession hook with state machine + event subscription"
```

### T4.3 · `CaptureButton` 组件

**文件**：新建 `frontend/components/Settings/IntegrationsSettings/CaptureButton.tsx`

**步骤**：

1. 创建文件：

```tsx
import { Wand2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { IntegrationGroup } from "@/lib/api/integrations";

export interface CaptureButtonProps {
  toolId: string;
  group: IntegrationGroup;
  disabled?: boolean;
  onStart: (toolId: string, groupId: string) => void;
}

/**
 * Renders the ⚡ button only when `group.capture` is non-null.
 * Returns null otherwise — keeps caller code uniform regardless of
 * whether the group supports auto-capture.
 */
export function CaptureButton({
  toolId,
  group,
  disabled,
  onStart,
}: CaptureButtonProps) {
  const { t } = useTranslation();
  if (!group.capture) return null;
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onStart(toolId, group.id)}
            aria-label={t("integrations.capture.button.label")}
          >
            <Wand2 className="h-4 w-4 mr-1" />
            {t("integrations.capture.button.label")}
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          {t("integrations.capture.button.tooltip")}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
```

2. 单测 `frontend/components/Settings/IntegrationsSettings/CaptureButton.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { CaptureButton } from "./CaptureButton";
import type { IntegrationGroup } from "@/lib/api/integrations";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (k: string) => k,
  }),
}));

describe("<CaptureButton>", () => {
  const baseGroup: IntegrationGroup = {
    id: "aqc",
    name: "爱企查",
    fields: [{ key: "cookies.aqc", label: "Cookie", type: "secret_textarea" }],
  };

  it("renders nothing when group.capture is absent", () => {
    const { container } = render(
      <CaptureButton
        toolId="enscan-go"
        group={baseGroup}
        onStart={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders the button when group.capture is present", () => {
    const group: IntegrationGroup = {
      ...baseGroup,
      capture: {
        login_url: "https://aiqicha.baidu.com",
        timeout_secs: 300,
        rules: [
          {
            type: "cookie",
            domain: ".aiqicha.baidu.com",
            name: "BDUSS",
            target_field: "cookies.aqc",
          },
        ],
      },
    };
    render(
      <CaptureButton
        toolId="enscan-go"
        group={group}
        onStart={() => {}}
      />,
    );
    expect(screen.getByRole("button")).toBeInTheDocument();
  });

  it("invokes onStart with (toolId, groupId) when clicked", () => {
    const onStart = vi.fn();
    const group: IntegrationGroup = {
      ...baseGroup,
      capture: {
        login_url: "https://aiqicha.baidu.com",
        timeout_secs: 300,
        rules: [],
      },
    };
    render(
      <CaptureButton toolId="enscan-go" group={group} onStart={onStart} />,
    );
    screen.getByRole("button").click();
    expect(onStart).toHaveBeenCalledWith("enscan-go", "aqc");
  });
});
```

**验证**：
```bash
pnpm vitest run frontend/components/Settings/IntegrationsSettings/CaptureButton.test.tsx
pnpm exec tsc --noEmit
```

**提交**：
```bash
git add frontend/components/Settings/IntegrationsSettings/CaptureButton.tsx frontend/components/Settings/IntegrationsSettings/CaptureButton.test.tsx
git commit -m "feat(capture): CaptureButton component with conditional render"
```

### T4.4 · `CaptureConfirmDialog` + `CaptureStatusToast`

**文件**：
- 新建 `frontend/components/Settings/IntegrationsSettings/CaptureConfirmDialog.tsx`
- 新建 `frontend/components/Settings/IntegrationsSettings/CaptureStatusToast.tsx`

**步骤**：

1. `CaptureConfirmDialog.tsx`：

```tsx
import { useTranslation } from "react-i18next";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import type { CaptureRecipe } from "@/lib/api/integrations";

export interface CaptureConfirmDialogProps {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  recipe: CaptureRecipe | null;
  onConfirm: () => void;
}

export function CaptureConfirmDialog({
  open,
  onOpenChange,
  recipe,
  onConfirm,
}: CaptureConfirmDialogProps) {
  const { t } = useTranslation();
  if (!recipe) return null;
  const fields = recipe.rules
    .map((r) =>
      "target_field" in r ? r.target_field : "",
    )
    .filter(Boolean)
    .join(", ");
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("integrations.capture.dialog.title")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("integrations.capture.dialog.description", {
              url: recipe.login_url,
              fields,
              ttl: recipe.timeout_secs,
            })}
            {recipe.instructions && (
              <span className="block mt-2 text-sm opacity-80">
                {recipe.instructions}
              </span>
            )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>
            {t("integrations.capture.dialog.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm}>
            {t("integrations.capture.dialog.start")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
```

2. `CaptureStatusToast.tsx`：

```tsx
import { CheckCircle2, Loader2, XCircle, Clock, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import type { CaptureSessionInfo, CaptureState } from "@/lib/api/integrations";

const ICON_BY_STATE: Record<CaptureState, JSX.Element> = {
  waiting_login: <Loader2 className="h-4 w-4 animate-spin" />,
  navigating: <Loader2 className="h-4 w-4 animate-spin" />,
  extracting: <Loader2 className="h-4 w-4 animate-spin" />,
  captured: <CheckCircle2 className="h-4 w-4 text-green-500" />,
  partial: <AlertTriangle className="h-4 w-4 text-yellow-500" />,
  failed: <XCircle className="h-4 w-4 text-red-500" />,
  timeout: <Clock className="h-4 w-4 text-red-500" />,
  cancelled: <XCircle className="h-4 w-4 text-gray-400" />,
};

export interface CaptureStatusToastProps {
  session: CaptureSessionInfo | null;
  remainingSecs: number;
  onCancel: () => void;
}

export function CaptureStatusToast({
  session,
  remainingSecs,
  onCancel,
}: CaptureStatusToastProps) {
  const { t } = useTranslation();
  if (!session) return null;
  const state = session.state;
  const icon = ICON_BY_STATE[state];

  const text = (() => {
    switch (state) {
      case "waiting_login":
      case "navigating":
      case "extracting":
        return t("integrations.capture.toast.waitingLogin", {
          remaining: remainingSecs,
        });
      case "captured":
        return t("integrations.capture.toast.captured", {
          count: session.captured_fields?.length ?? 0,
        });
      case "partial":
        return t("integrations.capture.toast.partial", {
          captured: session.captured_fields?.length ?? 0,
          failed: session.failed_rules?.length ?? 0,
        });
      case "timeout":
        return t("integrations.capture.toast.timeout");
      case "failed":
        return session.error_message ?? t("integrations.capture.toast.failed");
      case "cancelled":
        return t("integrations.capture.toast.cancelled");
    }
  })();

  const isInflight = state === "waiting_login" || state === "navigating" || state === "extracting";

  return (
    <div className="flex items-center gap-2 rounded-md border bg-card px-3 py-2 text-sm shadow-sm">
      {icon}
      <span className="flex-1">{text}</span>
      {isInflight && (
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("integrations.capture.dialog.cancel")}
        </Button>
      )}
    </div>
  );
}
```

**验证**：
```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/Settings/IntegrationsSettings/CaptureConfirmDialog.tsx frontend/components/Settings/IntegrationsSettings/CaptureStatusToast.tsx
```

**提交**：
```bash
git add frontend/components/Settings/IntegrationsSettings/CaptureConfirmDialog.tsx frontend/components/Settings/IntegrationsSettings/CaptureStatusToast.tsx
git commit -m "feat(capture): confirm dialog + status toast components"
```

### T4.5 · 在 `IntegrationGroup.tsx` 集成 capture

**文件**：修改 `frontend/components/Settings/IntegrationsSettings/IntegrationGroup.tsx`

**步骤**：

1. 用 `Read` 看现有 IntegrationGroup.tsx，找到 Save/Clear/Test 按钮所在 toolbar / row。
2. 在 import 段加：

```tsx
import { CaptureButton } from "./CaptureButton";
import { CaptureConfirmDialog } from "./CaptureConfirmDialog";
import { CaptureStatusToast } from "./CaptureStatusToast";
import { useCaptureSession } from "./hooks/useCaptureSession";
```

3. 在组件 body 内，紧邻 `const { ... } = useIntegrationGroup(...)`  加：

```tsx
  const capture = useCaptureSession();
```

4. 在 button row（Save/Clear/Test）旁加 CaptureButton：

```tsx
  <CaptureButton
    toolId={toolId}
    group={group}
    onStart={capture.start}
    disabled={capture.session?.state === "waiting_login" || capture.session?.state === "navigating" || capture.session?.state === "extracting"}
  />
```

5. 在组件返回 JSX 末尾（或紧贴 toolbar 上方）渲染 toast + dialog：

```tsx
  <CaptureStatusToast
    session={capture.session}
    remainingSecs={capture.remainingSecs}
    onCancel={capture.cancel}
  />
  <CaptureConfirmDialog
    open={capture.confirmOpen}
    onOpenChange={capture.setConfirmOpen}
    recipe={group.capture ?? null}
    onConfirm={capture.proceedAfterConfirm}
  />
```

6. 更新现有 `IntegrationGroup.test.tsx`，加一个 case：

```tsx
  it("hides capture button when group.capture is absent", () => {
    const { container } = render(<IntegrationGroup {...baseProps} />);
    // The CaptureButton renders null, so the toolbar count stays the same.
    const buttons = container.querySelectorAll("button");
    expect(
      [...buttons].some((b) => b.textContent?.includes("Auto Capture") || b.textContent?.includes("自动抓取")),
    ).toBe(false);
  });
```

> 如果原文件结构变了，按当前实际 toolbar 位置插入；不要改既有 Save/Clear/Test 逻辑。

**验证**：
```bash
pnpm vitest run frontend/components/Settings/IntegrationsSettings/
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/Settings/IntegrationsSettings/IntegrationGroup.tsx
```
预期：现有 17 个测试 + 新 1 个仍全绿。

**提交**：
```bash
git add frontend/components/Settings/IntegrationsSettings/IntegrationGroup.tsx frontend/components/Settings/IntegrationsSettings/IntegrationGroup.test.tsx
git commit -m "feat(capture): wire CaptureButton + toast + dialog into IntegrationGroup"
```

**Review Checkpoint**：
- 用户截图：⚡ 按钮在 toolbar 出现、二次确认 dialog 弹出、toast 三态显示
- 用户对 i18n 文案点 OK
- 用 `pnpm vitest run frontend/components/Settings/IntegrationsSettings/` 全绿

---

## Phase 5 · 接入 ENScan AQC + 端到端验证（90 分钟）

**目标**：唯一被纳入 P1 的真实 integration——ENScan_GO AQC——把 capture 段加进 toolsconfig；用户在 Settings → Integrations 点⚡按钮，跑通完整流程。

### T5.1 · ENScan AQC `capture` 段

**文件**：修改 `resources/toolsconfig/enscan-go.json`

**步骤**：

1. 用 `Read resources/toolsconfig/enscan-go.json` 找 `integration.groups[id=="aqc"]` 块。
2. 在 aqc group 中（test 字段之后）添加：

```jsonc
{
  "id": "aqc",
  "name": "爱企查 (AQC)",
  "fields": [
    { "key": "cookies.aqc", "type": "secret_textarea", "required": true }
  ],
  "test": { /* 原有 */ },
  "capture": {
    "login_url": "https://aiqicha.baidu.com",
    "success_url_pattern": "aiqicha\\.baidu\\.com/(home|company|usercenter)",
    "timeout_secs": 300,
    "instructions": "登录爱企查账号后，Golish 会自动提取 BDUSS cookie。",
    "rules": [
      {
        "type": "cookie",
        "domain": ".aiqicha.baidu.com",
        "name": "BDUSS",
        "target_field": "cookies.aqc",
        "required": true
      }
    ]
  }
}
```

> 注：cookie 名 `BDUSS` 是合理猜测；用户实测时如有更准确的（如 `STOKEN` / `BDUSS_BFESS`），改 schema 即可。

**验证**：
```bash
node -e 'JSON.parse(require("fs").readFileSync("resources/toolsconfig/enscan-go.json","utf8"))'
cd backend && cargo nextest run -p golish-integrations -E 'test(validate_capture_accepts_valid_recipe)' --status-level fail
```
预期：JSON valid；validate 单测仍 pass（一般 schema 也加单独的 fixture 测试，参考既有的 enscan-go.json 测试结构）。

**提交**：
```bash
git add resources/toolsconfig/enscan-go.json
git commit -m "feat(enscan): add capture recipe for AQC (cookie=BDUSS)"
```

### T5.2 · 端到端冒烟测试（手动）

**步骤**：

1. `just dev` 启动。
2. Settings → Integrations → 找到 ENScan_GO → AQC group。
3. 点⚡按钮。
4. 二次确认 dialog 出现 → 点 "打开浏览器并登录"。
5. 弹出窗口 → 在 `aiqicha.baidu.com` 用真实账号登录。
6. 登录后等 1-2 秒 → 弹窗自动关闭 → 卡片上 toast 变绿 "成功抓取 1 个字段"。
7. AQC group 的 cookies.aqc 字段显示 "已配置" badge（hax 来自 useIntegrationGroup 的 fieldValues 刷新）。
8. 跑 `enscan -n 小米 -type aqc -field icp` 验真实成功。

**记录证据到 agent-progress.md**：
- 截图三态 toast
- `enscan` 命令输出的关键行
- `~/Library/Application Support/com.golish.platform/capture-sessions/` 目录在会话结束后已清干净（`ls -la` 应只看见空目录或已删）

### T5.3 · 端到端反向测试（手动）

**步骤**：

按设计文档 §13 验收清单跑 6 个反向 case：

1. 5 分钟内不操作 → 卡片显示 "超时"
2. 中途点 "取消" → 弹窗立刻关 → 卡片回 idle
3. 同 group 重复点⚡ 在第一次未结束 → 409 错误 + 友好提示 "已有抓取任务进行中"
4. 弹窗里手动关窗 → 卡片显示 cancelled
5. `~/Library/Application Support/com.golish.platform/capture-sessions/<sid>/` 目录在 session 终止后被删
6. devtools console 跑 `await window.__TAURI_INTERNALS__.invoke("integrations_capture_status", { args: { session_id: "<id>" } })` 在 session 已 gc 后应返 `CAPTURE_SESSION_NOT_FOUND`

**记录所有结果到 agent-progress.md 的"已记录证据"段**。

### T5.4 · `just precommit` 全绿 + feature_list.json 切 passing

**步骤**：

1. `just precommit`，预期全绿（capture 相关改动应该全绿；如果 preexisting 警告还在，至少 capture 自己的文件 biome 干净 + Rust 全部 nextest pass）。
2. 更新 `feature_list.json`：把 `capture-engine` 条目 `status` 改为 `passing`；写 `evidence`。
3. 更新 `agent-progress.md`：本轮会话记录写齐。

**验证**：
```bash
just precommit
node -e 'JSON.parse(require("fs").readFileSync("feature_list.json","utf8"))'
```

**提交**：
```bash
git add feature_list.json agent-progress.md
git commit -m "chore(capture): mark capture-engine as passing + record evidence"
```

**Review Checkpoint**：
- 用户跑完 T5.2 端到端 + T5.3 反向 case
- 录屏一份完整跑通流程发给我
- 决定是否要进 P2（其余 4 个 rule type + 其余 4 个 ENScan group + FOFA / 0.zone）

---

## 任务依赖图

```
Phase 0 (spike) ─► Phase 1 (schema) ─┐
                                      ├─► Phase 2 (engine) ─► Phase 3 (IPC) ─► Phase 4 (UX) ─► Phase 5 (接入+E2E)
                                      │
                                      └─► Phase 1.5 (frontend ts mirror, T1.5) 
                                              └─► Phase 4 用
```

---

## 风险与缓解（对应设计文档 §10）

| # | 风险 | 缓解（计划中处理位置） |
|---|---|---|
| R1 | Tauri 2 API 名变化 | Phase 0 spike 强制锁定 + 写真实 binary 验三个 API |
| R2 | `data_directory` 在 Linux 行为差异 | spike 报告若失败 → 限定 P1 仅 macOS + Windows，progress 记 |
| R3 | HttpOnly cookie 可读吗 | Phase 0 spike 跑 15 秒后 cookies_for_url 输出 |
| R4 | cleartext 泄露 | T2.4 仅在 HashMap 内传输；不打日志；transition_and_emit payload 不含值 |
| R5 | 2FA 站点不工作 | P1 ENScan AQC 不需要 2FA；FOFA / 0.zone 待 P2 实测 |
| R6 | success_url_pattern 写错 | UI 在 toast 显示当前 webview URL → 用户可对照 pattern（P2 增强） |
| R7 | 多 session 并发干扰 | 同 (tool, group) 409；不同 group 各开独立 webview + data_dir |
| R8 | 用户手动关窗 | T2.3 navigation handler 自带，但需测；P2 加 `on_close` handler 触发 try_extract |
| R9 | URL scheme 攻击 | T1.4 validate_capture 拒非 http(s) |
| R10 | target_field 写错 | T1.4 cross-ref 校验 |

---

## 不在本计划范围（P2/P3 留作后续）

- P2 rule types：CookieJoined / LocalStorage / SessionStorage / PageContent / UrlQuery
- ENScan TYC / KC / RB / MIIT 的 capture recipe
- FOFA / 0.zone / Hunter / Quake / Shodan 的 capture recipe
- 多步骤 recipe（先访问 A 再 B 才能抠）
- XHR 响应抠取（点按钮触发后拦 response body）
- 跨域跳转白名单
- 抓取成功后自动 fire `integrations_test`（设计文档 §11 开放问题 2）
- vault 多打 `tags=["capture-source", "auto"]`（设计文档 §11 开放问题 5）
- AI provider key（OpenAI / Anthropic / Vertex）迁入 capture

---

## 验收清单（每个 Phase 结束 mandatory）

```
[ ] Phase 0: Tauri API spike 跑通 + 删除 spike binary
[ ] Phase 1: schema 类型 + 校验 + ts-rs 同步 + 全部单测绿 + 用户审 enum 覆盖度
[ ] Phase 2: engine 5 个测试绿 + check 通过 + spike 已删 + 用户审分层
[ ] Phase 3: 3 个 IPC 命令 + devtools 能 invoke + 用户截图弹窗
[ ] Phase 4: ⚡按钮 + dialog + toast + hook 全部加进 IntegrationGroup + vitest 18 个测试全绿 + 用户截图三态
[ ] Phase 5: 真跑 ENScan AQC 完整流程 + 反向 case 6 个 + just precommit 全绿
[ ] 最终: feature_list.json 切 passing + agent-progress.md 写齐
```

---

## 自检（按 writing-plans skill §自检 要求）

1. **规格覆盖度**：设计文档 §3-§8（schema / IPC / engine / UX / security）→ Phase 1-5 全覆盖；§9 实施分期 = Phase 0-5；§11 开放问题 4 个推到 P2/P3；§13 验收清单 11 条对应到 T5.2/T5.3 手动测 + Phase 1-2 单测。
2. **占位符扫描**：本文档无 "TODO" / "后续实现" 等占位符（所有步骤都有完整代码）；P2/P3 明确在「不在本计划范围」段列出。
3. **类型一致性**：`CaptureRecipe` / `CaptureRule` / `CaptureState` / `CaptureSessionInfo` / `FailedRule` / `CaptureEventPayload` 在 Phase 1 定义、Phase 2-5 引用，命名全一致。Tauri command 入参用 `CaptureStartArgs` / `CaptureSessionArgs` 包裹（避免 ipv `args: { tool_id, group_id }` 在前后端打架，Phase 3.1 + 3.2 双方都用 `args:` 包裹 payload）。
