# EAS 自适应服务指纹实现计划

## Task 1：建立 DB 权威 pending-port 计划

- 修改 `backend/crates/golish-db/src/repo/coverage_truth.rs`。
- 新增 `ServiceFingerprintPortPlan` 与 exact org/project/scope 查询。
- 先写纯 JSON/SQL 单测：历史内嵌 service不能替代current-epoch marker、Nmap weak marker、DNS/53、caller subset语义。
- 验证：focused `golish-db` coverage_truth tests + scoped Clippy/rustfmt。

## Task 2：Nmap XML 与强/弱指纹语义

- 修改 `backend/crates/golish-pentest/Cargo.toml`，复用 workspace `quick-xml`。
- 修改 `output_parser.rs`：trusted Nmap XML解析并保留 method/conf/product/version。
- 修改 `output_store/helpers.rs`、`output_store/targets.rs`：`method=table`、`?` 与 pseudo
  service只写 `service_attempt`；probed/product/version才写强 service fingerprint。
- 保留文本输出兼容。
- 先写 RED：XML probed/table、多 host、closed/filtered、truncated XML、`smtp?`。
- 验证：focused parser/output_store tests + `golish-pentest` Clippy/rustfmt。

## Task 3：服务端执行计划、有限并发和 timeout所有权

- 修改 `eas_capabilities.rs`：
  - schema移除模型 timeout；ports只收窄 pending；
  - `ServiceFingerprintBatch` 替换为 target/chunk plan；
  - chunk=16、`buffer_unordered(3)`、small-first；
  - fast intensity2、一次 4-port intensity0 recovery、最多8-port deep enrichment；
  - 每次 run调用原 `execute_guarded`。
- 先写纯计划/参数/聚合 RED 测试，覆盖 58-port target、混合小目标、caller交集、有限
  attempt和 deterministic排序。

## Task 4：增量 landing 与 per-target aggregate evidence

- XML完整 chunk在 command成功或有完整安全 records时立即调用 guarded output store。
- 记录 chunk `attempted/strong/weak/remaining`；不完整/截断 output不落终态。
- 新增 service-specific target aggregate evidence/outcome，保证一个 target只在所有分片结束后
  更新一次 asset-level technique outcome。
- wrapper顶层返回 per-target状态、network job数、recovery/deep状态，并继续关闭 generic
  storage/evidence hook。
- focused tests覆盖 found+weak混合、部分 timeout、全部 weak、guard landing failure。

## Task 5：提示词、methodology、模块卡与状态

- 修改 Prober prompt、StageRefiner、EAS methodology；移除模型 timeout/batch/retry所有权。
- 同步 `golish-pentest-app/pentest_bridge`、`golish-pentest/output_store`、`golish-db/repo`、
  `golish-agent-kit/task_orchestrator`、`golish-sub-agents/executor` 模块卡和 INDEX状态。
- 更新 `feature_list.json` verification/evidence 与 `agent-progress.md`。

## Task 6：定向验证

每条 Cargo命令前运行 `just space-guard`：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(service_fingerprint_port_plan) | test(confirmed_open_service_ports) | test(weak_service_names)'
cd backend && cargo nextest run -p golish-pentest -E 'test(nmap_xml) | test(nmap_weak_service) | test(nmap_table_guess) | test(nmap_closed) | test(tcpwrapped_and_pseudo_services)'
cd backend && cargo nextest run -p golish-pentest-app -E 'test(service_fingerprint)'
cd backend && cargo nextest run -p golish-agent-kit -E 'test(eas_coverage_gap_instruction_is_batch_first)'
cd backend && cargo nextest run -p golish-sub-agents -E 'test(test_prober_prompt_is_active_surface)'
cd backend && cargo clippy -p golish-db -p golish-pentest -p golish-pentest-app -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json resources/toolsconfig/nmap.json resources/harness/stages/external_attack_surface/spec.json
git diff --check
```

按 AGENTS.md §0.1 不主动运行 `init.sh` / `just precommit` / 全 workspace测试。外部目标实测
只使用用户已明确授权的 exact IP；若本轮不需要，不以外部扫描代替 deterministic测试。
