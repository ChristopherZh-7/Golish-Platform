# golish-integrations / storage

> **一句话职责**：`StorageBackend` 的三种实现——`ExternalFileBackend`（渲染字段到 YAML/JSON 合并进既有文件，原子写+滚动备份）、`VaultBackend`（写 `vault_entries` 表，一字段一行）、`SettingsBackend`（经 `SettingsManager` 点路径写）。

- **类型**：目录模块（属于 crate [`golish-integrations`](../golish-integrations.md)）
- **路径**：`backend/crates/golish-integrations/src/storage.rs`（+ `storage/`）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改集成凭据的持久化后端（external_file / vault / settings）时
- 改 ENScan 的 `~/.config/enscan/config.yaml` 合并写、vault 行聚合、settings 点路径写时

## 职责

实现 `crate::traits::StorageBackend`，按 schema 的 `Storage` 变体把字段值落盘/读取：
- `ExternalFileBackend`：把字段渲染成 YAML/JSON 合并到既有文件（原子写 + 滚动备份，`preserve_unknown_keys`）。
- `VaultBackend`：写 `vault_entries` 表（一字段一行，`tags=["integration-group", <tool>, <group>]`；并读旧 `tags=["intel-provider", X]` 做向后兼容别名）。
- `SettingsBackend`：经 `SettingsManager` 在 `SettingsStorage::key` 点路径读写。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ExternalFileBackend` | 外部文件后端（YAML/JSON 合并写） |
| `VaultBackend` | vault 表后端（一字段一行） |
| `SettingsBackend` | settings 点路径后端 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `storage.rs` | 模块根（`pub mod` + re-export） |
| `storage/external_file.rs` | 外部文件后端（原子写 + 备份） |
| `storage/vault.rs` | vault 表后端（含 intel-provider 旧 tag 兼容） |
| `storage/settings.rs` | settings 后端 |

## 依赖

- `crate::traits::StorageBackend`、`crate::schema`、`golish-db`（vault 表）、`golish-settings`、`golish-core`

## 注意事项 / 坑

- `ExternalFileBackend` 必须 `preserve_unknown_keys`（合并而非覆盖用户既有配置）+ 原子写 + 滚动备份，别改成整覆盖。
- `VaultBackend` 读旧 `intel-provider` tag 是**向后兼容别名**，别删（老数据会丢）。
- 一字段一行 + tags 聚合是 vault 的约定，新增字段沿用。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-integrations storage
```
