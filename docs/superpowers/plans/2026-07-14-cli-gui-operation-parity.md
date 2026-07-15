# CLI / GUI Operation 语义统一实现计划

> Design: [`docs/design/2026-07-14-cli-gui-operation-parity.md`](../../design/2026-07-14-cli-gui-operation-parity.md)
> Status: in_progress
> Date: 2026-07-14

## 执行规则

- 按用户要求不运行 `init.sh`、`just precommit` 或整套测试，先完成代码；每个 slice 只跑
  对应的极小 RED/GREEN 测试。
- 本功能的所有 company-only、loopback、CLI/GUI parity 与后续 live acceptance 固定使用
  `red_team` profile；其他 profile 尚未完成，不能作为本轮通过证据。
- 代码/单测阶段不调用真实 LLM、企业信息 provider、扫描器或外部目标；代码收口后按用户
  明确要求跑指定公司真实流程。公司名阶段可以先做被动 Scoping；主动 EAS 前仍需 exact
  target 确认，最终验收到 `attack_candidate` 即停，不进入尚未实现的后续阶段。
- 不改 DB schema/migration/generated IPC。
- 共享 dirty tree 中其他功能的改动全部保留，不顺手重构。

## Task 1：锁住现场 P0

**Files**

- `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`
- `scripts/run_tree.py`
- `scripts/tests/test_run_tree_runtime_memory.py`

**Steps**

1. RED：证明 authoritative asset SQL 接受 `org_id=NULL`，以及 run-tree 把 chat key 当
   `tasks.session_id` UUID。
2. asset/coverage/EAS truth 在未绑定 org 时 fail closed；exact org SQL 不再含 nullable
   whole-DB fallback。
3. run-tree 通过 `tasks -> sessions.chat_session_key` 找 operation。
4. 用指定 GUI session 只读回放确认 operation 可见。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish-agent-app authoritative_asset_queries_require_an_exact_org_scope --status-level fail
python3 -m unittest scripts.tests.test_run_tree_runtime_memory.RuntimeMemoryDiagnosisTests.test_runtime_operation_fallback_resolves_the_transcript_chat_key
python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1784017367012-1 --full --db
```

## Task 2：抽 fresh-operation shared context kernel

**Files**

- `backend/crates/golish-agent-app/src/ai/task_operation.rs`
- `backend/crates/golish-agent-app/src/ai/mod.rs`
- `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`

**Steps**

1. 先写 pure/config test，锁定 session upsert → tracker rebind → repo/project scope →
   orchestrator config 的单一路径和失败顺序。
2. 定义中立 typed launch/context；不得依赖 `golish` CLI 类型。
3. 机械搬迁 GUI fresh operation 的 context 构建，不改 continuity/gate/profile 行为。
4. GUI adapter 调 shared kernel；保留 Tauri event/lead/continuity 作为 adapter。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish-agent-app task_operation --status-level fail
```

## Task 3：CLI fresh slice 复用同一 kernel

**Files**

- `backend/crates/golish/src/stage_run/mod.rs`
- `backend/crates/golish/src/ai.rs`

**Steps**

1. 给 CLI adapter 写 launch-spec parity 红测。
2. 删除 `orchestrate()` 内重复 session upsert、tracker rebind、repo/project scope 和
   orchestrator setters，改调 shared kernel 的 slice entry。
