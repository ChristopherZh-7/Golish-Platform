# Stage purge 原子性与 technique 精确作用域

- 日期：2026-07-10
- 状态：已实施（focused verification 通过）
- 关联：`docs/design/2026-06-30-stage-reset-full-purge.md`

## 问题

`restart_from_stage_purge` 当前有两个破坏性边界缺口：

1. `purge_vuln_domain` 按 organization subtree 删除全部 `technique_outcomes`，会误删所选阶段祖先已经完成的技术结果；并且只在受影响阶段包含 vuln domain 时才执行，无法稳定表达“受影响阶段的 technique 集合”。
2. data-domain、completion ledger、wave 和 target status 的 SQL 分别直接通过 `PgPool` 执行。任一中间 SQL 失败时，之前的删除已经提交，stage reset 会留下不可重放的半清理状态。

## 决策

- `technique_outcomes` 的删除集合由 `affected stages` 对应的 embedded `StageSpec.expected_techniques` 做并集得到；SQL 同时限定 `organization_id = ANY(...)` 和 `technique = ANY(...)`。空 technique 集合严格 no-op。
- 删除 technique outcome 是 cross-stage purge 步骤，不再藏在 vuln data-domain 中。这样 Enumeration、EAS、TargetIntel 或攻击阶段重置都使用同一确定性规则。
- 所有事实域 SQL、`org_stage_completions`、`stage_asset_waves` 和 `targets.status` 回滚使用同一个 `sqlx::Transaction`。repo executor 接受 `&mut PgConnection`；command 层在执行前 begin，只在所有步骤成功后 commit，任一步失败显式 rollback。
- organization subtree 和 project path 是只读的 purge plan 输入，可在 transaction 前解析；它们不改变业务事实。checkpoint/cursor 仍沿用现有路径，本次至少保证破坏性的事实清理自身原子。
- 不改 schema，不清 `audit_log`，不执行真实 purge。

## 不变量

- 只作用于 operation engagement organization subtree。
- 只删除 affected stage specs 声明的 technique id，祖先 stage outcomes 保留。
- 任一事实 SQL 失败时，事实、台账、wave 和 target status 均保持 purge 前状态。
- 返回的 `StagePurgeCounts` 只在 transaction commit 成功后对调用方可见。

## 验证

- SQL-shape 单测锁住 organization + technique 双重过滤及空集合 no-op。
- command 单测锁住 affected embedded specs 的 technique union，不包含祖先 technique。
- focused nextest、fmt、clippy；不调用真实 reset/purge command。
