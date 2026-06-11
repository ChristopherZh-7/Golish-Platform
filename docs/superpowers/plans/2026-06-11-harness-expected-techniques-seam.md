# Harness `expected_techniques` 动态注入 seam 激活 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `executing-plans` 逐任务实现此计划。每个任务先写失败测试（TDD），看它失败，再写最小实现，再验证，再 commit。

**目标：** 把 stage gate 的 `expected_techniques`（覆盖矩阵的"期望技术分母"）从写死的静态 `spec.expected_techniques` 升级为按**真实 in-scope 资产类型动态生成**，激活早已预埋但未接线的 ③ seam。

**架构：** 已有的注入链路是 `GateContext.expected_techniques`（优先） > `StageSkeleton.expected_techniques` > `spec.expected_techniques`（静态回退），由 `validate_stage_gate_with_context` 合并。本计划新增一个纯函数 `technique_resolver`（按 `StageKind` + 资产类型集产出技术清单），让 `DefaultSprintContractGenerator` 填充 `skeleton.expected_techniques`，并在 live gate hook 把它接通。gate 仍是纯函数 / DB-free。

**技术栈：** Rust 2021、`golish-agent-kit`（harness）、`serde`、`cargo nextest`、`cargo clippy -D warnings`。

---

## 0. 背景与现状勘查（动手前先读真实源码）

> 先读仓根 `AGENTS.md`（开工流程 / 不变量 / 完成定义）与 `docs/design/2026-06-05-coverage-matrix.md` §6.5、`docs/design/2026-06-05-vuln-triage-technique-matrix.md` §5.5。

**覆盖矩阵机制**：每个 stage 在 `resources/harness/stages/<stage>.json` 声明 `expected_techniques`（一组 `GOLISH-*` / `WSTG-*` id）。gate 的 `coverage_complete` op（`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs::coverage_complete`）对 **每个 in-scope 资产 × 每类期望技术** 核是否有终态 `CoverageCell`，缺口即 Block。

**已存在的 seam（读这几处确认）**：
- `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`：`GateContext { in_scope_assets: Option<Vec<String>>, expected_techniques: Option<Vec<String>> }`；`coverage_complete` 里 `let techniques = ctx.expected_techniques.as_deref().unwrap_or(&spec.expected_techniques);`。
- `backend/crates/golish-agent-kit/src/harness/gate/mod.rs::validate_stage_gate_with_context`：把有效期望技术算成 `ctx.expected_techniques.clone().or_else(|| skeleton.map(|s| s.expected_techniques.clone()).filter(|t| !t.is_empty()))`——**即 skeleton 路与 ctx 路二选一已经接好**。
- `backend/crates/golish-agent-kit/src/harness/sprint_contract.rs`：`StageSkeleton.expected_techniques: Vec<String>`（`#[serde(default)]`，静态 JSON 现为空）。
- `backend/crates/golish-agent-kit/src/harness/stage_harness.rs`：`StageHarness.skeleton: Option<StageSkeleton>` 透传给 `validate_stage_gate_with_context`。

**缺口（P3 要补的）**：
1. `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs::apply_harness_gate_hook` 里 `let gate_ctx = GateContext { in_scope_assets, expected_techniques: None };`——③ 路写死 `None`。
2. `DefaultSprintContractGenerator`（`sprint_contract.rs`）不产出 `expected_techniques`（恒空）。
3. 没有"按资产类型 → 技术清单"的解析器。

**上游依赖与分期**：
- `in_scope_assets` 注入**已 live**（见 `agent-progress.md` 2026-06-06 P1a：`execute.rs::fetch_in_scope_assets_for_gate` → `repo.in_scope_assets(org_id)` → `targets` 表 `scope='in'`）。
- `targets` 表已有 `type` 字段（domain / ip / url / cidr …，见 `backend/crates/golish-db/src/repo/targets.rs` 与 `golish-pentest-domain` 的 `TargetType`）——**这是本计划 Phase A 能用的动态输入**。
- **Phase B（本计划不实现，仅记录）**：更细的"按服务指纹/技术栈"变化（如静态站点免 `GOLISH-ENUM-PARAM`）需要 EAS 把 per-asset 服务指纹落库，属未来增量；本计划只做到"按资产类型"。

