# 枚举阶段吞吐优化 实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现。每任务单独
> commit。诊断来源：本会话对 session `pentest-chat-1783070503216-1` 最后一次 run 的
> enumeration 阶段分析（run.log + subagent transcript）。
>
> **姊妹计划（勿重复）**：`docs/superpowers/plans/2026-07-03-enumeration-batch-katana.md`
> 已覆盖「三个内容采集工具批次多 target + katana 补充 + JS 轴终态自负」。本计划**不重复
> 批次化**，只补它没解决的三块：① worklist 不给全 URL 导致的串行 query_target_data；
> ② 长工具阻塞 → sub-agent 重启丢进度；③ 分母过大（alive-first 优先/wave/cap 预备）。
> 死资产排除（缩分母的一大杠杆）已在 commit `6cfdeaa2` 落地（skip_dead_assets +
> ongoing dead 标记），本计划将其作为既有前提，不再实现。

**目标：** 把 enumeration 从「18 分钟只完成 11% cell、被从头重启」优化到「一次 pass 内
稳定推进、长工具不拖垮 sub-agent、有限 worklist 优先覆盖高价值 live root」。

**架构：** 三条正交改动——(A) 让 `list_enumeration_web_roots` 直接回传可用的完整
`root_url`（scheme+port），消除逐 target 的 `query_target_data`；(B) 让长跑内容工具
（route_probe/browser/js_extract）不阻塞 sub-agent 的 turn 预算，避免超时重启丢进度；
(C) 让 enumeration worklist alive-first 排序并为后续 wave/cap 做预备，配合已落地的
死资产排除。

**技术栈：** Rust（`golish-agent-kit` tool_executors、`golish-agent-app` db_bridge、
`golish-sub-agents` registry/prompt、`golish-pentest-app` pentest_bridge）+ resource JSON/MD。

---

## 0. 诊断（带证据，全部来自最后一次 run）

最后一次 run 的 enumerator #1（subagent `enumerator-call_00_UDqLKwG5moEQ...`）时间线：

| 时段 | 动作 | 耗时 | 产出 |
|---|---|---|---|
| 09:45:45 | worklist + **20× `query_target_data`（逐个查）** | ~35s | 只为拼 `https://host:port` |
| 09:46:29 | `browser_collect_js_api`（20 URL） | 5m20s | JS 5/20、JSAPI 1/20 |
| 09:52:03 | `js_extract_apis`（20 URL） | 4m00s | JSAPI 3/20、param 1/20 |
| 09:56:35 | `route_probe_paths`（max_runtime_ms=300000） | ~7min **无结果事件** | sub-agent 未续、被从头重派 |

- **分母 = 93 web roots × 4 技术（JS/JSAPI/DIR/PARAM）= 372 cell**；18 分钟只 done 42（11%），
  DIR/PARAM 全 0。enumerator #1 transcript 末事件即 route_probe 的 request（无 result），
  主 agent 直到 10:03:46 才恢复并从头重派 enumerator #2。
- 错误极少（1 次 JSON repair + 1 次 empty one-shot），**不是「报错」问题，是吞吐 + 重启**。

### 0.1 根因 A：worklist 的 `root_url` 是裸 host，不含 scheme/port

`backend/crates/golish-agent-kit/src/tool_executors/security.rs:685`（`enumeration_web_roots_worklist`）
把每个 root 的 `root_url` 直接设成 `asset.value`（= 裸 host，如 `mss.moresec.cn`），**没有
scheme/port**。于是 enumerator 只能逐个 `query_target_data` 去查 http/https + 端口来拼完整
URL（20 次串行 LLM 往返）。而 `golish-agent-app/src/ai/db_bridge/recon.rs:57`
（`root_url_for`）/ `:77`（`derive_enumeration_web_roots`）**已有**从 `http_status`/`ports`/
`service` 推导 `scheme://host:port/` 的逻辑——worklist 没复用它。

### 0.2 根因 B：长跑内容工具串行阻塞 + 每次重派重做发现（**已勘误**）

