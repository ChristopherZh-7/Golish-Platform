# golish-recon-app / asset_intel

> **一句话职责**：Discover Assets 的资产情报服务——provider-agnostic：workspace 要候选、provider 返归一化记录、只有被批准的候选后续才成 scope；含 ENScan 子进程编排，输出写 `golish-projects` 文件存储。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/asset_intel/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改资产发现（候选采集、provider 归一、ENScan 子进程、候选→scope 流程）时
- 改 asset intel 的 runtime/service 子层或输出落盘时

## 职责

Phase 1 provider-agnostic 资产情报：`run_passive_intel` 等调被动 provider 取候选，归一成记录，写入 `OrganizationCandidates`（经 `organizations`），输出落 `golish-projects` 文件存储。候选需用户批准才进 scope。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_passive_intel`（service/runtime） | 被动情报采集入口 |
| 候选/归一记录类型 | provider-agnostic 记录 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 服务编排 + 候选写入 |
| `service/` / `runtime/` | 服务层 / 运行时（子进程/取消） |
| `runtime/{cli,http,native}.rs` | 三种 provider runtime kind：`cli_json`（ENScan）/`http_json`（0.zone/quake）/`native_provider`（fofa/hunter/shodan，桥接 `golish-intel-providers` 注册表） |

## 依赖

- crate 内 `organizations`（候选写）、`golish-pentest::models::ToolConfig`（ENScan 等）、`golish-projects`（输出落盘）、`golish-core`（事件）

## 注意事项 / 坑

- **provider-agnostic**：候选先归一、经用户批准才成 scope——别让发现直接写 scope（绕过审批）。
- ENScan 经子进程（`tokio::process`）；输出落 projects 文件存储（非 DB 大字段）。
- 被 `agent_tools`（harness target_intel 阶段）包装成 AI 工具调用。
- **三个 runtime kind**（`AssetIntelRuntimeConfig`）：`cli_json` / `http_json` / `native_provider`。新加测绘 provider 若 `golish-intel-providers` 注册表已有实现（fofa/hunter/shodan/0.zone/quake），优先写 `native_provider` toolsconfig（复用原生鉴权/编码/字段映射），别在 toolsconfig 用 http_json 重写 API。
- `native_provider` 凭据走 `read_vault_secret`（与 http_json 同款，含 legacy `name=tool_id` 回退）；无 key → `Unavailable`（不伪造，I8），`provider_output_is_trusted` 仅信 `Completed`/`CheckedEmpty`。
- 在 `target_intel` 中，`recon_map_assets` 是 provider survey + DB landing 通道（`target_assets` / `organizations.*`），不是“全网完整收集”的证明；2026-06-23 起 target_intel 不再允许 scan-tool fallback，完整性由 harness 覆盖门按每个 technique 的 found/blocked/checked_empty 终态判定。
- **Provider 资产边界**：0.zone 的 HTTP runtime 必须按官方 JSON body 调 `/api/data/`；`site` / `code` 是 broad search，不参与 asset-scope enrichment，也不能写 `organizations.domains` / `ip_ranges` / `asns` / target candidates。0.zone `domain_root` 只在 domain-keyed 模式运行 `root_domain=={{domain}}`。Quake 的 `hostname` 可能是 PTR/rDNS 噪声，只能保留在 raw evidence，不得提升为 owned domain / target / host-IP pair；优先用 `domain` 与 `service.http.host`。
- `landing.rs` 的 host→IP pair 只对真实 DNS/URL host 写 `targets.real_ip` / `dns_records`；pair 的 host 若本身是 IP literal，只能作为 IP target 候选保留，不能写成“IP target real_ip=另一个 IP”。`upsert_target` 还会按 target type/value 二次防护，避免已有 IP target 被 provider pair 污染。
- **`landing.rs` 落庫分三类**（agent 路径 `agent_intel.rs` enrich 段，promote 之后）：① `promote_profile_assets_to_targets` 把 owned host/ip/cidr/cert-host 升 `targets`（带 surveyed `real_ip`）；② `land_target_intel_coverage`→`target_assets(asset_type='subdomain')`；③ **P1（设计 2026-06-26）`land_service_assets`→`target_assets(asset_type='service', value='<port>/<proto>', port/protocol/service/version)`**——从 candidate `evidence.raw` 抽 per-host 端口/服务（`service_assets_from_candidates`，纯函数）。target promote 的 planning 层会把同一 surveyed IP 下的 `www.<apex>` / `<apex>` 折叠成一个 target，优先保留 apex；`m.` / `api.` 等真实子域不折叠。service landing exact host 找不到时会 fallback 到去 `www.` 的 apex target，避免 target 已折叠后端口/服务情报丢失。在此之前 provider 回的 port/transport/service 只活在 raw JSON / org-flat intel（`quake_services` 等），per-host 端口情报全丢；现在落进 `target_assets` 那 4 个现成列。字段映射默认覆盖 quake/fofa/0.zone（`domain/service.http.host/host/hostname/port/transport/service.name`，但 Quake 配置层不把 `hostname` 当 owner 字段），shodan/fofa 若 raw 键名不同为 best-effort（只在 port 解析成功时 emit）。`ReconRecordKind::Port|Service`（GUI org-recon 路径）仍无映射，是后续 follow-up。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app asset_intel
```
