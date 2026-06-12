# Harness StageClaim / HarnessFinding `technique` 字段实现计划（P5）

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 给 `StageClaim` / `HarnessFinding` 增加 `technique: Option<String>`（`#[serde(default)]` 向后兼容），把自由字符串 `kind` 与 coverage matrix 的 technique 维度（`GOLISH-INTEL-*` / `WSTG-*` 等）关联起来，使 gate 能从 claims/findings **自动派生**与**交叉校验** coverage。
**架构：** 类型层加可选字段（serde 默认 None，旧 JSON 零破坏）；schema_check 对 technique 做 taxonomy fail-closed 校验；rule_engine 三处扩展——`ItemField::Technique` 可寻址到 claims/findings、`coverage_complete` 新增 opt-in `derive_from_items` 派生、新 op `coverage_corroborated` 反向校验 found cell；spec 接线先落 `target_intel.json`。gate 保持纯函数 / DB-free / fail-closed。
**技术栈：** Rust 2021（serde / serde_json / uuid / cargo-nextest），harness 资源 JSON（`resources/harness/`）。

---

## 0. 背景与现状（执行者必读）

### P5 问题

`StageClaim.kind` / `HarnessFinding.kind` 是自由字符串（如 `dns_a_record` / `asn_lookup`），与 `CoverageCell.technique`（taxonomy 登记 id，如 `GOLISH-INTEL-DNS`）**没有任何结构化关联**。后果：

1. agent 做了工作（claims 有证据）但忘了在 coverage matrix 里登记 cell → `coverage_complete` BLOCK，gate 无法从 claims 推导出"其实测过了"；
2. 反过来，agent 可以提交 `status=found` 的 coverage cell 而 deliverable 里**没有任何**结构化观察支撑该 (asset × technique) → matrix 可凭空捏造（仅 evidence_refs 非空约束，证据不与 technique 关联）。

### 关键源码锚点（已勘察，2026-06-11）

| 锚点 | 位置 | 现状 |
|---|---|---|
| `StageClaim`（kind/subject/summary/evidence_ids） | `backend/crates/golish-agent-kit/src/harness/types.rs` ~L130 | 无 technique |
| `HarnessFinding`（finding_id/kind/subject/severity/evidence_refs） | 同文件 ~L146 | 无 technique |
| `CoverageCell.technique: String` | 同文件 ~L173 | 已有，taxonomy id |
| technique 词典 | `backend/crates/golish-agent-kit/src/harness/technique_taxonomy.rs`（`is_recognized` / `lookup`）+ `resources/harness/technique_taxonomy.json` | spec 侧 expected_techniques 有守卫测试；**agent 提交值无运行时校验** |
| rule engine（op 枚举 / `eval_one` / `coverage_complete` / `resolve`） | `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` | `ItemField::Technique` 只解析到 Coverage；`coverage_complete` 只看声明的 cells |
| schema_check | `backend/crates/golish-agent-kit/src/harness/gate/schema_check.rs` | 只查 stage_id / stage_run_id |
| 提交工具（serde 解析 deliverable + parameters 描述） | `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs` | claims/findings 描述无 technique |
| 文本回退解析 | `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` `parse_deliverable_from_content` | serde 路径，加字段后零改动 |
| **生产构造点（唯一）** | 同文件 `synthesize_confirm_only_deliverable`（~L1997-2014） | StageClaim 字面量，需补 `technique: None` |
| stage charter（agent 提示） | `golish-agent-kit/src/task_orchestrator/prompts/mod.rs` `stage_charter` | claims/findings 示例 JSON 无 technique |
| 首个接线 spec | `resources/harness/stages/target_intel.json` | 已声明 6 个 `GOLISH-INTEL-*` expected_techniques + `coverage_complete` 规则 |

### ts-rs / IPC 影响核实（已验证）

- `rg 'ts_rs|derive\(TS' backend/crates/golish-agent-kit` → **0 命中**：harness types 不做 ts-rs 导出。
- `rg 'StageClaim|HarnessFinding' frontend/` → **0 命中**：前端无引用。
- `rg 'StageClaim|HarnessFinding' backend/crates` → 仅 `golish-agent-kit` 内部 + `golish-agent-app` 的 serde 解析路径。

**结论：纯 harness 内部 gate 类型，不跨 IPC，无 ts-rs 同步义务（AGENTS.md M3/I5 不触发）。** 执行时用上面两条 `rg` 复核一次即可。

### 与 in-flight 改动的协调（重要）

工作区当前有未提交的 gate-integrity（H1/H2/H3，`docs/design/2026-06-10-gate-integrity-closure.md`）与 weak-model-submit-channel 改动，`rule_engine.rs` / `stage_spec.rs` / `target_intel.json` / `execute.rs` 都在被改。因此：

- 本计划内所有行号是 2026-06-11 勘察值，**仅作定位参考**；执行时一律用 `rg` 锚定符号，不要按行号盲改。
- 新增的 spec 守卫测试**断言"存在某规则"**（`iter().any(matches!(...))`），**不断言 gate_rules 总数**，避免与 H3（target_intel 补 `named_check:min_invocations`）撞车。

### 范围

- **In**：types 字段 + 全构造点迁移；schema_check taxonomy 校验（claims/findings）；rule_engine 三处扩展；`target_intel.json` 接线；submit 工具参数描述；stage charter 提示；全套 TDD 测试。
- **Out（明确不做）**：`CoverageCell.technique` 的运行时 taxonomy 校验（现网已有活体流量，blast radius 单独评估）；subject↔asset 归一化（URL vs host）；其它 11 个 stage spec 的接线（target_intel 验证后逐 stage 推开）；DB schema（deliverable 不落独立表，序列化为 JSON 字符串走 side-channel）；前端。

