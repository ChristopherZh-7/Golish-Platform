# golish-sub-agents

> **一句话职责**：sub-agent 系统——sub-agent 定义（自定义 system prompt + 工具限制）、registry、发现/加载（YAML frontmatter）、执行器（含 udiff 应用）、prompt registry/contributor，以及默认 sub-agent 集。

- **类型**：crate（Layer 2 · agent 基础设施）
- **路径**：`backend/crates/golish-sub-agents/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 定义/registry、`execute_sub_agent` 执行链、嵌套深度（`MAX_AGENT_DEPTH`）时
- 改 agent 文件加载（YAML frontmatter + 文件系统发现）、默认 sub-agent 集时
- 改 sub-agent prompt 模板（tera）/ contributor / skills 注入时

## 职责

提供 sub-agent 编排基础设施：定义专门化 sub-agent、管理可用 agent 注册表、在 agent 间传递 context、带工具支持地执行 sub-agent。通过 `ToolProvider` trait 注入工具定义/执行，避免对上层 agent runtime 的反向依赖。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `SubAgentDefinition` / `SubAgentRegistry` | sub-agent 定义与注册表 |
| `SubAgentContext` / `SubAgentResult` / `AgentSource` / `MAX_AGENT_DEPTH` | 上下文/结果/来源/深度上限 |
| `execute_sub_agent` / `SubAgentExecutorContext` / `ToolProvider` | 主执行函数 + 工具注入 trait；执行 context 分离事件 session key 与 DB persistence session UUID |
| `create_default_sub_agents` | 默认 sub-agent 集 |
| `discover_agents` / `AgentFileInfo` | 文件系统发现 + 加载 |
| `PromptRegistry` / `PromptContext` / `SubAgentPromptContributor` | prompt 注册/上下文/贡献者 |
| `StageToolGuard` / `StageToolHider` / `SubAgentToolRouter` / `SubAgentToolResultHook` / `SubAgentToolObserver` / `PostShellHook` / `SubAgentChainPersistence` | 阶段工具守卫/路由/工具结果后处理/telemetry observer/持久化（executor_types） |
| `BoundWorkerChainContext` / `BoundWorkerToolLifecycle` / `StageTeamLeaderBinding` | V2 stage worker 的 server-owned prebound chain、lease/version witness 与 awaited tool fence；只有 exact Company Controller claim 带 trusted leader binding |
| `STAGE_TEAM_UPDATE_PLAN_TOOL_NAME` / `STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME` / `STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME` | trusted Controller-only 工具；`update_plan` 是本地计划普通工具，后两者才分别形成 durable dispatch 与关闭 request epoch/准备 final turn 的 coordination barrier |
| `SubmitRepairMode` / `SubmitRepairKind` / `StageCapabilitySuggestion` | StageRefiner 产出的 repair directive 在 executor 内的兼容投影；负责 resume repair lock、capability-first gap action 展示与 allowed/forbidden tools |
| Plan B static roles | `candidate_hypothesis_controller`、`candidate_hypothesis_analyst`、`merge_conflict_critic`；均readonly、submit-result-only、无网络/扫描/delegation |

## 依赖

- **内部**：`golish-core`、`golish-udiff`、`golish-tools`、`golish-shell-exec`、`golish-llm-providers`、`golish-json-repair`、`golish-skills`
- **外部**：`rig-core`、`serde_yaml`、`tera`、`dirs`

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`、`golish-agent-app`、`golish`。整条 agent 栈都依赖它。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `definition/` | 定义/registry/context/result | [→](golish-sub-agents/definition.md) |
| `executor/` | 执行链（execute_sub_agent） | [→](golish-sub-agents/executor.md) |
| `executor_helpers/` | 执行辅助（content/history/helper） | [→](golish-sub-agents/executor_helpers.md) |
| `defaults/` | 默认 sub-agent 集 + prompt fallback | [→](golish-sub-agents/defaults.md) |

## 关键文件

`discovery.rs`、`file_loader.rs`、`prompt_registry.rs`、`prompt_contributor.rs`、`schemas.rs`、`transcript.rs`、`executor_types.rs`、`executor_udiff.rs`。

## 注意事项 / 坑

