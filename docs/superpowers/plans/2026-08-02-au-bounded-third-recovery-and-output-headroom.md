# AU 第三代有界恢复与实体 CLI 收口计划

## 目标

在不清除 Test1 既有事实/evidence、不改 schema 的前提下，修复 generation replay、AU 大分片输出截断及 exact CLI preflight 阻塞，并用真实 CLI 将当前 Application Understanding 跑到 Gate PASS。

## 执行步骤

1. 在 `golish-db` 的 AU 专用 compound recovery 中统一 source/replacement generation，持久化最多三次的恢复 fuel，并兼容精确的历史 generation-1 replacement。
2. 在 `golish-agent-app` 严格校验 recovery marker，覆盖 generation 0→1→2→3、legacy repair、耗尽与最终 PASS。
3. 将两个 AU bound terminal agent 的输出预算统一为 32768，保持唯一 `submit_result` 工具锁。
4. 在 CLI V2 loader 增加纯 classifier，只让完全 terminal、无 lease/tool/final submitter 的 AU exhausted shell进入 runtime 复验。
5. 依次执行受影响 crate 的 focused nextest、Clippy、rustfmt/diff 检查；每个 Cargo 前运行 `just space-guard`。
6. 构建 CLI，对 exact Test1 session/task/operation/org/stage 执行 Continue；用 `run_tree.py --db` 和数据库 smoke summary确认 generation 3、revision、Gate PASS 与 handoff。
7. 更新 feature、progress、模块卡与索引；不运行未授权全仓门禁，不 stage/commit/push。
