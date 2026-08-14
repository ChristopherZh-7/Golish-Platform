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
| Candidate review / verification | `Engagement/AttackCandidateReview`（DB-backed exact-plan approve/reject/resume）+ `Engagement/CandidateAttemptRows`（DB-backed Attempt/evidence-role/terminal read model）+ `Engagement/HypothesisRegistryAudit`（session/project-authorized只读 Registry audit） |
| 布局/导航 | `PaneContainer` / `TabBar` / `ActivityBar` / `HomeView` / `DetachedView` / `CommandPalette` / `QuickOpenDialog` |
| 渲染/弹窗 | `Markdown` / `MarkdownEditor` / `DiffView` / `ImageModal` / `*Popup`（FileCommand/Path/Slash/History） |
| 其它 | `Settings` / `Sidecar` / `SessionBrowser` / `FileEditorSidebar` / `NotificationWidget` / `ErrorBoundary` |

AIChatPanel 切换 conversation tab 时会激活关联 terminal 作为上下文，但必须通过 `terminalAutoFocus` suppression 保持 DOM 焦点在 chat 输入；当前 chat textarea 有焦点时是硬保护，`GridTerminal` / xterm / `UnifiedInput` / live terminal 的自动 focus 都要尊重它。绑定当前 conversation 的 terminal 或切 conversation 时必须先打 suppression 再改 active terminal；用户主动点击 terminal 时再清掉 suppression。
`AIChatPanel/hooks/useChatSend.ts` 只在纯 Chat 模式的首条消息前注入 `[System Context]` / `pentestSystemPrompt` 的全量终端与工具说明；Task/Profile lead 由后端 hidden policy 决定是否 handoff，不能把 `run_pty_cmd`、`recon_lookup_company`、`pentest_run` 等全量工具上下文伪装成用户消息塞进去，否则模型会尝试不可用工具、绕开 lead/harness 边界。
`AIChatPanel` 的 execution mode UI 以“Chat 引擎 + Task profile”建模：`task` 只是 legacy engine alias，不能作为 profile 写进 UI/store/backend；恢复、切会话、继续/重跑和 localStorage 入口都必须先归一到具体 profile id（如 `assessment` / `red_team`），否则按钮会显示普通 `Task` 且子菜单无选中项。
`AIChatPanel/providerConfig.ts` 只负责把用户显式 provider/model/workspace/key 与可见 endpoint 输入组装成 typed IPC config；settings-backed 隐藏字段由后端 shared provider bootstrap 统一补齐。Vertex Gemini 未配置 location 时使用与 CLI/shared factory 相同的 `us-central1`，不能退回 Vertex Anthropic 的 `us-east5` 默认区。
`AIChatPanel` 的 `ask_human` 渲染必须同时支持本地 hook state 和全局 `pendingAskHuman` store 兜底；事件可能先被 app-level AI event pipeline 路由到 conversation 绑定的 terminal session。提交/跳过可见 ask_human 时必须按 `requestId` 同步清理本地态和 store 兜底态（AI session / terminal session 两个 key 都可能有同一请求），避免确认后同一张卡从 store 副本回弹；右下角停止生成也必须同步清理这三处投影，不能留下已经取消的等待卡。子公司范围按钮把 `root_only` / `include_51` / `include_100` 协议值显示为中文，但点击仍原样提交协议值。Ask 模式的 `ask_human` 一律等待显式人工动作；只有 backend 已接受的 Run Everything (`run-all`) 模式才可对**原始 typed** 的普通 `confirmation` 或含选项 `choice` 启用倒计时。`scope_review` / `unit_review`、结构化或 legacy subsidiary-scope choice 与 phase-boundary confirmation 永远不能自动确认。`credentials`、`freetext`、未知类型和无选项 `choice` 也必须等待显式提交或跳过，不能因 presentation coercion 合成空凭据、空文本、第一选项或默认 Skip。TargetPanel 删除 asset target candidate tab 不改变这条边界：`unit_review` 仍通过兼容 candidate API 读取 `OrganizationCandidates.organizations` 并在聊天中审子公司，不能连同 target candidate UI 一起删除。
TargetIntel→EAS 的目标授权等待态使用 `task_progress.status="waiting_target_scope"`，StageMarker 显示 `Review scan targets` 与 amber pause 样式，不能再混成 `Waiting for approval`。实际授权仍由 `AskHumanInline` 的 `scope_review` 表完成：用户可以删除不授权的行，后端会拒绝新增或编辑行；该表永不自动确认。
`TargetPanel/hooks/useTargetData.ts` 是 Target 列表 mutation 后的本地收敛边界：单条/批量删除在后端确认后立即从 React state 移除成功 ID，再重读 DB；所有并发 `target_list` 使用单调请求序号，旧轮询或旧事件响应不得覆盖较新的删除结果。`TargetGroupedView` 的单条、分组和组织删除必须先经过受控的应用内 `Dialog`，不能调用 WebView/Tauri 的全局 `confirm()`；组织删除命令只表示 durable two-phase job 已接受，前端必须轮询组织 read model 直到 root row 确实消失，再刷新 Target rows 并关闭弹窗。`targets-changed` 继续用于其它已挂载 surface 的刷新提示，但不能作为删除后列表收敛的唯一机制。
`ScopeReviewTable` 未编辑确认时必须原样回传 backend snapshot 的 canonical
value、`target_type`、`scope`，不能把 IP/CIDR/URL/wildcard 默认改成 domain/in。
用户编辑的行只是 proposal；backend gate 会与 trusted UI/CLI pre-seed snapshot 精确比较并
fail closed，前端不得把 review 本身当成 target mutation API。
`unit_review` 不复用 target textarea：它使用 `UnitReviewDecisionRow` checkbox rows，
允许编辑 name/aliases/domains，但 `reviewRowId` / `candidateId` / `organizationId` 一经
seed 就不可随文本重算；手工行只生成一次 `crypto.randomUUID()` review-row id。
`AskHumanInline` 提交 `{rows: [...]}` 的 `UnitReviewSubmission`，不能退化成 names-only 数组。
`AIChatPanel` 空消息区必须区分「真实空会话」和「工作区/terminal/conversation 正在恢复」。`workspaceDataReady=false`、`pendingTerminalRestoreData` 存在、`terminalRestoreInProgress=true`，或 `activeSessionId` 尚未绑定时，右侧显示明确的恢复 loading；只有 active session 就绪后才显示“今天要做点什么呢”的空态。
滚动条视觉必须保持全 app 一致：原生 overflow、ANSI/xterm 输出和 Radix `ScrollArea` 都使用细滚动条；默认透明或低可见度，hover/focus/滚动时轻量显示。`scrollbar-none` 和 `UnifiedTimeline` 自绘滚动条是例外，不要被全局规则破坏。
`AIChatPanel` 底部聊天 textarea 不显示任何 scrollbar thumb；长消息、URL、token 按当前输入框宽度换行/断行，即使内容内部可滚动也不能露出横向或纵向滑块。
`AIChatPanel` 中 Thought 和正文连续出现时要读作同一段 assistant narrative：`ThinkingBlock` 自身不要再追加底部 margin，正文紧跟 Thought 时用轻微 compact top spacing，避免外层 segment gap 与 Thought margin 叠加造成“思考”和正文像两块孤立面板；同时保留略宽一点的整体 segment gap，让相邻对话片段有足够呼吸感。
Sub-agent reasoning 必须在 tool request/result、completed 和 error 边界前同步 flush，保证 Thought/工具/正文按事件顺序落入同一时间线。reasoning batch 的首末到达时间由 streaming buffer 传入 store；零宽 batch 只显示 `Thought`，真实小于 100ms 的片段显示 `<0.1s`，不得伪造 `0.001s` 精度。
`AIChatPanel` 是 task/profile operation 继续、重跑、恢复提示的唯一发送入口。Task 模式下输入栏工具区有 dev-only `StageResetMenu`；原地 full reset只显示 Target Intel / EAS / Enumeration / Vuln Triage，UI候选必须是当前或已通过stage，backend再强制它是current的真实DAG ancestor。null/unknown current、从未通过的分支、finished task与 Scoping/Candidate/Verification/Post-Exploit/Cleanup/Reporting全部禁用并提示创建新 operation/stage fork；不能用线性 roadmap index推断可跳转阶段。资产覆盖等 detail 面板不要再放 checkpoint/reset/re-run 按钮，也不能在 coverage/detail组件里直接裸调 AI发送 API。

