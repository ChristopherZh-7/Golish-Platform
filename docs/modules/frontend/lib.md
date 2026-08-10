# frontend / lib

> **一句话职责**：前端的非-UI 基础设施层——`api`（统一 Tauri 客户端，唯一允许 `invoke` 处）、`generated`（ts-rs 后端类型，**禁手改**）、`events`、`ai`、`pentest`、`models`、`settings`、`theme`、`timeline`、`terminal`、`i18n`、`ui-state` 等。

- **类型**：前端子系统
- **路径**：`frontend/lib/`（~260 ts/tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 调后端命令（必须经 `lib/api/<domain>.ts`，**禁裸 `invoke()`**）时
- 用跨 IPC 类型（从 `lib/generated/` import，ts-rs 生成）时
- 改前端 AI 逻辑、pentest 视图模型、主题、i18n、timeline、终端逻辑时

## 职责

承载前端所有非-UI 逻辑与后端边界。`api/` 是唯一允许 `invoke` 的地方（38 个域 wrapper + `client.ts` + `error-codes.ts`）；`generated/` 是 ts-rs 从后端生成的 wire 类型（手写文件**禁改**）；其余子目录是领域逻辑/工具。

## 关键子目录

| 子目录 | 说明 |
|---|---|
| `api/` | 统一 Tauri 客户端：`api.<domain>.<verb>`，38 域 wrapper + `client.ts`（持 `invoke`）+ `error-codes.ts` |
| `generated/` | **ts-rs 生成**的后端 wire 类型（禁手改） |
| `events/` / `ai/` | AI 事件类型 / 前端 AI 逻辑 |
| `pentest/` / `target-panel/` / `timeline/` | pentest 视图模型 / 目标面板 / 时间线 |
| `models/` / `settings/` / `theme/` / `i18n/` / `terminal/` / `ui-state/` / `serde_json/` | 模型 / 设置 / 主题 / 国际化 / 终端 / UI 状态 / JSON 工具 |

`stage-reset.ts` 是 ChatPanel dev full reset 的共享纯协议层：集中定义四个可原地重置的 Company stage、全部已知 harness stage、完整 committed receipt校验、local DAG suffix、current reset stage推断与selected stage v0 `in_progress` seed。组件、Zustand和localStorage persistence必须复用它；malformed/null receipt也先按本地suffix回卷，persistence再以自身stageOrder补齐durable-only descendants，不能分别手写stage列表或在plan缺席时用线性first-unpassed fallback猜current。
`api/harness-dev.ts` 只返回ts-rs生成的 `HarnessDevStageCheckpointResetResult`；wrapper不得丢弃`affectedStages/currentStage/refreshedStageCursor/resetGraphFlow/purgedFacts/purgeScopeOrgCount/purgeCounts/purgeNote`。前端以IPC返回作为backend commit边界，但仍通过`stage-reset.ts`验证业务回执后才auto-resume。

`ai/streaming-buffer.ts` 是流式更新的节流入口：text delta、reasoning/thinking、sub-agent thinking、tool output chunk、聊天气泡 thinking 都应先进入 16ms batch，再统一写 store；非文本边界事件（tool request/result/completed/error）前要 flush 对应 session/conv，保证显示顺序不漂。
`ai/execution-mode.ts` 是 execution mode/profile 的共享归一入口：`task` 是 legacy Task engine alias，恢复或写入持久化前必须用 `normalizeExecutionModeId` 转成具体 harness profile id（优先 last profile，否则 `assessment`）。`lastExecutionMode` localStorage 也存 profile id，不存裸 `task`，避免 reopen 后 UI 只显示普通 Task。
`scroll-stickiness.ts` 是 live detail / thinking panes 的贴底判定：向上滚动是用户接管信号，必须暂停 auto-follow；只有滚回底部阈值内才重新启用。
`ai/streaming-buffer.ts` 按 session + WorkerRun 级 `parent_request_id` 聚合 sub-agent reasoning，保留 batch 首末到达时间，并在工具/生命周期边界前 flush；Stage Team sibling worker 不得复用组织级 parent id，否则并发 Thought 会被拼成一条时间线。
`conversation-db-sync.ts` 的 autosave 指纹必须覆盖 timeline block 内容变化，而不只看 block 数量/最后一块 id；`sub_agent_activity.entries/toolCalls/result/thinking`、`ai_tool_execution.streamingOutput/result` 和 `Session.stageRuns[requestId]` 都是恢复 stage_run 历史详情所需状态，关窗前必须能触发 DB 保存。`terminal_state.stage_run_json` 允许 v2 包 `{ current, byRequestId }`，恢复端仍兼容旧的单个 `SessionStageRun` JSON。
`terminal-restore.ts` 对进程丢失时持久化为 running 的 sub-agent、running/backgrounded tool 与 generating prompt 一律收敛为 interrupted/failed 投影；后台 job registry 不随 terminal 持久化，因此不能把旧外部工具伪装成仍在运行。后续 durable worker 以相同 `parent_request_id` 真正续跑时，store 的 started 边界只恢复父 Agent；旧 tool 保持 interrupted，新请求才显示 running，历史 entries/tool ids 不丢失。