---

## 1. 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs` | 纯函数：`(StageKind, &[AssetClass]) -> Vec<String>`，按 stage + 资产类型集产出期望技术清单。唯一动态决策点。 | 新增 |
| `backend/crates/golish-agent-kit/src/harness/mod.rs` | `pub mod technique_resolver;` + re-export | 改 |
| `backend/crates/golish-agent-kit/src/harness/sprint_contract.rs` | `DefaultSprintContractGenerator` 调 resolver 填 `skeleton.expected_techniques`；新增 `generate_with_assets` 入口透传资产类型 | 改 |
| `backend/crates/golish-agent-kit/src/db_traits/repo.rs` | `in_scope_target_types(org_id) -> Vec<String>`（default 返回空，test double 零适配） | 改 |
| `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs` | impl `in_scope_target_types` 透传到 `targets` 端口 | 改 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | gate hook 用 resolver 计算 techniques 注入 `gate_ctx.expected_techniques` | 改 |
| `resources/harness/technique_taxonomy.json` | 复用已有词典做 resolver 输出的 id 合法性自检（测试期 fail-closed） | 只读引用 |

---

## 任务 1 · 新增 `technique_resolver` 纯函数（按资产类型产期望技术）

**文件：** 新增 `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`；改 `backend/crates/golish-agent-kit/src/harness/mod.rs`

### 步骤 1.1 — 先写失败测试

在 `technique_resolver.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::StageKind;

    #[test]
    fn target_intel_returns_all_intel_techniques() {
        // 任意资产类型，target_intel 都核全部 6 类被动情报技术。
        let t = resolve_expected_techniques(StageKind::TargetIntel, &[AssetClass::Domain]);
        assert!(t.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(t.contains(&"GOLISH-INTEL-WHOIS".to_string()));
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn enumeration_drops_param_when_no_web_asset() {
        // 纯 IP（无 web）资产：内容枚举不要求 PARAM（参数发现对非 web 无意义）。
        let ip_only = resolve_expected_techniques(StageKind::Enumeration, &[AssetClass::Ip]);
        assert!(!ip_only.contains(&"GOLISH-ENUM-PARAM".to_string()));
        // 有 web 资产时 PARAM 回来。
        let web = resolve_expected_techniques(StageKind::Enumeration, &[AssetClass::Url]);
        assert!(web.contains(&"GOLISH-ENUM-PARAM".to_string()));
    }

    #[test]
    fn empty_asset_set_falls_back_to_stage_default() {
        // 无资产信息 → 回退该 stage 的完整静态集（不擅自缩小）。
        let t = resolve_expected_techniques(StageKind::ExternalAttackSurface, &[]);
        assert!(t.contains(&"GOLISH-EAS-LIVENESS".to_string()));
        assert!(t.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(t.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));
    }

    #[test]
    fn stage_without_coverage_returns_empty() {
        // scoping / reporting 不做覆盖矩阵 → 空（coverage_complete no-op）。
        assert!(resolve_expected_techniques(StageKind::Scoping, &[AssetClass::Domain]).is_empty());
    }

    #[test]
    fn asset_class_parses_target_type_strings() {
        assert_eq!(AssetClass::from_target_type("domain"), AssetClass::Domain);
        assert_eq!(AssetClass::from_target_type("ip_address"), AssetClass::Ip);
        assert_eq!(AssetClass::from_target_type("url"), AssetClass::Url);
        // 未知 → Other（保守：当作可能有 web，不缩小技术集）
        assert_eq!(AssetClass::from_target_type("weird"), AssetClass::Other);
    }
}
```

### 步骤 1.2 — 运行确认失败

```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver
```
预期：编译失败（`resolve_expected_techniques` / `AssetClass` 未定义）→ 这是预期的"功能缺失"红灯。

### 步骤 1.3 — 写最小实现

在 `technique_resolver.rs` 顶部：