点击 full reset 后，组件调用 `harness_dev_reset_stage_checkpoint(mode="restart_from_stage_purge")` 并把 backend return视为 commit边界：共享 helper先按内存 roadmap算 suffix，conversation persistence再按自身snapshot stageOrder补齐durable-only descendants并返回union；只对一个真实存在且与事件路由优先级一致的canonical roadmap session owner应用同一union，绝不创建AI/terminal alias session，随后立即写selected stage v0 `in_progress` seed、验证完整typed receipt并发送可见用户消息`继续跑`。malformed post-commit receipt或auto-resume失败必须显示“阶段已重置，但自动继续失败”，保留已回卷roadmap并把`继续跑`放回输入框，不能误报数据库未提交。

reset进行时，textarea、普通 send、execution mode/model selector、附件、reset菜单以及conversation select/new/close/history共享同一个同步互斥ref；只有当前reset owner的auto-resume可以显式穿过该gate，继续前还要复核原conversation仍active。React state只用于渲染busy，不能作为防同tick双击、跨会话profile或普通消息竞态的唯一锁。

`StageRunOrgRows` 是 `stage_run` 到 Company Controller read model 的路由边界：Target Intel / EAS / Enumeration / Vuln 的所有 rows 带一致 exact `operation_id + stage_execution_id` 时只渲染 DB-backed `StageTeamRunView`，不得在其上方复制事件快照卡或恢复 `Main Agent -> Specialist` 卡。缺一个 pointer 或 mixed pointer 的整个 company-stage snapshot 必须 fail closed 为 rerun-required，不能逐行混合两个 scheduler。每个 organization 对应一个持续运行的 Company Controller；缺少 `leader:primary` 的旧固定 Team 数据不再恢复旧执行流，只显示需要重新运行本阶段的终态提示。Candidate/Verification 与后续阶段不进入该组件，继续使用自己的 typed view。

`AttackCandidateStageRunRows` 是 Candidate `stage_run` 的实时详情入口，和 company-stage 的 `StageRunOrgRows` 保持分离。`ToolCallDetailView` 必须优先用 selected request 的 explicit args/result 判定 stage；当生产调用只有 `{orgs: []}` 时，才允许用同 request 的 `stage_run_org_progress` rows 判定 `attack_candidate`。运行、阻塞与完成 rows 都要显示 Attack Analyst、organization、exact activity/blocker、evidence 数和运行流入口；不能因为 `candidate_review_required` 尚未到达或最终聚合失败而退回空白 generic detail。若 args/result 显式 stage 冲突则 fail closed，progress 不得覆盖冲突。Candidate Review 仍按 durable operation/wave API 读取，实时 Analyst 卡不取得审批 authority。

`StageRunOrgRows` 渲染 `stage_run` 的 organization→Controller 边界：外层 Main Agent 只发起 Stage，每个 organization 的 Company Controller 持续监控该 Unit、按缺口动态调用 SubAgent，并在同一运行流中等待、检查结果和继续补派。详情页即使只有一个 organization 也必须先显示该公司的 Controller 卡；用户点入后看到 Controller 思考/工具/SubAgent 树，子 Agent 再按 exact parent tool-call identity drill-in。`stage_run_org_progress` 只是刷新提示，不能把一次事件静默、一次 SubAgent 返回或短暂无 active tool 解释为暂停/完成；只有父 `stage_run` 已确定 terminal 时才停止 live 投影。若历史 transcript 缺少 terminal progress，但同 request 的 server-authored `stage_run` tool result 已明确 `success=true, passed=true`，AI event handler必须把该 request-scoped snapshot 中残留的 queued/running/blocked/pending rows收敛为 `passed` 并清 activity，作为 replay-safe显示 fallback；它不替代 DB/Gate truth，也不能从普通 prose 或不成功 result 推导完成。target_intel/EAS/Enumeration 这类有 coverage matrix 的阶段，资产覆盖不要塞在父级 org row，也不要在 `SubAgentDetailView` 的运行流里 inline 展开大矩阵；默认运行流只显示一条轻量资产覆盖 summary strip（done/pending/live 数字 + 当前工具/覆盖维度），用户点击 summary 进入完整矩阵；不要再额外铺「运行流 / 资产覆盖」两个大 tab。完整矩阵视图的返回放在资产覆盖卡 header 右侧，用小号「运行流」按钮返回时间线，避免和页面左上角「返回上级 Agent」冲突。资产覆盖表内部要合并 live work：从 sub-agent 正在运行的 `pentest_run`/shell/URL 参数解析当前资产、工具名、命令和覆盖维度，并在匹配资产行显示“正在补 LIVENESS/PORT/SERVICE · tool”，同时点亮对应 technique cell；如果单个命令包含多个 URL/IP/domain，或 `pentest_run` 通过 `input_lines` / `stdin` 批量输入资产，要显示“批量 N 个资产”语义，不要把第一个目标伪装成唯一当前资产；匹配不到已登记资产行的 running work 放进覆盖表底部的“运行中但尚未匹配到资产行”。进入完整矩阵后如果已有 live work，默认显示「正在做的资产」切片，用户点「看全部」才切回完整矩阵；如果 live work 刚完成、切换到下一个工具、或事件轮询短暂丢失某个 running item，要保留上一帧 running slice 一个稳定窗口，避免空态和新任务之间闪烁。完整矩阵顶部的 summary chips、live count、运行状态条、active group/asset count 都必须使用固定高度/固定最小宽度/`tabular-nums` 槽位，不能让数字或 running badge 出现/消失时推动页面重排。有 live work 时，矩阵顶部只能显示单行紧凑运行状态条（工具、覆盖维度、批量/当前目标、涉及资产数）；「只看运行中 / 看全部」切换按钮常驻在资产覆盖 header，不能依赖首次出现 running work 才渲染；如果用户停留在「只看运行中」且 running 清空，要显示空态和「看全部」按钮，不要自动跳回完整矩阵。完整矩阵里不要重复铺大号 running/related badge，只保留行高亮与 cell spinner。完整矩阵必须无横向滚动，把 type/source 收到 asset 副标题里，只保留紧凑 technique 状态格，并显示状态图例：found=命中、checked_empty=查空、error=工具/来源错误、blocked=阻塞、pending=未查、next_wave_pending=下批、not_applicable=不适用；`pending` 不能只靠弱点号表达，行副标题要同步展示“未查 LIVE/PORT/SVC”这类状态摘要，`checked_empty` 才能显示为“查空”；`next_wave_pending` / `new_in_stage` 行要显示在表内但不计入当前 wave 的 done/total 或 pending 计数；独立覆盖视图已经占满 detail 内容区，列表用自身滚动，不显示底部拖拽高度 handle；只有旧的 inline/折叠组件模式才允许可调高度。完整矩阵的资产 group 列表超过小列表阈值时必须窗口化渲染，只挂可视窗口和 overscan 内的 group，避免快速滚动时整张表的大量 grid/border/spinner 重绘造成卡顿或黑色空白。target_intel 的 organization 覆盖只显示为单独的「组织情报」条，不进入资产列表第一行，也不计入资产分母；组织情报的六个被动情报维度必须以可见 chip 展示 `DNS` / `WHOIS` / `ASN` / `CT证书` / `子域` / `OSINT` 和各自状态，不只画无标签的小状态格；EAS/Enumeration 的真实资产优先按 `real_ip` 做 IP 聚合：IP 行展示 direct 覆盖，解析到该 IP 的 domain/url 作为子行展示；没有 direct IP target 的解析 IP 只能显示为“解析聚合”分组行，并标明仅分组、不计覆盖，不能让用户误以为这行是未查或查空；domain/httpx 这类运行态要在 IP 行显示“关联 ...”弱提示，但只能点亮子资产自己的 direct technique cell，不能把 related work 算成 IP 本体扫描；loading/error/empty 都要保留。