`i18n/en.json`与`zh-CN.json`的managed background文案只描述inline handoff和explicit stop；不得出现hard deadline/countdown。进程activity由live output/`check_job`提供，elapsed time本身不是失败或自动终止信号。

## 依赖

- `@tauri-apps/api`（仅 `lib/api/client.ts`）；被 `components`/`hooks`/`store`/`services` 广泛消费

## 注意事项 / 坑

- **不变量 I5**：跨 IPC 类型从 `lib/generated/` import（ts-rs 生成），**不要手写第二份**；`generated/` 下手写文件禁改（AGENTS.md §2.8）。
- **不变量（AGENTS.md §2.3）**：组件调后端走 `lib/api/<domain>.ts`，**禁裸 `invoke()`**；`invoke` 只在 `api/client.ts`。
- **不变量 I1**：错误按 `error-codes.ts` 的 `code` 翻译，不靠 HTTP status 做业务判断。
- 加新后端域：加 `lib/api/<domain>.ts` wrapper + 在 `api/index.ts` 注册。
- `api/stage-team.ts` 只接受 ts-rs 生成的 exact operation/stage-execution read request，以及 exact row/checkpoint CAS、active-tool lifecycle record id、稳定 request id 和 closed decision 的 operator recovery request；返回值始终以 DB-authoritative Team hierarchy/decision 为准。Harness trace 中的 stage execution/unit id 只是 refresh pointer；wrapper/组件不得把 event payload 当 Gate、Barrier 或 Worker truth，也不得暴露 lease secret、checkpoint body、工具名称/参数/结果、raw output 或 dynamic-request budget/body。operator recovery 只允许把 unknown external action 标成 blocked，不能提供 replay 入口。
- `api/attack.ts` 只接受 ts-rs 生成的 operation/wave scope、Candidate id/plan hash/row version/decision/expiry，以及 recovery case/request-id/双 expected versions/closed decision/exact evidence ids；actor/project/org/target/action args/budget/lease/checkpoint 不得进入 mutation DTO。Verification queue read model同样由 ts-rs 生成，可包含只读 pending FactDelta enrichment 的安全 subject/reason/allowed-techniques 元数据，但不得返回 raw request/evidence；`ATTACK_RECOVERY_CONFLICT` 与既有 `ATTACK_*` code 由 `error-codes.ts` 稳定翻译。
- `api/cleanup.ts` 只消费 ts-rs 生成的 operation/org read scope 与 exact operation/project-scope/snapshot/org/obligation/row-version waiver CAS；request 不含 actor id，可信 local principal 只在后端解析。
- `api/reporting.ts` 只消费 ts-rs 生成的 operation scope 与 revision/source CAS；`buildReportReadModel`、read/list/artifact/finalize 都走该 wrapper。actor、project root、storage path/content key 不得出现在 request DTO。
- `api/investigation.ts` 只消费 ts-rs 生成的 exact `sessionId + operationId + stageExecutionId + stageRunRequestId` selector。summary/list可传optional-all-or-none snapshot quartet，detail必须传完整 quartet；`sessionId`由当前 Tool detail pane提供，后端用它解析live bridge workspace并重验operation/task/session/project。request禁止携带workspace path、project id、principal或“latest”回退。
- `api/investigation.ts`公开summary、hypothesis list/detail、campaign list/detail、timeline list与explicit stop七个wrapper。cursor是后端签发的opaque token；`expectedChangeSeq + expectedTemporalCutoff + expectedAuthorityEpochSetHash + expectedEarliestEffectiveValidUntil`共同固定一次exact snapshot，前端不得解析/改写cursor或从本地列表/event重建authority。stop request只接受server control projection给出的exact run-state head/change seq与stable idempotency key。
- Plan B ts-rs closed enums为`ProjectionEntityKind`、`ProjectionInvalidationReason`、`ProjectionSourceTimeStatusV1`、`TimelineEventKind`；相关request/envelope/view也全部从`generated/`导入。V1 legacy projection仅作历史兼容并显式显示unavailable字段，不能补齐Registry authority。
- `generated/GeneratedAiEvent.ts` 与 `GeneratedHarnessTraceKind.ts` 的 Candidate V2 terminal/consolidation 分支由 ts-rs 生成：字段只允许 immutable scope/wave/unit/org/candidate/attempt/consolidation ids、terminal status/decision、聚合 counts 与 replay flag；没有 plan/result body/lease/exploit payload，也没有新的 attack command DTO。改 Rust wire 后必须用类型生成流程同步，禁止手改生成文件。
- `api/temporal-graph.ts` 只消费 ts-rs 生成的 closed scope/request/result；它与手写 legacy `lib/ai/kg.ts` 分离，不能添加 actorId/projectPath authority 字段。
- `target-panel/org-tree.ts` 是 TargetPanel 左侧组织树投影入口，默认只用于公司层级和计数；`summarizeTargetCounts` 同时给出 own 与 subtree 口径，UI 主数字必须用 own，删除/汇总才用 subtree；`target-panel/asset-groups.ts` 负责右侧 Targets 面板的 IP ⇄ 域名/URL 联合分组，避免大型客户的资产列表遮住子公司层级。IP target 必须按自己的 `value` 成组，即使 `real_ip` 里有 provider 归因值；只有 domain / url 才用 `real_ip` 挂到 IP 组，否则会出现“IP 下面挂 IP”的误导。IP 组的展示列表要把 `www.<apex>` 折叠到 `<apex>`，优先展示 apex，但不要折叠 `m.` / `api.` 等真实子域，底层 `targets` 和计数仍保留原始资产。
- `api/security-analysis.ts` 是 Target Surface 安全数据的 IPC 边界；后端命令返回 `serde_json` 时字段是 Rust snake_case（如 `asset_type` / `target_id` / `status_code`），wrapper 必须在这里归一成前端接口声明的 camelCase（`assetType` / `targetId` / `statusCode` 等），不要让组件直接消费未规整的原始行。`Fingerprint.evidence` 的 canonical shape 是 observation array，但历史 DB 行可能仍是单个 JSON object；normalizer 必须把非空 object 包成单元素数组，不能用普通 `arrayField` 把旧指纹静默变成无证据。`targetSurfaceHierarchyGet(targetId, includeRelated)` 是 Phase 2.4 的 backend identity hierarchy wrapper；当前没有 ts-rs 生成类型时，本地 DTO 只描述 command 的 camelCase 输出，组件仍通过 adapter 合成后消费。Phase 2.5B 本地 DTO 扩展了 backend 的 legacy content 聚合：`BackendWebOriginDto.contentCounts`（`BackendWebOriginContentCountsDto | null`）、`BackendSurfaceSummaryDto` 的 `urlCount/apiCount/jsCount/paramCount/directoryEntryCount/passiveLogCount/evidenceCount/contentUnassignedCount/contentUnmatchedOriginCount`（均 `number | null`）、`BackendUnassignedWebDataDto.counts`（`BackendUnassignedWebDataCountsDto | null`）。归一化时用 `presentNumberField` 区分「字段缺失（旧 payload → null，前端回退自身推断计数）」与「存在且为 0」，`contentCounts` / `counts` 缺失整块时归一为 `null`，绝不把缺失当 0。Phase 2.5C 再加轻量 refs 与 backfill：`BackendWebOriginDto.refs` / `BackendUnassignedWebDataDto.refs`（`BackendWebOriginContentRefDto[]`，旧 payload 归一为空数组）；`surfaceIdentityBackfill(projectPath?, organizationId?)` 包 `target_surface_identity_backfill` 命令并归一返回 `SurfaceIdentityBackfillSummary`。refs 是轻量指针（kind/id/url/method?/status?/capture?/source?），不是完整 legacy row。2026-07-04 起 `BackendWebOriginDto.crawlObservations`（`BackendCrawlObservationDto[]`，旧 payload 归一为空数组）表示来源 origin 下的 crawler URL 观察项；它不是 `ApiEndpoint` row，也不代表 coverage truth。
- `tools.ts::toolResultIndicatesFailure` 是工具结果显示态的共享判定：transport success 不等于业务成功，组件画状态图标前要检查 rejected/needs_fix/error/failed、非 0 exit、以及 stderr 里的 ERROR/FATAL/EXCEPTION。
- `tools.ts::getToolActionLabel` 是工具卡片头部的共享人类动作文案入口，折叠态不要直接展示 `snake_case` 内部工具名；raw tool name 只作为 hover/debug 信息。`pentest_run` 应优先显示意图（如 `Probing services` / `Scanning ports`），不要把 `Running Nmap nmap ...` 这种重复文案带到卡片里；backend wrapper 如 `eas_fingerprint_web_stack` / `enum_crawl_same_origin_urls` 也必须显示业务动作（`Fingerprinting web services` / `Crawling same-origin URLs`），而不是暴露内部工具名。Vuln Nuclei 工具必须区分 general 与 fingerprint-targeted 两种动作，旧单一 sweep 名不再展示。`tools.ts::getToolPrimaryArg` 是聊天工具卡、sub-agent 折叠行、工具执行卡的共享参数摘要入口；`wait_for_background_jobs` 必须在折叠态显示实际 `timeout_secs`，没传时显示默认 300s；`pentest_run` 带 `input_lines` / `stdin` 批量输入时必须显示 batch 数量和首尾目标，并把 `{{input_file}}` 这类展示占位替换成 `[input file]`，避免多批次工具看起来像重复执行同一条命令；`enum_crawl_same_origin_urls` 要显示 `target_urls` 批量数量/首尾目标和 depth；两个 Nuclei wrapper 显示 singular `target_url` + techniques，不再显示历史 batch targets。`getPentestRunInputLines` 是同一批量输入的共享解析器，live coverage 资产匹配也要复用它，避免 UI 展示是 batch、覆盖匹配却看不到资产。

