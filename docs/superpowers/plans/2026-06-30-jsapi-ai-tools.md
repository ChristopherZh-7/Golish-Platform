# JS/API 收集与提取「工具内建 AI」实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 executing-plans 逐任务实现此计划。每个任务单独 commit；TDD（先红后绿）；DRY / YAGNI。

**目标：** 把两段自动 AI（AI-A 收集补全、AI-B 提取确认/补 param）焊进 `browser_collect_js_api` 与 `js_extract_apis` 两个 bridge 工具，使其自给自足，确定性核心不变，证据可追溯（I7/I8）。

**架构：** 新增一个 `LlmOneShot` 端口（`golish-app-core`，trait 对象），在 `register_pentest_tools` 注入两个工具；实现侧用 `golish_llm_providers::create_client_for_model(AiProvider::Deepseek, model, &settings)` + `LlmClient::one_shot_completion`，配置来自 `~/.golish/settings.toml [ai.deepseek]`（`SettingsManager`）。两工具内确定性逻辑先跑，AI 作为可降级叠加层；无 provider/key 自动退回纯确定性。

**技术栈：** Rust（tokio、anyhow、serde_json、regex、url）、`golish-llm-providers`(rig-core)、`golish-settings`、`golish-js-analyzer`、Playwright Node helper（不改其确定性）。

**关键既读证据（实现者无需重查）：**
- one-shot API：`golish_llm_providers::LlmClient::one_shot_completion(&self, system_prompt:&str, user_message:&str, temperature:Option<f64>, max_tokens:Option<u64>) -> anyhow::Result<String>`（`backend/crates/golish-llm-providers/src/lib.rs:263`）。
- 从 settings 建 client：`golish_llm_providers::create_client_for_model(provider:AiProvider, model:&str, settings:&golish_settings::GolishSettings) -> Result<LlmClient>`（`provider_trait/mod.rs:386`）。
- 设定：`GolishSettings.ai.deepseek: DeepSeekSettings{api_key:Option<String>, base_url:Option<String>, show_in_selector:bool}`（`golish-settings/src/schema/llm/openai_compat.rs:135`）；`SettingsManager::get().await -> GolishSettings`（`golish-settings/src/loader/mod.rs`）。
- 工具注册：`register_pentest_tools`（`golish-agent-app/src/ai/commands/bridge_config.rs:1319`）→ port `PentestToolFactory::create_bridge_tools`（`golish-app-core/src/ports/pentest/tools.rs:48`）→ `create_pentest_bridge_tools`（`golish-pentest-app/src/pentest_bridge/mod.rs:37`）→ `BrowserCollectJsApiTool::new(pool)` / `JsExtractApisTool::new(pool)`。注册点 `state.settings_manager: Arc<golish_settings::SettingsManager>` 可用。
- 默认 DeepSeek 模型：`deepseek-v4-flash`（`resources/llm-models/deepseek.json`）。
- 现状两工具：`browser_collect_js_api.rs`（确定性收集 + `ai_assist` 仅 hints）、`js_extract_apis.rs`（纯 regex + `ai_analysis` 仅 hints + 外层回喂 `param_hints`）。

---

## 文件结构（创建/修改的文件与职责）