- `MAX_AGENT_DEPTH` 限制嵌套递归——改 sub-agent 调 sub-agent 时务必尊重深度上限，防失控。
- 工具走 `ToolProvider` trait 注入（非直接依赖上层 runtime），保持本 crate 处于 L2，不要引入向上依赖。
- 默认 `recon` 子 agent 是 `target_intel` 的 provider-only 生产者：不暴露 `list_in_scope_targets` / `pentest_run`，避免在 intel 阶段查询尚未生产的目标或 fallback 到 subfinder/dig 类扫描路径；`prober` 消费 ranked attack-surface seeds，`enumerator` 必须先消费 `stage_worklist_status` / `stage_worklist_next` 的 DB-truth stage-local worklist，再把 `list_enumeration_web_roots` 当 web-root 上下文。`enumerator` 不暴露 `manage_targets` / `record_finding`，因为 `enumeration` 是 content coverage 阶段，产物是 DB truth + claims + non-found terminal coverage，不是资产状态更新或漏洞 findings。
- sub-agent 的 registry/router 工具执行会在 `with_agent_session`、`with_agent_tool_context` 和 `with_agent_tool_output_sender` 下运行；`SubAgentExecutorContext.operation_id` 由父 runtime 提供并原样写入 tool context，nested delegate 继续继承。这样 direct bridge tools 既能发实时 `tool_output_chunk`，也能使用可信 stage-attempt identity；新增执行分支时不能丢这些 scope 或改从模型参数取 operation id。
- `SubAgentExecutorContext.session_id` 是事件/trace 文本键，不保证能解析成 UUID；message-chain create/latest lookup 使用独立的 `persistence_session_id`。executor 仅为 legacy 裸 UUID session 保留回退，nested delegate 必须继承 persistence UUID，不能用 fresh chain 冒充精确 continuation。
- prebound V2 worker 不是普通 resume selector：chain id、session、operation/execution/unit、lease token/epoch、checkpoint version 全由 host 注入；load/checkpoint/tool landing 任一 fence 失败都标记 lease lost 并停止后续 work。generic chain SQL 只保留给普通非-stage sub-agent。
- `StageTeamLeaderBinding` 是 Company Controller 的 host-owned 窄授权，不得从 agent role/name 或模型参数推断。只有 active plan 中 exact `leader:primary`、controller role、running WorkItem 与匹配的 plan/unit/org/version fence 才能获得；普通 child、legacy worker 与 Candidate verifier 均为 `None`。只有该 binding 会把 `stage_team_dispatch_workers` / `stage_team_prepare_final_submission` 加入工具面；成功的真实 router result 会分别形成 `dispatch_accepted` / `prepare_final` executor barrier，并且必须先把完整 raw JSON tool result durable checkpoint，再把控制权交还 scheduler。模型 prose、伪造 status 或普通 worker 同名调用都不能触发 barrier。
- Company Controller 与动态 child 都通过 `recon` definition 执行，以保留 Target Intel 阶段业务工具；exact trusted leader binding 只给 Controller 追加普通 `update_plan` 与 coordination controls，child objective 还显式禁止创建/修改 Controller plan。Controller 的计划限制为 1–12 步，未完成时恰好一项 `in_progress`；首轮复杂任务、dispatch 前、child 输出回流后与 Gate gap 恢复后均须更新。`update_plan` 不是 terminal coordination barrier，单独调用不会结束 executor；协调轮必须继续到 `dispatch_accepted` 或 `prepare_final`。prepare 前计划全部 completed，final submit turn 只读该语义并直接调用 `submit_stage_deliverable`；BLOCK 后才由同一 Controller chain 重开 repair step。
- `BoundWorkerChainContext.return_on_first_durable_stage_submission` 仍是 host-owned 兼容策略：当同一 Company Controller 已被 scheduler 绑定为 final submitter 后，`submit_stage_deliverable` 的 accepted 或带 durable submission id 的 needs_fix 会在完整 tool-result checkpoint 后把 id 交还外层 Gate/finalization 流程。动态 child 不启用该策略并保留自身工作链语义；Candidate terminal-intent barrier也不受影响。DB/内存结构中仍可能出现 Aggregator 命名，这是持久化兼容字段，不代表运行时另建一个汇总 Agent。
- `SubAgentChainError` 是跨 runtime 的稳定链失败类型。只有未传 `resume` 才允许创建 fresh chain；字面 `latest` 与 exact UUID 的 miss/error/坏 JSON 都 fail closed。exact load 必须同时校验当前 persistence session 与 agent ownership；final chain body 写回成功后才可输出 resume marker。provider 明确报告 input context 超限时返回 `ProviderContextLimitExceeded`（携带当前 optional chain id），不能降级成普通 `SubAgentResult` 后被 stage-run 自动重试；分类必须要求 context/token-limit 语义，不能把所有 HTTP 400 或 TPM rate limit 都归进来。
- resumable chain 的历史必须保持 tool-call/tool-result 紧邻且 call id 成对；barrier/stall 退出和 provider stream error 都不能留下或继续持久化半轮。该约束在 executor control flow 与 serialize/restore validator 两层执行。
- exact/latest chain restore 在首次 provider I/O 前还要执行 deterministic provider-history compaction：历史 ToolResult 按工具保留 worklist/retry/checkpoint 关键状态并设单结果上限，重复 `RESUME REPAIR DIRECTIVE` 只留最新 bounded projection，完整 tool-call/result turn 原子保留；变更后的 body 必须先 durable rewrite。相同 compactor 也在每次 sub-agent model stream 前和最终 chain persist 前执行，确保同一长 segment 新增的结果不会再次把上下文撑爆，重复 exact retry 字节稳定。
- `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` 这类长耗时 direct bridge tools 不走 sub-agent 外层 `tokio::time::timeout`，否则 future 会被 drop，工具来不及写最终 DB truth。shell/pentest 命令的软超时转后台由上层 `golish-app-core::background_jobs` 处理；若要让 direct bridge tools 也返回 `backgrounded/job_id`，需要经 `ToolProvider` 注入后台适配 seam，不能在本 L2 crate 直接依赖 app-core。
- `SubAgentToolResultHook` 只提供通用结果后处理注入点；具体 harness/evidence/source_query 副作用由上层 runtime 注入，避免本 crate 反向依赖 DB/harness。
- `SubAgentToolObserver` 是上层 runtime 的工具观察点；当前 Mentor 只做 telemetry，不再注入模型可见纠错。真正的 repair guidance 由 runtime/agent-kit 的 StageRefiner 产出，再通过 `SubmitRepairMode` 注入 executor。
- sub-agent 工具结果要区分两条通道：`AiEvent::SubAgentToolResult` / transcript 保留 raw JSON 供 UI 和证据追溯；回灌给下一轮 LLM 的 `ToolResult` 会按工具做 model-visible compaction。`route_probe_paths` / `list_enumeration_web_roots` / JS collect/extract 这类大结果只给 counts、samples、next_action 和 raw-retained 标记，避免 stage_run worker 把完整 route/error/coverage 数组反复塞进上下文；browser batch 仍须逐 root 暴露 bounded completion/closure/page-resume diagnostics，不能把 50 个 root 压成一个不可定位的总数。
- `SubmitRepairMode` 的 coverage-gap repair 不再按“批量”本身拦截 wrapper；EAS gap 很多且 gate 给出非空 `coverage_gap_actions` 时，必须允许 EAS wrapper 的 batch 参数（`targets[]` 或 `target_urls[]`）并阻止 raw `pentest_run` / raw `whatweb`。EAS action 会按 `GOLISH-EAS-*` technique 自动放行直接 wrapper：LIVENESS→`eas_probe_http_liveness`、PORT→`eas_discover_ports`、SERVICE→`eas_fingerprint_services`、WEB-FINGERPRINT→`eas_fingerprint_web_stack`，即使 legacy `suggested_tools` 为空也不能把 wrapper 挡掉。enumeration gap repair 会按 `GOLISH-ENUM-*` action 自动放行 direct tools（`browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` / `enum_crawl_same_origin_urls`），并继续允许 `stage_worklist_status` / `stage_worklist_next` / `list_recent_evidence`；若是 ENUM gap，还允许 `list_enumeration_web_roots` 取带 `target_id` 的 web-root context。direct tool fence 会校验 `target_url`/`target_urls`/`base_url` 必须落在 `coverage_gap_actions` 点名资产内，且 `target_urls` 可为 bare URL、`{target_id,target_url|root_url|base_url|url}` worklist 对象，或带 per-target `recipe` 的 `browser_seed.target_urls` 对象。DIR gap 只能用 `route_probe_paths`，不允许回退 ffuf/gobuster/feroxbuster；PARAM gap 默认用 browser/js_extract/crawler 已观察到的请求、query、form 与 `param_hints` 补 `api_endpoints.params`，bounded browser seed discovery（katana）只能经 `enum_crawl_same_origin_urls` wrapper 作为 Playwright route/script seed 来源。repair 模式必须继续允许 `check_stage_asset_coverage`，因为它是 submit 前自检和修复收口的只读工具，不能被 repair lock 挡掉。它仍会阻止 CIDR/range sweep、隐藏 list file（未提供可校验的 `input_lines`/`stdin`）以及任何不在 `coverage_gap_actions` 中的目标，确保批量只覆盖 deterministic gate 点名的资产。若 coverage needs_fix 没有结构化 `coverage_gap_actions`，repair 只允许 coverage/DB 查询、后台 job 控制和 resubmit，不能启动 `pentest_run` 或 guessed-domain probes。
- `SubmitRepairMode.coverage_gap_actions` 始终全量保留给 allow/deny/target guard；模型文本只看 `total + stable hash + 原序前 20 条 sample + stage_worklist_next`，并受 32 KiB 硬上限约束。StageRefiner 传入的 `directive_message` 若已含该投影，`model_instruction()` 不得二次追加。blocked-tool 结果中的 `coverage_gap_actions` 是最多 20 条的投影对象（含 `total/hash/omitted/next_page_tool`），不是内部全量 vector；整个 blocked payload 有 64 KiB 降级上限。
- `CoverageGapAction.suggested_capabilities` 是模型可见的优先修复口径；本 crate 保留同形 `StageCapabilitySuggestion` DTO，避免向上依赖 `golish-agent-kit`。`suggested_tools` 继续存在，只作为工具/命令 hint 和旧 repair prompt 兼容字段。
- doc 注释提到的 `golish-web` / `vtcode-core` 为历史描述；当前 Cargo.toml 实际内部依赖以本卡「依赖」段为准。
- Plan B Candidate Team只执行closed artifact schemas。small input允许1个live lane，其余2–8；该数字是live concurrency，不是总WorkItem限制。artifact receipt/page receipt只证明持久化/bytes交付，不是semantic coverage、Gate或canonical truth；Plan C/D不在本crate。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents
```
