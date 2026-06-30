# frontend / components

> **一句话职责**：React UI 组件——按功能域分 ~39 个目录（AIChatPanel / AgentChat / HomeView / Settings / GridTerminal / FindingsPanel / MethodologyPanel / Sidecar / SubAgent* / TabBar / PaneContainer / CommandPalette / Markdown …），是整个桌面 UI 的视图层。

- **类型**：前端子系统
- **路径**：`frontend/components/`（~404 ts/tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何 UI 组件（聊天面板、终端、设置、findings/methodology 面板、sub-agent 视图、面板布局、命令面板等）时
- 改组件的 loading/error/empty 三态、与 store/hooks/api 的接线时

## 职责

桌面应用的视图层，按功能域分目录。关键域：`AIChatPanel`/`AgentChat`（AI 对话）、`GridTerminal`/`LiveTerminalBlock`/`CommandBlock`（终端）、`Settings`（设置含 IntelProviders）、`FindingsPanel`/`MethodologyPanel`/`DashboardPanel`/`AuditLogPanel`（pentest UI）、`SubAgentCard`/`SubAgentTreeView`/`SubAgentDetailView`（sub-agent）、`HomeView`（首页/engagement）、`PaneContainer`/`TabBar`/`ActivityBar`（布局）、`Markdown`/`StreamingOutput`/`DiffView`（渲染）。

## 关键子目录（节选）

| 域 | 组件目录 |
|---|---|
| AI 对话 | `AIChatPanel` / `AgentChat` / `StreamingOutput` / `SubAgentCard` / `SubAgentTreeView` / `SubAgentDetailView` / `SystemHooksCard` |
| 终端 | `GridTerminal` / `LiveTerminalBlock` / `CommandBlock` / `Ansi` |
| pentest UI | `TargetPanel` / `FindingsPanel` / `MethodologyPanel` / `DashboardPanel` / `AuditLogPanel` / `QuickNotes` |
| 布局/导航 | `PaneContainer` / `TabBar` / `ActivityBar` / `HomeView` / `DetachedView` / `CommandPalette` / `QuickOpenDialog` |
| 渲染/弹窗 | `Markdown` / `MarkdownEditor` / `DiffView` / `ImageModal` / `*Popup`（FileCommand/Path/Slash/History） |
| 其它 | `Settings` / `Sidecar` / `SessionBrowser` / `FileEditorSidebar` / `NotificationWidget` / `ErrorBoundary` |