`StageTeamRunView` 只在 `StageRunOrgRows` 收到一致的 exact `operation_id + stage_execution_id` refresh pointers 时挂载；pointer/event 不是 truth，组件必须经 `api/stage-team.ts` 持续重读 DB。默认态每个 organization 只显示 `leader:primary` Company Controller、其动态 SubAgent 的运行/完成/阻塞计数，以及最终 Gate 真值；不得在外层平铺固定工位或另一个汇总 Agent。Controller queued/waiting_dependency 仍表示调度中的持续运行过程，不能 fallback 到其他 Agent identity，也不能画成暂停；只有 Unit terminal 或父 `stage_run` terminal 才停止 live monitoring。缺少 `leader:primary` 的 read model 是不支持的旧运行，必须提示重新运行，绝不提供旧 Worker 流入口。
Plan 详情只展示 `maxWorkersActive` 对应的 live K（`N active workers max`）；不得把兼容 DTO 中的 `maxWorkersTotal` 渲染成生命周期额度，因为 Company Controller 的合法 child 总量由 scope/worklist/epoch 决定而不受该旧字段限制。

Controller 运行流只绑定 exact `::lead:<worker_run_id>`；动态子 Agent 使用 exact `::worker:<worker_run_id>` 并作为 Controller tool-call 的子节点出现。StageTeam read model 中为持久化兼容保留的 `aggregatorKind`、`aggregatorRole`、`isAggregator` 等 schema 字段不属于产品展示语义，不得生成卡片、标签或 fallback identity。Unit→Gate→Plan→Barrier/Request→WorkItem→Worker→Output 的安全调试字段放入显式“调度详情”折叠区；Plan hash、epoch、lease、schema、chain、request 等不得默认铺满页面。loading/error/empty 保留。应用/进程在 active tool 期间被关闭后，任何 `manual_required + activeToolCallId` Worker 都必须脱离调度详情显示在独立、始终可达的“中断恢复”卡中；每项动作只能走 exact owner/CAS command 记录 `blocked outcome unknown`，重试复用稳定 request id，绝不自动重放工具。全部中断项清零后才引导用户另发一次“继续”或使用右下角“重置阶段”；`stage_run` 摘要存在阻塞时必须提示点击卡片进入处理，不能让用户只看到不可操作的阻塞数字。

`SubAgentDetailView` 必须从 exact `::lead:<worker_run_id>` request identity 显示 `Company Controller`，不能沿用承载该运行流的通用 executor `agentName`。Controller 的 `stage_team_dispatch_workers` 工具行必须按 `args.workers + result.requests` 立即投影全部派发 assignment，而不是等 active child 出现后才画一张聚合卡：一次接受 N 个请求就稳定显示 N 张带序号、role 和 objective 的卡；已启动 child 以 durable `created_work_item_id` 匹配 exact `${tool_request_id}::worker:<worker_run_id>` identity 并可点击钻取，尚未 claim/启动的 assignment 显示不可点击“排队中”，拒绝项显示错误。一个 durable WorkItem 因 output-contract/provider failure 产生多代 WorkerRun 时，它们必须按 `created_work_item_id` 合并在同一 assignment：主卡只指向最新一代，下面显示先前失败原因与当前自动重试代次；不同 WorkItem 的真实 sibling 仍各自保留独立卡和 transcript，不能按 agent 名称或时间误合并。历史单 child exact `${tool_request_id}` 只作为兼容入口。点击把 child request identity 压入 detail stack，返回则回到同一 Controller 运行流。Controller 自身事件已 completed、但已派 child 仍 running 时，header 继续显示运行中，避免把后台扫描误报为 Controller 已结束。该 child 入口使用运行流树状连接线、独立 Agent identity 槽、任务摘要、紧凑状态 badge 和明确的进入箭头；运行中的 thinking 放在卡片底部 live strip，整体沿用 Golish 深色低饱和状态色，不复制普通 tool-call 折叠卡外观。
Sub-agent 的所有详情入口必须把 restored `interrupted` tool 显示为黄色终态且不再旋转；`SubAgentDetailView`、inline card 和 legacy `SubAgentDetailsModal` 不能各自把同一状态解释成 running。`backgrounded` 仍保留现有 detached-job 运行展示，等待最终 job result 收敛。

`SubAgentDetailView` 对 Company Controller 时间线中的 `update_plan` 使用 Codex 式“当前计划”卡：只接受 1–12 个完整 `{step,status}`，status 为 `pending|in_progress|completed` 且最多一个 `in_progress`；可见运行流只保留最新一份有效的 live/completed snapshot，旧版本、失败调用和非法参数不渲染成普通工具卡。原始调用仍完整保留在 transcript、`run.log` 和 `run_tree.py` 诊断输出中。计划卡展示 explanation、完成计数和步骤状态；它只是 Controller chain 内的工作计划，不得改变或推导 Unit/Gate truth；普通 child不拥有该工具。详情页必须解析 exact `::team::<organization_id>::lead:<worker_run_id>` identity 并关联同一 `stage_run` 的 organization row；若且仅若该 row 的 server-authored status 已是 `passed`，最新计划 snapshot 可做 display-only 终态收敛，把残留 pending/in_progress 显示为 completed 并标记“计划已完成”。该投影不回写 transcript/tool event，也不能由父工具 completed、普通 prose、blocked/stopped/error 推导；非 passed 终态必须保留真实未完成步骤。

`AttackCandidateReview` mount 后必须无条件按 `operation_id + wave_run_id` 读取 durable review API；`CandidateReviewRequired/Resumed` trace 只能触发 refresh。UI 展示 frozen target（live row 删除时仍可审）、exact plan hash/actions/budget/expiry；resume dispatcher 失败时保留已写 decisions 与 retry 按钮，不能退回未审批外观。
`CandidateAttemptRows` 同样只把 trace 当 refresh hint，并按 exact operation/wave 从 `attack_list_candidate_attempts` 重读 DB truth。它显示 frozen target、Candidate/Attempt identity、ordinal、persisted status 与 result 中的 proof/refutation/blocker evidence roles；只有 authoritative status=`verified` 才显示 Finding lineage 摘要，blocked 只显示 blocker/residual，绝不能把 Candidate hypothesis 当成 Finding，也不能在 IPC 未返回 id 时伪造 Finding id。其内嵌 `CandidateVerificationProtocol` 独立读取 `attack_list_verification_queue`，展示 queue/Worker/action authorization/terminal intent/barrier/receipt/recovery/Wave-unit 状态及 exact request-id/row-version/evidence-id；operator 只看到三种 closed recovery action。mutation 保留稳定 request id 以支持 response-loss replay，并明确显示“decision recorded; pending server convergence”，不能把已记录决定冒充 terminal。读模型还要单独展示 immutable pending FactDelta enrichment 的安全 subject、reason、allowed techniques 与状态，并明确 source Wave 保持 open、尚未创建 Candidate WorkItem；该区域只读，不得提供虚假的“自动补全”按钮或泄漏 raw evidence/request。两个异步 read path 都保留 loading/error/empty。`ToolCallDetailView` 在既有 `attack_candidate` stage-run 挂载点并排渲染 review 与 Attempt read model，二者必须接收同一组 exact operation/wave/refresh hint。
`candidate_attempt_terminalized` / `attack_wave_consolidated` 没有新增 UI authority 或模型可操作按钮：事件层只在 session 已存在、且 operation + source Wave 与当前 `candidateReviewHint` 精确匹配时提升 refresh token，并保留原 `resumeVersion`。组件随后仍按 exact operation/wave 重读既有 Candidate review/Attempt DB API；trace 的 status/decision/count 或可选 Finding id 不能直接物化 row、切换审批状态或推导下一 Wave。
Target Intel 的 coverage read model 必须按 `stage_started_at` 冻结 per-asset axis：本阶段运行中直接落库的 `source=asset_intel` Targets 可作为 handoff/`new_in_stage` 信息展示，但不能进入当前矩阵的 pending、done/total 或触发 denominator reflow。WHOIS 对这些新 domain 的非递归读取只更新单独的组织情报行，不产生新资产行；provider query root 仍排除它们。

