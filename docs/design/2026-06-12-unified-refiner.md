# 统一 Refiner：纠错通道收敛（砍合成兜底，一个入口、确定性分类、每类干净 prompt）

> 日期：2026-06-12
> 状态：设计（用户 2026-06-12 已拍板方向：收敛为单一 refiner 纠错通道；refiner 只产纠正、不自己过 gate；两个合成兜底一并砍）
> 关联：`docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md`（本设计**废止**其投影兜底）、`docs/design/2026-06-12-db-truth-driven-gate-and-diagnostic-reflector.md`（其 §5.4 诊断式 reflector 被本设计吸收为 C 类模板）、`docs/design/2026-06-04-*`（D2 confirm-only 合成，本设计废止）
> 不变量：AGENTS.md I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、§2.5（gate validator 是确定性规则）

---

## 1. 问题（live run 两连实锤，2026-06-12 mimo-v2.5-pro × moresec.cn）

跑 `--stage-run -p xiaomi -m mimo-v2.5-pro --to target_intel --org 默安科技 --target moresec.cn`（含 06-11/06-12 全部 4 个 PR 的新二进制），target_intel 两次 attempt 同因 BLOCK：

```
attempt 1 (log:483-492, 02:11:31):
  MiMo 未调 submit（content_len=852 散文）
  → 投影兜底触发，但可投 facts 只有 2 个 DB 哨兵（evidence_id=0，按红线不产 claim）
  → 合成出空 deliverable → vacuous BLOCK
  → 且投影兜底已把 missing_deliverable 置 false → submit-only 锁被截胡（submit_only=false）
  → vacuous ≠ coverage block → PR-C 诊断命令也不追加
  → 喂回 MiMo 的只剩通用 vacuous 劝说

attempt 2 (log:1341-1353, 02:42:57)：完全同型复发。
  repair 轮 MiMo 明明跑了大量 dig/gau、账本 evidence_id 已推进到 2247，
  但 stage-close 注入的 facts 仍只有 fact_count=2（哨兵）——repair 轮新证据
  没进投影窗口；MiMo 又没 submit（content_len=6331）→ 同样的空投影 → 同因 BLOCK。

attempt 3 / 终局 (log:2717-2742, 03:11:31，总跑 71min)：[PASS] scoping / [BLOCK] target_intel。
  最戏剧性的盖棺证据——MiMo 最后一轮自述（log:2717 response_preview）：
  "I have extensive evidence already collected from background jobs, but I never
   submitted a proper deliverable... evidence IDs for: DNS resolution: 2140, 2143,
   2151... ASN: 2333, 2344. Let me submit the deliverable with ..."
  模型自己数出了真实 ledger ids、自己诊断出「只差提交」——然后真调了 submit，
  交上来的却仍是 claims=0/findings=0/evidence_refs=0 的空壳（log:2724）→ vacuous BLOCK。
  而 submit-only 锁 + 「引用这些真实 ids」的 A 类纠正，本该在 attempt 1 就喂给它
  （它需要的全部信息机制层早就有），却被投影兜底连续三轮挡在门外。
```

**结论**：四套机制（投影兜底 / submit-only 锁 / PR-C 诊断 / 通用纠正）各自局部合理，叠加后形成了「最该上的两个全被挡掉」的交互矩阵：

| 互打 | 机制 A | 机制 B | 结果 |
|---|---|---|---|
| 截胡 | 投影兜底把 `missing_deliverable` 置 false | submit-only 锁前提 `missing_deliverable==true` | 锁永不触发（两连实证） |
| 错位 | PR-C 诊断只认 coverage block（`GOLISH-INTEL-*`/`never attempted`） | 投影必空 → BLOCK 原因恒为 vacuous | 诊断命令永不追加 |
| 大杂烩 | 各 `enforce_*` 用 `{correction}\n\n{prev}` 链式叠加 | 弱模型读到多段混拼文本 | 纠正信号被稀释 |

兜底每加一层，交互矩阵按乘法膨胀。**方向（用户拍板）：砍掉合成类兜底，纠错收敛为一个 Refiner 通道。**