```rust
//! 按 stage + in-scope 资产类型动态产出 coverage_complete 的期望技术清单
//! （设计 2026-06-05-coverage-matrix §6.5 ③ seam 的动态生成器）。纯函数、无 IO。
//! 输出 id 与 resources/harness/stages/*.json 的 expected_techniques 同命名空间。

use crate::harness::types::StageKind;

/// in-scope 资产的粗分类（来自 `targets.type`），决定哪些技术适用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Domain,
    Ip,
    Url,
    Cidr,
    /// 未知/其它：保守当作"可能含 web"，不缩小技术集。
    Other,
}

impl AssetClass {
    /// 映射 `targets.type` 字符串（与 golish-pentest-domain TargetType 对齐）。
    pub fn from_target_type(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "domain" | "subdomain" | "host" => Self::Domain,
            "ip" | "ip_address" | "ipv4" | "ipv6" => Self::Ip,
            "url" | "endpoint" | "web" => Self::Url,
            "cidr" | "range" | "netblock" => Self::Cidr,
            _ => Self::Other,
        }
    }

    /// 该资产是否可能承载 web 服务（决定 PARAM / JSAPI / DIR 等 web 技术是否要求）。
    fn maybe_web(self) -> bool {
        matches!(self, Self::Domain | Self::Url | Self::Other)
    }
}

/// 该 stage 的完整静态技术集（与 stage JSON 的 expected_techniques 保持一致；
/// 这是回退基线，动态逻辑只在此之上"按资产类型裁剪"，绝不新增 stage 未声明的技术）。
fn stage_baseline(stage: StageKind) -> Vec<&'static str> {
    match stage {
        StageKind::TargetIntel => vec![
            "GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS", "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT", "GOLISH-INTEL-SUBDOMAIN", "GOLISH-INTEL-OSINT",
        ],
        StageKind::ExternalAttackSurface => vec![
            "GOLISH-EAS-LIVENESS", "GOLISH-EAS-PORT", "GOLISH-EAS-SERVICE-FINGERPRINT",
        ],
        StageKind::Enumeration => vec![
            "GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM", "GOLISH-ENUM-JSAPI",
        ],
        // vuln_triage 的 15 类 WSTG 与具体服务强相关，Phase A 不裁剪（保持静态全集，
        // 走 spec 回退）；本 resolver 对它返回空 = 用 spec 静态值。
        _ => vec![],
    }
}

/// 主入口：按 stage + 资产类型集产出期望技术清单。
/// 规则（Phase A）：
///   - baseline 为空（如 scoping / vuln_triage）→ 返回空（gate 用 spec 静态值或 no-op）。
///   - 资产集为空 → 返回完整 baseline（不擅自缩小）。
///   - 否则按资产类型裁剪 web-only 技术：无任何 web 资产时去掉 GOLISH-ENUM-PARAM。
pub fn resolve_expected_techniques(stage: StageKind, assets: &[AssetClass]) -> Vec<String> {
    let baseline = stage_baseline(stage);
    if baseline.is_empty() {
        return vec![];
    }
    let any_web = assets.iter().any(|a| a.maybe_web());
    let has_assets = !assets.is_empty();
    baseline
        .into_iter()
        .filter(|t| {
            // 唯一的 Phase A 裁剪规则：有资产信息且全非 web 时，PARAM 不适用。
            !(has_assets && !any_web && *t == "GOLISH-ENUM-PARAM")
        })
        .map(String::from)
        .collect()
}
```

在 `backend/crates/golish-agent-kit/src/harness/mod.rs` 加：

```rust
pub mod technique_resolver;
```

### 步骤 1.4 — 运行确认通过

```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver
cargo clippy -p golish-agent-kit --all-targets -- -D warnings && cargo fmt -p golish-agent-kit --check
```
预期：5 个新测全过；clippy 0 告警；fmt clean。

