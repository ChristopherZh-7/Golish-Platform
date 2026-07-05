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
- `prober` prompt 是 `external_attack_surface` 的 active worker contract：优先 `list_attack_surface_seeds`，按 domain/ip/url/cidr 类型决定 liveness/port/service 动作；EAS 工具调用走 backend-owned wrappers（`eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services`），不要让模型手写 `httpx` / `naabu` / `masscan` / `nmap` / `pentest_run` 参数。wrapper 自己做 batch/list-file recipe、target 类型校验和 output-store/evidence 解析衔接；found coverage 由 DB truth 自动投影（targets / ports / fingerprints / technique_outcomes），不要手抄 found 矩阵；只为 DB 不能推导的 active negative / blocked / not_applicable 终态提交 coverage。Prober/Enumerator/Vuln Scanner 都必须暴露 `query_target_data`，否则 coverage-gap repair mode 的“查现有目标/证据后窄补洞”会退化成扫全量。
- `enumerator` 的默认工具集包含 `stage_worklist_status` / `stage_worklist_next`，prompt 要求每个 normal/repair pass 先读 stage-local DB-truth worklist，只处理 `stage_worklist_next.items` 点名的 asset×technique cell；`list_enumeration_web_roots` / `query_target_data` 只作上下文和细节查询，不能替代 worklist plan。
- `enumerator` / `browser` 的 JS/API 路径要 browser-first：每个 alive web service 先跑 `browser_collect_js_api(crawl_mode="standard", ai_assist=true)` 做 lazy chunk / runtime XHR closure；如果 `closure_complete=false`、`recursive_queue_remaining>0`、`status=closure_partial|timeout_partial` 或 `ai_assist.recommended=true`，由模型选择一次同策略 bounded recipe 二次调用；recipe pass 后停止继续升级，转 `js_extract_apis` 对已保存 JS 做静态 endpoint/secret/config/rule signal 抽取，不用 shell/curl/katana 替代 closure signal。
- `enumerator` 的 DIR 路径必须用 `route_probe_paths(targets=[...])` 且显式传前台预算（默认提示 `max_runtime_ms=60000`、`max_requests=2000`）：`timeout_partial` / `request_limited_partial` 不是无限等待理由，先刷新 `stage_worklist_status` / `check_stage_asset_coverage`，只有 DIR 仍 pending/error 或有新 DB seeds 时才补同一 root。
- Enumerator 默认工具集不再暴露 `pentest_run` / `pentest_list_tools`；katana supplement 只能通过 `enum_crawl_same_origin_urls(target_urls=[...])` 触发。该 wrapper 只补 same-origin/current-org target 的 `api_endpoints(source='crawler')`；第三方外链属于 crawler context，不能通过 prompt 或后端 output_store 自动 promotion 成当前 org 的新 target。
- `vuln_scanner` 是 `vuln_triage` 的默认 stage specialist：工具集只暴露 `stage_worklist_status` / `stage_worklist_next` / `vuln_run_formulaic_sweep` / `wait_for_background_jobs` / `check_job` / `kill_job` / `list_recent_evidence` / `check_stage_asset_coverage` / `query_target_data` / `submit_stage_deliverable` 等收口工具，不暴露 `pentest_run` / `pentest_list_tools`。prompt 要求按 worklist 的 asset×technique gap 调 `vuln_run_formulaic_sweep(targets=[...], techniques=[...])`，底层 nuclei/sqlmap/wpscan recipe 由后端 wrapper 固定并落 `technique_outcomes`。
- prober / enumerator / vuln_scanner prompt 的后台任务约束要强调：慢扫描只跑一次，提交前先用 `wait_for_background_jobs` 显式等待并读取完成 job 的 stdout/stderr tail；不要在后台 job 未完成时反复 submit 或重跑同一命令。
- Orchestrator/planner 的通用“安全任务用 pentester”规则必须让位于 active harness stage 的 `stage_run` 指令：处在 specialist stage 时，primary agent 不应直接调用 `sub_agent_pentester` 来补洞，而应调用 `stage_run`，让 org fan-out、stage worklist 和 gate recovery loop 接管。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents defaults
```
