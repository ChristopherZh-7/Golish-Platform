# Stage Team canonical 派发与拒绝状态收敛实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划，并使用 test-driven-development 完成每个 RED→GREEN 循环。

**目标：** 让 Enumeration 等 Company Controller 的 target-scoped child 请求稳定进入 durable WorkerRun，并让所有拒绝在模型和 UI 中显示为真实失败而非永久排队。

**架构：** sub-agent tool schema 明确 target canonical selector；runtime 在 DB 写入前把已知 target shorthand 规范化、去重，但继续由 DB 做 frozen scope authorization；全部拒绝保留 durable decisions，前端对新旧 rejection payload 都收敛为 error。

**技术栈：** Rust 2021、serde/serde_json、UUID、React 19、TypeScript、Vitest、cargo-nextest。

---

## 文件结构

- 修改 `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`：定义模型可见 target canonical ref schema。
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`：规范化 refs、去重并回传完整 rejection decisions。
- 修改 `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：历史全部拒绝结果的 exact-code fallback。
- 修改 `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：UI rejection replay regression。
- 修改对应 `docs/modules/` 卡、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`：同步合同与证据。

### Task 1：登记问题并建立 RED tests

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤：**

1. 在现有 Controller tool schema test 中断言：

```rust
assert_eq!(
    tools[1].parameters.pointer(
        "/properties/workers/items/properties/subject_refs/items/required"
    ),
    Some(&serde_json::json!(["kind", "target_id"]))
);
```

2. 新增 runtime test，输入两个相同 `target_id`、不同 `target_url`、均缺 `kind` 的 refs，期望 repo 只收到：

```json
[{"kind":"target","target_id":"<same uuid>"}]
```

3. 扩展 `stage_team_dispatch_all_rejected_does_not_enter_waiting_barrier`，期望
   `status=dispatch_rejected`、`accepted_count=0`、`requests[0].decision=rejected`。
4. 新增 frontend replay test：tool status 为 error、result 仅含
   `STAGE_TEAM_DISPATCH_NONE_ACCEPTED` 时，两张 args-derived 卡均显示 error，queued 为零。
5. 每次 Cargo 前执行 `just space-guard`，分别运行 focused nextest/Vitest，确认因缺少目标行为而 RED。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：新增断言失败；不能以编译错误或环境错误冒充 RED。

### Task 2：实现 provider-compatible schema 与 runtime canonicalization

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`

**步骤：**

1. 把 `subject_refs.items` 改为无 `oneOf/anyOf` 的 provider-compatible object schema：

```json
{
  "type": "object",
  "properties": {
    "kind": {"type":"string","enum":["target"]},
    "target_id": {"type":"string"}
  },
  "required": ["kind", "target_id"],
  "additionalProperties": false
}
```

2. 新增纯函数 `canonicalize_stage_team_subject_refs`：先反序列化合法 `CanonicalFactKey`；仅对键集合为
   `target_id`/`target_url` 的 shorthand 解析 UUID 并构造 `CanonicalFactKey::Target`；其它形状返回
   `STAGE_TEAM_DISPATCH_WORKER_INVALID`。
3. 把 canonical JSON string 放入 `HashSet`，保留首次出现顺序并丢弃重复 target；之后才计算
   `request_material/request_sha256`。
4. 不修改 golish-db 的 `dynamic_request_subject_rejection`，确保 authorization 边界没有放宽。
5. 运行 Task 1 backend focused tests，确认 GREEN。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
```

预期：相关测试全部通过，非法非-target shorthand 仍被 host 拒绝。

### Task 3：回传 durable rejection 并修复历史 UI

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤：**

1. `accepted_count == 0` 时直接构造失败 payload，保留 `decisions`：

```json
{
  "code":"STAGE_TEAM_DISPATCH_NONE_ACCEPTED",
  "status":"dispatch_rejected",
  "accepted_count":0,
  "requests":[...]
}
```

2. 保持返回 tuple 的 bool 为 `false`，确保 Controller 不进入 accepted barrier。
3. 前端仅在 exact code 为 `STAGE_TEAM_DISPATCH_NONE_ACCEPTED` 且 assignment 没有逐项 decision 时，使用
   `rejected` fallback；不能从任意 error/prose 推断。
4. 运行 backend/frontend focused tests，确认 GREEN。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：全部拒绝不 park Controller，且 UI 中 error 卡数量等于 workers 数量、queued 为零。

### Task 4：文档同步与聚焦验证

**文件：**

- 修改：`docs/modules/backend/golish-sub-agents/executor.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤：**

1. 记录 target-only model schema、runtime safe shorthand canonicalization、DB reauthorization 和 rejected UI 真值。
2. 运行相关包 nextest/Vitest、`cargo check`、all-target Clippy、rustfmt、TypeScript、Biome、JSON/diff checks。
3. 按用户要求不运行 `init.sh`。只有新鲜证据满足 feature verification 才把状态改为 `passing`；否则保持
   `in_progress` 并记录缺口。
4. 共享 dirty tree 不自动 stage/commit/push。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
cd backend && cargo check -p golish-sub-agents -p golish-agent-runtime
cd backend && cargo clippy -p golish-sub-agents -p golish-agent-runtime --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
pnpm typecheck
pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
jq empty feature_list.json
git diff --check
```

预期：所有命令 exit 0、无 warning；不运行外部 provider、扫描器或真实目标。