> **勘误（读码后更正）**：本节初稿曾断言「route_probe 的 `max_runtime_ms=300000` 与
> enumerator `idle_timeout=300s` 同量级 → 跑满触发 idle 超时重启」。**读
> `golish-sub-agents/src/executor/response_parsing.rs:1557` `use_sub_agent_outer_tool_timeout`
> 后确认该假设错误**：`browser_collect_js_api` / `js_extract_apis` / `route_probe_paths`
> **被明确排除**在外层 `tokio::time::timeout` 之外（注释：这些 Rust bridge 工具自带进度块、
> 可合法跑过 LLM idle 预算，套外层 timeout 会 drop future 杀掉工具）。enumerator 又
> `timeout_secs=None`（无 overall cap）。**所以长工具不会被 sub-agent 超时杀掉**，也不是重启主因。

实际观测（enumerator #1 transcript 末事件 = route_probe 的 request，无 result；run.log 在
10:04:02 **戛然而止**）：最可能是**用户手动停了这次 run**，而非超时。真正拖慢吞吐的是：

- **串行阻塞的慢工具 + 巨大分母**：browser_collect（20 URL ~5min）→ js_extract（20 URL
  ~4min）→ route_probe（5min 预算）依次**同步**在 enumerator turn 内跑，一轮只覆盖前 20 /
  93 个 root。93 root 全跑完需数十分钟到数小时。
- **每次重派重做发现**：主 agent 每 fan 一个新 enumerator，就要重新 `stage_worklist_status`
  + `list_enumeration_web_roots` + （旧行为）20× `query_target_data`，重付启动成本。PR-A 去掉
  了其中最贵的 20× query_target_data。

→ 结论：B 不是超时 bug；吞吐由**分母大小（PR-C + 已落地死资产）× 每 root 工具耗时（批次多
target，见姊妹计划）× 每次重派的发现开销（PR-A）**共同决定。无需改超时/后台化来「防重启」。

### 0.3 根因 C：分母过大（单 org 93 web roots）

`moresec.cn` 一个 org 就 93 个 web root，多为同 app/同 real_ip 的 vhost 子域，低产
（browser JS 命中 5/20、JSAPI 1/20）。EAS 已有 `attack_surface_priority`
（`golish-app-core/src/domain/targets.rs`）与别名折叠（`eas_port_delegated_domain_values`），
但 enumeration 的 web root 集**还没有安全的 cap/wave 分母语义**。死资产排除已落地（commit
`6cfdeaa2`），是缩分母的第一杠杆；本计划当前只落 alive-first 排序 + cap helper 预备，不直接折叠同 IP vhost。

---

## 1. 文件清单（每档职责）

| 档案 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/tool_executors/security.rs` | 改 | `enumeration_web_roots_worklist` 每 root 补完整 `root_url`（scheme+port）+ `port`/`scheme` 字段；因 `agent-kit` 不依赖 `app-core`，URL 推导用本地小 adapter 镜像共享规则 |
| `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs` | 改 | 让 coverage snapshot 的 asset 携带 `http_status`/`ports`/`webserver`（worklist 推 URL 的输入）；若已带则仅核对 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs` | 改 | 支持后台执行模式（spawn background job + 立即返回 job_id），或把默认 `max_runtime_ms` 降到远低于 idle 预算 |
| `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs` | 改 | 视 B 的方案调 enumerator `idle_timeout`/`max_iterations`（若选「提预算」路线） |
| `backend/crates/golish-app-core/src/domain/targets.rs` | 改 | 新增 `web_root_url` / `rank_enumeration_web_roots` 纯函数 + 单测；当前只做 alive-first 排序/可选 cap helper，不折叠同 IP vhost |
| `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs` | 改 | 复用 `app-core::domain::targets::web_root_url`，避免 agent-app 私有 URL 推导继续漂移 |
| `resources/harness/stages/enumeration/methodology.md` | 改 | 明确「worklist 已给完整 URL，禁止逐个 query_target_data 拼 URL」+ 长工具后台化口径 |
| `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs` | 改 | `build_enumerator_prompt` 同步上述口径 |

---

## 2. PR-A · worklist 直接给完整 root_url（消除串行 query_target_data）

### Task A.1 · coverage snapshot 携带 web 元数据（TDD）

**档案**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

