# Operation Harness 重建（re-anchor）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `executing-plans` 逐任务实现此计划，每个任务后跑该任务的「验证」命令，绿了再 commit，再进下一任务。

**目标：** 把 Phase 1 已落地的 `external_attack_surface` stage harness 从已删除的 SecurityView/recon 模型，重新锚定（re-anchor）到 target-centric 的 Target Surface Workbench 数据模型，并补做从未跑过的手动 E2E 验证。

**架构：** harness gate 逻辑本身与数据模型无关（只吃 `ExternalAttackSurfaceDeliverable` + Evidence Ledger）。re-anchor 的真实工作面很窄：① `resources/harness/evidence_kinds.json` 补 target-centric evidence kind；② 新增 `harness/surface_mapping.rs` 把 claim/finding kind 映射到 Surface Workbench tab（Surface/Sitemap/JS-API/Sensitive/Identity）；③ 把「stage done = Surface + JS-API 覆盖（Sitemap 软）」编码为一个新 gate check 并接进 5→6 check 流水线；④ 同步 backfill 关键词、e2e fixture；⑤ 翻 flag 做手动 E2E。**不移动** Evidence Ledger / harness crate 归属（分层已合法）。

**技术栈：** Rust 2021（`golish-agent-kit` L4.1 / `golish-pentest` L2.0）、`serde`、`cargo nextest`、配置驱动 `resources/harness/*.json`、Tauri 2 + React（仅 E2E 观测）。

---

## 决策锚点（design doc 2026-06-01 §11，已由用户授权拍板）

| # | 决策 | 取值 | 影响本计划 |
|---|---|---|---|
| D1 | 现有 Phase 1 代码处置 | **A · re-anchor**（保留代码，解旧引用 + 重映射） | 不重写；只做 Task 1–6 |
| D2 | MVP stage "done" 判定 | **A 精练版**：Surface + JS-API 为硬要求；Sitemap 为软要求（honest-empty 允许，须显式 skip） | Task 2/3 的 `D2_REQUIRED_CATEGORIES` |
| D3 | 集成点 | **A · 保留 `execute.rs` 末端 hook**，与 legacy-bridge 清理对齐，最小改动 | Task 3 只动 gate 流水线，不改 orchestrator 签名 |
| D4 | 验证策略 | **A · validation-first**：不加新 stage，先把 E2E 跑通 | Task 7 = 手动 E2E runbook，无新 stage |
| 文档位置 | `docs/design/` + `docs/superpowers/plans/` | OK | — |

**D2 精练理由（磁盘实证）**：`golish-agent-app/src/ai/db_bridge/recon.rs::query_target_data_impl` 实际返回 `assets / endpoints / fingerprints / js_analysis / scan_logs`——Surface（fingerprints/services）与 JS-API（js_analysis/endpoints）都有确定的后端数据源，可作硬要求；而 `2026-05-28-target-surface-workbench.md` §8.6 明确 Sitemap/Sensitive tab 当前是 "honest empty/loading state if no backend data"，所以 Sitemap 设为软要求（允许 honest-empty，但必须落进 `skipped_checks` 区分"已检查为空 vs 未检查"，对齐 AGENTS.md I8）。

---

## 文件结构（创建/修改清单）

| 文件 | 职责 | 动作 |
|---|---|---|
| `resources/harness/evidence_kinds.json` | evidence aging 注册表 | 修改：补 6 个 target-centric kind |
| `backend/crates/golish-agent-kit/src/harness/surface_mapping.rs` | claim/finding kind → Surface Workbench category 纯映射 + 覆盖计算 | **新建** |
| `backend/crates/golish-agent-kit/src/harness/mod.rs` | module 声明 + re-export | 修改：加 `pub mod surface_mapping` + re-export |
| `backend/crates/golish-agent-kit/src/harness/gate/surface_coverage_check.rs` | 第 6 个 gate check：D2 覆盖判定 | **新建** |
| `backend/crates/golish-agent-kit/src/harness/gate/mod.rs` | gate 流水线调度 | 修改：5→6 check |
| `backend/crates/golish-agent-kit/src/harness/e2e_tests.rs` | e2e fixture | 修改：`happy_deliverable` 补 1 个 JS-API finding |
| `backend/crates/golish-agent-kit/src/task_orchestrator/harness_backfill.rs` | stage 关键词 backfill | 修改：补 JS/API/sitemap 关键词（可选，conditional） |
| `resources/harness/stages/external_attack_surface.json` | stage spec | 修改：`required_checks` 加 `surface_workbench_coverage` |