| 文件 | 职责 | 阶段 |
|---|---|---|
| `golish-app-core/src/ports/llm/one_shot.rs`（新） | 定义 `LlmOneShot` 端口 trait（tool 调 AI 的唯一接口，避免核心依赖 LLM crate） | P0 |
| `golish-app-core/src/ports/llm/mod.rs`（新）+ `ports/mod.rs` | 导出 `llm` 模块 | P0 |
| `golish-app-core/src/ports/pentest/tools.rs` | `create_bridge_tools` 端口加 `llm: Option<Arc<dyn LlmOneShot>>` 参数 | P0 |
| `golish-agent-app/src/ai/llm_one_shot.rs`（新） | `SettingsLlmOneShot`：实现 `LlmOneShot`，持 `Arc<SettingsManager>`，内部 `create_client_for_model + one_shot_completion` | P0 |
| `golish-agent-app/src/ai/commands/bridge_config.rs` | `register_pentest_tools` 构造 `SettingsLlmOneShot` 并下传 | P0 |
| `golish-pentest-app/src/pentest_bridge/mod.rs` | `create_pentest_bridge_tools` 透传 `llm` 到两工具构造 | P0 |
| `golish-pentest-app/src/pentest_bridge/ai_oneshot.rs`（新） | 共享：`call_llm_json<T>`（一次 one-shot + JSON 解析 + timeout + 降级）、`extract_json_object` | P0 |
| `golish-js-analyzer/src/lib.rs` | `Endpoint` 加 `#[serde(default)] source: EndpointSource{Regex,Ai}` | P1 |
| `golish-pentest-app/src/pentest_bridge/js_extract_apis.rs` | AI-B：触发闸 + 切片 + one-shot 补抽/确认 + 幻觉护栏 + param 内化 + 合并去重 + provenance | P1 |
| `golish-pentest-app/src/pentest_bridge/js_ai_extract.rs`（新） | AI-B 纯函数：触发闸、切片、幻觉护栏、合并（与工具 IO 解耦，便于单测） | P1 |
| `golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs` | AI-A：`needs_more` + 精简信号 + one-shot 生 recipe + 有界第二遍编排 | P2 |
| `golish-pentest-app/src/pentest_bridge/js_ai_recipe.rs`（新） | AI-A 纯函数：`needs_more`、`compact_signals`、`sanitize_recipe`、`recipe_has_work` | P2 |
| `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`golish-js-analyzer.md` | 模块卡同步 | P3 |
| `feature_list.json` / `agent-progress.md` | 功能条目 + 进度 | P3 |

---

## 阶段 0：LLM one-shot 端口 + 工具注入

### 任务 0.1：定义 `LlmOneShot` 端口 trait

**文件：** 创建 `backend/crates/golish-app-core/src/ports/llm/one_shot.rs`、`ports/llm/mod.rs`；改 `ports/mod.rs`

**步骤：**

1. 写 `one_shot.rs`：

```rust
use anyhow::Result;

