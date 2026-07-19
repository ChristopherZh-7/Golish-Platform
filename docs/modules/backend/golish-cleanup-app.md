# golish-cleanup-app

> **一句话职责**：Cleanup P7b 应用服务与 production adapter——obligation/absence/waiver 内核、DB-global recovery worker、Gate truth 与两阶段组织删除端口。

- **类型**：crate（Layer 3 · application service）
- **路径**：`backend/crates/golish-cleanup-app/`
- **状态**：✅ C5/P7a kernel + C8/P7b closeout 已实现（共享树待完整 precommit）

## 何时该读这张卡

- 接入 Post-Exploit side-effect prepare transaction 时
- 修改 cleanup claim/reconcile/absence/waiver adapter 时
- 开放 Cleanup Tool 或 worker 前确认 P7a 边界时

## 职责

- `CleanupObligationPort::record_action_and_obligation` 是 side-effect canonical prepare 的原子端口。
- `PgCleanupRepository` 验 server principal、sealed operation/project/org-at-time、exact evidence 后调用 `golish-db` compound repo。
- waiver 只接受 opaque trusted principal、完整 frozen `operation/project_scope/snapshot/org` identity 和 expected row version；本地 command 与 repo 都重验，不能只凭 model-visible obligation UUID 授权。
- `CleanupCloseoutPort` 是 Reporting/Cleanup Gate 的唯一 closeout truth 读口；除 missing/nonterminal/undisclosed 三计数外，还返回 relationally-invalid terminal truth 计数与已披露 residual obligation ID 集合，上层不得复制 Cleanup status SQL。
- `CleanupCloseoutRuntime` 以 DB-global lease 回收过期 attempt 并推进组织 deletion job；文件 I/O 委托 adapter 且只发生在 claim transaction 提交后。若进程在 artifact cleanup 成功提交后、hard delete 前退出，下一 worker tick 会恢复该 DB-only continuation。
- `OrganizationDeletionPort` 在 adapter 内解析 C0 local principal，并要求 caller 提交当前 active workspace path witness；DB 将其解析为 server-owned active `project_scopes.canonical_project_path` 后校验完整 subtree，caller/model 不传 actor id 或 artifact root。
- deletion request 冻结 subtree/target 后，DB trigger 同时把 organization 与其 target identity 设为只读；重叠 parent/child deletion job 被拒绝，避免 committed artifact plan 漂移。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `CleanupObligationPort` | action+obligation 与 waiver 的 trusted transaction boundary |
| `PgCleanupRepository` | production exact-scope adapter |
| `CleanupKernel` | side-effect prepare / waiver application service |
| `validate_independent_absence` | executor/verifier/evidence/resource hash 独立性检查 |
| `reconcile_action` / `may_reclaim` | lease-expiry 与 retry 决策纯函数 |
| `CleanupCloseoutPort::closeout_counts` | missing/nonterminal/undisclosed residual/invalid-terminal DB truth |
| `CleanupCloseoutRuntime` / `OrganizationArtifactCleaner` | DB-global claim/recovery 与 transaction 外 artifact cleanup |
| `OrganizationDeletionPort` | trusted 两阶段删除 request boundary |

## 依赖

- **内部**：`golish-cleanup-domain`、`golish-post-exploit-domain`、`golish-db`
- **外部**：`async-trait`、`chrono`、`serde`、`serde_json`、`thiserror`、`uuid`

## 注意事项 / 坑

- transaction 只登记 canonical action/obligation；任何外部副作用必须在 transaction 外执行。
- exact replay 不得追加 evidence 或改变 resource/strategy/deadline。
- Cleanup Tool 只接受 obligation id；execute/absence 在没有 typed executor/verifier 时 fail closed，waiver tool 只能建议。
- 组织删除固定为 invalidation deliveries → artifact cleanup lease → 独立 hard-delete transaction；不得在 request transaction 做文件 I/O。每个 invalidation 冻结 event-catalog projector manifest，manifest 外后来新增的 pending projector 不得反向阻塞；artifact/hard-delete 失败写 durable retry-not-before，claim 只选已到期 job 并按 requested time 公平排序，不能让最老失败 job hot-loop 饿死其它 ready job。
- `OrganizationDeletionPort::request_organization_deletion` 会把 DB 的 active stage-fork admission conflict 保留为 typed `CleanupError`；上层可直接指导用户先终止阶段任务，不能先创建 deletion job、清理 artifact 后才依赖 hard-delete trigger 失败。
- P2 repo 暂保留兼容字段 `target_live_id`；migration trigger 原子双写 authoritative `target_id_at_time` / nullable `live_target_id` / canonical snapshot，并用约束保证两个 live alias 只能指向同一 at-time target。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-cleanup-app --status-level fail
cd backend && cargo nextest run -p golish-db --test cleanup_obligation_kernel --status-level fail
```
