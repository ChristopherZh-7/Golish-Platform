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
- `db_bridge/recon.rs` 实现 `org_subtree_units` 时走 `golish_db::repo::organizations::subtree`，供 root-bound `stage_run` 读取完整 DB organization tree；这条路径不要降级成仅信任工具参数里的 `orgs`。
- `query_target_data` 的 enumeration 读模型支持 `sections=["directories","coverage","web_roots"]`：`directories` 读 target-bound `directory_entries`，`coverage` 返回 DIR/PARAM/JSAPI 的 found-only DB truth summary，`web_roots` 从 EAS 已落 target URL / http metadata / web-like ports 派生根 URL。缺席的 coverage fact 不是 checked_empty。
- `ai_get_stage_asset_coverage` 是 `stage_run` 详情页的只读矩阵接口：按 `(organization_id, stage, optional session_id)` 返回 asset × technique snapshot，found 来自 `coverage_truth`，checked_empty/error/blocked/not_applicable 来自 `technique_outcomes` + `source_query_log` terminal rows + 同 session `audit_log` evidence facts，适用性必须和 gate 一样用 `AssetClass::classify(Some(target_type), value)` 的 value-aware 口径，不能只看 `targets.type`；URL 形态值即使存成 `domain` 也不能冒出假的 PORT/SERVICE pending。EAS LIVENESS lookup 使用 endpoint key（保留 URL port/path），PORT/SERVICE-FINGERPRINT lookup 使用 `canonical_asset_key` 的 host key，必须和 `technique_outcomes` 写入侧保持一致，避免 `http://ip:port` 资产和裸 IP outcome 漂移。同 session evidence 里 EAS LIVE/PORT 的解析失败/无目标错误（如 failed to resolve、no targets specified）在 UI read-model 显示为 `checked_empty`，不把可解释的终态负结果画成 generic error。EAS 中若 PORT 已经 terminal `checked_empty/not_applicable`，read-model 会把仍 pending 的 SERVICE-FINGERPRINT 派生为 `not_applicable`，因为没有开放端口就没有服务指纹面；有显式 SERVICE outcome 时不覆盖该 outcome。Tauri UI 命令允许在 session id 缺失或对不上当前 terminal rows 时，用同 org 最新 terminal outcome 做显示兜底，避免已查空被画成未查；但 `check_stage_asset_coverage` 走 DB trait 调用时必须保持 strict session 口径，不能用旧 run 结果帮 agent 通过提交前预检。`check_stage_asset_coverage` 复用同一个 snapshot helper 给 agent 做提交前预检，输出 `ready_to_submit` / `gap_examples` / `next_action`，避免模型在看不到前端覆盖表时盲交；worker objective 必须把它当作 `submit_stage_deliverable` 前的 mandatory self-check，而不是提交失败后的修复提示。开启 `asset_wave_barrier` 的 stage 会把 `stage_started_at` 之后新发现的资产标为 `new_in_stage` / `next_wave_pending`：它们继续展示在 UI 中，但不计入当前 batch 分母，也不阻塞当前 batch submit；`stage_run` 不会在单个 org gate PASS 后立刻自动消费这些资产，它们是后续 global delta expansion pass 的 backlog。`StageAssetCoverageRow.real_ip` 来自 `targets.real_ip`，供前端在 EAS/Enumeration 里按 IP 聚合 domain/url 子资产和 related live work；它是解释关系，不是新的 gate 判定来源。`error` 表示工具/来源执行失败，`blocked` 表示 scope/key/rate-limit/人工阻塞，二者不要混成同一个状态。target_intel 会额外返回 organization 覆盖行，让 WHOIS/ASN/OSINT 在尚未落真实 target 前也能被解释；organization 行不计入资产 summary，DNS/CT/Subdomain 在该行固定 `not_applicable`，source/provider terminal rows 只有 WHOIS/ASN/OSINT 可回卷到 organization 行，不能把公司名变成 `公司 × DNS/Subdomain` 缺口。前端必须把 organization 行显示为「组织情报」，不要混入资产列表或资产分母。它只解释当前 gate 输入，不自行决定 PASS/BLOCK。
- EAS coverage read-model 的直接分母是 host/IP 扫描主体：如果 domain/url/`http://IP:port/path` 资产通过 `real_ip` 或 URL host 解析到同 org 已有 direct IP target，则该行保留作解释/子行，LIVE/PORT/SERVICE 全部显示 `not_applicable` 且不计入 `done/total/pending/new` 分母；没有对应 direct IP target 时保持保守，domain/url 仍作为直接覆盖资产，避免吞掉唯一可扫目标。
- `ai_get_stage_asset_coverage` 在 `enumeration` 会优先把资产列表收敛到 EAS `GOLISH-EAS-LIVENESS` found 的 domain/url web roots；若没有任何 EAS live truth，则不收敛为空，保持 fail-safe 分母。这个 read-model 口径必须和 `org_gate` 的 per-org gate 分母一致。
- `list_attack_surface_seeds` 走 `golish_app_core::domain::targets::rank_attack_surface_seeds`，返回给 Prober 的主扫 worklist 必须和 EAS 覆盖分母同口径：已有 direct IP target 时，解析到该 IP 的 domain/url/`http://IP:port/path` 只作为资产关系/解释存在，不再作为主扫 seed；没有 direct IP target 时保持原 domain/url seed。
- enumeration coverage 的 pending cell 只能建议当前一方工具：DIR → `route_probe_paths`；PARAM → `browser_collect_js_api` + `js_extract_apis`（从浏览器观测/查询串/表单/param_hints 得到参数）；JSAPI → `browser_collect_js_api` + `js_extract_apis`。不要再把 `ffuf` / `arjun` 注入 `suggested_tools`，避免 worklist 和 methodology/prompt 边界打架。
- `db_bridge/evidence.rs` 同时实现 evidence ledger、`technique_outcomes`、`source_query_log` 的 harness read/write seam；`source_query_facts` 只投影 source/provider terminal rows，不代表 found。
- `harness_submit_tool` 的 `submit_stage_deliverable` schema 是模型看到 coverage cell 字段的最后一道说明；DB-truth 阶段（target_intel / EAS found cells）要明确让模型不要手抄 found coverage，只提交 DB 不能推导的 terminal cells；保留 SERVICE-FINGERPRINT denominator 文案仅用于显式 non-DB-truth / negative cells。specialist fan-out 阶段提交 `stage_run_pass_token` claim 时，submit preview 只做结构/伪造 evidence-id 检查并先收进 side-channel，最终 closeout 由 orchestrator 从 `org_stage_completions` 重算验 token，不要先按普通 claim 要 evidence。
- `submit_stage_deliverable` 对 `enumeration` 的瘦交付说明要保持明确：`findings: []`，claims 使用 `web_root_enumerated` / `directories_discovered` / `api_endpoints_discovered` / `params_discovered` / `js_candidates_reviewed` 等 kind，DB-derived found cells 来自 `directory_entries` / `api_endpoints`，coverage 只提交 DB 不能推导的 checked_empty/blocked/not_applicable。
- `harness_submit_tool` 的预检会读取 active operation 的 `stage_started_at`；当 stage spec 开启 `asset_wave_barrier`（当前 EAS）时，预检资产轴和 DB truth freshness 都按该 cutoff 收敛，避免模型在当前 wave 尚未完成时被新发现资产撑大的分母反复拦住。
- `submit_stage_deliverable` 若 gate 因 `coverage_complete` BLOCK，会把 `HarnessRecoveryActions.coverage_gap_actions` 原样放进 `needs_fix.coverage_gap_actions`，让 sub-agent 下一轮拿到结构化 action list（而不是只读 `reasons` 里的前 8 个字符串）。
- `submit_stage_deliverable` 在 active stage 内会先检查本 session 归因的 background jobs；生产默认快速 `needs_fix`，要求模型调用 `wait_for_background_jobs` 显式等待、读取完成 job 的 stdout/stderr tail 后再提交。`GOLISH_SUBMIT_RECONCILE_WAIT_MS` 可恢复旧的 bounded in-submit wait，但不应作为默认 UI 体验。
- `commands/bridge_config.rs` 每个非 title-gen session 会监听 `background_jobs` completion 与 live output：completion 负责 evidence/note，并把成功 job 的 retained stdout（优先 `background_jobs::manager().snapshot()`，fallback completion tail）送入 `golish-pentest::output_store::maybe_detect_and_store_via_context` 做结构化落库；live output 转成 `AiEvent::ToolOutputChunk` 给前端现有工具详情面板追加显示。同步前台 `PostShellHook` 使用 `maybe_detect_and_store_via_context` 并传入当前 harness org，让 EAS 主动发现的新资产进入该 org 分母。后台 completion 的 evidence `tool_name` 必须从命令行解析真实工具名，不能统一记成 `background_job`：`httpx`/`whatweb`/`curl`/`wget` 要落 `http_probe`，`nmap` 落 `nmap`，`naabu` 落 `port_probe`，否则 submit preview / gate 的 `min_invocations` 看不到真实工具证据。WhatWeb 可能通过 Ruby wrapper 启动（`ruby .../whatweb`），解析时要透过解释器取真实工具名。`httpx -l` / `httpx <<'GOLISH_STDIN'` / `nmap -sn -iL` 这类批量探活 completion 要读取 input file 或 heredoc/stdin body，把每个非 CIDR host/IP 的 LIVENESS outcome 写入 `technique_outcomes`：输出命中为 `found`，无命中为 `empty`，避免批量探活零输出被 gate 当作 never attempted；LIVENESS 的 asset key 必须保留 URL endpoint 的 port/path（例如 `http://x.com:90` 写成 `x.com:90`），不能用 host-level canonicalization 折叠成裸域名。`naabu -list` / `masscan -iL` 这类批量端口扫描完成后还要读取 input file，把每个非 CIDR host/IP 的 PORT outcome 写入 `technique_outcomes`：有开放端口为 `found`，无开放端口为 `empty`。`whatweb --input-file` / `nmap -sV -iL` 这类批量服务指纹 completion 也读取 input file，为每个输入 host/URL 写 `GOLISH-EAS-SERVICE-FINGERPRINT` outcome：输出命中的资产为 `found`，无命中为 `empty`，避免批量 whatweb/nmap 已跑但 gate 仍按 never attempted 卡住；`nmap -sn -iL` 不应误落 service fingerprint。所有后台 EAS batch outcome 写入前必须按当前 `organization_id` 的 in-scope `targets.value` 和 `targets.real_ip` 建 allowlist：LIVENESS 用 endpoint key，PORT/SERVICE 用 host key；org 下没有 in-scope asset 时直接跳过 outcome upsert，不能把猜测域名或其解析 IP 记到该 org 的 `technique_outcomes`。batch input file 参数可能出现在 `--input-file='/abs/path'` 这类 equals+quoted 形态，解析时必须先剥掉值两侧引号再判断绝对路径，不能拼成 `workspace/'/abs/path'`。
- `cancel_ai_generation` / `shutdown_ai_session` 不只设置 `AgentBridge.cancelled`；还必须 kill 当前 `session_id` 归因的 running background jobs，避免 ChatPanel Stop 只停主 agent 而留下 worker 扫描进程继续跑。
- `harness_submit_tool` 在 validate-on-submit 前会从已引用的真实 evidence id（顶层 `evidence_refs`、claim `evidence_ids`、finding/coverage refs）查询 ledger kind，并按 stage spec 回填 `required_checks_done`（如 `http_probe`），避免后台证据已存在但模型漏填 hint 时被误导去重跑。
- `commands/core/chat.rs` 的 Task/profile 模式是“默认工作预设”，不是每条消息都强制进 harness：普通发送先跑一个 flexible lead-agent turn（正常聊天/写代码/调试工具 + `start_operation` handoff），只有模型主动调用 `start_operation` 才进入 Scoping→Reporting 的 TaskOrchestrator；没调用就把 lead turn 回复作为最终答案。显式 `/task ...` / `/harness ...` 仍可绕过 lead turn 直接进 harness。对“帮我搞一下平安”“对 https://target 做渗透”“目标 example.com 进红队模式”这类当前消息已有目标+操作意图的句子，代码层会窄口径直接进 harness，避免再赌 lead 模型是否愿意调用 tool；普通问候/身份问题/代码调试语句仍留在 flexible lead。lead turn 还会通过 `execute_with_turn_instructions` 注入隐藏的 Task/Profile lead policy：口语化操作意图（如“搞一下 <公司>”“帮我打/测/扫 <目标>”“开搞/进红队模式”，以及已给过目标后的“就搞他/整个集团”）应调用 `start_operation` 交给 Scoping，而不是在 lead 外层反复追问；lead objective 只保留用户给出的目标标签/活动，不扩写域名、子公司、IP 段或实际扫描范围。该 policy 不应作为用户消息写入 UI/历史。
- Task/Profile 收到短“继续/接着/resume/continue”类 operation continuation 时，`commands/core/chat.rs` 会先用当前 chat session 查 `latest_resumable_by_session`；如果存在 checkpointed task，直接进入 `execute_task_mode`/`TaskOrchestrator::resume`，不要先跑 flexible lead 去靠模型重建上下文。没有 checkpoint 或不是 operation continuation 时才回落到 normal lead。
- 同一路径里，真正“裸继续”（如 `继续` / `接着跑` / `continue the previous stage`，且不含“先解释/看日志/不要扫”等 steering）会给 `TaskOrchestrator` 打开一次性 `stage_run` fast-resume hint；带条件或诊断意图的继续仍走普通 resume，让模型先回答/读上下文。
- Task/Profile fresh operation 会做 continuity preflight：`start_operation.continuity_decision` 默认为 `ask_before_reuse`，发现旧 DB progress 且没有同会话 checkpoint 时，必须通过共享 `ask_human`/approval coordinator 发 `AskHumanRequest(input_type="choice")` 卡片确认，不要只 emit 普通 assistant 文本；用户明确“复用/沿用/用已有”后走 `reuse_existing` 并把 `ContinuityAdoptionPlan` 交给 orchestrator；用户明确“重新开始/不要复用”或 Skip/timeout 后走 `start_fresh`。如果没有 coordinator（单测/降级路径），才回落到文本确认。
- Task/Profile lead 不应暴露 shell、`query_target_data` / `list_in_scope_targets` / `list_attack_surface_seeds` / `check_stage_asset_coverage`、`manage_organizations` / `manage_targets` / `recon_*` / `pentest_run` / `stage_run` / `submit_stage_deliverable`，否则会绕过 Scoping 的 `ask_human`/gate，在 harness 外直接查库/跑命令。
- 扁平 re-export 是 A3 删 umbrella 的兼容垫片，镜像 umbrella 旧导出；别乱删。
- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app ai
```
