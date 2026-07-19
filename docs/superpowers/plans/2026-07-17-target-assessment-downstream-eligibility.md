# EAS Gate 修复、Target 状态与实体 CLI 实现计划

**目标：** 修复原 operation 在 WhatWeb retryable errors 后无法继续过 Gate 的恢复链；在 Target
Surface 展示 evidence-backed producer 状态；用同一 Test1 operation 的真实 CLI 跑到 EAS Gate
PASS。全程不运行 `init.sh`。

**执行原则：** Gate 与 Enumeration routing 继续只信后端 DB/evidence authority；前端只解释
typed ledger outcome，不从自然语言或任意 raw output 推断成功/排除。

## Task 1：冻结现场与失败原因

**状态：完成。**

- 用 `scripts/run_tree.py --full --db` 和 embedded PostgreSQL 确认最后夜间 run、session、
  operation、organization、execution 和九个 exact gaps。
- 区分 Nmap success 与 WhatWeb attempt 1 `connection_reset`。
- 确认第一次独立 Turn 恢复只重提 deliverable，第二次触发 shared-fuel exhaustion。

## Task 2：修复 bounded Controller continuation

**状态：完成。**

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- `backend/crates/golish-db/migrations/20260717000001_stage_team_controller_turn_resume_fuel.sql`
- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**实现：**

1. Gate checkpoint 和 successor checkpoint 携带 exact server gap manifest。
2. Controller coordination objective 强制 `update_plan → worklist → original producer`，禁止 coordination
   turn 直接 submit，禁止 relabel error。
3. `max_controller_gate_repairs=1` 保持不变；新增独立 `max_controller_turn_resumes=2`，repo 与 DB
   trigger 都硬上限 2。
4. migration 校验 resumed checkpoint 的 full JSON 与 durable gap exact equality。

**验证：** focused runtime unit/integration tests、fresh embedded-PG migration tests、`cargo build -p golish`。

## Task 3：精简且可信的 Target 状态

**状态：完成。**

**文件：**

- `backend/crates/golish-db/src/models/pentest.rs`
- `backend/crates/golish-pentest-app/src/security_analysis.rs`
- `frontend/lib/api/security-analysis.ts`
- `frontend/components/TargetPanel/surface/whatWebAssessment.ts`
- `frontend/components/TargetPanel/surface/WhatWebAssessmentBadge.tsx`
- `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`
- `frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`
- `frontend/components/TargetPanel/surface/tabs/EvidenceTab.tsx`

**实现：**

1. Audit read DTO 透传 typed evidence authority fields。
2. 只接受 `audit_role=evidence`、exact technique、canonical evidence asset；found/checked-empty 直接看
   typed outcome，error/blocked 还必须通过 bounded structured payload consistency 校验。
3. Web Origin 表显示单一 WhatWeb 状态列；无 evidence 显示 `Not assessed`。
4. 删除 whole-Target worst-state、重复 badge 和 speculative downstream exclusion。

**验证：** parser、badge、table alignment、Evidence wording、API normalization tests；Biome + TypeScript。

## Task 4：真实 CLI 同实体验收

**状态：完成。**

```bash
backend/target/debug/golish /Users/christopherzheng/golish-platform/Test1 \
  --stage-run-resume c14e6e10-4343-4b9e-9642-2617bfb56754 \
  --resume-to external_attack_surface \
  --expect-session 2626e09d-8447-4b54-9246-5dc15528bc8c \
  --expect-task c14e6e10-4343-4b9e-9642-2617bfb56754 \
  --expect-operation c14e6e10-4343-4b9e-9642-2617bfb56754 \
  --expect-org acca4a29-3ac7-4a41-95e3-dfaf85d54f21 \
  --expect-stage external_attack_surface \
  --auto-approve --db-smoke-summary --verbose \
  -e '继续：只处理服务端持久化的 exact coverage gaps；先读取 worklist，按 producer 契约完成重试，不要把 error 改写成 checked_empty。'
```

**接受条件：** 同一 operation/org/Controller chain；attempt 2/3 实际执行；九格 terminal；Gate
PASS；CLI exit 0；Target/ports 保留；无扫描进程残留。

## Task 5：全仓门禁与交接

**状态：完成。**

1. 更新 backend/frontend module cards、`feature_list.json` 和 `agent-progress.md`。
2. 运行 `just space-guard`、`just precommit`、`jq empty feature_list.json`、`git diff --check`。
3. 只有计划内 verification、实体 CLI、全门禁和 clean-state checklist 都通过后，才把唯一 active
   feature 标为 `passing`；否则保持 `in_progress` 并记录精确剩余项。

本轮不自动 commit、stage 或 push。
