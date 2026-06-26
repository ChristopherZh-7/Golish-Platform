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
- `query_target_data` 的 enumeration 读模型支持 `sections=["directories","coverage","web_roots"]`：`directories` 读 target-bound `directory_entries`，`coverage` 返回 DIR/PARAM/JSAPI 的 found-only DB truth summary，`web_roots` 从 EAS 已落 target URL / http metadata / web-like ports 派生根 URL。缺席的 coverage fact 不是 checked_empty。
- `ai_get_stage_asset_coverage` 是 `stage_run` 详情页的只读矩阵接口：按 `(organization_id, stage, optional session_id)` 返回 asset × technique snapshot，found 来自 `coverage_truth`，checked_empty/error/blocked 来自 `technique_outcomes` + `source_query_log` terminal rows，适用性复用 `technique_resolver`。`error` 表示工具/来源执行失败，`blocked` 表示 scope/key/rate-limit/人工阻塞，二者不要混成同一个状态。target_intel 会额外返回 organization 覆盖行，让 provider/WHOIS/OSINT 在尚未落真实 target 前也能被解释；前端必须把它显示为「组织情报」，不要混入资产列表或资产分母。它只解释当前 gate 输入，不自行决定 PASS/BLOCK。
- `db_bridge/evidence.rs` 同时实现 evidence ledger、`technique_outcomes`、`source_query_log` 的 harness read/write seam；`source_query_facts` 只投影 source/provider terminal rows，不代表 found。
- `harness_submit_tool` 的 `submit_stage_deliverable` schema 是模型看到 coverage cell 字段的最后一道说明；DB-truth 阶段（target_intel / EAS found cells）要明确让模型不要手抄 found coverage，只提交 DB 不能推导的 terminal cells；保留 SERVICE-FINGERPRINT denominator 文案仅用于显式 non-DB-truth / negative cells。
- `submit_stage_deliverable` 若 gate 因 `coverage_complete` BLOCK，会把 `HarnessRecoveryActions.coverage_gap_actions` 原样放进 `needs_fix.coverage_gap_actions`，让 sub-agent 下一轮拿到结构化 action list（而不是只读 `reasons` 里的前 8 个字符串）。
- `submit_stage_deliverable` 在 active stage 内会先检查本 session 归因的 background jobs；生产默认快速 `needs_fix`，要求模型调用 `wait_for_background_jobs` 显式等待、读取完成 job 的 stdout/stderr tail 后再提交。`GOLISH_SUBMIT_RECONCILE_WAIT_MS` 可恢复旧的 bounded in-submit wait，但不应作为默认 UI 体验。
- `commands/bridge_config.rs` 每个非 title-gen session 会监听 `background_jobs` completion 与 live output：completion 负责 evidence/note，并把成功 job 的 stdout tail 送入 `golish-pentest::output_store::maybe_detect_and_store_via` 做结构化落库；live output 转成 `AiEvent::ToolOutputChunk` 给前端现有工具详情面板追加显示。同步前台 `PostShellHook` 使用 `maybe_detect_and_store_via_context` 并传入当前 harness org，让 EAS 主动发现的新资产进入该 org 分母。后台 completion 的 evidence `tool_name` 必须从命令行解析真实工具名，不能统一记成 `background_job`：`httpx`/`whatweb`/`curl`/`wget` 要落 `http_probe`，`nmap` 落 `nmap`，`naabu` 落 `port_probe`，否则 submit preview / gate 的 `min_invocations` 看不到真实工具证据。WhatWeb 可能通过 Ruby wrapper 启动（`ruby .../whatweb`），解析时要透过解释器取真实工具名。
- `cancel_ai_generation` / `shutdown_ai_session` 不只设置 `AgentBridge.cancelled`；还必须 kill 当前 `session_id` 归因的 running background jobs，避免 ChatPanel Stop 只停主 agent 而留下 worker 扫描进程继续跑。
- `harness_submit_tool` 在 validate-on-submit 前会从已引用的真实 evidence id（顶层 `evidence_refs`、claim `evidence_ids`、finding/coverage refs）查询 ledger kind，并按 stage spec 回填 `required_checks_done`（如 `http_probe`），避免后台证据已存在但模型漏填 hint 时被误导去重跑。
- 扁平 re-export 是 A3 删 umbrella 的兼容垫片，镜像 umbrella 旧导出；别乱删。
- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app ai
```