### 设计决策（写死，执行者不要重新发明）

| # | 决策 | 理由 |
|---|---|---|
| D1 | 派生 = `coverage_complete` 的 **opt-in 布尔** `derive_from_items`（serde default false），只把 technique 标注且 `subject == asset` 的 claim/finding 当作该 (asset × technique) 的 **found 终态**；**不**把 item 的 subject 加进资产维度全集 | 缺省行为逐字节不变；资产全集应来自权威注入（GateContext.in_scope_assets）或显式声明，从 claim subject 派生全集会被 ASN/URL 等 subject 噪声放大成不可能完成的矩阵 |
| D2 | 派生只覆盖 `Found`：claim/finding 是"观察到了什么"，无法表达 absence；`checked_empty` 仍必须显式 cell + 证据（AGENTS.md I8） | "已检查为空 ≠ 未检查"是平台核心不变量 |
| D3 | 交叉校验 = 新 op `coverage_corroborated`：每个 `status==found` 的 cell 必须有 ≥1 个 `technique == cell.technique && subject == cell.asset` 的 claim/finding；其余终态豁免 | found 是唯一"声称有产出"的状态，凭空 found 是 P5 要堵的造假面；checked_empty 的把关已由 cell 自身 evidence 规则承担 |
| D4 | technique 值（Some 时）必须 `technique_taxonomy::is_recognized`，否则 schema_check BLOCK（含空字符串） | fail-closed：typo 的 id 永远 corroborate 不了任何 cell，与其留下永不匹配的静默缺口不如立刻打回；与 spec 侧 `all_embedded_expected_techniques_are_recognized` 守卫同一哲学 |
| D5 | subject ↔ asset 为**精确字符串相等**（MVP）；charter/工具描述明确要求 agent 用同一标识串 | 归一化（scheme 剥离/大小写/端口）是独立课题，先用提示语约束 |

---

## 1. 文件结构

**修改（生产代码）：**

