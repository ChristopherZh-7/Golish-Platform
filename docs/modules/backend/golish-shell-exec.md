# golish-shell-exec

> **一句话职责**：shell 命令执行——从用户 rc 文件（`.zshrc`/`.bashrc`）继承 PATH 执行命令，含长命令的流式输出变体；GUI/agent 的权威 `run_pty_cmd` 生命周期由 `golish-app-core` 受管进程层接管。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-shell-exec/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `run_pty_cmd` 工具、shell 命令执行、PATH 继承、流式输出、超时/取消时
- pentest 工具调用（scan-runner/pentest）底层 shell 执行相关时

## 职责

执行 shell 命令并正确继承用户 shell 的 PATH；支持流式输出（mpsc 实时 chunk）。本 crate 的 `RunPtyCmdTool` 是兼容 fallback；GUI 主/子 agent 注册的是 `golish-app-core::VisibleRunPtyCmdTool`，从 spawn 起返回同一受管进程 handle。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `RunPtyCmdTool`（工具名 `run_pty_cmd`） | 同步 shell 工具 |
| `execute_streaming` / `OutputChunk` / `OutputStream` / `StreamingResult` | 流式执行 |
| `build_shell_command` / `which_executable` / `default_shell_invocation` | shell 构建/查找 |
| `Tool`（re-export 自 golish-core） | — |

## 依赖

- **内部**：`golish-core`、`golish-platform`

## 被谁依赖 / 改动影响面

`golish-tools`（注册 run_pty_cmd）、`golish-scan-runner`、`golish-pentest`、`golish-pentest-mcp`、`golish-sub-agents`。

## 关键文件（无目录子模块）

| 文件 | 作用 |
|---|---|
| `tool.rs` | `RunPtyCmdTool` |
| `streaming.rs` | 流式执行 |
| `shell.rs` / `cross_shell.rs` | shell 类型检测 + rc-file 感知包装 |
| `process_group.rs` | Unix 进程组（保留给显式取消清理整条 pipeline） |
| `common.rs` | `MAX_OUTPUT_SIZE` / `resolve_cwd` / `truncate_output` |

## 注意事项 / 坑

- `run_pty_cmd` 的成功/失败遵循 golish-tools 的契约（成功带 `exit_code:0`，失败非零/带 error）。
- agent 路径不得直接调用本 crate 的 `execute_streaming` 绕过共享 manager；`run_command` alias也必须路由到 `VisibleRunPtyCmdTool`。
- legacy `timeout` 输入不再形成elapsed watchdog；兼容 fallback自然等待退出并返回`automatic_kill:false`。真正长命令由app-core manager提供live output、可调yield、`check_job`与显式`kill_job`。
- 显式取消靠进程组清理整条 pipeline，别只杀父进程。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-shell-exec
```
