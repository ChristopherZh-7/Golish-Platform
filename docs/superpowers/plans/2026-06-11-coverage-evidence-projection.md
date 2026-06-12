# Coverage = 证据账本投影 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `executing-plans` 逐任务实现此计划。每个任务先写失败测试（TDD），看它失败，再写最小实现，再验证，再 commit。

**目标：** 把 stage coverage 矩阵从「模型手写」改为「harness 对证据账本的确定性投影」；弱模型提交退化为「确认 scope + 交 findings」。先 `target_intel` 单阶段灰度。
**架构：** 三个独立可回滚 PR。PR1 让结构化提交可靠到达 gate（侧信道权威）；PR2 给 evidence 加 `(technique, outcome)` 列并由 recon 工具权威写入（含「跑了→空」行）；PR3 让 `coverage_complete` 从这些证据事实投影出 Found/CheckedEmpty 格。
**技术栈：** Rust 2021、`golish-agent-kit`（gate 纯函数）、`golish-agent-app`（db_bridge / 工具落库）、`golish-db`（sqlx migration + audit repo）、`golish-pentest`（evidence_ledger）、`cargo nextest` + `cargo clippy -D warnings`。
**关联设计：** `docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`（§4 完整性约束、§5.0 目标态、§7 决策：D-store=正经列、D-scope=先 target_intel）。

---

## 0. 背景与现状（动手前先读）

先读仓根 `AGENTS.md`（§2.5 安全语义、§2.7 改 schema 需确认[已授权]、I7/I8/I10）+ 上面那份设计 §4/§5.0。已核对的现状锚点：

- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs:1771`：stage-close 只 `parse_deliverable_from_content(&content)`，**不读工具侧信道**。
- `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs:82-97`：每子任务跑前 `*harness_last_deliverable.write() = None`（重置）；跑后**仅当** content 无 deliverable 签名时才把侧信道 append 回 content。
- `backend/crates/golish-agent-bridge/src/agent_bridge/config.rs:422 harness_last_deliverable_handle()`：暴露侧信道句柄。
- `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs:279 coverage_complete` / `:346-352 derive_from_items`（从 claim/finding 派生 Found）/ `:464 coverage_corroborated`。
- `backend/crates/golish-agent-kit/src/harness/gate/mod.rs GateContext`（`in_scope_assets` / `expected_techniques`，本计划加 `evidence_facts`）。
- evidence 落库点：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs:431` 与 `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs:205`（background job），都走 `repo.evidence_append(op_id, None, session_id, project_path, tool, kind, subject, raw)`。
- `backend/crates/golish-db/src/repo/audit/mod.rs:236 recent_evidence_ids_for_session`（`SELECT id FROM audit_log WHERE audit_role='evidence' AND session_id=$1`）。
- `resources/harness/technique_taxonomy.json`（`GOLISH-INTEL-*` 登记）、`resources/harness/stages/target_intel.json`（gate_rules）。

---

# PR1 · stage-close 认工具侧信道为权威来源（运输鲁棒性，无 migration）

**根因**：模型经 `submit_stage_deliverable` 工具交了结构化交付物（进侧信道），但若它最终文本为空，stage-close 的 content 解析见 `content_len=0` → `missing_deliverable` BLOCK。trait_impl.rs 的 append 守卫（「仅当 content 无签名时 append」）在「content 完全为空」这条路上有缝。

## 任务 1.1 · 让侧信道在 content 为空/无签名时一定回流

**文件：** 改 `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs`（先 Read :82-120 确认现有 append 分支与变量名）。

### 步骤 1.1.1 — 先读真实代码
Read `trait_impl.rs:82-120`，确认 `harness_last_deliverable` 读取分支、`content` 变量、`execution_context.harness_stage` 判定。

### 步骤 1.1.2 — 写失败测试
在该 crate 放一个针对「append 决策」的纯函数化测试。若现有逻辑内联在 async 方法里难测，先抽一个纯函数：