Enumeration 资产覆盖矩阵必须把内容枚举拆成四个可见列：`JS` / `DIR` / `PARAM` / `API`。`JS` 表示 JS 文件收集，`API` 表示 JSAPI/API endpoint 抽取；前端 technique key 解析要先识别 `JSAPI` 再识别 `JS`，避免 `GOLISH-ENUM-JSAPI` 被错误归到 JS 列。running work 的维度文案也要跟随这四列，例如 `正在补 JS · browser_collect_js_api` 和 `正在补 API · js_extract_apis` 不能混写。

Enumeration 同一 `target_id` 可以展开为多个 exact Web Origin 行，`StageAssetCoveragePanel` 的 group/React key 必须同时包含 `target_id + value(origin)`，不能让 HTTP/HTTPS 行碰撞或丢失。`partial` 使用独立“部分完成”状态、行摘要和 summary chip，仍计入未完成资产，不能算进 done/ready。

资产覆盖滚动性能约束：当前常见规模（300 多资产 / 500 多 group 以下）直接渲染并配合 `content-visibility` 让浏览器跳过屏幕外绘制；只有超大完整矩阵才进入虚拟化。虚拟化时 scroll 事件必须同步刷新可视窗口，内容缩短时在 layout 阶段夹住 `scrollTop`，虚拟 spacer 自身要有稳定背景，避免快速滚动或 active/all 切换时露出一帧黑色空白。用户滚动/拖动资产覆盖矩阵后进入短暂阅读冻结窗口，polling 与 live work 的新快照只能排队，不能立即替换当前可见矩阵，避免正在看某个资产时列表突然刷新。

聊天工具卡、pending approval 卡、`ToolExecutionCard` 和 `SubAgentDetailView` 折叠工具行的主文案必须使用 `frontend/lib/tools.ts::getToolActionLabel` 这类人类动作句子（如 `Waiting for background jobs` / `Probing services`），不要把 `wait_for_background_jobs` 这类内部 `snake_case` tool id 直接作为卡片标题；raw id 只适合 hover/debug/展开详情。后台工具（`status:"backgrounded"`）必须在聊天工具卡、`ToolExecutionCard`、`ToolCallDetailView`、`SubAgentDetailView` 和 `UnifiedInput` 状态行里保持同一语义：backgrounded 是 live/non-terminal，不显示成功绿勾；detail 模式会隐藏底部输入行，所以 header 要挂 `BackgroundJobsBadge` 作为会话级后台任务入口。sub-agent 后台工具要保留 `backgrounded` 状态并按 `job_id` 接收 `tool_background_completed` 回填，避免完成前突然切成最终 Output 样式。
工具结果里的 `ai_assist` / `ai_analysis` 不是普通噪声字段，但它们也不代表工具内部真实调用了 LLM：聊天工具卡、`ToolCallDetailView` 和 `SubAgentDetailView` 默认只展示结论型 `Key Findings` 摘要，回答“找到了什么 / 落库了什么 / AI 是否补到端点”。`browser_collect_js_api` 聚合 runtime API/保存 JS/落库数，`route_probe_paths` 聚合 verified paths/请求数/空结果，`js_extract_apis` 聚合 API/参数/secrets/落库数。frameworks/libraries/rule_matches、AI prompt/response 等调试明细不再作为默认摘要块渲染，只保留在原始 JSON 里追证据。真正的工具运行动态应来自 `tool_output_chunk` 的实时 Output 区，而不是等工具结束后才展示这些 summary 字段。
工具调用 request-id 锚点、调试编号、以及 sub-agent / 调用树里的工具次数汇总只作为内部导航和调试数据保留，不在 ChatPanel、工具详情页、sub-agent inline/detail 卡或左侧调用树里渲染成可见徽标或计数；用户仍可展开查看具体工具调用行。
`BackgroundJobsBadge`不能只依赖已到达的job registry；detail从当前工具或sub-agent工具列表看出backgrounded数量时用fallback count显示`N running`。每个registry row是exact navigation button。`BackgroundJobPanel`显示running/stopping/terminal、job id、managed elapsed与last-output age，并明确“无自动进程截止；需要时显式停止”；initial yield只保留为tool-result诊断元数据，不作为“转后台”产品语义或倒计时。Stop请求被backend接受后只显示`Stopping…`，最终状态只能由terminal event决定。

`ToolCallDetailView` 和 `SubAgentDetailView` 对 shell-like 工具（`run_pty_cmd` / `run_command` / `pentest_run`，以及带 `args.tool_name` + `background/timeout_secs` 的后台工具包装参数）必须在 running/backgrounded 时固定显示 Output 区；没有 stdout/stderr chunk 时显示 pending 状态，一旦 `tool_output_chunk` 到达就用同一区域追加，避免 detail 只显示 Input、让用户误以为工具没有运行。completed/error 且 stdout/stderr 为空时也要显示 `No output.`，不要把 Output 区整个隐藏。注意：sub-agent 里的 `pentest_run` 不是 `run_pty_cmd`，但仍应按 shell-like 输出渲染，同时保留它自己的工具名和 Input args。
非 shell-like 但会发 `tool_output_chunk` 的后端直连工具（例如 `browser_collect_js_api` / `js_extract_apis`）在 running/backgrounded 时也必须显示 Output 区：有 chunk 就实时追加，没有 chunk 就显示 `Waiting for output...`，不能只展示静态 Input 和标题 spinner；工具完成后再切回结构化 result / `ToolAiTraceSummary` 展示。`ToolAiTraceSummary` 不渲染单独的「AI Pass」或 request/response 采样；AI 只在 `Key Findings` 里体现为 `AI +N` / recipe round / `HAE candidates N` / `HAE promoted N` 等结果信号，详细 `ai_dialogue` 留在 raw JSON。`HAE candidates` 是 JS extract 的候选池，不等于已落库 API；只有 promoted/landed 才代表进入 endpoint 集合。
 detail/live thinking/output 的自动跟随滚动必须用 rAF 合并，并且只在用户贴近底部、且没有向上滚动意图时跟随；用户在 detail 外层或 `ThinkingBlock` 内部往上滚时必须暂停自动贴底，直到用户手动滚回底部再恢复。running/backgrounded 的长 Output 只渲染尾部窗口，完整数据保留在 store/result，避免每个 chunk 重新 parse 全量 ANSI 文本。
sub-agent 的 `sub_agent_text_delta.accumulated` / `sub_agent_reasoning.accumulated` 是当前 LLM response 的全量帧，不是孤立增量；store 必须按“上一条 tool_call 之后的当前 response”回填同一个 text/thinking entry，detail 渲染还要兼容清理旧的短前缀残片，避免 `n` / `Let me run` 这类流式前缀被冻成独立正文。provider 退化出的文本工具调用标记（包括 `<tool_call>` / `<invoke>` / `DSML` 伪标签）属于内部工具通道，不属于 agent prose，`SubAgentDetailView` 渲染前必须剥掉，不能让 `submit_stage_deliverable` 参数或 coverage JSON 混进正文。`SubAgentDetailView` 视觉分组里 Thought 和正文属于同一组 agent narrative；Thought 是弱辅助元信息，正文不再显示 `Agent Output` 标题，紧跟 Thought 的正文要压缩顶部间距，不在二者之间画 full-width 分隔线或连续左侧 rail；紧跟 narrative 的 tool call 是该段叙述的 action，用轻量连接线和低背景工具行表达归属；tool call 后再出现新的 Thought/正文才开始下一组。
`SubAgentDetailView` 里由 `StageRefiner` / submit-repair 恢复注入的 `STAGE REFINER DIRECTIVE` 不属于普通 agent prose：要解析成紧凑的 `Stage Refiner` 修复卡，默认只显示 stage、repair kind、gap/action 数、batch-first 与 allowed/blocked tools 摘要，原始 directive 只能在 Details 里展开，避免系统纠错 prompt 淹没运行流。
detail header、运行中 footer、后台任务 badge 都属于 live 状态提示，必须保持高对比；`BackgroundJobsBadge` 的 popover elapsed 时间要在 jobs 存在时自行按秒刷新，不能只依赖外部 store 变更触发重渲染。
`TaskGroupShell` 展开/收起承载大量 live tool/sub-agent 行时，不要用 `grid-template-rows` 或高度动画；这类动画会在工具流更新时逐帧重排，优先即时展开/收起并只保留颜色/状态的轻量过渡。