## 测试入口

```bash
just check-fe   # biome + typecheck（含 ts-rs 绑定漂移检查）
just test-fe    # vitest
```

## Durable dispatch recovery UI support（2026-07-17）

- `api/stage-team.ts::getStageTeamReadModel` 也是 Company Controller detail 的历史派工恢复 authority：组件只用
  exact `operationId + stageExecutionId` 读取，并以 `requests[].parentWorkerRunId`、`acceptedWorkItemId` 和
  `workItems[].workers[]` 重建缺失的 UI 投影。
- `i18n/en.json` 与 `i18n/zh-CN.json` 提供恢复派工的 loading/error/empty/success 文案；这些文案不改变
  scheduler truth，也不能从 prose 推断 durable Request。

## Operation-scoped Vuln coverage API（2026-07-19）

- `api/stage-coverage.ts` 的 read wrapper可携带 operation id；Stage Team Vuln视图必须传 exact operation，不允许从最新 run或本地事件猜 coverage。返回的 cell `details` 只用于 attempt/retry显示，terminal truth仍由后端 state与 evidence refs决定。

## Organization deletion blocker translation（2026-07-19）

- `api/error-codes.ts` 把 `ORGANIZATION_DELETE_ACTIVE_STAGE_FORK` 翻译为“仍有活动执行者或未决工具结果，需先停止或恢复”的可操作提示；`i18n/*.json` 的删除确认同步披露 quiescent paused Task会被自动停止。未知 code 仍保留 backend fallback message，不能从字符串解析 blocker 类型。

## Hypothesis Registry readonly client（Plan B，2026-07-30）

- 历史operation-only Registry audit不再拥有production IPC入口；组件源码仅接受测试注入并默认fail closed。不得把它的selector伪装成unified exact-stage request；focused退役入口为`HypothesisRegistryAudit.test.tsx`与`ToolCallDetailView.candidate.test.tsx`。

## Unified Investigation exact client（2026-08-08）

- Plan B legacy Registry audit与unified exact-stage adapter是两条authority链：legacy组件不得把operation-only selector灌入新wrapper；unified direct route不得回退legacy Registry/Campaign authority。focused wrapper入口为`frontend/lib/api/investigation.test.ts`，production chain由`InvestigationWorkspaceRoute.test.tsx`覆盖。
