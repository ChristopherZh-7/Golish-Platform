# golish-settings

> **一句话职责**：集中式 TOML 配置系统（`~/.golish/settings.toml`）——环境变量插值、原子写、首次运行模板、类型安全 schema。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-settings/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改配置项、读 `GolishSettings`、解析 API key、需要 env fallback 时
- 配置加载/插值/写盘出问题时

## 职责

管理 Golish 配置：从 `~/.golish/settings.toml` 加载、`$VAR` / `${VAR}` 环境变量插值、temp+rename 原子写、首次运行生成模板、serde 默认值的类型安全 schema。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `SettingsManager` | 加载/获取/保存设置 |
| `GolishSettings` | 顶层设置 schema |
| `get_with_env_fallback(opt, &["ENV"], default)` | 取值 + 环境变量回退 |
| `settings_path()` / `apply_proxy_env()` | 路径 / 代理环境 |
| `ProjectSettings` / `ProjectSettingsManager` | 项目级设置 |
| `schema::AiProvider`（被 golish-models re-export） | provider 枚举 |

## 依赖

- **内部**：无（仅外部 crate：serde / toml / tokio 等）

## 被谁依赖 / 改动影响面

`golish`、各 `*-app`、`golish-models`、`golish-llm-providers`、`golish-agent-kit/runtime/bridge`、`golish-integrations`、`golish-pty`、`golish-prompts`、`golish-tools`、`golish-synthesis`、`golish-sidecar`、`golish-artifacts` 等。改 schema 字段影响面广。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `loader/` | 加载/插值/原子写/env fallback | [→](golish-settings/loader.md) |
| `project/` | 项目级设置 | [→](golish-settings/project.md) |
| `schema/` | 设置类型定义（含 AiProvider） | [→](golish-settings/schema.md) |

## 注意事项 / 坑

- 值支持 `$VAR` / `${VAR}` 两种插值；写盘走 temp 文件 + rename 保证原子。
- `AiProvider` enum 在 `schema/`，模型相关代码常从这里 re-export。
- 加 schema 字段记得给 serde 默认值，否则旧配置文件加载会失败。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-settings
```
