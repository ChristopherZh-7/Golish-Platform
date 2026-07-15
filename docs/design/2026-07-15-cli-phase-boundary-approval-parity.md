# CLI / GUI Phase Boundary Approval Parity

> Status: accepted for implementation
> Date: 2026-07-15
> Parent: `2026-07-14-cli-gui-operation-parity.md`

## 问题

真实 `red_team` CLI slice `Scoping → Attack Candidate` 在 Target Intel gate 已通过后停在
`Target Intel → External Attack Surface`。原因不是 DAG 或 gate：GUI 在这里展示 typed
`confirmation` 卡，而 headless `--auto-approve` policy 对所有 confirmation 固定 decline，且
CLI 没有等价的显式 Confirm 输入。operation 因此保持 `waiting/target_intel`，长 slice 无法跨
phase。

## 决策

新增 `--approve-phase-boundaries`，只在同时给出 `--auto-approve` 时有效。它表示本次 CLI
调用显式执行 GUI phase card 的 Confirm 动作：

- 只批准 profile 定义的 `confirmation` phase crossing；
- 不批准 `unit_review`、credentials、freetext、unknown choice；
- `scope_review` 和 subsidiary choice 仍只从 exact `--target` / typed subsidiary flags 派生；
- 未给 flag 时 phase confirmation 继续 fail closed；
- exact resume 也只能由本次显式 flag 批准新的 phase crossing，不改 frozen scope/profile。

因此 GUI 与 CLI 共用同一个 orchestrator、phase map 和 approval coordinator；差异仅是 GUI
点击 Confirm，CLI 传入一个可审计的 typed flag。

## 验证

1. 参数解析拒绝单独使用 `--approve-phase-boundaries`；
2. typed policy 只有在 flag=true 时批准 confirmation；
3. scope/unit/credential/freetext 的 fail-closed 行为不变；
4. 真实公司 acceptance 显式携带该 flag，必须实际进入 EAS 并继续至 Candidate 或暴露下一个
   确定性 blocker。
