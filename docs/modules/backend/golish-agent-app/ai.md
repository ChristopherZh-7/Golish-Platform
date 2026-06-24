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
| `commands/` | Tauri handlers；`commands/bridge_config.rs` 负责注册 background completion/output listeners |
| `db_bridge/` / `tracking_bridge/` | 主要 bridge 实现 |

## 依赖

- crate 内 app-core；`golish-agent-bridge`/`kit`/`runtime`、`golish-db`/`graphiti`/`indexer`/`sidecar`/`mcp`、`tauri`

## 注意事项 / 坑

- 各 bridge 是**依赖倒置的实现侧**：agent-kit 定义 trait（`db_traits`），这里用 golish-db 等实现并注入——别把 golish-db 依赖塞回 agent-kit。
- `db_bridge/evidence.rs` 同时实现 evidence ledger、`technique_outcomes`、`source_query_log` 的 harness read/write seam；`source_query_facts` 只投影 source/provider terminal rows，不代表 found。
- `harness_submit_tool` 的 `submit_stage_deliverable` schema 是模型看到 coverage cell 字段的最后一道说明；EAS 的 explicit coverage 要在这里也讲清 SERVICE-FINGERPRINT denominator（已指纹开放端口 / 发现开放端口），避免 gate retry 时继续交空分母。
- `submit_stage_deliverable` 在 active stage 内会先检查本 session 归因的 background jobs；生产默认快速 `needs_fix`，要求模型调用 `wait_for_background_jobs` 显式等待、读取完成 job 的 stdout/stderr tail 后再提交。`GOLISH_SUBMIT_RECONCILE_WAIT_MS` 可恢复旧的 bounded in-submit wait，但不应作为默认 UI 体验。
- `commands/bridge_config.rs` 每个非 title-gen session 会监听 `background_jobs` completion 与 live output：completion 负责 evidence/note，live output 转成 `AiEvent::ToolOutputChunk` 给前端现有工具详情面板追加显示。后台 completion 不能统一记成 generic evidence：`httpx`/`whatweb`/`curl`/`wget` 要落 `http_probe`，`nmap` 落 `nmap`，否则 submit preview / gate 的 `min_invocations` 看不到真实工具证据。
- `harness_submit_tool` 在 validate-on-submit 前会从已引用的真实 evidence id（顶层 `evidence_refs`、claim `evidence_ids`、finding/coverage refs）查询 ledger kind，并按 stage spec 回填 `required_checks_done`（如 `http_probe`），避免后台证据已存在但模型漏填 hint 时被误导去重跑。
- 扁平 re-export 是 A3 删 umbrella 的兼容垫片，镜像 umbrella 旧导出；别乱删。
- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app ai
```
