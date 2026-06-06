# Intel 阶段 AI 驱动 · P0（被动闭环接入 target_intel）实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现。每个 Task 独立 commit，TDD 优先（先写失败测试）。高风险/删 UI 不在 P0（P2 再做）。
>
> **设计文档：** `docs/design/2026-06-06-intel-stage-ai-driven-per-mode.md`（用户 2026-06-06 拍板 Option B：被动闭环→target_intel，主动→external_attack_surface/enumeration；Q1=渗透 `passive_intel=skip` 空跑过 gate；Q3=主动引擎加 tool_kinds 拆分→ P1）。

**目标：** 让 AI 在 harness 的 `target_intel` 阶段驱动同事的被动 recon 引擎（ENScan 子公司发现 + 0.zone/quake 字段富化），产出自动落账为 evidence，过 `coverage_complete` gate；渗透模式按 `intel_policy=skip` 跳过被动直奔主动。
**架构：** profile 新增 `intel_policy` 块作差异源；被动引擎（`run_providers_for_org` + `select_subsidiary_providers`/`select_enrichment_providers`）经 recon-app 新 facade 包成 2 个 agent 工具；工具产出在 `tool_execution/direct` 加 evidence_append 块自动落账；`synthesize_stage_subtask` 的 `K::TargetIntel` 分支按 policy 出指令。
**技术栈：** Rust（serde / sqlx / 既有 asset_intel 引擎 / 既有 harness gate）、cargo nextest。**P0 不动前端、不改 DB schema。**

---

## 文件结构（先锁分解）

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/profile.rs` | `IntelPolicy` / `PassiveIntelMode` 类型 + `Profile.intel_policy` 字段 | 改 |
| `resources/harness/profiles/{pentest,red_team,assessment,bug_bounty,cloud_assessment,smoke,assessment.sprint_skeleton}.json` | 各模式 `intel_policy` 配置 | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs` | 新 facade `run_passive_intel(pool, tools, org_id, phase, config) -> PassiveIntelSummary`（包 scan + select_* + run_providers_for_org）| 新建 |
| `backend/crates/golish-recon-app/src/asset_intel/mod.rs` | `pub use agent_intel::*`；把 `run_providers_for_org`/`select_*` 暴露给同 crate 新模块（已 `pub(crate)`，够用）| 改 |
| `backend/crates/golish-recon-app/src/agent_tools/mod.rs` | `ReconDiscoverSubsidiariesTool` / `ReconEnrichAssetsTool`（impl `golish_core::Tool`）| 新建 |
| `backend/crates/golish-recon-app/src/lib.rs` | `pub mod agent_tools;` | 改 |
| `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs` | `BridgeToolSelection` 加 `recon_passive: bool` | 改 |
| `backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs` | `BRIDGE_ROWS` 加两行工具说明 | 改 |
| `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`（或工具装配处）| 用 pool+ConfigManager 构造并注册两个工具 | 改 |
| `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs` | 新增 evidence_append 块：两个 recon 工具产出落账 | 改 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | `synthesize_stage_subtask` 的 `K::TargetIntel` 分支按 `intel_policy` 分流 + `intel_policy_for_ctx` | 改 |
| `feature_list.json` / `agent-progress.md` | 登记 feature + 收尾 | 改 |

> 依赖顺序：T1（类型）→ T2（配置）→ T3（facade）→ T4（工具）→ T5（注册）→ T6（证据落账）→ T7（prompt）→ T8（收口）。T1/T2/T7 可与 T3-6 并行。

---

## Task 1 · `IntelPolicy` 数据模型（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/profile.rs`

**步骤 1.1** 在 `ScopingPolicy` 定义之后、`Profile` 之前新增类型（与 `ScopingPolicy` 同款：容器级 `serde(default, deny_unknown_fields)`）：

