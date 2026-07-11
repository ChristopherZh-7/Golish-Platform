# Sub-agent checkpoint addressability 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 fresh/exact sub-agent 在 graceful failure、provider context failure 和 outer timeout 后仍把最后一个成功 checkpoint 的 chain UUID 结构化交给 `stage_run`。

**架构：** 在首次 provider 请求前写 initial body snapshot，并通过 inner/outer 共享槽记录“已成功 checkpoint”的 UUID；`SubAgentResult` 和 typed error JSON 结构化携带该 UUID，runtime/stage-run 优先消费结构化字段并保留旧 marker fallback。

**技术栈：** Rust 2021、Tokio、Serde、Rig message history、cargo nextest。

## 文件结构

- `backend/crates/golish-sub-agents/src/definition/mod.rs`：`SubAgentResult.chain_id` 的兼容类型合同。
- `backend/crates/golish-sub-agents/src/executor/{mod,inner,chain_persist}.rs`：initial snapshot、共享 checkpoint id、failure/timeout 返回。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{sub_agent_call,stage_run_call}.rs`：结构化 UUID 透传与 worker map。
- `docs/modules/backend/golish-sub-agents/executor.md`：checkpoint/addressability 合同。
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`：runtime 消费优先级。
- `docs/modules/INDEX.md`：模块卡状态核对。

## 任务 1：锁定结构化结果兼容性

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/definition/mod.rs`
- 修改：`backend/crates/golish-sub-agents/src/definition/tests.rs`

**步骤：**

1. 新增 RED 测试：反序列化不含 `chain_id` 的历史 `SubAgentResult` 必须得到 `None`；含 UUID 时必须 round-trip。
2. 运行测试确认字段尚不存在而失败。
3. 增加 `#[serde(default, skip_serializing_if = "Option::is_none")] pub chain_id: Option<Uuid>`，更新构造点。

**验证：**

```bash
cd backend && cargo test -p golish-sub-agents definition::tests --lib
```

预期：新旧 JSON 均可读，UUID round-trip 一致。

**提交：**

```bash
git add backend/crates/golish-sub-agents/src/definition
git commit -m "feat(sub-agent): expose durable chain identity"
```

## 任务 2：在首次请求前发布 initial snapshot

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor/chain_persist.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/inner.rs`

**步骤：**

1. 新增 RED 测试：fresh chain 创建后、模型 stream 返回错误前，recording persistence 必须已收到一个 provider-valid body update，失败结果必须携带该 UUID。
2. 在 initial user prompt 与 repair directive 都进入 history 后调用现有 body-only checkpoint；成功后写入共享 checkpoint slot。
3. 所有 inner graceful failure 只返回共享槽中的 UUID，不在失败分支尝试保存 dangling/partial history。
4. 保持完整 tool batch 后的 checkpoint 位置不变，并断言 usage update 仍只发生在正常 finalization。

**验证：**

```bash
cd backend && cargo test -p golish-sub-agents checkpoint --lib
cd backend && cargo test -p golish-sub-agents stream_error --lib
```

预期：initial 与完整 batch body update 可见；partial batch 无 update；usage 无重复。

**提交：**

```bash
git add backend/crates/golish-sub-agents/src/executor
git commit -m "fix(sub-agent): checkpoint fresh chains before provider work"
```

## 任务 3：让 outer timeout 返回最后 checkpoint UUID

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor/mod.rs`

**步骤：**

1. 新增 RED 测试：模型在 initial snapshot 后永久 pending，outer timeout 返回 `success=false`，但 `chain_id` 等于 recording persistence 的 chain UUID。
2. 在 outer wrapper 创建共享 checkpoint slot并传入 inner；timeout drop inner future 后只读取该槽，不执行 async persistence。
3. timeout response 追加兼容 marker，仅当槽内 UUID 存在。

**验证：**

```bash
cd backend && cargo test -p golish-sub-agents outer_timeout --lib
```

预期：timeout 返回可寻址 UUID，且数据库 body 已在 timeout 前写入。

**提交：**

```bash
git add backend/crates/golish-sub-agents/src/executor/mod.rs
git commit -m "fix(sub-agent): retain checkpoint identity across timeout"
```

## 任务 4：透传 UUID 并绑定 stage worker

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 新增 RED 测试：普通 failed `SubAgentResult` 的 ToolExecutionResult 含 `chain_id`；`ProviderContextLimitExceeded` error JSON 保留 variant 内 UUID；stage-run 结构化字段优先于 response marker，字段缺失时 marker 仍生效。
2. `sub_agent_call` 输出结构化 `chain_id`，typed error mapper按 variant 提取 UUID。
3. `stage_run` 从 `result.value.chain_id` 解析 UUID，失败时再解析 legacy marker；仅对有效 UUID 写 worker checkpoint。

**验证：**

```bash
cd backend && cargo test -p golish-agent-runtime sub_agent_chain --lib
cd backend && cargo test -p golish-agent-runtime stage_run_worker_chain --lib
```

预期：结构化字段、typed error 和 marker fallback 测试全部通过。

**提交：**

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs
git commit -m "fix(stage-run): preserve failed worker checkpoint identity"
```

## 任务 5：同步模块卡并运行全门禁

**文件：**

- 修改：`docs/modules/backend/golish-sub-agents/executor.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/INDEX.md`

**步骤：**

1. 记录 initial/atomic checkpoint、结构化 UUID、marker fallback 和 hard-kill 边界。
2. 运行两个 crate 全量测试、clippy、fmt，再运行仓库门禁。

**验证：**

```bash
cd backend && cargo nextest run -p golish-sub-agents -p golish-agent-runtime --status-level fail
cd backend && cargo clippy -p golish-sub-agents -p golish-agent-runtime --all-targets -- -D warnings
cd backend && cargo fmt -p golish-sub-agents -p golish-agent-runtime -- --check
git diff --check
just precommit
```

预期：全部 exit 0，clippy 零 warning。

**提交：**

```bash
git status --short
```

只核对工作树；没有用户提交/推送授权时不执行 commit 或 push。
