# Enumeration 批次化 + 每资产终态收口（含 katana 补充）

> Enumeration 的 error/partial 终态语义、Web Origin 分母以及 raw-IP /
> DNS-only `not_applicable` 注入已由
> `docs/design/2026-07-10-enumeration-origin-terminal-closeout.md` 收敛：无法物化
> `scheme://host:port` 的 host 直接不进入 Enumeration 分母，不再合成
> rootless `not_applicable` cell。本文其余 batch/工具分工设计继续有效。

> 状态：Accepted（用户 2026-07-03 拍板：分步骤、不要包装成一个大工具、批次整资产集一次跑、katana 作补充、先实现后修 build）。
> 关联：`docs/design/2026-07-01-enumeration-four-axis-and-ip-web.md`（四轴 + IP-web）、
> `docs/design/2026-06-26-enumeration-deliverables-and-flow.md`、
> `docs/design/2026-06-25-enumeration-js-api-collection.md`。

## 1. 背景与问题

最近多轮 moresec.cn 实跑，enumeration 阶段反复 `submit → needs_fix → retry 3/3` 用尽 → org `blocked`。诊断出三条叠加根因，加一个效率问题：

- **根因 1 · 分母灌爆**：EAS/httpx 给一批裸 IP 设了 `http_status`，`enum_ip_web_coverage` 把这些 web-capable IP 全拉进枚举分母，逐个要 4 格终态。其中不少是共享/CDN/仅 DNS 的 IP，浏览器抓不到 JS、route probe 空。
- **根因 2 · JS 轴无法落 `checked_empty`（致命）**：`GOLISH-ENUM-JS` 的终态只由 runtime hook `record_enumeration_bridge_evidence`（`golish-agent-runtime/.../direct/mod.rs`）投影，而该 hook **只在主代理 direct 工具路径跑**。enumerator 子代理的 `post_tool_result_hook`（`sub_agent_call.rs`）只挂了 `record_recon_passive_evidence`。于是子代理跑 `browser_collect_js_api` 抓到 0 个 JS 时，`GOLISH-ENUM-JS` 永远停在 `not_attempted` → gate 数学上不可能通过。DB 实测该技术 0 行印证。
- **根因 3 · repair 死锁**：coverage-gap submit-repair 白名单把 `stage_worklist_status/next`、`list_recent_evidence` 挡掉。**已在前几轮修复**（`executor_types.rs` + `response_parsing.rs`）。
- **效率问题**：三个内容采集工具（`browser_collect_js_api` / `js_extract_apis` / `route_probe_paths`）都是**单 target**，LLM 只能一个个 web root 发工具调用，几十次往返，又慢又烧 token，还容易撞 repair loop。

### 1.1 现状核对（2026-07-03，行号可能漂移）

已经落库正确的（**根因 2 大部分已修**）：
- `route_probe_paths.rs::upsert_dir_outcome` → `GOLISH-ENUM-DIR` found/empty/error + `append_bridge_evidence` + `technique_outcomes::upsert`。
- `js_extract_apis.rs::upsert_param_outcome`（PARAM）+ `upsert_jsapi_outcome`（JSAPI），均带 evidence。
- `browser_collect_js_api.rs::upsert_jsapi_outcome`（JSAPI）+ evidence。
- 分母收窄：`org_gate.rs::enumeration_eas_live_web_worklist` + `enum_ip_web_coverage` 已把分母收到「EAS liveness domain + web-capable IP」。
- EAS 有 `eas_service_not_applicable_assets`（仅开 53/无服务面 → `not_applicable`），但 **enumeration 无对应机制**。

**唯一真正的落库缺口**：`GOLISH-ENUM-JS` 的 found/empty/error 终态只由 runtime hook 落，子代理路径丢失。这就是「跑了几十次 browser_collect_js_api 仍 JS=0 行」的真因。

## 2. 目标 / 非目标

**目标**
1. **P0（根治死循环）**：把 `GOLISH-ENUM-JS` 终态 upsert 移进 `browser_collect_js_api` 工具自身（与 JSAPI/PARAM/DIR 同款），使子代理路径也能落 JS 的 found/`checked_empty`。移除 runtime hook 的 JS 投影，避免重复。
2. **P1（效率 · 批次，分步骤）**：三个内容采集工具支持**批次多 target**（可选数组入参），内部循环复用单 target 逻辑并聚合结果，每 target 各自落终态。保留单 target 入参向后兼容。**不新增大一统工具**——每个工具仍是一个独立步骤。
3. **P2（katana 补充 · 合并去重）**：katana 通过 `pentest_run(tool_name=katana, args=-list <urls>)` 对整份 web root 清单批次跑，输出 URL 落 `api_endpoints(source='crawler')`（已有 `output_store/endpoints.rs`）。JSAPI/PARAM 的 DB 真值天然合并 browser + js_extract + katana 三源（`api_endpoints` 去重靠 `(target_id, url, method)` 唯一索引）。prompt/methodology 引导 katana 作补充语料，不作主体。
4. **P3（分母收窄 · 可选）**：给 enumeration 补一个确定性 `not_applicable`——web-capable 但内容探测确证无果的 IP 归 `not_applicable`，降噪。优先级最低（批次落 `checked_empty` 已能关闭分母）。

