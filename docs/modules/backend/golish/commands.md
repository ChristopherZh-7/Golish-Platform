# golish / commands

> **一句话职责**：GUI 进程的 Tauri 命令面（按域分组）——`fs`（文件 CRUD/watcher/路径补全）、`proc`（进程/终端/shell/git/history）、`project`（项目级 agent 资产：prompts/rules/skills）、`ui`（主题/IME/前端日志转发）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/commands/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 golish-staying 的 Tauri 命令（文件/进程/终端/git/项目资产/UI chrome）时
- 改文件 watcher / 路径补全 / 前端日志转发时

## 职责

承载留在 golish crate 的 Tauri 命令（未拆进 per-domain app crate 的那些），按域分组。消费方经 `crate::commands_facade::<domain>` 访问；命令面不再走本层 `pub use *`（facade 是单一事实源）。

## 公开接口

| 子模块 | 说明 |
|---|---|
| `fs` | 文件 CRUD / watcher / 路径补全（含 `FileWatcherState`） |
| `proc` | 进程 / 终端 / shell / git / history |
| `project` | 项目级 agent 资产（prompts/rules/skills） |
| `ui` | 主题 / IME / 前端日志转发 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 4 域声明 + 必要类型 re-export（如 `FileWatcherState`） |
| `fs/` / `proc/` / `project/` / `ui/` | 各域命令 |

## 依赖

- crate 内 state/app-core；`tauri`、`notify`（watcher）、`nucleo-matcher`（补全）

## 注意事项 / 坑

- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`；经 `commands_facade` 暴露给 `generate_handler!`。
- 类型 re-export（如 `FileWatcherState`）供 `app::tauri_app` managed-state 注册；命令本身不在本层 `pub use`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish commands
```