detail 里的状态图标不能只信 transport/completed 状态；`whatweb` 这类工具可能 `exit_code=0` 但 stdout/stderr 表达依赖缺失或 fatal error，主工具 detail、sub-agent detail、聊天摘要和 tool execution card 都要复用 `toolResultIndicatesFailure` 后再画绿色勾。
`SubAgentDetailView` header 也不能只信原始 `subAgent.status`：如果 completed agent 仍有 running/backgrounded 工具，header 要显示运行态/后台态；如果 completed agent 的最后一个工具调用失败（典型是 `submit_stage_deliverable` needs_fix/error 后无成功提交），header 要显示错误，避免“业务卡住但顶部已完成”的误导。

TargetPanel 左侧树默认只做组织导航：子组织和公司计数保留，但 IP/URL/域名资产不在左树展开；左树主数字只表示该组织自己的目标数，含子公司汇总只能作为弱化 `Σ` 口径展示，不能再和本级计数同权重混用；右侧资产页默认展示本公司资产，父公司有子公司资产时才提供“本公司 / 含子公司”切换。asset-map 的 current-run normalized domain/IP 已由后端直接落成 org-bound `scope=in, source=asset_intel` Targets，因此 TargetPanel 不再显示 target candidate tab，也不再提供 approve/reject/promote 动作；结果直接进入 Targets/Activity。旧 candidate DTO/command/`intel.engagement.candidates` schema 暂时保留兼容，但不作为 TargetPanel read model；子公司候选继续留在聊天 `ask_human(unit_review)`。右侧 Targets 面板按 IP 联合展示资产；IP 行是可进入的聚合主体，域名/URL 子行只是该 IP 下的 HTTP identity/origin，不作为独立 target workbench 入口；只有没有解析 IP 的 unresolved 域名/URL 才保留直接进入详情的兜底入口。不要把大量 IP 重新铺成 org 的第一层 children，否则母子公司层级会被资产列表淹没。
右侧 host selection 必须从独立 `buildHostTree` 投影取 node，不能从仅含组织导航的 `buildOrgTree` 查找；只有 domain 的 `real_ip`、没有独立 IP Target 的 resolution-only 分组也必须可打开 synthetic IP workbench，并显示其 related domains。
资产覆盖 compact/header summary 必须使用后端 `StageAssetCoverageSnapshot.summary`，不要从 rows 重新计算分母；EAS 这种 wave-aware stage 还要把父 `stage_run` 工具的 startedAt 传给 `ai_get_stage_asset_coverage(stageStartedAt)`，让后端能把运行中新发现资产标成 `next_wave_pending` 而不是混进当前 wave 的 done/total。
Target detail 展开区必须显式展示 active landing 写回的 top-level recon fields（`real_ip` / `http_status` / `http_title` / `webserver` / `cdn_waf` / `os_info` / `content_type`），即使 `ports[]` 还没有对应 entry；per-port metadata 和 fingerprints 继续在 Services / Fingerprints 区展示。
`StageAssetCoveragePanel` 对后端运行时扩展字段 `suggested_capabilities` 只做 tooltip/可读提示（例如 `capability: Fingerprint services`），不改 generated TS 类型、不参与 done/pending 计算，也不能替代 DB/gate truth；旧 `suggested_tools` 仍显示为 fallback/hint。
`SubAgentDetailView` 的独立资产覆盖当前只支持 Target Intel/EAS/Enumeration。VulnTriage 的 operation-scoped outcome/evidence 需要可信 `operation_id + chat session` 双轴，而该 UI 尚无 operationId 状态链；因此即使兼容 `coverageAxis` 里也含 `DIR`，也必须按 stage label 排除，不能误判成 Enumeration 后调用错误 read model。
`TargetSurfaceWorkbench` 的 IP/host Surface 相关域名列表展示为只读归属关系：无论当前 subject 是 synthetic host 还是真实 IP target，域名/URL 子行都不切换到 domain target，也不展示 domain.ports；端口只属于 IP Services，IP Services 可汇总当前 IP target 与 related domain/url targets 的 ports 后去重展示。相关域名列表只能包含真实 DNS/URL host，不能把值本身是 IP literal 的 target 挂到“域名”区。IP/host 详情的数据读取要合并当前 IP target 与 related domain targets 的 `api_endpoints` / `js_analysis_results` / `directory_entries` / `fingerprints` 等 surface 数据，否则 JS/API/指纹已落在域名 target 上时会在 IP 聚合详情里显示为空。IP 顶层必须提供 `Fingerprints` tab：新 array/旧 object evidence 都归一为 observation 数组；只把含显式 canonical URL/origin 的指纹挂到每一个匹配 Web Origin，无 origin 的 legacy row 放在 `Target-level / unassigned fingerprints`，不得按 host/端口猜测；confidence 同时兼容 DB `0..1` 与旧 UI `0..100` 口径。Sitemap 作为 IP 聚合详情里的 origin/path 视图展示这些合并后的 HTTP 证据，不再通过“点域名进入另一个 workbench”来查看同一批数据。后端没带 `ports` 的 target 也必须在进入详情前兜底成空数组，避免点击目标时报错退回主界面。Target surface header 不放 `Run baseline recon` / `Collect JS` / `Match vulns` 这类手动扫描按钮；采集和匹配由 AI/harness 工具流发起，前端只保留本地 surface data refresh。
Phase 2.4 起 IP TargetSurfaceWorkbench 会尝试读取 `target_surface_hierarchy_get` 的 backend identity hierarchy，但它只提供 `NetworkEndpoint` / `WebOrigin` / `Observation` 身份层；legacy web content 仍来自现有 frontend `buildSurfaceHierarchy` 输入（`api_endpoints` / `js_analysis_results` / `directory_entries` / params / evidence）。`backendSurfaceHierarchy.ts` 用精确 `origin` 字符串（`scheme://host:port`）做 union：backend origin 提供 identity，frontend origin 提供 Sitemap/APIs/JS/Params/Evidence；backend-only origin 仍显示并提示 legacy content 尚未 linked，frontend-only origin 保留为 `frontend_inferred`。backend command error、`legacy_fallback`、`backend_unavailable` 或空 identity 都不能让 IP 页面失败，必须回退到 frontend-inferred hierarchy。domain/url target 继续走旧 `Identity / Surface / Sitemap / Sensitive / Evidence` 视图，不受 backend hierarchy 影响。
Phase 2.5B 起 WebOrigin 的**展示计数**（URL/API/JS/Params/Evidence，加上 tooltip 的 directory-entry 与 passive-log）优先用 backend `contentCounts`，其次 frontend origin 计数，最后 0；`WebOriginVM.contentCountSource`（`backend_content_counts` / `frontend_content_inferred`）标记计数来源，且**detail rows 永远来自 frontend legacy arrays**，adapter 不因为 backend count 存在就伪造 rows，也不把 backend count 和 frontend count 相加。IP Overview / summary 的 content counts 同样优先 backend summary，否则回退 frontend summary（`findingCount` 永远 frontend）。Origin detail Overview 显式列出 identity source 与 content count source；当 backend count > 已加载 frontend rows 时提示「row-level content is still loaded from the legacy frontend data sources」，当 backend origin 有 count 但完全没有 frontend rows 时提示「row-level content is not loaded in this view yet」。backend summary 的 `contentUnassignedCount` / `contentUnmatchedOriginCount` 在 IP Overview 以「Backend content aggregation found X unassigned items and Y unmatched-origin items」轻提示展示，unmatched-origin **不会**被物化成 WebOrigin。
Phase 2.5C 起 backend 每个 WebOrigin / unassigned 还会带**轻量 refs**（`WebOriginVM.contentRefs`，来自 `BackendWebOriginContentRefDto`）。当某 origin 的 frontend legacy rows 未加载（backend-only identity）时，WebOrigin detail 的 APIs / JS / Sitemap tab 用 `BackendRefList` 渲染这些轻量 refs（kind/method/status/url/source 只读列表），明确标注「From backend content index」——refs 只是指针，**绝不**被提升成完整 `ApiEndpoint` / `JsAnalysisResult` row；有 frontend rows 时仍优先展示完整 rows。frontend-inferred / fallback origin 的 `contentRefs` 恒为空。IP header 提供「Build identity from data」按钮调用 `surfaceIdentityBackfill`（`target_surface_identity_backfill`）从既有 legacy 数据 additive/idempotent 地填充 identity 三表后自动 reload，让 backend hierarchy 从 fallback 变为真实数据源。
2026-07-07 起 WebOrigin detail 不再展示独立 `Crawl` tab：`WebOriginVM.crawlObservations` / backend `crawl_observations` 仍可作为兼容数据存在，但 Katana/历史 crawler URL 只属于 seed/observation 语义，不是 Target Surface 的主成果面。主详情页继续以 Sitemap/APIs/JS/Params/Evidence 展示 browser/js_extract/route_probe 落库后的结果，避免把 seed URL 误读为最终 coverage truth。
Target Surface 的 Sitemap 是 **Burp 风格的全量 URL 站点地图**（2026-07-01 前端 hierarchy 调整）：把「API/runtime 端点」、「`.js` 文件本身」和 `directory_entries` 的 verified URL 合并进同一棵 origin/path 树，用 `SitemapItem.kind`（`endpoint` / `script` / `directory`）区分，并提供 All / URLs / Endpoints / Scripts 过滤。`buildSitemapItems(endpoints, jsResults, directoryEntries)` 同时消费 `api_endpoints`（`source='crawler'` / `source='js_analysis'` / JS 类来源，`kind='endpoint'`）、`js_analysis_results`（每个 JS 文件一个 `kind='script'` 节点，`source='js_file'`，带 `size_bytes`）和 `directory_entries`（`kind='directory'`）。IP/host workbench 不再展示一个全局混合 Sitemap，而是先经 `surfaceHierarchy.ts` 聚合为 `IP -> NetworkEndpoint -> WebOrigin`，在 Web Origin detail 内按 origin 展示 Sitemap/APIs/JS/Params/Evidence；无法解析完整 origin 的数据进入“未归属 Web 数据”。Sitemap 树根显示为显式端口 origin（例如 `https://host:443` / `http://host:80`），避免浏览器 `URL.origin` 省略默认端口后让 WebOrigin 归属看起来丢端口。点击 endpoint 显示 method/path/status/content-type/params/headers/capture_path，并从 `js_analysis_results.endpoints_found` 反查 JS source_file/line/confidence；点击 script 显示 filename/size/framework·library·endpoint·secret 计数/sourcemap/risk/`raw_analysis.ai_review`，并在 `raw_analysis.hae_route_candidate_count` / `raw_analysis.hae_route_candidates` 存在时显示 `HAE candidates N`，明确这是 HaE/Linkfinder 风格路径候选，不等于已落库 API。被显式 `max_file_bytes` cap 跳过的 bundle 仍作为 script 节点出现并标注 `skipped`（来自后端 `raw_analysis.skipped`），但默认 `js_extract_apis` 会读取所有已保存 JS。`api_endpoints.params` 渲染为参数 chips，不在前端从 URL 临时重新解析；`capture_path` 存在时作为 HTTP request/response 包入口展示，不存在时明确显示未落包。
`useTargetSurfaceData` 监听 `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` / `pentest_run` 等工具结果后自动 reload，让落库后的 API、params、paths、JS 文件不需要手动刷新才出现。它对 assets/endpoints/fingerprints/JS/passive/timeline/directory/logs 逐 source `allSettled`：一个可选 source 失败要保留其它成功数组和 backend hierarchy，并通过 `sourceErrors` + 聚合 `error` 呈现部分失败；禁止单个 `directory_entry_list` 错误把整页重置成 empty。Evidence tab 对 target-bound `eas.fingerprint_web_stack` audit 的 JSON `raw_output` 做窄解析：展示 failure class、Attempt N/3 与 producer error/blocked；attempt 3 + independent confirmation 只能说明 Enumeration 将重新验证 exact-origin eligibility，前端不能提前声称已排除。Web Origins 表的单一 WhatWeb 状态列以 audit ledger 的 typed `audit_role/evidence_technique/evidence_outcome/evidence_asset` 为权威：`found`、`checked_empty`、retry pending、producer blocked、malformed evidence 与未检查分开展示；error/blocked 还必须通过 bounded structured payload consistency 校验。不要恢复 whole-Target worst-state、Overview/detail 重复 badge，或从任意 raw prose 推断成功/下游排除。Topology surface 摘要只拉 `api_endpoints` / `js_analysis_results` / `directory_entries` 的轻量计数，目标节点显示 API/params，surface 节点/Inspector 显示 API/params/paths/JS。
Target 左侧 org tree 的 chevron 只表示“有可展开内容”：有子公司，或在资产树模式下有资产组；没有下一层的 org leaf 不显示 chevron。展开/收起只能点 chevron，点击公司行主体只选中并展示右侧详情；双击公司行主体也可展开/收起。root-level 主公司折叠采用 accordion 口径：展开一个主公司时收起其它主公司。

