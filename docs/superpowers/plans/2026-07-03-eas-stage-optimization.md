# EAS 阶段优化 / 报错根治实现计划

> Date: 2026-07-03
> Status: P0/P1/P2/P3/P4 已全部落地（本会话按用户指令直接落地，不跑 commit / 不跑耗时验证；仅 ReadLints + spec.json 解析验证）。收口待用户跑 `just precommit`。
> Author: BajieAsk-agent-5（全栈工程师会话）
> 触发：用户在最后一次 pentest run（session `pentest-chat-1783070503216-1`,
> workspace `/Users/christopherzheng/golish-platform/Test1`）观察到 EAS
> (`external_attack_surface`) 阶段"一直报错"，要求根治 + 补齐逻辑缺口。

> **给后续 session 的话**：本文件是断点续传的 system-of-record。若本会话中断，
> 照"## 4. 执行清单"的勾选状态接着做即可。P1 死资产标记有一份更细的既有 plan
> `docs/superpowers/plans/2026-07-02-dead-asset-liveness-state.md`（Task 1.1~3.2
> 逐行代码/SQL 都写好了），P1 直接照那份执行。

---

## 1. 诊断（带证据，全部来自最后一次 run 的 backend.log + run.log）

最后一次 run 时间窗：2026-07-03 09:24:01 ~ 10:04:02 UTC（本地 17:24~18:04）。
按 backend.log 全量出现次数排名，"一直报错"实为三类不同来源叠加：

| 排名 | 现象 | 全量次数 | 最后一次 run | 性质 | 层面 |
|---|---|---|---|---|---|
| 1 | `Template 'recon'/'prober'/'enumerator' not found` | 366 | 12 | 代码缺陷（非致命，fallback 有效） | golish-sub-agents |
| 2 | `invalid input value for enum agent_type: "recon"/"prober"` | 105 | 3 | 代码/迁移缺陷（tracking 写入失败） | golish-db + tracking_bridge |
| 3 | deepseek `SSE error`（`AddrNotAvailable` / `tls handshake eof`） | 19 | 1 | 运行环境（网络 + flash 弱模型） | 非代码 |

伴生：`JSON parse failed→repair`（弱模型坏 JSON）、`LLM returned empty
one-shot`（弱模型空回应）、`BLOCKED by submit repair`（这是**正常** harness
行为，非错误）。

### 1.1 根因一：Template not found（366 次）

- `golish-sub-agents/src/defaults/builder/registry.rs` 的
  `create_default_sub_agents_from_registry` 对 `recon`/`prober`/`enumerator`
  调 `tmpl_or_fallback!("recon", build_recon_prompt())`。
- 但 `golish-sub-agents/src/prompt_registry.rs` 的 `TEMPLATES` 静态数组**没有**
  注册这三个，`prompts/` 下也没有对应 `.tera` 文件（有 `analyzer.tera` /
  `worker.tera` 等，就是没有 recon/prober/enumerator）。
- 于是每次构建子 agent，render 这三个都失败 → `tracing::warn!` → fallback 到
  硬编码 `build_recon_prompt()` 等。**功能正常**（fallback 生效），但每次刷 warn。

### 1.2 根因二：agent_type enum 缺值（105 次）

- DB 枚举 `agent_type`（`migrations/20260408000001_initial.sql` 定义 +
  `20260412000002_extend_agent_type.sql` 扩展）当前值：
  `primary, pentester, coder, searcher, memorist, reporter, adviser,
  reflector, enricher, installer, summarizer, assistant, analyzer, explorer,
  researcher, executor, js_harvester, js_analyzer, worker, planner`。
- **缺少** `recon, prober, enumerator, browser, refiner, orchestrator` 等实际
  子 agent id。
- `golish-agent-app/src/ai/tracking_bridge/records.rs` 的
  `record_agent_call_impl`（`agent_logs`，`$2::agent_type`/`$3::agent_type`）与
  `record_msg_log_impl`（`msg_logs`，`$4::agent_type`）在写 recon/prober/
  enumerator 记录时 cast 失败 → `[db-track] agent_call: error ... invalid
  input value for enum agent_type`。这是**可观测性写入失败**（agent_logs/
  msg_logs 少行），不中断 pentest 主流程。

### 1.3 根因三：deepseek 网络错误（19 次，非代码）