> 任务顺序保证每一步独立可编译可测；硬要求（Task 1–3 + 6）先行，软增强（Task 4–5）次之，E2E（Task 7）最后。

---

## Task 1 · 扩展 evidence_kinds.json（target-centric kind）

**文件：** `resources/harness/evidence_kinds.json`

**背景：** `EvidenceKindRegistry`（`golish-pentest/src/evidence_kinds.rs`）把本 JSON 解析为开放 map，跳过 `$` 前缀键，未注册 kind 走 7 天 fallback——所以**加键安全**，不破坏既有 8 kind 断言。

**步骤：** 在现有 8 个 kind 后追加 6 个 target-centric kind，并更新 `$comment`：

```json
{
  "$schema": "../../docs/design/2026-05-26-evidence-ledger-on-existing-audit-log.md#6.1",
  "$comment": "Evidence kind aging registry (Doc 1 §6.1). 2026-06-01 re-anchor: 追加 target-centric kind (target_asset/fingerprint/api_endpoint/js_analysis/sitemap/sensitive_exposure) 对齐 Target Surface Workbench (2026-05-28)。stage_spec.override -> evidence_kinds.json default -> 7 days fallback。改阈值=改 PR。",

  "dns_a": { "default_max_age_secs": 86400 },
  "dns_aaaa": { "default_max_age_secs": 86400 },
  "ct_log": { "default_max_age_secs": 604800 },
  "cve_feed": { "default_max_age_secs": 86400 },
  "nmap": { "default_max_age_secs": 259200 },
  "http_probe": { "default_max_age_secs": 21600 },
  "shodan_query": { "default_max_age_secs": 3600 },
  "whois": { "default_max_age_secs": 2592000 },

  "target_asset": { "default_max_age_secs": 86400 },
  "fingerprint": { "default_max_age_secs": 86400 },
  "api_endpoint": { "default_max_age_secs": 86400 },
  "js_analysis": { "default_max_age_secs": 86400 },
  "sitemap": { "default_max_age_secs": 86400 },
  "sensitive_exposure": { "default_max_age_secs": 43200 }
}
```

**验证：**
```bash
python3 -m json.tool resources/harness/evidence_kinds.json >/dev/null && echo JSON_OK
cargo nextest run -p golish-pentest -E 'test(evidence_kinds)'
# 预期：from_json/singleton/registered_kinds 全绿（len 现为 14，旧 8 kind 断言仍在）
```

**提交：** `feat(harness): add target-centric evidence kinds for Surface Workbench re-anchor`

---

## Task 2 · 新建 surface_mapping.rs（纯映射 + 覆盖计算）

**文件：** `backend/crates/golish-agent-kit/src/harness/surface_mapping.rs`（新建）

**步骤：** 写如下完整模块。`SurfaceCategory` 用关键词把 claim/finding `kind` 映射到 Surface Workbench tab；`SurfaceCoverage` 聚合 deliverable 覆盖了哪些 category；`D2_REQUIRED_CATEGORIES` 编码 D2 硬要求。