| 文件 | 职责 |
|---|---|
| `backend/crates/golish-agent-kit/src/harness/types.rs` | 两个 struct 加字段 + serde 兼容测试 |
| `backend/crates/golish-agent-kit/src/harness/gate/schema_check.rs` | technique taxonomy fail-closed 校验 |
| `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` | `ItemField::Technique` 解析扩展、`derive_from_items`、新 op `coverage_corroborated` |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 生产构造点 `synthesize_confirm_only_deliverable` 补 `technique: None`（+2 个测试构造点） |
| `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | stage charter 示例与提示 |
| `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs` | 工具参数描述 + 捕获测试 |
| `resources/harness/stages/target_intel.json` | 首个接线：derive_from_items + coverage_corroborated |

**修改（仅测试构造点补字段，见 §2 清单）：** `e2e_tests.rs`、`eval.rs`、`rag_prior.rs`、`surface_mapping.rs`、`gate/mod.rs`、`gate/vacuous_check.rs`、`gate/freshness_check.rs`、`gate/scope_check.rs`、`gate/contract_check.rs`、`gate/surface_coverage_check.rs`、`gate/finding_verification_check.rs`（均在 `golish-agent-kit` 内）。

**不创建任何新文件**（无新模块；逻辑全部落在既有职责文件内）。

---

## 2. 构造点迁移清单（Task 1 的硬清单）

加字段后 `cargo check -p golish-agent-kit` 会以 E0063（missing field）把所有遗漏点标红——以编译错误为准，本清单用于预估与核对。除标注外全部加 `technique: None`。

**生产代码（1 处）：**

- `task_orchestrator/subtask_phases/execute.rs` · `synthesize_confirm_only_deliverable` 内 `StageClaim {` （~L2000）→ `technique: None`（confirm-only 合成 claim 与任何技术类无关）。

**测试代码（按文件，`StageClaim {` / `HarnessFinding {` 字面量）：**

| 文件 | StageClaim | HarnessFinding |
|---|---|---|
| `harness/e2e_tests.rs` | L57 | L78, L85, L94, L189 |
| `harness/gate/rule_engine.rs` | L734, L776 | helper `finding()` L623 |
| `harness/gate/mod.rs` | L285, L339, L357, L395, L444, L575, L734 | L403, L418, L483, L583 |
| `harness/gate/vacuous_check.rs` | L200 | L153, L170, L216 |
| `harness/gate/freshness_check.rs` | L299 | L263, L279, L319, L342, L368, L398 |
| `harness/gate/scope_check.rs` | L104, L116, L137 | L149 |
| `harness/gate/contract_check.rs` | — | L168, L177（helper `deliverable_with_findings` 内） |
| `harness/gate/surface_coverage_check.rs` | — | helper `finding()` L99 |
| `harness/gate/finding_verification_check.rs` | — | helper `finding()` L103 |
| `harness/surface_mapping.rs` | L242 | helper `finding()` L200 |
| `harness/eval.rs` | helper `claim()` L175 | helper `finding()` L184 |
| `harness/rag_prior.rs` | — | L319, L326 |
| `task_orchestrator/subtask_phases/execute.rs` | L2653（测试） | L2661（测试） |

注意：很多文件用本地 helper fn（`finding(kind)` / `claim(evidence)`）构造，**只改 helper 一处**即可覆盖该文件全部用例。serde/`json!` 解析路径（submit 工具、`parse_deliverable_from_content`、rule_engine 的 `parse(...)` 测试 helper）**零改动**——`#[serde(default)]` 兜底。

---

## Task 1 · types.rs 加字段 + 全构造点迁移 + serde 兼容测试

**文件：**
- 修改：`backend/crates/golish-agent-kit/src/harness/types.rs`
- 修改：§2 清单全部文件

> 说明：加字段会让所有字面量构造点同时编译失败，无法拆成"先红测试、后实现"两个可编译状态，故本任务一个 commit 完成：先写测试（红=编译不过）→ 加字段 → 迁移构造点 → 绿。

**步骤 1 — 在 `types.rs` 的 `mod tests` 末尾新增测试（此刻编译失败，即 TDD 红）：**

```rust
#[test]
fn stage_claim_and_finding_old_json_without_technique_parses() {
    // P5 向后兼容：旧 JSON（无 technique 字段）必须照常解析为 None。
    let c: StageClaim = serde_json::from_str(
        r#"{"kind":"dns_a_record","subject":"example.com","summary":"A 1.2.3.4","evidence_ids":[1]}"#,
    )
    .unwrap();
    assert!(c.technique.is_none());

    let f: HarnessFinding = serde_json::from_str(
        r#"{"finding_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33","kind":"subdomain","subject":"a.example.com","severity":"info","evidence_refs":[1]}"#,
    )
    .unwrap();
    assert!(f.technique.is_none());
}

#[test]
fn stage_claim_and_finding_technique_roundtrip() {
    let c = StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![EvidenceAuditId::new(1)],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    };
    let j = serde_json::to_string(&c).unwrap();
    let back: StageClaim = serde_json::from_str(&j).unwrap();
    assert_eq!(back.technique.as_deref(), Some("GOLISH-INTEL-DNS"));

    let f = HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "subdomain".to_string(),
        subject: "a.example.com".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![EvidenceAuditId::new(1)],
        technique: Some("GOLISH-INTEL-SUBDOMAIN".to_string()),
    };
    let j = serde_json::to_string(&f).unwrap();
    let back: HarnessFinding = serde_json::from_str(&j).unwrap();
    assert_eq!(back.technique.as_deref(), Some("GOLISH-INTEL-SUBDOMAIN"));
}
```

**步骤 2 — 加字段（沿用 `CoverageCell.note` 的 `#[serde(default)]` 风格，不加 skip_serializing_if）：**

```rust
/// Doc 3 §4.3 StageClaim · 每个 claim 必有 evidence_refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageClaim {
    pub kind: String,
    pub subject: String,
    pub summary: String,
    pub evidence_ids: Vec<EvidenceAuditId>,
    /// P5（2026-06-11）：该 claim 佐证的技术类 id（technique_taxonomy.json 登记，
    /// 如 GOLISH-INTEL-DNS / WSTG-INPV-05）。None = 未标注（旧数据 / 与 coverage
    /// 无关的 claim）。Some 时 schema_check 按词典 fail-closed 校验；
    /// coverage_complete(derive_from_items) / coverage_corroborated 据此关联矩阵。
    #[serde(default)]
    pub technique: Option<String>,
}
```

```rust
/// Doc 3 §4.3 Finding · 结构化交付.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessFinding {
    pub finding_id: Uuid,
    pub kind: String,
    pub subject: String,
    pub severity: FindingSeverity,
    pub evidence_refs: Vec<EvidenceAuditId>,
    /// P5（2026-06-11）：同 [`StageClaim::technique`]。
    #[serde(default)]
    pub technique: Option<String>,
}
```

**步骤 3 — 迁移构造点：** 跑 `cd backend && cargo check -p golish-agent-kit 2>&1 | rg 'E0063|missing field'`，对照 §2 清单逐个补 `technique: None`。helper fn 文件只改 helper。`execute.rs` 的生产点 `synthesize_confirm_only_deliverable` 同样补 `technique: None`。

**步骤 4 — 验证：**

```bash
cd backend && cargo check -p golish-agent-kit -p golish-agent-app
cd backend && cargo nextest run -p golish-agent-kit --status-level fail
```

预期：编译零错；nextest 全绿（含两个新测试）。

**步骤 5 — Commit：** `feat(harness): add optional technique field to StageClaim/HarnessFinding (P5)`

---

## Task 2 · schema_check 对 technique 做 taxonomy fail-closed 校验

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/schema_check.rs`

**步骤 1 — 先写失败测试（追加到 `mod tests`；`StageClaim` 已在测试 imports 中）：**

```rust
#[test]
fn blocks_unregistered_claim_or_finding_technique() {
    let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
    let mut d = make_deliverable("external_attack_surface");
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-TYPO".to_string()),
    });
    match run(&d, &spec) {
        GateCheckOutcome::Block { reasons, .. } => {
            assert!(reasons.iter().any(|r| r.contains("GOLISH-INTEL-TYPO")));
        }
        GateCheckOutcome::Pass => panic!("unregistered technique must Block"),
    }
}

#[test]
fn passes_registered_or_absent_technique() {
    let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
    let mut d = make_deliverable("external_attack_surface");
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    });
    d.claims.push(StageClaim {
        kind: "note".to_string(),
        subject: "example.com".to_string(),
        summary: "untagged claim stays legal".to_string(),
        evidence_ids: vec![],
        technique: None,
    });
    assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
}
```

**步骤 2 — 运行确认红：**

```bash
cd backend && cargo nextest run -p golish-agent-kit schema_check --status-level fail
```

预期：`blocks_unregistered_claim_or_finding_technique` 失败（当前 run 不看 technique → Pass）。

**步骤 3 — 实现。** 文件头部加 `use super::super::technique_taxonomy;`，在 `run` 的 stage_id 检查之后、`if reasons.is_empty()` 之前插入：

```rust
    // P5 fail-closed：claim / finding 的 technique（若有）必须已在
    // technique_taxonomy.json 登记——typo 的 id 永远关联不上任何 coverage cell，
    // 在 schema 层立刻打回，而不是留给 coverage 永不匹配的静默缺口。
    for c in &deliverable.claims {
        if let Some(t) = c.technique.as_deref() {
            if !technique_taxonomy::is_recognized(t) {
                reasons.push(format!(
                    "claim '{}' carries unregistered technique '{}' — use a registered id from technique_taxonomy.json (e.g. GOLISH-INTEL-DNS / WSTG-INPV-05) or omit the field",
                    c.kind, t
                ));
            }
        }
    }
    for f in &deliverable.findings {
        if let Some(t) = f.technique.as_deref() {
            if !technique_taxonomy::is_recognized(t) {
                reasons.push(format!(
                    "finding '{}' carries unregistered technique '{}' — use a registered id from technique_taxonomy.json or omit the field",
                    f.kind, t
                ));
            }
        }
    }