```rust
/// target_intel 阶段的 per-profile 行为策略（设计 2026-06-06-intel-stage-ai-driven-per-mode §3.2）。
/// 全字段 serde default：旧 profile JSON 无此块时取保守默认（跑被动）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct IntelPolicy {
    /// 是否跑被动情报（run）还是跳过（skip，渗透：资产明确直奔主动）。
    pub passive_intel: PassiveIntelMode,
    /// 红队专用：先做 ENScan 子公司发现。
    pub discover_subsidiaries: bool,
    /// 字段富化（0.zone/quake/fofa…）。
    pub enrich_assets: bool,
}

/// 被动情报模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PassiveIntelMode {
    /// 跑被动收集（默认）。
    #[default]
    Run,
    /// 跳过被动（渗透：资产已在 scoping 确认）。
    Skip,
}

impl Default for IntelPolicy {
    /// 保守默认（旧 profile）：跑被动、不主动发现子公司、做富化。
    fn default() -> Self {
        Self {
            passive_intel: PassiveIntelMode::Run,
            discover_subsidiaries: false,
            enrich_assets: true,
        }
    }
}
```

**步骤 1.2** `Profile` 加字段（紧跟 `scoping_policy` 后）：

```rust
    /// target_intel 阶段 per-profile 策略（设计 2026-06-06）。缺省 = IntelPolicy::default()。
    #[serde(default)]
    pub intel_policy: IntelPolicy,
```

**步骤 1.3（测试）** 在 `profile.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
#[test]
fn intel_policy_defaults_when_absent() {
    let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
        "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
        "cleanup_required":false,"evidence_required":true}"#;
    let p = load_profile_from_json(json).expect("parse");
    assert_eq!(p.intel_policy.passive_intel, PassiveIntelMode::Run);
    assert!(!p.intel_policy.discover_subsidiaries);
    assert!(p.intel_policy.enrich_assets);
}

#[test]
fn intel_policy_parses_pentest_skip_and_red_team_full() {
    let pentest = r#"{"id":"pentest","display_name":"P","max_authorization":"controlled_exploit",
        "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
        "cleanup_required":false,"evidence_required":true,
        "intel_policy":{"passive_intel":"skip","discover_subsidiaries":false,"enrich_assets":false}}"#;
    let p = load_profile_from_json(pentest).expect("parse");
    assert_eq!(p.intel_policy.passive_intel, PassiveIntelMode::Skip);

    let red = r#"{"id":"red_team","display_name":"R","max_authorization":"post_exploit_red_team",
        "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
        "cleanup_required":true,"evidence_required":true,
        "intel_policy":{"passive_intel":"run","discover_subsidiaries":true,"enrich_assets":true}}"#;
    let r = load_profile_from_json(red).expect("parse");
    assert!(r.intel_policy.discover_subsidiaries);
    assert!(r.intel_policy.enrich_assets);
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit intel_policy --status-level fail   # 2 passed
cargo clippy -p golish-agent-kit --all-targets -- -D warnings
```
**提交：** `feat(harness): add IntelPolicy per-profile target_intel config`

---

## Task 2 · 7 个 profile JSON 加 `intel_policy`

**文件：** `resources/harness/profiles/*.json`（在 `evidence_required` 同级追加）。

- `pentest.json`：`{"passive_intel":"skip","discover_subsidiaries":false,"enrich_assets":false}`
- `red_team.json`：`{"passive_intel":"run","discover_subsidiaries":true,"enrich_assets":true}`
- `assessment.json` / `assessment.sprint_skeleton.json` / `bug_bounty.json` / `cloud_assessment.json`：`{"passive_intel":"run","discover_subsidiaries":false,"enrich_assets":true}`
- `smoke.json`：`{"passive_intel":"skip","discover_subsidiaries":false,"enrich_assets":false}`

**验证：**
```bash
for f in resources/harness/profiles/*.json; do python3 -m json.tool "$f" >/dev/null && echo "$f OK"; done
cd backend && cargo nextest run -p golish-agent-kit profile --status-level fail   # 嵌入 profile 加载测试全过
```
**提交：** `feat(harness): configure per-mode intel_policy in profiles`

---

## Task 3 · recon-app 被动 facade

**文件：** 新建 `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`；`asset_intel/mod.rs` 加 `mod agent_intel; pub use agent_intel::{run_passive_intel, PassiveIntelPhase, PassiveIntelSummary};`

**步骤 3.1** facade（包 scan_toolsconfig + select_* + run_providers_for_org，返回可序列化摘要供工具落账）：

