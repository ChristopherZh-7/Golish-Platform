# Vuln 最终封存 Surface N/A 物化实现计划

**目标：** 把 Gate已认可的 backend-derived Enumeration surface N/A安全、幂等地物化到 operation-scoped `technique_outcomes`，让180/180 Vuln coverage能通过V2 final seal。

**架构：** `golish-agent-runtime`在Company Controller Gate PASS后重读 exact operation/org/session coverage snapshot，复用agent-kit的可信snapshot解析器，仅物化固定manifest-authority N/A，再沿用既有raw outcome catalog和全量相等封存规则。DB schema、Gate规则和producer landing不变。

**技术栈：** Rust 2021、Tokio、Serde JSON、cargo-nextest。

## 任务1：锁定可信提取与identity语义

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 增加focused RED测试：
   - `vuln_gate_materializes_only_trusted_surface_manifest_not_applicable_cells`：只收完整source+authority+固定technique+canonical origin，忽略deliverable伪造项。
   - `vuln_terminal_materialization_uses_operation_run_and_exact_coverage_identity`：snapshot参数为exact operation/chat session，upsert run id为operation id、source为manifest。
   - 更新Company Controller run-id contract测试，Vuln返回operation id而Enumeration保持None。
2. Cargo前运行`just space-guard`，再运行精确test filter，确认旧实现RED。

## 任务2：最小实现

**文件：** 同上。

1. `GateTerminalOutcome`携带固定source；现有Target Intel/EAS继续使用`submit_stage_deliverable`。
2. `gate_terminal_outcomes_to_materialize`对Vuln调用`trusted_vuln_surface_not_applicable_from_snapshot`，从同一可信cell读取非空server note，并返回`Result`以fail closed。
3. materialization snapshot seam增加operation id并调用`stage_asset_coverage_for_operation`；函数分离coverage session id与outcome run id，并先用`validated_exact_web_origin_axis_from_coverage_snapshot`校验stage/org/session envelope。
4. Company Controller对Vuln使用`operation_id.to_string()`作为outcome run id，并校验exact final-sealed Enumeration handoff的operation/org/scope/stage/schema/authority（普通同operation=`deliverable_final_seal`，冻结fork输入=`stage_fork_final_seal`）及非空、正数、去重且未超限的source evidence ids。随后通过窄seam在当前operation/org/Unit/chat/project追加facts=None的新鲜aggregate attestation，物化行只引用这个新id；Enumeration仍跳过。
5. 重跑任务1测试，确认GREEN。

## 任务3：回归与静态检查

每条Cargo前运行`just space-guard`，再运行：

```bash
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(vuln_gate_materializes_only_trusted_surface_manifest_not_applicable_cells) | test(vuln_terminal_materialization_uses_operation_run_and_exact_coverage_identity) | test(passed_gate_terminal_materialization_) | test(producer_terminal_race_counts_as_successful_materialization) | test(company_controller_materializes_) | test(vuln_final_seal_)'
cd backend && cargo clippy -p golish-agent-runtime --lib --tests -- -D warnings
cd backend && cargo fmt -p golish-agent-runtime -- --check
```

不运行未获授权的init、precommit或全workspace测试，不修改live DB，不发起真实扫描。

## 任务4：文档与证据

更新`docs/modules/backend/golish-agent-runtime.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`与`docs/modules/INDEX.md`，并在`feature_list.json`/`agent-progress.md`记录RED/GREEN、静态检查和live acceptance仍待同operation continue。运行JSON、唯一in-progress与scoped diff检查。