```

（`is_recognized("")` 为 false → `Some("")` 也会被打回，符合 fail-closed。）

**步骤 4 — 验证：** 重跑步骤 2 命令，预期全绿。

**步骤 5 — Commit：** `feat(harness): fail-closed taxonomy validation for claim/finding technique in schema_check`

---

## Task 3 · rule_engine：`ItemField::Technique` 可寻址到 claims / findings

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

让 spec 作者能写数据规则（如「target_intel 的每个 claim 必须带 technique」——本计划不强制接线，仅提供积木）。`None → ""`，使 `non_empty` 谓词语义 = "已标注"。

**步骤 1 — 先写失败测试（追加到 `mod tests`，复用文件内既有 `parse` / `test_spec` helper；deliverable 用本地构造避免依赖其它 helper 签名）：**

```rust
#[test]
fn technique_field_resolves_on_claims_and_findings() {
    // P5：non_empty(technique) 对 claims/findings 可用；None 视作空（未标注）。
    let rule = parse(
        r#"{ "op":"for_all","over":"claims",
             "require":{"pred":"non_empty","field":"technique"},
             "on_fail":{"reason":"claims must be technique-tagged"} }"#,
    );
    let mut d = StageDeliverable {
        stage_id: "vuln_triage".to_string(),
        stage_run_id: uuid::Uuid::new_v4(),
        claims: vec![StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        }],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![],
        required_checks_done: vec![],
        coverage: vec![],
    };
    assert!(eval(&d, &test_spec(), &[rule.clone()])[0].is_pass());

    d.claims.push(StageClaim {
        kind: "untagged".to_string(),
        subject: "example.com".to_string(),
        summary: "no technique".to_string(),
        evidence_ids: vec![],
        technique: None,
    });
    assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
}

#[test]
fn technique_eq_pred_matches_on_findings() {
    let rule = parse(
        r#"{ "op":"count_at_least","over":"findings",
             "where":{"pred":"eq","field":"technique","value":"GOLISH-INTEL-SUBDOMAIN"},
             "min":1,"on_fail":{"reason":"need a subdomain-technique finding"} }"#,
    );
    let d = StageDeliverable {
        stage_id: "vuln_triage".to_string(),
        stage_run_id: uuid::Uuid::new_v4(),
        claims: vec![],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![HarnessFinding {
            finding_id: uuid::Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "a.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![],
            technique: Some("GOLISH-INTEL-SUBDOMAIN".to_string()),
        }],
        required_checks_done: vec![],
        coverage: vec![],
    };
    assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
}
```

**步骤 2 — 确认红：** `cd backend && cargo nextest run -p golish-agent-kit rule_engine --status-level fail` → 两个新测试因 `resolve` 返回 `field Technique not valid for claim item` 而失败。

**步骤 3 — 实现。** 在 `resolve` 的 match 中、claim/finding 分支组内各加一臂（放在 `(ItemRef::Claim(c), ItemField::EvidenceIds)` 与 `(ItemRef::Finding(f), ItemField::Severity)` 之后均可）：

```rust
        (ItemRef::Claim(c), ItemField::Technique) => {
            Ok(FieldVal::Text(c.technique.as_deref().unwrap_or("")))
        }
        (ItemRef::Finding(f), ItemField::Technique) => {
            Ok(FieldVal::Text(f.technique.as_deref().unwrap_or("")))
        }
```

**步骤 4 — 验证：** 重跑步骤 2 命令，全绿。

**步骤 5 — Commit：** `feat(harness): resolve ItemField::Technique on claims/findings in gate rule engine`

---

## Task 4 · `coverage_complete` 新增 opt-in `derive_from_items` 派生

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

**步骤 1 — 先写失败测试（复用既有 `parse` / `spec_with_expected` / `deliverable_with_coverage` / `cov_cell` helper）：**

```rust
#[test]
fn coverage_complete_derive_from_items_fills_gap_from_tagged_claim() {
    // P5 派生：声明 cells 只盖 WHOIS，DNS 缺口由 technique 标注的 claim
    // （subject == asset）派生为 found 终态 → Pass。
    let rule = parse(
        r#"{ "op":"coverage_complete","derive_from_items":true,
             "on_fail":{"reason":"intel coverage incomplete"} }"#,
    );
    let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
    let mut d = deliverable_with_coverage(vec![cov_cell(
        "example.com",
        "GOLISH-INTEL-WHOIS",
        CoverageStatus::CheckedEmpty,
        vec![1],
    )]);
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    });
    assert!(eval(&d, &spec, &[rule])[0].is_pass());
}

#[test]
fn coverage_complete_derive_off_keeps_blocking() {
    // 缺省 derive_from_items=false → 行为与现状逐字节一致：标注 claim 不抵 cell。
    let rule = coverage_complete_rule(); // 既有 helper，无 derive_from_items
    let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
    let mut d = deliverable_with_coverage(vec![cov_cell(
        "example.com",
        "GOLISH-INTEL-WHOIS",
        CoverageStatus::CheckedEmpty,
        vec![1],
    )]);
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    });
    match &eval(&d, &spec, &[rule])[0] {
        GateCheckOutcome::Block { reasons, .. } => {
            assert!(reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"), "{reasons:?}");
        }
        GateCheckOutcome::Pass => panic!("derive off must keep current blocking behavior"),
    }
}