```rust
/// 决定喂给 gate 的最终 content：优先 agent 文本里已有的 deliverable；
/// 否则回退工具侧信道捕获的结构化交付物（append 成 ```json fence）。
/// 二者皆空才返回原文本（让 stage-close fail-closed）。
fn resolve_gate_content(agent_text: &str, side_channel: Option<&str>) -> String {
    if content_has_deliverable_signature(agent_text) {
        return agent_text.to_string();
    }
    match side_channel {
        Some(j) if !j.trim().is_empty() => {
            format!("{agent_text}\n\n```json\n{j}\n```")
        }
        _ => agent_text.to_string(),
    }
}
```
测试：
```rust
#[test]
fn empty_agent_text_falls_back_to_side_channel() {
    let out = resolve_gate_content("", Some("{\"stage_id\":\"target_intel\"}"));
    assert!(out.contains("```json"));
    assert!(out.contains("target_intel"));
}
#[test]
fn agent_text_with_deliverable_wins_over_side_channel() {
    let out = resolve_gate_content("```json\n{\"stage_id\":\"x\"}\n```", Some("{\"stage_id\":\"y\"}"));
    assert!(out.contains("\"x\"") && !out.contains("\"y\""));
}
#[test]
fn both_empty_returns_text_unchanged() {
    assert_eq!(resolve_gate_content("prose only", None), "prose only");
}
```
> `content_has_deliverable_signature` = 把现有「content 无 deliverable 签名」判定抽成纯函数（Read 现有判定逻辑后照抄其语义）。

### 步骤 1.1.3 — 运行确认失败
```bash
cd backend && cargo nextest run -p golish-agent-bridge resolve_gate_content
```
预期：编译失败（函数未定义）。

### 步骤 1.1.4 — 实现：抽纯函数 + 在 `execute_subtask` 调它
把 trait_impl.rs 现有内联 append 逻辑替换为调用 `resolve_gate_content(&content, side_channel.as_deref())`，关键变化 = **content 为空也回退侧信道**（旧逻辑只在「有 content 但无签名」时回退）。

### 步骤 1.1.5 — 验证 + commit
```bash
cd backend && cargo nextest run -p golish-agent-bridge
cargo clippy -p golish-agent-bridge --all-targets -- -D warnings && cargo fmt -p golish-agent-bridge --check
git add backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs
git commit -m "fix(harness): fall back to submit-tool side-channel when agent text carries no deliverable (PR1)"
```

---

# PR2 · evidence 加 `(technique, outcome)` 列 + recon 工具权威写入

> §2.7 已授权改 schema；按 I10「先扩 nullable 列 → 再上写入代码 → 再上读取（PR3）」。

## 任务 2.1 · 核 evidence hash 输入口径（动 schema 前必做）
Read `backend/crates/golish-pentest/src/evidence_ledger/append.rs`（`EvidenceInput` + hash 计算）。确认 hash 输入字段集；新列若**不在** hash 输入 → 零影响；若在 → 仅新行带值、旧行内容不变（加 nullable 列不回填）。把结论记进 `agent-progress.md`。**不写代码，只确认。**

## 任务 2.2 · migration：audit_log 加两列（nullable）
**文件：** 新增 `backend/crates/golish-db/migrations/<next>_evidence_technique_outcome.sql`（先 Glob `backend/crates/golish-db/migrations/*.sql` 看命名序号与既有 ALTER 风格）。

### 步骤 2.2.1 — 写 migration（向后兼容，纯加列）
```sql
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS evidence_technique TEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS evidence_outcome   TEXT;
COMMENT ON COLUMN audit_log.evidence_technique IS 'GOLISH-*/WSTG technique id this evidence proves; NULL for non-evidence rows (coverage projection, design 2026-06-11)';
COMMENT ON COLUMN audit_log.evidence_outcome   IS 'found|empty — whether the technique run produced a result or was run-but-empty; NULL=unknown/legacy';
```

### 步骤 2.2.2 — 验证迁移加载
```bash
cd backend && cargo nextest run -p golish-db
```
预期：迁移在测试 PG 上跑通（沿用现有迁移测试机制）；旧行两列为 NULL。

### 步骤 2.2.3 — commit
```bash
git add backend/crates/golish-db/migrations/
git commit -m "feat(db): add nullable evidence_technique/evidence_outcome to audit_log (PR2, additive/I10-safe)"
```

## 任务 2.3 · `evidence_append` 透传 `(technique, outcome)`
**文件：** `golish-pentest/src/evidence_ledger/append.rs`（`EvidenceInput` 加 `technique: Option<&str>` + `outcome: Option<&str>`，写进 INSERT 的新列）；`golish-agent-kit/src/db_traits/repo.rs` 的 `evidence_append` trait 签名加这两个可空参（default 透传 None 保兼容）；`golish-agent-app/src/ai/db_bridge/evidence.rs` impl 透传。

### 步骤 2.3.1 — 先写失败测试（db_bridge 或 evidence_ledger 层）
断言：`evidence_append(..., technique=Some("GOLISH-INTEL-DNS"), outcome=Some("found"))` 写入后，新增的只读查询能取回该行的 `(technique, outcome)`。

### 步骤 2.3.2 — 运行确认失败 → 实现 → 验证
```bash
cd backend && cargo nextest run -p golish-pentest -p golish-agent-app evidence
cargo clippy -p golish-pentest -p golish-agent-app --all-targets -- -D warnings
```

### 步骤 2.3.3 — commit
```bash
git commit -am "feat(evidence): evidence_append carries technique+outcome into audit_log (PR2)"
```

## 任务 2.4 · recon 工具落库时写入 `(technique, outcome)`（含「跑了→空」）
**文件：** `golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs:431` 附近 + `golish-agent-app/src/ai/commands/bridge_config.rs:205`（background job）。先 Read 两处确认 `effective_tool_name`/结果空判定可得。

### 步骤 2.4.1 — 建「工具→技术」权威映射（保守，纯函数 + 单测）
新增纯函数（放 `golish-agent-kit/src/harness/technique_resolver.rs` 旁或新模块）：
```rust
/// recon 工具/子项 → 被动情报技术类。仅收录无歧义项；未知→None（不派生）。
pub fn recon_tool_technique(tool: &str, sub_kind: Option<&str>) -> Option<&'static str> {
    match (tool, sub_kind) {
        ("dns_resolve", _) | (_, Some("dns_a")) | (_, Some("dns_aaaa")) => Some("GOLISH-INTEL-DNS"),
        ("whois", _) | (_, Some("whois")) => Some("GOLISH-INTEL-WHOIS"),
        (_, Some("ct_log")) => Some("GOLISH-INTEL-CT"),
        ("subfinder", _) | (_, Some("subdomain")) => Some("GOLISH-INTEL-SUBDOMAIN"),
        ("recon_enrich_assets", _) => Some("GOLISH-INTEL-OSINT"),
        _ => None,
    }
}
```
> ⚠️ 此映射只在「无歧义」时返 Some——`recon_enrich_assets` 实际富化多类（DNS/ASN/CT/OSINT），MVP 先归 OSINT；细分留 PR3 后增量（或由 recon_enrich 各子项分别落多条带各自 technique 的 evidence，见设计 §5.3 A1，执行时 Read recon_enrich 产物结构再定）。单测钉死每条映射 + 未知返 None。

### 步骤 2.4.2 — 落库点接线 + outcome 判定
落库时 `technique = recon_tool_technique(...)`；`outcome = if 结果非空 { "found" } else { "empty" }`（「跑了→空」也落行——这是 I8 在数据层的兑现）。`technique=None` 的证据照旧落（不带标注，投影不计）。

### 步骤 2.4.3 — 验证 + commit
```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app
cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
git commit -am "feat(recon): tag passive-intel evidence with technique+outcome incl run-but-empty (PR2)"
```

## 任务 2.5 · 只读投影查询：按 session 取 `(asset, technique, outcome, id)`
**文件：** `golish-db/src/repo/audit/mod.rs`（照 `recent_evidence_ids_for_session` 加一个 `evidence_facts_for_session`，`SELECT subject, evidence_technique, evidence_outcome, id FROM audit_log WHERE audit_role='evidence' AND session_id=$1 AND evidence_technique IS NOT NULL`）；`db_traits/repo.rs` + `db_bridge/evidence.rs` 接线（default 空）。
### 步骤：先写失败测试（写两条带 technique 的 evidence → 查回两条 fact）→ 实现 → 验证 → commit `feat(db): evidence_facts_for_session read-only projection source (PR2)`。

---

# PR3 · coverage_complete 投影 + target_intel 接线 + 活体

## 任务 3.1 · `GateContext` 加 `evidence_facts`
**文件：** `golish-agent-kit/src/harness/gate/mod.rs`（Read 确认 `GateContext` 定义）。加：
```rust
/// 证据投影事实（PR3）：从账本注入的 (asset, technique, outcome) 三元组。
/// None = 不启用投影（逐字节回退旧行为）。
pub evidence_facts: Option<Vec<EvidenceFact>>,
```
```rust
#[derive(Debug, Clone)]
pub struct EvidenceFact { pub asset: String, pub technique: String, pub outcome: EvidenceOutcome, pub evidence_id: i64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceOutcome { Found, Empty }
```
（`Default` 派生 None；现有构造点用 `..Default::default()` 不受影响——先 Read 所有 `GateContext { ... }` 构造点确认。）

## 任务 3.2 · `coverage_complete` 增 `derive_from_evidence`
**文件：** `golish-agent-kit/src/harness/gate/rule_engine.rs`（`GateRule::CoverageComplete` 加 `#[serde(default)] derive_from_evidence: bool`；`coverage_complete()` 签名 + 调用处 :251 同步）。

### 步骤 3.2.1 — 先写失败测试（§4 约束逐条钉死）
```rust
#[test] // 约束1+4：有 found 事实 → 该格视为已覆盖（Found）
fn derive_from_evidence_found_fills_cell() { /* fact{a,DNS,Found,#7} → (a×DNS) 不再缺口 */ }
#[test] // 约束2(I8红线)：empty 事实 → 仅当 terminal 含 CheckedEmpty 时算覆盖；绝不被当 Found
fn empty_fact_is_checked_empty_not_found() { /* fact{a,DNS,Empty} → 若 gate 要 found 仍缺口；要终态则 CheckedEmpty 满足 */ }
#[test] // 约束2：无任何事实 → not_attempted 缺口仍 BLOCK（缺证据≠checked_empty）
fn no_fact_still_blocks() { }
#[test] // 约束4：completeness 不被放宽——缺技术列仍 BLOCK
fn evidence_derive_does_not_fabricate_completeness() { }
#[test] // 兼容：derive_from_evidence=false / evidence_facts=None → 逐字节旧行为
fn disabled_is_byte_identical() { }
```

### 步骤 3.2.2 — 运行失败 → 实现
在 `coverage_complete` 的「格是否覆盖」判定里，于 `declared || derived(from items)` 之外加 `|| derived_from_evidence`：当 `derive_from_evidence` 且 `ctx.evidence_facts` 含 `asset==asset && technique==tech` 的事实时——`Found` 事实 → 该格当 `Found` 终态；`Empty` 事实 → 当 `CheckedEmpty` 终态（受 `terminal_status` 约束，与 declared 同口径）。**绝不**把 Empty 当 Found；**绝不**因缺事实而填格。

### 步骤 3.2.3 — 验证 + commit
```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine
cargo clippy -p golish-agent-kit --all-targets -- -D warnings && cargo fmt -p golish-agent-kit --check
git commit -am "feat(harness): coverage_complete derive_from_evidence projection (PR3, I8-safe)"
```

## 任务 3.3 · live gate hook 注入 evidence_facts
**文件：** `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（照 `fetch_in_scope_target_types_for_gate` 加 `fetch_evidence_facts_for_gate` → `repo.evidence_facts_for_session(chat_session_id)`，组装 `EvidenceFact`，塞进 `GateContext.evidence_facts`；空/出错 → None 回退）。验证 + commit。

## 任务 3.4 · target_intel 开开关
**文件：** `resources/harness/stages/target_intel.json` 的 `coverage_complete` 规则加 `"derive_from_evidence": true`。其余 11 stage 不动（D-scope=灰度）。验证 JSON 合法 + kit 测试全绿 + commit。

## 任务 3.5 · 活体验证 + 登记
```bash
cd backend && cargo run -q -p golish --bin golish -- <Test1 workspace> --stage-run --profile assessment --from scoping --to target_intel --provider xiaomi --model mimo-v2.5-pro --auto-approve --org "默安科技" --target moresec.cn -e "..."   # GOLISH_API_KEY=<用户的key>
grep -a "gate decision: .* stage_id=target_intel\|Operation paused\|evidence_facts" ~/.golish/backend.log | tail
```
预期对照：修前 target_intel pause；修后弱模型凭「确认 + findings + 引用 evidence id」过 gate（coverage 由投影补全）。把命令/退出码/关键输出复制进 `agent-progress.md`「已记录证据」；更新 `feature_list.json`；更新 `docs/modules/` 相关卡。commit `docs(harness): record coverage-projection PR1-3 evidence`。

---

## 自检

**1. 规格覆盖度：** 设计 §5.0 投影模型 → PR3 任务 3.2；§5.1 C → PR1；§5.3 A1 落库 technique → PR2 任务 2.3/2.4；evidence 列(D-store 法1) → PR2 任务 2.2；只读投影源 → PR2 任务 2.5；target_intel 灰度(D-scope) → PR3 任务 3.4；§4 完整性四约束 → PR3 任务 3.2.1 五测逐条钉。全有任务。

**2. 占位符扫描：** 无 TODO/「后续实现」；少数「先 Read <file> 确认签名」是项目既有约定（动前核真实上下文），非空洞占位——每处都给了精确文件:行与意图。

**3. 类型一致性：** `EvidenceFact{asset,technique,outcome:EvidenceOutcome,evidence_id}` / `EvidenceOutcome{Found,Empty}`（任务 3.1 定义）贯穿 3.2/3.3；`recon_tool_technique(tool, sub_kind)->Option<&'static str>`（2.4.1）输出喂 evidence_append 的 technique（2.3）；`evidence_facts_for_session`（2.5）产出喂 `fetch_evidence_facts_for_gate`（3.3）→ `GateContext.evidence_facts`（3.1）→ `coverage_complete`（3.2）。签名贯穿一致。

**边界：** 每 PR 独立可回滚（PR1 不依赖 PR2/3；`derive_from_evidence` 默认 false + evidence_facts=None → 旧行为）；schema 仅加 nullable 列（I10）；不自动造 checked_empty（I8）；动 schema 前先核 hash 输入（2.1）。