```rust
//! Agent-facing facade over the passive asset-intel engine. Wraps the GUI
//! commands' (asset_intel_hydrate_subsidiaries / _enrich_organization) inner
//! pipeline so an agent Tool can drive it without the Tauri command layer.
use std::sync::Arc;
use serde::Serialize;
use uuid::Uuid;
use golish_app_core::GolishError;
use super::{select_enrichment_providers, select_subsidiary_providers, AssetIntelHydrateConfig};
use super::super::ToolsConfigState; // ToolsConfigState 在 recon-app 顶层（见 asset_intel/mod.rs 引用）

#[derive(Debug, Clone, Copy)]
pub enum PassiveIntelPhase { Subsidiaries, Enrich }

#[derive(Debug, Serialize)]
pub struct PassiveIntelSummary {
    pub run_id: String,
    pub company: String,
    pub phase: &'static str,
    pub status: String,             // Completed / Partial / Failed
    pub organizations: usize,
    pub targets: usize,
    pub providers: Vec<String>,     // 选中的 provider id
}

pub async fn run_passive_intel(
    pool: Arc<sqlx::PgPool>,
    tools: ToolsConfigState,
    organization_id: Uuid,
    phase: PassiveIntelPhase,
    config: AssetIntelHydrateConfig,
) -> Result<PassiveIntelSummary, GolishError> {
    let org = golish_db::repo::organizations::get_one(pool.as_ref(), organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {organization_id}")))?;
    let pentest_config = tools.0.get().await;
    let scan = golish_pentest::scan_toolsconfig_with_status(
        &pentest_config.toolsconfig_dir, pentest_config.tools_dir());
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error.unwrap_or_else(|| "toolsconfig scan failed".into())));
    }
    let selected = match phase {
        PassiveIntelPhase::Subsidiaries => select_subsidiary_providers(&scan.tools, &[])?,
        PassiveIntelPhase::Enrich => select_enrichment_providers(&scan.tools, &[])?,
    };
    let provider_ids: Vec<String> = selected.iter().map(|t| t.id.clone()).collect();
    let run = super::run_providers_for_org(
        None, pool.as_ref(), &pentest_config, &scan.tools, selected,
        &org, &org.name, &config,
    ).await?;
    Ok(PassiveIntelSummary {
        run_id: run.run_id,
        company: org.name,
        phase: match phase { PassiveIntelPhase::Subsidiaries => "subsidiaries", PassiveIntelPhase::Enrich => "enrich" },
        status: format!("{:?}", run.status),
        organizations: run.candidates.organizations.len(),
        targets: run.candidates.targets.len(),
        providers: provider_ids,
    })
}
```

> 执行期先 `Read` 确认：`ToolsConfigState` 的真实导入路径（`asset_intel/mod.rs:37` 定义在 `golish_recon_app::asset_intel::ToolsConfigState`，runner.rs 用的是 `crate::asset_intel::ToolsConfigState`）；`run_providers_for_org` 当前 `pub(crate)`，同 crate 调用 OK；`scan_toolsconfig_with_status` 与 runner.rs:89 用法一致。`select_enrichment_providers` 已 `pub(crate)`（capability.rs）。

**验证：**
```bash
cd backend && cargo check -p golish-recon-app
cargo clippy -p golish-recon-app --all-targets -- -D warnings
```
**提交：** `feat(recon): add run_passive_intel agent facade over asset-intel engine`

---

## Task 4 · 两个被动 agent 工具

**文件：** 新建 `backend/crates/golish-recon-app/src/agent_tools/mod.rs`；`lib.rs` 加 `pub mod agent_tools;`

**步骤 4.1** 仿 `golish-pentest-app/src/pentest_bridge/manage_organizations.rs`（`impl golish_core::Tool`，IDOR 绑 project_path，schema 抽成可单测自由函数）。两个工具持 `Arc<PgPool>` + `ToolsConfigState`，`execute` 解析 `organization_id`（或 `company` 先 `organizations::find_or_create`）、做 IDOR 校验（`organizations::get_one` + `project_path` 比对，照 manage_organizations.rs:262-265）、调 `run_passive_intel(...)`、返回 `serde_json::to_value(summary)`。`name()`：`recon_discover_subsidiaries` / `recon_enrich_assets`；前者 `phase=Subsidiaries` 后者 `Enrich`。结果 JSON 必须含 `"company"` 字段（T6 落账取 subject 用）。

**步骤 4.2（测试）** schema 纯函数测试（不依赖 DB）：断言 `parameters()` 含 `organization_id`/`company` 字段；`name()` 正确。