```rust
//! Surface Workbench category 映射（2026-06-01 re-anchor）。
//!
//! 把 external_attack_surface stage 的 "done" 判定（design doc 2026-06-01 §D2）
//! 重新锚定到 pivot 后的 Target Surface Workbench 数据模型
//! (docs/design/2026-05-28-target-surface-workbench.md)：deliverable 的
//! claim/finding kind 关键词映射到 Surface Workbench tab，gate 据此推理覆盖度，
//! 而非已删除的 SecurityView 模型。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::types::ExternalAttackSurfaceDeliverable;

/// Target Surface Workbench tab 分类（external_attack_surface stage 可产证据的子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceCategory {
    /// `Identity` tab：host/IP/DNS/ASN/CDN 解析。
    Identity,
    /// `Surface` tab：端口/服务/HTTP 探测/指纹。
    Surface,
    /// `Sitemap` tab：robots/sitemap.xml/爬虫路径。
    Sitemap,
    /// `JS / API` tab：JS 文件、source map、抽取的 API 端点。
    JsApi,
    /// `Sensitive` tab：密钥/泄漏暴露。
    Sensitive,
}

impl SurfaceCategory {
    /// 把 claim/finding `kind` 字符串（小写关键词匹配）映射到 Surface Workbench
    /// category。落在 surface 分类法之外的 kind 返回 None。
    ///
    /// 顺序敏感：更专的 category（Sitemap/JsApi/Sensitive）先判，避免 generic
    /// 关键词（如 "path" 命中 endpoint）误吞。
    pub fn from_kind(kind: &str) -> Option<Self> {
        let k = kind.to_lowercase();
        if contains_any(&k, &["sitemap", "robots", "crawl", "site_path", "path_discovery"]) {
            return Some(Self::Sitemap);
        }
        if contains_any(
            &k,
            &["js_", "javascript", "api_endpoint", "endpoint", "api_route", "source_map", "sourcemap"],
        ) {
            return Some(Self::JsApi);
        }
        if contains_any(
            &k,
            &["secret", "leak", "sensitive", "exposure", "credential", "token_exposure"],
        ) {
            return Some(Self::Sensitive);
        }
        if contains_any(
            &k,
            &["port", "service", "http", "fingerprint", "tech_stack", "banner", "tls", "subdomain"],
        ) {
            return Some(Self::Surface);
        }
        if contains_any(&k, &["dns", "asn", "cdn", "whois", "ip_resolution", "identity"]) {
            return Some(Self::Identity);
        }
        None
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// deliverable 的 claim+finding 触及了哪些 Surface Workbench category。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SurfaceCoverage {
    pub categories: BTreeSet<SurfaceCategory>,
}

impl SurfaceCoverage {
    pub fn from_deliverable(d: &ExternalAttackSurfaceDeliverable) -> Self {
        let mut categories = BTreeSet::new();
        for c in &d.claims {
            if let Some(cat) = SurfaceCategory::from_kind(&c.kind) {
                categories.insert(cat);
            }
        }
        for f in &d.findings {
            if let Some(cat) = SurfaceCategory::from_kind(&f.kind) {
                categories.insert(cat);
            }
        }
        Self { categories }
    }

    pub fn covers(&self, cat: SurfaceCategory) -> bool {
        self.categories.contains(&cat)
    }
}

/// D2 硬要求 category（design doc 2026-06-01 §D2 option A 精练版）：
/// Surface + JsApi 必须有证据覆盖（后端数据源经 query_target_data 确实存在：
/// fingerprints/endpoints/js_analysis）。
pub const D2_REQUIRED_CATEGORIES: &[SurfaceCategory] =
    &[SurfaceCategory::Surface, SurfaceCategory::JsApi];

/// D2 软要求 category（属 D2 意图但当前无保证后端数据源，允许 honest-empty）。
pub const D2_SOFT_CATEGORIES: &[SurfaceCategory] = &[SurfaceCategory::Sitemap];

/// 返回 deliverable 未覆盖的硬要求 category 列表（空 = 满足 D2 硬门槛）。
pub fn missing_required_categories(
    d: &ExternalAttackSurfaceDeliverable,
) -> Vec<SurfaceCategory> {
    let cov = SurfaceCoverage::from_deliverable(d);
    D2_REQUIRED_CATEGORIES
        .iter()
        .copied()
        .filter(|c| !cov.covers(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{FindingSeverity, HarnessFinding, StageClaim};
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    #[test]
    fn from_kind_maps_known_kinds() {
        assert_eq!(SurfaceCategory::from_kind("http_service"), Some(SurfaceCategory::Surface));
        assert_eq!(SurfaceCategory::from_kind("subdomain"), Some(SurfaceCategory::Surface));
        assert_eq!(SurfaceCategory::from_kind("fingerprint"), Some(SurfaceCategory::Surface));
        assert_eq!(SurfaceCategory::from_kind("api_endpoint"), Some(SurfaceCategory::JsApi));
        assert_eq!(SurfaceCategory::from_kind("js_secret"), Some(SurfaceCategory::JsApi));
        assert_eq!(SurfaceCategory::from_kind("sitemap_path"), Some(SurfaceCategory::Sitemap));
        assert_eq!(SurfaceCategory::from_kind("sensitive_exposure"), Some(SurfaceCategory::Sensitive));
        assert_eq!(SurfaceCategory::from_kind("dns_a"), Some(SurfaceCategory::Identity));
    }

    #[test]
    fn from_kind_unknown_returns_none() {
        assert_eq!(SurfaceCategory::from_kind("billing_refactor"), None);
        assert_eq!(SurfaceCategory::from_kind(""), None);
    }

    fn finding(kind: &str) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        }
    }

    fn deliverable_with(findings: Vec<HarnessFinding>) -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
        }
    }

    #[test]
    fn coverage_collects_distinct_categories() {
        let d = deliverable_with(vec![finding("http_service"), finding("api_endpoint")]);
        let cov = SurfaceCoverage::from_deliverable(&d);
        assert!(cov.covers(SurfaceCategory::Surface));
        assert!(cov.covers(SurfaceCategory::JsApi));
        assert!(!cov.covers(SurfaceCategory::Sitemap));
    }

    #[test]
    fn missing_required_when_only_surface_present() {
        let d = deliverable_with(vec![finding("http_service")]);
        let missing = missing_required_categories(&d);
        assert_eq!(missing, vec![SurfaceCategory::JsApi]);
    }

    #[test]
    fn no_missing_when_surface_and_jsapi_present() {
        let mut d = deliverable_with(vec![finding("http_service")]);
        d.claims.push(StageClaim {
            kind: "api_endpoint_observed".to_string(),
            subject: "api.example.com/v1".to_string(),
            summary: "GET 200".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
        });
        assert!(missing_required_categories(&d).is_empty());
    }
}
```