**非目标**
- 不改 EAS/target_intel 的 coverage 判定；不动 gate coverage 矩阵核心算法（`rule_engine.rs::coverage_complete`）。
- 不引入 ffuf/gobuster 等外部目录工具（enumeration 仍禁用）。
- 不改 `api_endpoints` / `js_analysis_results` / `directory_entries` schema。

## 3. 方案详解

### 3.1 P0 · JS 终态工具自负（根治）

在 `browser_collect_js_api.rs` 新增 `record_js_outcome`（镜像 `record_jsapi_outcome`）：
- 计算 `js_outcome`：`persisted_js_rows > 0` → `found`；否则看 `status`（timeout/error/`js_persist_errors` 非空）→ `error`，否则 `empty`（跑了→无 JS = checked-empty，I8）。
- `append_bridge_evidence`（`kind="js_collection"`, `technique=TECH_ENUM_JS`）+ `technique_outcomes::upsert`。
- 在 `execute_single` 结尾（record_jsapi_outcome 旁）调用。

在 `golish-agent-runtime/.../direct/mod.rs`：
- 删除 `record_enumeration_bridge_evidence` 对 `browser_collect_js_api` 的 `GOLISH-ENUM-JS` 投影（现在工具自负）。整个 `enumeration_evidence_projections` 目前只投影 browser→JS 一种，删空后连同 helper 一并移除（保留 `record_recon_passive_evidence` 不动）。
- 影响面：主代理 direct 路径原本靠此 hook 落 JS；改为工具自负后主/子路径一致。避免「工具 upsert + hook upsert」重复。

### 3.2 P1 · 批次多 target（分步骤）

每个工具 `execute()` 头部解析批次参数；给了数组就循环调用抽出来的 `execute_single(...)`，聚合成 `{ batch: true, results: [...], summary: {...} }`；否则走单 target（保留旧 schema）。

| 工具 | 单 target 参数 | 批次参数 | 每 target 落的技术 |
|---|---|---|---|
| `browser_collect_js_api` | `target_url` | `target_urls: string[]` | JS + JSAPI |
| `js_extract_apis` | `target_url` | `target_urls: string[]` | JSAPI + PARAM |
| `route_probe_paths` | `target_id`+`base_url` | `targets: {target_id, base_url}[]` | DIR |

约束：
- 批次上限（如 50）防止一次调用跑爆；空批次、超限、畸形项或重复 exact-origin
  整批拒绝，避免调用方误以为被静默截掉的 target 已检查。
- 批次内单 target 失败不中断整批（收集 per-target error，继续下一个）。
- 每 target 仍各自 `append_bridge_evidence` + `technique_outcomes::upsert`（复用 execute_single 内已有逻辑）。
- 聚合结果保留每 target 的 `_evidence_id` / outcome，方便子代理引用与 gate 交叉核对。

2026-07-11 transport 补充：`js_extract_apis` 的 single-target 结果仍保留完整
`endpoints` / candidates / rule matches / AI diagnostics；batch 继续保留
`results[].result` 层级，但内层改为 `bounded_batch_summary_v1`。每 root 摘要硬上限
8 KiB，保留 target/status/completion、JSAPI/PARAM outcomes 与 persisted flags、全部
计数、partial/retry 原因、小样本、capture manifest 与 DB 表引用；完整 endpoint / HAE /
rule / AI dialogue 数组不再复制进 batch。50-root 合成响应测试上限 512 KiB，低于
transcript JSONL 单条 1 MiB 限制。sub-agent 的 model-visible compactor 必须识别
`batch=true`，逐 root 使用 `endpoints_total` 等真实计数，禁止把 sample 长度或顶层缺失
`endpoints` 解释成“没有发现 endpoint”。

### 3.3 P2 · katana 合并去重

- katana 已在 `resources/toolsconfig/katana.json` 注册（`category=recon`, `subcategory=crawler`, 支持 `-list`），enumeration spec `allowed_tool_types` 含 `recon/crawler`。
- katana 走 `pentest_run(tool_name="katana", args="-list <file> -jc -silent ...")`，输出 URL 经 `output_store` 的 `endpoint_add` → `endpoints.rs::store_endpoint` 落 `api_endpoints(source='crawler')`。
- 合并去重天然发生：`api_endpoints` 唯一索引 `(target_id, url, method)`；JSAPI 真值 `source IN ('js_analysis','crawler')`、PARAM 真值 `params` 非空——browser / js_extract / katana 三源写同一张表，coverage 自动并集去重。
- prompt/methodology 增补：批次 browser → 批次 js_extract → 一次 katana `-list` 补充 → 批次 route_probe 的推荐顺序，并强调 katana 是**补充 URL 语料**、不替代 browser 闭包。