**验证：**
```bash
cd backend && cargo nextest run -p golish-recon-app recon_discover_subsidiaries recon_enrich_assets --status-level fail
cargo clippy -p golish-recon-app --all-targets -- -D warnings
```
**提交：** `feat(recon): add recon_discover_subsidiaries / recon_enrich_assets agent tools`

---

## Task 5 · 注册工具进 Task specialist 工具集

**文件：** `execution_mode/policy.rs`（`BridgeToolSelection` 加 `recon_passive: bool`，default true、smoke/最小集 false、`tool_names()` 推 `recon_discover_subsidiaries`+`recon_enrich_assets`）；`prompt_render.rs`（`BRIDGE_ROWS` 加两行）；工具装配处（`Grep "ManageOrganizationsTool::new" backend/crates/golish-agent-app` 找到实例化点，用同一 pool + ConfigManager 构造两个新工具加入 agent toolset）。

**步骤 5.1** 先 `Grep "ManageOrganizationsTool::new\|BridgeToolSelection" backend/crates/golish-agent-app backend/crates/golish-agent-runtime` 定位装配点；ConfigManager 句柄在 app 层（host 把 `Arc<ConfigManager>` 同时给 PentestState 与 recon asset-intel，见 asset_intel/mod.rs 注释 31-36）。

**步骤 5.2（测试）** 扩 `prompt_render_tests.rs`：断言工具表/名单含 `recon_discover_subsidiaries`、`recon_enrich_assets`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-runtime prompt_render --status-level fail
cargo clippy -p golish-agent-runtime --all-targets -- -D warnings
```
**提交：** `feat(runtime): expose recon passive tools to task specialists`

---

## Task 6 · 被动工具产出自动落账 evidence（过 gate 命门）

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`

**步骤 6.1** 在 `pentest_run` 落账块（约 L352-399）之后，加一段（这两个工具 `execute` 返回 JSON 非 stdout，故把整段结果序列化为 raw_output、`company` 作 subject）：

```rust
// recon passive tools (recon_discover_subsidiaries / recon_enrich_assets) return a
// JSON summary (not stdout). Book it to the ledger so target_intel coverage cells
// can cite a REAL evidence id (otherwise the passive-intel deliverable is "fabricated"
// and the gate loops). Mirrors the pentest_run block.
if matches!(effective_tool_name, "recon_discover_subsidiaries" | "recon_enrich_assets")
    && is_success
    && ctx.harness_stage.is_some()
{
    if let Some(tracker) = ctx.events.db_tracker {
        if let Some(repo) = tracker.repo() {
            let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
            let ev_subject = v.get("company").and_then(|c| c.as_str())
                .filter(|c| !c.is_empty()).unwrap_or(effective_tool_name);
            let ev_raw = serde_json::to_string(&v).unwrap_or_default();
            match repo.evidence_append(
                op_id, None, ctx.events.session_id, tracker.project_path(),
                effective_tool_name, effective_tool_name, ev_subject, &ev_raw,
            ).await {
                Ok(id) => { appended_evidence_id = Some(id);
                    tracing::info!(target: "harness::evidence", tool = %effective_tool_name,
                        evidence_id = id, "recon passive evidence appended"); }
                Err(e) => tracing::warn!(target: "harness::evidence", error = %e,
                    "recon passive evidence append failed (continuing)"),
            }
        }
    }
}
```

> 先 `Read` 该文件确认 `appended_evidence_id` / `v` / `ctx.events.db_tracker` / `tracker` 变量在该作用域可见（与 pentest_run 块同作用域）。

**步骤 6.2（测试）** 该路径依赖 PgPool，难纯单测；至少加注释化的 doc-test 或在 T8 集成里验证。本 Task 验证以 `cargo check` + clippy + T8 集成为准。

**验证：**
```bash
cd backend && cargo check -p golish-agent-runtime
cargo clippy -p golish-agent-runtime --all-targets -- -D warnings
```
**提交：** `feat(runtime): book recon passive tool output to evidence ledger`

---

## Task 7 · target_intel prompt 按 intel_policy 分流

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**步骤 7.1** 加 `intel_policy_for_ctx(exec_ctx)`（照 `scoping_policy_for_ctx` L1847-1854）。