`api.deepseek.com` 的 SSE 流被中断（`tls handshake eof` = TLS 握手被切断；
`AddrNotAvailable` = 本地无法分配连接，疑似 httpx/naabu 并发大量 socket 占尽
ephemeral port）。会打断 prober/enumerator 子 agent 的流式生成 → 空回应 /
坏 JSON → 重试。**不是代码 bug**，是运行环境 + `deepseek-v4-flash` 弱模型。
缓解方向（配置/环境，不在本次代码修复内）：换更稳的非 flash 模型、给扫描工具
并发/rate 设上限、给 LLM 请求加重试退避。

---

## 2. 用户的两个逻辑澄清（本次已答复，作为设计约束记录）

1. **分母没有扩大**：EAS `spec.json` 开了 `asset_wave_barrier: true`，
   `org_gate.rs:215,257-266` 用 `in_scope_assets_created_before(cutoff)` 冻结
   分母到 `stage_started_at` 之前的资产。扫描中新发现的资产不进当前波分母，
   记为 `next_wave_pending` backlog。用户"过阶段后再补齐新 port"的心智 =
   `global delta expansion` 设计方向（见 §3 P2）。
2. **httpx 探域名是对的**：EAS 按资产类型分流——domain/url → httpx（liveness +
   HTTP 指纹，不端口扫域名）；ip/cidr → naabu 端口扫（端口扫即探活）。没有
   独立"先 ping IP"步骤。"不活标记死"（liveness_state）目前**未实现**（P1）。

---

## 3. 修复分期

### P0 · 消除"一直报错"的两个高频噪音源（本会话做，纯代码 + additive migration）

- **P0-A** Template not found：让 recon/prober/enumerator 不再 render 不存在的
  模板。**方案 D（最小、零风险、行为等价）**：在
  `registry.rs::create_default_sub_agents_from_registry` 把这三个从
  `tmpl_or_fallback!("recon", build_recon_prompt())` 改为直接
  `build_recon_prompt()`（不走 registry、不 render、不 warn）。
  - 为什么不跟随 installer/browser/enricher 的"新建 .tera + 注册"pattern：
    prober/enumerator 的 prompt 正文含大量 `{{input_file}}`、`{target_id,
    base_url}` 花括号，直接 `include_str!` 进 Tera 会被当模板变量解析（`{{ }}`
    渲染成空串），必须整体 `{% raw %}` 包裹且易与 hardcoded 漂移。行为等价的
    方案 D 更稳。若未来要支持这三个的 DB prompt override，再按 `{% raw %}`
    包裹补 `.tera`（记为 follow-up，非本次）。
- **P0-B** agent_type enum：新 migration
  `20260703000001_extend_agent_type_stage_agents.sql`，
  `ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'recon'/'prober'/'enumerator'/
  'browser'/'refiner'/'orchestrator'`（跟随 `20260412000002` pattern，additive
  I10，向后兼容）。**只落文件，不跑迁移**（应用启动时 sqlx 自动跑）。

### P1 · 死资产标记 liveness_state（本会话做 Phase 1 inert 部分）

照既有 plan `docs/superpowers/plans/2026-07-02-dead-asset-liveness-state.md`：
- Phase 1（inert，零行为变化）：Task 1.1 migration（加 `liveness_state` /
  `liveness_reason` 列 + 回填）、Task 1.2 sqlx model + `TARGET_ROW_COLS`、
  Task 1.3 app-core domain Target（ts-rs）+ `compute_liveness_state` 纯函数。
- Phase 2（写点蓋值）：**已落地**——只在 hit-landing 写点蓋 `alive`（见 §4
  P2 清单）。ongoing dead/unreachable 標記（探空/探错時蓋 dead）**仍缺**，是
  剩餘 follow-up（見下）。
- Phase 3（下游 gate 分母剔 dead，gray-switch）：**已落地**——`skip_dead_assets`
  flag（enumeration/vuln_triage 開、EAS 不開）+ org_gate/execute.rs 分母剔除
  （guarded 不清空非空軸）+ `dead_asset_values` 查詢。按
  `docs/design/2026-07-02-recon-gaps-followups.md §4` 分析，本次只加獨立 bool
  flag + 新增 `dead_asset_values` 查詢、**不動** wave next-dispatch（問題二 B）與
  crediting 判據（問題三），屬 §4 判定的低衝突 additive 部分。

### P2 · global delta runner（本会话只落接线点文档，不实现）

