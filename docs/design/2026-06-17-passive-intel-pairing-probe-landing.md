# 2026-06-17 · 测绘被动情报：域名↔IP 配对 + 探活 + 自动落库（资产收集闭环）

> 起因：排查 run `pentest-chat-1781675306265-1`（目标 中国平安集团，stage `target_intel`）。
> enrich 报告发现 **189 资产 / 54 org**，但**只有 23 个落地为 target**（20 域名 + 3 IP）。
> 直查实时 DB（`postgres://golish:golish_local@localhost:15432/golish`）确认根因：测绘把海量
> 资产灌进了 `organizations` 档案列，却没有"档案 → target"的自动通道；域名与 IP 被拆成两个
> 独立数组、丢了配对；查询只用 `org:"公司名"` 单维度。本文用这些运行时证据给出修复设计。

## 0. 根因（运行时证据，2026-06-17 直查 DB / 实测 DNS）

| 现象 | 证据（实时 DB / 实测） | 根因落点 |
|---|---|---|
| 发现多、落地少 | org(平安集团) 档案 `domains=151 / ip_ranges=86 / asns=19 / business_systems=148`，但 `targets=23`（20 域名+3 IP）→ **~85% 发现资产卡在档案、未进 target** | enrich 只写 `organizations` 列（`profile_patch`），无"档案→`targets`"自动通道；现状靠 agent 手动 `manage_targets add` 挑一小撮 |
| 域名↔IP 配对丢失 | provider 返回每条含 `ip`+`domain`/`hostname`，但 `normalize.profile_fields` 把它们抽成独立的 `domains[]` 与 `ip_ranges[]` | `normalize.rs::extract_profile_field_entries` 按"每条规则独立产 (kind,field,value) 三元组"，无"同一记录内 domain↔ip 成对"的抽取 |
| 仍要二次 DNS 才有 real_ip | `targets.real_ip` 仅由 `land_dns_records`（公网解析，LIMIT 128 / 3s）回填；测绘自带的 IP 没用上 | 落 target 的 `persist_target_record` INSERT 不带 `real_ip`；real_ip 与测绘配对脱钩 |
| "未解析"域名 = 真 NXDOMAIN 也可能丢 IP | `cloud/job/yun.pingan.com`：`dns_records=0`、实测系统解析全 NXDOMAIN；但测绘可能观测到其 IP | 当前只信 DNS，不信测绘观测 IP → 内网/CDN/灰度域名拿不到 IP |
| 查询面窄 | 6 源全调（`0.zone,enscan-go,quake,fofa,hunter,shodan`），但 quake/fofa 仅 `org:"公司名"`（fofa 还只 `site`）；0.zone 7 类较全 | provider JSON 的 `requests`/`queries` 只声明 org 维；缺 domain / cert / icp 维 |
| 测绘混入噪声 | 档案 `domains` 含 `194.1.broad.ha.dynamic.163data.com.cn` 等第三方反解 PTR | 入库无 scope 过滤（org-recon 路径有 `value_belongs_to_organization`，enrich 路径没接） |

净结果：**信息收集不闭环**——测绘"发现"很广，但"配对 / 过滤 / 入库 / 探活"四步缺位，导致面板里
只看到极少数 target，且"未解析域名"既可能漏 IP、也可能混噪声。

## 1. 目标 / 影响面 / 验证 / 回滚

- **目标**：把"测绘多源发现"变成可复现的闭环——**多维查询 → 汇总去重 → 域名↔IP 配对 → scope
  过滤去噪 → 自动入库为 target（带测绘 real_ip）→ 探活回填存活/端口/指纹 → 前端按 real_ip 归位**。
- **影响面**：`golish-recon-app/src/asset_intel/*`（normalize 配对、自动提升、探活）、
  `golish-recon-app/src/organization_recon/persistence.rs`（复用 scope 过滤 + 落库）、
  `resources/intel-providers/*.json`（多维查询）、`resources/toolsconfig/httpx.json`（探活）。
  **不改 DB schema**（`targets.real_ip` / `target_assets` / `dns_records` 列均已存在）。
- **验证**：单测（配对抽取、scope 过滤、liveness 状态映射纯函数）+ 端到端跑一次 pingan
  `target_intel`，直查 `targets` 数量从 ~23 升到"档案过滤后规模"、`real_ip` 多数非空、
  `dns_records`/`target_assets` 同步增长；面板"未解析域名"仅剩真正无 IP 者。`just precommit` 全绿。
- **回滚点**：每阶段独立 commit；provider JSON 多维查询、normalize 配对、自动提升、探活步均可
  单独 revert；自动提升与探活都"失败只告警不阻断已提交的 enrich"（沿用 `land_*` 非致命约定）。

## 2. 目标管线（设计）