AIChatPanel 切换 conversation tab 时会激活关联 terminal 作为上下文，但必须通过 `terminalAutoFocus` suppression 保持 DOM 焦点在 chat 输入；当前 chat textarea 有焦点时是硬保护，`GridTerminal` / xterm / `UnifiedInput` / live terminal 的自动 focus 都要尊重它。绑定当前 conversation 的 terminal 或切 conversation 时必须先打 suppression 再改 active terminal；用户主动点击 terminal 时再清掉 suppression。
`AIChatPanel/hooks/useChatSend.ts` 只在纯 Chat 模式的首条消息前注入 `[System Context]` / `pentestSystemPrompt` 的全量终端与工具说明；Task/Profile lead 由后端 hidden policy 决定是否 handoff，不能把 `run_pty_cmd`、`recon_lookup_company`、`pentest_run` 等全量工具上下文伪装成用户消息塞进去，否则模型会尝试不可用工具、绕开 lead/harness 边界。
`AIChatPanel` 的 execution mode UI 以“Chat 引擎 + Task profile”建模：`task` 只是 legacy engine alias，不能作为 profile 写进 UI/store/backend；恢复、切会话、继续/重跑和 localStorage 入口都必须先归一到具体 profile id（如 `assessment` / `red_team`），否则按钮会显示普通 `Task` 且子菜单无选中项。
`AIChatPanel` 的 `ask_human` 渲染必须同时支持本地 hook state 和全局 `pendingAskHuman` store 兜底；事件可能先被 app-level AI event pipeline 路由到 conversation 绑定的 terminal session。提交/跳过可见 ask_human 时必须按 `requestId` 同步清理本地态和 store 兜底态（AI session / terminal session 两个 key 都可能有同一请求），避免确认后同一张卡从 store 副本回弹。`scope_review` / `unit_review` 是安全范围的人审边界，不能用倒计时默认确认；只有轻量 confirmation/choice/freetext/credentials prompt 可以保留自动默认动作。
`AIChatPanel` 空消息区必须区分「真实空会话」和「工作区/terminal/conversation 正在恢复」。`workspaceDataReady=false`、`pendingTerminalRestoreData` 存在、`terminalRestoreInProgress=true`，或 `activeSessionId` 尚未绑定时，右侧显示明确的恢复 loading；只有 active session 就绪后才显示“今天要做点什么呢”的空态。
滚动条视觉必须保持全 app 一致：原生 overflow、ANSI/xterm 输出和 Radix `ScrollArea` 都使用细滚动条；默认透明或低可见度，hover/focus/滚动时轻量显示。`scrollbar-none` 和 `UnifiedTimeline` 自绘滚动条是例外，不要被全局规则破坏。
`AIChatPanel` 底部聊天 textarea 不显示任何 scrollbar thumb；长消息、URL、token 按当前输入框宽度换行/断行，即使内容内部可滚动也不能露出横向或纵向滑块。
`AIChatPanel` 中 Thought 和正文连续出现时要读作同一段 assistant narrative：`ThinkingBlock` 自身不要再追加底部 margin，正文紧跟 Thought 时用轻微 compact top spacing，避免外层 segment gap 与 Thought margin 叠加造成“思考”和正文像两块孤立面板；同时保留略宽一点的整体 segment gap，让相邻对话片段有足够呼吸感。
`AIChatPanel` 是 task/profile operation 继续、重跑、恢复提示的唯一发送入口。Task 模式下输入栏工具区有 dev-only `RotateCcw` 按钮；点击后先对当前 operation 的第一个未通过阶段调用 `harness_dev_reset_stage_checkpoint(mode="restart_stage")` 清掉本阶段 agent_run / stage_run worker / graph-flow repair 状态，再通过 `useChatSend` 发出可见用户消息 `继续跑`，从而走 `sendPromptSession` / TaskOrchestrator resume 链路；资产覆盖等 detail 面板不要再放 checkpoint/reset/re-run 按钮，也不能在 coverage/detail 组件里直接裸调 AI 发送 API。