/// 端口：给 bridge 工具一个“单次 LLM 文本完成”的能力，而不让
/// golish-app-core / golish-pentest-app 直接依赖 LLM provider crate。
/// 实现侧（golish-agent-app）封装 DeepSeek client 构造与 one_shot_completion。
#[async_trait::async_trait]
pub trait LlmOneShot: Send + Sync {
    /// 单次完成。`temperature`/`max_tokens` 为 None 时用实现侧默认。
    /// 返回模型输出的纯文本；调用方负责解析（如抽 JSON）。
    async fn complete(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String>;

    /// 是否可用（无 api_key / provider 未配置时返回 false，调用方据此降级）。
    async fn is_available(&self) -> bool;
}
```

2. `ports/llm/mod.rs`：

```rust
pub mod one_shot;
pub use one_shot::LlmOneShot;
```

3. `ports/mod.rs` 加：`pub mod llm;`

**验证：** `cd backend && cargo check -p golish-app-core`（预期 exit 0）。

**提交：** `feat(app-core): add LlmOneShot port for tool-side one-shot completions`

### 任务 0.2：扩展 `create_bridge_tools` 端口签名

**文件：** `backend/crates/golish-app-core/src/ports/pentest/tools.rs`

**步骤：**

1. 在 `create_bridge_tools` trait 方法签名末尾加参数 `llm: Option<std::sync::Arc<dyn crate::ports::llm::LlmOneShot>>`。

**验证：** `cd backend && cargo check -p golish-app-core`（会因实现未更新而在依赖 crate 失败；本任务仅核 app-core 本身编译该 trait 定义 OK，下游在 0.3/0.4 修）。

**提交：** `feat(app-core): thread optional LlmOneShot into create_bridge_tools port`

### 任务 0.3：实现 `SettingsLlmOneShot`

**文件：** 创建 `backend/crates/golish-agent-app/src/ai/llm_one_shot.rs`；在 `ai/mod.rs` 加 `pub mod llm_one_shot;`

**前置：** 确认 `golish-agent-app/Cargo.toml` 已依赖 `golish-llm-providers`、`golish-settings`（注册点已用 `state.settings_manager`，settings 应已是依赖；如缺 `golish-llm-providers` 则本任务加入）。

**步骤：**

1. 写实现：

```rust
use std::sync::Arc;
use anyhow::Result;
use golish_app_core::ports::llm::LlmOneShot;
use golish_llm_providers::{create_client_for_model, AiProvider};
use golish_settings::SettingsManager;

/// 固定走 DeepSeek 的 one-shot 实现；配置来自 ~/.golish/settings.toml [ai.deepseek]。
pub struct SettingsLlmOneShot {
    settings: Arc<SettingsManager>,
    model: String, // 默认 "deepseek-v4-flash"
}

impl SettingsLlmOneShot {
    pub fn new(settings: Arc<SettingsManager>) -> Self {
        Self { settings, model: "deepseek-v4-flash".to_string() }
    }
}

#[async_trait::async_trait]
impl LlmOneShot for SettingsLlmOneShot {
    async fn complete(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String> {
        let settings = self.settings.get().await;
        let client = create_client_for_model(AiProvider::Deepseek, &self.model, &settings).await?;
        client
            .one_shot_completion(system_prompt, user_message, temperature, max_tokens)
            .await
    }

    async fn is_available(&self) -> bool {
        let settings = self.settings.get().await;
        settings
            .ai
            .deepseek
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}
```

**验证：** `cd backend && cargo check -p golish-agent-app`（预期 exit 0）。

**提交：** `feat(agent-app): SettingsLlmOneShot implementing LlmOneShot over DeepSeek`

### 任务 0.4：在注册点注入，并透传到两工具构造

**文件：** `bridge_config.rs`（register_pentest_tools）、`golish-pentest-app/src/pentest_bridge/mod.rs`（create_pentest_bridge_tools）；`GolishPentestToolFactory` 的 `create_bridge_tools` 实现

**步骤：**

1. `register_pentest_tools` 内构造：
```rust
let llm_one_shot: Option<Arc<dyn golish_app_core::ports::llm::LlmOneShot>> =
    Some(Arc::new(crate::ai::llm_one_shot::SettingsLlmOneShot::new(state.settings_manager.clone())));
```
   并把 `llm_one_shot` 传入 `create_bridge_tools(...)` 调用末参。
2. `GolishPentestToolFactory::create_bridge_tools` 实现：签名加 `llm` 参数，转调 `create_pentest_bridge_tools(..., llm)`。
3. `create_pentest_bridge_tools`：签名加 `llm: Option<Arc<dyn LlmOneShot>>`；改两处构造为
   `BrowserCollectJsApiTool::new(pool.clone(), llm.clone())`、`JsExtractApisTool::new(pool.clone(), llm.clone())`（构造签名在 P1/P2 任务里扩展；本任务先扩 struct 字段，见下）。
4. 两 struct 暂加字段：
```rust
pub struct JsExtractApisTool { pool: Arc<PgPool>, llm: Option<Arc<dyn LlmOneShot>> }
impl JsExtractApisTool { pub fn new(pool: Arc<PgPool>, llm: Option<Arc<dyn LlmOneShot>>) -> Self { Self { pool, llm } } }
```
   `BrowserCollectJsApiTool` 同样加 `llm` 字段与构造参数。更新其它 `::new(` 调用点（grep `BrowserCollectJsApiTool::new` / `JsExtractApisTool::new`，含测试，传 `None`）。

**验证：**
```bash
cd backend && cargo check -p golish-pentest-app -p golish-agent-app
cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api js_extract_apis --status-level fail
```
预期 exit 0（注入后行为不变，AI 默认未触发）。

**提交：** `feat(pentest): inject LlmOneShot handle into js collect/extract tools`

### 任务 0.5：共享 one-shot JSON helper

**文件：** 创建 `backend/crates/golish-pentest-app/src/pentest_bridge/ai_oneshot.rs`；`pentest_bridge/mod.rs` 加 `mod ai_oneshot;`

**步骤：**

1. 写 helper（含 JSON 抽取 + timeout + 降级语义）：
```rust
use std::sync::Arc;
use std::time::Duration;
use golish_app_core::ports::llm::LlmOneShot;
use serde_json::Value;

const AI_CALL_TIMEOUT_MS: u64 = 60_000;

/// 单次 LLM 调用并把响应解析为 JSON。任何失败（无 llm / 不可用 / 超时 /
/// 非 JSON）都返回 None（调用方降级到确定性结果），不向上抛错。
pub async fn call_llm_json(
    llm: &Option<Arc<dyn LlmOneShot>>,
    system_prompt: &str,
    user_json: &str,
    temperature: f64,
    max_tokens: u64,
) -> Option<Value> {
    let llm = llm.as_ref()?;
    if !llm.is_available().await { return None; }
    let fut = llm.complete(system_prompt, user_json, Some(temperature), Some(max_tokens));
    let text = match tokio::time::timeout(Duration::from_millis(AI_CALL_TIMEOUT_MS), fut).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => { tracing::warn!("[jsapi-ai] one-shot failed: {e}"); return None; }
        Err(_) => { tracing::warn!("[jsapi-ai] one-shot timed out"); return None; }
    };
    extract_json_object(&text)
}

/// 从可能含 ```json 围栏或前后噪声的文本里抽第一个 JSON 对象/数组。
pub fn extract_json_object(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) { return Some(v); }
    // ```json ... ``` 围栏
    if let Some(start) = t.find("```") {
        if let Some(rest) = t[start + 3..].split_once('\n') {
            if let Some(end) = rest.1.find("```") {
                if let Ok(v) = serde_json::from_str::<Value>(rest.1[..end].trim()) { return Some(v); }
            }
        }
    }
    // 最外层 { .. } 或 [ .. ]
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(s), Some(e)) = (t.find(open), t.rfind(close)) {
            if e > s {
                if let Ok(v) = serde_json::from_str::<Value>(&t[s..=e]) { return Some(v); }
            }
        }
    }
    None
}
```

**步骤（测试）：** 在该文件 `#[cfg(test)]` 加：
```rust
#[test]
fn extract_json_handles_fenced_and_bare() {
    assert!(extract_json_object("{\"a\":1}").is_some());
    assert!(extract_json_object("noise\n```json\n{\"a\":1}\n```\ntail").is_some());
    assert!(extract_json_object("prefix [1,2,3] suffix").is_some());
    assert!(extract_json_object("not json").is_none());
}
```

**验证：** `cd backend && cargo nextest run -p golish-pentest-app extract_json --status-level fail`（预期 1 passed）。

**提交：** `feat(pentest): shared one-shot LLM JSON helper with timeout + degrade`

---

## 阶段 1：AI-B（`js_extract_apis` 提取确认 + 补 param）

> 落地 `docs/design/2026-06-09-js-api-extraction-ai-augmented.md` 的 hybrid（触发闸/切片/幻觉护栏），并把 param 从外层回喂改为工具内 AI 读文本补全。

### 任务 1.1：`Endpoint` 加 `source` 字段

**文件：** `backend/crates/golish-js-analyzer/src/lib.rs`

**步骤：**

1. 加枚举与字段：
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EndpointSource { #[default] Regex, Ai }
```
   在 `pub struct Endpoint { ... }` 末尾加：`#[serde(default)] pub source: EndpointSource,`
