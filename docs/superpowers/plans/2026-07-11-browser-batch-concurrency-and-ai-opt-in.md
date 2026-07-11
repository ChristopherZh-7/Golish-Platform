# Browser 批次并发与工具内 AI 显式启用实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 `browser_collect_js_api` 在不削弱单 origin 授权、证据与终态合同的前提下，以最多 4 路有界并发处理 batch，并让工具内 AI 只有在 `ai=true` 且 `ai_assist=true` 时才启用。

**架构：** 保留现有单 root `execute_single` 作为唯一生产路径，在 batch 外层用 `buffer_unordered` 调度带输入 index 的 future，再按 index 恢复稳定输出顺序。AI 启用条件集中到纯函数；Enumeration prompt 默认传 `ai_assist=false`，使 deterministic collection 成为默认行为。

**技术栈：** Rust 2021、Tokio、futures `StreamExt::buffer_unordered`、`serde_json`、cargo nextest、clippy。

## 文件结构

- `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`：batch schema、并发归一、并发调度、AI opt-in、单元/异步回归测试。
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`：Enumeration 的 deterministic-first batch 调用合同。
- `backend/crates/golish-sub-agents/src/defaults/tests.rs`：prompt 合同回归。
- `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：公开 batch/AI 行为。
- `docs/modules/backend/golish-sub-agents/defaults.md`：默认 prompt 行为。
- `docs/modules/INDEX.md`：模块卡状态保持为已覆盖。

## 任务 1：以失败测试锁定 batch 并发与稳定顺序

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`

**步骤：**

1. 新增异步测试 `browser_batch_concurrency_is_bounded_and_results_keep_input_order`：构造 8 个 indexed future，用共享 active/peak 计数器与 barrier 证明峰值为 4；让一个 future 返回 `Err`，仍断言最后一个 sibling 被执行且聚合结果恢复为 index `0..7`。
2. 先运行测试，确认它因 `browser_batch_concurrency` / `run_browser_batch_bounded` 尚不存在而失败：

   ```bash
   cd backend && cargo test -p golish-pentest-app browser_batch_concurrency_is_bounded_and_results_keep_input_order --lib
   ```

3. 实现 `browser_batch_concurrency(args, target_count)`：默认 4，输入归一到 `1..=4`，并不超过 accepted target 数。
4. 实现 `run_browser_batch_bounded`：给输入附 index，经 `buffer_unordered(concurrency)` 执行，收集后按 index 排序；不使用 `tokio::spawn`，保留 task-local org/session/operation context。
5. 让 batch 路径复用既有单 root `execute_single`，逐项保留成功/错误，不因单项错误取消 sibling；响应写入实际 `batch_concurrency`。

**验证：**

```bash
cd backend && cargo test -p golish-pentest-app browser_batch_concurrency_is_bounded_and_results_keep_input_order --lib
```

预期：测试通过，peak concurrency 为 4，结果 index 顺序为 `0..7`，错误 sibling 不阻止最后一项完成。

**提交：**

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs
git commit -m "perf(enumeration): bound browser batch concurrency"
```

## 任务 2：以失败测试锁定工具内 AI 显式 opt-in

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`

**步骤：**

1. 新增纯函数测试 `enumeration_internal_ai_is_opt_in_and_ai_assist_false_wins`，覆盖四个输入：缺省关闭、仅 `ai_assist=true` 仍关闭、`ai=true && ai_assist=true` 开启、`ai_assist=false` 覆盖 `ai=true`。
2. 新增 schema 测试 `schema_exposes_bounded_batch_concurrency_and_opt_in_internal_ai`，断言 `batch_concurrency.default=4`、`maximum=4`、`ai.default=false`。
3. 先运行两项测试，确认它们因 schema/启用条件仍沿用隐式默认而失败：

   ```bash
   cd backend && cargo test -p golish-pentest-app enumeration_internal_ai_is_opt_in_and_ai_assist_false_wins --lib
   cd backend && cargo test -p golish-pentest-app schema_exposes_bounded_batch_concurrency_and_opt_in_internal_ai --lib
   ```

4. 实现 `browser_internal_ai_enabled(args)`，只返回 `ai_assist && ai`；`ai` 缺省为 `false`。
5. 在 tool schema 公开 `ai: boolean` 与 `batch_concurrency`，并让单 root AI recipe 分支只读取统一 helper 的结果。

**验证：**

```bash
cd backend && cargo test -p golish-pentest-app enumeration_internal_ai_is_opt_in_and_ai_assist_false_wins --lib
cd backend && cargo test -p golish-pentest-app schema_exposes_bounded_batch_concurrency_and_opt_in_internal_ai --lib
```

预期：两项通过；默认调用不会产生隐藏工具内 LLM 请求。

**提交：**

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs
git commit -m "fix(enumeration): require explicit browser AI opt-in"
```

## 任务 3：收紧 Enumerator prompt 并同步模块卡

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- 修改：`backend/crates/golish-sub-agents/src/defaults/tests.rs`
- 修改：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改：`docs/modules/backend/golish-sub-agents/defaults.md`
- 修改：`docs/modules/INDEX.md`

**步骤：**

1. 在 prompt 测试中断言推荐 browser batch 明确使用 `ai_assist=false`，route batch 明确使用 `batch_concurrency=4`，不得指导模型默认开启内部 AI。
2. 先运行测试，确认旧 prompt 缺少对应约束时失败：

   ```bash
   cd backend && cargo test -p golish-sub-agents enumeration --lib
   ```

3. 更新 Enumerator prompt：当前 worklist page 作为 bounded batch 处理；browser 默认 deterministic collection，只有明确诊断需要时才允许受限二次 recipe。
4. 更新两张模块卡，记录并发上限、稳定结果顺序、单 root 安全检查复用，以及 `ai_assist=false` 对 `ai=true` 的硬覆盖；保持 `docs/modules/INDEX.md` 状态为 `✅`。

**验证：**

```bash
cd backend && cargo test -p golish-sub-agents enumeration --lib
git diff --check -- docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/INDEX.md
```

预期：prompt 测试通过，文档无空白错误。

**提交：**

```bash
git add backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-sub-agents/src/defaults/tests.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/INDEX.md
git commit -m "docs(enumeration): define deterministic browser batch contract"
```

## 任务 4：运行范围回归与仓库门禁

**文件：**

- 验证：上述所有文件

**步骤：**

1. 运行 browser collector 现有 deadline、authorization、outcome 与 batch 测试，确认有界并发未改变单 root 语义。
2. 运行两个相关 crate 的完整测试与 clippy。
3. 运行仓库提交前门禁；失败必须修复到全绿后才可将 feature 标为 `passing`。

**验证：**

```bash
cd backend && cargo nextest run -p golish-pentest-app -p golish-sub-agents --status-level fail
cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents --all-targets -- -D warnings
cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents -- --check
just precommit
```

预期：所有命令 exit 0，clippy 零 warning。

**提交：**

```bash
git status --short
```

仅核对状态；是否提交或推送由用户授权决定，不在本计划中自动执行。