`StageRunOrgRows` 渲染 `stage_run` 的 AI worker 执行边界：详情页必须表达 `Main Agent` 只负责调度，`Recon/Prober/Enumerator Agent` 等 specialist worker 按 org 执行并可 drill-in 到子 agent 对话/工具调用；即使只有 1 个 org，也显示为 `1 worker`，不要折叠成主 agent 自己完成阶段。`stage_run_org_progress` 是最后一次 live 快照；父 `stage_run` tool 已 completed/error/interrupted/expired 时，running/queued worker 只能投影为 stopped 展示，不能继续画 spinner / Running 文案。target_intel/EAS/Enumeration 这类有 coverage matrix 的阶段，资产覆盖不要塞在父级 org row，也不要在 `SubAgentDetailView` 的运行流里 inline 展开大矩阵；默认运行流只显示一条轻量资产覆盖 summary strip（done/pending/live 数字 + 当前工具/覆盖维度），用户点击 summary 进入完整矩阵；不要再额外铺「运行流 / 资产覆盖」两个大 tab。完整矩阵视图的返回放在资产覆盖卡 header 右侧，用小号「运行流」按钮返回时间线，避免和页面左上角「返回上级 Agent」冲突。资产覆盖表内部要合并 live work：从 sub-agent 正在运行的 `pentest_run`/shell/URL 参数解析当前资产、工具名、命令和覆盖维度，并在匹配资产行显示“正在补 LIVENESS/PORT/SERVICE · tool”，同时点亮对应 technique cell；如果单个命令包含多个 URL/IP/domain，或 `pentest_run` 通过 `input_lines` / `stdin` 批量输入资产，要显示“批量 N 个资产”语义，不要把第一个目标伪装成唯一当前资产；匹配不到已登记资产行的 running work 放进覆盖表底部的“运行中但尚未匹配到资产行”。进入完整矩阵后如果已有 live work，默认显示「正在做的资产」切片，用户点「看全部」才切回完整矩阵；如果 live work 刚完成、切换到下一个工具、或事件轮询短暂丢失某个 running item，要保留上一帧 running slice 一个稳定窗口，避免空态和新任务之间闪烁。完整矩阵顶部的 summary chips、live count、运行状态条、active group/asset count 都必须使用固定高度/固定最小宽度/`tabular-nums` 槽位，不能让数字或 running badge 出现/消失时推动页面重排。有 live work 时，矩阵顶部只能显示单行紧凑运行状态条（工具、覆盖维度、批量/当前目标、涉及资产数）；「只看运行中 / 看全部」切换按钮常驻在资产覆盖 header，不能依赖首次出现 running work 才渲染；如果用户停留在「只看运行中」且 running 清空，要显示空态和「看全部」按钮，不要自动跳回完整矩阵。完整矩阵里不要重复铺大号 running/related badge，只保留行高亮与 cell spinner。完整矩阵必须无横向滚动，把 type/source 收到 asset 副标题里，只保留紧凑 technique 状态格，并显示状态图例：found=命中、checked_empty=查空、error=工具/来源错误、blocked=阻塞、pending=未查、next_wave_pending=下批、not_applicable=不适用；`pending` 不能只靠弱点号表达，行副标题要同步展示“未查 LIVE/PORT/SVC”这类状态摘要，`checked_empty` 才能显示为“查空”；`next_wave_pending` / `new_in_stage` 行要显示在表内但不计入当前 wave 的 done/total 或 pending 计数；独立覆盖视图已经占满 detail 内容区，列表用自身滚动，不显示底部拖拽高度 handle；只有旧的 inline/折叠组件模式才允许可调高度。完整矩阵的资产 group 列表超过小列表阈值时必须窗口化渲染，只挂可视窗口和 overscan 内的 group，避免快速滚动时整张表的大量 grid/border/spinner 重绘造成卡顿或黑色空白。target_intel 的 organization 覆盖只显示为单独的「组织情报」条，不进入资产列表第一行，也不计入资产分母；组织情报的六个被动情报维度必须以可见 chip 展示 `DNS` / `WHOIS` / `ASN` / `CT证书` / `子域` / `OSINT` 和各自状态，不只画无标签的小状态格；EAS/Enumeration 的真实资产优先按 `real_ip` 做 IP 聚合：IP 行展示 direct 覆盖，解析到该 IP 的 domain/url 作为子行展示；没有 direct IP target 的解析 IP 只能显示为“解析聚合”分组行，并标明仅分组、不计覆盖，不能让用户误以为这行是未查或查空；domain/httpx 这类运行态要在 IP 行显示“关联 ...”弱提示，但只能点亮子资产自己的 direct technique cell，不能把 related work 算成 IP 本体扫描；loading/error/empty 都要保留。

资产覆盖滚动性能约束：当前常见规模（300 多资产 / 500 多 group 以下）直接渲染并配合 `content-visibility` 让浏览器跳过屏幕外绘制；只有超大完整矩阵才进入虚拟化。虚拟化时 scroll 事件必须同步刷新可视窗口，内容缩短时在 layout 阶段夹住 `scrollTop`，虚拟 spacer 自身要有稳定背景，避免快速滚动或 active/all 切换时露出一帧黑色空白。用户滚动/拖动资产覆盖矩阵后进入短暂阅读冻结窗口，polling 与 live work 的新快照只能排队，不能立即替换当前可见矩阵，避免正在看某个资产时列表突然刷新。

