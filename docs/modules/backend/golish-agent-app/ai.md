# golish-agent-app / ai

> **一句话职责**：agent 命令面 + AppState-free 桥接——`commands/*` Tauri handlers + 各 bridge（db/tracking/session/graph/embedder/sidecar）+ harness 工具（submit/trace/start_operation），并扁平 re-export agent 栈（A3 删 golish-ai umbrella 后直接从实现 crate 暴露）。

- **类型**：目录模块（属于 crate [`golish-agent-app`](../golish-agent-app.md)）
- **路径**：`backend/crates/golish-agent-app/src/ai/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 agent Tauri command（`ai/commands/*`）、各 bridge（把 agent-kit 的 trait 接到 golish-db/graphiti/indexer/sidecar 实现）时
- 改 harness 工具（`harness_submit_tool`/`harness_trace_tool`/`start_operation_tool`）时

## 职责

agent 服务命令面宿主。`commands/` 是 Tauri handlers；各 `*_bridge` 把 `golish-agent-kit::db_traits` 等 trait 用具体基础设施（golish-db / graphiti / indexer / sidecar / embedder）实现并注入；harness 工具暴露 stage harness 给 agent。`ai/mod.rs` 还扁平 re-export agent 栈（替代已删的 `golish-ai` umbrella）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | agent Tauri command handlers |
| `db_bridge` / `tracking_bridge` / `session_bridge` / `graph_bridge` / `embedder_bridge` / `sidecar_bridge` | trait → 基础设施实现注入 |
| `harness_submit_tool` / `harness_trace_tool` / `start_operation_tool` | harness 工具 |
| flat re-export（`AgentBridge` / `tool_*` / `route_tool_execution` / …） | 替代 golish-ai umbrella（A3） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 子模块声明 + 扁平 re-export |
| `commands/` | Tauri handlers |
| `db_bridge/` / `tracking_bridge/` | 主要 bridge 实现 |

## 依赖

- crate 内 app-core；`golish-agent-bridge`/`kit`/`runtime`、`golish-db`/`graphiti`/`indexer`/`sidecar`/`mcp`、`tauri`

## 注意事项 / 坑

- 各 bridge 是**依赖倒置的实现侧**：agent-kit 定义 trait（`db_traits`），这里用 golish-db 等实现并注入——别把 golish-db 依赖塞回 agent-kit。
- `db_bridge/evidence.rs` 同时实现 evidence ledger、`technique_outcomes`、`source_query_log` 的 harness read/write seam；`source_query_facts` 只投影 source/provider terminal rows，不代表 found。
- 扁平 re-export 是 A3 删 umbrella 的兼容垫片，镜像 umbrella 旧导出；别乱删。
- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app ai
```