**验证：**
```bash
cargo nextest run -p golish-agent-kit -E 'test(harness::surface_mapping)'
# 预期：6 测全绿
```

**提交：** `feat(harness): add Surface Workbench category mapping module`

---

## Task 3 · 接进 mod.rs（声明 + re-export）

**文件：** `backend/crates/golish-agent-kit/src/harness/mod.rs`

**步骤 1：** 在 `pub mod stage_spec;` 后加：
```rust
pub mod surface_mapping;
```

**步骤 2：** 在 `pub use stage_spec::{...};` 后加：
```rust
pub use surface_mapping::{
    missing_required_categories, SurfaceCategory, SurfaceCoverage, D2_REQUIRED_CATEGORIES,
    D2_SOFT_CATEGORIES,
};
```

**验证：**
```bash
cargo nextest run -p golish-agent-kit -E 'test(harness::surface_mapping)'
# 预期：通过 re-export 路径仍可解析
```

**提交：** `feat(harness): export surface_mapping from harness module`

---

## Task 4 · 新建第 6 个 gate check：surface_coverage_check.rs

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/surface_coverage_check.rs`（新建）

**步骤：** D2 判定——缺硬要求 category 则 Block，并把 Sitemap 软要求做成「未覆盖且未显式 skip → 仅 hint 不 block」。

```rust
//! surface_coverage_check（2026-06-01 re-anchor · design doc §D2）。
//!
//! 把 stage "done" 判定锚到 Target Surface Workbench：
//!   - 硬要求 D2_REQUIRED_CATEGORIES (Surface + JsApi) 未覆盖 → Block
//!   - 软要求 Sitemap 未覆盖且未在 skipped_checks 显式声明 → 仅 hint（不 block）
//!
//! 空 deliverable 由 vacuous_check 先拦；本 check 只在有内容时判覆盖度。

