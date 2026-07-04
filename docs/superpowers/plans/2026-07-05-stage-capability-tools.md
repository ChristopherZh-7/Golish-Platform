# Stage Capability Tools 实现计划

> Design: [`docs/design/2026-07-05-stage-capability-tools.md`](../../design/2026-07-05-stage-capability-tools.md)
> Status: proposed
> Date: 2026-07-05

## 目标

把当前面向模型的 `suggested_tools` / concrete tool list 升级为 stage capability contract：

- AI 选择“做什么能力”。
- 后端决定“用什么底层工具、怎样过滤输入、怎样落库”。
- gate 继续只认 DB truth / evidence。

第一轮不直接替换所有工具执行，只做 capability registry + `suggested_capabilities` + prompt/refiner/UI 口径统一；第二轮再实现 EAS wrapper runner。

## 当前事实

- `StageAssetCoverageCell` 当前只有 `suggested_tools`。
- `stage_worklist_next` 直接透传 coverage cell 的 `suggested_tools`。
- `CoverageGapAction` 只有 `suggested_tools`。
- `stage_refiner` 以第一个 suggested tool 生成 `RepairAction.tool`。
- `stage_run_call` objective 强调 `allowed_tool_types` 和 concrete tool names。
- EAS / Enumeration 的 DB truth 和 evidence 落账路径已经存在，不能被 capability 设计绕开。

## Phase 0：文档和 feature tracking

### Task 0.1 写设计文档

新增：

- `docs/design/2026-07-05-stage-capability-tools.md`

### Task 0.2 写实现计划

新增：

- `docs/superpowers/plans/2026-07-05-stage-capability-tools.md`

### Task 0.3 feature_list 增加条目

新增 `stage-capability-tools-2026-07-05`，状态 `not_started`。

> 不把它设为 `in_progress`，因为当前仓库已有多个历史 in_progress 条目；本轮用户要求先写文档，不启动代码实现。

## Phase 1：纯 capability registry

### Task 1.1 新增 `stage_capability.rs`

文件：

- `backend/crates/golish-agent-kit/src/harness/stage_capability.rs`
- `backend/crates/golish-agent-kit/src/harness/mod.rs`

定义：

- `StageCapabilitySpec`
- `StageCapabilitySuggestion`
- `CapabilityRisk`
- `CapabilityRunnerKind`

函数：

```rust
pub fn capabilities_for_stage(stage: StageKind) -> Vec<&'static StageCapabilitySpec>;
pub fn capabilities_for_technique(stage: StageKind, technique: &str) -> Vec<&'static StageCapabilitySpec>;
pub fn suggested_tools_for_technique(stage: StageKind, technique: &str) -> Vec<String>;
pub fn capability_by_id(id: &str) -> Option<&'static StageCapabilitySpec>;
```

第一批 registry 覆盖：

- `target_intel`
- `external_attack_surface`
- `enumeration`
- `vuln_triage`
- `attack_candidate`
- `verification`

### Task 1.2 单测

测试要求：

- 每个 expected technique 至少能映射到 0 个或多个明确 capability；EAS / ENUM 必须非空。
- 每个 capability 的 `tool_names` 都能被当前 stage `allowed_tool_types` 允许，或者是 non-scan direct/meta tool。
- `target_intel` capability 不能暴露 `pentest_run` / `nmap` / `httpx`。
- `enumeration` capability 不能暴露 `ffuf` / `gobuster` / `feroxbuster` / `dirsearch`。
- `eas.fingerprint_services` 只建议 `nmap`，WhatWeb 只能在 instruction 里作为 HTTP(S) supplemental，不作为 generic SERVICE gap tool。

