# AU 第三代有界恢复、输出余量与 CLI exact-resume 设计补充

> 本文补充 `2026-08-01-au-exhausted-runtime-no-purge-recovery.md`；原设计的 no-purge、exact identity 与 fail-closed 边界保持不变。

## 现场问题

Test1 的第一次换代留下了 source/replacement 均为 generation 1 的历史兼容形状，后续 Continue 因 replay 要求 `source + 1` 而失败。修复 generation 后，包含 38 条 route 的 `web_origin` 分片又在 8192 token 输出预算内只产生说明文字、没有调用 `submit_result`。此外，CLI 的普通 V2 preflight 要求每个 Team Unit 都有 leader Worker，使已经关闭且 leader 被 supersede 的 AU 恢复壳无法进入专用恢复事务。

## 决策

- AU response-non-contract 恢复由数据库事务拥有 generation：replacement 必须是 source generation + 1，并在同一事务持久化恢复 marker；最多允许 generation 1、2、3，之后确定性拒绝。
- 兼容已经产生的“generation 1 replacement + 缺 marker”历史形状，但只允许精确修复为 generation 2；不删除或改写 source Worker/output/evidence。
- AU shard modeler 与 company synthesizer 的最大输出统一为 32768 token。工具面仍只有 `submit_result`，schema、轮次和 durable retry fuel 不变。
- CLI 仅对精确的 AU exhausted recovery shell 放宽“必须存在 leader Worker”的读侧 preflight：Unit 必须 gate-blocked、plan closed、leader item 是 server-seeded superseded、所有 required item terminal 且至少一个 exhausted、全部 Worker terminal 且无 lease/active tool/final submitter。任何其它 no-leader 状态继续 fail closed；真正 mutation 前仍由 runtime/repository 重新校验全部条件。

## 不变量

- 不改 schema/migration，不 purge target、scan fact、evidence、handoff 或 immutable output。
- 不允许无限 provider 重试；第三代再次耗尽即停止。
- CLI 放宽不是恢复 authority，只是让严格匹配的壳进入拥有事务校验的 AU runtime。
- Gate PASS 仍只来自现有 Application Model finalizer 和关系型 authority。

## 验收

- focused nextest 覆盖 generation 0→1→2、legacy repair→2、2→3 与最终 PASS。
- defaults 测试锁定两个 AU terminal agent 的 32768 token 上限。
- CLI preflight 纯测试证明只接受精确静止壳。
- Test1 使用真实 CLI/provider 执行 generation 3，两个 shard、company synthesis、deliverable 和 Gate 全部 PASS，并发布 AU handoff。
