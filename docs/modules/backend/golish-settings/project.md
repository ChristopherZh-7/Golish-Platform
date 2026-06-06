# golish-settings / project

> **一句话职责**：per-project 设置——`{workspace}/.golish/project.toml` 里只存**覆盖项**（AI provider / model / agent_mode），不替代全局 `~/.golish/settings.toml`。

- **类型**：目录模块（属于 crate [`golish-settings`](../golish-settings.md)）
- **路径**：`backend/crates/golish-settings/src/project/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改项目级设置覆盖（每个 workspace 记住自己的 provider/model/agent_mode）时
- 改 `project.toml` 加载/保存/清除逻辑时

## 职责

`ProjectSettingsManager` 管理 per-workspace 的设置覆盖：从 `{workspace}/.golish/project.toml` 加载，只持有 `Some()` 的覆盖字段（不影响其它全局配置）。原子写（unique temp 名 + rename），无覆盖时不落盘，`clear` 删文件。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ProjectSettings` / `ProjectAiSettings` | 覆盖结构（`ai.{provider,model,agent_mode}`，全 `Option`） |
| `ProjectSettingsManager`（`new(workspace)` / `get` / `update` / `update_ai_settings` / `set_model` / `set_agent_mode` / `reload` / `clear` / `config_path`） | per-project 设置句柄 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `ProjectSettings` + `ProjectSettingsManager` 本体 |
| `tests.rs` | 单测 |

## 依赖

- `crate::schema::AiProvider`、`tokio`（`RwLock`/`fs`）、`toml`、`serde`、`anyhow`

## 注意事项 / 坑

- **只存覆盖项**：`Option` 字段 `None` 时不序列化（`skip_serializing_if`），全 `None` 时 `save` 直接返回（不建空文件）。
- 与全局设置是**叠加**关系（项目覆盖全局），不是替换；消费方需自行 merge。
- 原子写用 `project.toml.{pid}.tmp` 唯一名防并发冲突。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-settings project
```