2. 修所有构造 `Endpoint { ... }` 的位置（`patterns.rs` 的 `endpoint_from_*`）补 `source: EndpointSource::Regex,`。

**步骤（测试）：** `lib_tests.rs` 加：
```rust
#[test]
fn endpoint_source_defaults_to_regex_on_old_json() {
    let ep: Endpoint = serde_json::from_str(r#"{"method":"GET","path":"/a","auth":"none","url_kind":"literal","source_file":"a.js","line":1,"confidence":1.0,"kind":"fetch","has_path_params":false,"id_param_position":null}"#).unwrap();
    assert_eq!(ep.source, EndpointSource::Regex);
}
```

**验证：** `cd backend && cargo nextest run -p golish-js-analyzer --status-level fail`（预期全绿）。

**提交：** `feat(js-analyzer): add Endpoint.source (regex|ai), serde-default regex`

### 任务 1.2：AI-B 纯函数模块（触发闸 / 切片 / 幻觉护栏 / 合并）

**文件：** 创建 `backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_extract.rs`；`mod.rs` 加 `mod js_ai_extract;`

**步骤（先写测试，红）：** 见每函数下的测试。实现 4 个纯函数：

1. 触发闸：
```rust
pub struct TriggerThresholds { pub min_eps_per_kb: f64, pub minified_avg_line: usize }
impl Default for TriggerThresholds { fn default() -> Self { Self { min_eps_per_kb: 0.2, minified_avg_line: 2000 } } }

/// 判断单文件是否值得喂 AI：低产出 或 高度 minified。
pub fn should_ai_analyze(source: &str, regex_endpoint_count: usize, t: &TriggerThresholds) -> bool {
    let kb = (source.len() as f64 / 1024.0).max(0.001);
    let eps_per_kb = regex_endpoint_count as f64 / kb;
    let lines = source.lines().count().max(1);
    let avg_line = source.len() / lines;
    eps_per_kb < t.min_eps_per_kb || avg_line > t.minified_avg_line
}
```
   测试：`fetch` 稀疏的大文件命中；端点密集的小文件不命中；单行 minified 命中。

