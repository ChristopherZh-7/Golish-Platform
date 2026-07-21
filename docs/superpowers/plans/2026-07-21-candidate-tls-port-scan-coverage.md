# Candidate TLS 与端口扫描覆盖语义实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 AI 对 TLS observation 的 Candidate 判断有冻结证据、具体理由和可执行验证方案，并让 EAS 端口扫描通过 profile 和 evidence outcome 准确表达局部与完整覆盖。
**架构：** Candidate Decision Gate 不替 AI 决定漏洞，只对 actionable TLS 拒绝笼统的跳过理由，并继续由既有 classifier 从 AI 的 Candidate 决策派生 immutable plan。EAS wrapper 新增 server-owned scan profile，profile 同时决定底层命令、PORT/LIVENESS verdict 和返回的 coverage metadata；数据库 schema 不变。
**技术栈：** Rust 2021、sqlx/Postgres 现有 evidence ledger、Naabu/Nmap wrapper、cargo-nextest。

## 修改文件

- `backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`：TLS observation 的 mandatory replay / metadata-only 决策约束与纯函数测试。
- `resources/harness/stages/attack_candidate/methodology.md`：告诉 analyst 哪些 TLS 条目必须 Candidate，哪些只是元数据。
- `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`：scan profile 解析、命令、verdict、coverage result 与测试。
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`：Prober 对 concrete IP 用 full profile 完成 PORT Gate，CIDR 先 bounded discovery。
- `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`：repair hint 使用业务 wrapper/profile，不再固定 Top 1000 raw recipe。
- `docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`：同步公开合同。
- `feature_list.json`、`agent-progress.md`：记录状态、命令、退出码与证据。

## 任务 1：TLS Candidate 策略 RED

**文件：** `backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`

1. 增加 `actionable_tls_observation_rejects_generic_no_candidate_reason`：构造 `weak-cipher-suites` 的 `nuclei_match_v1` manifest，提交 `observation_not_exploitable`，断言错误码 `ATTACK_TLS_NO_CANDIDATE_REASON_TOO_GENERIC`。
2. 增加 `actionable_tls_observation_accepts_specific_evidenced_no_candidate`：同一 observation 使用 `context_refuted` 和 frozen evidence 可以通过，证明结论仍由 AI 决定。
3. 增加 `tls_metadata_observation_accepts_metadata_no_candidate`：`ssl-issuer` 以 `tls_metadata_only` 正常终结；另证明有完整假设时仍允许 Candidate。
4. 增加 `actionable_tls_observation_builds_low_priority_nuclei_plan`：`deprecated-tls` Candidate 生成 `verify.nuclei_template_replay`、priority `low`、冻结 template/URL。
4. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(actionable_tls_observation_) | test(tls_metadata_observation_)'
```

预期：测试因尚未存在 TLS decision closure policy 而失败，不是编译夹具错误。

## 任务 2：TLS Candidate 策略 GREEN

**文件：** `backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`、`resources/harness/stages/attack_candidate/methodology.md`

1. 添加窄 observation 分类和 reason policy：

```rust
enum NucleiObservationClass {
    Other,
    TlsSecurity,
    TlsMetadata,
}

fn nuclei_observation_class(observation: &Value) -> NucleiObservationClass
```

只在 `schema=nuclei_match_v1` 且 `technique=WSTG-CRYP-03` 时按设计中的 template allowlist 分类。
2. 在 `build_candidate_acceptance` 的 no-candidate 分支执行策略：TlsSecurity 拒绝 generic reason，但接受具体 evidence-backed 例外；TlsMetadata 推荐 `tls_metadata_only`，不禁止 AI 在有完整假设时创建 Candidate。
3. 方法学写明 AI 默认如何判断、哪些理由不能跳过，以及 Candidate 只是低风险安全重放，不能提前宣称漏洞成立。
4. 重跑任务 1 命令，预期全部通过。

## 任务 2A：大 Manifest 选择器与重复 Candidate 预检

**文件：** `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`、`backend/crates/golish-sub-agents/src/defaults/prompts/orchestration.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. `candidate_decision_groups` 增加 exact `nuclei_template_ids` selector；selector 只能命中 frozen `nuclei_match_v1` template id，未知/重叠/混用 fail closed。
2. Candidate Nuclei group 只允许一个 template id；metadata no-candidate group 可以组合多个 template。
3. acceptance batch 对 normalized target+technique+hypothesis 做语义 identity 预检；重复返回 `ATTACK_CANDIDATE_DUPLICATE_IDENTITY` 和 exact 冲突 keys。
4. submit preview 复用 acceptance 预检，让模型在 durable submission 和 final seal 前修复；prompt 指示保留一个 Candidate并以 `duplicate_candidate` 终结重复项。
5. 在克隆库实跑 CLI，先观察 typed duplicate rejection，再确认同一 Analyst chain 修复并最终 Gate PASS；不批准 Candidate。

## 任务 3：端口 profile 与返回语义 RED

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 增加 `port_scan_profiles_build_bounded_scanner_recipes`，断言 quick/standard/full 分别生成 Naabu Top 100、Top 1000、full 和 Nmap Top 100、Top 1000、`-p-`。
2. 增加 `incomplete_port_profile_never_terminalizes_port_coverage`，局部扫描即使发现开放端口也必须是 `partial`；无命中 LIVENESS 也必须 `partial`。
3. 增加 `full_port_profile_terminalizes_found_and_empty`，full 才允许 PORT `found/empty`。
4. 增加 `full_port_profile_accepts_only_bounded_cidr_and_one_address_family` 与 `port_scan_result_reports_coverage_and_next_action`。
5. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(port_scan_profile) | test(incomplete_port_profile) | test(full_port_profile)'
```

