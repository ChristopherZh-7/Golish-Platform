# 诊断式 reflector：BLOCK 纠正注入 DB 真值现状 + 建议命令（PR-C）

> 配 `.cursor/skills/test-driven-development/SKILL.md`。承接 PR-A/B（`coverage_truth` + DB 真值投影注入 `ctx.evidence_facts`）。

**目标：** 把 coverage gate BLOCK 时回灌给重做循环的 `repair_correction` 从「机械念缺口」升级为「诊断式」——告诉模型 ①**DB 真值现状**（该 run 已在业务表落了哪些 `(asset × technique)`）②被动情报缺口（gate reasons 已有）③**每类缺口的具体下一步命令**（确定性建议表，强调 DNS 用全量 `dig` 才落 `dns_records`）。

**架构：** 纯确定性增强 `build_gate_correction`（execute.rs），消费已注入的 `ctx.evidence_facts`（PR-A/B 合并的账本+DB 真值 facts）。不调独立 reflector LLM，不改模型配置链路。

---

## 0. 现状（已核对）

- gate BLOCK → `build_harness_gate_outcome` 尾部 `repair_correction = Some(build_gate_correction(&decision))`（execute.rs:2007）→ `HarnessGateOutcome.repair_correction` → 重做循环 `pending_gate_correction` 注入 `## IMPORTANT CORRECTION`（execute.rs:187-192）让 agent 重做。
- `build_gate_correction(decision)`（execute.rs:2084）只渲染 `decision.reasons` + `recovery_actions`（repair_tool_calls/missing_evidence/hints）——**不含 DB 现状**（设计 §3.4 痛点）。
- `apply_harness_gate_hook` 在 `gate_ctx`（含 `evidence_facts`）作用域内调 `build_gate_correction`，故可把 `gate_ctx.evidence_facts` 传进去。
- `executor.reflect`（execute.rs:217）是 text-only 路径的独立 reflector LLM（agent 返回纯文本无工具时），其模型走既有配置——**本 PR 不碰它**。

---

## 1. 关键决策

- **D-C1 · 确定性增强，不调独立 reflector LLM。** 理由：① 设计 §8 风险「弱模型能力下限…救不了吐空内容的模型」——确定性 hint 比再赌一次 LLM 更稳；② 「reflector 模型由前端/settings 配置」是既有 `executor.reflect`（text-only）机制，本 PR 不需要改它；③ 诊断内容（DB 现状 + 命令）是确定性的，可 TDD、可复现。
- **D-C2 · 诊断 = DB 现状 + 命令建议；「近期行为」(设计 §5.4 ③) 暂不做。** 「模型最近 N 步在重复什么错」需查 `tool_calls` 历史（跨 repo + 噪声大、边际价值低），本 PR 不做，记 progress 作为后续增强。DB 现状 + 命令是命中「看 DB 缺口给具体下一步」诉求的核心。
- **D-C3 · 只在 coverage BLOCK 追加诊断。** 仅当 `decision.reasons` 含被动情报缺口信号（`GOLISH-INTEL-` 或 `never attempted`）时追加命令建议段；DB 现状段仅当 `evidence_facts` 有 Found 项时追加。其它 stage 的 BLOCK 纠正逐字不变（零回归）。
- **红线：** 诊断只读 `evidence_facts`（已注入的真值）+ 静态命令表，不写库、不臆造数据；不改 gate 判定。

---

## 2. 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 改 | `passive_intel_command_hint`（technique→命令静态表）+ `build_db_truth_diagnosis`（DB 现状渲染）+ `build_gate_correction` 加 `evidence_facts` 参数追加诊断段 + 调用点传参 + 测试 |

---

## 任务 1 · technique→命令静态表（TDD）

### 步骤 1.1 — 测试先行（红）

在 execute.rs 测试区（`build_gate_correction` 相关测试附近，搜 `build_gate_correction` 现有测试）加：

```rust
    mod diagnostic_reflector_tests {
        use super::super::*;

        #[test]
        fn command_hint_covers_passive_intel_techniques_and_warns_dig_full_banner() {
            assert!(passive_intel_command_hint("GOLISH-INTEL-DNS").unwrap().contains("dig"));
            assert!(
                passive_intel_command_hint("GOLISH-INTEL-DNS").unwrap().contains("+noall +answer")
                    || passive_intel_command_hint("GOLISH-INTEL-DNS").unwrap().contains("not +short"),
                "DNS hint must steer to full banner so records persist to dns_records"
            );
            assert!(passive_intel_command_hint("GOLISH-INTEL-SUBDOMAIN").unwrap().contains("subfinder"));
            assert!(passive_intel_command_hint("GOLISH-INTEL-WHOIS").unwrap().contains("whois"));
            assert!(passive_intel_command_hint("GOLISH-INTEL-CT").unwrap().contains("crt.sh"));
            assert!(passive_intel_command_hint("GOLISH-INTEL-ASN").is_some());
            assert!(passive_intel_command_hint("GOLISH-INTEL-OSINT").is_some());
            // 未知 technique → None（不臆造命令）。
            assert!(passive_intel_command_hint("GOLISH-INTEL-BOGUS").is_none());
        }
    }
```