### 步骤 1.5 — Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/technique_resolver.rs backend/crates/golish-agent-kit/src/harness/mod.rs
git commit -m "feat(harness): technique_resolver — per-asset-class expected techniques (P3 ③ seam)"
```

---

## 任务 2 · 让 `DefaultSprintContractGenerator` 填充 `skeleton.expected_techniques`

**文件：** 改 `backend/crates/golish-agent-kit/src/harness/sprint_contract.rs`

### 步骤 2.1 — 先写失败测试

在 `sprint_contract.rs` 的 `mod tests` 内：

```rust
#[test]
fn resolver_populates_skeleton_expected_techniques_for_enumeration_ip_only() {
    use crate::harness::technique_resolver::AssetClass;
    let techs = DefaultSprintContractGenerator::expected_techniques_for(
        StageKind::Enumeration,
        &[AssetClass::Ip],
    );
    // 纯 IP → 无 PARAM；DIR / JSAPI 仍在。
    assert!(!techs.contains(&"GOLISH-ENUM-PARAM".to_string()));
    assert!(techs.contains(&"GOLISH-ENUM-DIR".to_string()));
}
```

### 步骤 2.2 — 运行确认失败

```bash
cd backend && cargo nextest run -p golish-agent-kit resolver_populates_skeleton_expected_techniques_for_enumeration_ip_only
```
预期：编译失败（`expected_techniques_for` 未定义）。

### 步骤 2.3 — 写最小实现

在 `sprint_contract.rs` 的 `impl DefaultSprintContractGenerator` 区域（若无 inherent impl 则新增一个）：

```rust
impl DefaultSprintContractGenerator {
    /// ③ seam 动态生成：按 stage + 资产类型集产出期望技术（委托纯函数 resolver）。
    /// 调用方（gate hook / generator）从 in-scope 资产的 `targets.type` 算出 AssetClass 集。
    pub fn expected_techniques_for(
        stage_kind: StageKind,
        assets: &[crate::harness::technique_resolver::AssetClass],
    ) -> Vec<String> {
        crate::harness::technique_resolver::resolve_expected_techniques(stage_kind, assets)
    }
}
```

> 说明：本任务只暴露生成入口（纯函数包装），不改 `generate()` 的签名（避免牵动现有调用方）；活体注入在任务 4 由 gate hook 直接调 `expected_techniques_for`，走 `GateContext` ③ 路。`generate()` 内的 `contract_text` 渲染可选地追加 expected_techniques 行（非必须，留作后续）。

### 步骤 2.4 — 运行确认通过 + 全量回归

```bash
cd backend && cargo nextest run -p golish-agent-kit
cargo clippy -p golish-agent-kit --all-targets -- -D warnings && cargo fmt -p golish-agent-kit --check
```
预期：全绿（含任务 1 的 5 测 + 本测）。

### 步骤 2.5 — Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/sprint_contract.rs
git commit -m "feat(harness): DefaultSprintContractGenerator exposes expected_techniques_for (P3)"
```

---

## 任务 3 · DB trait 加 `in_scope_target_types`（资产类型来源）

**文件：** 改 `backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`

> 模式照抄已 live 的 `in_scope_assets`（`agent-progress.md` 2026-06-06 P1a）：trait 默认返回空集，app 层经 `ReconTargetsPort` 覆盖；空集 → resolver 收到 `&[]` → 回退 baseline，绝不误缩小。

### 步骤 3.1 — 先写失败测试

在 `repo.rs` 的 `mod tests`（或新增）加一个 test double 断言默认空：

```rust
#[tokio::test]
async fn default_in_scope_target_types_is_empty() {
    struct Dummy;
    #[async_trait::async_trait]
    impl DbRepoProvider for Dummy { /* 仅实现必需方法，其余走 default */ }
    let got = Dummy.in_scope_target_types(None).await.unwrap();
    assert!(got.is_empty());
}
```
（若 `DbRepoProvider` 必需方法过多，改在 app 层 `recon.rs` 写集成测试断言透传；二选一，保持"先红"。）

### 步骤 3.2 — 运行确认失败

```bash
cd backend && cargo nextest run -p golish-agent-kit default_in_scope_target_types_is_empty
```
预期：编译失败（方法未定义）。

### 步骤 3.3 — 写最小实现

`repo.rs` 的 `DbRepoProvider` trait 加：

```rust
/// in-scope 资产的 `targets.type` 集（org 收窄）。default 空集——app 层经端口覆盖。
/// 供 harness ③ seam 的 technique_resolver 决定按资产类型裁剪期望技术。
async fn in_scope_target_types(
    &self,
    _org_id: Option<uuid::Uuid>,
) -> anyhow::Result<Vec<String>> {
    Ok(vec![])
}
```

