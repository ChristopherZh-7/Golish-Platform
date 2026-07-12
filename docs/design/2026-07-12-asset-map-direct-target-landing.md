# Asset Map 直接落 Target 设计

## 1. 问题与现场证据

最新 Target Intel 运行 `pentest-chat-1783796952452-1` 中，provider 已返回资产：

- `0.zone normalized 49`
- `quake normalized 854`
- current-run target observations：45
- host/IP pairs：56

但最终日志是 `promoted=0`、`service_assets=0`，数据库中
`targets=0 / target_assets=0 / dns_records=0`。原因不是凭据失败，而是 landing
先从既有 trusted target 读取授权根；company-only Scoping 没有 target 行，导致
`authorized_hosts=[]`，随后本轮 provider 发现全部被 ownership filter 丢弃。

同时，provider target observations 被写进
`organizations.intel.engagement.candidates.targets`，TargetPanel 又提供
Approve/Reject/Promote 的人工候选工作区。这与当前产品的 AI/harness 全程编排不一致。

## 2. 新合同

`recon_map_assets` 的当前调用结果按以下顺序处理：

```text
已配置 provider 并发查询
  -> current-run 归一化 domain/IP observations
  -> current-run 内 canonical 去重
  -> org-bound targets(scope=in, source=asset_intel) upsert
  -> hostname/IP pairs 写 dns_records，并缓存 domain.real_ip
  -> service observations 写 target_assets(service)
  -> 当前域名集合内的严格父子关系写 target_assets(subdomain)
  -> 返回 observation 与实际 landing 计数
```

这里的“current-run”是硬边界：只消费本次 `AssetIntelRun.candidates`、
`observed_domain_hosts` 和由本次 evidence 提取的 `HostIpPair`。不得把累计
organization profile、历史候选 JSON 或历史 targets 重新当成本轮发现。

### 2.1 目标身份

- 合法 concrete hostname 落 `target_type=domain`。
- URL observation 只贡献 canonical host；Web Origin 由后续 EAS/Enumeration 建立。
- canonical IPv4/IPv6 observation 以及 pair 中的每个 IP 都落 `target_type=ip`。
- wildcard、空值、malformed host、CIDR 和无法分类的噪声不落。
- 去重键为 `(organization_id, project_path, target_type, canonical value)`；
  `www` 与 apex 不折叠，同一 domain 的多个 IP 全保留在 `dns_records`。
- domain 的 `real_ip` 只缓存确定性的一个 primary（IPv4 优先、同族字典序），
  不替代完整 DNS 边。

### 2.2 范围与安全边界

provider 结果写成 `scope=in`，因为这是本产品 Target Intel 到 EAS 的确定性资产交接。
真正开始主动扫描仍受 `target_intel -> external_attack_surface` 的
`human_approval.required_before=active_scan` 边界约束。

已有同组织 exact identity 若被用户明确设为 `scope=out`，provider 重跑不得把它
翻回 `in`。跨组织同值不得接管；只允许同组织或 legacy `organization_id IS NULL`
的 exact identity 兼容认领。

`source=asset_intel` 继续不是自动递归 provider-query root。这样本次发现会进入
Target/EAS handoff，但不会在后续 retry 中因自身存在无限扩展查询范围。显式
domain repair 仍要求 trusted intake root。

### 2.3 当前阶段分母冻结

Target Intel 的 per-asset coverage axis 冻结在 `stage_started_at`：只校验阶段开始前
已经存在的 input Targets，再加稳定的 `organization:<uuid>` context row。本阶段
运行中刚写入的 `source=asset_intel` domain/IP 是 EAS handoff output，不能反向
制造新的 Target Intel pending cells 或改变 done/total；否则一次正常资产发现会让
生产它的阶段自行扩张并再次进入 submit loop。

`recon_lookup_whois` 可以非递归读取这些新 domain，按 registrable apex 补当前组织的
WHOIS context。这不会把新 Target 加回当前 per-asset axis，也不会授权 domain-keyed
provider 递归扩张。

## 3. 候选路径收口

- `AssetIntelRun.candidates` 保留为 provider adapter 的**瞬时归一化结构**，因为
  pair/service/target landing 需要它；它不再代表人工审核队列。
- Enrich / `recon_map_assets` / organization-recon asset stage 强制
  `create_candidates=false`，不再写 durable `engagement.candidates.targets`。
- 子公司 `organizations` bucket 保留。`ask_human(unit_review)` 仍只读取该 bucket，
  Scoping 的范围边界不受影响。
- 删除 TargetPanel 的 Candidates tab、人工 approve/reject/promote handler、
  “Review scope”假入口和“Create target candidates after review”设置。
- 旧 JSON shape、DTO 和 candidate commands 暂时保留兼容；不做 schema/migration，
  不自动删除历史 JSON，以免误删共容器里的 subsidiary candidates。

## 4. 计数与 gate 语义

工具结果必须区分：

- `observedTargets`：本次 provider 归一化 observation 数；
- `targets`：本次实际成功 upsert/reuse 的 Target 数；
- `landedDomains` / `landedIps` / `dnsRecords` / `serviceAssets` /
  `subdomainAssets`：各业务表实际写入计数。

`Result::Ok(0)` 不得伪造 technique `found`。有可落 observation 但
`targets=0`，或有合法 pair 但 `dnsRecords=0`，必须把 summary 标为 `Partial`
并返回 retryable landing error；submit/gate 不能把“provider 跑过”当成“交接完成”。

## 5. 兼容与不做事项

- 不改 DB schema/migration，不清历史 rows/JSON。
- 不删除 OrganizationCandidate DTO、candidate command 或 AskHuman 的 subsidiary reader。
- 不在本改动中发起任何真实 provider/API/主动扫描请求。
- 不运行 `./init.sh`；按用户要求只跑定向 Rust/前端验证。
