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
| `recon` | recon 出站端口（`ReconTargetsPort`/`ReconScansPort`/`ReconDirectoryPort`/… + `Pg*Adapter`）；`ReconTargetsPort::in_scope_values_created_before` 用 `targets.created_at <= cutoff` 给 wave-aware stage 冻结资产轴；`ReconDirectoryPort` 提供 `directory_entries` list / exists / insert；`ReconScansPort` 提供 `api_endpoints` insert / list / count + `api_endpoints_upsert_merge_params`（`ON CONFLICT (target_id,url,method)` 并集合并 params，给 js_extract AI param recipe 用）/ js_analysis / fingerprints / passive_scans |
| `pentest` / `vuln` / `agent` / `platform` | 各服务出站端口 + adapter |

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

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core ports
```