`golish-agent-app/src/ai/db_bridge/recon.rs` impl（透传到 `ReconTargetsPort`，照 `in_scope_assets` 写法；若端口无该方法，先在 `golish-app-core` 的 `ReconTargetsPort` + `golish-db` 的 `targets` repo 加 `in_scope_types(project_path, org_id)`，SQL `SELECT DISTINCT type FROM targets WHERE scope='in' AND (...org...)`，并按 §I2 IDOR 收 org/project）：

```rust
async fn in_scope_target_types(&self, org_id: Option<Uuid>) -> Result<Vec<String>> {
    self.targets_port.in_scope_types(None, org_id).await
}
```

### 步骤 3.4 — 运行确认通过

```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app db_bridge
cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
```

### 步骤 3.5 — Commit

```bash
git add backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs
# 若动了端口/SQL：一并 add golish-app-core / golish-db 的对应文件
git commit -m "feat(db): in_scope_target_types for harness technique resolver (P3)"
```

> ⚠️ 若本任务触及 `golish-db` 的 SQL / schema，按 AGENTS.md §2.7 **先与用户确认**；本计划默认只加 `SELECT DISTINCT`（读路径，无 migration），不改 schema。

---

## 任务 4 · 在 live gate hook 接通 ③（替掉写死的 `None`）

**文件：** 改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

### 步骤 4.1 — 先写失败测试

gate hook 是 DB 相关的集成点，纯函数验证放在 resolver（任务 1 已覆盖）。本任务用一个**针对 hook 的小集成测试**断言："当 in-scope 资产类型全为 IP 时，enumeration 的 gate 用的期望技术不含 PARAM"。若 hook 难以单测，则改为断言 `apply_harness_gate_hook` 调用后 trace 里 `expected_techniques` 字段非默认（加一个可观测字段并断言）。最小可行：在 `execute.rs` 抽一个纯函数

```rust
fn gate_expected_techniques(stage: StageKind, target_types: &[String]) -> Option<Vec<String>> { ... }
```
并对它写测试：

```rust
#[test]
fn gate_expected_techniques_ip_only_enumeration_drops_param() {
    let t = gate_expected_techniques(StageKind::Enumeration, &["ip_address".into()]).unwrap();
    assert!(!t.contains(&"GOLISH-ENUM-PARAM".to_string()));
}

#[test]
fn gate_expected_techniques_none_when_no_target_types() {
    // 无资产类型信息 → None（回退 spec 静态值，零行为变更）。
    assert!(gate_expected_techniques(StageKind::Enumeration, &[]).is_none());
}
```

### 步骤 4.2 — 运行确认失败

```bash
cd backend && cargo nextest run -p golish-agent-kit gate_expected_techniques
```
预期：编译失败（函数未定义）。

### 步骤 4.3 — 写最小实现

在 `execute.rs` 加纯函数 + 在 `apply_harness_gate_hook` 调用：

```rust
/// 把 in-scope 资产类型映射成本次 gate 的动态期望技术（③ seam）。
/// 返回 None = 无资产类型信息 → 回退 spec 静态值（零行为变更）。
fn gate_expected_techniques(stage: StageKind, target_types: &[String]) -> Option<Vec<String>> {
    if target_types.is_empty() {
        return None;
    }
    let classes: Vec<crate::harness::technique_resolver::AssetClass> = target_types
        .iter()
        .map(|s| crate::harness::technique_resolver::AssetClass::from_target_type(s))
        .collect();
    let techs = crate::harness::sprint_contract::DefaultSprintContractGenerator::expected_techniques_for(stage, &classes);
    if techs.is_empty() { None } else { Some(techs) }
}
```

在 `apply_harness_gate_hook`（以及循环耗尽分支）把：

```rust
let gate_ctx = crate::harness::GateContext { in_scope_assets, expected_techniques: None };
```
改为（先在调用前 `let target_types = self.repo.in_scope_target_types(self.harness_org_id).await.unwrap_or_default();` 取类型，再传入 hook，与现有 `fetch_in_scope_assets_for_gate` 同款"非空才用"守卫）：

