# Phase 0：DB 真值权威 gate 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `executing-plans` 逐任务实现此计划；改 gate 前先读设计 `docs/design/2026-06-12-redteam-phase0-db-truth-authoritative-gate.md` 与总纲 `2026-06-12-redteam-db-truth-master.md`。

**目标：** 让 coverage gate 的 `found` 终态只能由真实证据事实（账本派生或 DB 业务表投影，经 `ctx.evidence_facts`）兑现，模型自报 found（coverage cell / technique-tagged claim）不再单独算 found；`checked_empty` 收紧为「自报 + 真 Empty 账本事实」。全部走逐技术灰度开关，缺省零回归。

**架构：** 纯函数改动集中在 `golish-agent-kit/src/harness/gate/rule_engine.rs::coverage_complete`。真值通道（`execute.rs::fetch_evidence_facts_for_gate` 合并账本 + DB 真值进 `ctx.evidence_facts`）已就绪，本期不动。`target_intel.json` 开灰度开关。

**技术栈：** Rust（golish-agent-kit）、serde、cargo nextest。

---

## 文件结构

| 文件 | 职责 | 操作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` | `CoverageComplete` 规则变体 + `coverage_complete()` 纯函数 + 单测 | 修改 |
| `resources/harness/stages/target_intel.json` | 开 `authoritative_found` 灰度（先 DNS/SUBDOMAIN/ASN/CT） | 修改 |
| `backend/crates/golish-agent-kit/src/harness/stage_spec.rs` | 守卫测试：target_intel 的 coverage_complete 已开 authoritative_found | 修改（加断言） |

> 不动：`execute.rs`、`coverage_truth.rs`、`evidence_facts.rs`、hook、migration（真值通道已就绪）。

---

## Task 1 — 给 `CoverageComplete` 加 `authoritative_found` + `authoritative_techniques` 字段

**文件：** `rule_engine.rs`（enum 定义 ~line 51-67）

**步骤 1.1（先写失败测试）：** 在 `rule_engine.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
#[test]
fn parses_authoritative_found_fields() {
    let rule = parse(
        r#"{ "op":"coverage_complete", "authoritative_found": true,
             "authoritative_techniques": ["GOLISH-INTEL-DNS"],
             "on_fail": { "reason": "x" } }"#,
    );
    assert!(matches!(
        rule,
        GateRule::CoverageComplete {
            authoritative_found: true,
            authoritative_techniques: Some(ref t),
            ..
        } if t == &vec!["GOLISH-INTEL-DNS".to_string()]
    ));
    // 缺省：两字段不写 → false / None（零回归）
    let legacy = coverage_complete_rule();
    assert!(matches!(
        legacy,
        GateRule::CoverageComplete { authoritative_found: false, authoritative_techniques: None, .. }
    ));
}
```

**步骤 1.2：** 运行 `cargo nextest -p golish-agent-kit -E 'test(parses_authoritative_found_fields)'` → 失败（字段不存在，编译错）。

**步骤 1.3：** 给 enum 变体加字段：

```rust
    CoverageComplete {
        #[serde(default)]
        terminal_status: Option<Vec<CoverageStatus>>,
        #[serde(default)]
        derive_from_items: bool,
        #[serde(default)]
        derive_from_evidence: bool,
        /// Phase 0（设计 2026-06-12-redteam-phase0）：true 时 `found` 终态只认
        /// `ctx.evidence_facts` 的 Found 事实，自报 found cell / tagged claim 不再算数。
        /// 缺省 false = 旧行为逐字节不变。
        #[serde(default)]
        authoritative_found: bool,
        /// 仅这些 technique 收紧（None = 全部期望技术）。灰度用：落点未到位的
        /// 技术（如 WHOIS/OSINT）暂不收紧。
        #[serde(default)]
        authoritative_techniques: Option<Vec<String>>,
        on_fail: OnFail,
    },
```

**步骤 1.4：** 改 match arm（~line 276）解构并透传新字段：

```rust
        GateRule::CoverageComplete {
            terminal_status,
            derive_from_items,
            derive_from_evidence,
            authoritative_found,
            authoritative_techniques,
            on_fail,
        } => coverage_complete(
            d,
            spec,
            ctx,
            terminal_status.as_deref(),
            *derive_from_items,
            *derive_from_evidence,
            *authoritative_found,
            authoritative_techniques.as_deref(),
            on_fail,
        ),