## 依赖

- `react`、Tailwind 4；消费 `store`（状态）、`hooks`（行为）、`lib/api`（后端）、`lib/generated`（类型）

## 注意事项 / 坑

- **不变量（AGENTS.md §2.3）**：组件**禁裸 `invoke()`**，走 `lib/api/<domain>`；三态 UI（loading/error/empty）每条异步路径都要画。
- 跨 IPC 类型 import 自 `lib/generated/`（ts-rs），别手写。
- Cleanup `stage_run` 的标准 Tool detail 会按 authoritative trace 中的 operation/org identity 挂载 `CleanupObligationList`；组件仍经 trusted IPC 重验 scope，trace 只负责定位和刷新，不能成为 waiver/Gate truth。Waiver draft 按 obligation 隔离；首次点击只冻结 exact operation/project/snapshot/org、row-version、residual/evidence 请求并展示复核，第二次确认才提交。复核后输入漂移不能改变已冻结 payload，刷新/identity/CAS 漂移会取消确认。
- `Engagement/ReportReadModelView` 显式呈现 loading/error/empty，空态由用户触发 DB-authoritative build；已有 revision 可按完整 DB source set rebuild。claim 只展示 structured value + canonical source version + evidence audit id。Final publish 必须二次确认：首次点击冻结 exact `operationId + revisionId + sourceSetHash + rowVersion` 并清晰展示待确认状态，第二次点击只能提交该 frozen payload；refresh/rebuild/operation identity 或 current revision CAS 漂移必须先取消/拒绝确认，不能用新 read model 偷换 payload。组件不能传 actor/project path/storage key，也不能自动 publish。真实可达入口在 `AIChatPanel` 消息区：Reporting gate/deliverable trace 只提供 operation + refresh pointer，组件仍通过 IPC 重读 DB truth；切换 operation 时用新 identity remount。`ToolCallDetailView` 另保留兼容入口，但只在 selected `stage_run` 的 args/result 明确为 `reporting` 且 operation identity 无冲突时挂载；双边 identity 冲突必须 fail closed。
- `Engagement/HypothesisRegistryAudit` 必须同时接收当前 `ToolCallDetailView.sessionId` 与 exact Candidate operation id；三条 read request 都透传这对 selector，由后端把 session绑定到live bridge workspace与operation project。组件缓存/loading/error/detail状态按 `sessionId + operationId` 双轴隔离，切 tab/session 时不得短暂展示上一 workspace 的 Registry 数据。组件不传 workspace path、project id或principal。
- `HypothesisRegistryAudit` 分别呈现summary/list/detail的loading/error/empty，refresh保留旧数据但显式标`stale`；显示冻结rollout mode、residual codes、at-time subject、legacy projection状态与`legacy_unavailable`字段。cursor/change sequence/temporal snapshot只用于readonly一致性，trace仅触发refresh，绝不成为Gate或projection truth。
- 组件多，改前先定位功能域目录；大组件（AIChatPanel/HomeView/GridTerminal）已内部拆分，遵循其既有拆分。

