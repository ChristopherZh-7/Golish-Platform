# golish-reporting-app

> **一句话职责**：以 ports 编排 REPEATABLE READ source snapshot、确定性 validation、claim-set fenced narrative、redaction 与显式用户 publication；不拥有 SQL、具体文件系统实现或 LLM 工具。

- **类型**：Rust crate（L3 domain service）
- **路径**：`backend/crates/golish-reporting-app/`
- **状态**：✅ C9 已实现

## 职责

- `ReportTruthPort` 要求 build 的 source、claim、typed evidence/blocked-decision authority、frozen scope 与 Cleanup closeout 全部来自同一个 REPEATABLE READ READ ONLY snapshot，并在持久化/发布前重跑同一完整 canonical source 查询。
- `ReportReadModelBuilder` 在 snapshot/current exact match 后才持久化 validated revision；port 返回持久化后的 CAS `row_version`，stale 时不留半成品。
- Narrative renderer 只按已有 claim id 回填文案；新增、删除、重复 claim id 均拒绝。无 renderer 可走 deterministic template。
- `ReportFinalizer` 先按 deterministic content key 去重并稳定排序，再在 DB transaction 外执行 stage→content-address promote→read-back verify，避免同批 duplicate self-lock 与逆序并发 ABBA；返回值仍严格保留调用方输入顺序和重复项。每个唯一 artifact 的 `ArtifactPublicationReservation` 保留到短事务锁定 current revision 及其 manifest/section/claim/citation/evidence、重验 source exact match、claim/citation hashes、validation attestation 与 Cleanup closeout并写至少一个 artifact ref + exact final outbox 后才释放。DB deferred constraint 会在 commit 再核对 exact current、active local principal、artifact 与 outbox，`final` 不能被普通 SQL enum flip 伪造。
- publication 必须有 server-owned principal 和显式确认；Reporting stage 不可 auto-finalize。
- DB rollback 后 content blob 作为 grace-period orphan 留给 GC；不能在 DB transaction 内做文件、LLM 或 HTTP。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ReportTruthPort` | 完整 source snapshot 与 validated revision 持久化边界 |
| `ReportArtifactStore` / `ArtifactPublicationReservation` | content-addressed stage/promote/verify/discard/GC 边界；promote 返回 lease，finalizer 持有至 DB attach 完成 |
| `ReportPublicationPort` | 短事务显式 publication 边界；source/citation/attestation/Cleanup 必须在同一 DB snapshot 内重验 |
| `NarrativeRenderer` / `apply_narrative` | 无工具 renderer 与 claim-set fence |
| `ReportFinalizer` | 文件在事务外、DB publication 在短事务内的两阶段编排 |
| `BuiltReportRevision` / `ReportReadModelBuilder` | 带 expected-current revision fence 的 build→validate→persist 编排 |

## 依赖 / authority

- 依赖 `golish-reporting-domain`；基础设施 adapter 放在上层 composition/agent app。
- PostgreSQL adapter 与 IPC 入口在 `golish-agent-app` 中重新解析 server-owned project authority，build/persist/finalize 均拒绝 retired、path rebind、跨 project/operation 或未 final-sealed 的 scope；model 不接受 caller-supplied 路径作为 authority。
- Cleanup closeout 必须消费 Cleanup-owned port；不能在 Reporting 复制 status SQL。
- TechniqueOutcome 只接受 final-sealed、未 invalidated `stage_handoffs.canonical_fact_refs` 与 canonical row 形成双向一一对应的 exact composite row/content/evidence；handoff 本身也进入 source manifest。自由文本 `run_id` 不能单独作为 ownership。
- `ReportRevisionFinalized.v1` 当前只留 immutable pointer event；没有真实 consumer 时不生成 placeholder delivery，也不路由 Assertion/Document/Embedding/Graph。
- 具体 artifact store 由 `golish-projects` + `golish` 组合：Unix 使用 dirfd/`*at`，Windows 使用 capability handle，两端共享 content-addressed reservation/GC 语义。Windows backend 已经 GNU target 交叉编译、Clippy 与 test binary 链接验证；当前 macOS host 未执行 Windows runtime tests，需由 Windows CI/真机覆盖运行期行为。

## 测试入口

```bash
just space-guard
cd backend
cargo nextest run -p golish-reporting-app --no-tests=fail
# 真实 DB authority、IPC 与 Candidate→Report replay closeout：
cargo nextest run -p golish-agent-app \
  --test reporting_authority \
  --test reporting_ipc_authorization \
  --test v2_closeout_replay \
  --no-tests=fail
```
