# golish-agent-kit / harness

> **一句话职责**：Operation Harness（Phase 1c）——把 chat panel 的 task 模式重构为 stage harness：Profile/StageSpec/Operation DAG 投影 + 终态 NlSlice + 确定性 intent classifier + 每 tool call 前 authz + `StageHarness::validate_gate`（6 个 gate check）+ Sprint Contract。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/harness/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 stage harness（gate 校验、stage 转移、profile/DAG 投影、intent 分类、sprint contract）时
- 改 6 个 gate check（schema/scope/contract/vacuous/freshness）或前置 authz 时
- 任何 evidence gate PASS/BLOCK 语义（I7/I8）相关时

## 职责

落地 stage harness MVP（design Doc 1/2/3）。`stage_harness` 主入口（`for_stage` + `validate_gate`）；`operation_graph` 加载 base DAG + profile 投影 + `next_stages`；`intent_classifier` 确定性词库分类；`gate/` 6 个 check 调度；`sprint_contract` 生成 finding 数量范围。

## 公开接口

| 符号 | 说明 |
|---|---|
| `StageHarness`（`for_stage` / `validate_gate`） | 主入口 |
| `Profile` / `StageSpec` / Operation DAG（投影 + `next_stages`） | 阶段定义 + DAG |
| `IntentClassifier` / `NlSlice` | 确定性意图分类 / 终态 4 字段 |
| `gate`（schema/scope/contract/vacuous/freshness 6 check） | 确定性证据门 |
| `SprintContract` + Generator / `pre_action_authorizer` | 契约 / 前置 authz |

## 关键文件

| 文件 | 作用 |
|---|---|
| `stage_harness.rs` / `stage_transition.rs` | 主入口 / gate→下一 stage |
| `operation_graph.rs` / `profile.rs` / `stage_spec.rs` | DAG 投影 / profile / stage 定义 |
| `gate/` | 6 个确定性 check |
| `evidence_facts.rs` | 从工具命令/输出派生 coverage facts（passive intel + EAS） |
| `intent_classifier.rs` / `nl_slice.rs` / `sprint_contract.rs` / `pre_action_authorizer.rs` | 分类 / 终态 / 契约 / authz |

## 依赖

- crate 内 `golish-pentest::evidence_ledger`（scope label）；resources/harness JSON

## 注意事项 / 坑

- **不变量 I7/I8**：gate 是**确定性规则**（schema/scope/contract/vacuous/freshness），不能拿模型自报当通过；`scope_check` 验 evidence_refs 当前 label = InScope。
- `target_intel` 的 6 个 `GOLISH-INTEL-*` 覆盖列仍必核，但阶段不再暴露任何 scan-tool selector（`allowed_tool_types=[]`）：found 只能来自 `recon_map_assets` / `recon_lookup_whois` 等 registry/provider 工具落库后的 DB truth；缺 provider、无结果或不适用要走 `blocked` / `checked_empty` / `not_applicable` 终态，不能切 CLI fallback。
- `source_coverage` 规则读取 `GateContext.source_queries`（来自 `source_query_log`）来证明 provider/source 已终态尝试；它不投影 found，found 仍由 DB/ledger truth 决定。
- `coverage_complete` 在 `derive_from_evidence=true` 时也可消费 `source_query_log` 的终态 source row 来关闭**非 found** gap：精确 technique（如 RDAP/WHOIS）按空/阻断终态处理，`recon_map_assets` provider survey 只覆盖 provider-backed intel 技术；自报 `found` 仍必须有 DB/ledger truth，不能靠 source row 过门。
- `coverage_complete` BLOCK 时会在 `HarnessRecoveryActions.coverage_gap_actions` 输出结构化缺口清单（`asset` / `technique` / `reason` / `suggested_tools`），供 `submit_stage_deliverable` 和 sub-agent repair mode 直接告诉模型“只补这些目标/技术”，不要只靠自然语言 reason 猜。
- `external_attack_surface` 现在走 DB-truth 瘦交付：`facts_from_db_truth=true`，`coverage_complete.authoritative_found=true` 只认 targets/ports/fingerprints/technique_outcomes 投影的 found LIVENESS/PORT/SERVICE-FINGERPRINT；`coverage_denominator.authoritative=true` 不再要求手抄 denominator。主动 negative 终态仍需显式 `checked_empty+evidence` 或 `blocked/not_applicable+note`，直到 active Empty 事实源完全覆盖。
- EAS 工具证据事实也在 `evidence_facts.rs` 派生：`httpx` / `nmap -sn` / `naabu` / `whatweb` 会映射到对应 `GOLISH-EAS-*` technique；`nmap` 的 DNS failure（stderr `Failed to resolve`）会成为 `error` terminal fact，避免同一 liveness gap 无限重试。
- `external_attack_surface` 的 `stage_run_pass_token` claim 视为 Surface closeout 信号：它只在 orchestrator 按 per-org completion ledger 重算通过后才有效，避免 fan-out 阶段主 agent 只交 pass token 时被 `surface_coverage` 误拦；泛化的 `discovery` claim 仍不映射到 Surface。
- stage tool whitelist 只约束真实扫描调用；`check_job` / `kill_job` / `list_jobs` / `wait_for_background_jobs` 是后台 job 控制面，必须 exempt，否则 submit barrier 报“后台任务仍在跑”后 worker 无法等待输出或检查明显卡死的 job。
- 设计见 `docs/design/2026-05-26-*`；内层 harness 当前 deferred（见 AGENTS.md §6）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit harness
```