## 测试入口

```bash
just check-fe   # biome + typecheck
just test-fe    # vitest（含组件快照/交互测试）
```

## AIChatPanel context compaction visibility (2026-07-14)

- The main chat timeline keeps the latest successful context-compaction notice visible instead of removing it after five seconds.
- The completed notice is an expandable disclosure that explains the short-term context transition and shows the pre-compaction token count.
- Raw summarizer input and generated memory summaries remain hidden to avoid exposing sensitive conversation content.
- This is conversation-context visibility only; it is not a browser for Memory Fabric `ContextPack`, `Episode`, or `Assertion` records.

## AIChatPanel execution profile authority (2026-07-15)

- Task/Profile selection is committed to React state, terminal session state, localStorage and conversation persistence only after an initialized backend bridge accepts `set_execution_mode`.
- A rejected profile switch keeps the previous GUI mode and renders a conversation error; it must not leave the picker showing Pentest/Red Team while the bridge remains in Chat or another stale profile.
- Session initialization is incomplete when a restored profile is rejected, and every send revalidates the selected profile before dispatching the prompt. Profile-sync failure therefore produces zero prompt/tool execution.
- Stage-reset continuation awaits the same profile commit before sending its resume prompt.

## Stage Team dispatch rejection truth（2026-07-17）

- `SubAgentDetailView` 优先消费 `stage_team_dispatch_workers` 的逐项 `requests[].decision`。新结果中的 rejected
  assignments 显示 error，绝不显示 queued。
- 对没有逐项 decisions 的历史 transcript，只在 exact
  `code=STAGE_TEAM_DISPATCH_NONE_ACCEPTED` 时把 args-derived assignments 收敛为 error；其它未知工具失败不得从
  prose 猜测 durable decision。
- 对 terminal `stage_team_dispatch_workers` error，若不存在 accepted Request 且没有 exact nested child，args-derived assignment 必须显示 error，并展示结构化 `code: error`；不能继续显示 queued。只要已有 accepted Request 或 child，仍按 durable identity 呈现，避免把部分成功误报成全失败。

## Stage Team restored dispatch visibility（2026-07-17）

- 如果 Company Controller 本地 timeline 已缺失 `stage_team_dispatch_workers` tool，但同 session 已恢复出
  `${dispatch_request_id}::worker:<worker_run_id>` child，`SubAgentDetailView` 通过当前 `SessionStageRun` 的 exact
  operation/execution pointer 读取 DB-authoritative Stage Team read model。
- 只选择 `request.parentWorkerRunId` 等于当前 `::lead:<controller_worker_run_id>` 的 Request；
  `acceptedWorkItemId` 和 child WorkerRun id 决定卡片归属。已有 live Agent 接回可点击详情，没有 event snapshot
  的 durable child 仍按 DB status 显示 queued/running/completed/error。
- 原 dispatch tool 存在时原始 tool args/result 优先且不重复恢复；read-model identity mismatch、loading、error、
  empty 分开展示，error 可重试。禁止从 Controller prose 或“准备派 3 个”文字伪造 assignments。
- 恢复分组属于历史运行流，不是常驻 footer：用该 Controller 最早 durable Request `createdAt` 定位到第一个更晚的
  timestamped Thought/tool 之前。主 Agent 后续继续输出时，已完成 Worker 卡必须留在派工历史点并自然滚离，
  不能持续固定在时间线底部；没有可信 timestamp 的纯文本不能用 prose 猜位置。

## Vuln worklist progress and attempt lanes（2026-07-19）

- `StageTeamRunView` 对每个 Vuln Unit读取 exact operation/org coverage并聚合 terminal/total/remaining cells；例如显示 `340/360 cells 终态 · 剩余 20`。coverage loading/error/empty是独立状态，不能用 Worker卡数量代替 Gate denominator。
- Vuln 虽继续复用 DB-authoritative Stage Team read model，但产品层必须显示为“漏洞扫描调度器 / 扫描分片 / 证据门禁”，不能把 formulaic Nuclei WorkItem冒充成可点击的 LLM SubAgent。只有存在 exact Controller transcript mapping时才提供“查看 AI运行流”；没有 transcript的 host-executed分片不生成伪入口。
- Vuln 顶部提供 terminal/total的证据覆盖进度条与明确的待检查数量。历史 partial/error cell显示为“历史失败”，active automatic retry Worker显示“自动重试”，manual recovery Worker显示“待人工恢复”；三者可以同时存在，历史红色不代表当前 retry已经失败，manual recovery也不能被画成普通自动重试。
- 通用 Company Controller阶段仍可显式展开 Unit→Plan→Request→WorkItem→Worker调试字段；Vuln产品视图不显示这套原始调度详情入口，避免把 hash、lease、schema和durable request内部结构暴露成主产品体验。Vuln 的中断恢复卡是安全操作面而不是 raw scheduler detail：即使“继续”生成的新 `stage_run` 立即再次停在 `operator_recovery_required`，用户仍可从该 block 的“处理阻塞”详情逐项封存未知结果；处理第一项时其它项保持可操作，只有全部处理完才进入再次继续或重置的路径。

## Organization deletion active-stage blocker（2026-07-19）

- `TargetGroupedView` 的删除确认明确说明：没有活动执行者的 paused stage Task会被自动停止；仍在执行或结果待确认的 Task继续阻止删除。
- typed `ApiError` 继续使用 canonical error-code translation；active stage-fork blocker 直接留在确认框并停止删除轮询/Target reload，避免把 admission rejection误报成后台 cleanup 超时。

## Hypothesis Registry audit UI（Plan B，2026-07-30）

- `HypothesisRegistryAudit`保留为Plan B legacy presentation/test source，不提供rollout promotion、timeline authoring、Campaign/Prepared Action、queue-centric执行或Plan C/D操作。2026-08-08 unified exact DTO切换后，Candidate `ToolCallDetailView`已移除其operation-only production mount，默认API也fail closed；它不能伪造stage execution/request/snapshot selectors去调用unified wrapper。focused入口：`frontend/components/Engagement/HypothesisRegistryAudit.test.tsx`与`frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`。

## Unified Investigation full-pane route（2026-08-02）

- unified `Investigation` 只从当前 `stage_run` 的既有 `tool-detail` full-pane 进入。resolver 必须让 args/result/live rows 的 operation、stage execution 与 selected outer request完全一致；outer `call_...` 只用于选择/展示，exact IPC 使用 `stage_run:<stage_execution_id>` durable authority。pane terminal id必须经 `conversationTerminals` 精确映射到 conversation `aiSessionId`，缺映射时 fail closed，禁止把 terminal id当 AI session、禁止 latest fallback。
- Investigation 右侧 Agent transcript 的真实scroll owner必须复用`useTranscriptAutoScroll`：初次选择或切换到新的stable `actorId + transcriptRequestId`时跟随该对话bottom；同一actor增量、tool output布局增长继续贴底，但用户wheel-up/scroll-up阅读历史后暂停，直到回到transcript bottom。Hypothesis/Campaign等非Agent selection保持普通滚动，不能被Agent follow逻辑强拉。
- `InvestigationWorkspaceView` 是纯 presentational view model：左栏按真实 Main → organization bounded read session → Analysis Task 单 Primary/ordered subtasks/dynamic+nested workers → Hypothesis → Verification Task 单 Primary/workers → typed Operator artifact 排列。缺失 Main/transcript 显式 unavailable，绝不生成 `__main__` 或把 Operator 伪装成 Agent。
- `InvestigationWorkspaceRoute` 是唯一 production adapter：首次 summary 用 exact operation/execution/request 且四个 expected snapshot 字段全为 null；其后的 hypothesis/campaign/timeline page 与 hypothesis/campaign detail 全部固定到 summary 返回的 projection change-seq、temporal cutoff、authority epoch set hash 与 earliest valid-until。projection seq 与 run-control seq 是独立域：exact identity不要求二者相等，refresh/page只用前者，Stop CAS只用后者及run-state head。任何 continuation identity/snapshot 冲突都 fail closed，refresh event/gap 与 stale stop只触发新的 no-seq bootstrap，绝不改用 latest selector或重试 stop。后端命令只使用 owning conversation 的 `aiSessionId`，实时 transcript/refresh selector只使用当前 pane session；absent `activeSubAgents` 必须返回模块级稳定空集合，避免 React 19 external-store snapshot循环。
- Hypothesis/Campaign click只改组件本地`agent | hypothesis | campaign` selection；旧`PendingPreparedActionPanel`及其ToolCall/Campaign挂载点已物理删除，Campaign详情只展示历史审计投影。stop仍只提交server control projection的exact run-state head/change seq与稳定idempotency key；reset/fork availability只来自同一control projection。
- actor transcript由 exact `transcriptRequestId` 匹配同session live/restored `ActiveSubAgent.parentRequestId`，0条或多条均显示unavailable，不能猜最新Agent。Plan D旧 `InvestigationWorkspace/` 保留为legacy source且默认 API fail closed，不再由 `PaneLeaf`、`DetailViewMode` 或全局 store提供独立 route。focused入口：`InvestigationWorkspaceRoute.test.tsx`、`InvestigationWorkspaceView.test.tsx`、`ToolCallDetailView.investigation.test.tsx`。

