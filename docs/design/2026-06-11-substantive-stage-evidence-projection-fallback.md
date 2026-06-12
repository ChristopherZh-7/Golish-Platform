# Substantive 阶段交付物：证据投影兜底（弱模型不调 submit 的救援）

> Superseded by `docs/design/2026-06-12-unified-refiner.md`（PR-R2 已删除本投影兜底：
> 2026-06-12 live run 两连实证它把 `missing_deliverable` 置 false、截胡 submit-only 锁；
> missing-deliverable 现保持 fail-closed BLOCK，由统一 Refiner 的 A/B 类纠正驱动主
> agent 自己提交，deliverable 永远出自 agent 之手）。

> 日期：2026-06-11
> 状态：已废止（原状态：设计（待用户审 §9 决策表 → 再写实现计划））
> 作者：BaJie MCP-agent-3
> 关联：`docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`（PR1-3 已实现，本设计是其 §5.0 投影模型的逻辑终点）、`docs/design/2026-06-11-weak-model-submit-channel.md`、`docs/design/2026-06-02-submit-stage-deliverable-tool.md`
> 不变量：AGENTS.md I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、§2.5（安全语义变更）

---

## 1. 问题（live run 实证，2026-06-11 mimo-v2.5-pro × moresec.cn）

弱模型在 `target_intel` 反复 BLOCK，三次 repair（`MAX_REFLECTOR_RETRIES=3`）耗尽仍过不去。本轮在 **OpenAI 与 Anthropic 两个协议端点**都跑了对照，剖出同一个根本病的两种表现：

| 端点 | 通道 | submit 行为 | 结果 |
|---|---|---|---|
| OpenAI 兼容 | XML 文本式（recovery 救） | **根本不调** `submit_stage_deliverable`——产纯文本「汇报」或调 `update_plan`/`sub_agent_memorist` | stage-close 解析文本无 deliverable → BLOCK |
| Anthropic 兼容 | 原生 tool_use | **调了但填空 `{}`**（32 次全 `args_len=2`）→ 工具 `rejected: missing field stage_id` | 死循环（无熔断） |

逐字证据（OpenAI run transcript `completed[1]`，13:15:27 BLOCK 那一刻的 957 字 response）：

> I'll start the `target_intel` stage… Enrichment collected 46 targets with **evidence_id 1835**… Let me delegate to the pentester for subfinder, dns_resolve, and whois… Let me try **submitting the deliverable**… **Submitting the stage deliverable now.**

它口头说「正在提交」，但 `tool_calls=0`——**把「叙述要提交」当成了「提交」**。reflector 官方定性：`Diagnosed OVERTHINKING failure — agent analyzed evidence but didn't call submit_stage_deliverable`。

**关键：活干完了、证据都进了账本。** 这次 run 的 evidence ledger（session `stage-run-56e56ae8…`）有 13 条 evidence，**11 条带 `(technique=GOLISH-INTEL-DNS, asset=<子域>, outcome∈{found,empty})`**（PR2 已落库，含 `mshoneypot.moresec.cn=empty` 这种「跑了→空」行）。卡的只是「弱模型把现成证据打包成一份 deliverable 并调 submit」这一下动作。

### 为什么 substantive 阶段会死锁（设计有意）

`task_orchestrator/subtask_phases/execute.rs`（~L1828）的 stage-close 分支：

```
parse_deliverable_from_content(&content) == None 时：
├─ confirm-only stage（spec.allowed_tool_types 为空，如 scoping/reporting）
│    → synthesize_confirm_only_deliverable()  合成最小 deliverable，不死锁  ✅
└─ substantive stage（有 allowed_tool_types，如 target_intel/EAS/enumeration）
     → missing_deliverable_gate_outcome()  fail-closed BLOCK
        （注释：「their findings must never be fabricated」）
```