```

**步骤 1.5：** 给 `coverage_complete` 加两个形参（签名）：

```rust
#[allow(clippy::too_many_arguments)]
fn coverage_complete(
    d: &StageDeliverable,
    spec: &StageSpec,
    ctx: &GateContext,
    terminal_status: Option<&[CoverageStatus]>,
    derive_from_items: bool,
    derive_from_evidence: bool,
    authoritative_found: bool,
    authoritative_techniques: Option<&[String]>,
    on_fail: &OnFail,
) -> GateCheckOutcome {
```

**步骤 1.6：** `cargo nextest -p golish-agent-kit -E 'test(parses_authoritative_found_fields)'` → 通过（其余测试可能因签名变动暂不编译，下一 Task 修）。

**验证：** `cargo check -p golish-agent-kit` exit 0（若其它调用点报错，Task 2 修）。

**提交：** `feat(harness): add authoritative_found fields to coverage_complete rule`

---

## Task 2 — 实现 found/empty 终态收紧逻辑（核心）

**文件：** `rule_engine.rs::coverage_complete`（逐格循环 ~line 369-405）

**步骤 2.1（先写失败测试）：** 加 7 个单测（设计 §7）。先给两个测试辅助（若已存在 `projection_ctx` / `cov_cell` 则复用）：

```rust
fn authoritative_rule(techniques: Option<&[&str]>) -> GateRule {
    let techs = match techniques {
        Some(t) => format!(
            ",\"authoritative_techniques\":{}",
            serde_json::to_string(&t.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
        ),
        None => String::new(),
    };
    parse(&format!(
        r#"{{ "op":"coverage_complete","authoritative_found":true,"derive_from_evidence":true{techs},
             "on_fail":{{"reason":"coverage incomplete"}} }}"#
    ))
}