#[test]
fn coverage_complete_derive_requires_matching_subject() {
    // 派生要求 subject == asset 精确相等；不同 subject 的标注 claim 不抵缺口，
    // 也不扩资产全集（D1：资产维度仍取声明 cells / 注入集）。
    let rule = parse(
        r#"{ "op":"coverage_complete","derive_from_items":true,
             "on_fail":{"reason":"intel coverage incomplete"} }"#,
    );
    let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
    let mut d = deliverable_with_coverage(vec![cov_cell(
        "example.com",
        "GOLISH-INTEL-WHOIS",
        CoverageStatus::CheckedEmpty,
        vec![1],
    )]);
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "other.com".to_string(),
        summary: "A 5.6.7.8".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    });
    match &eval(&d, &spec, &[rule])[0] {
        GateCheckOutcome::Block { reasons, .. } => {
            assert!(reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"), "{reasons:?}");
        }
        GateCheckOutcome::Pass => panic!("subject mismatch must not derive coverage"),
    }
}
```

**步骤 2 — 确认红：** `derive_from_items` 字段未知 → `parse` helper 内 `serde_json::from_str::<GateRule>` panic（unknown field 不报错？注意：`GateRule` 无 `deny_unknown_fields`，未知字段会被**静默忽略**，于是第一个测试因派生未实现而 Block 失败，第三个测试碰巧过——**这正是要先跑红确认的点**）。

```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine::tests::coverage_complete_derive --status-level fail
```

**步骤 3 — 实现。** ① 枚举变体加字段：

```rust
    CoverageComplete {
        #[serde(default)]
        terminal_status: Option<Vec<CoverageStatus>>,
        /// P5（2026-06-11）：true 时，technique 标注且 subject == asset 的
        /// claim/finding 视作该 (asset × technique) 的 found 终态（自动派生）。
        /// 只补 covered 判定，不扩资产全集（D1）；absence 仍须显式 cell（D2/I8）。
        /// 缺省 false = 行为与旧版逐字节一致。
        #[serde(default)]
        derive_from_items: bool,
        on_fail: OnFail,
    },
```

② `eval_one` 的对应分支改为：

```rust
        GateRule::CoverageComplete {
            terminal_status,
            derive_from_items,
            on_fail,
        } => coverage_complete(
            d,
            spec,
            ctx,
            terminal_status.as_deref(),
            *derive_from_items,
            on_fail,
        ),
```

③ `coverage_complete` 函数签名加 `derive_from_items: bool` 参数，gap 循环改为：

```rust
    let mut gaps: Vec<String> = Vec::new();
    for asset in &assets {
        for tech in techniques {
            let declared = d
                .coverage
                .iter()
                .any(|c| c.asset == *asset && c.technique == *tech && terminal.contains(&c.status));
            // P5 派生（D1/D2）：标注 claim/finding = found 终态；仅当 found 本身
            // 属于本规则的 terminal 集时才生效（terminal_status 收窄时不越权）。
            let derived = derive_from_items
                && terminal.contains(&CoverageStatus::Found)
                && (d.claims.iter().any(|c| {
                    c.subject == *asset && c.technique.as_deref() == Some(tech.as_str())
                }) || d.findings.iter().any(|f| {
                    f.subject == *asset && f.technique.as_deref() == Some(tech.as_str())
                }));
            if !declared && !derived {
                gaps.push(format!("({asset} × {tech})"));
            }
        }
    }