`synthesize_confirm_only_deliverable`（execute.rs:2108）只对 confirm-only 阶段兜底，且合成的是**占位 claim + 空 evidence_refs + 空 findings**。substantive 阶段故意不兜底——怕凭空伪造 findings。

于是矛盾：**substantive 阶段设计上必须 agent 亲自 submit（防伪造 findings），但弱模型不会调 submit → 死锁。**

前身设计 `2026-06-11-coverage-auto-derive-from-evidence.md` §8 已预见这个「能力下限」，但当时只给了「必要时换强模型」。本设计给一个**不换模型**的解法。

---

## 2. 目标

当 substantive **情报/枚举类**阶段的 agent 没产出可解析 StageDeliverable，但**账本里已有该 run 的真实 evidence facts** 时，由 harness **从证据账本投影合成一份 deliverable**（claims + coverage + evidence_refs 来自真实 evidence，**findings 留空**），再走同一套 gate，而非直接 BLOCK 死锁。

这是前身 §5.0「coverage = 证据账本只读投影」的逻辑终点：既然 coverage 已能从 evidence 投影（PR3 `derive_from_evidence`），claims/evidence_refs 同样能投影；那么当模型连 submit 都不调时，harness 完全可以从账本投影出整份 deliverable 的「事实部分」，不必依赖弱模型那一下动作。

### 非目标（明确排除）

- **不投影 findings**（漏洞断言）。findings 必须 agent 真报——这是 §1 注释「findings must never be fabricated」的红线，本设计严格守住。
- **不对漏洞类阶段开**（vuln_triage 等 finding-producing 阶段保持 fail-closed BLOCK）。
- **不削弱任何 gate 校验**：投影出的 deliverable 过同一套 completeness/corroborated/denominator/evidence-existence 校验。
- **不自动造 `checked_empty`**（沿用 PR2/PR3：`empty` 只来自真实「跑了→空」evidence 行，不从「缺证据」推断）。
- 不改 evidence schema/hash（PR2 已落 `evidence_technique/evidence_outcome` 列，本设计只读）。
- **不做 Anthropic 端点熔断**（用户 2026-06-11 决定不管 Anthropic 端点，聚焦 OpenAI 生产默认）。Anthropic 端点的「调 submit 填空 → rejected 死循环」不在本设计范围——见 §10。本设计只针对 **OpenAI 端点**「agent 产纯文本不调 submit → stage-close BLOCK」这条路（loop 因产纯文本自然终止，直达 stage-close，投影兜底即生效，无需熔断）。

---

## 3. 现状勘查（动手前先读，已核对 2026-06-11）

| 机制 | 位置 | 现状 |
|---|---|---|
| stage-close 解析 + 兜底分支 | `execute.rs:1828` | `parse_deliverable_from_content==None` → confirm-only 合成 / substantive BLOCK |
| confirm-only 判定 | `execute.rs:1838` | `load_embedded_stage_spec(kind).allowed_tool_types.is_empty()` |
| confirm-only 合成 | `execute.rs:2108 synthesize_confirm_only_deliverable` | 占位 claim、空 evidence_refs/findings/coverage |
| substantive BLOCK | `execute.rs:2149 missing_deliverable_gate_outcome` | `gate_allowed=false` + repair_correction + `missing_deliverable=true` |
| **evidence facts 通路（PR3，已存在）** | `execute.rs:1240 fetch_evidence_facts_for_gate` | `repo.evidence_facts_for_session(sid)` → `Vec<EvidenceFact>`，已在 gate hook 注入 `GateContext.evidence_facts` |
| EvidenceFact 结构 | `harness/gate/rule_engine.rs:196` | `{ asset, technique, outcome: Found/Empty, evidence_id }` |
| coverage 投影（PR3） | `rule_engine.rs:316 derive_from_evidence` | `coverage_complete` 已能从 `ctx.evidence_facts` 投影 Found/CheckedEmpty 格 |
| 只读查询 | `db_traits/repo.rs:331 evidence_facts_for_session` | 按 session 取 `(asset, technique, outcome, id)` |
| StageDeliverable 必填字段 | `harness/types.rs:213` | `stage_id/stage_run_id/claims/evidence_refs` 无 serde default（空 `{}` 解析失败=Anthropic 填空死循环根因） |

