# golish-app-core / ports

> **一句话职责**：provider 端服务 ports（servitization S1-2）——每个服务一个出站端口：`*Port` trait（remote-ready 契约，只可序列化参数、无 `PgPool`/闭包）+ in-proc `Pg*Adapter`（**唯一**允许调该服务 `golish_db::repo` 的地方）；消费方持 `Arc<dyn *Port>`，绝不直碰别家 repo。

- **类型**：目录模块（属于 crate [`golish-app-core`](../golish-app-core.md)）
- **路径**：`backend/crates/golish-app-core/src/ports/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何**跨服务**读写（一个服务要读另一个服务的数据）时——必须经端口，不直碰别家 repo
- 改 `*Port` trait 契约或 `Pg*Adapter`（repo 唯一调用点）时

## 职责

把横向（服务间）耦合收敛成端口。每个子模块一个服务的出站端口：trait 定义 remote-ready 契约（只序列化参数），`Pg*Adapter` 是唯一允许 `golish_db::repo::<that service>` 的地方。消费方注入 `Arc<dyn *Port>`。设计见 `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`。

## 公开接口

| 子模块 | 说明 |
|---|---|
| `recon` | recon 出站端口（`ReconTargetsPort`/`ReconScansPort`/`ReconDirectoryPort`/… + `Pg*Adapter`）；`ReconTargetsPort::target_add` 收到 manual/imported/customer-provided/CLI 等 trusted intake 时，会把同 project/org/type/value 的 discovery Target 原地升级来源，既不制造重复 Target，也不跨 org/project 接管；scope=out 不会被重新激活。`ReconTargetsPort::in_scope_values_created_before` 用 `targets.created_at <= cutoff` 给 wave-aware stage 冻结资产轴；`ReconDirectoryPort` 的 target-bound list 与 `ReconAssetsPort` / `ReconScansPort` 的 target-bound list/count/stats adapter 都走 repo current-owner reads，只返回 child project 仍匹配 current in-scope target 的行；`ReconDirectoryPort` 另提供显式 project list / exists / insert，`ReconScansPort` 另提供 `api_endpoints` insert + params merge / js_analysis / fingerprints / passive_scans。Active Enumeration 的 JS/browser/route producer 必须调用 `*_guarded` 变体并传 `TargetWriteGuard`，让 adapter 在同一短事务锁 target raw snapshot 后写业务行，不能用旧的 unguarded 方法替代。长时 route producer 进一步使用 `directory_entry_add_guarded_if_attempt_current`，同时传 `TechniqueOutcomeAttemptGuard + run/origin/DIR`；adapter 返回 `Applied|Superseded`，并在同一短事务锁 target、operation epoch、engagement subtree 与 current generation 后才写 `directory_entries`。 |
| `pentest` / `vuln` / `agent` / `platform` | 各服务出站端口 + adapter；`vuln::WikiKbPort::wiki_get_page` 按 normalized relative path 从与 wiki search 相同的持久索引读取 exact page，避免 packaged runtime 的 filesystem root 与已索引 corpus 漂移 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 5 个服务端口子模块声明 |
| `recon/` / `pentest/` / `vuln/` / `agent/` / `platform/` | 各服务 `*Port` trait + `Pg*Adapter` |

## 依赖

- `async-trait`、`golish-db`（仅 adapter 内）；消费方只依赖 trait

## 注意事项 / 坑

- **铁律**：app 服务**横向**读写必须走端口（`Arc<dyn *Port>`），**禁止**直接 `golish_db::repo::<别家>`；这是 ALLOWLIST 从 28→0 的成果，别开倒车。
- `*Port` trait 必须 object-safe（remote-ready）：只序列化参数，无 `PgPool`/闭包/泛型。
- `TargetWriteGuard` 是可序列化的 DB-layer ownership witness；guarded recon port 只负责一次短 DB transaction，严禁把浏览器/HTTP/LLM 等长耗操作包进 target row lock。
- wiki exact-page read 仍受既有 repository/provider 组合边界约束：app-core adapter 只等值转发路径并返回序列化 `WikiPage`，不把 DB handle、project filesystem root 或跨组织 ledger 读取权交给 cognition worker。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core ports
```
