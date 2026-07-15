# golish-recon-app / asset_intel

> **一句话职责**：provider-agnostic 资产情报与 Target Intel handoff——本轮归一化 domain/IP 直接去重落 org-bound `scope=in` Targets，并写 DNS/service/subdomain 关系；子公司候选仍走人审。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/asset_intel/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改资产发现（provider 归一、current-run Target landing、子公司候选人审）时
- 改 asset intel 的 runtime/service 子层或输出落盘时

## 职责

Phase 1 provider-agnostic 资产情报：`run_passive_intel` 调被动 provider，把本轮 target-shaped records 暂存在 `AssetIntelRun.candidates` 作为归一化 adapter，随后由 `landing.rs` 将 canonical domain/IP 直接去重写成当前 org 的 `scope=in, source=asset_intel` Targets，并补 `dns_records`、service/subdomain `target_assets`。这些新 Targets 是 EAS handoff output，不反向加入本轮已在 `stage_started_at` 冻结的 Target Intel coverage axis。asset-map 不再把 target candidates 持久化为 TargetPanel 审批队列；`OrganizationCandidates.organizations` 仍服务子公司发现与 `ask_human(unit_review)`。原 candidate DTO/JSON/command shape 保留兼容。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_passive_intel`（service/runtime） | 被动情报采集入口 |
| `plan_current_run_targets` / `land_current_run_targets` | current-run domain/IP canonical plan 与 org-bound Target/DNS 落库；`recon_map_assets` writer 对 exact Target identity 取 transaction advisory lock |
| `TargetLandingSummary` | 本轮实际写入的 targets/domains/ips/dns_records 计数；零计数不是 found |
| `OrganizationCandidates` | runtime 归一化 adapter；durable organization bucket 仅保留子公司兼容/人审用途 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` / `agent_intel.rs` | 服务编排 + current-run landing/summary |
| `landing.rs` | domain/IP Target、DNS edge、service handoff 的 canonical plan/write |
| `service/` / `runtime/` | 服务层 / 运行时（子进程/取消） |
| `runtime/{cli,http,native}.rs` | 三种 provider runtime kind：`cli_json`（ENScan）/`http_json`（0.zone/quake）/`native_provider`（fofa/hunter/shodan，桥接 `golish-intel-providers` 注册表） |

## 依赖

- crate 内 `organizations`（org profile + subsidiary candidate 兼容）、`targets`/`golish-db`（Target/DNS/target_assets 写入）、`golish-pentest::models::ToolConfig`（ENScan 等）、`golish-projects`（输出落盘）、`golish-core`（事件）

## 注意事项 / 坑

