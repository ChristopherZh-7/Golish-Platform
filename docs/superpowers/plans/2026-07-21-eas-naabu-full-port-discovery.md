# EAS Naabu 全端口发现与 Nmap 定向指纹实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把EAS full端口发现从易超时的Nmap Connect全扫切换为固定Naabu Connect `1-65535`，保留严格Gate证明，并让Nmap只对confirmed-open ports做服务指纹。
**架构：** `EasDiscoverPortsTool` 持有profile→scanner/range/rate/deadline的唯一recipe；full生成v2 attestation，read-side从immutable evidence独立重算manifest、receipt、command和stdout完整性。旧Nmap v1 attestation保留只读兼容，数据库schema不变。
**技术栈：** Rust 2021、serde_json、sha2、sqlx/Postgres evidence ledger、Naabu、Nmap、cargo-nextest。

## 修改文件

- `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`：Naabu full recipe、600秒合同、v2 coverage/attestation、Naabu manifest完整性与focused tests。
- `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`：严格读取Naabu v2 terminal proof，同时保留Nmap v1兼容与tamper tests。
- `resources/harness/stages/external_attack_surface/methodology.md`：明确Naabu发现端口、Nmap只指纹开放端口。
- `docs/design/2026-07-21-candidate-tls-port-scan-coverage.md`：标注其Nmap full部分被新设计取代。
- `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/INDEX.md`：同步模块事实源。
- `feature_list.json`、`agent-progress.md`：登记状态与新鲜验证证据。

## 任务1：Naabu full profile RED

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 修改 `port_scan_profile_recipes_are_server_owned_and_bounded`，要求full满足：

```rust
assert_eq!(full.tool_name, "naabu");
assert!(full.tool_args.contains("-p 1-65535"));
assert!(full.tool_args.contains("-s c"));
assert!(full.tool_args.contains("-rate 1000"));
assert!(!full.tool_args.contains("-top-ports"));
assert_eq!(full.timeout_secs, 600);
```

2. 把full完整性夹具改为runner result：成功空stdout和manifest内开放endpoint可完整；foreign endpoint、非canonical输出、非零exit或任一truncation必须不完整。
3. 把full计数夹具改为Naabu `IP:port`，确认只有真实开放endpoint产生LIVENESS，空结果不会伪造host-up。
4. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(port_scan_profile_recipes_are_server_owned_and_bounded) | test(full_port_profile_terminalizes_only_after_complete_target_manifest) | test(full_port_profile_counts_only_real_open_port_liveness)'
```

预期：旧实现仍返回Nmap/1900秒/XML，focused tests按预期失败。

## 任务2：Naabu full profile GREEN

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 为full生成唯一logical recipe：

```rust
format!("-list {{input_file}} -iv {address_family} -p 1-65535 -s c -rate 1000 -timeout 1000 -retries 1 -verify -silent -no-stdin")
```

并设置 `tool_name="naabu"`、`port_scope="tcp-1-65535"`、`complete_for_gate=true`、`timeout_secs=600`。
2. `PortScanProfile::Full` 使用 `profile_version=2` 与 `eas_port_scan_coverage_v2`；quick/standard保持v1 partial。
3. 把 `port_scan_manifest_complete` 改为读取完整runner result：full要求tool success、exit 0、无truncation；stdout每个非空行必须由 `parse_naabu_open_port` 解析且IP属于exact expanded-host manifest。空stdout在这些条件全满足时合法。
4. `apply_port_scan_result_contract`、`attach_port_scan_target_results`、`accept_full_empty_port_landing` 和evidence publication都复用同一个manifest判定，不允许返回层比evidence层更宽松。
5. v2 attestation加入expanded-host `coverage_receipt` 与scanner stdout SHA-256；query中的profile version跟随plan，不再硬编码1。
6. 重跑任务1命令，预期全部通过。

## 任务3：Gate read-side RED

**文件：** `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`

1. 新增 `naabu_full_port_attestation` fixture，构造精确v2 profile、manifest hash、coverage receipt、command、stdout hash和target identity。
2. 修改 `eas_port_terminal_requires_full_scan_attestation`，以 `tool_name=naabu` + v2作为当前terminal证据并要求成功投影。
3. 新增 `eas_port_terminal_rejects_tampered_naabu_v2_attestation`，逐项篡改：scanner、port scope、timeout、command、manifest hash、receipt host/count/completed、truncation、stdout hash、foreign stdout endpoint、target id；每个都必须投影为空。
4. 新增 `eas_port_terminal_keeps_strict_legacy_nmap_v1_compatibility`，旧v1完整XML仍接受，XML under-accounted则拒绝。
5. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app -E 'test(eas_port_terminal_)'
```

