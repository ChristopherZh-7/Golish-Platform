# golish-recon-app / organization_recon

> **一句话职责**：组织级 recon 编排原语——分阶段 runner（active/persistence/export/runner/state）+ artifact 与归一化记录契约（`NormalizedReconRecord`），让 asset-intel adapter 用同一 evidence 格式。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/organization_recon/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改组织级 recon 的分阶段运行（stage runner、状态快照、导出）时
- 改归一化 recon 记录/artifact 契约（`NormalizedReconRecord`/`OrganizationReconStageName`）时

## 职责

组织级 recon 的编排原语：`runner` 跑分阶段流程、`state` 持 `OrganizationReconState` + 运行快照、`active`/`persistence`/`export` 各阶段动作、`normalize`/`artifacts`/`types` 定义归一记录与产物契约（供 asset-intel adapter 复用同一 evidence 格式）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `OrganizationReconState` | 运行时状态 |
| `commands::*` | 组织 recon Tauri 命令（含 `recon_backfill_real_ip`：从已有 `dns_records` A 记录回填 `targets.real_ip`，IP-centric 树 Phase 0） |
| `ORGANIZATION_RECON_EVENT` | 进度事件名 |
| `NormalizedReconRecord` / `OrganizationReconRunSnapshot` / `OrganizationReconStageName` / `OrganizationReconRunStatus` / `OrganizationReconExportResult` | 归一记录 / 快照 / 阶段 / 状态 / 导出 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `runner.rs` / `state.rs` | 分阶段 runner / 状态 |
| `active.rs` / `persistence.rs` / `export.rs` | 各阶段动作 |
| `normalize.rs` / `artifacts.rs` / `types.rs` | 归一 / 产物 / 类型 |

## 依赖

- crate 内 `organizations`/`asset_intel`；`golish-db`、`golish-core`（事件）

## 注意事项 / 坑

- 归一记录/artifact 契约要与 asset-intel adapter 共用——改契约会影响两边 evidence 格式一致性。
- 分阶段 runner 长耗 + 可取消；进度经 `ORGANIZATION_RECON_EVENT` 发前端。
- `persistence.rs::land_dns_records` 落 `dns_records`：A/AAAA 走系统 stub resolver（`tokio::net::lookup_host`），**CNAME/MX/TXT 走 hickory-resolver**（`tokio::net::lookup_host` 只回 A/AAAA）。因为没有任何 stage 主动跑 dig（target_intel 禁 scan-tool fallback、EAS 复用继承 DNS），这里是 CNAME/MX/TXT **唯一**的采集点；DNS 查询打解析器不碰目标主机，仍属 zero-touch。每次查询独立 bounded（3s timeout）+ 非致命；resolver 构造失败时仍落 A/AAAA。`refresh_per_asset_landing_summary` 还会返回确实尝试解析但无 A/AAAA/CNAME/MX/TXT 的 `dns_empty_hosts`，由有 evidence id 的 app bridge 写成 `technique_outcomes(GOLISH-INTEL-DNS, empty)`，避免“已查为空”和“未查”混淆。
- `real_ip` 只认地址记录：首个 A、否则首个 AAAA（**绝不**用 CNAME/MX/TXT 的 value，否则会污染 `targets.real_ip`）。设计 2026-06-15 Phase 0；存量数据用 `recon_backfill_real_ip` 命令一次性回填（不重新解析）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app organization_recon
```