### 步骤 1.2 — 实现（绿）

在 execute.rs（`build_gate_correction` 之前）加：

```rust
/// 设计 2026-06-12 §5.4 · 被动情报 technique → 具体下一步命令建议（确定性表）。
/// `None` = 未知 technique（不臆造命令，保守）。`<asset>` 由模型按 in-scope 资产替换。
fn passive_intel_command_hint(technique: &str) -> Option<&'static str> {
    match technique {
        "GOLISH-INTEL-DNS" => Some(
            "dig <asset> ANY +noall +answer  (use the FULL banner, NOT +short — only full \
             ANSWER-SECTION rows persist to dns_records and satisfy the gate)",
        ),
        "GOLISH-INTEL-SUBDOMAIN" => Some("subfinder -d <asset> -silent"),
        "GOLISH-INTEL-WHOIS" => Some("whois <asset>"),
        "GOLISH-INTEL-ASN" => {
            Some("whois -h whois.cymru.com \" -v <ip>\"  (ASN/netblock ownership for the resolved IP)")
        }
        "GOLISH-INTEL-CT" => {
            Some("curl -s 'https://crt.sh/?q=<asset>&output=json'  (Certificate Transparency)")
        }
        "GOLISH-INTEL-OSINT" => Some("theHarvester -d <asset> -b all   (emails / hosts / leaks)"),
        _ => None,
    }
}

/// 本 PR 灰度的被动情报 technique 全集（target_intel expected_techniques 的镜像，
/// 用于在缺口诊断里列出每类的命令建议）。
const PASSIVE_INTEL_TECHNIQUES: &[&str] = &[
    "GOLISH-INTEL-DNS",
    "GOLISH-INTEL-WHOIS",
    "GOLISH-INTEL-ASN",
    "GOLISH-INTEL-CT",
    "GOLISH-INTEL-SUBDOMAIN",
    "GOLISH-INTEL-OSINT",
];
```

---

## 任务 2 · DB 真值现状渲染（TDD）

### 步骤 2.1 — 测试先行（红）

```rust
        #[test]
        fn db_truth_diagnosis_lists_found_facts_and_is_none_when_empty() {
            use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
            assert!(build_db_truth_diagnosis(&[]).is_none(), "no facts → no DB-status section");
            let facts = vec![
                EvidenceFact { asset: "moresec.cn".into(), technique: "GOLISH-INTEL-DNS".into(), outcome: EvidenceOutcome::Found, evidence_id: 0 },
                EvidenceFact { asset: "moresec.cn".into(), technique: "GOLISH-INTEL-ASN".into(), outcome: EvidenceOutcome::Empty, evidence_id: 5 },
            ];
            let out = build_db_truth_diagnosis(&facts).unwrap();
            assert!(out.contains("moresec.cn") && out.contains("GOLISH-INTEL-DNS"));
            // 只列 Found（DB 真有数据）；Empty 不算「DB 已有」(I8：empty≠有数据)。
            assert!(!out.contains("GOLISH-INTEL-ASN"), "Empty fact is not 'persisted data'");
        }
```

### 步骤 2.2 — 实现（绿）

```rust
/// 设计 2026-06-12 §5.4 · 渲染「DB 真值现状」：该 run 已在业务表/账本确认有数据的
/// `(asset × technique)`（仅 `Found`——`Empty` 是「跑了→空」，不算「DB 已有数据」，I8）。
/// `None` = 无 Found 事实（不追加空段）。
fn build_db_truth_diagnosis(
    facts: &[crate::harness::gate::rule_engine::EvidenceFact],
) -> Option<String> {
    use crate::harness::gate::rule_engine::EvidenceOutcome;
    let mut found: Vec<String> = facts
        .iter()
        .filter(|f| f.outcome == EvidenceOutcome::Found)
        .map(|f| format!("- {} × {}", f.asset, f.technique))
        .collect();
    if found.is_empty() {
        return None;
    }
    found.sort();
    found.dedup();
    Some(format!(
        "\n### DB truth status (already persisted — do NOT re-run these)\n{}\n",
        found.join("\n")
    ))
}
```

---

## 任务 3 · build_gate_correction 追加诊断段（TDD）

### 步骤 3.1 — 改签名 + 测试先行（红）

`build_gate_correction` 加参数 `evidence_facts: Option<&[EvidenceFact]>`。先改调用点（execute.rs:2007）：