**关键现状**：投影所需的基础设施（`EvidenceFact`、`fetch_evidence_facts_for_gate`、`derive_from_evidence`）PR3 已全部就位。本设计主要是**新增一个「从 evidence facts 合成 deliverable」的函数 + 改 execute.rs:1862 的 substantive BLOCK 分支为「先尝试投影兜底」+ 一个 opt-in 门控**。

---

## 4. 完整性约束（核心，继承前身 §4 并加 findings 红线）

投影兜底必须满足以下约束，**任一不满足则不兜底，回退原 fail-closed BLOCK**：

1. **只从真 evidence facts 投影（保 I7）**：合成 deliverable 的 claims/coverage/evidence_refs 全部来自 `fetch_evidence_facts_for_gate` 取回的真实账本行（`audit_role='evidence'` 且 `evidence_id` 真实存在）。fabricated-ref 存在性校验（`enforce_evidence_existence`）照跑——投影的 id 本就来自账本，必过。
2. **findings 永远留空（新红线，保「不伪造 findings」）**：投影**只产 claims + coverage（Found/CheckedEmpty）+ evidence_refs**，`findings: vec![]`。漏洞断言不可投影。
3. **CheckedEmpty 只来自真实 `empty` 行（保 I8）**：复用 PR3 `derive_from_evidence` 同款语义——有 `found` 行→Found，有 `empty` 行→CheckedEmpty，**无行→not_attempted（缺口，gate 照常 BLOCK）**。投影补不出完整性。
4. **completeness/corroborated/denominator 三 gate 不动**：投影只是把「证据派生的格」喂进 gate，随后跑**同一套**校验。缺证据的 (资产×技术) 仍判缺口 BLOCK——**投影只能让「真做了的事实」过关，补不出没做的覆盖**。
5. **空账本不兜底**：`fetch_evidence_facts_for_gate` 返回 `None` 或空 → 回退原 BLOCK（没证据=真没干，该 BLOCK）。

> 一句话保证：投影兜底 = 「把弱模型本该照着账本手抄的 deliverable 事实部分，改成 harness 确定性地抄；findings 仍留给 agent 真报」。证据是同一批真 id，gate 是同一套校验，findings 红线不破。**比让弱模型手填更可信（来源是确定性账本，不是自然语言）。**

---

## 5. 设计

### 5.1 触发条件（execute.rs:1862 substantive 分支改造）

```
parse_deliverable_from_content(&content) == None && !confirm_only 时：
  若 spec.synthesize_from_evidence_when_missing == true            // §7 opt-in 门控
     && let Some(facts) = fetch_evidence_facts_for_gate(planned)   // PR3 已有
     && !facts.is_empty():
        deliverable = synthesize_from_evidence(stage_kind, &facts, exec_ctx)
        // 落 warn 日志（投影兜底，非 agent 提交），继续走 gate
  否则:
        原 missing_deliverable_gate_outcome() BLOCK（fail-closed 不变）
```

### 5.2 投影函数（新增，纯函数）

```rust
/// 从账本真实 evidence facts 投影一份「事实部分」deliverable。
/// findings 永远空（红线）；claims/coverage 来自 facts；evidence_refs = facts 的真实 id。
fn synthesize_from_evidence(
    stage: StageKind,
    facts: &[EvidenceFact],
    exec_ctx: &ExecutionContext,
) -> StageDeliverable
```