## 2026-08-10 · Stage Agent tree recovery

- Production Stage/SubAgent navigation has one detail surface: `ToolCallDetailView` resolves the owning `stage_run`, then mounts `StageRunDetailShell` and `StageTeamWorkspaceView`.
- The left rail is the exact organization → Company Controller → dynamic/nested Worker call tree. The right pane renders only the selected Agent's transcript, current visible plan and tool calls; a ChatPanel SubAgent card selects the same Agent inside this tree.
- `App/detailFocus.ts` makes `tool-detail` and historical `sub-agent-detail` full-width workspace modes. `AIChatPanel` stays mounted for event projection but enters `renderUi=false`: its hooks continue projecting conversation events while the heavy ChatPanel DOM, resize handle and reopen control are released until timeline mode resumes.
- Projection-only隐藏仍释放ChatPanel重DOM，但`useChatAutoScroll`必须把同conversation旧viewport的`scrollTop + follow intent`保存在hook内，并在timeline返回后的新DOM上重绑wheel/scroll listener与ResizeObserver：原来贴底就定位到隐藏期间新增消息后的最新bottom，原来主动上滑则恢复原位置。该像素状态不写store/DB，也不跨应用重启。
- The retired standalone `SubAgentDetailView` and its private `StageAssetCoveragePanel` are removed; they must not be registered as a second production detail route.
- Focused tests: `frontend/App/detailFocus.test.ts`, `StageRunDetailShell.test.tsx`, `StageTeamWorkspaceView.test.tsx`, `StageTeamRunView.test.tsx`, `StageRunOrgRows.test.tsx`, and `SubAgentInlineCard.test.tsx`.

## 2026-08-11 · Stage Agent tool activity disclosure

- `StageTeamWorkspaceView` groups only adjacent ordinary `tool_call` transcript entries after applying the 200-entry presentation bound. Text/thinking, `update_plan`, `stage_team_dispatch_workers`, `sub_agent_*`, and missing tool identities terminate the group and retain their dedicated renderers.
- The default surface is a deterministic human activity summary. The first disclosure shows individual tool/runner/subject/status rows; the second shows exact command/output/Job context; `AI Tool raw data` is a third, independently collapsed Input/Result tree. Raw JSON must never dominate the default transcript again.
- `toolActivityPresentation.ts` is the single extraction contract; `ToolActivityDisclosure.tsx` owns the nested activity/tool/execution/raw renderer. A result-level `command` or exact `runner_execution.command` is an executed fact; `run_command` / `run_pty_cmd` / `shell` args may be displayed only as `requested`. EAS, Nuclei and `pentest_run` commands are never reconstructed because the backend owns recipes, config resolution and input-file substitution. Raw command strings remain byte-for-byte display/copy values; live mixed output is not duplicated with partial stderr.
- `vuln_probe_anonymous_access` is an in-process Rust HTTP producer, not a CLI wrapper. Its exact top-level `observations[]` become an HTTP execution variant showing separate Origin, method/path, Query overrides, status/error, verdict and response fingerprint. The UI never fabricates `$ curl` or joins partial query metadata into a claimed full URL; zero selected/sent requests remain a visible completed review rather than an empty card.
- Tool/process status is presentation only and must not imply Gate, coverage, Finding, or evidence success. Tools without an exact command or typed execution variant remain commandless and retain Raw Input/Result access.
- Focused regressions: `frontend/components/Engagement/toolActivityPresentation.test.ts` and `frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`.

## 2026-08-12 · Investigation retained-run presentation convergence

- A retained unified run may project one durable `investigation_primary` as both the Main coordinator and the Analysis Primary semantic role. The Investigation adapter canonicalizes that alias only when worker, transcript, organization and owning stage request all match exactly; Main remains the sole selectable transcript and the Analysis task renders a non-interactive `Handled by Main` alias. All other duplicate transcript identities still fail closed.
- `Bounded read session` is an immutable context authority, not an Agent. It remains visible as a non-interactive context row and is excluded from transcript selection/deep-link identity.
- Runtime `passed` is a successful terminal status for Workers, Subtasks and Tasks. The rail aggregates terminal/blocked/running states instead of treating every non-`completed` value as running.
- Investigation transcripts use the shared Agent message, latest-plan and Tool activity disclosure primitives in durable entry order. At most 200 entries render; terminal machine `submit_result` payloads stay behind the collapsed tool/raw disclosure instead of becoming assistant prose.
- Hypotheses show a humanized frozen predicate-schema title in the rail, keep the exact canonical predicate in a collapsed detail, and expand only the selected hypothesis or the path containing the selected actor. Closed runs hide unavailable Stop/Reset/Fork actions.
- Focused regressions: `InvestigationWorkspaceRoute.test.tsx` and `InvestigationWorkspaceView.test.tsx`.

## 2026-08-10 · WebContent memory stability

- Stage detail never renders the full ChatPanel tree and the selected Agent transcript at the same time. Projection-only ChatPanel mode preserves event ingestion and local conversation state without retaining its message/tool/SubAgent DOM.
- `StageTeamWorkspaceView` renders at most the newest 200 transcript entries, pins the current visible Plan when it falls outside that tail, and shows the exact omitted count. This is a presentation bound only: the store, `transcript.json`, and `run.log` keep the complete history.
- `useTargetSurfaceData` derives subscriptions from a sorted, de-duplicated target-id value key. Poll refreshes that return the same target set must not tear down and recreate AI-event or custom-event listeners because an array instance changed.
- The focused memory regressions are `AIChatPanel.reporting.test.tsx`, `StageTeamWorkspaceView.test.tsx`, and `useTargetSurfaceData.test.ts`.

## 2026-08-10 · Message-scoped Stage Plans and fixed Stage workspace

- Harness Plan cards are message-scoped. `useAiChatEvents` freezes an entered stage onto the next
  assistant message; projected future seeds remain absent. `anchorStagePlan` is first-write-wins,
  and `stagePlanPersistence` stores the same immutable mapping for refresh/restart recovery.
- `useTaskPlanState.stageIdsByMessage` is the only inline projection. A stage's later plan versions
  update that historical card in place; unanchored stages are visible only in the explicit workflow
  control, never guessed onto an arbitrary old message.
- `StageProgressBar` is a compact status strip below the conversation tabs. It does not duplicate
  the active Plan; the full stage roadmap opens only from its workflow button.
- A Company Controller `stage_run` replaces the generic Tool header/body/footer. The unified Stage
  workspace receives `h-full/min-h-0/overflow-hidden`; `StageTeamWorkspaceView` owns the bounded
  Agent transcript viewport and scrolls output internally.
- Focused tests: `StagePlanStack.test.tsx`, `StageProgressBar.test.tsx`,
  `hooks/useAiChatEvents.test.tsx`, `hooks/useTaskPlanState.test.ts`,
  `stagePlanPersistence.test.ts`, `store/workflow.test.ts`,
  `ToolCallDetailView.stage-workspace.test.tsx`, and `StageTeamWorkspaceView.test.tsx`.
