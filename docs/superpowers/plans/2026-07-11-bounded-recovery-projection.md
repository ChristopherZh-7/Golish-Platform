# Bounded recovery-action model projection implementation plan

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 保留完整 recovery actions 作为确定性工具 guard 真相，同时把所有模型可见 recovery 文本和 blocked-tool payload 限制为稳定、可分页的有界投影。

**架构：** `golish-agent-kit` 使用已有全量 `RepairDirective.actions` 和 `gap_hash` 生成最多 20 条、32 KiB 的投影；转换到 `SubmitRepairMode` 时保留全量 actions，但用投影标记阻止二次展开。`golish-sub-agents` 对独立创建的 repair mode 使用同样的样本/硬上限，并把 blocked payload 的完整数组替换为 `total/hash/sample/omitted/next_page_tool` 对象。

**技术栈：** Rust 2021、serde/serde_json、sha2（agent-kit 已有）、cargo nextest、clippy。

## 文件边界

- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`：StageRefiner 投影、全量转换保持、1176-action tests。
- 修改 `backend/crates/golish-sub-agents/src/executor_types.rs`：SubmitRepairMode 投影、幂等去重、blocked payload 投影、1176-action tests。
- 更新 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-sub-agents.md` 和 `docs/modules/INDEX.md`：记录模型投影与内部 guard 的边界。

`executor/response_parsing.rs` 正由并行的 chain/context agent 修改。本任务不触碰该文件；blocked payload 旧数组断言由主 agent 在并行改动汇合后统一适配为投影对象断言。

## Task 1：写失败回归测试

**步骤：**

1. 在 StageRefiner tests 构造 1,176 个带唯一 marker 的 Enumeration gaps。
2. 断言模型文本包含 `total=1176`、稳定 hash、前 20 个样本和分页指令，不包含 action 21/1176，且不超过 32 KiB。
3. 断言 `to_submit_repair_mode()` 后内部 action 数仍为 1,176；重复 `model_instruction()` 完全相等。
4. 在 SubmitRepairMode tests 覆盖同样的独立 `directive_message=None` 路径以及 blocked payload 的 64 KiB 上限。

**验证：**

```bash
cd backend && cargo nextest run -p golish-agent-kit recovery_projection --status-level fail
cd backend && cargo nextest run -p golish-sub-agents recovery_projection --status-level fail
```

预期：新测试因当前代码展开全部 1,176 actions、缺少 hash/分页摘要而失败。

## Task 2：实现最小有界投影

**步骤：**

1. 增加常量：样本 20、instruction 32 KiB、blocked payload 64 KiB。
2. StageRefiner 仅渲染前 20 条，头部先写 total/hash/pagination；对最终 UTF-8 字符串做确定性硬截断。
3. `to_submit_repair_mode()` 继续复制全量 actions，并传递已经带投影标记的 bounded directive。
4. SubmitRepairMode 在没有已有投影时生成 stable full-vector hash + bounded sample；已有投影时不重复追加。
5. blocked payload 用 bounded projection object 替换完整 action array；tool guard 逻辑继续读取内部全量 vector。

**验证：** 重跑 Task 1 命令，预期全部通过。

## Task 3：兼容性与静态检查

**步骤：**

1. 运行两 crate 的 recovery/repair scoped tests。
2. 格式化并对两个 crate 跑 scoped clippy。
3. 对本任务文件运行 `git diff --check`。
4. 向主 agent 明确交接 `response_parsing.rs` 中旧 blocked payload 数组断言的单点适配。

**验证：**

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_refiner --status-level fail
cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair --status-level fail
cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -- --check
cd backend && cargo clippy -p golish-agent-kit -p golish-sub-agents --lib --tests -- -D warnings
git diff --check -- backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs backend/crates/golish-sub-agents/src/executor_types.rs docs/design/2026-07-11-bounded-recovery-projection.md docs/superpowers/plans/2026-07-11-bounded-recovery-projection.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-sub-agents.md docs/modules/INDEX.md
```

预期：所有命令 exit 0，无 warning；不运行 DB、live stage 或 repo-wide precommit。
