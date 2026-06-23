# target_intel provider source closure plan

## 目标

先把 `target_intel` 收敛成 provider-source terminal 模型的第一版：阶段内不再允许
scan-tool fallback；agent 只跑 provider registry 工具并提交 DB 无法投影的终态。

## Phase 0 - 现状确认

- 确认 `target_intel` 仍要求 6 个 coverage technique。
- 确认 `technique_outcomes` 与 `source_query_log` 已存在，可作为后续 provider/source 审计层。
- 确认 `recon_map_assets` 当前返回的是 summary/provider ids，不是 per-provider terminal 明细。

## Phase 1 - 收紧阶段边界

- 修改 `target_intel/spec.json`：`allowed_tool_types` 改为空数组。
- 修改 methodology：删除 CLI fallback 和 URL-history optional 路径；强调 provider 不可用/失败时交终态。
- 修改 stage charter / orchestrator prompt：把示例采集路径改成
  `recon_map_assets` / `recon_lookup_whois`。
- 修改 refiner passive intel hint：所有 technique 的下一步都指向 provider/WHOIS registry 工具或提交
  terminal status，不再给 shell 命令。

## Phase 2 - provider/source 审计增强

- `PassiveIntelSummary` 透传底层 `AssetIntelRun.provider_status` 为
  `providerStatus`。
- `golish-agent-runtime` 在 recon passive evidence 入账后，把 provider/source
  terminal rows 写入 `source_query_log`：
  - `completed` → `found`
  - `checked_empty` → `empty`
  - `unavailable` → `blocked`
  - `failed` → `error`
- `recon_lookup_whois` 写一条 org 级 `rdap / lookup_whois` source row，带
  `GOLISH-INTEL-WHOIS` technique。

## Phase 3 - source coverage gate read

- `source_query_log` 增加 `list_for_run` 只读 repo。
- `DbRepoProvider` / `EvidenceLedgerQuery` 增加 `source_query_facts` seam，app bridge 映射
  `golish-db` row → `SourceQueryFact`。
- `GateContext` / `GateContextBuilder` 增加 `source_queries`，三条 gate 入口都注入：
  - submit 预检：`harness_submit_tool::gate_context`
  - per-org fan-out：`harness/org_gate.rs`
  - 主 stage close：`task_orchestrator/subtask_phases/execute.rs`
- `GateRule::SourceCoverage` 只验证 terminal source row：
  - provider survey (`map_assets`) 覆盖 DNS/SUBDOMAIN/ASN/CT/OSINT 的 source-attempt 证明；
  - RDAP (`lookup_whois`) 覆盖 WHOIS；
  - `blocked` / `not_applicable` + note 可作为“无可调用 source/provider”的终态；
  - source row 不投影 found，DB/ledger truth 仍是 found 唯一来源。

## Phase 4 - duplicate guard

- runtime registry 执行前查同 run 的 source rows：
  - `recon_map_assets` → `map_assets`
  - `recon_lookup_whois` → `lookup_whois`
  - `recon_discover_subsidiaries` → `discover_subsidiaries`
- 已有 terminal row 时返回 `skipped_duplicate=true` + `existing_evidence_ids`，不再调用 provider。

## Phase 5 - 测试

- 更新 `stage_spec`、`resources`、`tool_taxonomy`、`subtask_phases` 相关断言。
- 跑 targeted cargo tests：
  - `cargo test -p golish-agent-kit target_intel --lib`
  - `cargo test -p golish-agent-kit stage_methodology --lib`
  - `cargo test -p golish-agent-kit command_hint --lib`
  - `cargo test -p golish-agent-kit osint_tools --lib`
  - `cargo test -p golish-agent-runtime provider_status_rows --lib`
  - `cargo test -p golish-agent-runtime whois_result_maps --lib`
  - `cargo test -p golish-recon-app summary_serializes_with_camel_friendly_fields --lib`
  - `cargo test -p golish-agent-kit source_coverage --lib`
  - `cargo test -p golish-agent-kit source_query_fact_does_not_make_authoritative_found_pass --lib`
  - `cargo test -p golish-agent-kit context_builder --lib`
  - `cargo test -p golish-db source_query_log --lib`
  - `cargo test -p golish-agent-runtime duplicate_guard --lib`
  - `cargo check -p golish-agent-app -p golish-agent-runtime`

## 风险

- 收紧后 DNS/CT/ASN/OSINT 只能依赖 provider landing 或人工 terminal status；这会降低“自动过 gate”的概率，但能避免伪造完整性。
- `source_query_log` 已 gate-read，但只保证“source/provider 已终态尝试”，不能保证被动数据全网完整。
- duplicate guard 当前按 tool action 粒度跳过；如果未来要做 provider 级 partial rerun，需要把 provider capability/selection 纳入 guard key。
- 完整性定义必须由 provider capability matrix 决定，不能用“跑完 recon_map_assets”替代。
