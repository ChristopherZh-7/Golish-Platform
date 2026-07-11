# golish-agent-kit / tool_executors

> **一句话职责**：各类工具的具体执行实现——web fetch、plan、ask_human（barrier）、memory（搜索/存储/列举 + code/guide store）、knowledge_base（wiki 漏洞知识）、security（finding 管理/分析）、graph（实体/关系知识图）、sploitus、shell helper。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/tool_executors/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改某类工具的具体执行（memory/knowledge_base/security/graph/sploitus/ask_human/web/plan/shell）时
- 加新工具执行器或改其结果契约时

## 职责

提供 `tool_execution` 路由后落到的具体执行逻辑，按域分文件。注意 workflow 工具执行在 golish crate（避免与 WorkflowState/BridgeLlmExecutor 的循环依赖）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `execute_ask_human_tool` | ask_human barrier 工具 |
| `error_result` / `extract_string_param` / `ToolResult` | 公共 helper（common） |
| `graph` / `graph_trait` | 知识图执行 + trait |
| `knowledge_base` / `security` | wiki 知识库 / finding 管理（pub 模块） |
| memory / plan / shell / web（内部模块） | 各域执行 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `memory.rs` / `knowledge_base/` / `security.rs` | 记忆 / 知识库 / 安全分析 |
| `graph.rs` / `graph_trait.rs` | 知识图 + trait |
| `ask_human.rs` / `plan.rs` / `shell.rs` / `web.rs` / `common.rs` | barrier / 计划 / shell / web / 公共 |

## 依赖

- crate 内 `tool_execution`；`golish-tools`、`golish-pentest`（evidence/finding）、`golish-graphiti`（图，经 trait）

## 注意事项 / 坑