- **asset-map 与 subsidiary discovery 分流**：asset-map 本轮 `kind=target` records 只是内存归一化 adapter，不进入 durable target candidate queue；其中的 canonical domain/IP，以及 host-IP pair 中的 canonical IP，直接按组织去重落 `scope=in, source=asset_intel` Target。只有 `OrganizationCandidates.organizations` 继续作为子公司候选，供 `ask_human(unit_review)` 审批。
- **Target 身份不等于递归查询根或主动扫描授权**：上述 `scope=in` 是 Target Intel → 后续阶段的确定性交接身份；本阶段自动 domain expansion 仍只从 manual/imported/customer_provided/stage-run-seed/seed/cli 等 pre-stage trusted roots 取值，明确排除 `source=asset_intel`，避免本轮发现递归放大。EAS/其它 active scan 仍必须先通过其 human approval gate，不能因为被动 landing 自动开扫。
- **coverage axis 冻结**：Target Intel 的 per-asset denominator 只包含 `stage_started_at` 时已经存在的 in-scope Targets，另加稳定的 `organization:<uuid>` context row。本阶段之后新落的 `source=asset_intel` Targets 不生成当前 run 的 coverage cells，也不改变 done/total；它们交给 EAS/后续 run。WHOIS 可读取这些新 domain Targets 补当前 org context 的注册信息，但这既不扩 per-asset denominator，也不允许 provider 递归查询。
- ENScan 经子进程（`tokio::process`）；输出落 projects 文件存储（非 DB 大字段）。
- 被 `agent_tools`（harness target_intel 阶段）包装成 AI 工具调用。
- **三个 runtime kind**（`AssetIntelRuntimeConfig`）：`cli_json` / `http_json` / `native_provider`。新加测绘 provider 若 `golish-intel-providers` 注册表已有实现（fofa/hunter/shodan/0.zone/quake），优先写 `native_provider` toolsconfig（复用原生鉴权/编码/字段映射），别在 toolsconfig 用 http_json 重写 API。
- `native_provider` 凭据走 `read_vault_secret`（与 http_json 同款，含 legacy `name=tool_id` 回退）；无 key → `Unavailable`（不伪造，I8）。provider survey 只有至少一个 native provider 真正成功才可分类为 `Completed`/`CheckedEmpty`；全部执行失败必须是 `Failed`，全部缺凭据/不适用才是 `Unavailable`，不能把运行错误伪装成“已查为空”。
- `native_provider` 的 provider-wide terminal 要求本轮**所有**适用 query 都成功：有记录才 `Completed`，总记录为 0 才 `CheckedEmpty`；“部分 query 有记录/为空 + sibling query 错误”一律保持 `Failed`。但只要 evidence 明确 `succeededQueries>0`，成功 sibling 的 records/profile patch 仍参与 landing，状态和 exact technique 继续 nonterminal、允许重试；不能因为 provider-wide Failed 把真实返回的数据丢掉。native evidence 会保留每条 `queryType + status`，`agent_intel.rs` 生成 `techniqueStatus` 时优先按 query→technique 聚合（同一 technique 任一 query 错误即 `error`），只有没有 typed query evidence 时才回退 provider-wide 状态，避免一次成功结果替其它 capability 签字。
- `agent_intel.rs` 的 summary 同时返回 provider-wide status 与 `techniqueStatus`：仅按 provider 声明 capability 映射 SUBDOMAIN/DNS/ASN/CT/OSINT 的 exact found/empty/error/blocked；例如 ENScan 的 checked-empty 可以关闭它声明的 OSINT/SUBDOMAIN，但 domains-only capability 不能伪造 OSINT。runtime 把这些 exact rows 写 `source_query_log`，generic row 只保留 survey provenance。
- 在 `target_intel` 中，`recon_map_assets` 是 provider survey + **current-run business landing** 通道：本轮 canonical identities 写 `targets`，host-IP pairs 写 A/AAAA `dns_records`，具体 host/service observations 写 `target_assets(subdomain|service)`，组织资料仍写 `organizations.*`。provider prose、`observedTargets` 或工具 `Result: Ok(0)` 只证明调用/观测，不能当 `found`；DNS/subdomain/org 事实只有实际写入计数 `> 0` 才能支撑对应 frozen-axis technique/org outcome。Target/service rows 分别证明 EAS identity/service handoff 成功，不会新增当前 run coverage row，也不能单独伪造 Intel found。它仍不是“全网完整收集”的证明，完整性由 harness 覆盖门按各 technique 的 terminal outcome 判定。
- `agent_intel.rs` 的普通 enrich/org-company 路径最多从本轮开始前已存在的 trusted-source domain/URL/wildcard roots 自动补 5 个 domain-keyed provider runs；返回 summary 的 `domainExpansions[]` 供 runtime 写 `source_query_log(target=<apex>)`。本轮新落 `source=asset_intel` rows 不进入这个递归 provider root snapshot。显式 `AssetIntelHydrateConfig.domain` 仍只跑 domain-keyed templates，用作 targeted repair/manual supplement，且不会再次递归扩展。
- **Provider 资产边界**：0.zone 的 HTTP runtime 必须按官方 JSON body 调 `/api/data/`；`site` / `code` 是 broad search，不参与 asset-scope enrichment，也不能写 `organizations.domains` / `ip_ranges` / `asns` / target candidates。0.zone `domain_root` 只在 domain-keyed 模式运行 `root_domain=={{domain}}`。Quake 的 `hostname` 可能是 PTR/rDNS 噪声，只能保留在 raw evidence，不得提升为 owned domain / target / host-IP pair；优先用 `domain` 与 `service.http.host`。
- `landing.rs` 先把 provider 的 domain/host/URL 字段归一为 concrete hostname（例如 `https://App.Example.com:8443/a` → `app.example.com`），丢弃 wildcard、malformed value 与错误类型；`plan_current_run_targets` 对本轮 domain/IP identity 确定性去重。host-IP pair 的 host 形成 domain Target，pair 中的每个 canonical IP 也形成独立 IP Target；所有 A/AAAA edge 同时写 `dns_records`，并从完整集合确定性选择 primary `real_ip`（IPv4 优先、同族字典序）。`real_ip` 仅供展示/排序/兼容读取，不代表唯一关系、存活或主动扫描授权；被动 landing 不写 `is_alive` / `last_alive_at`。
- **查询种子去重不等于 Target 去重**：当 provider root 集合同时明确包含 `example.com` 与 `api.example.com` 时，只省掉被父域查询覆盖的 child query；`api.example.com` Target 仍独立保留。若只有 child，则禁止自行上卷 apex。FOFA/Quake/native 记录优先以具体 `service.http.host`/`host` 作为 Target、DNS 与 service owner，registrable `domain` 只作 profile/provenance，避免同 IP vhost 在进入 exact Target planner 前被并掉。
- domain-keyed 的一跳 provider pass 不把关系 IP 再提升为独立 Target；IP 仍保存在 exact hostname 的 `real_ip`/A/AAAA 关系中。不同 exact hostname 即使同 IP 也不得在 Intel 阶段折叠，物理 `IP:transport:port` 的执行去重属于 `NetworkEndpoint`，Web 身份仍按 exact `scheme+host+port` 区分。
- `land_current_run_targets` 按 `(organization_id, target_type, exact normalized value, project)` 幂等 upsert，新 rows 固定 `scope=in, source=asset_intel`；只有 legacy `organization_id IS NULL` 的精确同行可被当前 org 接管，不能跨 org/value 误合并。`recon_map_assets` 的并发 provider writer 会先按 `(project, target_type, exact canonical value)` 取 transaction-scoped advisory lock，再完成 lookup/claim/insert，避免同一 FQDN 的 SELECT→INSERT 竞态；锁键不按 apex、解析 IP 或 CNAME 合并，因此兄弟 vhost 始终是不同 Target。这个保证只覆盖本 landing path，GUI/manual intake 与历史重复清理不在此承诺内。随后 `land_target_intel_coverage` 写本轮具体 hostname 的 `target_assets(subdomain)`，`land_service_assets` 写 exact-host `target_assets(service)`；service 不使用去 `www.` alias fallback。
- **本轮 freshness 边界**：`AssetIntelRun.candidates`、`observed_domain_hosts`、host-IP/service 提取及 SUBDOMAIN freshness 只消费当前 provider invocation；不得拿历史 profile/Targets 重新配对，也不得刷新旧 `target_assets.updated_at` 冒充本轮发现。target candidate queue 对 enrichment 路径关闭；旧 `OrganizationCandidate*` DTO、`AssetIntelHydrateConfig.create_candidates`、candidate commands 与 `intel.engagement.candidates` JSON 仍保留兼容，但 TargetPanel 不再读取/展示 target candidate tab。
- Target Intel 的递归 provider 查询根只取 trusted-source domain/URL/wildcard snapshot，包含 `customer_provided`，排除 `asset_intel` 与其它 provider-derived source。Wildcard 行只承担 `GOLISH-INTEL-SUBDOMAIN`；found 需实际落 strict-child domain/`target_assets(subdomain)`，apex 不在 `*.` 授权内，模式本身不能授权 EAS/Enumeration。
- provider 回的 port/transport/service 通过 `service_assets_from_candidates` 写入 per-host `target_assets`。候选按身份合并后必须遍历主 `evidence.raw` 与全部 `evidence.sources[*].raw`，所以同一 hostname 的 80/443/8443 等多条服务不会 first-record-wins 丢失。字段映射默认覆盖 quake/fofa/0.zone（`domain/service.http.host/host/hostname/port/transport/service.name`，但 Quake 配置层不把 `hostname` 当 owner 字段），shodan/fofa 若 raw 键名不同为 best-effort（只在 port 解析成功时 emit）。`ReconRecordKind::Port|Service`（GUI org-recon 路径）仍无映射，是后续 follow-up。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app asset_intel
```