---

## 2. 目标 / 非目标

**目标**：
1. gate BLOCK（含 missing-deliverable）后的**全部纠错**收敛到唯一入口 `Refiner`：确定性分类 → 每类一个独立、干净的 prompt 模板 → 决定是否锁 submit-only。
2. 砍掉两个「后端代为合成 deliverable」的兜底（投影兜底 + confirm-only 合成）——**deliverable 永远出自主 agent 之手**。
3. 废掉链式 correction 拼接：一次 BLOCK 按优先级取**主因**渲染一个模板，次因最多附录一行。

**非目标**：
- **不动 gate 判错**：vacuous / coverage(complete+corroborated+denominator) / fabricated / kinds / freshness / red_team scoping flow / scoping human gate 全部保持。Refiner 收敛的是「判错之后怎么纠」，不是「怎么判错」。
- 不动重试循环骨架（`MAX_REFLECTOR_RETRIES=3`、BLOCK→重喂、耗尽→final BLOCK）。
- 不动 DB 真值哨兵 facts（PR-A）的注入与「哨兵不产 claim/不进 evidence_refs」红线——它属于 gate 判错侧。
- 不引入新 LLM 调用：Refiner 默认纯模板（快、稳、可单测）。06-12 §5.4 拍板的「reflector 模型走 settings 配置」保留为 C 类的**可选**增强位，本期不实现。

---

## 3. 现状勘查（已核对源码，行号为 2026-06-12 工作树）

纠错/兜底共 6 处，散在 `execute.rs`（3567 行）：

| # | 机制 | 位置 | 去向 |
|---|---|---|---|
| 1 | 文本 reflector（`looks_like_text_only_response` → `executor.reflect()` LLM 产纠正） | execute.rs:248-259, 217-243; helpers.rs:36; prompts/pipeline.rs:9 | **归并**为 Refiner F 类（模板化，砍 LLM 调用） |
| 2 | gate BLOCK 重试 + `build_gate_correction` 通用纠正 | execute.rs:186-319, 2144-2207 | **归并**为 Refiner 入口 + B/C 类模板 |
| 3 | PR-C 诊断增强（仅 coverage block 触发 DB 真值现状 + 命令 hints） | execute.rs:2182-2205 | **归并**进 C 类模板，触发扩展到 vacuous |
| 4 | submit-only 通道（`refine_missing_deliverable_correction` + `build_submit_only_correction` + `harness_submit_only` tool_choice 锁） | execute.rs:1401-1441, 1656-1679, 184-194 | **保留**为 Refiner A 类（锁是机制级约束，实测有效） |
| 5 | confirm-only 合成 deliverable（D2） | execute.rs:1875-1883, 2262-2297 | **砍**：missing → A 类 submit-only 锁的「无证据变体」 |
| 6 | 投影兜底 `synthesize_from_evidence`（06-11） | execute.rs:1884-1933, 2318-2360; stage_spec.rs:120 开关; target_intel.json | **砍**：missing 保持 missing，走 A/B 类 |
| - | 各 `enforce_*` 的链式 correction 拼接（`{correction}\n\n{prev}`） | execute.rs:1487-1490, 1534-1537, 1595-1598, 1645-1648 | **废**：enforce_* 只置「事实标记」，渲染统一交给 Refiner |

附：gate 之后的 PASS→BLOCK 翻转校验（`enforce_evidence_existence/kinds/freshness/scoping_red_team_flow`）是**判错**，全部保留；只是它们不再各自手搓 correction 文本。

---

## 4. 红线（任一违反即否决实现）