```rust
        Some(build_gate_correction(&decision, gate_ctx.evidence_facts.as_deref()))
```

> 注：`gate_ctx` 在 `build_harness_gate_outcome`/`apply_harness_gate_hook` 尾部仍在作用域（`evidence_facts` move 进 `gate_ctx` 后，用 `gate_ctx.evidence_facts` 访问）。执行前确认 2007 行所在函数能访问 `gate_ctx`（搜上下文：若 `build_gate_correction` 在独立 `build_harness_gate_outcome(decision, ...)` 里调用，则把 `evidence_facts` 作为该函数参数透传）。

测试：

```rust
        #[test]
        fn gate_correction_appends_db_status_and_command_hints_on_coverage_block() {
            use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
            use crate::harness::{GateResult, HarnessRecoveryActions};
            let decision = GateResult {
                allowed: false,
                reasons: vec!["coverage incomplete: never attempted (moresec.cn × GOLISH-INTEL-DNS)".into()],
                recovery_actions: None,
            };
            let facts = vec![EvidenceFact {
                asset: "moresec.cn".into(),
                technique: "GOLISH-INTEL-SUBDOMAIN".into(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 0,
            }];
            let c = build_gate_correction(&decision, Some(&facts));
            assert!(c.contains("DB truth status"), "DB 现状段");
            assert!(c.contains("GOLISH-INTEL-SUBDOMAIN"), "列已 Found 的类");
            assert!(c.contains("Suggested next commands"), "命令建议段");
            assert!(c.contains("dig") && c.contains("subfinder"), "含被动情报命令");
        }

        #[test]
        fn gate_correction_unchanged_for_non_coverage_block() {
            use crate::harness::GateResult;
            let decision = GateResult {
                allowed: false,
                reasons: vec!["finding count below minimum".into()],
                recovery_actions: None,
            };
            let c = build_gate_correction(&decision, None);
            assert!(!c.contains("Suggested next commands"), "非 coverage BLOCK 不追加命令段");
            assert!(!c.contains("DB truth status"));
        }
```

> 执行前 `Grep "struct GateResult"` / `pub struct GateResult` 确认字段名（`allowed`/`reasons`/`recovery_actions`）与构造方式；测试构造按真实定义调整（可能需 `..Default::default()` 或 builder）。

### 步骤 3.2 — 实现（绿）

`build_gate_correction` 末尾（`s` 返回前）追加：

```rust
    // 设计 2026-06-12 §5.4 · 诊断式增强：coverage BLOCK 时附 DB 真值现状 + 每类缺口
    // 的具体下一步命令（确定性，不赌 reflector LLM）。非 coverage BLOCK 逐字不变。
    let is_coverage_block = decision
        .reasons
        .iter()
        .any(|r| r.contains("GOLISH-INTEL-") || r.contains("never attempted"));
    if let Some(facts) = evidence_facts {
        if let Some(db_status) = build_db_truth_diagnosis(facts) {
            s.push_str(&db_status);
        }
    }
    if is_coverage_block {
        s.push_str("\n### Suggested next commands (run per in-scope asset, replace <asset>)\n");
        for tech in PASSIVE_INTEL_TECHNIQUES {
            if let Some(cmd) = passive_intel_command_hint(tech) {
                s.push_str(&format!("- {tech}: {cmd}\n"));
            }
        }
        s.push_str(
            "\nAfter running these, re-collect evidence and resubmit. The gate measures \
             the DATABASE: a technique counts as covered only once its data is actually \
             persisted (organizations.asns/.certificates, target_assets, dns_records).\n",
        );
    }
    s
```

> 注：原 `build_gate_correction` 结尾是 `s`（隐式返回）。把上面这段插在 `s` 之前、所有现有 push 之后。

---

## 任务 4 · 验证

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(diagnostic_reflector) | test(gate_correction)' 2>&1 | tail
cargo nextest run -p golish-agent-kit 2>&1 | tail -4     # 无回归（584+ 全绿）
cargo clippy -p golish-agent-kit --lib -- -D warnings
cargo fmt -p golish-agent-kit -- --check
```

---

## 自检
- 设计 §5.4 ①DB 现状 → 任务 2 ✅；②缺口 → gate reasons 已有 + 任务 3 命令建议覆盖 ✅；③近期行为 → D-C2 明确不做，记 progress。
- reflector 模型前端配置 → D-C1：既有 `executor.reflect` 机制不改 ✅。
- 占位符：`GateResult` 字段/构造「执行前 Grep 确认」（真未知点）；`build_gate_correction` 调用点能否访问 `gate_ctx` 同标注。
- 类型一致：`build_gate_correction(decision, evidence_facts: Option<&[EvidenceFact]>)` 新签名 + 唯一调用点同步。
- 红线：只读 facts + 静态表，不写库、不改 gate。