```

（函数其余部分——空 techniques no-op、资产维度解析、P0 空矩阵 Block、缺口聚合——**不动**。）

**步骤 4 — 验证：**

```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine --status-level fail
```

预期：3 个新测试 + 既有 coverage_complete 全部用例（noop/blocks/passes/empty-coverage/ctx-override）全绿，证明缺省路径未被扰动。

**步骤 5 — Commit：** `feat(harness): opt-in derive_from_items lets technique-tagged claims/findings satisfy coverage_complete`

---

## Task 5 · 新 op `coverage_corroborated`（found cell 反向交叉校验）

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

**步骤 1 — 先写失败测试：**

```rust
#[test]
fn coverage_corroborated_blocks_unbacked_found_cell() {
    // P5 交叉校验：found cell 无 technique 匹配（同 asset）的 claim/finding → Block。
    let rule = parse(
        r#"{ "op":"coverage_corroborated",
             "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
    );
    let d = deliverable_with_coverage(vec![cov_cell(
        "example.com",
        "GOLISH-INTEL-DNS",
        CoverageStatus::Found,
        vec![1],
    )]);
    match &eval(&d, &test_spec(), &[rule])[0] {
        GateCheckOutcome::Block { reasons, .. } => {
            assert!(reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"), "{reasons:?}");
        }
        GateCheckOutcome::Pass => panic!("uncorroborated found cell must Block"),
    }
}

#[test]
fn coverage_corroborated_passes_with_matching_tagged_item() {
    let rule = parse(
        r#"{ "op":"coverage_corroborated",
             "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
    );
    let mut d = deliverable_with_coverage(vec![cov_cell(
        "example.com",
        "GOLISH-INTEL-DNS",
        CoverageStatus::Found,
        vec![1],
    )]);
    d.claims.push(StageClaim {
        kind: "dns_a_record".to_string(),
        subject: "example.com".to_string(),
        summary: "A 1.2.3.4".to_string(),
        evidence_ids: vec![],
        technique: Some("GOLISH-INTEL-DNS".to_string()),
    });
    assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
}

#[test]
fn coverage_corroborated_exempts_non_found_cells() {
    // checked_empty / blocked / not_applicable 豁免：absence 没有对应的结构化观察，
    // 其把关由 cell 自身 evidence/note 规则承担（D3）。
    let rule = parse(
        r#"{ "op":"coverage_corroborated",
             "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
    );
    let d = deliverable_with_coverage(vec![
        cov_cell("a", "GOLISH-INTEL-DNS", CoverageStatus::CheckedEmpty, vec![1]),
        cov_cell("a", "GOLISH-INTEL-WHOIS", CoverageStatus::Blocked, vec![]),
        cov_cell("a", "GOLISH-INTEL-ASN", CoverageStatus::NotApplicable, vec![]),
    ]);
    assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
}
```

**步骤 2 — 确认红：** 未知 op → `parse` helper 的 expect panic（`GateRule` 是 `#[serde(tag="op")]` 闭合枚举，未知 op 解析报错 = fail-closed 生效的证明）。

```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine::tests::coverage_corroborated --status-level fail
```

**步骤 3 — 实现。** ① 枚举加变体（放在 `CoverageDenominator` 之后）：

```rust
    /// P5（2026-06-11）交叉校验：每个 status == found 的 coverage cell 必须有 ≥1 个
    /// technique 匹配的 claim/finding 佐证（item.technique == cell.technique 且
    /// item.subject == cell.asset，精确相等，D5）。found 之外的终态豁免（D3）：
    /// absence 无结构化观察可佐证，由 cell 自身 evidence/note 规则把关。
    CoverageCorroborated { on_fail: OnFail },
```

② `GateRule::summary()` 第一组 match 臂追加 `| GateRule::CoverageCorroborated { on_fail, .. }`（漏掉会编译错，non-exhaustive）。

③ `eval_one` 加分支：

```rust
        GateRule::CoverageCorroborated { on_fail } => coverage_corroborated(d, on_fail),
```

④ 新纯函数（放在 `coverage_denominator` 之后，沿用同款缺口聚合）：

```rust
/// `coverage_corroborated` 求值（纯函数，P5 设计 D3/D5）。
fn coverage_corroborated(d: &StageDeliverable, on_fail: &OnFail) -> GateCheckOutcome {
    let mut gaps: Vec<String> = Vec::new();
    for cell in &d.coverage {
        if cell.status != CoverageStatus::Found {
            continue;
        }
        let corroborated = d.claims.iter().any(|c| {
            c.subject == cell.asset && c.technique.as_deref() == Some(cell.technique.as_str())
        }) || d.findings.iter().any(|f| {
            f.subject == cell.asset && f.technique.as_deref() == Some(cell.technique.as_str())
        });
        if !corroborated {
            gaps.push(format!("({} × {})", cell.asset, cell.technique));
        }
    }
    if gaps.is_empty() {
        return GateCheckOutcome::Pass;
    }
    const MAX_SHOWN: usize = 8;
    let shown = gaps
        .iter()
        .take(MAX_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if gaps.len() > MAX_SHOWN {
        format!(" (+{} more)", gaps.len() - MAX_SHOWN)
    } else {
        String::new()
    };
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: no technique-tagged claim/finding backs {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}
```

注意 `CoverageStatus` 需要 `PartialEq`（已有 derive）。`cell.status != CoverageStatus::Found` 直接可用。

**步骤 4 — 验证：** 重跑步骤 2 命令 + `cargo nextest run -p golish-agent-kit rule_engine --status-level fail`，全绿。

**步骤 5 — Commit：** `feat(harness): coverage_corroborated gate op cross-validates found cells against technique-tagged items`

---

## Task 6 · `target_intel.json` 接线 + spec 守卫测试

**文件：**
- 修改：`resources/harness/stages/target_intel.json`
- 修改：`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`（测试）

**步骤 1 — 先写失败测试（追加到 `stage_spec.rs` 的 `mod tests`，仿照既有 `enumeration_requires_per_asset_content_coverage` 写法；只断言存在性，不断言条数——见 §0 协调注意）：**

```rust
    // P5（2026-06-11）：target_intel 的 coverage 必须既能从 technique 标注的
    // claims 派生（derive_from_items），又对 found cell 做反向佐证
    // （coverage_corroborated）。只断言存在性，不锁 gate_rules 总数。
    #[test]
    fn target_intel_coverage_derives_and_corroborates() {
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::TargetIntel)
            .expect("load target_intel spec");
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    derive_from_items: true,
                    ..
                }
            )),
            "target_intel coverage_complete must enable derive_from_items"
        );
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageCorroborated { .. }
            )),
            "target_intel must declare a coverage_corroborated rule"
        );
    }
```

**步骤 2 — 确认红：**

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_spec --status-level fail
```

**步骤 3 — 改 `target_intel.json`。** ① `coverage_complete` 规则对象加一行 `"derive_from_items": true`：

```json
    {
      "op": "coverage_complete",
      "derive_from_items": true,
      "on_fail": {
        "reason": "intel coverage incomplete: some (in-scope asset \u00d7 expected intel technique) cells were never attempted",
        "hints": ["for every in-scope asset, give each expected intel technique a terminal status: found+evidence / checked_empty+evidence / blocked|not_applicable+note", "claims/findings tagged with a `technique` id (GOLISH-INTEL-*) on the same subject auto-derive their cell"]
      }
    }
```

② `gate_rules` 数组末尾追加：

```json
    {
      "op": "coverage_corroborated",
      "on_fail": {
        "reason": "every 'found' intel coverage cell must be corroborated by a technique-tagged claim/finding on the same asset",
        "hints": ["tag each intel claim/finding with its `technique` id (GOLISH-INTEL-DNS / -WHOIS / -ASN / -CT / -SUBDOMAIN / -OSINT)", "use the SAME subject string as the coverage cell's asset"]
      }
    }
```

（保持 JSON 其余内容不动；若 H3 已在该文件加了 `named_check:min_invocations` 规则，两者互不冲突，追加即可。）

**步骤 4 — 验证：**

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_spec technique_taxonomy resources --status-level fail
```

预期：新守卫测试绿；`all_twelve_stage_specs_load` / `all_embedded_expected_techniques_are_recognized` 等既有资源守卫保持绿（typed-enum 解析新 op 成功即证明接线合法）。

**步骤 5 — Commit：** `feat(harness): wire derive_from_items + coverage_corroborated into target_intel stage spec`

---

## Task 7 · submit 工具参数描述 + 捕获测试

**文件：** 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`

serde 解析零改动（Task 1 已兜底）；本任务只改 `parameters()` 的 schema 描述让模型知道可以/应该填 `technique`，并用测试钉住"标注 claim 能被接受且进 side-channel"。

**步骤 1 — 先写测试（追加到 `mod tests`）：**

```rust
    // P5 · technique 标注的 claim 解析、通过 gate 预检、并完整进入 side-channel。
    #[tokio::test]
    async fn accepts_technique_tagged_claims_and_captures_them() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["claims"][0]["technique"] = json!("GOLISH-INTEL-DNS");
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));

        let captured = sink.read().await.clone().expect("deliverable captured");
        assert!(captured.contains("GOLISH-INTEL-DNS"));
    }

    // P5 · 未登记的 technique id 在 submit 预检（schema_check）就 needs_fix，
    // 不会以 accepted 误导 agent 前进。
    #[tokio::test]
    async fn needs_fix_on_unregistered_technique() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["claims"][0]["technique"] = json!("GOLISH-INTEL-TYPO");
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let reasons = out["reasons"].as_array().expect("reasons");
        assert!(
            reasons
                .iter()
                .any(|r| r.as_str().unwrap_or("").contains("GOLISH-INTEL-TYPO")),
            "{reasons:?}"
        );
        assert!(sink.read().await.is_some(), "still stashed for the stage-close gate");
    }