1. **gate 判错零改动**：分类器只消费 gate 已产出的事实（reasons / missing_deliverable / fabricated ids / 缺失 kinds / 过期 ids），不重新判定。
2. **deliverable 永远出自主 agent**：后端不再代为合成任何 StageDeliverable（两个 synthesize_* 全砍）。submit-only 锁逼出来的也算 agent 之手——它经过 agent 的 LLM 决策与 submit 工具侧信道。
3. **findings 永不由后端产生**（沿用既有红线，随合成兜底一并消失——无合成即无投影 findings 的问题）。
4. **I8 不破**：砍投影兜底不影响 checked_empty 语义（它由账本「跑了→空」真实 outcome 产生，在 gate 判错侧）。
5. **fail-closed 不破**：missing-deliverable 依旧 BLOCK（`missing_deliverable_gate_outcome` 保留）；重试耗尽依旧 final BLOCK。Refiner 只改「喂回去的纠正长什么样」。

---

## 5. 设计

### 5.1 唯一入口 + 确定性分类

新模块 `task_orchestrator/refiner.rs`。gate（+ enforce_* 翻转）结束后，BLOCK 时调用：

```rust
pub(super) struct RefineInput<'a> {
    pub stage: StageKind,
    pub gate_reasons: &'a [String],          // gate decision.reasons
    pub recovery: Option<&'a RecoveryActions>,
    pub missing_deliverable: bool,
    pub fabricated_ids: &'a [i64],
    pub available_real_ids: &'a [i64],       // 账本真实 ids（newest first）
    pub evidence_kinds: &'a HashMap<i64, String>,
    pub missing_kinds: &'a [String],         // enforce_evidence_kinds 标记
    pub expired_ids: &'a [i64],              // enforce_evidence_freshness 标记
    pub red_team_flow_correction: Option<&'a str>, // enforce_scoping_red_team_flow 标记
    pub confirm_only_stage: bool,            // spec.allowed_tool_types.is_empty()
    pub evidence_facts: Option<&'a [EvidenceFact]>, // C 类诊断用（含 DB 哨兵）
    pub text_only: bool,                     // F 类（gate 前的文本响应检测）
}

pub(super) struct RefineDecision {
    pub class: RefineClass,      // 日志/HarnessTrace 用
    pub correction: String,      // 渲染好的单模板文本
    pub submit_only_lock: bool,  // 是否锁 tool_choice = submit
}
```

分类是**纯函数 match，按危害优先级取主因**（一个 BLOCK 多原因时只渲染最高优先级模板，其余压成附录一行 `Also fix: ...`）：

| 优先级 | 类 | 判定条件（全部来自 RefineInput 事实） | 动作 |
|---|---|---|---|
| 1 | **D · Fabricated** | `!fabricated_ids.is_empty()` | 防伪造模板（列真实 ids），不锁 |
| 2 | **A · SubmitOnly** | `missing_deliverable &&（confirm_only_stage ‖ !available_real_ids.is_empty()）` | submit-only 模板 + **锁** |
| 3 | **B · RedoStage** | `missing_deliverable` 且账本空且非 confirm-only | 重做模板（stage charter + 必跑工具），不锁 |
| 4 | **C · CoverageOrVacuous** | reasons 含 vacuous / `GOLISH-INTEL-*` / `never attempted` / corroborated | 诊断式模板（DB 真值现状 + 每类命令），不锁 |
| 5 | **E · EvidenceQuality** | `!missing_kinds.is_empty() ‖ !expired_ids.is_empty()` | 缺 kind / 过期模板，不锁 |
| 6 | **G · ScopingFlow** | `red_team_flow_correction.is_some()` | 透传该纠正（已是针对性文本） |
| 7 | **F · TextOnly** | `text_only`（gate 前路径） | 「停止散文、调工具」模板，不锁 |
| 8 | 兜底 | 其它 | 通用模板（现 build_gate_correction 的素体） |

要点：
- **A 类吸收 confirm-only**（砍 #5 后）：confirm-only stage 的 missing → 模板说「该阶段无扫描工具，你唯一动作是调 `submit_stage_deliverable` 提交确认型 claim（evidence_ids 可为空）」+ 锁。后端不再代填。
- **A 类的 ids 列表**：沿用 `recent_evidence_ids(sid, 25)` + kind 标签（`#2247 (dns_a)`）。**修复投影兜底截胡 bug 的方式就是删掉投影兜底**——missing 保持 missing，A 类自然触发。
- **C 类扩展**：`build_db_truth_diagnosis`（已实现）+ `PASSIVE_INTEL_TECHNIQUES` 命令 hints 从「仅 coverage」扩到 vacuous——正是两连 BLOCK 中缺失的那块。
- **F 类模板化**：砍 `executor.reflect()` 的 LLM 调用（PentAGI Reflector 模式），text-only 检测保留，纠正改为确定性模板。`reflect()` trait 方法与 `reflector_system_prompt` 标记 deprecated，暂不删（trait 是公共接口，删除另起 PR）。