预期：旧read-side仅接受Nmap v1，Naabu v2当前证据测试失败。

## 任务4：Gate read-side GREEN

**文件：** `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`

1. 保留现有Nmap v1 validator并拆成独立 helper；新增Naabu v2 manifest hash domain separator：

```text
eas_port_scan_manifest_v2\0full\0profile_version=2\0family=<4|6>\0port_scope=tcp-1-65535\0tool=naabu\0args=<fixed recipe>...
```

2. terminal producer分派只允许 `(nmap,v1)` 或 `(naabu,v2)`；其它tool/schema组合fail closed。
3. v2 validator精确检查profile、同族最多4 host、expanded count、fixed command、600秒、launch/exit/truncation、manifest hash、receipt exact set、每host `1..65535/count=65535/completed=true`、stdout hash与manifest内endpoint。
4. target id必须等于evidence row target id；requested identity必须等于current target value/name之一，继续由既有org/project/scope/asset checks组成完整授权。
5. 重跑任务3命令，预期新旧两类证明通过、全部tamper case拒绝。

## 任务5：方法学和模块卡

**文件：** `resources/harness/stages/external_attack_surface/methodology.md`、`docs/design/2026-07-21-candidate-tls-port-scan-coverage.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/INDEX.md`

1. 把“full Nmap XML manifest”改为“Naabu full v2 receipt”；保留quick/standard partial和宽CIDR policy block。
2. 明确 `eas_fingerprint_services` 的Nmap只扫描DB confirmed-open ports，不再承担全端口发现。
3. 在旧设计头部标注仅端口full/v1部分被新设计取代。
4. 模块卡写入Naabu v2/Nmap v1兼容、fixed rate/deadline、read-side tamper checks；INDEX状态保持✅并更新日期说明。

## 任务6：定向验证与收尾

**文件：** `feature_list.json`、`agent-progress.md`

1. 所有Cargo命令前运行：

```bash
just space-guard
```

2. 运行focused tests：

```bash
cd backend && cargo nextest run -p golish-pentest-app -E 'test(port_scan_profile_recipes_are_server_owned_and_bounded) | test(incomplete_port_profile_never_terminalizes_port_coverage) | test(full_port_profile_terminalizes_only_after_complete_target_manifest) | test(full_port_profile_counts_only_real_open_port_liveness) | test(full_empty_manifest_is_valid_zero_record_landing) | test(full_port_profile_accepts_only_bounded_cidr_and_one_address_family) | test(port_scan_result_reports_coverage_and_next_action)'
cd backend && cargo nextest run -p golish-agent-app -E 'test(eas_port_terminal_) | test(eas_target_bound_evidence_requires_producer_and_current_org) | test(eas_terminal_outcome_ref_must_match_guarded_audit_quadruple)'
```

3. 运行scoped quality checks：

```bash
cd backend && cargo clippy -p golish-pentest-app -p golish-agent-app --lib --tests -- -D warnings
cd backend && cargo fmt -p golish-pentest-app -p golish-agent-app -- --check
jq empty feature_list.json
git diff --check
```

4. 把命令、退出码、test run id和关键结果写入 `agent-progress.md`。只有focused tests、Clippy、rustfmt、JSON和diff全绿后才将feature从`in_progress`改为`passing`并填写evidence；未获授权的init/precommit/full workspace测试明确记录为未运行。
5. 本轮不对外部目标发起验收扫描，也不停止当前GUI的Nmap/Naabu进程；新binary需后续重启GUI后才会用于下一次full调用。

## 计划自检

- 规格覆盖：Naabu full、Nmap open-port-only、600秒合同、v2 producer/read-side proof、partial fail-closed、旧v1兼容、方法学和focused验证均有对应任务。
- 类型一致性：统一使用 `eas_port_scan_coverage_v2`、`eas_port_scan_attestation_v2`、profile version 2、scanner `naabu` 和timeout 600。
- 授权边界：复用既有target guard/CIDR containment，不改schema/migration，不新增raw参数或外部验收扫描。
- 无占位符：所有命令、函数职责、fixed recipe、hash domain和tamper维度已明确。