### 3.4 P3 · enumeration not_applicable（已实现 2026-07-03）

- 数据源**复用** EAS 已有的 `eas_service_not_applicable_assets`（SQL = `only_dns_port_without_service_surface`：in-scope IP/CIDR 且端口真值证明只开 DNS/53、无任何服务/版本/webserver/technology 面）。不新增 DB 查询或 trait method——同一批「只开 53 无 web 面」的 IP，对 EAS 是 SERVICE not_applicable，对 enumeration 是 JS/DIR/PARAM/JSAPI 四轴 not_applicable，语义一致。
- 注入路径与 EAS 对称：`org_gate.rs`（stage-close per-org gate）与 `harness_submit_tool.rs::gate_context`（submit 预检）的 `not_applicable_coverage` 增 enumeration 分支，把这些 IP × 四个 ENUM 技术塞进 `GateContext.not_applicable_coverage`；`rule_engine::coverage_complete` 的 `context_not_applicable_ok`（已有）据此把 cell 终态化，无需 agent 自报。
- UI/worklist 同步：`stage_coverage.rs` 新增 `apply_enum_content_not_applicable`，对这些 IP 把仍 pending 的 ENUM 轴改 not_applicable + note，保证读模型与 gate 一致（否则 worklist 仍把它们列为 pending）。
- 效果与边界：一个 IP 若从未进入 web-capable 分母（无 http_status），注入是安全 no-op；对「有残留/陈旧 http_status 但端口真相只开 53」的矛盾 IP 提供确定性降噪。批次落 `checked_empty`（P0）已能关闭大多数分母，本项是叠加的确定性降噪，不改变「真有 web 服务的 IP 仍需枚举」的语义。

## 4. 落库契约（不变量）

- I8：「已检查为空」≠「未检查」。批次每 target 必须落一个终态：`found`（DB 业务表有行，自动投影）或 `checked_empty`（跑了→空）或 `blocked`/`not_applicable`（带 note）。
- `found` 永远由 DB 业务表真值投影（`directory_entries` / `api_endpoints` / `js_analysis_results`），工具**不**手写 found 格；工具 upsert 的 provenance 行主要承载 empty/error/blocked/not_applicable。
- evidence 走 `append_bridge_evidence`（真 `audit_role='evidence'` 行，可被 `list_recent_evidence` 查、被 gate 交叉核对）。
- 批次不改变单 target 的落库语义，只是把 N 次调用合并为 1 次。

## 5. 影响面

- Rust：`golish-pentest-app`（三个 bridge 工具 + evidence.rs）、`golish-agent-runtime`（direct/mod.rs 删 JS hook）、`golish-db`（P3 可选新增 SQL）。
- Resource：`resources/harness/stages/enumeration/{methodology.md,spec.json}`。
- Prompt：`golish-sub-agents/.../execution_planning.rs`（enumerator prompt）。
- 前端：无（coverage 读模型 `stage_coverage.rs` 已按 DB 真值，四轴已就绪）。

## 6. 风险与回滚

- **重复 evidence**：P0 移 hook 时须同时删 runtime 投影，否则主代理路径 JS 双写。回滚：恢复 hook + 撤工具自负。
- **批次超时**：一次批次跑几十个 web root 可能整体超时。用批次上限 + per-target 硬超时 + per-target 失败隔离缓解。
- **repair 模式 target 比对**：`coverage_gap_direct_tool_target_block_reason` 只看单 `target_url`/`base_url`。批次参数在 repair 模式需逐个对照 `coverage_gap_actions`（否则放行不安全）。实现时在 repair 模式对批次数组逐项校验。
- 每个 PR 可独立回滚：P0（JS 自负）/ P1（批次）/ P2（katana prompt）/ P3（not_applicable）互不依赖。

## 7. 验证

- 单测：`browser_collect_js_api` JS outcome（found/empty/error）判定；三工具批次解析与聚合；route_probe 批次 repair 模式逐项校验。
- 集成：`cargo nextest -p golish-pentest-app`、`-p golish-agent-runtime`、`-p golish-db`。
- 实跑：重启 app 续跑 moresec enumeration，`run_tree.py --db` 确认 `GOLISH-ENUM-JS` 有 empty/found 行、批次一次调用覆盖整资产集、gate 能通过。

> 用户指示：本轮先实现全部，中途不跑 precommit / 大测试，最后统一修 build。