3. 外层重复 upsert 只保留必要 bootstrap，或由 kernel 完全接管；失败不得 warning 后继续。
4. terminal report/event collector 保持 CLI adapter 私有。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish -E 'test(task_operation) | test(stage_run)' --status-level fail
```

## Task 4：统一 profile、provider 与 DAG launch spec

**Files**

- `backend/crates/golish-agent-app/src/ai/task_operation.rs`
- `backend/crates/golish-agent-app/src/ai/session.rs`
- `backend/crates/golish/src/cli/bootstrap/agent_init.rs`
- `frontend/components/AIChatPanel/**`

**Steps**

1. profile id 与 objective 原子提交，GUI 不再吞 backend mode 更新失败。
2. full/slice 使用同一 typed entry；profile/DAG config 只解析一次。
3. CLI/GUI 都用同一 provider factory；未知 provider fail closed。
4. provider matrix pure test 比较 provider/model/endpoint/tool registry。

## Task 5：统一 trusted scope intake 和子公司 policy

**Files**

- `backend/crates/golish-agent-app/src/ai/task_operation.rs`
- `backend/crates/golish/src/stage_run/runtime_v2.rs`
- `backend/crates/golish/src/cli/args.rs`
- Scoping adapter/tests

**Steps**

1. 裸公司名只生成 `SubjectLabel`，不能直接 freeze authority。
2. CLI target flags 与 GUI review 生成相同 confirmed target rows/snapshot。
3. 子公司默认和比较统一为契约定义的 51% 边界。
4. 50/51、provider-discovery-not-authority 和 empty-target tests RED→GREEN。

## Task 6：typed ApprovalPort、direct-EAS 防绕过与 shared resume

**Files**

- shared approval/application service
- `backend/crates/golish/src/stage_run/mod.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**Steps**

1. `scope_review/choice/unit_review` 使用 typed auto policy；拒绝通用 auto-approved。
2. TargetIntel→EAS 与 direct EAS slice 共用可信 target barrier，generic approval 不可绕过。
3. exact resume 的 session/repo/project/orchestrator setup 移入 shared kernel。
4. 两端在相同 Candidate rows 上都以 Candidate gate/DB truth 为验收终点；本轮不进入或释放
   后续 Verification，Candidate review CLI adapter 随后续阶段实现另开任务。

## Task 7：company-only 与 Candidate parity fixtures

**Files**

- focused app/CLI integration fixtures
- scripted executor / loopback fixture

**Steps**

1. `red_team` 公司名-only：空 target snapshot，Scoping 可完成，但主动阶段前等待 concrete target；
   断言无扫描 evidence、manifest、Candidate。
2. `red_team` 正向 loopback：一个明确授权 exact URL，scripted EAS/Enumeration/Vuln outcomes 和
   Candidate decision；比较两 adapter 的 scope、stage、handoff、manifest 和 terminal rows。
3. normalized event/DB parity 忽略随机 UUID/时间戳，其余必须全等。

## Task 8：文档、模块卡与现场验收

**Files**

- `docs/modules/backend/golish-agent-app/ai.md`
- `docs/modules/backend/golish/stage_run.md`
- `docs/modules/INDEX.md`
- `agent-progress.md`
- `feature_list.json`

**Steps**

1. 记录 focused RED/GREEN 命令和未运行的大门禁。
2. 用“广州有创网络科技有限公司”启动真实 Scoping；若被动阶段提出 target，取得 exact
   target 确认后继续。CLI/GUI 已共享执行内核，至少实际跑一条完整路径至 Candidate。
3. 核对 transcript、run.log、`run_tree.py --full --db` 与 DB rows；Candidate gate 未通过前
   保持 `in_progress/blocked`，并明确不运行后续未实现阶段。

## Task 9：phase-boundary typed Confirm 对齐

**Design**

- `docs/design/2026-07-15-cli-phase-boundary-approval-parity.md`

**Files**

- `backend/crates/golish/src/cli/args.rs`
- `backend/crates/golish/src/stage_run/mod.rs`
- `scripts/stage_smoke.py`

**Steps**

1. RED：证明 headless long slice 即使给出 typed auto policy，仍会固定 decline GUI 的
   phase `confirmation`。
2. 新增 `--approve-phase-boundaries`；只有与 `--auto-approve` 同时出现时，才把该 exact
   confirmation 解析为显式 CLI Confirm。
3. 不放松 scope/unit/credentials/freetext/unknown decision。
4. stage smoke 显式透传 flag；重新跑指定公司的 `Scoping → Attack Candidate`。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish -E 'test(headless_typed_approval_policy_uses_only_explicit_cli_authority) | test(test_args_stage_run_accepts_explicit_phase_boundary_approval)' --status-level fail
```

## Task 10：Vuln Nuclei 无受信模板死循环收口

**Design**

- `docs/design/2026-07-15-vuln-nuclei-no-runnable-templates.md`

**Files**

- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/landing.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`

**Steps**

1. RED：锁定 Nuclei 精确 `no templates provided for scan` fatal 当前被误判为可重试 error。
2. 新增窄 `Blocked` completion；只在 non-zero、空 stdout、未截断且精确 fatal 时生效。
3. guarded landing 发布 evidence-backed `blocked` technique outcome，wrapper 返回 complete，
   不降级 `-dut`，不伪造 checked-empty。
4. 相邻未知 stderr/exit/truncation 继续 fail closed；重新跑指定公司的真实链路至 Candidate。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish-pentest-app vuln_ --status-level fail
```

## Task 11：Vuln 匿名访问 read-model / final-gate 对齐

**Design**

- `docs/design/2026-07-15-vuln-anonymous-access-gate-parity.md`

**Files**

- `backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- `docs/modules/backend/golish-agent-kit/harness.md`

**Steps**

1. RED：复现 worklist 已把 `WSTG-ATHN-04 not_applicable` 视为 terminal，但 final gate 丢弃
   dedicated wrapper source。
2. producer matrix 明确要求匿名访问只能来自 `vuln_probe_anonymous_access`；N-day 与其余
   Nuclei technique 保持现有来源。
3. gate fixture 改为 producer-owned `coverage=[]`，验证 exact-origin DB truth 自行闭格。
4. 用新二进制恢复/重跑真实 operation，确认 Vuln PASS 并进入 Attack Candidate。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish-agent-kit vuln_triage_ --status-level fail
```

## Task 12：Candidate coordinator pass-token 收口

**Design**

- `docs/design/2026-07-15-candidate-coordinator-pass-token-closeout.md`

**Files**

- `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- `docs/modules/backend/golish-agent-app/ai.md`

**Steps**

1. RED：复现 `attack_analyst` 已通过并签发 pass token，但顶层 coordinator 因无
   `StageRunUnit` 被 Candidate manifest preview 拒绝。
2. 只对 trusted Main + no-unit/no-worker/no-attempt 的 aggregate closeout，在规范化 token 后
   跳过 unit-scoped preview；worker fence 不放松。
3. final fan-out gate 继续从 current-operation `org_stage_completions` 重算 token。
4. 用新二进制重跑至 Candidate PASS，并确认 slice 在 Candidate 停止。

**Focused verification**

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(attack_candidate_coordinator_pass_token_does_not_require_worker_unit_context) | test(coordinator_stage_run_pass_token_skips_per_unit_submission)' --status-level fail
```
## Task 13: Candidate terminal slice and blocker semantics

- [x] Add a RED/GREEN test proving a V2Only `--to attack_candidate` terminal
  slice does not read the review barrier.
- [x] Resolve the projected successor before Candidate crossing barriers.
- [x] Teach the shared Candidate methodology and evidence-list contract that
  `blocked` never proves WAF or another cause.
- [ ] After explicit authorization to modify `golish-db`, atomically close the
  exact terminal stage execution with the task completion write.