fn fact(asset: &str, tech: &str, outcome: EvidenceOutcome) -> EvidenceFact {
    EvidenceFact { asset: asset.into(), technique: tech.into(), outcome, evidence_id: 1 }
}
```

测试 1（自报 found 无事实 → BLOCK，回归基线）：

```rust
#[test]
fn authoritative_found_self_report_without_fact_blocks() {
    let rule = authoritative_rule(None);
    let spec = spec_with_expected(&["GOLISH-INTEL-WHOIS"]);
    // 模型自报 found，但 ctx 无 WHOIS 事实
    let d = deliverable_with_coverage(vec![cov_cell(
        "a", "GOLISH-INTEL-WHOIS", CoverageStatus::Found, vec![1],
    )]);
    let ctx = projection_ctx(&["a"], vec![]); // 无事实
    assert!(
        !eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass(),
        "authoritative: self-reported found without a Found fact must BLOCK"
    );
}
```

测试 2（自报 found 有事实 → PASS）：

```rust
#[test]
fn authoritative_found_with_fact_passes() {
    let rule = authoritative_rule(None);
    let spec = spec_with_expected(&["GOLISH-INTEL-DNS"]);
    let d = deliverable_with_coverage(vec![cov_cell(
        "a", "GOLISH-INTEL-DNS", CoverageStatus::Found, vec![1],
    )]);
    let ctx = projection_ctx(&["a"], vec![fact("a", "GOLISH-INTEL-DNS", EvidenceOutcome::Found)]);
    assert!(eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass());
}
```

测试 3（tagged claim 但无事实 → 不再算 found）：

```rust
#[test]
fn authoritative_found_tagged_claim_without_fact_blocks() {
    let rule = authoritative_rule(None);
    let spec = spec_with_expected(&["GOLISH-INTEL-WHOIS"]);
    let mut d = deliverable_with_coverage(vec![]); // 一格未写
    d.claims.push(tagged_claim("a", "GOLISH-INTEL-WHOIS", vec![1])); // 旧 derive_from_items 会放行
    let ctx = projection_ctx(&["a"], vec![]);
    assert!(
        !eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass(),
        "authoritative: tagged claim is no longer a found source"
    );
}
```

测试 4（灰度：仅 DNS 收紧，WHOIS 仍走自报）：

```rust
#[test]
fn authoritative_techniques_scopes_tightening() {
    let rule = authoritative_rule(Some(&["GOLISH-INTEL-DNS"]));
    let spec = spec_with_expected(&["GOLISH-INTEL-WHOIS"]);
    // WHOIS 不在 authoritative 集 → 自报 found 仍算终态
    let d = deliverable_with_coverage(vec![cov_cell(
        "a", "GOLISH-INTEL-WHOIS", CoverageStatus::Found, vec![1],
    )]);
    let ctx = projection_ctx(&["a"], vec![]);
    assert!(
        eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass(),
        "technique not in authoritative set keeps legacy self-report"
    );
}
```

测试 5/6（checked_empty 收紧）：

```rust
#[test]
fn authoritative_checked_empty_requires_empty_fact() {
    let rule = authoritative_rule(None);
    let spec = spec_with_expected(&["GOLISH-INTEL-WHOIS"]);
    let d = deliverable_with_coverage(vec![cov_cell(
        "a", "GOLISH-INTEL-WHOIS", CoverageStatus::CheckedEmpty, vec![1],
    )]);
    // 无 Empty 事实 → BLOCK
    let ctx_none = projection_ctx(&["a"], vec![]);
    assert!(!eval_with_context(&d, &spec, &[rule.clone()], &ctx_none)[0].is_pass());
    // 有 Empty 事实 → PASS
    let ctx_empty = projection_ctx(&["a"], vec![fact("a", "GOLISH-INTEL-WHOIS", EvidenceOutcome::Empty)]);
    assert!(eval_with_context(&d, &spec, &[rule], &ctx_empty)[0].is_pass());
}
```

测试 7（缺省零回归）：

```rust
#[test]
fn authoritative_off_keeps_legacy_self_report() {
    // 不开 authoritative_found（缺省 false）→ 自报 found 仍算终态
    let rule = parse(
        r#"{ "op":"coverage_complete","derive_from_items":true,
             "on_fail":{"reason":"x"} }"#,
    );
    let spec = spec_with_expected(&["GOLISH-INTEL-WHOIS"]);
    let d = deliverable_with_coverage(vec![cov_cell(
        "a", "GOLISH-INTEL-WHOIS", CoverageStatus::Found, vec![1],
    )]);
    let ctx = projection_ctx(&["a"], vec![]);
    assert!(eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass());
}
```

> 若 `tagged_claim` / `projection_ctx` / `spec_with_expected` / `eval_with_context` 辅助名与现有不符，按文件里既有的 PR3（derive_from_evidence）测试用的同名辅助对齐——先 `rg "fn projection_ctx|fn tagged_claim|fn spec_with_expected" rule_engine.rs` 核实签名再写。

**步骤 2.2：** 运行新测试 → 失败（逻辑未改）。

**步骤 2.3：** 替换逐格循环体（line 369-405 的 `for asset { for tech { ... } }` 内层）为：

```rust
    let mut gaps: Vec<String> = Vec::new();
    for asset in &assets {
        for tech in techniques {
            let authoritative = authoritative_found
                && authoritative_techniques
                    .map_or(true, |list| list.iter().any(|t| t == tech));

            let cell_status = |want: CoverageStatus| {
                d.coverage
                    .iter()
                    .any(|c| c.asset == *asset && c.technique == *tech && c.status == want)
            };
            let has_fact = |want: EvidenceOutcome| {
                ctx.evidence_facts.as_deref().is_some_and(|facts| {
                    facts
                        .iter()
                        .any(|f| f.asset == *asset && f.technique == *tech && f.outcome == want)
                })
            };
            let tagged = derive_from_items
                && (d.claims.iter().any(|c| {
                    c.subject == *asset && c.technique.as_deref() == Some(tech.as_str())
                }) || d.findings.iter().any(|f| {
                    f.subject == *asset && f.technique.as_deref() == Some(tech.as_str())
                }));

            // found：authoritative 时只认真值；否则旧三条（自报 cell / tagged / 真值）。
            let found_ok = terminal.contains(&CoverageStatus::Found)
                && if authoritative {
                    has_fact(EvidenceOutcome::Found)
                } else {
                    cell_status(CoverageStatus::Found)
                        || tagged
                        || (derive_from_evidence && has_fact(EvidenceOutcome::Found))
                };
            // checked_empty：authoritative 时自报 + 真 Empty 事实双要；否则自报 or 真值。
            let empty_ok = terminal.contains(&CoverageStatus::CheckedEmpty)
                && if authoritative {
                    cell_status(CoverageStatus::CheckedEmpty) && has_fact(EvidenceOutcome::Empty)
                } else {
                    cell_status(CoverageStatus::CheckedEmpty)
                        || (derive_from_evidence && has_fact(EvidenceOutcome::Empty))
                };
            // blocked / not_applicable：自报 + note（判断态，不变）。
            let other_ok = (terminal.contains(&CoverageStatus::Blocked)
                && cell_status(CoverageStatus::Blocked))
                || (terminal.contains(&CoverageStatus::NotApplicable)
                    && cell_status(CoverageStatus::NotApplicable));

            if !(found_ok || empty_ok || other_ok) {
                gaps.push(format!("({asset} × {tech})"));
            }
        }
    }