聊天工具卡、pending approval 卡、`ToolExecutionCard` 和 `SubAgentDetailView` 折叠工具行的主文案必须使用 `frontend/lib/tools.ts::getToolActionLabel` 这类人类动作句子（如 `Waiting for background jobs` / `Probing services`），不要把 `wait_for_background_jobs` 这类内部 `snake_case` tool id 直接作为卡片标题；raw id 只适合 hover/debug/展开详情。后台工具（`status:"backgrounded"`）必须在聊天工具卡、`ToolExecutionCard`、`ToolCallDetailView`、`SubAgentDetailView` 和 `UnifiedInput` 状态行里保持同一语义：backgrounded 是 live/non-terminal，不显示成功绿勾；detail 模式会隐藏底部输入行，所以 header 要挂 `BackgroundJobsBadge` 作为会话级后台任务入口。sub-agent 后台工具要保留 `backgrounded` 状态并按 `job_id` 接收 `tool_background_completed` 回填，避免完成前突然切成最终 Output 样式。
工具结果里的 `ai_assist` / `ai_analysis` 不是普通噪声字段，但它们也不代表工具内部真实调用了 LLM：聊天工具卡、`ToolCallDetailView` 和 `SubAgentDetailView` 都要把它们提升为独立的 Collector Hints / Static Analysis Hints 摘要块，展示 recommended/reasons/next step、采样信号、候选文件和 line hints；原始 JSON 可以继续保留在下方，方便调试。真正的工具运行动态应来自 `tool_output_chunk` 的实时 Output 区，而不是等工具结束后才展示这些 summary 字段。
工具调用 request-id 锚点、调试编号、以及 sub-agent / 调用树里的工具次数汇总只作为内部导航和调试数据保留，不在 ChatPanel、工具详情页、sub-agent inline/detail 卡或左侧调用树里渲染成可见徽标或计数；用户仍可展开查看具体工具调用行。
`BackgroundJobsBadge` 不能只依赖已到达的 job registry；detail 已经能从当前工具或 sub-agent 工具列表看出 backgrounded 数量时，要用 fallback count 显示 `N running`，避免详情页没有任何后台入口。`ToolCallDetailView` / `SubAgentDetailView` header 使用该 badge 时必须保留稳定槽位，后台任务出现/消失只能改变内容可见性，不能让 header 宽度和页面布局抖动。

`ToolCallDetailView` 和 `SubAgentDetailView` 对 shell-like 工具（`run_pty_cmd` / `run_command` / `pentest_run`，以及带 `args.tool_name` + `background/timeout_secs` 的后台工具包装参数）必须在 running/backgrounded 时固定显示 Output 区；没有 stdout/stderr chunk 时显示 pending 状态，一旦 `tool_output_chunk` 到达就用同一区域追加，避免 detail 只显示 Input、让用户误以为工具没有运行。completed/error 且 stdout/stderr 为空时也要显示 `No output.`，不要把 Output 区整个隐藏。注意：sub-agent 里的 `pentest_run` 不是 `run_pty_cmd`，但仍应按 shell-like 输出渲染，同时保留它自己的工具名和 Input args。
非 shell-like 但会发 `tool_output_chunk` 的后端直连工具（例如 `browser_collect_js_api` / `js_extract_apis`）在 running/backgrounded 时也必须显示 Output 区：有 chunk 就实时追加，没有 chunk 就显示 `Waiting for output...`，不能只展示静态 Input 和标题 spinner；工具完成后再切回结构化 result / `ToolAiTraceSummary` 展示。
 detail/live thinking/output 的自动跟随滚动必须用 rAF 合并，并且只在用户贴近底部、且没有向上滚动意图时跟随；用户在 detail 外层或 `ThinkingBlock` 内部往上滚时必须暂停自动贴底，直到用户手动滚回底部再恢复。running/backgrounded 的长 Output 只渲染尾部窗口，完整数据保留在 store/result，避免每个 chunk 重新 parse 全量 ANSI 文本。
