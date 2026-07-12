# golish-recon-app / organization_recon

> **一句话职责**：组织级 recon 编排原语——分阶段 runner（active/persistence/export/runner/state）+ artifact/归一化记录契约，并与 asset-intel landing 协同完成本轮 concrete host 的 DNS/subdomain/service durable handoff。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/organization_recon/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改组织级 recon 的分阶段运行（stage runner、状态快照、导出）时
- 改归一化 recon 记录/artifact 契约（`NormalizedReconRecord`/`OrganizationReconStageName`）时

## 职责

组织级 recon 的编排原语：`runner` 跑分阶段流程、`state` 持 `OrganizationReconState` + 运行快照、`active`/`persistence`/`export` 各阶段动作、`normalize`/`artifacts`/`types` 定义归一记录与产物契约（供 asset-intel adapter 复用同一 evidence 格式）。asset-map 的 Target identity 先由 `asset_intel::landing` 直接落库，再把当前 invocation 的 concrete hosts 交给本模块写 DNS 与 `target_assets(subdomain)`；exact-host `target_assets(service)` 由相邻的 `asset_intel::landing::land_service_assets` 写入。两条路径都不能从累计历史重建“本轮发现”。

## 公开接口

| 符号 | 说明 |
|---|---|
| `OrganizationReconState` | 运行时状态 |
| `commands::*` | 组织 recon Tauri 命令（含 `recon_backfill_real_ip`：从已有 `dns_records` A 记录回填 `targets.real_ip`，IP-centric 树 Phase 0） |
| `PerAssetLandingSummary` | target-intel per-asset refresh 的 typed DNS 汇总（records / found / partial / empty / error hosts + refresh failure） |
| `land_target_intel_coverage` | current-run concrete hostname/subdomain 关系落库；返回实际 business row 计数（service handoff 由调用方的 `asset_intel::landing` 完成） |
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
- `persistence.rs::land_dns_records` 落 `dns_records`：Hickory 可用时显式分别查 A/AAAA，并识别 typed `NoRecordsFound`；macOS 默认 resolver 若含 `%interface` scoped link-local nameserver，先从 `/etc/resolv.conf` 过滤该类地址并重建 typed resolver（不硬编码公共 DNS），仍失败才用 OS resolver 做 10 秒有界的正向 fallback。getaddrinfo/NXDOMAIN error 不能证明 empty。CNAME/MX/TXT 也只有 typed no-record 才 empty，配置/传输/timeout 均为 error。Target Intel 本身没有 asset wave，因此每次 refresh 必须按 `created_at/value/id` 稳定顺序覆盖 `stage_started_at` snapshot 内**全部** domain Targets，仅把并发切成 128 一组；不能 `LIMIT 128` 固定重复同一批并让其余 frozen-axis DNS cell 永久饥饿。本阶段之后新落的 `source=asset_intel` domain 不追加进 refresh worklist/coverage denominator，其 provider DNS edge 由 current-run handoff 路径写入，留给 EAS/后续 run 消费。因为 target_intel 禁 scan-tool fallback、EAS 复用继承 DNS，这里是 bounded zero-touch refresh 点；全部适用查询与 business writes 成功且有记录才=`found`，查询/写入 mixed success 为 `partial`，只有所有适用组明确 no-record 才=`empty`。已成功 records 可以保留，但 partial/error 仍允许 duplicate guard 重试，后续完整 found/empty 会覆盖旧 error；整组记录写成功前不刷新 `real_ip`。`refresh_per_asset_landing_summary` 分开返回 `dns_found_hosts` / `dns_partial_hosts` / `dns_empty_hosts` / `dns_error_hosts` / `dns_refresh_failed`，app bridge 写对应 `technique_outcomes` 或 source error，严格保持“已查为空”≠“执行失败”。重复观测刷新 `dns_records.created_at`。
- `real_ip` 只认完整 A/AAAA 集合并确定性选 primary（IPv4 优先、同族字典序；**绝不**用 CNAME/MX/TXT）。它只是 host 的展示/排序缓存，不是唯一 DNS 关系、存活信号或主动授权；被动解析不得更新 `is_alive` / `last_alive_at`。存量数据可用 `recon_backfill_real_ip` 一次性回填（不重新解析）。
- `persistence.rs` 对 legacy GUI/org-recon 的 `ReconRecordKind::Ip` 仍默认 fail-closed：该路径的被动 IP observation 不会自行生成可执行 Target，也不会写回 `organizations.ip_ranges` 自我授权。这个约束不覆盖 asset-map 的确定性交接路径：`asset_intel::landing::plan_current_run_targets` 会把**当前 invocation** target records/host-IP pairs 中的 canonical IP 直接去重落为 org-bound `scope=in, source=asset_intel` IP Target，同时保留 host↔IP DNS edge。即便已经 landing，后续 active scan 仍必须通过 human approval，不能从被动 observation 自动开扫。
- `land_subdomain_assets` 只用调用方传入的**本轮具体 hostname observations**写/刷新 `target_assets(subdomain)`；不得扫描并重配当前组织全部历史 targets，也不得接受 `*.` pattern 本身。完整 URL observation 先提取 concrete hostname，host-only discovery 即使没有 A/AAAA 也可形成 domain Target；host-IP pair 的 canonical IP Target 由前置 asset-intel landing 建立，不由本函数推断。`land_service_assets` 同样只挂 current-run exact host，不做 apex/`www` alias fallback。
- Target Intel per-asset coverage axis 在 `stage_started_at` 冻结；前置 asset-intel landing 在该时间点后创建的 domain/IP Targets 只进入 EAS handoff，不加入当前 run 分母或生成 pending cells。DNS/subdomain durable rows可作为 frozen root outcome 的本轮事实，WHOIS 可读取新 domain Target 补 `organization:<uuid>` 注册信息，但都不能借此扩轴；service rows 只表示 EAS handoff context，不是 Target Intel coverage cell。
- coverage/outcome 必须依据实际 durable rows：DNS/subdomain write count `> 0` 才能报告对应 frozen-axis cell 的 `found`，WHOIS/ASN/CT/OSINT 则依据 org business rows；service write count `> 0` 只报告 handoff 成功。provider 文案、`observedTargets`、调用成功或 `Result: Ok(0)` 都不能代替 business write。mixed success 继续用 partial/error 表达，不能把“执行成功但零写入”伪造成 found。
- WHOIS/RDAP 可读取已经 materialize 的 `source=asset_intel` domain Target 做非递归 registration enrichment；provider domain expansion 则只从当前组织 trusted-source in-scope domain/URL/wildcard snapshot 取根（按 apex 去重），排除 `source=asset_intel`，也不再合并 `organizations.domains/app_domains`。Wildcard 只作查询根/子域授权模式，模式行本身
  不执行 WHOIS。每轮保留 typed terminal：有内容=`found`、明确空=`checked_empty`、
  执行失败=`error`、凭据/限流/授权阻塞=`blocked`；`error` 只证明尝试过。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app organization_recon
```