**步骤 7.2** `synthesize_stage_subtask` 增 `intel_policy: &IntelPolicy` 参数（调用点 L627/L820 已有 `scoping_policy`，并排取 `intel_policy` 传入），`K::TargetIntel` 分支：

```rust
K::TargetIntel => {
    use crate::harness::profile::PassiveIntelMode;
    let mut steps = String::new();
    if matches!(intel_policy.passive_intel, PassiveIntelMode::Skip) {
        steps.push_str("Assets were confirmed during scoping; this engagement skips passive intel. Mark each expected intel technique coverage cell as not_applicable with a short note (\"assets confirmed in scoping; passive intel skipped per mode\"), then submit_stage_deliverable. Do NOT run passive providers.");
    } else {
        if intel_policy.discover_subsidiaries {
            steps.push_str("1) Call recon_discover_subsidiaries(company=<subject>) to enumerate subsidiary/affiliate organizations via the enterprise-intel provider; review results. ");
        }
        if intel_policy.enrich_assets {
            steps.push_str("2) Call recon_enrich_assets(organization_id=<confirmed org>) to passively collect domains/IPs/ICP/apps/emails. ");
        }
        steps.push_str("3) For each in-scope asset, give every expected intel technique (GOLISH-INTEL-DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT) a terminal coverage status citing the evidence ids the tools recorded, then submit_stage_deliverable. Do NOT perform active scanning here.");
    }
    ("Passive Target Intel",
     format!("Collect passive intelligence for `{target}`. {steps}"),
     "pentester")
}
```

**步骤 7.3（测试）** 断言文案随 policy 变化：`skip` → 含 "not_applicable"、不含 `recon_discover_subsidiaries`；红队（discover+enrich）→ 含 `recon_discover_subsidiaries`+`recon_enrich_assets`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit target_intel_prompt --status-level fail
cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail   # 回归
```
**提交：** `feat(harness): branch target_intel prompt by intel_policy`

---

## Task 8 · 收口（集成 + 全量验证 + 进度）

**步骤 8.1** 若引入跨 IPC 新类型，按 `docs/development.md` 跑 ts-rs 同步。P0 的 `IntelPolicy`/`PassiveIntelSummary` 暂不过 IPC（仅后端），无需同步；如后续前端要读，再加 `#[derive(ts_rs::TS)]`。
**步骤 8.2** 全量：`just precommit`（= check + test，必须全绿）。
**步骤 8.3** 更新 `agent-progress.md`（目标/已跑命令+输出/证据/风险/下一步）+ `feature_list.json`（feature `intel-stage-ai-driven-p0` 由 in_progress→passing 填 evidence；未全绿则 in_progress/blocked 写明）。
**步骤 8.4** 人工 smoke（`just dev`）：红队模式跑 target_intel → 应弹 `recon_discover_subsidiaries`/`recon_enrich_assets` 调用 → evidence 落账 → coverage gate PASS；渗透模式 → 跳过被动直接 external_attack_surface。
**提交：** `chore: wire intel-stage passive AI-driven end-to-end + progress`

---

## 自检（writing-plans）

**1. 规格覆盖度（对照设计 §3）：** §3.2 intel_policy→T1/T2；§3.3 工具→T3/T4/T5；§3.4-B 渗透 skip→T7；「过 gate evidence」→T6（落账）+T7（coverage 引证）；§3.5 prompt→T7。P0 不含主动工具（§10 P1）、不删前端（§10 P2）。
**2. 占位符扫描：** 各 Task 给真实签名/代码；「执行期 Read 确认变量名/导入路径」是改前定位（global-enforcement §7），非 TODO。
**3. 类型一致性：** `IntelPolicy`/`PassiveIntelMode`（T1）在 T7 引用一致；`run_passive_intel`/`PassiveIntelPhase`/`PassiveIntelSummary`（T3）在 T4 引用一致；工具名 `recon_discover_subsidiaries`/`recon_enrich_assets` 在 T4 定义、T5 注册、T6 落账、T7 prompt 四处一致；evidence 落账依赖工具结果含 `company` 字段（T4 产出 ↔ T6 取 subject）一致。

> P1（设计 §10）：`recon_active_surface`/`recon_port_scan` 拆 `run_active_collection`（加 tool_kinds）接 external_attack_surface/enumeration；ASN 工具；evidence 对齐各 stage gate。P2：移除前端 recon 按钮（需用户确认）。
