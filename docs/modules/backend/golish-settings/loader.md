# golish-settings / loader

> **一句话职责**：`SettingsManager`——加载/保存 `~/.golish/settings.toml`、`$VAR`/`${VAR}` 环境变量插值、原子写（temp+rename）、首次运行模板生成、前向兼容 schema 迁移。

- **类型**：目录模块（属于 crate [`golish-settings`](../golish-settings.md)）
- **路径**：`backend/crates/golish-settings/src/loader/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改设置加载/保存逻辑、dot-notation 取值（`ai.vertex_ai.project_id`）、原子写时
- 改环境变量插值（`$VAR`/`${VAR}`、proxy env）或 schema 迁移（`migrate_settings`）时
- 设置文件并发写冲突、首次运行模板生成问题时

## 职责

`SettingsManager` 是设置的运行时句柄：从磁盘加载（带迁移 + env 解析）、缓存（`RwLock`）、dot-notation 读写、原子持久化、首次运行从 `template.toml` 生成。`write_mutex` 串行化文件写，防快速 onChange 时 temp 文件 rename 竞态。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SettingsManager`（`new` / `load_standalone` / `get` / `update` / `get_value` / `set_value` / `reset` / `ensure_settings_file` / `reload`） | 设置运行时句柄 |
| `settings_path()` | `~/.golish/settings.toml` 路径 |
| `apply_proxy_env` / `get_with_env_fallback`（来自 `env`） | proxy env / env 回退取值 |
| `migrate_settings`（来自 `migration`） | 前向兼容 schema 迁移 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `SettingsManager` 本体 + dot-notation 嵌套读写 |
| `env.rs` | `$VAR`/`${VAR}` 插值 + proxy env + `resolve_env_vars` |
| `migration.rs` | 旧版 TOML 迁移链 |

## 依赖

- `crate::schema::GolishSettings`、`tokio`（`RwLock`/`Mutex`/`fs`）、`toml`、`dirs`、`anyhow`

## 注意事项 / 坑

- **原子写**：所有保存走 temp 文件 + rename，且 `write_mutex` 串行化——别改成直接覆盖写（会丢/串数据）。
- **迁移在反序列化前**：`load_from_path` 先对 raw `toml::Value` 跑 `migrate_settings` 再 deser；改 schema 必须 bump `SCHEMA_VERSION`（在 `schema/`）并加迁移条目。
- env 插值在加载时一次性解析进缓存；运行期改 env 不自动重读，需 `reload`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-settings loader
```