2. 切片（关键词窗口 + 合并重叠）：
```rust
const WINDOW: usize = 1500;
const MAX_CHUNKS: usize = 12;
pub fn slice_network_windows(source: &str) -> Vec<String> { /* 用 regex 定位 fetch|axios|XMLHttpRequest|\.ajax|\.(get|post|put|delete|request)\(|baseURL|api|endpoint|url\s*[:=]，取 ±WINDOW，合并重叠，截 MAX_CHUNKS */ }
```
   测试：两个相邻命中合并为一个窗口；超过 MAX_CHUNKS 被截断；无命中返回空。

3. 幻觉护栏：
```rust
/// AI 端点 path 去掉 ${...} 占位后的最长字面子串（>=6）必须实锚源码。
pub fn ai_path_anchored(path: &str, source: &str) -> bool {
    path.split(|c| c == '$' || c == '{' || c == '}')
        .map(str::trim).filter(|s| s.len() >= 6)
        .max_by_key(|s| s.len())
        .map(|frag| source.contains(frag))
        .unwrap_or(false)
}
```
   测试：真实片段锚到→true；编造路径→false；纯模板 `${x}`（无≥6 字面）→false。

4. 合并去重：
```rust
pub fn merge_dedup(regex_eps: Vec<Endpoint>, ai_eps: Vec<Endpoint>) -> Vec<Endpoint> { /* by (method.to_upper, path)，tie 取 regex */ }
```
   测试：同 (method,path) regex 优先；ai 独有的保留且 source=Ai。

**验证：** `cd backend && cargo nextest run -p golish-pentest-app js_ai_extract --status-level fail`（预期全绿）。

