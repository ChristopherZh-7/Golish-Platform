# 2026-06-02 · `submit_stage_deliverable` 工具设计

> 把「阶段交付物（StageDeliverable）」的提交从「LLM 在最终消息里打印 ```json」改成一个**确定性工具调用**，让交付与具体模型的散文习惯、以及「主 agent vs 子 agent 谁来给」彻底解耦。
>
> 关联：`docs/design/2026-05-20-agent-harness-strategy.md`（内层 harness）、`backend/crates/golish-agent-kit/src/harness/`（gate / StageDeliverable / evidence ledger）。本设计是该 harness 的「交付通道」加固。

---

## 1. 背景与问题（实测根因）

当前阶段「完成」的判定链（已核实）：

1. subtask 打 `harness_stage` 标。
2. agent 在**最终消息文本**里输出一份 `StageDeliverable`（```json 块）。
3. `apply_harness_gate_hook`（`subtask_phases/execute.rs:920`）用 `parse_deliverable_from_content`（同文件 `:1181`）从**该文本**里解析。
4. 解析到 → `validate_stage_gate` 跑确定性 check + `enforce_evidence_existence/kinds` 对 DB 账本交叉验证 → PASS 则 cursor 前进（阶段完成）。
5. **解析不到 → `gate skipped`，cursor 不前进**（阶段不算完成）。

**实测问题**（session `pentest-chat-1780401146315-1`，mimo / deepseek 均复现）：

- 主 orchestrator 是「纯协调者」，习惯把交付物**委派给 `sub_agent_reporter`**，自己最终消息写**散文**（「reporter 已编译 deliverable…」）。
- `StageDeliverable` 落在 reporter 子 agent 的 `submit_result`（barrier）里，**不在 orchestrator 最终文本**。
- gate 只读 orchestrator 文本 → `harness gate skipped: ... no parseable StageDeliverable JSON block`（`backend.log` 11:59:27 Scoping、12:06:52 ExternalAttackSurface 两处实证）。
- reflector 进一步「涌现式」误导：建议「去用 sub_agent_reporter 提交」，把模型推离「自己打印 JSON」。

**结论**：根因不是「主 vs 子」，而是**「交付通道用『解析散文里的 JSON』太脆」**。

---

## 2. 不变量（本设计不改）

- **I-A**：阶段完成 = 确定性 gate 在一份 `StageDeliverable` 上 PASS，且其 `evidence_refs` 在 audit ledger 真实存在（`enforce_evidence_existence`）。
- **I-B**：「已检查为空」≠「未检查」——`vacuous_check` + `skipped_checks` 语义保留（AGENTS.md I8）。
- **I-C**：gate 是确定性规则，不接受「agent 自信说完成」（AGENTS.md I7）。

> 本设计只换「交付物如何抵达 gate」的**通道**，不动「gate 如何验证」。

---

## 3. 目标 / 非目标

**目标**

- G1：交付物通过一个**结构化工具入参**抵达 gate，不再依赖解析最终消息文本。
- G2：**谁调谁交**——orchestrator 或 `reporter` 子 agent 都能提交，「主 vs 子」不再影响成败。
- G3：（可选 Phase 2）**提交即校验**：工具内即时跑 gate，把 PASS/BLOCK + 原因**当场**回灌给 agent，形成紧反馈，而非跑完整段才发现 skip。
- G4：向后兼容——保留文本解析路径（含已上线的 ① 子 agent 捕获兜底）作为降级。

**非目标**

- 不改 `validate_stage_gate` 的 check 逻辑、不改 evidence ledger schema。
- 不强制移除 charter 里的文本路径（迁移期并存）。

---

## 4. 工具设计

### 4.1 工具签名

- **name**：`submit_stage_deliverable`
- **可见性**：仅 stage 任务（`harness_active_stage.is_some()`）时注入；像 `submit_result` 一样是「元/编排」工具，**不进任何 stage 的 `allowed_tools`/`forbidden_tools` 计费**（在 `pre_action_authorizer` 里豁免，等同 orchestration 工具豁免）。
- **谁能调**：`orchestrator`、`reporter`（两个 agent 的 tool 列表都加；其余子 agent 不需要）。
- **入参 schema**（与 `harness/types.rs:161` 的 `StageDeliverable` 一一对应）：

```json
{
  "stage_id": "external_attack_surface",
  "stage_run_id": "<uuid v4>",
  "claims": [{"kind": "http_service_observed", "subject": "<host>", "summary": "...", "evidence_ids": [1]}],
  "evidence_refs": [1, 2, 3],
  "findings": [{"finding_id": "<uuid v4>", "kind": "subdomain", "subject": "<host>", "severity": "info", "evidence_refs": [2]}],
  "skipped_checks": [{"check": "...", "reason": "..."}],
  "required_checks_done": ["dns_resolve"]
}
```

### 4.2 行为

**Phase 1（最小，推荐先做）· 仅捕获**

1. 校验 `stage_id` == 当前活动 stage（`harness_active_stage`）；不匹配 → 返回 error 给 agent。
2. 把入参原样序列化，写入 bridge side-channel `harness_last_deliverable`（**复用** ① 已加的字段）。
3. 返回 `{ "status": "deliverable_received" }` 给 agent。
4. 收尾时 `execute_subtask` 把 side-channel 里的 deliverable 喂给现有 gate（**复用** ① 的 append 逻辑，或直接把结构体交给 gate，跳过文本 parse）。