### 5.2 每类一个独立干净模板

`refiner.rs` 内每类一个 `fn render_<class>(input) -> String`，禁止跨类拼接。现有文本素材直接迁移：
- A ← `build_submit_only_correction`（+ confirm-only 变体新增）
- B ← `missing_deliverable_gate_outcome` 的 correction
- C ← `build_gate_correction` 素体 + `build_db_truth_diagnosis` + 命令 hints
- D ← `block_outcome_for_fabricated` 的 correction
- E ← `enforce_evidence_kinds` / `enforce_evidence_freshness` 的 correction
- F ← 新写（≤120 词：你只输出了文本、没调工具；列本 stage 可用工具；下一步调哪个）

enforce_* 改为**只置事实标记**（`outcome.missing_kinds` / `outcome.expired_ids` / `outcome.red_team_flow_correction`），不再自己 `format!` 拼 correction——渲染权全部上收 Refiner。

### 5.3 砍掉清单（随 PR-R2/R3 删除）

| 删除物 | 位置 |
|---|---|
| `synthesize_from_evidence` + 其 6 个单测 | execute.rs:2318-2360, 3351-3489 |
| `apply_harness_gate_hook` 投影分支 | execute.rs:1884-1933（收敛为 `missing_deliverable_gate_outcome`） |
| `stage_spec.synthesize_from_evidence_when_missing` 字段 + 2 个单测 | stage_spec.rs:117-120, 378-411 |
| `target_intel.json` 的 `"synthesize_from_evidence_when_missing": true` | resources/harness/stages/ |
| `synthesize_confirm_only_deliverable` + confirm-only 分支 | execute.rs:1865-1883, 2262-2297 |
| `executor.reflect()` 调用点（LLM 反思路径） | execute.rs:212-243 |
| 06-11 投影兜底设计文档头部加 `> Superseded by 2026-06-12-unified-refiner.md` | docs/design/ |

### 5.4 重试循环的接线变化（execute.rs）

```text
旧：gate → enforce_*(各自拼 correction) → refine_missing_deliverable_correction(可能被截胡)
    → pending_gate_correction = outcome.repair_correction
新：gate → enforce_*(只置标记) → RefineDecision = refiner::classify_and_render(RefineInput)
    → pending_gate_correction = decision.correction
    → pending_submit_only   = decision.submit_only_lock
    → HarnessTrace 记 decision.class（可观测：本次喂的是哪类纠正）
```

两个 gate 调用点（execute.rs:268 循环内 / :431 耗尽后）共用。耗尽后那次只记 trace 不重试（现状不变）。

---

## 6. 影响面

| 文件 | 改动 | 风险 |
|---|---|---|
| `task_orchestrator/refiner.rs`（新） | 分类器 + 7 类模板 + 单测 | 低（纯函数） |
| `subtask_phases/execute.rs` | 接线替换；删两个 synthesize_* 与投影分支；enforce_* 改置标记 | 中（核心循环，TDD 全覆盖） |
| `harness/stage_spec.rs` | 删 `synthesize_from_evidence_when_missing` | 低 |
| `resources/harness/stages/target_intel.json` | 删开关字段 | 低 |
| `task_orchestrator/types.rs` / `prompts/pipeline.rs` | `reflect()` / reflector prompts 标记 deprecated | 低 |
| `HarnessGateOutcome` | 增 `missing_kinds` / `expired_ids` / `red_team_flow_correction` 标记字段 | 低 |

