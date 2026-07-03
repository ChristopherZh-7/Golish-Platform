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
| `operation_continuity.rs` | cross-session adoption 的 IO-free cursor math：按 reusable prefix 计算 entry stage + remaining DAG allowlist |
| `gate/` | 6 个确定性 check + `rule_engine` gate op（含 `candidate_grounded` / `candidate_disposition_complete`，设计 2026-07-02） |
| `chain_wave.rs` | attack_candidate⇄verification 波次循环的纯决策函数 `decide_chain_wave`（去重+燃料+链深收敛），DB-free、可单测；活体游标覆写接线在 graph-flow 层（待做） |
| `evidence_facts.rs` | 从工具命令/输出派生 coverage facts（passive intel + EAS） |
| `intent_classifier.rs` / `nl_slice.rs` / `sprint_contract.rs` / `pre_action_authorizer.rs` | 分类 / 终态 / 契约 / authz |

## 依赖

- crate 内 `golish-pentest::evidence_ledger`（scope label）；resources/harness JSON

## 注意事项 / 坑

- **不变量 I7/I8**：gate 是**确定性规则**（schema/scope/contract/vacuous/freshness/DB truth），不能拿模型自报当通过；模型提交里的 `evidence_refs` / `evidence_ids` 只是可选 ledger 调试引用，不能作为必填交付字段。若模型写了 id，runtime 仍必须校验它真实存在，假 id 直接 `needs_fix`。
- `target_intel` 的 6 个 `GOLISH-INTEL-*` 覆盖列仍必核，但阶段不再暴露任何 scan-tool selector（`allowed_tool_types=[]`）：found 只能来自 `recon_map_assets` / `recon_lookup_whois` 等 registry/provider 工具落库后的 DB truth；缺 provider、无结果或不适用要走 `blocked` / `checked_empty` / `not_applicable` 终态，不能切 CLI fallback。
- `target_intel` 的 SUBDOMAIN 是 registrable-apex 维度；被动发现出的叶子子域名、`www.*` 主机、URL 形态资产都不再要求继续做 SUBDOMAIN 枚举，避免越发现越把 coverage 分母撑大。
- `source_coverage` 规则读取 `GateContext.source_queries`（来自 `source_query_log`）来证明 provider/source 已终态尝试；它不投影 found，found 仍由 DB/ledger truth 决定。
- `coverage_complete` 在 `derive_from_evidence=true` 时也可消费 `source_query_log` 的终态 source row 来关闭**非 found** gap：精确 technique（如 RDAP/WHOIS）按空/阻断终态处理，`recon_map_assets` provider survey 只覆盖 provider-backed intel 技术；自报 `found` 仍必须有 DB/ledger truth，不能靠 source row 过门。
- `coverage_complete` BLOCK 时会在 `HarnessRecoveryActions.coverage_gap_actions` 输出结构化缺口清单（`asset` / `technique` / `reason` / `suggested_tools`），供 `submit_stage_deliverable` 和 sub-agent repair mode 直接告诉模型“只补这些目标/技术”，不要只靠自然语言 reason 猜。
- `external_attack_surface` 现在走 DB-truth 瘦交付：`facts_from_db_truth=true`，`coverage_complete.authoritative_found=true` 只认 targets/ports/fingerprints/technique_outcomes 投影的 found LIVENESS/PORT/SERVICE-FINGERPRINT；`coverage_denominator.authoritative=true` 不再要求手抄 denominator。主动 negative 终态仍需显式 `checked_empty` 或 `blocked/not_applicable+note`，但不要求模型手写 evidence id。
- `GateContext.not_applicable_coverage` 是 DB/调用方注入的确定性终态集合（例如 EAS DNS/53-only IP/CIDR 的 `GOLISH-EAS-SERVICE-FINGERPRINT`），由 `GateContextBuilder` 归一后供 `coverage_complete` 消费；只有规则 terminal set 包含 `NotApplicable` 时才可关闭 cell，不能用它让模型自报的 found 通过。design 2026-07-03：`org_gate` / `harness_submit_tool` 的 enumeration 分支复用同一个 `eas_service_not_applicable_assets`（只开 53 无 web 面的 IP）把这些 IP × `GOLISH-ENUM-JS/DIR/PARAM/JSAPI` 四轴也注入 not_applicable_coverage——即「只开 DNS 的 IP 不是内容枚举根」，避免陈旧 http_status 让共享 DNS/CDN IP 楔住 enumeration gate；对从未进 web-capable 分母的 IP 是安全 no-op。
- `external_attack_surface` 的 PORT/SERVICE-FINGERPRINT 只适用于 IP/CIDR host 资产；domain/url 只承载 LIVENESS/vhost。没有可委托/已注册 IP 的域名不能在 EAS 里被当成 PORT/SERVICE 扫描主体，缺 IP 是 target_intel/DNS 落库缺口。
- `enumeration` 的 per-org gate 资产轴会在进入 `coverage_complete` 前优先收敛到 EAS 已有 `GOLISH-EAS-LIVENESS` found truth 的 web-capable target（domain/url），并额外纳入 `targets.http_status IS NOT NULL` 的 IP/CIDR web 资产。Enumeration 覆盖轴固定为四类：`GOLISH-ENUM-JS`（JS 收集）、`GOLISH-ENUM-DIR`、`GOLISH-ENUM-PARAM`、`GOLISH-ENUM-JSAPI`；裸 IP 仍默认不适用，只有 `web_capable_assets` 命中的 IP/CIDR 才要求这四类内容枚举。如果没有任何 EAS live truth 且没有 web-capable IP 资产，则保持原 in-scope 资产轴 fail-safe，避免空 worklist 造成假通过。这个口径必须和 `ai_get_stage_asset_coverage` / `check_stage_asset_coverage` 保持一致。
- `enumeration.allowed_tool_types` 只允许内容枚举类工具：`recon/crawler`、`web/route-probe`。不要把 `recon/http` 加回 enumeration；也不要把外部目录爆破 `web/dir-fuzzer`（ffuf/gobuster/feroxbuster/dirsearch）或主动隐藏参数爆破工具加回。DIR 默认由 `route_probe_paths` 消费 JS/API path prefix + 小字典；PARAM 默认从 browser/js_extract/crawler 已观察到的请求、query、form 与 `param_hints` 落 `api_endpoints.params`。
- `external_attack_surface` 开启 `asset_wave_barrier=true`：当前 batch 的资产分母冻结在 `operation_state.stage_started_at` 之前已存在的 in-scope target；本阶段运行中新落的资产保留进 DB，但作为后续 global delta expansion backlog 处理，不能把当前 gate 分母边跑边撑大，也不能在单个 org PASS 后立刻自动递归重跑。
- `coverage_complete` 要区分“模型没交 coverage 矩阵”和“调用方明确注入 `in_scope_assets=Some([])`”：前者仍按 I8 BLOCK；后者代表 DB 真值下该 org/stage 零资产，EAS 等阶段应 vacuous pass，并与 `check_stage_asset_coverage.ready_to_submit=true` 保持一致，避免零资产 org 被推入 repair 后开始猜测扫描。
- EAS 工具证据事实也在 `evidence_facts.rs` 派生：`httpx` / `nmap -sn` / `naabu` / `whatweb` 会映射到对应 `GOLISH-EAS-*` technique；`GOLISH-EAS-LIVENESS` 对 URL endpoint 使用 gate-compatible join key（去 scheme/大小写但保留 port/path），不能把 `http://host:90` 的探活事实折叠为裸 `host`；`nmap` 的 DNS failure（stderr `Failed to resolve`）会成为 `error` terminal fact，避免同一 liveness gap 无限重试。
- `external_attack_surface` 的 `stage_run_pass_token` claim 视为 Surface closeout 信号：它只在 orchestrator 按 per-org completion ledger 重算通过后才有效，避免 fan-out 阶段主 agent 只交 pass token 时被 `surface_coverage` 误拦；泛化的 `discovery` claim 仍不映射到 Surface。
- fan-out 阶段的 pass-token closeout 必须按 scoping 绑定的 engagement root org subtree 核 `org_stage_completions`；只有没有 root 绑定时才允许 legacy 全库 org 口径。若 `operation_state.current_stage` 仍是该 fan-out stage，closeout 还必须要求 completion 晚于本次 `operation_state.stage_started_at`，防止旧 run 的 passed ledger 生成当前 stage 的 pass token。否则同一 embedded DB 里的 sibling/test org 或旧 completion 会把当前 operation 卡死。
- `operation_continuity` 只做纯决策：输入 profile-projected DAG + `ContinuitySnapshot`，输出已 adopt 阶段、第一未满足 stage、remaining-stage allowlist。是否允许复用、是否询问用户、DB snapshot 怎么构建都在上层；不要把 DB 查询塞进 harness 纯模块。上层 continuity preflight 必须有 engagement root 才能把 scoping 标为 reusable。
- stage tool whitelist 只约束真实扫描调用；`check_job` / `kill_job` / `list_jobs` / `wait_for_background_jobs` 是后台 job 控制面，必须 exempt，否则 submit barrier 报“后台任务仍在跑”后 worker 无法等待输出或检查明显卡死的 job。
- `tool_taxonomy.rs` 把 Golish direct enumeration tools（`browser_collect_js_api` / `js_collect` / `js_extract_apis` / `route_probe_paths`）也视为 scan taxonomy 成员，分别归到 `recon/crawler` 与 `web/route-probe`；外部目录工具归 `web/dir-fuzzer`，不在 enumeration allow-list。`whatweb` 仍归 `recon/http`，因此 EAS 可用、enumeration 不可用。
- **攻击段三阶段（设计 2026-07-02）**：`StageKind` 现有 13 变体，vuln 段 = `vuln_triage`（**公式化扫描**：specialist=vuln_scanner，10 类可机械批量跑技术，`found`→finding）→ `attack_candidate`（**新 StageKind**：推理合成 `AttackCandidate` 假设，无扫描工具，`candidate_grounded` gate 要求每条有 rationale，证据真实性由 backend ledger/DB truth 解决）→ `verification`（**真打**：`candidate_disposition_complete` gate 要求每个 approved candidate 达终态 verified/refuted/blocked）。DAG **保持无环**（无 `verification→attack_candidate` 回边）；a→b→c 波次回流由 `chain_wave` 在 graph-flow 层游标覆写实现（活体接线待做）。SSTI/SSRF/LFI/认证绕过/业务逻辑从 vuln_triage 移交 attack_candidate。`authoritative_found` 暂未对 vuln_triage 开启（nuclei/dir/weakpw/tls 的 technique_outcomes 写路径未覆盖，开了会永久 BLOCK；只开 `derive_from_evidence`）。
- 设计见 `docs/design/2026-05-26-*` 与 `docs/design/2026-07-02-attack-stage-formulaic-candidate-exploit.md`；内层 harness 当前 deferred（见 AGENTS.md §6）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit harness
```
