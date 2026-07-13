# golish-reporting-domain

> **一句话职责**：定义只由 canonical facts、final-sealed authority、evidence citations 与 frozen cleanup truth 构成的 Report Read Model，以及 validation/publication 双轴和 fail-closed validator。

- **类型**：Rust crate（L1 纯领域契约）
- **路径**：`backend/crates/golish-reporting-domain/`
- **状态**：✅ C9 已实现

## 职责

- 用 `ReportSourceVersion(kind, canonical row id, row_version, content_hash)` 表示 reportable source。
- 对完整、稳定排序的 source list 计算 `source_set_hash`；新增、更新、删除或失效任一 source 都会使 finalize stale。
- 被 canonical fact 引用的 `audit_log` evidence 本身也是 `EvidenceAudit` source；完整 body、run/role 与 frozen organization metadata 进入 source hash，不能只冻结 evidence id。
- TechniqueOutcome 的 authority 同时冻结 exact `StageHandoff` final seal；missing row、重复 ref、row/hash/evidence drift 均不能形成 source snapshot。
- blocked Cleanup residual 只能从 typed `CleanupBlockedDecisionTruth` 投影 actor、reason、residual 与该 decision 的完整 evidence membership；obligation-creation evidence 不能充当 blocked 决策引用。
- `ReportClaim` 只保存 typed value 和 citation ids；evidence body 不复制进 claim。
- validator 要求每个事实 claim 有同 frozen organization、同 revision、manifest 内 source且可解析到 typed `EvidenceAuditTruth` 的 citation；evidence 必须 exact operation、`audit_role=evidence`、单一 canonical owner，并与 manifest 中的 audit id/hash 完全一致。
- Candidate 只有经 current verified Finding lineage 才能进入 Findings；blocked/waived cleanup 必须披露 residual。
- Cleanup `missing_obligation_count`、`nonterminal_obligation_count`、`undisclosed_residual_count` 或 `invalid_terminal_truth_count` 任一非零均 fail closed。
- `ValidationStatus` 与 `PublicationStatus` 正交：Gate 认 current validated revision，是否 final 不改变 validation attestation。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ReportReadModel` / `ReportSourceSnapshot` | 冻结 scope、claims/citations 与完整 canonical source set |
| `canonical_source_set_hash` | 长度前缀 canonical source 编码的 SHA-256 |
| `validate_report` | citation、scope、secret、lineage、cleanup、snapshot/current 双轴确定性检查 |
| `EvidenceAuditTruth` | evidence ledger 的 exact run/role/org/source authority；缺失、跨组织或 hash/source 未绑定时 fail closed |
| `CleanupBlockedDecisionTruth` | retained blocked decision + actor/reason/residual + exact decision/evidence membership authority |
| `ReportRevision` | validation/publication 双轴历史 revision |

## 不变量 / 坑

- RAG、Memory ContextPack、wiki、Graph/KG 都不能生成 claim，也不是 Gate authority。
- citation label 只用于展示，不是 source truth；必须解析 canonical row version/hash + evidence id。
- `EvidenceAudit` 不能作为 manifest 外的旁路引用；audit body 或 ownership metadata 漂移必须改变 complete source-set hash。
- `CleanupBlockedDecision` manifest source 与 typed truth、blocked residual claim、citation evidence set 必须双向完全绑定；不能回退到 `CleanupObligation` creation evidence。
- `StageHandoff` 与其 TechniqueOutcome canonical ref 都是 authority 链的一部分；仅有自由文本 run id 或仅有 outcome row 都不足以进入 Reporting。
- `source_set_hash` 覆盖完整 reportable source set，不能只 hash renderer 消费的 manifest 子集。
- secret value 不能进入 read model、renderer prompt 或 artifact。

## 测试入口

```bash
just space-guard
cd backend
cargo nextest run -p golish-reporting-domain --no-tests=fail
# PostgreSQL authority 与 IPC 边界的集成验证：
cargo nextest run -p golish-agent-app \
  --test reporting_authority \
  --test reporting_ipc_authorization \
  --no-tests=fail
```