sub-agent 的 `sub_agent_text_delta.accumulated` / `sub_agent_reasoning.accumulated` 是当前 LLM response 的全量帧，不是孤立增量；store 必须按“上一条 tool_call 之后的当前 response”回填同一个 text/thinking entry，detail 渲染还要兼容清理旧的短前缀残片，避免 `n` / `Let me run` 这类流式前缀被冻成独立正文。provider 退化出的文本工具调用标记（包括 `<tool_call>` / `<invoke>` / `DSML` 伪标签）属于内部工具通道，不属于 agent prose，`SubAgentDetailView` 渲染前必须剥掉，不能让 `submit_stage_deliverable` 参数或 coverage JSON 混进正文。`SubAgentDetailView` 视觉分组里 Thought 和正文属于同一组 agent narrative；Thought 是弱辅助元信息，正文不再显示 `Agent Output` 标题，紧跟 Thought 的正文要压缩顶部间距，不在二者之间画 full-width 分隔线或连续左侧 rail；紧跟 narrative 的 tool call 是该段叙述的 action，用轻量连接线和低背景工具行表达归属；tool call 后再出现新的 Thought/正文才开始下一组。
`SubAgentDetailView` 里由 `StageRefiner` / submit-repair 恢复注入的 `STAGE REFINER DIRECTIVE` 不属于普通 agent prose：要解析成紧凑的 `Stage Refiner` 修复卡，默认只显示 stage、repair kind、gap/action 数、batch-first 与 allowed/blocked tools 摘要，原始 directive 只能在 Details 里展开，避免系统纠错 prompt 淹没运行流。
detail header、运行中 footer、后台任务 badge 都属于 live 状态提示，必须保持高对比；`BackgroundJobsBadge` 的 popover elapsed 时间要在 jobs 存在时自行按秒刷新，不能只依赖外部 store 变更触发重渲染。
`TaskGroupShell` 展开/收起承载大量 live tool/sub-agent 行时，不要用 `grid-template-rows` 或高度动画；这类动画会在工具流更新时逐帧重排，优先即时展开/收起并只保留颜色/状态的轻量过渡。

detail 里的状态图标不能只信 transport/completed 状态；`whatweb` 这类工具可能 `exit_code=0` 但 stdout/stderr 表达依赖缺失或 fatal error，主工具 detail、sub-agent detail、聊天摘要和 tool execution card 都要复用 `toolResultIndicatesFailure` 后再画绿色勾。
`SubAgentDetailView` header 也不能只信原始 `subAgent.status`：如果 completed agent 仍有 running/backgrounded 工具，header 要显示运行态/后台态；如果 completed agent 的最后一个工具调用失败（典型是 `submit_stage_deliverable` needs_fix/error 后无成功提交），header 要显示错误，避免“业务卡住但顶部已完成”的误导。