```rust
let gate_ctx = crate::harness::GateContext {
    in_scope_assets,
    expected_techniques: gate_expected_techniques(stage_hint.stage_kind, &target_types),
};
```
并在 gate trace 加 `expected_techniques_count` 字段便于线上核实。

### 步骤 4.4 — 运行确认通过 + 全量回归

```bash
cd backend && cargo nextest run -p golish-agent-kit
cargo clippy -p golish-agent-kit --all-targets -- -D warnings && cargo fmt -p golish-agent-kit --check
```
预期：全绿。**重点回归**：现有不传资产类型的测试仍走 `None` → spec 静态值 → 行为不变。

### 步骤 4.5 — Commit

```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs
git commit -m "feat(harness): activate dynamic expected_techniques in live gate hook (P3 ③)"
```

---

## 任务 5 · 活体验证 + 文档/登记收尾

**文件：** 改 `agent-progress.md`、`feature_list.json`、`docs/design/2026-06-05-coverage-matrix.md`（把 §6.5 ③ 从 deferred 标为 done-PhaseA）

### 步骤 5.1 — 全量后端测试

```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app
```
预期：全绿。把通过行复制进 `agent-progress.md` 的"已记录证据"。

### 步骤 5.2 — 活体（需 LLM key + 网络，按 AGENTS.md §3 记录证据）

```bash
# headless 单阶段：对一个授权目标跑 enumeration，看 gate trace 的 expected_techniques_count
just stage assessment enumeration "<授权目标>"   # 或 golish --stage-run --only enumeration --org <ORG> --target <T>
grep -a "expected_techniques_count" ~/.golish/backend.log | tail
```
预期：trace 出现非默认 `expected_techniques_count`（IP-only 目标应比 web 目标少 1=PARAM）。

### 步骤 5.3 — 登记 + Commit

更新 `agent-progress.md`（本轮目标/已完成/证据/commit/风险/下一步）、`feature_list.json`（该条 `status` 与 `evidence`），并在 `docs/design/2026-06-05-coverage-matrix.md` §6.5 注明 ③ Phase A 已落地、Phase B（per-service 细粒度）仍 deferred。

```bash
git add agent-progress.md feature_list.json docs/design/2026-06-05-coverage-matrix.md
git commit -m "docs(harness): record P3 expected_techniques dynamic-injection Phase A"
```

---

## 自检

**1. 规格覆盖度：**
- "③ seam 写死 None" → 任务 4。
- "generator 不产 expected_techniques" → 任务 2。
- "没有资产类型→技术的解析器" → 任务 1。
- "资产类型来源" → 任务 3。
- "活体 + 登记" → 任务 5。全部有对应任务。

**2. 占位符扫描：** 无 TODO / "后续实现" / 无代码的步骤；每个代码步骤都给了真实代码块与命令。任务 3 的 SQL/端口分支显式标注了"若触及 schema 先问用户"。

**3. 类型一致性：** `AssetClass`（任务 1 定义）→ 任务 2 `expected_techniques_for(StageKind, &[AssetClass])` → 任务 4 `from_target_type` 把 `targets.type` 字符串转 `AssetClass`，签名贯穿一致；`resolve_expected_techniques(StageKind, &[AssetClass]) -> Vec<String>` 全程同名。

**边界与风险：**
- **零行为变更保证**：无资产类型信息时 `gate_expected_techniques` 返回 `None` → 回退 `spec.expected_techniques`，与现状逐字节一致（现有测试不破）。
- **绝不放大**：resolver 只在 baseline 内裁剪，绝不新增 stage 未声明的技术（避免凭空加门禁）。
- **Phase B（不在本计划）**：按 per-asset 服务指纹细化（静态站免 PARAM 等）需 EAS 落库 per-asset 指纹，留作后续增量；本计划只到"按资产类型"。
- **DB 触碰**：任务 3 仅读路径（`SELECT DISTINCT type`），不改 schema；若实现时发现需要 migration，停下按 §2.7 问用户。