用户要的"过阶段后自动再调一个东西补齐新发现的 port"= `global delta
expansion`。既有 plan `docs/superpowers/plans/2026-06-28-eas-global-delta-
expansion.md`：Phase 1（停 per-org 自动开波 + withhold pass token）已实现；
Phase 2/3/4（expansion candidate 读模型 → web endpoint promotion → 全局 delta
stage runner）**pending**。接线点见该 plan + `stage_run_call.rs`
（`prepare_stage_asset_wave` / `complete_stage_asset_wave` 一带）。本次不动。

---

## 4. 执行清单（勾选 = 已落地）

- [x] P0-A：`registry.rs` 三处改直接 hardcoded（recon/prober/enumerator）
      —— 已落地（`registry.rs:138/164/193` 直接 `build_recon/prober/enumerator_prompt()`，
      带注解说明为何不走 registry；不再 render 缺失模板、不再刷 warn）。
- [x] P0-B：新 migration 扩 agent_type enum（6 个值）
      —— `20260703000001_extend_agent_type_stage_agents.sql` 已存在
      （recon/prober/enumerator/browser/refiner/orchestrator）。
- [x] P1-Task1.1：migration `20260703000002_targets_liveness_state.sql` 加
      `liveness_state`/`liveness_reason` 列 + CHECK 约束 + 回填 + 部分索引。
- [x] P1-Task1.2：`golish-db` models/pentest.rs Target 加两列 + repo/targets.rs
      `TARGET_ROW_COLS`（`:98` const + `:718`/`:834` 测试常量三处同步）。
- [x] P1-Task1.3：`golish-app-core` domain/targets.rs Target 加 `liveness_state`/
      `liveness_reason`（`#[ts(optional)]`）+ `compute_liveness_state` 纯函数 + 4 个单测。
- [x] 连带对齐（plan 未逐条列，但为编译/数据贯通必须同改）：
      - `golish-app-core/src/ports/recon/targets.rs` 的 `TargetRow` struct + `From` impl
      - `golish-recon-app/src/targets/types.rs` 的 `TargetRow` struct + `From` impl
      - `golish-recon-app/src/targets/cmds.rs` 4 处显式 SQL 投影补两列
      - 3 个测试 fixture 补两列：`golish-agent-app/.../db_bridge/recon.rs`、
        `golish-recon-app/.../organization_recon/active.rs`、
        `golish-pentest-app/.../target_surface_hierarchy.rs`（后者是 golish-db 模型 Target）
- [x] **P2 写点蓋 alive**（死资产 plan Phase 2 的 alive 部分）：
      - `golish-db/repo/targets.rs`：`update_recon_extended_by_id` 抽
        `build_update_recon_extended_sql`+`eas_hit_alive_predicate_sql`，hit 蓋
        `liveness_state='alive'`（ELSE 保留，**不**在 per-hit 落庫標 dead）；
        `set_real_ip_by_id` 蓋 alive。+ 2 SQL 單測。
      - `golish-pentest/output_store/targets.rs`：AI-tool recon 落庫同款蓋 alive。
- [x] **P2 ongoing dead 標記**（死资产 plan Phase 2 的 dead 部分 · 原「最關鍵剩餘」）：
      - `golish-db/repo/targets.rs`：`mark_dead_if_no_signal_by_id`（+SQL 測）——
        **guarded** UPDATE：只在 row 仍無 alive 信號（http_status NULL + real_ip 空
        + 無開放埠）且非 'alive' 時蓋 `liveness_state='dead'`；冪等、與 P2 alive
        蓋值/naabu 落埠**順序無關**（有埠者恒為 alive）。
      - `golish-agent-app/ai/commands/bridge_config.rs`：EAS 批量 liveness outcome
        落庫（httpx）判 `!found`（探到但未存活）時，經 `mark_eas_liveness_dead_asset`
        （復用 `load_eas_landing_targets_for_asset`+`prefer_exact_landing_targets`
        解析目標行）標 dead。至此 dead 對**新** run 也生效，不再只靠 P1 backfill。
      - 侷限：批量 liveness 只分 found/empty，DNS-fail/WAF-block 一律標 'dead'（非
        'unreachable'）；guard 保證有埠/後續命中會翻回 alive（self-correcting）。
      - `mark_unreachable_if_no_signal_by_id`（+共用 guard builder + SQL 測）已備為
        DB primitive，但**刻意不從批量路徑接線**：批量 httpx 無 per-asset error 信號，
        且 P3 不剔 unreachable → 把「不可達」標 unreachable 反而會把它留在分母（與
        「死域名不灌分母」的目標相悖）。故批量一律標 'dead'（denominator-correct），
        unreachable 留給未來「有真 probe-error 信號的 per-asset 探測路徑」使用。