```

**步骤 2 — 确认状态：** 第一个测试加完 Task 1-2 后应直接绿（回归钉子）；第二个依赖 Task 2 的 schema_check（gate 预检走 `validate_stage_gate`，schema_check 在其中）→ 也应绿。**若第二个红，说明 schema_check 未进 submit 预检路径，回查 Task 2。**

```bash
cd backend && cargo nextest run -p golish-agent-app harness_submit_tool --status-level fail
```

**步骤 3 — 更新 `parameters()` 描述（两处 description 字符串整体替换）：**

`claims`：

```rust
                "claims": {
                    "type": "array",
                    "description": "Observations, each {kind, subject, summary, evidence_ids:[int], technique?:string}; every evidence_id must also appear in evidence_refs. When a claim evidences one of the stage's expected techniques, set `technique` to that REGISTERED id (e.g. GOLISH-INTEL-DNS, WSTG-INPV-05) and use the SAME subject string as the coverage cell's asset — technique-tagged claims corroborate 'found' coverage cells and can auto-derive cells you did the work for but forgot to declare. Unregistered ids are rejected.",
                    "items": { "type": "object" }
                },
```

`findings`：

```rust
                "findings": {
                    "type": "array",
                    "description": "Findings, each {finding_id:uuid, kind, subject, severity, evidence_refs:[int], technique?:string}; tag `technique` with the registered technique id the finding evidences (same id namespace as the coverage matrix; omit when none applies).",
                    "items": { "type": "object" }
                },
```

**步骤 4 — 验证：** 重跑步骤 2 命令 + `cargo nextest run -p golish-agent-app --status-level fail`，全绿。

**步骤 5 — Commit：** `feat(harness): document technique tagging in submit_stage_deliverable schema + pin capture tests`

---

## Task 8 · stage charter 提示（prompts/mod.rs）

**文件：** 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`

**步骤 1 — 先写失败测试（追加到该文件 `mod tests`，仿照既有 `stage_charter_lists_expected_techniques_when_set`（~L714）：第二参就是 `&ScopingPolicy::default()`，已勘察确认）：**

```rust
    /// P5：声明 expected_techniques 的 stage，charter 必须教 agent 给 claims/findings
    /// 打 technique 标注（派生 + 佐证），且示例 JSON 含 technique 字段。
    #[test]
    fn stage_charter_mentions_technique_tagging_when_expected() {
        use crate::harness::stage_spec::load_stage_spec_from_json;

        let spec = load_stage_spec_from_json(
            r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "expected_techniques":["GOLISH-INTEL-DNS","GOLISH-INTEL-WHOIS"]}"#,
        )
        .expect("spec parses");
        let charter = stage_charter(&spec, &ScopingPolicy::default());
        assert!(
            charter.contains("\"technique\""),
            "claims/findings example JSON must include the technique field"
        );
        assert!(
            charter.contains("Tag claims/findings with `technique`"),
            "charter must explain technique tagging"
        );

        // expected_techniques 为空的 stage 不渲染标注教学（与 coverage_line 同生命周期）。
        let without = load_stage_spec_from_json(
            r#"{"id":"scoping","kind":"scoping","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .unwrap();
        assert!(!stage_charter(&without, &ScopingPolicy::default())
            .contains("Tag claims/findings with `technique`"));
    }
```

**步骤 2 — 确认红：** `cd backend && cargo nextest run -p golish-agent-kit prompts --status-level fail`

**步骤 3 — 实现。** ① `coverage_line` 的 `format!` 文本末尾（`(blocked / not_applicable cells are exempt from the denominator.)` 之后、闭引号之前）追加：

```text
\n- **Tag claims/findings with `technique`** — set each claim's/finding's `technique` field to the matching expected technique id above, using the SAME `subject` string as the cell's `asset`. Technique-tagged items corroborate your 'found' coverage cells (a 'found' cell with NO matching tagged claim/finding on the same asset is rejected) and can auto-derive cells you did the work for but forgot to declare.
```