不动：gate rule_engine、coverage 三 gate、DB 真值注入（stage-close hook 查业务表那段）、submit 工具侧信道、HarnessTrace schema 之外的部分。

---

## 7. 决策记录

- **D1 · 收敛范围** ✅（用户 2026-06-12）：6 个纠错/兜底收敛为 Refiner 单通道；gate 判错零改动。
- **D2 · refiner 不自己过 gate** ✅（用户接受）：refiner 只产纠正 + 锁决策；deliverable 永远出自主 agent。
- **D3 · 两个合成兜底都砍** ✅（用户「同意」整案）：confirm-only 合成 → A 类锁的无证据变体。
- **D4 · F 类模板化（砍 reflect() LLM 调用）**：推荐砍——与「确定性、干净」哲学一致，且 reflect() 用的还是主 agent 同一个弱模型，纠正质量无保证。**待用户确认**（若想保留 LLM 反思，可作为 C/F 类的可选增强位）。
- **D5 · 分类优先级**（§5.1 表）：fabricated > submit-only > redo > coverage/vacuous > quality > flow > text-only。依据：危害大小 + 实测频率。**待用户确认**。

---

## 8. 风险与缓解

- **confirm-only stage 死锁回归**（砍 D2 合成后弱模型连确认都不交）：A 类锁 tool_choice=submit 实测能逼出 submit 动作（本次 live run attempt 3 锁生效、MiMo 交了 deliverable）；3 次耗尽仍不交 → final BLOCK 是正确的 fail-closed 语义。scoping 是 pipeline 入口，若实测死锁率不可接受，回退方案是仅对 scoping 恢复合成（一行 revert PR-R3）。
- **砍投影兜底后「干了活但 3 次都不肯 submit」直接 BLOCK**：这正是设计意图——A 类锁给了 3 次「只许 submit」的机会，仍不交说明模型不可用，假装它交了（投影）只会把垃圾放行到下游 stage。
- **模板取主因可能漏次因**：附录行 `Also fix: <次因一句话>` 保底；HarnessTrace 记完整 reasons，可观测不丢失。
- **行为对照回归**：PR-R1 先做纯接线（模板内容≈现状），活体对照（同 profile/model/target）确认无回归后再上 R2/R3 砍除。

---

## 9. 分阶段路线（每 PR 独立可回滚、TDD 全绿）

- **PR-R1 · Refiner 骨架**：refiner.rs（分类器 + 模板迁移 + 单测）；execute.rs 接线替换 + enforce_* 改置标记；C 类诊断扩展到 vacuous（两连 BLOCK 的直接止血）。行为上 ≈ 现状 + 修复两处互打。
- **PR-R2 · 砍投影兜底**：§5.3 表第 1-4 行删除；06-11 设计文档标 Superseded。
- **PR-R3 · 砍 confirm-only 合成**：A 类无证据变体接管；§5.3 表第 5 行删除。
- **PR-R4 · F 类模板化**：text-only 路径走 Refiner；reflect() 调用点删除、trait 标 deprecated。
- **活体收口**：同命令对照跑（xiaomi/mimo × moresec.cn），验证 ① missing 时 submit-only 锁真触发（日志 `submit_only=true`）② vacuous BLOCK 的纠正含 DB 诊断 + 命令 ③ 无投影合成日志。证据落 `agent-progress.md`。

---

## 10. 验证计划（TDD）

- 分类器单测：8 类各给最小 RefineInput 断言 class/lock；多原因并存时主因优先级正确；附录行存在。
- 模板单测：每类模板含其关键要素（A 含真实 ids + 锁语义；C 含 DB 诊断段 + 命令 hints——vacuous 与 coverage 两种入口都触发；D 含 fabricated ids；F 不含「重做整个 stage」字样）。
- 接线回归：missing + 账本有证据 → `pending_submit_only=true`（修复截胡）；missing + 账本空 → B 类；confirm-only missing → A 类锁（PR-R3 后）。
- 删除回归：`synthesize_from_evidence*` 全工作区 0 引用；target_intel spec 解析不再有该字段；`just precommit` 全绿。