```
recon_enrich_assets(org)
  │
  ├─① 拉取：每源按【多维】查询（ICP + 自有根域 + 证书 + org 兜底），深翻页
  │     0.zone(7类) · quake(org/icp/domain/cert) · fofa(domain/cert/icp/org) · hunter · shodan
  │
  ├─② 汇总去重：merge_candidates（跨源合并，已有）
  │
  ├─③ 域名↔IP 配对：从每条测绘记录抽 (domain, ip) 对（新增 normalize 抽取）
  │     → 得到 host_map: domain -> primary_ip（测绘观测值）
  │
  ├─④ scope 过滤：value_belongs_to_organization（自有根域/网段才留；去 163data 这类噪声）
  │
  ├─⑤ 入库为 target：过滤后域名/IP upsert 成 scope=in target；
  │     real_ip 优先用 host_map 的测绘 IP；记 source='asset_intel'
  │
  ├─⑥ 探活：httpx/dnsx 对已入库 target 跑一遍（非门槛！）
  │     回填 status(live/dead/unknown) + 开放端口 + http 标题/server/CDN；
  │     测绘没给 IP 的域名，DNS 兜底解析一次
  │
  └─⑦ 归位：前端 buildHostTree 按 real_ip 自动把域名挂到 IP host 节点（已有）
```

> Phase F 决策（2026-06-17）：⑥ 主动 httpx 探活**下沉到 EAS**（target_intel 保持 zero-touch）；
> 本阶段 real_ip 仅来自测绘配对（Phase A）+ 被动 DNS（land_dns_records）。liveness / 端口 / 指纹属 EAS。
> 实现落点：Phase C 在 target_intel 内只留 `probe::liveness_from_httpx` 纯映射 + 单测（不接 enrich 主路径），
> 其 IO 调用方由 EAS specialist 接管。见配套计划 Task 13。

## 3. 关键设计决策（不变量，违反即回到老问题）

- **D1 · real_ip 优先取测绘配对值，DNS 仅兜底**：测绘记录里的 (domain, ip) 是平台实际观测
  （含 CDN 后/内网真实 IP），比本机 DNS 全。real_ip 取 host_map；host_map 缺失才走 DNS。
- **D2 · 探活绝不是入库门槛（项目铁律 I8）**：探活失败（打不通 / NXDOMAIN）的资产**照样入库，
  仅标 `status=dead/inactive`**，不得丢弃。否则 `cloud/job/yun.pingan.com` 这类"DNS 不通但真实
  存在"的高价值内网/灰度资产会被亲手漏掉——区分"已检查为空"与"未检查"是 gate 的基础。
- **D3 · scope 过滤在入库前，按"自有标识"判定**：复用 `value_belongs_to_organization`
  （属自有根域 / 网段才算），挡掉同 IP 同租户、第三方域名污染目标面。
- **D4 · 自动提升非致命、幂等**：profile→target 用 upsert（`persist_target_record` 的
  existing 分支语义），失败只 `tracing::warn` 不回滚已提交的 enrich（沿用 `land_target_intel_coverage`）。
- **D5 · 多维查询绑定"你拥有的东西"**：ICP 备案主体 + 自有根域 + 证书 subject 优先（高精度高召回），
  `org:"公司名"` 仅兜底（org 在测绘 ≈ ASN/IP 持有者 ≈ 云厂商，既漏云上又混同租户）。
- **D6 · key 等级感知**：quake `domain`/`service.cert` 为会员字段；注册级 key 取不到时
  退化用 `hostname`/`service.http.host`，不报错、记一条 note。

## 4. 不做（YAGNI / 边界）

> Phase F 决策（2026-06-17）：原 §2⑥ 的「httpx 轻量探活」与本阶段 zero-touch 契约冲突
> （methodology 明确禁用 httpx，`human_approval.required_before:["active_scan"]`）。**改判：主动 httpx
> 探活下沉到 EAS**；target_intel 的 real_ip 仅来自测绘配对 + 被动 DNS。下面「轻量探活（httpx 存活/指纹）」
> 一句保留为历史设计语境，实际不在本阶段执行（见 Task 13）。

- 不做主动扫描（端口爆破 / 漏洞）——本设计止于被动测绘 + 轻量探活（httpx 存活/指纹）。
- 不改 DB schema；liveness 用既有 `targets.status` + `dns_records`/`target_assets`，不新增表。
- 不动子公司发现（`promote.rs`）逻辑；子公司 enrich 仍由 stage_run 逐 org 调度（"还没轮到"非缺陷）。
- 不引新测绘源；先把已接入的 6 源查全、配对、落库做扎实。

## 5. 配套实现计划

见 `docs/superpowers/plans/2026-06-17-passive-intel-pairing-probe-landing.md`。
