# golish / models

> **一句话职责**：模型注册表 Tauri 命令——把 `golish-models` 的模型/能力/provider 元数据暴露给前端。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/models/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改前端获取模型列表/能力/provider 元数据的 Tauri 命令时

## 职责

`golish-models`（模型注册表）的 Tauri 命令面，供前端 Settings/模型选择 UI 读取可用模型 + provider 元数据。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | 模型注册表 Tauri 命令 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub mod commands` |
| `commands/` | 模型命令 |

## 依赖

- `golish-models`、`tauri`

## 注意事项 / 坑

- 模型能力以 `golish-models` 注册表 metadata 为准（JSON 驱动）；命令只读暴露，别在此硬编码模型清单。

## 测试入口

```bash
cd backend && cargo nextest run -p golish models
```
