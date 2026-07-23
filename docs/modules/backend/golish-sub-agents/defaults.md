# golish-sub-agents / defaults

> **一句话职责**：默认 sub-agent 定义——`create_default_sub_agents`(_from_registry) 装配预配置 sub-agent；`prompts` 持硬编码 `build_*_prompt` + `WORKER_PROMPT_TEMPLATE`（作为模板驱动 registry 版本的 fallback）。

- **类型**：目录模块（属于 crate [`golish-sub-agents`](../golish-sub-agents.md)）
- **路径**：`backend/crates/golish-sub-agents/src/defaults/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改默认 sub-agent 集（worker 等）的定义或硬编码 prompt fallback 时
- 改模板驱动版本（`prompts/*.tera` + DB override）与硬编码 fallback 的关系时

## 职责

`builder` 公开构造器 `create_default_sub_agents` / `create_default_sub_agents_from_registry` 装配 `SubAgentDefinition`；`prompts` 持每个硬编码 `build_*_prompt` + `WORKER_PROMPT_TEMPLATE` 常量，作为模板驱动 registry 版本（优先 `prompts/*.tera` + DB override）的 fallback。

## 公开接口

| 符号 | 说明 |
|---|---|
| `create_default_sub_agents` / `create_default_sub_agents_from_registry` | 默认集构造器 |
| `WORKER_PROMPT_TEMPLATE` | worker prompt 模板常量 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export builder + 常量 |
| `builder/` | `SubAgentDefinition` 装配 |
| `prompts/` | 硬编码 `build_*_prompt` + 模板常量 |

## 依赖

- crate 内 `definition::SubAgentDefinition`、`prompt_registry`；tera（模板）

## 注意事项 / 坑

- 硬编码 prompts 是 **fallback**：registry 优先用 `prompts/*.tera` + DB override；改默认行为先确认走的是哪条路径。
- `from_registry` 版本会合并 DB/模板 override；纯 `create_default_sub_agents` 是无 registry 的基线。
- `prober` prompt 是 `external_attack_surface` 的 active worker contract：优先 `list_attack_surface_seeds`，按 domain/ip/url/cidr 类型决定 liveness/port/service 动作；EAS 工具调用走 backend-owned wrappers（`eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services` / `eas_fingerprint_web_stack`），不要让模型手写 `httpx` / `naabu` / `masscan` / `nmap` / `whatweb` / `pentest_run` 参数。`eas_discover_ports`在可调initial yield结束时可返回同一managed `job_id`，进程不会按elapsed自动停止，完成后由server-owned typed reconciler落guarded business/evidence/outcome；transport yield不进入EAS recipe/coverage/Gate。`eas_fingerprint_services` 一次接收 concrete-IP targets，通常不传 ports，更不能传 process timeout；backend 用 exact DB pending ports 自行 chunk/concurrency/slow-IP isolation/一次 recovery，partial 后模型只刷新 worklist/coverage，不能调大预算或原样重放整批。`eas_fingerprint_web_stack` 只在 HTTP(S) 已确认后按每个 web origin 做 WhatWeb web-stack enrichment，并必须优先原样复制 worklist `details.recommended_args.target_urls`，或用同 item 的 `target_id + details.missing_origins` 组成对象，禁止从 443 等端口猜测/重写 scheme。它不能替代非 Web open port 的 nmap service/version，也不能用同 IP 上一个域名的 WhatWeb 结果代表另一个 Host/SNI。found coverage 由 DB truth 自动投影（targets / ports / technique_outcomes），不要手抄 found 矩阵；只为 DB 不能推导的 active negative / blocked / not_applicable 终态提交 coverage。Prober/Enumerator/Vuln Scanner 都必须暴露 `query_target_data`；任何拥有raw shell/pentest权限的角色还必须同时有`check_job`/`kill_job`，否则coverage repair和进程liveness判断都会失去权威入口。
- Prober 的 PORT closure 必须使用 `eas_discover_ports(..., scan_profile="full")`；不能再传 scanner/top_ports/ports/rate/timeout。`quick`/`standard` 只做 discovery 且永远保持 PORT partial，返回的 `next_action` 是下一步来源。Full 的固定安全预算只允许最多四个展开 IPv4 地址（CIDR `/30` 或更窄）或 exact IPv6 `/128`；更大范围不能由模型静默拆分、缩窄或自报 checked-empty，wrapper 会零网络写入 evidence-backed policy-blocked LIVENESS/PORT。positive discovery 的 guarded child IP 才做 supplemental full。
- Recon/Prober 对剩余 honest terminal 使用同一个 typed `terminal_exceptions` 数组做 preflight，并把返回的 `coverage_to_submit` 原样交付；不得借预演新增 asset/technique。`submit_stage_deliverable=accepted` 后立即停止，禁止刷新 worklist、改 target status、重跑 provider/wrapper 或再次提交。Recon 和 Prober 都没有 `manage_targets`：Recon 只能由 backend 在 trusted domain/wildcard root 下落 strict child，不能用 profile/provider 发明 root/IP；Prober 的 CIDR 子 IP 由 guarded output-store 按 in-range provenance 创建。CIDR 行只闭 LIVENESS/PORT，子 IP 在 supplemental wave 闭 SERVICE/WEB；wildcard 行 EAS/Enumeration 全 N/A。
- `enumerator` 的默认工具集包含 `stage_worklist_status` / `stage_worklist_next`，prompt 要求每个 normal/repair pass 先读 stage-local current-run outcome worklist，并用 `prefer=["pending","error","partial"]` 只处理 `items` 点名的 exact-origin asset×technique cell；`list_enumeration_web_roots` / `query_target_data` 只作上下文和细节查询，不能替代 worklist plan。四轴是 JS/JSAPI/DIR/PARAM，不能漏掉 GOLISH-ENUM-JS。
- Enumerator 每页先用 `enum_preflight_web_origins` 对 distinct `{target_id,target_url}` 做 trusted transport preflight；只把 `reachable_origins`/`pending_origins` 喂给 producers，preflight `blocked_origins` 已由 evidence-backed DB truth 闭合四轴。`terminal_exceptions` 只能省略或 `[]`，最终 deliverable 固定 `coverage: []`。一页仍最多 200 cells / 50 distinct roots。
- `enumerator` / `browser` 的 JS/API 路径要 deterministic browser-first：每个 reachable web service 先跑 `browser_collect_js_api(crawl_mode="standard", ai_assist=false)`，再跑 `js_extract_apis(ai=false)`；只按确定性 closure diagnostics 做 bounded recovery，不把 Enumeration 变成 AI triage loop。普通 partial/error 始终非终态；但 guarded checkpoint resume 后，重复的 collection-blocking failure 可由 browser 持久化 `enumeration_collection_recovery_exhausted` evidence，并将 JS/JSAPI/PARAM 三轴发布为 blocked。遇到 `blocked + recovery_exhausted=true` 不再重试；API-body-only、same-invocation duplicate 或 persistence failure 仍不得 terminalize。
- `enumerator` 的 DIR 路径必须用 `route_probe_paths(targets=[...])`；completion run 省略 `max_runtime_ms` 与 `max_requests`，只可调 bounded batch concurrency。`timeout_partial` / `request_limited_partial` 即使已保留 sampled rows 也仍是非终态；route 工具用同 run/session/operation/stage/owner/origin/plan 的 durable v8 cursor 保存 network queue、pending business write 和 terminal publication。第一 network failure 保留 pending，相同 fingerprint 稳定 2 次或累计 3 次才 exhaust；队列闭合且无其他 incompleteness 时才可用 `dir_probe_recovery_exhausted` 将 DIR 发布为 blocked。business-write/terminal-publication breaker 只保持 partial：当 `automatic_retry_allowed=false` 且有 `manual_repair_reason` 时停止普通 error/queue retry，修复后只对该 root 使用提示中的 `retry_exhausted_*` flag。累计计数不占用下一次 invocation-local budget。只有主动接受非终态诊断采样时才显式设置 request limit。
- Enumerator 默认工具集不再暴露 `pentest_run` / `pentest_list_tools`；katana supplement 只能通过 `enum_crawl_same_origin_urls(target_urls=[...])` 触发。该 wrapper 只补 same-origin/current-org target 的 `api_endpoints(source='crawler')`；第三方外链属于 crawler context，不能通过 prompt 或后端 output_store 自动 promotion 成当前 org 的新 target。
- `vuln_scanner` 是 `vuln_triage` 的默认 stage specialist：工具集只暴露 `stage_worklist_status` / `stage_worklist_next` / `vuln_nuclei_general` / `vuln_nuclei_fingerprint_targeted` / `list_recent_evidence` / `check_stage_asset_coverage` / `query_target_data` / `submit_stage_deliverable` 等收口工具，不暴露 raw `nuclei` / `pentest_run` / `pentest_list_tools` / `record_finding`。九个 WSTG gap 走 general wrapper，`GOLISH-NDAY` 只走 fingerprint-targeted wrapper；每次从 worklist/query 取 singular `target_id + exact target_url` 并显式传 `techniques[]`。两个 wrapper 都 foreground、自落 evidence/outcome，Vuln Scanner 提交 `findings=[]`、`coverage=[]`，也不再暴露 `wait_for_background_jobs` / `check_job` / `kill_job`。
- 后台任务约束按wrapper contract区分：Prober的`eas_discover_ports`使用managed handle，AI按进程存活、累计输出和last-output activity决定继续等或显式kill；其它三个EAS wrapper与Vuln Scanner两个Nuclei wrapper仍强制foreground。bounded wait只结束观察，不杀后台进程。
- Orchestrator/planner 的通用“安全任务用 pentester”规则必须让位于 active harness stage 的 `stage_run` 指令：处在 specialist stage 时，primary agent 不应直接调用 `sub_agent_pentester` 来补洞，而应调用 `stage_run`，让 org fan-out、stage worklist 和 gate recovery loop 接管。
- `attack_analyst` 是 reasoning-only Candidate specialist；prompt 区分 initial `vuln_triage_handoff` 与 follow-on `fact_delta_consolidation`，zero-input org 必须 terminal 且不启动 placeholder worker，提交后只等待 durable review/resume，绝不能决定或打开下一 Wave。manifest 超过 20 项时第一响应必须直接用 `candidate_decision_groups`，通过 exact keys、canonical kind prefixes 或 exact `nuclei_template_ids` 在 server-frozen manifest 内选择；Candidate Nuclei group 一次只选一个 template，重复 target+technique+hypothesis 保留一个并把其余项终结为 `duplicate_candidate`。`candidate_verifier` 工具面精确为 ordinal wrapper、recent evidence、Attempt submit，无 raw runner/Finding writer/scanner/background control/nested delegation；FactDelta proposal 必须绑定 exact canonical ref/version/hash + evidence，但 verifier 不接受/消费 proposal，也不决定 `opened_next_wave|closed_no_delta|exhausted`。hardcoded 与 registry builder 必须一致。
- `post_exploit_operator` 是四个 Post-Exploit V2 stage 共用的 per-org Worker specialist；默认工具面只有四个 `post_exploit_*` wrapper、read-only target/evidence 与 deliverable submit。stage whitelist 每次只显示当前 stage 的一个 wrapper；无 `pentest_run`/shell/Finding writer/background control/nested delegation。prepared/approval-required 不得表述为 executed，executor unavailable/outcome unknown 不得绕过或盲重放。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents defaults
```