TargetPanel 左侧树默认只做组织导航：子组织和公司计数保留，但 IP/URL/域名资产不在左树展开；左树主数字只表示该组织自己的目标数，含子公司汇总只能作为弱化 `Σ` 口径展示，不能再和本级计数同权重混用；右侧资产页默认展示本公司资产，父公司有子公司资产时才提供“本公司 / 含子公司”切换。右侧 Targets 面板按 IP 联合展示资产，点击 IP、域名或 URL 进入 target workbench。不要把大量 IP 重新铺成 org 的第一层 children，否则母子公司层级会被资产列表淹没。
资产覆盖 compact/header summary 必须使用后端 `StageAssetCoverageSnapshot.summary`，不要从 rows 重新计算分母；EAS 这种 wave-aware stage 还要把父 `stage_run` 工具的 startedAt 传给 `ai_get_stage_asset_coverage(stageStartedAt)`，让后端能把运行中新发现资产标成 `next_wave_pending` 而不是混进当前 wave 的 done/total。
Target detail 展开区必须显式展示 active landing 写回的 top-level recon fields（`real_ip` / `http_status` / `http_title` / `webserver` / `cdn_waf` / `os_info` / `content_type`），即使 `ports[]` 还没有对应 entry；per-port metadata 和 fingerprints 继续在 Services / Fingerprints 区展示。
`TargetSurfaceWorkbench` 的 IP/host Surface 相关域名列表必须可 drill-in：无论当前 subject 是 synthetic host 还是真实 IP target，都要传 `onSelectDomain`，点击域名行应切换到对应 domain target。IP/host 详情的数据读取要合并当前 IP target 与 related domain targets 的 `api_endpoints` / `js_analysis_results` / `directory_entries` 等 surface 数据，否则 JS/API 已落在域名 target 上时会在 IP 聚合详情里显示为空；后端没带 `ports` 的 target 也必须在进入详情前兜底成空数组，避免点击目标时报错退回主界面。Target surface header 不放 `Run baseline recon` / `Collect JS` / `Match vulns` 这类手动扫描按钮；采集和匹配由 AI/harness 工具流发起，前端只保留本地 surface data refresh。
Target Surface 的 Sitemap 是 JS/runtime API 的确定性证据视图，不是目录扫描/route probe 的路径列表：只从 `api_endpoints` 中 `source='crawler'` / `source='js_analysis'` / JS 类来源构建，按 origin/path segment 建树，点击 endpoint 后展示 method/path/status/content-type/params/headers/capture_path，并从 `js_analysis_results.endpoints_found` 反查对应 JS source_file/line/confidence。`directory_entries` / `target_assets` / `route_probe_paths` 的乱扫或候选路径不要混入 Sitemap。`api_endpoints.params` 渲染为参数 chips，不在前端从 URL 临时重新解析；`capture_path` 存在时作为 HTTP request/response 包入口展示，不存在时明确显示未落包。独立 `JS / API` tab 暂时不在 Target Surface 顶部暴露，避免和 Sitemap 表达同一件事。
`useTargetSurfaceData` 监听 `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` / `pentest_run` 等工具结果后自动 reload，让落库后的 API、params、paths、JS 文件不需要手动刷新才出现。Topology surface 摘要只拉 `api_endpoints` / `js_analysis_results` / `directory_entries` 的轻量计数，目标节点显示 API/params，surface 节点/Inspector 显示 API/params/paths/JS。
Target 左侧 org tree 的 chevron 只表示“有可展开内容”：有子公司，或在资产树模式下有资产组；没有下一层的 org leaf 不显示 chevron。展开/收起只能点 chevron，点击公司行主体只选中并展示右侧详情；双击公司行主体也可展开/收起。root-level 主公司折叠采用 accordion 口径：展开一个主公司时收起其它主公司。

## 依赖

- `react`、Tailwind 4；消费 `store`（状态）、`hooks`（行为）、`lib/api`（后端）、`lib/generated`（类型）

## 注意事项 / 坑

- **不变量（AGENTS.md §2.3）**：组件**禁裸 `invoke()`**，走 `lib/api/<domain>`；三态 UI（loading/error/empty）每条异步路径都要画。
- 跨 IPC 类型 import 自 `lib/generated/`（ts-rs），别手写。
- 组件多，改前先定位功能域目录；大组件（AIChatPanel/HomeView/GridTerminal）已内部拆分，遵循其既有拆分。

## 测试入口

```bash
just check-fe   # biome + typecheck
just test-fe    # vitest（含组件快照/交互测试）
```