- [x] **P4 前端徽章**（死资产 plan Phase 4）：
      - `frontend/lib/pentest/types.ts` 視圖模型 `Target` 加 FE-only
        `liveness_state?`/`liveness_reason?`（ts-rs 重生成前的過渡；**不改** generated
        `Target.ts`，I5）。
      - 新 `frontend/components/TargetPanel/LivenessBadge.tsx`（alive 綠/dead 紅/
        unreachable 黃/未探 不渲染）；`TargetTreeRow` 名稱旁掛徽章；`TargetDetail`
        Recon Facts 加 Liveness 行（帶 reason）。
      - 過渡期安全：backend 重建前 `liveness_state` 為 undefined → 徽章不渲染，
        不影響編譯；`just check` 跑後 ts-rs 自動補 generated 型別。
- [x] **P3 下游 gate 分母剔 dead**（gray-switch）：
      - `coverage_truth.rs`：`dead_asset_values`（只剔 `liveness_state='dead'`）+ SQL 單測。
      - `db_traits/repo.rs` trait `dead_asset_values`（默認空）+ `db_bridge/{mod,recon}.rs` impl。
      - `stage_spec.rs` `skip_dead_assets` flag + `finding_verification_check.rs` 字面補；
        `enumeration`/`vuln_triage` spec.json 開、EAS 不開；spec 單測。
      - `org_gate.rs` + `execute.rs` 兩處分母剔 dead（guarded 不清空非空軸）。
      - seed JSON（`in_scope_targets_impl`/`attack_surface_seeds_impl`）帶 `liveness_state`。
- [x] ReadLints 检查所有改动文件 —— 全部无 lint。

> **迁移文件命名冲突注意**：P0-B 与 P1-Task1.1 都想用 `20260703000001`。
> 已按规划分开：P0-B = `20260703000001_extend_agent_type_stage_agents.sql`，
> P1-Task1.1 = `20260703000002_targets_liveness_state.sql`（顺延一个序号）。

> **剩余 follow-up（本会话未做、留给后续 session · 均非阻塞）**：
> - **ts-rs 重生成 / 全量驗證**：本會話按用戶指令未跑 `cargo`/`just`。`Target.ts`
>   仍缺 `liveness_state?`（前端已用 FE-only 型別過渡，不影響編譯）；跑
>   `cargo test -p golish-app-core` 或 `just check` 會自動重生成。收口時跑
>   `just precommit` 一次確認全綠（見 §5）。
> - **unreachable per-asset 接線**：DB primitive `mark_unreachable_if_no_signal_by_id`
>   已備；待有「per-asset probe-error 信號」的探測路徑（非批量）時接上。
> - **global delta runner（原 §3 P2）**：`2026-06-28-eas-global-delta-
>   expansion.md` Phase 2/3/4，與本任務無關，仍 pending。
>
> **本任務（EAS 報錯根治 + 死資產 liveness）P0/P1/P2/P3/P4 已全部落地。**

---

## 5. 验证（用户指令：本会话不跑，仅记录给后续 session / 用户启动时用）

```bash
# P0-A
cd backend && cargo nextest run -p golish-sub-agents prompt_registry --status-level fail
# P0-B + P1（迁移在应用启动或 sqlx migrate run 时生效）
cd backend && cargo check -p golish-db -p golish-app-core -p golish-agent-app
cd backend && cargo nextest run -p golish-app-core compute_liveness_state --status-level fail
# 全量收口（用户择机）
just precommit
```

预期：不再出现 `Template 'recon'/'prober'/'enumerator' not found` 与
`invalid input value for enum agent_type` 两类日志。

---

## 6. 不变量 / 风险

- I10（schema 向后兼容）：enum 用 `ADD VALUE IF NOT EXISTS`；targets 加列
  nullable 无 default，先扩后用，回滚安全。
- I7/I8：本次不动 gate 判定语义，不新增 found 判据，不混淆 checked_empty。
- AGENTS.md §2.7：改 DB schema 属高风险——用户已在对话中明确授权"直接修"，
  且均为 additive；本会话**只落迁移文件，不执行迁移**（不启动服务/不 migrate）。