**提交：** `feat(pentest): pure helpers for AI-B (trigger/slice/anchor/merge)`

### 任务 1.3：把 AI-B 接入 `js_extract_apis::execute`

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`

**步骤：**

1. 在 regex 抽取（现有 `extract_from_files`）之后、落库之前插入 AI pass（仅当 `self.llm` 可用且 `ai != Some(false)`）：
```rust
let ai_on = args.get("ai").and_then(|v| v.as_bool()).unwrap_or(true);
let mut ai_added: Vec<Endpoint> = Vec::new();
let mut ai_param_hints: Vec<ParamHint> = Vec::new();
if ai_on {
    let thresholds = TriggerThresholds::default();
    let mut budget_files = 8usize; let mut budget_bytes = 256_000usize;
    for (file, src) in sources.iter() {
        if budget_files == 0 || budget_bytes == 0 { break; }
        let rc = report.endpoints.iter().filter(|e| &e.source_file == file).count();
        if !should_ai_analyze(src, rc, &thresholds) { continue; }
        let chunks = slice_network_windows(src);
        let payload = serde_json::json!({ "source_file": file, "chunks": chunks });
        let Some(v) = call_llm_json(&self.llm, AI_EXTRACT_SYSTEM, &payload.to_string(), 0.0, 4096).await else { continue; };
        budget_files -= 1; budget_bytes = budget_bytes.saturating_sub(src.len().min(budget_bytes));
        for raw in v.get("endpoints").and_then(|x| x.as_array()).into_iter().flatten() {
            if let Some(ep) = parse_ai_endpoint(raw, file) {        // method/path/auth → Endpoint{source:Ai}
                if ai_path_anchored(&ep.path, src) { ai_added.push(ep); }
            }
        }
        for raw in v.get("params").and_then(|x| x.as_array()).into_iter().flatten() {
            if let Some(h) = parse_ai_param_hint(raw, src) { ai_param_hints.push(h); } // 仅保留 name 见于 src 的
        }
    }
}
let filtered_owned = merge_dedup(report.endpoints.clone(), ai_added);
// 后续 min_confidence 过滤、落库改用 filtered_owned；param 合并把 ai_param_hints 并入既有 param_hints
```
2. 定义系统 prompt 常量（沿用 2026-06-09 §3.4）：
```rust
const AI_EXTRACT_SYSTEM: &str = "You extract real HTTP API call-sites from JavaScript chunks for an authorized security test. \
Return strict JSON {\"endpoints\":[{\"method\",\"path\",\"auth\"}],\"params\":[{\"path\",\"method\",\"params\":[name]}]}. \
method uppercase; path is the request path/URL; auth in {none,bearer,cookie,header,unknown}. \
Only output endpoints/params that literally appear in the supplied chunks. Never invent. If unsure, omit.";
```
3. `parse_ai_endpoint`：构造 `Endpoint{ source: EndpointSource::Ai, confidence: 0.9, kind: CallSiteKind::Fetch, url_kind: UrlKind::Literal, .. }`（line 取 0）。
4. 落库时 `api_endpoints` provenance：在 `raw` JSON（现 `js_analysis_insert` 的 `raw`）加 `"ai_added": <bool>`；endpoint upsert 的 `params` 已经会并入 `ai_param_hints`（复用现有 `merged_params_for` 路径，把 `ai_param_hints` 视为内部来源的 `ParamHint`）。
5. summary 加 `by_source: {regex, ai}` 与 `ai_used: bool`。

**步骤（测试）：** 用一个 mock `LlmOneShot`（返回固定 JSON）注入 `JsExtractApisTool`，在 `dry_run=true` 下断言：
```rust
struct MockLlm(String);
#[async_trait::async_trait]
impl LlmOneShot for MockLlm {
    async fn complete(&self,_:&str,_:&str,_:Option<f64>,_:Option<u64>) -> anyhow::Result<String> { Ok(self.0.clone()) }
    async fn is_available(&self) -> bool { true }
}
```
   断言：AI 返回一个**锚得到**的端点 → 出现在结果且 `source=ai`；AI 返回一个**编造**端点 → 被丢弃；`ai=false` 时不调用 AI（用一个 panic-on-call 的 mock 验证不被调用）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest-app js_extract --status-level fail
cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings
```
预期 exit 0。