**步骤**：
1. 先读 `stage_asset_coverage_snapshot` 现有 asset 投影（`assets[].{target_id,value,target_type,
   coverage,...}`）。确认是否已带 `http_status`/`ports`/`webserver`；本 run 证据显示**未带**
   （否则 enumerator 不必逐个 query）。
2. 在 asset 投影里补 `http_status`（`Option<i32>`）、`ports`（`Vec<Value>`，直接透传
   `targets.ports`）、`webserver`（`String`）三个字段。数据来自 in-scope targets 读取处
   （与 `in_scope_typed_assets` 同源），不新增查询就能拿到则复用；否则在快照组装处 join。
3. 测试：给一个带 `http_status=200`/`ports=[{port:443,service:"https"}]` 的 target，断言
   snapshot 的对应 asset 含这三个字段。

**验证**：
```bash
cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail
```
**Commit**：`feat(enum): carry http_status/ports/webserver in coverage snapshot assets`

### Task A.2 · worklist 用元数据拼完整 root_url（TDD）

**档案**：`backend/crates/golish-agent-kit/src/tool_executors/security.rs`（`enumeration_web_roots_worklist` `:647`）

**步骤**：
1. 复用 `root_url_for` 的推导规则（scheme：service 含 https/ssl 或 port∈{443,8443,9443} →
   https，否则 http；port_suffix：http:80/https:443/无 port → 空，否则 `:port`）。因该函数在
   `golish-agent-app`（下游 crate），不能直接依赖——**在 `golish-app-core/src/domain/targets.rs`
   新增 `pub fn web_root_url(host, port, service) -> (String /*url*/, String /*scheme*/, Option<u16>)`**
   （把 `root_url_for` 上移到共享域），`golish-agent-app` 复用共享函数。`golish-agent-kit`
   保持不依赖 `app-core` 的层级边界，所以在 `security.rs` 保留一个小的本地 adapter：
   从 snapshot 的 `http_status`/`ports`/`webserver` 先挑 web port，再镜像 `web_root_url` 的
   scheme/port-suffix 规则；两侧用单测 pin 住，避免 silent drift。
2. `enumeration_web_roots_worklist` 里，对每个 asset：从 `http_status`/`ports`/`webserver`
   推 `(root_url, scheme, port)`；`root_url` 用推导值（含 scheme+port，末尾 `/`），并新增
   `"scheme"`/`"port"` 字段。asset 是 URL 类型（value 已带 scheme）则直接规范化。无任何 web
   信号 → 退回 `http://host/` 并标 `"needs_probe": true`。
3. 更新既有测试 `enumeration_web_roots_worklist_returns_live_root_contract`
   （`security.rs:1212`）：断言 `web_roots[0]["root_url"]` 现在是 `https://host/`（或带 port），
   不再是裸 host。

**验证**：
```bash
cd backend && cargo nextest run -p golish-agent-kit enumeration_web_roots --status-level fail
cd backend && cargo nextest run -p golish-app-core web_root_url --status-level fail
```
**Commit**：`feat(enum): worklist returns full scheme://host:port root_url (kill per-target probe)`

### Task A.3 · methodology + prompt 禁止逐个拼 URL

**档案**：`resources/harness/stages/enumeration/methodology.md`、
`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`（`build_enumerator_prompt`）

**步骤**：
1. methodology「Recommended sequence」加一句：**`list_enumeration_web_roots` 已返回每个 root 的
   完整 `root_url`（scheme+port），直接把这些 URL 批量喂给内容工具；禁止对每个 target 调
   `query_target_data` 只为拼 URL。** 仅当需要更深字段（如具体 API 路径）时才 drill-down。
2. `build_enumerator_prompt` 同步该口径（保留「worklist 是权威计划」「不 hand-write found」等约束）。
3. 测试：`golish-sub-agents/src/defaults/tests.rs` 已有 prompt 断言，补一条 `assert!(prompt
   .contains("root_url"))` 且不诱导 per-target query。

**验证**：
```bash
cd backend && cargo nextest run -p golish-sub-agents enumerator --status-level fail
```
**Commit**：`docs(enum): worklist gives full URLs; forbid per-target query_target_data`

---

## 3. PR-B · 长工具不拖垮 sub-agent（防重启丢进度）

