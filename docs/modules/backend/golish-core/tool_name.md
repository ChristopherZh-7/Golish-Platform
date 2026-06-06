# golish-core / tool_name

> **一句话职责**：工具名与分类枚举——`ToolName`（agent 工具的强类型名）+ `ToolCategory`（工具分类），取代散落的字符串字面量。

- **类型**：目录模块（属于 crate [`golish-core`](../golish-core.md)）
- **路径**：`backend/crates/golish-core/src/tool_name/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 新增/重命名 agent 工具，需要登记 `ToolName` 枚举值时
- 改工具分类（`ToolCategory`）或工具名↔字符串映射时

## 职责

把 agent 工具名与分类做成强类型枚举（而非到处写字符串），供 registry、policy、definitions 等跨模块引用，避免拼写漂移。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ToolName` | agent 工具的强类型名枚举 |
| `ToolCategory` | 工具分类枚举 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `name.rs` | `ToolName` 枚举 + 字符串映射 |
| `category.rs` | `ToolCategory` 枚举 |

## 依赖

- 仅 crate 内基础（`serde`）

## 注意事项 / 坑

- 这是工具名的**单一事实源**：新增工具时在此登记，再到 `golish-tools` 实现 + `definitions/` 暴露 schema，三处保持一致。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-core tool_name
```
