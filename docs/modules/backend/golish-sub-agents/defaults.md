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
- `prober` prompt 是 `external_attack_surface` 的 active worker contract：优先 `list_attack_surface_seeds`，按 domain/ip/url/cidr 类型决定 liveness/port/service 动作；EAS 工具调用要 batch-first（`httpx` JSONL list/stdin/input_lines；`naabu -list {{input_file}}`、`masscan -iL {{input_file}}`、`nmap -iL {{input_file}}`、`whatweb --input-file={{input_file}}`、`gowitness file -f {{input_file}}` 均由 `pentest_run.input_lines` 自动写临时 list 文件），不要一资产一个前台 `pentest_run`；found coverage 由 DB truth 自动投影（targets / ports / fingerprints / technique_outcomes），不要手抄 found 矩阵；只为 DB 不能推导的 active negative / blocked / not_applicable 终态提交 coverage。Prober/Enumerator 都必须暴露 `query_target_data`，否则 coverage-gap repair mode 的“查现有目标/证据后窄补洞”会退化成扫全量。
- `enumerator` 的默认工具集包含 `stage_worklist_status` / `stage_worklist_next`，prompt 要求每个 normal/repair pass 先读 stage-local DB-truth worklist，只处理 `stage_worklist_next.items` 点名的 asset×technique cell；`list_enumeration_web_roots` / `query_target_data` 只作上下文和细节查询，不能替代 worklist plan。
- `enumerator` / `browser` 的 JS/API 路径要 browser-first：每个 alive web service 先跑 `browser_collect_js_api(crawl_mode="standard", ai_assist=true)` 做 lazy chunk / runtime XHR closure；如果 `closure_complete=false`、`recursive_queue_remaining>0`、`status=closure_partial|timeout_partial` 或 `ai_assist.recommended=true`，由模型选择一次同策略 bounded recipe 二次调用；recipe pass 后停止继续升级，转 `js_extract_apis` 对已保存 JS 做静态 endpoint/secret/config/rule signal 抽取，不用 shell/curl/katana 替代 closure signal。
- prober / enumerator prompt 的后台任务约束要强调：慢扫描只跑一次，提交前先用 `wait_for_background_jobs` 显式等待并读取完成 job 的 stdout/stderr tail；不要在后台 job 未完成时反复 submit 或重跑同一命令。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents defaults
```
