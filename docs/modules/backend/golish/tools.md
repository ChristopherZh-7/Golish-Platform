# golish / tools

> **一句话职责**：工具薄包装——`pub use golish_tools::*` 兼容性 re-export（基础设施在 `golish-tools` crate）；pentest 工具服务已外移到 `golish-pentest-app`，scoping 守卫已移到 `golish-app-core`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/tools/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 排查 `crate::tools::*` 路径来源时（实际在 `golish-tools`）
- 确认 pentest 工具/scoping 已外移到哪 crate 时

## 职责

对 `golish-tools` 基础设施的薄兼容性包装（`pub use golish_tools::*`）。历史上还含 pentest 工具服务 + scoping，但 crate-per-service 拆分后：pentest 工具→`golish-pentest-app`（M3）、scoping→`golish-app-core`（L5）。工具实现见 [`golish-tools`](../golish-tools.md)。

## 公开接口

| 符号 | 说明 |
|---|---|
| re-export `golish_tools::*` | `ToolRegistry` / 各工具 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub use golish_tools::*`（+ 外移说明注释） |

## 依赖

- `golish-tools`

## 注意事项 / 坑

- 纯 re-export：改工具去 `golish-tools`，别在此加逻辑。
- **历史路径已变**：`crate::tools::scoping` 已删（用 `golish_app_core::scoping`）；pentest 工具在 `golish-pentest-app`。别按旧路径找。

## 测试入口

```bash
cd backend && cargo nextest run -p golish tools
```