- **claims**：对每条 `outcome==Found` 的 fact 产一条 claim `{ kind: technique 的语义名, subject: fact.asset, summary: "backend-projected from evidence #<id>", evidence_ids: [fact.evidence_id], technique: Some(fact.technique) }`。
- **coverage**：直接交给 gate 的 `derive_from_evidence`（PR3）从 `ctx.evidence_facts` 投影——投影函数自身**不必**手填 coverage（留空数组，让 gate 投影），或显式镜像一份；二者等价，倾向「留空 + 靠 gate 投影」最 DRY。
- **evidence_refs**：facts 里所有 `evidence_id` 去重。
- **findings**：`vec![]`（红线）。
- **stage_id/stage_run_id**：harness 注入（`stage.as_str()` + 新 uuid）——这也顺带解决了「弱模型填不出 stage_id 必填字段」。

### 5.3 与 gate 的衔接

合成出的 deliverable 走**原封不动**的 `validate_stage_gate_with_context`：
- `coverage_complete(derive_from_items + derive_from_evidence)` 用 `ctx.evidence_facts` 投影 + 校验完整性 → 缺证据的格仍 BLOCK。
- `coverage_corroborated` 用投影 claims（带 technique+subject）佐证 Found 格 → 因 claims 与 coverage 同源（都来自 facts），天然对齐。
- evidence-existence / freshness 照跑。

投影只是替弱模型「打包」，gate 仍是终判。**投影兜底不等于必过**——证据不足照样 BLOCK。

---

## 6. 安全性论证（不破 I7/I8/不伪造 findings）

| 担忧 | 为什么不破 |
|---|---|
| 伪造 findings？ | findings 永远 `vec![]`（§4.2 红线）。投影不碰漏洞断言。 |
| 凭空造覆盖？ | claims/coverage 只从真实 evidence facts 投影；无 fact 的格 = not_attempted = gate BLOCK（§4.4）。投影补不出没做的覆盖。 |
| 自动造 checked_empty（破 I8）？ | CheckedEmpty 只来自真实 `empty` 行（PR2 落库的「跑了→空」），不从「缺证据」推断（§4.3）。 |
| evidence_refs 造假？ | evidence_id 来自账本 `evidence_facts_for_session`，存在性校验必过（§4.1）。比模型自报更可信。 |
| 绕过「agent 必须自己干」？ | 投影只在 agent **已经把工具跑完、证据进账本**后兜底「打包」动作。没跑工具→空账本→不兜底→BLOCK（§4.5）。它救的是「打包」不是「干活」。 |

---

## 7. 边界 gating（哪些阶段能投影兜底）

**opt-in spec 字段**（最安全、逐阶段灰度，沿用 PR3 `derive_from_evidence` 的 opt-in 风格）：

`resources/harness/stages/<stage>.json` 新增 `"synthesize_from_evidence_when_missing": true`（`#[serde(default = false)]`，逐字节向后兼容）。

- **先只对 `target_intel` 开**（情报阶段，findings 常空，投影安全）。
- **漏洞类阶段（vuln_triage 等）永不开**——它们必须 agent 真报 finding，保持 fail-closed BLOCK。
- EAS/enumeration 验稳 target_intel 后再逐个评估（它们也是情报/枚举类，findings 通常空，但要确认无 finding-producing 语义）。

> 为什么用 opt-in spec 字段而非按 StageKind 硬编码：与 PR3 一致、逐阶段灰度可控、回退一行（删字段）、不在代码里写死「哪些 stage 是情报类」的脆弱分类。

---

## 8. 影响面

| crate / 文件 | 改动 | 风险 |
|---|---|---|
| `golish-agent-kit` `harness/stage_spec.rs` | `StageSpec` 加 `synthesize_from_evidence_when_missing: bool`（serde default false） | 低（加性、可空、有守卫测试） |
| `golish-agent-kit` `execute.rs:1862` substantive 分支 | 改为「门控 + 有 facts → 投影；否则原 BLOCK」 | 中（核心 stage-close，TDD 覆盖） |
| `golish-agent-kit` `execute.rs` 新增 `synthesize_from_evidence` | 纯函数：facts → deliverable（findings 空） | 中（TDD 全覆盖：findings 必空、claims 来自 facts、空 facts 不兜） |
| `resources/harness/stages/target_intel.json` | 开 `synthesize_from_evidence_when_missing: true`（单阶段灰度） | 低 |
| （可选）熔断兜底 | submit 连续 rejected/不调 N 次 → 转 BLOCK，治 Anthropic 32 次空提交死循环 | 中（独立小改，见 §10 挂账） |