**提交：** `feat(pentest): AI-B hybrid extraction with hallucination guard + param`

---

## 阶段 2：AI-A（`browser_collect_js_api` 收集补全）

### 任务 2.1：AI-A 纯函数模块

**文件：** 创建 `backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_recipe.rs`；`mod.rs` 加 `mod js_ai_recipe;`

**步骤（先测试，红）：**

1. `needs_more`：
```rust
/// 依确定性收集结果 JSON 判断是否值得让 AI 补 recipe。
pub fn needs_more(result: &serde_json::Value) -> bool {
    let g = |k: &str| result.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let closure_complete = result.get("closure_complete").and_then(|v| v.as_bool()).unwrap_or(true);
    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("ok");
    !closure_complete
        || g("recursive_queue_remaining") > 0
        || g("ai_review_refs_total") > 0
        || matches!(status, "closure_partial" | "timeout_partial")
}
```
   测试：closure 未完/有 queue/有 ai_review_refs/partial 各 true；干净 ok 为 false。

2. `compact_signals(result) -> Value`：抽 `ai_assist.context.{ai_review_refs_sample, script_observations, recursive_errors_sample}` + 顶层计数，做成精简 payload（限大小）。测试：超量样本被截断到上限。

3. `sanitize_recipe(value, target_origin) -> Recipe`：仅保留同源 string（manifest_paths/script_urls/routes/click_texts）、合法 chunk_pairs（`id` 数字、`hash` hex），各有数量上限。测试：站外 URL 被剔除；非法 chunk_pair 被剔除。

4. `recipe_has_work(&Recipe) -> bool`。测试：空→false；任一非空→true。

**验证：** `cd backend && cargo nextest run -p golish-pentest-app js_ai_recipe --status-level fail`（全绿）。

**提交：** `feat(pentest): pure helpers for AI-A (needs_more/compact/sanitize)`

### 任务 2.2：把 AI-A 接入 `browser_collect_js_api::execute`

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`

**步骤：**

1. 把现有“spawn .mjs 一次 + 解析 stdout JSON”抽成内部 `async fn run_collector_once(&self, args_like, recipe: Option<&Value>) -> Result<Value>`（带 `--recipe-json` 当 recipe 存在）。第一遍 `recipe=None`（与现状等价）。
2. 第一遍后加有界 AI 循环：
```rust
let ai_on = args.get("ai").and_then(|v| v.as_bool()).unwrap_or(true);
const MAX_AI_ROUNDS: usize = 3;
let mut result = run_collector_once(&base, None).await?;
let mut rounds = 0usize;
let mut rationale: Vec<String> = Vec::new();
while ai_on && rounds < MAX_AI_ROUNDS && needs_more(&result) {
    let payload = compact_signals(&result);
    let Some(v) = call_llm_json(&self.llm, AI_RECIPE_SYSTEM, &payload.to_string(), 0.0, 2048).await else { break; };
    let needs = v.get("needs_second_pass").and_then(|x| x.as_bool()).unwrap_or(false);
    if let Some(r) = v.get("rationale").and_then(|x| x.as_str()) { rationale.push(r.to_string()); }
    let recipe = sanitize_recipe(v.get("recipe").cloned().unwrap_or(serde_json::json!({})), &parsed.origin().ascii_serialization());
    if !needs || !recipe_has_work(&recipe) { break; }
    result = run_collector_once(&base, Some(&serde_json::to_value(&recipe)?)).await?;
    rounds += 1;
}
```
3. `AI_RECIPE_SYSTEM`（沿用 probe §askDeepSeekForRecipe）：要求只建议同源 URL、不发明、不重复已抓/已失败、最小集合、可判 `needs_second_pass=false`，回 `{needs_second_pass, recipe, discard_refs, rationale}`。
4. 落库与现状一致（确定性 `source='crawler'`）；audit detail 加 `ai_recipe_rounds: rounds`、`ai_recipe_rationale: rationale`。
5. `.mjs` 不改（仍对 recipe 做同源/上限校验，二次防护）。

**步骤（测试）：** 由于 execute 依赖 spawn Node（重），仅对 §2.1 纯函数做单测（已覆盖判定/sanitize）；execute 级别用现有冒烟（`scripts/js_api_pipeline_test.mjs` 路径不在单测内）。新增一个单测：注入 panic-on-call mock + 构造一个“干净”收集结果，断言 `needs_more=false` 路径下 AI 不被调用（通过把循环判定抽为可测函数实现）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api js_ai_recipe --status-level fail
cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings
node --check scripts/browser_collect_js_api.mjs
```
预期 exit 0。