预期：缺少 profile 类型、命令和 outcome policy 而失败。

## 任务 4：端口 profile 与 evidence outcome GREEN

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 新增 `PortScanProfile` / `PortScanPlan`，解析 `scan_profile`；缺省 standard；与 legacy `top_ports` 同时出现时返回参数错误。
2. full 只允许最多四个展开地址的 IPv4 `/30` 或更窄 CIDR以及 exact IPv6 `/128`；更宽 CIDR 在 runner 前写 target-guarded policy-blocked evidence/outcome。删除模型可达的 legacy/Masscan/custom 参数入口。
3. 把 profile 传入 `persist_guarded_eas_evidence_and_outcomes`，通过下列策略派生 verdict：

```rust
PORT && !profile.complete() => EasTechniqueVerdict::partial(open_count)
LIVENESS && !profile.complete() && alive_count == 0 => EasTechniqueVerdict::partial(0)
_ => EasTechniqueVerdict::from_count(count)
```

4. 在 wrapper 结果加入 `scan_coverage` 和 incomplete `next_action`，保留 exact command、landing/evidence 字段。
5. 重跑任务 3 命令，预期全部通过。

## 任务 4A：PORT 证明的 read-side Gate 与 `-Pn` 语义

**文件：** `golish-pentest/output_parser.rs`、`golish-db/repo/{audit,technique_outcomes}.rs`、`golish-agent-app/ai/db_bridge/evidence.rs`

1. Nmap XML parser 分离 manifest coverage 与业务 landing；`reason=user-set` 不发布 liveness/child asset。
2. full 必须逐 host 证明 65535 个端口被 accounting，且 runstats success；attestation 冻结 canonical manifest/fixed recipe/raw XML。
3. Gate read side 从 audit raw_output 重算 hash、重解析 XML并检查 exact producer/source；policy blocked 只接受 closed reason-code allowlist、no-network 和 exact target guard。
4. 端口 outcome 改用 monotonic guarded upsert，后来的 quick/standard 不能覆盖已有 terminal。
5. 返回的开放端口改为 `{ip,port,protocol}` endpoints，并返回 per-target evidence ids/error code。

## 任务 5：Agent 使用合同

**文件：** `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`

1. Prober 指令改成 concrete IP 用 `scan_profile="full"` 才能完成 PORT；quick/standard 仅用于发现且保持 partial。
2. CIDR 指令改成 bounded standard discovery，具体 child IP 进入 supplemental wave 后 full。
3. repair hint 不再建议 raw `naabu -top-ports 1000`，改为 `eas_discover_ports(..., scan_profile="full")`。
4. 更新对应字符串守卫测试。

## 任务 6：定向验证与文档收尾

**文件：** 模块卡、索引、`feature_list.json`、`agent-progress.md`

1. 运行受影响测试：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(actionable_tls_observation_) | test(tls_metadata_observation_) | test(candidate)'
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(port_scan_profile) | test(incomplete_port_profile) | test(full_port_profile) | test(discover_ports)'
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -E 'test(prober) | test(eas_discover_ports)'
```

2. 运行受影响 crate Clippy 与格式：

```bash
just space-guard
cd backend && cargo clippy -p golish-agent-kit -p golish-pentest-app -p golish-sub-agents --lib --tests -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
git diff --check
```

3. 把每条命令、退出码和关键输出写入 `agent-progress.md`；只有全部定向证据通过才把 feature 标为 `passing`。大型 `init.sh`、全量测试和 `precommit` 按项目策略不运行并如实记录。
4. 用 `scripts/run_tree.py` 读取真实 clone run，确认 Candidate per-org Gate 与主 stage Gate 均 PASS；只读核对 stage completed、Unit passed、Candidate proposed 数量和 Attempt=0。

## 计划自检

- 规格覆盖：TLS 强制重放、TLS 元数据过滤、scan profile、CIDR 边界、partial/full、返回契约、Agent 使用方式和定向验证均有对应任务。
- 类型一致性：统一使用 `PortScanProfile` / `PortScanPlan`、`NucleiObservationClass`、`tls_metadata_only` 和 `ATTACK_TLS_NO_CANDIDATE_REASON_TOO_GENERIC`。
- 无数据库 migration；若实现发现现有字段无法保持 deterministic Gate，停止并向用户申请 schema 修改授权，不静默扩展范围。