**ts-rs/IPC**：`StageSpec`/`StageDeliverable`/`EvidenceFact` 是 harness 内部类型，不跨 IPC（前身设计已核 `rg ts_rs` 0 命中），无同步义务。实现时复核一次。

---

## 9. 待决策（请用户拍板）

- **D1 · 门控方式**：opt-in spec 字段 `synthesize_from_evidence_when_missing`（推荐，§7）vs 按 StageKind 硬编码分类。倾向前者。
- **D2 · coverage 来源**：投影函数留空 coverage 数组、靠 gate 的 `derive_from_evidence` 投影（DRY，推荐）vs 投影函数显式镜像一份 coverage。
- **D3 · claims 粒度**：每条 Found fact 一条 claim（推荐，便于 corroborated 对齐）vs 按 (asset×technique) 聚合。
- ~~D4 · 是否同时做熔断~~ **已定：不做**（用户 2026-06-11「不管 Anthropic 的事情」）。聚焦 OpenAI 端点，OpenAI 端点产纯文本即终止 loop → 直达 stage-close → 投影兜底生效，不需要熔断。
- **D5 · 灰度范围**：先只 target_intel（推荐）vs 一次性给所有情报/枚举类阶段开。

---

## 10. 风险与缓解

- **投影掩盖「没测全」**：投影只产有真证据的 Found/Empty 格；completeness gate 不动 → 缺证据的格仍 BLOCK。投影救「打包」，救不了「漏测」。
- **某情报阶段其实会产 finding**：opt-in 门控逐阶段评估；漏洞类永不开；findings 红线兜底（投影永不产 finding，模型漏报 finding 是另一个问题，不因本设计加剧）。
- **弱模型连工具都没跑**（空账本）：不兜底 → BLOCK（§4.5）。本设计不降低「必须真跑工具」的门槛。
- **Anthropic 端点 32 次空提交死循环（out-of-scope）**：用户 2026-06-11 决定不管 Anthropic 端点，本设计聚焦 OpenAI 生产默认。Anthropic 端点的死循环（submit 反复填空 rejected、到不了 stage-close）如未来要治，另开熔断设计——不在本设计范围。OpenAI 端点不存在这个问题（产纯文本即终止 loop → 直达 stage-close → 投影兜底生效）。
- **灰度回滚**：删 spec 字段一行回退；默认 false 即旧 fail-closed 行为，逐字节兼容。

---

## 11. 验证计划（实现阶段，TDD）

- **纯函数单测** `synthesize_from_evidence`：findings 必空；claims 来自 facts 且 technique/subject 对齐；evidence_refs = facts 真实 id 去重；空 facts → 不调用（上层门控）。
- **stage-close 分支单测**：门控 off → 原 BLOCK 逐字节不变；门控 on + 有 facts → 投影并过 gate；门控 on + 空账本 → 仍 BLOCK；漏洞类阶段（门控 off）→ 仍 BLOCK。
- **gate 集成**：投影 deliverable 过 `coverage_complete`(derive_from_evidence) + `coverage_corroborated`；缺证据的格仍 BLOCK（投影补不出完整性）。
- **活体**（弱模型 + target_intel + moresec.cn，复现本轮场景）：对照修前（agent 不调 submit → pause）→ 修后（账本有 evidence → 投影兜底过 gate）。证据落 `agent-progress.md`。
- 收口：`just precommit` 全绿；`code-audit` 复核 findings 红线确未被投影触碰。
