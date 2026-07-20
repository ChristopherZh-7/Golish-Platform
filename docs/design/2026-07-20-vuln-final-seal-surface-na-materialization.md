# Vuln 最终封存的可信 Surface N/A 物化设计

## 问题

Vuln coverage 会依据 final-sealed Enumeration surface manifest，对没有可执行 GET query 参数或没有 HTTP endpoint 的 exact Web Origin，确定性生成四类 `not_applicable`：`WSTG-INPV-05`、`WSTG-INPV-01`、`WSTG-INPV-12`、`WSTG-ATHN-04`。这些 cell 同时带 `source=enumeration_surface_manifest` 与 `details.authority=enumeration_surface_manifest`，per-org Gate 会把它们作为 backend-owned terminal truth。

V2 final seal 则要求每个 terminal coverage cell 都有一条 exact `(organization_id, operation_id, asset, technique)` `technique_outcomes` 行。现有 Gate-PASS terminal materialization 只处理 Target Intel/EAS 的 model-submitted `blocked/not_applicable`，并明确跳过 Vuln。因此 coverage 可以达到 180/180、Gate PASS，但 raw outcome catalog 只有扫描 producer 写入的 108 行；缺失的 72 个可信 surface N/A 令 final seal 按 180 != 108 fail closed。

## 决策

在 Company Controller Gate PASS 与 final-seal catalog 读取之间，增加一个窄的 Vuln backend-derived terminal materialization 分支。

1. 必须从 exact operation/org/chat-session 的新鲜 Vuln coverage snapshot重新读取 authority；不读取 model deliverable 来决定 Vuln 写入。
2. 复用 `trusted_vuln_surface_not_applicable_from_snapshot` 验证 exact canonical origin、固定 technique allowlist、`state=not_applicable`、`source=enumeration_surface_manifest` 与 matching authority details。
3. 只物化上述可信 cell，outcome固定为 `not_applicable`，source固定为 `enumeration_surface_manifest`，query保留 server-authored note。不能直接把 Enumeration旧 evidence ids挂到Vuln行：它们早于Vuln freshness floor，stage fork时还可能属于source operation。运行时改为以exact final-sealed Enumeration handoff作lineage，仅接受普通同operation的`deliverable_final_seal`或冻结fork输入的`stage_fork_final_seal`，在当前Vuln operation/org/Unit/chat session/project下追加一条新鲜的backend aggregate attestation evidence；所有可信N/A行共享这个正数id。
4. outcome run id必须是 durable operation id；coverage session id仍是当前真实 chat evidence session。二者不得混用。
5. 写入复用 `upsert_terminal_technique_outcome_if_unfinished`：缺行或 partial/error可补齐；既有 found/empty/blocked/not_applicable producer truth赢得竞争且不被覆盖。
6. 任一 snapshot envelope、authority、note或 DB write失败都阻止 final seal；保留现有“一条 materialized canonical outcome 对应一个 terminal cell”的全量相等检查。

## 不变边界

- 不修改 Gate terminal集合、final-seal 180=180完整性约束、DB schema/migration、IPC或前端类型。
- 不把 model-authored Vuln coverage、普通 Nuclei错误、超时、partial/error或任意 source的 N/A写成终态。
- 不给 surface N/A伪造、任意挑选或跨stage直接复用旧 evidence id；旧Enumeration evidence集合只作为新鲜attestation raw lineage metadata。缺 handoff、scope/operation/org不一致、空/非法/超限source evidence、attestation append失败或返回0都fail closed。
- Target Intel/EAS既有 submit-exception materialization语义保持不变；Enumeration仍不做这类终态物化。

## 当前 operation恢复

升级后的 backend 对 operation `7dcef9f0-17c8-40e9-a5b8-db06a86f9f27` 发一个独立“继续”请求。运行时复用已保存的 Vuln submission，重新读取 180/180 snapshot，幂等补齐 72 条可信 N/A，再构建 final seal。无需重跑任何 Nuclei shard，也无需重启 Scoping、EAS或Enumeration。

## 定向验证

- 纯函数：Vuln只提取带完整 manifest authority的固定 N/A；拒绝 forged source/authority、unsupported technique、非canonical origin和空 note；完全忽略 deliverable中的 model N/A。
- 异步 materialization：Vuln snapshot先校验stage/org/session envelope并使用exact operation id + chat session；attestation facts=None且绑定当前Unit/project，raw保存handoff identity/hash/scope/gate time/source evidence与排序后的N/A cell集合；写入使用operation run id + manifest source + fresh attestation id。append/upsert失败均fail closed，既有terminal producer row竞争胜出算成功。
- helper contract：Company Controller只为 Target Intel/EAS返回chat run id，为Vuln返回operation run id，Enumeration保持None。
- 回归：现有 final-seal完整 outcome-set测试、Target Intel/EAS terminal materialization测试继续通过；受影响 crate Clippy零warning、rustfmt通过。