验证：

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail
```

## Phase 2：coverage/worklist/gate suggestions

### Task 2.1 扩展 `StageAssetCoverageCell`

文件：

- `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`
- generated TS 由 ts-rs 后续生成，不手写 `frontend/lib/generated/*`

改动：

- 新增 `suggested_capabilities: Vec<StageCapabilitySuggestion>`
- `suggested_tools` 由 `suggested_capabilities[].tools` 派生
- 保留旧字段

验证：

```bash
cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail
```

### Task 2.2 更新 `stage_worklist_next`

文件：

- `backend/crates/golish-agent-kit/src/tool_executors/security.rs`

改动：

- worklist item 增加 `suggested_capabilities`
- `gap_examples` / compact coverage 增加 `suggested_capabilities`
- `worklist_contract` 文案从 “Run suggested tool(s)” 改为 “Close the suggested capability/cell”

验证：

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_worklist coverage_preflight --status-level fail
```

### Task 2.3 扩展 gate recovery action

文件：

- `backend/crates/golish-agent-kit/src/harness/types.rs`
- `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
- `backend/crates/golish-agent-kit/src/harness/org_gate.rs`

改动：

- `CoverageGapAction` 增加 `suggested_capabilities`
- 旧 `suggested_tools` 继续填充
- `coverage_gap_action()` 从 capability registry 派生 suggestions

验证：

```bash
cd backend && cargo nextest run -p golish-agent-kit gate coverage_complete --status-level fail
```

## Phase 3：refiner / sub-agent repair mode capability-first

### Task 3.1 `RepairAction` 增加 capability

文件：

- `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`

改动：

- `RepairAction` 增加 `capability_id: Option<String>`
- `repair_actions_for` 优先读取 `CoverageGapAction.suggested_capabilities`
- `command_hint_for` 从 `(stage, capability_id)` 生成，不再散落 match tool string
- `model_instruction` 显示 `capability=<id>`，再显示 implementation tool

验证：

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_refiner refiner --status-level fail
```

### Task 3.2 sub-agent `CoverageGapAction` 兼容扩展

文件：

- `backend/crates/golish-sub-agents/src/executor_types.rs`
- `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

改动：

- `CoverageGapAction` 增加 `suggested_capabilities`
- `coverage_gap_action_instruction` 优先展示 capability
- `append_direct_enumeration_repair_tools` 继续由 suggested tools 兜底，避免旧 transcript 破

验证：

```bash
cd backend && cargo nextest run -p golish-sub-agents coverage_gap submit_repair --status-level fail
```

## Phase 4：stage_run objective 和 methodology

### Task 4.1 specialist objective 改成 capability-first

文件：

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

改动：

- `build_org_objective` 在 coverage contract 后加入 stage capability list。
- 保留 concrete tools，但明确它们是 implementation details。
- self-check 文案从 `suggested_tools` 改成 `suggested_capabilities / suggested_tools legacy`。

验证：

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run_call build_org_objective --status-level fail
```

### Task 4.2 更新 methodology

文件：

- `resources/harness/stages/external_attack_surface/methodology.md`
- `resources/harness/stages/enumeration/methodology.md`
- `resources/harness/stages/target_intel/methodology.md`
- `resources/harness/stages/vuln_triage/methodology.md`

改动：

- 把 “suggested_tools” 语言改成 “capability / work item”。
- 仍明确 tool boundary，避免模型误以为 capability 可以绕过 stage whitelist。

## Phase 5：UI 轻量展示

### Task 5.1 coverage tooltip 优先 capability

文件：

- `frontend/components/Engagement/StageAssetCoveragePanel.tsx`
- `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`

改动：

- pending/error cell title 展示 capability label。
- raw tools 只作为 hover/debug details。
- 保持旧 generated type 兼容：字段不存在时 fallback 到 `suggested_tools`。

验证：

```bash
pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx
pnpm exec tsc --noEmit --pretty false
```

## Phase 6：EAS wrapper runner

### Task 6.1 暴露 `run_stage_capability`

文件候选：

- `backend/crates/golish-agent-kit/src/tool_executors/security.rs`
- `backend/crates/golish-tools/src/definitions/security_tools.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`

Schema：

```json
{
  "capability_id": "eas.fingerprint_services",
  "work_item_ids": ["<target_id>:GOLISH-EAS-SERVICE-FINGERPRINT"],
  "limit": 20
}
```

执行前校验：

- active harness stage 必须存在。
- capability stage 必须等于 active stage。
- work item 必须属于当前 org/stage/worklist。
- work item state 必须是 pending/error。
- capability risk 必须不超过 profile authorization。

### Task 6.2 实现 `eas.probe_http_liveness`

行为：

- 只处理 LIVENESS gap。
- domain/url/vhost 批量走 `httpx`。
- IP 有 HTTP 端口时可形成 URL endpoint。
- 结果由 existing output-store / background completion / technique_outcomes 写入。

### Task 6.3 实现 `eas.discover_ports`

行为：

- 只处理 IP/CIDR host PORT gap。
- domain/url 直接返回 not_applicable/blocked suggestion，不扫。
- 默认 naabu；masscan 只在大范围且 profile 允许时用。
- list-file/input_lines 由后端生成，模型不传 shell args。

### Task 6.4 实现 `eas.fingerprint_services`

行为：

- 只处理 SERVICE-FINGERPRINT gap。
- 先查 DB confirmed open ports。
- 无 open ports：写 not_applicable/blocked terminal suggestion。
- 有 open ports：按 shared port set 分组跑 `nmap -Pn -sV`。
- WhatWeb 只允许 confirmed HTTP(S) endpoint supplemental，不作为 generic service fallback。

验证：

```bash
cd backend && cargo nextest run -p golish-agent-kit run_stage_capability stage_capability --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime run_stage_capability --status-level fail
```

活体 smoke：

```bash
python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --db --full
```

期望：

- Prober 先调用 `stage_worklist_next`。
- Prober 使用 `run_stage_capability` 关闭 EAS gaps。
- 不出现 broad `nmap -sV -iL` raw domain/url batch。
- output-store 落 `targets.ports` / `fingerprints` / `technique_outcomes`。
- `stage_worklist_status.ready_to_submit` 收敛。

## Phase 7：Enumeration 接入评估

Enumeration 已有 direct tools，先不急着改成 wrapper。评估条件：

- live run 仍出现模型逐 URL loop。
- `route_probe_paths` 批次仍过大或过慢。
- AI 仍把 `ffuf` / `gobuster` / external dir fuzzer 当默认路径。

若满足，给以下能力接入 `run_stage_capability`：

- `enum.collect_browser_surface`
- `enum.extract_js_apis`
- `enum.probe_routes`

重点不是换工具，而是固定 batch policy 和 worklist re-query cadence。

## Phase 8：Vuln / Verification 延后

Vuln 和 Verification 风险更高，必须等待：

- vuln scanner handler 对每个 WSTG class 写入 `technique_outcomes`
- authoritative techniques 扩展完成
- attack candidate persistence 完成
- verification approval/candidate binding 设计完成

之后才允许把 formulaic sweep / approved PoC 做成 wrapper。

## 收尾验证

每个实现 slice 结束至少跑对应 scoped checks。准备标 `passing` 前必须跑：

```bash
just precommit
```

并做一次 live EAS 或 Enumeration smoke，把 run id、worklist 收敛、DB truth 证据写入 `agent-progress.md` 和 `feature_list.json.evidence`。