- workflow 工具**不在这里**（在 golish，避循环依赖）；别往这加 workflow 执行。
- graph 走 `graph_trait`（注入），不直接依赖 golish-graphiti 具体实现。
- `security.rs` 里的 `check_stage_asset_coverage` 是只读提交预检：从 app bridge 取和前端覆盖面板同源的 stage asset snapshot，但 agent-facing 输出必须压缩成 `ready_to_submit`、cell 计数和 `gap_examples`；即使模型传 `include_assets=true` 也不返回完整 `assets` 矩阵，避免把大覆盖表灌回 LLM 上下文。执行路径必须传入 active harness org/stage/operation，避免模型在 stage 外或错误组织上看覆盖；`check_stage_asset_coverage`、`stage_worklist_*`、`list_enumeration_web_roots` 的显式 `organization_id` 若与 active `harness_org_id` 不同必须直接拒绝，不能覆盖绑定组织做跨 org read。`next_wave_pending` cell 表示当前 stage 中新发现、等待下一批 `stage_run` 的资产，不能当作当前 wave 的 pending/error gap 阻止提交。
- `stage_worklist_status` / `stage_worklist_next` 是 `check_stage_asset_coverage` 同源的 DB-truth worklist view：status 返回 compact preflight + 下一步工具建议，next 返回按 `prefer` 过滤的 asset×technique work items（默认 pending/error/partial，带 suggested_tools/evidence_refs/stage-specific focus 和可选 `details`）。Enumeration snapshot 只含 `exact_web_origin=true` 的 canonical HTTP(S) origin；rootless alive host、unknown TCP service 与无法确定 scheme/port 的 target 从 total/done/pending/gap 全部排除，不能显示成 `missing_exact_web_origin` 或伪 done。`work_item_id` 包含 `target_id + origin value + technique`，同 target 的 HTTP/HTTPS/非默认端口不会相互覆盖。Enumeration 的 found/empty 只认 current-run fresh exact-origin `technique_outcomes`；partial/error 一律未完成，business rows 与自然语言不能关闭 work item。blocked 只认 current-target evidence-backed 的 preflight 四轴、route DIR recovery exhaustion、browser JS/JSAPI/PARAM recovery exhaustion；worklist 文案必须提示普通 partial/error 继续恢复，但 producer 已持久化 `blocked + recovery_exhausted=true` 的 owned cell 不再重试。Enumeration 三个 worklist/preflight 入口缺 active run/session 或当前 stage cutoff 时直接 fail closed，不读 latest outcome。EAS 输出会额外写明工具边界：domain/URL LIVENESS 用 httpx，IP/CIDR PORT 优先 naabu/masscan，SERVICE 对确认 open ports 跑 `eas_fingerprint_services` / nmap -sV；若 gap 带 `details.missing_open_ports`，worker 应直接用这些端口补扫。WhatWeb 只用于确认 HTTP(S) endpoint 的技术指纹。
- `query_target_data` 在 active `harness_org_id` 下必须先读 `in_scope_targets(Some(org_id))` 做 target ownership guard：只有 `target_id` 出现在当前 org/subtree 的 in-scope rows 才能继续 drill-down；foreign、unowned、invalid row 或 ownership 查询失败都 fail-closed，不能借只读详情接口跨组织读取 endpoint/JS/coverage 数据。没有 active org 的 legacy/chat 路径保持原兼容行为。
- `log_operation` / `discover_apis` / `save_js_analysis` / `fingerprint_target` / `log_scan_result` 是未携带 immutable `TargetWriteGuard` 的 legacy mutation API：主 agent 默认配置不再暴露，active harness stage 在 executor chokepoint 一律拒绝。非 harness 兼容调用至少要通过当前 workspace/org 的 in-scope target read guard；阶段事实必须改用 stage-specific guarded producer，禁止把这些旧 API 加回 specialist/default tool list。
- `vuln_triage` 的 coverage snapshot / worklist 要生成公式化扫描的 10 个 WSTG/GOLISH technique cell；如果 worklist/preflight 拿到 0 个 cell，必须返回 `ready_to_submit=false` + `coverage_denominator_missing=true`，不能把空矩阵当作可提交状态。该阶段的 suggested capability 是 `vuln.run_formulaic_sweep`，工具 hint 必须是 `vuln_run_formulaic_sweep`；`nuclei` / `sqlmap` / `wpscan` 只存在于 backend wrapper 的固定 recipe 内，PASS/BLOCK 仍只看 DB/gate truth。
- `check_stage_asset_coverage` 在 `enumeration` 会把 `gap_examples` 解释为 EAS-confirmed exact-origin worklist，并附带 `worklist_semantics` / `deliverable_contract`；gap_examples 也带 `root_url` / `base_url` / `scheme` / `port`，DIR gap 可直接喂给 `route_probe_paths`。worker 必须补齐 JS/JSAPI/DIR/PARAM 四轴并恢复普通 partial；producer 已落 recovery-exhausted blocked 时停止重试。submit 只允许 summary claims、`findings: []`、`coverage: []`，不能手写 found/empty/blocked/not_applicable，也不能把 `directory_entries` / `api_endpoints` / `js_analysis_results` 当 completion truth。
- `check_stage_asset_coverage` / `stage_worklist_*` 对 Enumeration 的权威 `assets: []` 保持零分母 ready，与 gate 的 `Some([])` vacuous PASS 一致；缺失 snapshot 或查询失败不能伪装成零分母。
- Enumeration 的 `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next` 仅保留 `terminal_exceptions` wire 兼容：省略、`null`、`[]` 可接受，任何非空数组都 fail-closed，preview 固定 `persisted=false` / `accepted_cells=0` / `coverage_to_submit=[]`。模型不能借 cloned snapshot 手写 blocked/not_applicable；终态只来自 guarded backend truth。`stage_worklist_next` 的 `limit` 仍是 cell cap（最大 200），Enumeration 同时最多返回 50 个 distinct exact-origin root，并给出 `root_count` / `matching_root_count` / `omitted_root_count`。
- `list_enumeration_web_roots` 是 enumeration 的只读入口：复用同一 coverage snapshot，只返回 EAS-confirmed exact web origins、pending/partial/terminal techniques、suggested tools 和 wrapper 工具边界；explicit URL 或 confirmed-open HTTP(S) service metadata 可物化 root，bare host / unknown TCP / closed port 一律不返回。它默认只返回 25 个 root、最多 50 个，并把 pending/error/partial root 排在 terminal root 前面；Enumerator 应把返回页当当前小 wave 处理，fresh outcomes 落库后刷新下一页，不能一次请求/处理整个 org。browser seed 只能调用 `enum_crawl_same_origin_urls`，不能暴露 raw `katana` 或 raw `pentest_run`；DIR completion 用 `route_probe_paths` page batch，优先 `batch_concurrency=4` 与 per-root runtime budget，不默认设置会制造 request-limited partial 的 `max_requests`，也不能用 `manage_targets` 改资产状态。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_executors
```
