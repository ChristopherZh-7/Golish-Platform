# golish-agent-runtime / execution_mode

> **一句话职责**：执行模式策略框架——每个模式（chat/task/未来 plan/debug）是一个 `ExecutionModePolicy`，决定本 turn LLM 看到哪些工具；`build_tool_list` 完全委托给 `ExecutionModeRegistry` 选中的 policy，加模式无需改 `tool_list.rs`。

- **类型**：目录模块（属于 crate [`golish-agent-runtime`](../golish-agent-runtime.md)）
- **路径**：`backend/crates/golish-agent-runtime/src/execution_mode/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加新执行模式（plan/debug…）或改 chat/task 模式的工具可见性策略时
- 改 `ExecutionModeRegistry` 注册、`PolicyContext`、prompt 模板渲染时

## 职责

把"本 turn 给 LLM 哪些工具"抽象成 policy。加模式只需：① `modes/<name>.rs` 实现 `ExecutionModePolicy`；② `modes/mod.rs` 加 `pub mod`；③ 在 `ExecutionModeRegistry::default` 注册；④（可选）加 tera 模板。`tool_list.rs` 不用动。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ExecutionModePolicy` / `ModeLabel` | 模式策略 trait + 标签 |
| `ExecutionModeRegistry`（registry） | 模式注册表 |
| `PolicyContext` | 策略上下文 |
| `AgentToolSelection` / `BridgeToolSelection` / `RuntimeToolSelection` | 各层工具选择结果 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `policy.rs` / `registry.rs` | policy trait / 注册表 |
| `modes/` | 各模式实现 |
| `context.rs` / `prompt_render.rs` / `selection_apply.rs` | 上下文 / 模板渲染 / 应用选择 |

## 依赖

- crate 内 `agentic_loop`（`build_tool_list` 委托）；tera（模板）

## 注意事项 / 坑

- **加模式不改 `tool_list.rs`**——这是本抽象的全部意义；都走 registry + policy。
- 与 `golish-agent-kit::execution_mode`（Chat/Task 枚举）不同层：那是模式枚举，这是模式 policy 框架。
- Task/profile policy 有两种 depth-0 形态：`PolicyContext.harness_active=false` 是普通 lead turn，暴露 flexible chat/coding 辅助工具 + `ask_human` + `start_operation`，但不暴露 shell、Tavily/web search、`security_analysis` 静态工具（`query_target_data` / `list_in_scope_targets` / `list_attack_surface_seeds` / `check_stage_asset_coverage`）、`manage_organizations` / `manage_targets` / `recon_*` / `pentest_run` / `submit_stage_deliverable` / `stage_run`；`harness_active=true` 才是 stage primary，暴露 submit/stage orchestration 工具并收紧普通工具面。active stage primary 还必须保留后台 job 控制面 `wait_for_background_jobs` / `check_job` / `kill_job`，否则 `submit_stage_deliverable` 因 pending background job 返回 needs_fix 后，模型会被 ToolGuard 挡在正确修复动作之外。不要把二者合并，否则会重现“你好直接进 Scoping”“正经任务无法自然启动”或“没进 harness 却直接查库/跑 shell”的问题。
- `selection_apply.rs` 必须真正按 `StaticGroupSelection` 过滤静态目录工具；不能只判断 `any_enabled()` 后把 `ToolSelectionConfig::main_agent()` 全量塞回去。Task/Profile lead 依赖这个机械过滤来避免安全查询工具泄漏。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime execution_mode
```