```

**步骤 2.4：** 运行 7 个新测试 + 全 crate 测试 → 全绿：

```
cargo nextest -p golish-agent-kit -E 'test(authoritative)' && cargo nextest -p golish-agent-kit
```

预期：7 新测试通过；旧 coverage_complete 测试（derive_from_evidence/items/terminal_status 等）因缺省 authoritative_found=false 仍全绿（零回归）。

**步骤 2.5：** `cargo clippy -p golish-agent-kit --all-targets -- -D warnings` → 零告警；`cargo fmt -p golish-agent-kit`。

**提交：** `feat(harness): authoritative found terminal — only real evidence facts satisfy found`

---

## Task 3 — target_intel 灰度开 authoritative_found（先 DNS/SUBDOMAIN/ASN/CT）

**文件：** `resources/harness/stages/target_intel.json`（coverage_complete 规则，~line 45-52）

**步骤 3.1：** 改 coverage_complete 规则块：

```json
    {
      "op": "coverage_complete",
      "derive_from_items": true,
      "derive_from_evidence": true,
      "authoritative_found": true,
      "authoritative_techniques": ["GOLISH-INTEL-DNS", "GOLISH-INTEL-SUBDOMAIN", "GOLISH-INTEL-ASN", "GOLISH-INTEL-CT"],
      "$comment_authoritative": "Phase 0 灰度：仅对有 DB/账本真值源的 4 类收紧 found；WHOIS/OSINT 待 Phase 1 落点到位后纳入。",
      "on_fail": { "...": "保持原 reason/hints 不变" }
    }
```

> 保留原 `on_fail.reason` 与 `hints` 原文，只加 3 个字段。

**步骤 3.2：** `python3 -m json.tool resources/harness/stages/target_intel.json > /dev/null` → 合法 JSON。

**步骤 3.3（守卫测试）：** 在 `stage_spec.rs` 的 target_intel 守卫测试加断言：

```rust
assert!(
    s.gate_rules.iter().any(|r| matches!(
        r,
        crate::harness::gate::rule_engine::GateRule::CoverageComplete { authoritative_found: true, .. }
    )),
    "target_intel coverage_complete must enable authoritative_found (Phase 0)"
);
```

**步骤 3.4：** `cargo nextest -p golish-agent-kit -E 'test(stage_spec)'`（或对应测试名）→ 绿。

**提交：** `feat(harness): target_intel coverage gate enables authoritative_found gray rollout (DNS/SUBDOMAIN/ASN/CT)`

---

## Task 4 — 全量验证 + 活体回归

**步骤 4.1：** 全 crate 测试 + 静态：

```
cargo nextest -p golish-agent-kit
cargo clippy -p golish-agent-kit --all-targets -- -D warnings
cargo fmt -p golish-agent-kit --check
cargo check --workspace
```

全部 exit 0。

**步骤 4.2（活体回归，可选但强烈推荐）：** 用既有 transcript `stage-run-c4422add` 的 deliverable 作对照，或重跑：

```
./target/debug/golish --stage-run -p deepseek -m deepseek-chat --to target_intel --org 默安科技 --target moresec.cn --auto-approve --verbose > /tmp/golish-stage-run-phase0.log 2>&1
```

预期对照（vs split.log「假过」基线）：
- DNS / SUBDOMAIN：DB 有真数据 → found → 这两格 PASS。
- ASN / CT：若 enrich 真落 `organizations.asns|certificates` → found；否则 BLOCK（正确，真没采到）。
- 日志 `never attempted (... × GOLISH-INTEL-ASN)` 等出现 = found 收紧生效。
- WHOIS / OSINT 不在 authoritative 集（灰度）→ 仍按旧自报（Phase 1 再收）。

**步骤 4.3：** `just precommit` 全绿。

**步骤 4.4：** 更新 `agent-progress.md`（本轮目标/已完成/验证证据/commit）+ `feature_list.json` 新条目状态。

**提交：** `test(harness): Phase 0 full verification + progress`（progress/feature_list 一并）

---

## 自检（writing-plans §自检）

1. **规格覆盖**：设计 §4.1（found 只认真值）→ Task 2；§4.3（灰度开关）→ Task 1+3；§7 七个 DoD 单测 → Task 2 步骤 2.1 全覆盖；checked_empty 收紧 → 测试 5/6。✅
2. **占位符扫描**：on_fail 原文保留（Task 3 注明「保持原 reason/hints」），无 TODO；逐格循环给了完整替换代码。✅
3. **类型一致**：`CoverageStatus{Found,CheckedEmpty,Blocked,NotApplicable}`、`EvidenceOutcome{Found,Empty}`、`EvidenceFact{asset,technique,outcome,evidence_id}` 均与 `rule_engine.rs`/`types.rs` 现有定义一致；测试辅助名（projection_ctx/cov_cell/spec_with_expected/eval_with_context/tagged_claim）需在写测试前 `rg` 核实并对齐（步骤 2.1 已注明）。✅