use super::super::surface_mapping::{missing_required_categories, SurfaceCategory};
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable) -> GateCheckOutcome {
    // 空 deliverable 留给 vacuous_check；这里不重复拦。
    if deliverable.claims.is_empty() && deliverable.findings.is_empty() {
        return GateCheckOutcome::Pass;
    }

    let missing = missing_required_categories(deliverable);
    let sitemap_skipped = deliverable
        .skipped_checks
        .iter()
        .any(|s| s.check.to_lowercase().contains("sitemap"));
    let cov = super::super::surface_mapping::SurfaceCoverage::from_deliverable(deliverable);
    let sitemap_soft_gap = !cov.covers(SurfaceCategory::Sitemap) && !sitemap_skipped;

    if missing.is_empty() {
        if sitemap_soft_gap {
            tracing::info!(
                target: "harness::gate::surface_coverage_check",
                stage_id = %deliverable.stage_id,
                "surface_coverage pass (sitemap soft-gap noted)"
            );
        }
        return GateCheckOutcome::Pass;
    }

    let reasons: Vec<String> = missing
        .iter()
        .map(|c| {
            format!(
                "surface coverage gap: required Surface Workbench category {:?} has no evidence-backed claim/finding",
                c
            )
        })
        .collect();

    let mut recovery = HarnessRecoveryActions::default();
    for c in &missing {
        match c {
            SurfaceCategory::Surface => {
                recovery.hints.push(
                    "run http_probe / fingerprint_target to produce Surface (ports/services/fingerprints) evidence".to_string(),
                );
                recovery.repair_tool_calls.push("http_probe".to_string());
                recovery.missing_evidence_kinds.push("fingerprint".to_string());
            }
            SurfaceCategory::JsApi => {
                recovery.hints.push(
                    "collect JS + extract API endpoints to produce JS/API evidence".to_string(),
                );
                recovery.repair_tool_calls.push("query_target_data".to_string());
                recovery.missing_evidence_kinds.push("api_endpoint".to_string());
            }
            _ => {}
        }
    }
    if sitemap_soft_gap {
        recovery.hints.push(
            "Sitemap tab empty: either crawl for sitemap evidence OR add an explicit skipped_checks entry (checked-empty != unchecked, AGENTS.md I8)".to_string(),
        );
    }

    tracing::warn!(
        target: "harness::gate::surface_coverage_check",
        stage_id = %deliverable.stage_id,
        missing = ?missing,
        "surface_coverage block"
    );
    GateCheckOutcome::Block { reasons, recovery }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{FindingSeverity, HarnessFinding, SkippedCheckRecord};
    use golish_pentest::evidence_ledger::{EvidenceAuditId, SkipReason};
    use uuid::Uuid;

    fn finding(kind: &str) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        }
    }

    fn deliverable(findings: Vec<HarnessFinding>) -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
        }
    }

    #[test]
    fn empty_deliverable_passes_here() {
        let d = deliverable(vec![]);
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn surface_plus_jsapi_passes() {
        let d = deliverable(vec![finding("http_service"), finding("api_endpoint")]);
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn only_surface_blocks_on_missing_jsapi() {
        let d = deliverable(vec![finding("http_service")]);
        match run(&d) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons.iter().any(|r| r.contains("JsApi")));
                assert!(recovery.repair_tool_calls.iter().any(|c| c == "query_target_data"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn sitemap_explicit_skip_does_not_add_soft_hint_blocking() {
        let mut d = deliverable(vec![finding("http_service"), finding("api_endpoint")]);
        d.skipped_checks.push(SkippedCheckRecord {
            check: "sitemap_crawl".to_string(),
            reason: SkipReason::Other {
                explanation: "no robots.txt / sitemap.xml present".to_string(),
                evidence_ref: EvidenceAuditId::new(1),
            },
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }
}
```

**验证：**
```bash
cargo nextest run -p golish-agent-kit -E 'test(harness::gate::surface_coverage_check)'
# 预期：4 测全绿
```

**提交：** `feat(harness): add surface_coverage_check gate (D2 Surface Workbench done criterion)`

---

## Task 5 · 接进 gate 流水线（5→6 check）+ 更新 e2e fixture

**文件 A：** `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`

**步骤 A1：** 在 `pub mod scope_check;` 区块加：
```rust
pub mod surface_coverage_check;
```

**步骤 A2：** 在 `validate_external_attack_surface_gate` 的 check 数组里，把 `vacuous_check::run(...)` 之后插入新 check：
```rust
    for outcome in [
        schema_check::run(deliverable, spec),
        scope_check::run(deliverable),
        contract_check::run(deliverable, contract),
        vacuous_check::run(deliverable, spec),
        surface_coverage_check::run(deliverable),
        freshness_check::run(deliverable, spec),
    ] {
```

**文件 B：** `backend/crates/golish-agent-kit/src/harness/e2e_tests.rs`

**步骤 B1：** `happy_deliverable` 当前只有 `subdomain` + `http_service`（都映射 Surface），新 check 会因缺 JS-API 而 Block，导致 `e2e_happy_path_external_attack_surface_passes_gate` 红。在 `happy_deliverable` 的 `d.findings.push(http_service...)` 之后追加一个 JS-API finding：
```rust
    d.findings.push(HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "api_endpoint".to_string(),
        subject: "api.example.com/v1/login".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![http_eid],
    });
```

**步骤 B2：** 检查 `e2e_contract_check_below_min_subdomain_blocks` 等用例：它们 `retain(|f| f.kind != "subdomain")` 后仍保留 http_service + api_endpoint，contract_check 只看 subdomain/http_service 计数，新增 api_endpoint finding 不在 skeleton 期望集合内（contract_check 只校验已知 kind 的 range，未知 kind 不计），故不受影响。若 `contract_check` 对未知 kind 报错，则改为在该用例局部移除 api_endpoint——执行时按真实报错决定。

**验证：**
```bash
cargo nextest run -p golish-agent-kit -E 'test(harness)'
# 预期：原 88 + 本轮新增 (surface_mapping 6 + surface_coverage 4) 全绿；e2e 10 全绿
```

**提交：** `feat(harness): wire surface_coverage into gate pipeline; update e2e happy fixture`

---

## Task 6 · stage spec 记录新 check（配置驱动留痕）

**文件：** `resources/harness/stages/external_attack_surface.json`

**步骤：** `required_checks` 数组追加 `"surface_workbench_coverage"`（文档/可观测用途；gate 以代码流水线为准，但 spec 留痕保持配置一致）：
```json
  "required_checks": [
    "scope_status_present",
    "evidence_non_empty",
    "unchecked_distinct_from_checked_empty",
    "out_of_scope_targets_excluded",
    "min_tool_invocations_per_check",
    "surface_workbench_coverage"
  ],
```

**注意：** `stage_spec.rs` 测试 `external_attack_surface_required_checks_count` 断言 `len()==5`，需同步改为 `6`：
```rust
        assert_eq!(s.required_checks.len(), 6);
```

**验证：**
```bash
python3 -m json.tool resources/harness/stages/external_attack_surface.json >/dev/null && echo OK
cargo nextest run -p golish-agent-kit -E 'test(harness::stage_spec)'
```

**提交：** `feat(harness): record surface_workbench_coverage in stage spec required_checks`

---

## Task 7 · 手动 E2E（D4 validation-first · 从未做过 · 需运行时）

> 此任务**需要 `just dev` 跑起 Tauri app + 人工观测**，无法纯自动化产出证据。executing-plans 工作者须在真实运行环境执行并把截图/日志贴进 `agent-progress.md`「已记录证据」。

**前置：**
```bash
just kill            # 清 1420 端口
GOLISH_HARNESS_STAGE_MODE=true just dev
```

**步骤：**
1. 新建/选一个 target（如 `portal.example.com`），进入 Target Surface Workbench。
2. 新建 task 模式 task：`评估 portal.example.com 的 external attack surface，列出端口/服务/指纹与 JS/API`。
3. 观测 ① 生成的 subtask 被 `harness_backfill` 打上 `external_attack_surface`（日志 `target=harness::backfill`）；② agent 调 `http_probe`/`fingerprint_target`/`query_target_data`，evidence 落 audit_log；③ agent 交 `ExternalAttackSurfaceDeliverable` JSON；④ content 末尾出现 `## Harness Gate Decision` JSON（日志 `target=harness::hook` PASS）。
4. **反例**：构造一个不调任何工具、claims/findings 为空的 deliverable → gate 必须 `BLOCK` + `recovery_actions` 非空（日志 `gate decision: BLOCK`）。
5. **D2 反例**：交一个只有 Surface finding、无 JS-API、且未 skip sitemap 的 deliverable → `surface_coverage_check` Block，reason 含 `JsApi`。

**验证（证据要求）：** 把 4 段日志/截图（backfill / evidence / gate PASS / gate BLOCK+recovery）贴进 `agent-progress.md`。无新鲜证据不得把 feature 标 passing（AGENTS.md §3）。

**提交：** 无代码改动则不 commit；E2E 证据更新 `agent-progress.md` 后 commit `docs(harness): record manual E2E evidence for stage_mode`。

---

## Task 8 · 收敛（precommit + feature_list + 文档）

**步骤：**
```bash
just precommit       # = check + test 全绿才允许 commit
```
- `feature_list.json`：把 `harness-rebuild-2026-06-01` 条目按 D4 进度更新（P1 代码绿 → 但 E2E 未做前仍 `in_progress`，E2E 证据齐后才 `passing`）。
- design doc 2026-06-01：`Status` 已在本轮转 Approved；E2E 完成后追加「P2 验证证据」段。

**提交：** `chore(harness): converge harness-rebuild feature_list + progress`

---

## 自检（writing-plans §自检）

1. **规格覆盖度**：design doc §3 D1（Task 1–6 re-anchor 不重写 ✓）、§3 D2（Task 2/3/4 编码覆盖判定 ✓）、§3 D3（Task 5 仅动 gate 不改 orchestrator 签名 ✓）、§3 D4（Task 7 validation-first 无新 stage ✓）、§6 C3 重映射数据源（Task 1+2 ✓）、§7 测试策略回归+新增（Task 5 验证含原 88 ✓）。
2. **占位符扫描**：无 TODO/待定；每个代码步骤含完整代码块。
3. **类型一致性**：`SurfaceCategory`/`SurfaceCoverage`/`missing_required_categories`/`D2_REQUIRED_CATEGORIES` 在 Task 2 定义，Task 3 re-export，Task 4 消费，命名一致；`ExternalAttackSurfaceDeliverable`/`HarnessFinding`/`StageClaim`/`SkippedCheckRecord`/`SkipReason`/`GateCheckOutcome`/`HarnessRecoveryActions` 均为既有类型（已对照 `types.rs`/`gate/mod.rs`/`evidence_ledger`）。

## 风险与回滚

| 风险 | 缓解 |
|---|---|
| 新 gate check 破坏既有 e2e | Task 5 B1 同步更新 happy fixture；Task 2/4 用例先证逻辑 |
| evidence_kinds.json 破坏 registry | Task 1 已核 `EvidenceKindRegistry` 为开放 map，加键安全 |
| contract_check 对新 finding kind 报错 | Task 5 B2 给出 fallback：局部移除 api_endpoint |
| legacy-bridge 清理与 hook 冲突 | D3 保留 hook、不改 orchestrator 签名，冲突面最小 |
| Sitemap 硬要求导致 gate 不可过 | D2 精练：Sitemap 软要求 + honest-empty skip 通道 |

回滚：本计划全部改动可按 task 粒度 `git revert`；flag 默认 OFF，未翻 flag 前对生产路径零影响。