**提交：** `feat(pentest): AI-A bounded recipe second-pass inside browser collector`

---

## 阶段 3：降级、可观测、文档、清单

### 任务 3.1：降级与 provenance 回归

**文件：** 两工具 + 测试

**步骤：** 加单测：`llm=None`（无注入）时两工具行为与今天逐字节一致（纯确定性，不调 AI、summary `ai_used=false`）。

**验证：** `cd backend && cargo nextest run -p golish-pentest-app --status-level fail`（全绿）。

**提交：** `test(pentest): degrade-to-deterministic when no LLM provider`

### 任务 3.2：模块卡 + 清单 + 进度

**文件：** `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-js-analyzer.md`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`

**步骤：** 更新两张模块卡（新增 AI-A/AI-B、`LlmOneShot` 注入、provenance、降级）；`feature_list.json` 加条目 `jsapi-ai-tools`；`agent-progress.md` 记录本轮证据。

**验证：** `cd backend && jq empty ../feature_list.json`（或对应路径）+ 人工读卡。

**提交：** `docs(pentest): module cards + feature_list for jsapi AI tools`

### 任务 3.3：全量门禁

**步骤：**
```bash
cd backend && cargo nextest run -p golish-js-analyzer -p golish-pentest-app -p golish-agent-app -p golish-app-core --status-level fail
cd backend && cargo clippy -p golish-js-analyzer -p golish-pentest-app -p golish-agent-app -p golish-app-core --all-targets -- -D warnings
cd backend && cargo fmt --check
node --check scripts/browser_collect_js_api.mjs
```
全绿后视情况 `just precommit`（受本机 pnpm gate 影响，至少跑后端 scoped 验证并记录证据）。

**提交：** 无（验证步骤）。

---

## 自检（写完计划后对照设计 §1-§9）

- 规格覆盖：AI-A（设计 §3.1）→ P2；AI-B（§3.2）→ P1；vehicle（§3.3/§9-1）→ P0；provenance/I7-I8（§5）→ 1.3/2.2/3.1；退化（§8）→ 3.1；文档（§6）→ 3.2。无遗漏。
- 占位符：无 TODO/“后续实现”；每步有代码或精确命令。
- 类型一致：`LlmOneShot`(0.1)→注入(0.4)→`call_llm_json`(0.5)→AI-B(1.x)/AI-A(2.x) 名称一致；`EndpointSource`(1.1) 在 1.2/1.3 使用一致；`needs_more/sanitize_recipe/recipe_has_work`(2.1) 在 2.2 使用一致。

> 实现顺序：P0 → P1 → P2 → P3，逐任务红绿 + commit。开始实现前若 §9-1 之外出现新未知（如 `create_client_for_model` 签名细节），先读码再写，不猜。