> **先做诊断 Task B.0 确认根因，再择方案**——B 的确切修法取决于「sub-agent 到底为何在
> route_probe 后被从头重派」。不要跳过诊断直接改。

### Task B.0 · 确认重启根因（只读，出结论）

**步骤**：
1. 读 `backend/crates/golish-sub-agents/src/executor/inner.rs`（`process_llm_stream` +
   `idle_timeout` 分支 `:362`）与 `agentic_loop/tool_execution/direct/stage_run_call.rs`
   的 sub-agent 生命周期，确认：idle_timeout 是否只在 LLM streaming 期间计时（则长工具本身
   不触发 idle），还是 tool 执行也计入。
2. 读 `route_probe_paths.rs::execute` 确认它是否同步 await 到底、`max_runtime_ms` 语义
   （硬超时还是软预算）、超时后返回 Ok 还是 Err。
3. 结论三选一并记录：(a) idle 计时把长工具算 idle → 提预算或后台化；(b) 工具硬超时返 Err →
   sub-agent 误判失败 → 修返回语义；(c) max_iterations(40) 耗尽 → 调 iteration 预算 / 批次减少往返。

**验证**：把结论写进本文件 §5「诊断结论」小节（无代码改动）。
**Commit**：`docs(enum): record sub-agent restart root cause for route_probe`

### Task B.1 · 按 B.0 结论修（后台化优先）