> Phase 1 = 把 ① 的「启发式从子 agent 结果里捞」升级成「agent 显式、结构化地交」。更准（不靠 `contains("stage_run_id")` 猜）、更稳。

**Phase 2（增强）· 提交即校验**

1-2 同上；3 改为：工具内**当场**跑 `validate_stage_gate(spec, profile)` + `enforce_evidence_existence/kinds`：
   - PASS → side-channel 标记「已过」，返回 `{status: "passed"}`，agent 可直接结束。
   - BLOCK → 返回 `{status: "blocked", reasons:[...], recovery:[...]}`，agent **当场**按原因修，重提交（替代现在「跑完整段才 reflector」的慢回路）。

---

## 5. 集成点（文件级落点）

| # | 改动 | 位置 |
|---|---|---|
| 5.1 | 工具定义 + handler | 新增 `golish-agent-app`（或 `golish-pentest-app`）一个 `submit_stage_deliverable` tool，仿 `record_finding` 注册（`ai/commands/bridge_config.rs` 里 `[pentest-bridge] Registered tool`） |
| 5.2 | handler 读 active stage + 写 side-channel | 复用 `AgentBridge.harness_active_stage` + `harness_last_deliverable`（① 已加） |
| 5.3 | gate 喂入 | `bridge_executor/trait_impl.rs::execute_subtask` 收尾：side-channel 有则交 gate（① 已有雏形）；Phase 2 改为工具内直接调 `golish-agent-kit::harness::gate::validate_stage_gate` |
| 5.4 | charter 改口 | `task_orchestrator/prompts/mod.rs::stage_charter`：把「There is NO submit tool … print ```json」改成「调用 `submit_stage_deliverable` 提交你的交付物」；`stage_discipline_reminder()` 同步改口 |
| 5.5 | reflector 改口 | stage 任务下，reflector 纠正话术指向 `submit_stage_deliverable`，**不再**误导用 `sub_agent_reporter`（治本节 §1 的涌现误导） |
| 5.6 | 授权豁免 | `harness/pre_action_authorizer.rs`：`submit_stage_deliverable` 加入 orchestration 豁免（不受 allowed/forbidden 约束） |

---

## 6. 兼容 / 迁移 / 开关

- **flag 闸**：`GOLISH_HARNESS_SUBMIT_TOOL`（默认 off → 灰度）。off 时走现有文本路径 + ① 兜底；on 时注入工具 + charter 改口。
- **降级**：即使工具上线，`parse_deliverable_from_content` 文本路径**保留**为兜底（模型偶尔仍直接打印 JSON 也能过）。三条路径优先级：工具入参 > 文本 ```json > ① 子 agent 捕获。
- **无 schema/DB 变更** → 无迁移风险（I10 不触发）。

---

## 7. 边界与风险

- **重复提交**：同 `stage_run_id` 幂等（后到覆盖 side-channel）；不同 stage_run_id 视为重交，取最后一次。
- **错 stage 提交**：`stage_id` 与 active stage 不符 → 直接 error，不污染。
- **空跑提交**：gate 的 `min_invocations` + `enforce_evidence_existence` 仍兜底（没真跑工具 → evidence_refs 兑现不了 → BLOCK），工具不绕过验证。
- **双 fence / 格式**：工具收结构化入参，**不再有** ```json 解析的双 fence 问题（① 的隐患一并消除）。
- **风险等级**：中。动 charter/prompt（影响所有 stage run）+ 新工具 + 授权豁免。需 flag 灰度 + 重编 + 真机验。

---

## 8. 验证计划（证据优先）

1. **单测**：工具 schema 解析；handler 写 side-channel；stage_id 不匹配返回 error；Phase 2 的 validate-on-submit PASS/BLOCK 分支。
2. **真机**：开 flag，跑 example.com 外部攻击面 recon，看 `backend.log`：
   - 出现 `submit_stage_deliverable` tool call；
   - `gate decision: PASS` + `cursor advanced`（不再 `gate skipped`）。
3. **回归**：关 flag，确认文本路径 + ① 兜底仍工作。
4. `just precommit` 全绿后才可合。

---

## 9. 与已上线 ① 的关系

- ① = 「从子 agent 结果**启发式捕获** deliverable 并追加喂 gate」，是**务实补丁**，立即缓解 gate skip。
- 本工具 = ① 的**正态化**：把「捕获」换成「agent 显式结构化提交」，更准、更稳、可即时反馈。
- 迁移后，① 的 `harness_last_deliverable` side-channel **被复用**（不浪费），只是写入方从「runtime 启发式」变成「工具 handler 显式」。

---

## 10. 决策点（需你拍）

- D1：先做 Phase 1（仅捕获，最小）还是直接上 Phase 2（提交即校验，反馈更好但改动更大）？
- D2：工具 handler 放 `golish-agent-app` 还是 `golish-pentest-app`？（建议 `golish-agent-app`，与 harness 同源）
- D3：charter 是「只留工具路径」还是「工具 + 文本双路径并存」？（建议并存，灰度更安全）
