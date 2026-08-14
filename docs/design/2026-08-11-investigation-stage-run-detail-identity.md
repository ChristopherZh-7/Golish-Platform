> Superseded by `2026-08-11-investigation-detail-read-identity.md` for read routing. Runtime progress may expose the outer call, but durable Investigation authority remains stable per StageExecution.

# Investigation `stage_run` 详情真实身份契约

## 问题

Unified Investigation 已经在数据库中正常运行，但 ChatPanel 的 `stage_run` 卡片无法进入 Investigation Workspace。普通 Company Stage 会用真实外层 tool request id 生成 `StageRunOrgProgress`；Investigation 则提前进入专用 runtime，既不发这条首帧事件，又把 `owning_stage_run_request_id` 写成 `stage_run:<stage_execution_id>`。聊天卡实际选中的是 `call_...`，前端 exact resolver 因 request identity 冲突而按设计 fail closed。

实体证据来自 Test1 session `pentest-chat-1786431419372-1`：外层请求 `call_01_K261nZhwhJmlygvGOrUL3507` 只有 `{orgs:[...]}`，Investigation `stage_run_org_progress` 与 projection refresh 均为空；同一时刻 DB execution `65ef8af7-5eb8-4ce7-87a5-836d5b5e6e35`、Unit、Primary 和多个 cognition Worker 已处于 running/passed，证明执行正常而路由身份缺失。

## 决策

1. 外层真实 `stage_run` tool request id 是统一 Investigation 的唯一 owning request identity；禁止再生成 `stage_run:<execution-id>` synthetic owner。
2. `execute_unified_investigation_stage_run` 必须接收 `tool_id`。在 exact operation、StageExecution 与 frozen Team set 验证完成后，立即为每个 Unit 发 request-scoped `StageRunOrgProgress`，其 `agent_request_id` 为 `<tool_id>::team::<org-id>`。
3. replayed closure 发 `passed` 首帧；新执行发 `running` 首帧。事件只是详情刷新/路由 pointer，Investigation Workspace 仍通过 exact IPC 重读 DB projection，不把 event 当业务 truth。
4. terminal success result同时回传 `stage + operation_id + stage_execution_id + stage_run_request_id`，让 transcript restore 即使缺少 live store，也能恢复同一 exact route。
5. 不增加 latest fallback，不从当前 stage、时间顺序或 synthetic execution id 猜 tool request。旧的已运行 synthetic operation 不在线改库；新契约只对新启动/新二进制中的 Investigation 生效。

## 安全边界

- tool request id 只来自 runtime 当前 dispatch 的 `tool_id`，不接受模型参数。
- operation/execution/scope/Unit 继续由 frozen runtime-memory 与 repository 重验。
- frontend 现有 operation/execution/request 三方一致校验保持不变；任何冲突仍显示 unavailable。
- 不改 schema、migration、IPC generated types、Gate、evidence 或扫描授权。

## 定向验证

- `golish-agent-runtime` unit：真实 tool request 持久化、Investigation progress 的 request/op/execution/unit identity、terminal selector helper。
- frontend focused：running Investigation row 能进入 exact workspace；conflicting identity 继续 fail closed。
- 受影响 Rust 单文件 fmt、crate lib Clippy `-D warnings`；受影响前端 Biome 与 typecheck。