② 大 `format!` 模板里 claims 示例行改为：

```text
    {{"kind": "http_service_observed", "subject": "<host>", "summary": "<what was observed>", "evidence_ids": [<int_id_from_a_real_tool_result>], "technique": "<registered technique id backing this claim — omit if none applies>"}}
```

③ findings 示例行改为：

```text
    {{"finding_id": "<random uuid v4>", "kind": "subdomain", "subject": "<host>", "severity": "info", "evidence_refs": [<int_id_from_a_real_tool_result>], "technique": "<registered technique id — omit if none applies>"}}
```

**步骤 4 — 验证：** 重跑步骤 2 命令；并跑 `cargo nextest run -p golish-agent-kit prompts execute --status-level fail` 确认既有 charter 断言（如 `external_attack_surface_charter_surfaces_liveness_technique`）未破。

**步骤 5 — Commit：** `feat(harness): stage charter teaches technique tagging on claims/findings`

---

## Task 9 · 全量验证 + 收尾（AGENTS.md §3/§4）

**步骤 1 — 全量回归：**

```bash
cd backend && cargo fmt --all
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail
just lint-rust
just precommit
```

预期：全绿、clippy 零 warning。证据（命令 + 退出码 + 关键输出）抄进 `agent-progress.md`「已记录证据」段。

**步骤 2 — ts-rs 复核（应为零命中，否则停下重评）：**

```bash
rg 'ts_rs|derive\(TS' backend/crates/golish-agent-kit
rg 'StageClaim|HarnessFinding' frontend/
```

**步骤 3 — 文档与状态：**

- `agent-progress.md`：新增会话记录（目标 / 已完成 / 验证证据 / commit 列表 / 风险 / 下一步）。
- `feature_list.json`：本功能条目置 `passing` 并填 `evidence`（若执行会话开工时尚未追加条目，先按 AGENTS.md §1.3 补一条再流转状态）。
- 模块卡：若 `docs/modules/backend/golish-agent-kit.md` 存在，更新「公开接口」（types 字段、rule_engine 新 op）；不存在则按模板补卡 + 更新 `docs/modules/INDEX.md`。

**步骤 4 — Commit：** `chore(harness): P5 technique-field rollout bookkeeping`（仅文档/状态文件）。

---

## 3. 向后兼容矩阵

| 面 | 旧输入 / 旧行为 | 新行为 | 兼容性 |
|---|---|---|---|
| 旧 deliverable JSON（无 technique） | 解析成功 | `technique: None`，schema_check 不看 None | ✅ 零破坏（Task 1 测试钉住） |
| 序列化输出 | 无字段 | 多一个 `"technique": null` / `"technique":"..."` 键 | ✅ 消费方均为 serde 解析（side-channel 字符串 / gate），无 schema 强校验 |
| `coverage_complete` 未写 `derive_from_items` 的 11 个 stage | 现行为 | serde default false → 逐字节一致（Task 4 `derive_off` 测试钉住） | ✅ |
| `coverage_corroborated` 未接线的 stage | 无此规则 | 不求值 | ✅ |
| 写错 op 名 / terminal_status | spec 加载期报错 | 同（闭合枚举） | ✅ fail-closed 保持 |
| target_intel 活体 run | 不标 technique 也可过（cell 自报） | found cell 必须有标注佐证 → 不标会 BLOCK + hints 教标注 | ⚠️ 行为收紧（这正是 P5 目的）；charter/工具描述/needs_fix 三处教学，重试环可自愈 |

## 4. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 弱模型不给 claims 打 technique → target_intel found cell 全 BLOCK 重试 | charter（Task 8）+ 工具 schema 描述（Task 7）+ on_fail hints（Task 6）三处显式教学；只接线 target_intel 一个 stage，活体验证后再推广 |
| subject ↔ asset 字符串形态不一致（`https://example.com` vs `example.com`）→ 佐证/派生 false-negative | D5 精确匹配 + 提示语强制"同一标识串"；归一化作为后续课题挂账 |
| technique 标错但已登记（DNS claim 标成 WHOIS）→ 错佐证 | 确定性 gate 无法判语义；residual。后续可按 kind→technique 启发式（参考 `surface_mapping::from_kind` 先例）做 lint-级 hint，不在本计划 |
| 与 in-flight H1/H2/H3 改动冲突（同文件） | §0 协调注意：rg 锚定、存在性断言、执行前 `git status` 核对该文件是否有未提交改动并先沟通 |
| `GateRule` 未 `deny_unknown_fields`，spec 写错 `derive_from_items` 拼写会被静默忽略 | Task 6 守卫测试用 typed-enum matches! 断言 `derive_from_items: true` 真解析出来，拼错即红 |
| 派生绕过 cell 级 evidence 规则（derived cell 无 for_all found⇒evidence 检查） | claims 自身受 `for_all claims ⇒ evidence_ids 非空`（target_intel 已有）约束，证据链不缺位；明确写入设计决策 D1 |

## 5. 自检对照（writing-plans 模板）

- 规格覆盖度：P5 诉求四要素——字段（Task 1）、派生（Task 4+6）、交叉校验（Task 5+6）、向后兼容（serde default + 兼容矩阵）——均有对应任务。提交解析（Task 7）、构造点（§2 + Task 1）、agent 教学（Task 8）、收尾证据（Task 9）覆盖任务委托方列的全部注意点。
- 占位符扫描：无 TBD/TODO；所有代码块给全文。
- 类型一致性：`technique: Option<String>`、`derive_from_items: bool`、op 名 `coverage_corroborated`、谓词字段名 `technique` 全文一致。
