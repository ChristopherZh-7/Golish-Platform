# Active Recon 目标范围一次确认设计

> The exact TargetIntel→EAS target-scope review in this design remains current.
> Its statement that other post-Scoping boundaries keep generic confirmation is
> superseded by `docs/design/2026-07-18-scoping-only-routine-human-confirmation.md`.

## 背景

`target_intel` 是被动情报阶段。它可以发现 domain、IP、CIDR、URL 或 wildcard，但这些 provider 派生结果不是客户授权，不能直接成为主动扫描范围。当前流程在 Target Intel Gate PASS 后复用通用 `waiting_approval` 状态，因此 UI 只显示 “Waiting for approval”；即使用户批准通用阶段边界，也不能制造可信目标，运行仍会停在 `ACTIVE_RECON_TRUSTED_TARGET_REQUIRED`。

本设计把这个含混的停点改成一次明确的目标范围确认：展示本次 operation 的 Target Intel 候选，用户确认全部或删除部分行，系统原子固化所选范围，然后直接进入 External Attack Surface（EAS），不再弹第二个通用 phase approval。

## 安全不变量

1. 公司名只是 engagement subject，不是主动扫描授权。
2. 只有当前 operation、当前 engagement organization、当前 `target_intel` 阶段产生的 provider 候选可以进入确认表。
3. 返回列表必须非空，且只能是所展示列表的原样子集；新增、改写、改变类型或重复行一律拒绝。
4. 确认、目标 scope/source 更新和 operation-bound 授权快照在同一数据库事务内完成。
5. 未选择候选标记为 `scope='out'`；选择项标记为 `scope='in'` 且来源升级为 `customer_provided`。
6. 恢复运行时，授权快照必须仍绑定同一个 operation/org，且所选目标与当前可信 Scoping snapshot 完全一致；历史组织目标不能被 company-only launch 借用。
7. DB 读取、候选漂移、空选择、超时、Skip、畸形 JSON、没有交互 coordinator 均 fail closed。
8. 直接从 EAS stage slice 启动时不提供 provider 候选升级捷径；它仍要求调用方预先提供可信精确目标。

## 交互流程

```text
Target Intel Gate PASS
        |
        v
本阶段刷新了 provider target? -- yes --> 展示完整 active target denominator
        |
        no
        v
已有当前调用可信目标? -- yes --> 直接进入 EAS
        |
        no
        v
读取当前 operation/org 的 Target Intel 候选
        |
        v
AskHuman(scope_review) + waiting_target_scope
        |
        +-- skip/timeout/malformed/empty/edited --> HOLD
        |
        v
原子写入 selected in/customer_provided
+ unselected out + operation-bound snapshot + audit
        |
        v
跳过通用 before_active_scan approval，直接进入 EAS
```

## 后端契约

`DbRepoProvider` 增加三个 fail-closed 方法：

- `active_recon_scope_review_candidates(operation_id, organization_id)`：只有 operation 当前 Target Intel window 内确有 refreshed `source='asset_intel'` 行时才打开 review；返回值是该 exact org 当前全部 trusted + asset-intel `scope='in'` target denominator，避免历史 in-scope target 躲在确认表外却进入 EAS。
- `active_recon_scope_review_apply(operation_id, organization_id, request_id, presented, selected)`：锁住 operation row，重读候选并比对 `presented`，校验 `selected` 是原样非空子集，然后原子更新 targets、`operation_state.state_blob.active_recon_target_scope` 和 audit log。
- `active_recon_scope_review_authorized(operation_id, organization_id)`：读取 operation-bound 快照，并要求它与当前可信 Scoping snapshot 完全一致。

canonical identity 使用现有 `canonical_scoping_target` 语义，并包含 target type；domain/wildcard 小写、IP 标准化、CIDR 屏蔽 host bits、URL 使用标准 origin/value 语义。数据库层不接受仅按 value 匹配的模糊更新。

`TaskOrchestrator::two_level_phase_gate` 对 `TargetIntel -> ExternalAttackSurface` 使用专用流程：

- 没有 refreshed provider target 且 trusted target 已存在：放行，并跳过通用阶段审批；
- provider output 扩大了 denominator：无论是否已有 trusted seed，都发一次完整 `scope_review`，确认成功后放行；
- 无 trusted target 且无可确认 provider target：保持 hold；
- 失败：返回 `Held`，UI 保持在 Target Intel；
- 其它阶段边界继续使用原通用审批。

## 前端契约

新增 task progress 状态 `waiting_target_scope`。AI Chat 显示“确认扫描目标范围”，而不是“Waiting for approval”。实际列表继续复用 `AskHumanInline` / `ScopeReviewTable`：删除行表示排除，编辑成新值会被后端拒绝，避免 UI 直接扩大授权边界。

## 持久化与恢复

不新增 migration。授权快照写入现有 `operation_state.state_blob`，schema version 为 1，至少包含 operation id、organization id、request id、presented rows 和 selected rows；同一事务写 `audit_log` action `active_recon_target_scope_approved`。company-only resume 只有在同一 operation 的快照与可信目标完全一致时才可继续。

## 验证边界

本轮按用户要求不运行 `init.sh`、`just precommit` 或全量测试。只运行：

- `golish-agent-kit` 专用 orchestrator 回归；
- `golish-agent-app` target-scope DB bridge 单元/集成回归；
- AI Chat hook/marker 的聚焦 Vitest；
- affected-file format、Biome 与 `git diff --check`。