**档案**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`
（必要时 `browser_collect_js_api.rs`/`js_extract_apis.rs`）

**步骤（方案「后台化」，若 B.0 结论支持）**：
1. 给 route_probe_paths 加 `run_in_background: bool`（默认沿用现状）。为 true 时：spawn 一个
   `golish_app_core::background_jobs` 作业跑探测循环、立即返回 `{ "background": true,
   "job_id": ... }`，落库仍在作业内完成（DIR outcome upsert 不变）。
2. enumerator 用 `route_probe_paths(run_in_background=true, ...)` + `wait_for_background_jobs`
   收口——与 EAS naabu/httpx 的后台批处理同构（参考 `bridge_config.rs` 的
   `maybe_store_background_batch_*`）。这样单个长探测不占 sub-agent 的同步 turn。
3. 若 B.0 结论是 (a) 且后台化成本高，**退路**：把 enumerator `idle_timeout` 提到 600s
   （`registry.rs`），并把 route_probe 默认 `max_runtime_ms` 降到 120000，使单次远小于预算。

**步骤（方案「预算对齐」，最小改动退路）**：
- 仅改 `registry.rs`：enumerator `.with_idle_timeout(600)`；并在 methodology/prompt 要求
  route_probe 传 `max_runtime_ms<=180000`。不动工具执行模型。

**验证**：
```bash
cd backend && cargo nextest run -p golish-pentest-app route_probe --status-level fail
cd backend && cargo check -p golish-sub-agents
```
**Commit**：`fix(enum): stop long route_probe from starving the enumerator turn`

---

## 4. PR-C · worklist 优先排序 + gate cap 预备

### Task C.1 · web root alive-first 优先排序纯函数（TDD）

**档案**：`backend/crates/golish-app-core/src/domain/targets.rs`

**步骤**：
1. 新增 `pub fn rank_enumeration_web_roots(roots: Vec<Target>, cap: Option<usize>) -> Vec<Target>`：
   - alive roots（`http_status IS NOT NULL`）优先，再按 `attack_surface_priority` 降序，最后按
     `value` 稳定排序；`cap` 截断（`None`=不截）。
   - **不折叠同 IP vhost**：两个 vhost 可能是不同 app，不能仅凭 `real_ip` 把它们从 enumeration
     分母里合并。真正改 gate 分母前必须先设计 next-wave/backlog 语义。
2. 单测：给同 real_ip 的两个 vhost + 未验证 root，断言同 IP vhost 都保留、未验证 root 下沉、
   cap=1 截断且高优先在前。

**验证**：
```bash
cd backend && cargo nextest run -p golish-app-core rank_enumeration_web_roots --status-level fail
```
**Commit**：`feat(enum): rank enumeration web roots (pure fn)`

### Task C.2 · worklist 排序：alive-first（安全子集，已做）

**档案**：`backend/crates/golish-agent-kit/src/tool_executors/security.rs`
（`enumeration_web_roots_worklist`）

**步骤（已实现）**：
1. 加 `enum_worklist_asset_priority(asset)`：有 `http_status` +100、有开放 `ports` +10。
2. `enumeration_web_roots_worklist` 在 `take(limit)` **之前** stable-sort 资产（alive/高价值
   在前），保证 limit 截断时保住值得枚举的 live root，且平手保持 created_at 原序。
3. 测试 `enumeration_worklist_orders_alive_roots_first_within_limit`：limit=1 时 alive root
   胜出、truncated=true。

> 这是**展示/worklist 排序**，不动 gate 分母语义 → 安全、可 ReadLints 验证。

### Task C.3 · gate 分母 cap（**已延后**，需 gate 测试协调）

**为何延后**：把 cap 应用到 **gate 覆盖率分母**（而非 worklist 展示）会改变 PASS 语义——
被 cap 掉的 root 必须走 `asset_wave_barrier` 的 next-wave backlog 机制（`stage_coverage.rs`
`is_next_wave_asset` / `NEXT_WAVE_PENDING`）才不会误 BLOCK。这是 `recon-gaps-followups.md §4`
标注的「execute.rs 分母注入共用触点」，需与 wave 计划对齐 + 有 gate PASS/BLOCK parity 测试才
能安全落地，且本会话无法跑 cargo/gate 测试验证。`rank_enumeration_web_roots`（C.1）已备为
canonical 排序，待做 C.3 时复用。**本会话不改 gate 分母。**

---

## 5. 诊断结论（Task B.0 已完成）

**B.0 结论：PR-B 不需要实现。** 读 `golish-sub-agents/src/executor/response_parsing.rs:1557`
（`use_sub_agent_outer_tool_timeout`）确认 `browser_collect_js_api`/`js_extract_apis`/
`route_probe_paths` **被明确排除**在外层 `tokio::time::timeout` 之外（否则会 drop future 杀掉
自带进度块的长工具）；enumerator `timeout_secs=None`（无 overall cap）。故长工具**不会**被
sub-agent 超时杀掉，「防重启」无 bug 可修。run.log 在 10:04:02 戛然而止 = 最可能用户手动停止。

→ **PR-B 撤销**（B.1 不做）。吞吐优化改由 PR-A（去 20× query_target_data）+ PR-C（alive-first worklist）+
姊妹计划的批次多 target 承担。若未来真观测到长工具被杀，再回来做后台化。

---

## 6. 最终收口

```bash
cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-app-core -p golish-pentest-app -p golish-sub-agents
just precommit   # 用户择机；本 epic 结束统一跑
```
1. 逐 crate `cargo check` 修编译；改动 crate `cargo nextest`。
2. 更新 `docs/modules/**` 受影响卡（tool_executors / pentest_bridge / task_orchestrator）+
   `agent-progress.md` + `feature_list.json`。
3. 与姊妹计划 `2026-07-03-enumeration-batch-katana.md` 的落地顺序：**先 batch-katana（批次多
   target）再本计划 PR-A（全 URL）**——批次工具的入参就是 worklist 的 `root_url` 数组，全 URL
   落地后批次调用才不需 per-target 拼装。PR-B/PR-C 与二者正交，可任意穿插。

---

## 7. 规格覆盖自检

- 根因 A（串行 query_target_data）→ PR-A（A.1 快照带元数据、A.2 worklist 拼全 URL、A.3
  prompt 禁止逐个拼）。✅
- 根因 B（长工具重启丢进度）→ PR-B（B.0 诊断 + B.1 后台化/预算对齐）。✅
- 根因 C（分母过大）→ PR-C（alive-first 优先/后续 cap 预备；gate 分母 cap 延后）+ 已落地的死资产排除（commit `6cfdeaa2`）。✅
- 批次多 target / katana / JS 终态自负 → 姊妹计划，不在此重复。✅
- 不变量：I5（Target.ts 由 ts-rs 生成勿手改；本计划不改前端类型）、I8（cap 掉的 root 记
  backlog 非「已检查为空」）、I10（spec 新字段 additive 默认关）。
