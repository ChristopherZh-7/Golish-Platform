# Vuln no-purge rollover 终态采用设计

## 背景

旧 Vuln execution `8fcceab2-fe5c-4b76-b795-29008528ddba` 已达到 `180/180`，并持久化了 exact submission `7bf57b10-9af1-43c9-bfa4-d7142566550c`。它因 final seal 的结构性 N/A 物化缺口未能生成 handoff，随后又被旧 binary 错误终态化。现有 legacy recovery 使用通用 `supersede_stage_checkpoint` 建立 replacement execution；虽然 `fact_purge=None` 保留了 facts/evidence，该事务仍把 `operation_state.stage_started_at` 更新为 replacement 时间。

Vuln coverage 将 `stage_started_at` 作为 freshness floor。新的 floor 排除了旧 execution 已完成的 108 个 producer outcome，只留下 72 个由 Enumeration manifest 确定性投影的结构性 N/A，于是 replacement 又生成 `18 origins × 2 capabilities = 36` 个扫描 shard。最后 `https://129.28.12.57:443` 的 broad Nuclei attempt-5 在 300 秒 deadline 后把五个旧终态覆盖为 `partial`，形成 `175/180` 且 `0 shard` 的永久阻塞。

克隆库闭环又暴露了三个只会在 finalizer 路径串联出现的兼容缺口：replacement Unit 自己的 `started_at` 仍晚于旧 terminal evidence，导致 final-seal resolver 与 worklist 使用不同 freshness window；持久化的 provider retry checkpoint 实际为 JSON array，而旧恢复代码按 object 解析并遮蔽原始 final-seal error；结构性 N/A 物化使用 host-only key，把 18 个 `scheme://host:port` origin 折叠为 14 个 host，使 72 个 N/A 只应用 56 个。三者均需 fail-closed 修复，不能通过减少分母绕过。

本设计不修改 schema/migration，不复制旧 submission，不复活旧 worker，也不放宽 Gate。

## 目标

1. 今后的 exact finalizer no-purge rollover 保留原 Vuln stage freshness epoch；replacement 只重建运行壳，不重扫已完成 worklist。
2. 对已经进入 legacy replacement 的现场，确定性重放同 operation 的旧 terminal evidence 到 canonical `technique_outcomes`，恢复原 freshness floor，然后由当前 Controller 生成新的 current-execution submission 并正常 final seal。
3. 新的 `partial/error` attempt 只能作为审计诊断，不得把同 operation/asset/technique 已存在的 terminal canonical truth 降级。
4. 结构性 N/A 仍由现有 exact Enumeration handoff lineage + fresh aggregate attestation 物化；最终 `180 == 180` 完整性检查保持不变。
5. finalizer resolver、coverage snapshot 与 current Unit 使用同一个受证明的 source freshness epoch；精确 HTTP(S) origin 在终态物化中不得按 host 合并。
6. provider retry checkpoint 保留原链数组，runtime 只解开 server-owned wrapper，不猜测或丢弃 chain。

## 安全不变量

### No-purge runtime rollover

- 只接受 same operation、same stage、same organization、same sealed scope snapshot 的 source/replacement。
- source 必须有唯一 exact durable submission，并匹配旧 final submitter、attempt epoch、lease、closed manifest 与 barrier。
- `fact_purge` 必须为 `None`；存在 active external tool、identity/hash 漂移、ambiguous submission 或 scope drift 时 fail closed。
- 旧 execution/unit/worker/output/submission 保持不可变历史；replacement 必须产生新的 plan/worker/submission/handoff。
- 旧 Gate PASS 不是新 pass token。当前 Controller 仍需在 current fence 下重新执行 coverage、materialization、Gate 与 final seal。

### Terminal evidence adoption

- adoption 只在 server-owned Vuln Controller 的 exact current worker fence 内运行。
- legacy marker 必须唯一指向 current replacement 与一个 same-stage source execution；source submission、org、scope snapshot 和 frozen manifest 必须一致。
- 只处理 current canonical `partial/error` cell；每格必须找到 source epoch 内、replacement epoch 前的唯一 latest terminal evidence。
- evidence 必须属于 exact operation/org/target/exact origin/technique，producer 必须与当前 capability 相同，outcome 只能是 `found|empty|blocked|not_applicable`。
- adoption 是 audit-ledger → materialized projection 的重放；不改旧 evidence，不删除新的 partial evidence，不把 timeout 伪造成 checked-empty。
- 任一 cell 缺少 exact terminal witness 时整个 adoption 回滚，保持 Gate BLOCK。

### Canonical monotonicity

- attempt-start `partial` 仅能插入缺行或刷新现有 `partial/error`。
- 若任一 sibling 已是 terminal，整组 attempt-start 以 superseded 返回，网络扫描不得启动。
- 显式 developer reset 先按现有 purge contract 删除受影响 outcome，因此合法新阶段仍可创建新 attempt。

## 状态流

### 今后的 recovery

1. legacy finalizer witness 在事务内通过。
2. 创建 replacement execution/unit，但保留 source `stage_started_at`，记录 `company_finalizer_no_purge_recovery` provenance。
3. 下一次 `stage_run` 的 coverage 直接读取旧 terminal outcomes，worklist 为 `180/180`、`0 shard`。
4. 当前 Controller 关闭 request epoch，生成新的 submission。
5. 物化 72 个 trusted structural N/A，重验 Gate，final seal 生成新 handoff并推进 stage。

### 已发生错误 rollover 的 compatibility recovery

1. 当前 Controller 在 build worklist 前调用 DB adoption。
2. DB 锁定 operation/current execution/unit/plan/worker 和 source recovery witness。
3. 恢复 source stage freshness floor，并从 immutable audit evidence 重放仅有的 unfinished terminal cells。
4. coverage 重新计算为 `180/180`，不创建 scanner shard。
5. 当前 Controller 按正常 current fence 生成新 submission并 final seal。

## 克隆库验收结果

2026-07-20 使用 production `golish` 的只读 repeatable-read snapshot 创建 `golish_gatefix_20260720_d`，复制 180 张 public table 且逐表计数零 mismatch。新 CLI 仅在显式隐藏参数 `--stage-run-test-database golish_gatefix_*` 下选择克隆库，数据库名受小写前缀、字符集和 63 字节上限约束，不能静默选择 production DB。

同 operation `7dcef9f0-17c8-40e9-a5b8-db06a86f9f27` continuation 生成新 submission/handoff，最终 Gate 返回 `PASS`，Unit `e353c601-99d8-4754-b5ca-2374bcf353e7` 为 `passed`，replacement execution `55445b83-ef20-4a68-84ce-e036ee499592` 为 `completed`。canonical outcome 为 180 个 distinct exact-origin technique cell（found 12、empty 78、not_applicable 90），finalizer-only 窗口新增 Nuclei tool call 为 0。source production DB 的 operation freshness、五个 partial/error 和 audit marker 均保持原样，证明验收没有写入 live DB。

## 验收

- 定向 DB 测试：future rollover 保留 freshness floor；legacy compatibility adoption 恢复 exact terminal cells；所有 identity/hash/drift 反例 fail closed。
- technique outcome 测试：terminal row 不被 later partial/error attempt marker 降级，整组 CAS 不产生半更新。
- runtime 测试：adoption 后 worklist `180/180`、shards=0，继续走 PrepareFinal，不调用 scanner。
- 独立数据库：从 Test1 克隆真实历史，使用新 binary/CLI 继续；断言新 submission + 新 Vuln handoff、stage/unit PASS、180 canonical outcomes，且 scanner evidence 计数在 finalizer-only 期间不增加。
