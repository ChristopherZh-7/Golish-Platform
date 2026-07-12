# Intel / EAS 资产身份与关系闭环设计

> 状态：Accepted for implementation
> 日期：2026-07-11
> 范围：Scoping → Target Intel → External Attack Surface；不改数据库 schema，不扩大组织授权范围。

## 1. 结论

Golish 需要把“资产身份”“观察到的关系”“主动扫描授权”分开保存：

- 域名和 IP 是不同身份，DNS 解析只建立 `Domain -> IP` 观察边。
- 一个域名可以对应多个 IP，一个 IP 也可以承载多个域名；这是多对多关系。
- `www.example.com` 与 `example.com` 是两个域名资产，即使它们解析到同一 IP。
- DNS 解析到一个 IP **不等于** Golish 获得直接扫描该 IP 的授权。CDN、云 WAF、共享托管地址默认只能作为关系事实保存。
- EAS 的端口与服务按 `IP:port` 复用；站点存活和 Web 指纹按精确 `scheme://host:port` 分开验证。
- `targets.real_ip` 只是展示、排序和兼容用途的 primary-IP 缓存，不是关系真值，也不能证明存活。

本轮不增加 migration。现有 `targets`、`dns_records`、`network_endpoints`、`web_origins`、`web_origin_observations` 已足够表达正确模型。

## 2. 身份图

```text
组织授权范围
  ├─ targets(domain/url) ── dns_records ──> observed IP
  │          │
  │          └─ web_origin_observations ──> web_origins(scheme://host:port)
  │                                              │
  └─ targets(ip/cidr，必须独立授权) ──> network_endpoints(IP:port)
                                                 │
                                                 └─ web_origin_observations
```

| 存储 | 唯一身份/语义 | 能否直接进入主动扫描 worklist |
|---|---|---|
| `targets(domain)` | 精确 hostname；小写、去尾点，保留 `www` | 可以，限域名/URL 安全能力 |
| `targets(url)` | scope seed 或兼容资产；Web 执行时转 exact origin | 可以，限精确 origin |
| `targets(ip)` | 组织明确授权的网络主机身份 | 可以，承担 PORT 与 per-open-port SERVICE |
| `targets(cidr)` | 组织明确授权的网络范围 | 可以，本行只承担 range LIVENESS/PORT；child IP 下波承担 SERVICE/WEB |
| `targets(wildcard)` | `*.example.com` 被动 strict-child 授权模式 | 否；Intel 只承担一个 SUBDOMAIN 子域扩展格，模式本身永不执行 |
| `dns_records` | 域名到 IP 的完整多值观察事实 | 不会单独授予 IP 主动扫描权 |
| `targets.real_ip` | 一个确定性 primary IP 缓存 | 不作为 gate、授权或等价键 |
| `network_endpoints` | `IP:port` | 继承已授权 IP/CIDR 的网络能力 |
| `web_origins` | `scheme://host:port` | 必须由当前阶段探测确认并逐 origin 闭格 |
| `web_origin_observations` | origin、endpoint、target 的多对多观察 | 作为 EAS→Enumeration 的精确 Web 真值 |

Target Intel 另有一条只读的 `target_type=organization` 覆盖上下文，内部稳定
asset key 为 `organization:<uuid>`，只承担 WHOIS/ASN/OSINT；它不是 `targets`
扫描身份，绝不能进入 EAS。终态按当前 `organization_id + run_id +
organization:<uuid> + technique` 保存，不使用可变/可碰撞的组织名作 key。

例：

```text
moresec.cn      --A--> 203.0.113.10
www.moresec.cn  --A--> 203.0.113.10
www.moresec.cn  --A--> 203.0.113.11
```

应保留两个 domain target 和三条 DNS 边。只有当 `203.0.113.10/11`
本身已由可信 UI/CLI 写成当前组织的显式 IP target，或它们由一个显式授权
CIDR target 的 guarded in-range landing 产生时，才能执行端口与服务工作；
`organizations.ip_ranges` 只是 profile 元数据，不能单独授权。

## 3. 授权与归属

### 3.1 Domain 归属

被动发现的 hostname 只有在确定性规则确认它是当前组织可信 domain/URL
seed 的 exact host 或 strict child 时，才可成为 domain target。`*.example.com`
只授权 strict child，不授权 apex；wildcard 行本身不作为 literal host 做 DNS/WHOIS/EAS 执行，
但它在 Target Intel 有且仅有 SUBDOMAIN 格，必须以实际促进的 child target
或诚实 empty/error/blocked 终态收口；被动 provider 可以用 base domain 查 strict children。
判归属不得把 hostname 改写成 apex。
`organizations.domains/app_domains`、证书 SAN 或 provider 标签都只是观察/候选，
不能独立变成新的授权根。

### 3.2 IP 授权

可执行 IP/CIDR target 只有两个授权来源：

1. 用户通过可信 UI/CLI 在 stage 前显式提供并写入的 IP/CIDR target；
2. 已有、仍属于同一组织/项目且 `scope=in` 的精确 IP/CIDR target。

仅来自 `organizations.ip_ranges`、A/AAAA、证书、HTTP redirect 或第三方 provider
的 IP 不自动升级为可执行 target。它可以关联到已有授权 IP target，
但不能扩权。

### 3.3 组织隔离

所有 target upsert/lookup 必须以 `organization_id + target_type + canonical value` 为主键语义：

1. 优先复用当前组织精确行；
2. 可认领同 project、`organization_id IS NULL` 的 legacy 行；
3. 永不复用或改写 sibling organization 的行。

`project_path` 是 workspace 边界，不是 ownership 边界。

### 3.4 Scoping 边界

Scoping 只确认授权主体、组织树和用户给出的原始目标清单，不做 DNS、WHOIS、
HTTP 或端口探测。Red Team 可依 profile 依次做一次 `unit_review` 和一次
`scope_review`：前者确认组织单位，后者仅确认用户提供的 domain/IP/URL，不得把
组织 OSINT 结果转成 target。原始 seed 必须由可信 UI/CLI 在 stage 前写入
`targets`；Scoping 与 Target Intel 均不能用模型断言扩大 scope。若 review 通过的
seed 未落库，必须明确阻塞。提交 accepted 后立即停止并进入 Target Intel。
`scope_review` 必须与 backend 读取的 trusted snapshot 精确相等：对每行比较
canonical value、`target_type`、`scope`。前端未修改确认必须原样保留类型/范围；
编辑是 proposal，不是模型或 Scoping 可以直写的变更，需由可信 intake 更新后重审。

## 4. Target Intel 合同

### 4.1 Landing

- provider/DNS 的 `(exact hostname, canonical IP)` 按 pair 去重，而不是按 hostname first-IP-wins。
- provider 的 domain/host/URL 字段先规范成 concrete hostname；完整 URL 只能贡献其 host，绝不能以 `target_type='domain'` 原样写入。wildcard pattern 与 IP literal 不进入 domain promotion。
- `www`、apex、其他 vhost 分别落为 target；证书 hostname 同样保留。
- 所有 A/AAAA 都写 `dns_records`；`real_ip` 从有效地址中确定性选择（IPv4 优先，同族按规范字符串排序）。
- 更新 `real_ip` 不得把 passive DNS 观察写成 `liveness_state=alive`。
- service/WHOIS/DNS 证据只能挂到当前组织、精确类型与精确值的 target。
- Hickory 可用时分别查询 A 与 AAAA，并且只有两个 typed query 都得到
  `NoRecordsFound` 才能把地址组记为 empty；任一 transport/error 都保持非终态。
  macOS 系统 resolver 含 `%interface` scoped link-local nameserver、导致默认 Hickory
  构造失败时，先从 `/etc/resolv.conf` 重建可用的 typed resolver（过滤 scoped
  link-local，不硬编码公共 DNS）；仍失败才用 OS resolver 做 10 秒有界的**正向**
  fallback。OS resolver 能取到地址就落 A/AAAA，NXDOMAIN/getaddrinfo error 不能被
  拿来证明 checked-empty。
  CNAME/MX/TXT 同样只有 typed no-record 才是 empty。
- Target Intel 没有 asset wave，DNS refresh 必须覆盖当前组织全部 domain target；
  只允许用 128 并发分块控制资源，不能用固定 `LIMIT 128` 重复最新一批并让其余
  asset 永久饥饿。
- WHOIS 输入只从当前组织 trusted domain/URL/wildcard target snapshot 提取
  registrable domains；组织 profile 不是授权/查询根，不能与 target snapshot 混合扩围。

### 4.2 终态

每个 provider query 必须区分：

- `found`：请求成功且落到记录；
- `empty`：至少一个适用请求成功，但确实无记录；
- `error`：所有真实请求失败；
- `blocked`：没有适用输入或确定性前置条件不满足，并带原因。

`error` 不是 `empty`，generic provider 的 `found` 也不能自动代替 DNS、WHOIS、证书、服务等独立 coverage 技术。多 query provider 只要任一适用 query 失败，provider-wide 状态就保持 Failed，即使 sibling query 已返回部分 records；成功 sibling 的 records 仍须落库，但不能关闭该 provider/technique 的重试。候选合并后的所有 `evidence.sources[*].raw` 都参与 host-IP pair 与 service/port 提取，不能因同 host 去重而只保留第一条端口。失败尝试仍需写 source/audit 行，确保可追踪。重复调用 guard 必须同时检查 generic action、`action:<technique>` 精确行和 DNS technique outcome，任一 error/partial/running 都必须放行重试。

DNS attempt 同样允许“保存部分事实但不宣称终态”：只要 A/AAAA/CNAME/MX/TXT
任一适用 typed query 或记录写入失败，本次 host outcome 为 `partial/error`，已成功的
records 可保存，旧 error 可被后续完整 found/empty 覆盖；不得让任意一个 A record 的
Found 掩盖辅助关系查询或 business write 失败。只有整组业务写成功才刷新 primary
`real_ip`。

多能力 provider 必须同时返回 `techniqueStatus`：仅对它声明支持的精确技术写
found/empty/error/blocked。provider-wide 结果只说明 survey 状态，不能替代独立
技术终态。

人工 review queue 可以累计，但本轮 gate freshness 只能来自本次 provider invocation
的 profile entries/candidates/observed hosts。不能读取累计 `fresh.domains`，也不能把
全部历史 in-scope targets 重配成当前 observations 后刷新 `target_assets.updated_at`。

### 4.3 Freshness

- `target_assets` 重观察以 `GREATEST(discovered_at, updated_at)` 判断新鲜度。
- 本轮不改 schema；`dns_records.created_at` 暂时作为“最近观察时间”，冲突更新时刷新。后续若需要同时保留首次发现时间，应单独申请 additive `observed_at` migration。
- WHOIS fresh stage 必须真实重查，不能因为旧值存在就伪造 `empty`。

## 5. External Attack Surface 合同

### 5.1 技术轴

| 资产类型 | LIVENESS | PORT | SERVICE | WEB-FINGERPRINT |
|---|---:|---:|---:|---:|
| domain/url | required | N/A | N/A | 对每个确认 origin required |
| ip | required | required | 对确认开放端口 required | 对每个确认 origin required |
| cidr | required（range outcome） | required | N/A（由 supplemental child IP 承担） | N/A（由 supplemental child IP 承担） |
| wildcard | N/A | N/A | N/A | N/A |

解析到同一 IP 的域名不再整行 alias/N/A。只复用 IP 的 PORT/SERVICE；域名自身仍需 LIVENESS 与 exact-origin WEB。
CIDR 扫出的 in-range 子 IP 必须先 guarded 落成带 CIDR provenance 的精确 IP
target，再进 supplemental wave 做 SERVICE/WEB；不得用 range 行代替子 IP 闭格。
`rank_attack_surface_seeds` 必须确定性排除 wildcard pattern，只把已落库的
concrete strict-child domain 交给 EAS；wrapper 的 wildcard 拒绝只是最后 backstop。

### 5.2 Exact Web Origin

`http://a.example:80`、`https://a.example:443`、`https://a.example:8443`、`https://b.example:443` 是四个不同执行身份，即使它们最后连到同一 endpoint。

HTTP 探测 landing 必须同时写：

1. target 的兼容 port/url 数据；
2. `web_origins` exact identity；
3. 有地址时的 `network_endpoints`；
4. target-bound `web_origin_observations`。

EAS WEB gate 使用确定性 barrier：当前组织、当前 stage cutoff 后确认的 required origins，必须各有当前 run 的 `found` 或 `empty` WEB-FINGERPRINT outcome，并且 evidence 对齐。`required - completed` 非空即 BLOCK；查询失败 fail closed。

兼容的 target-level `WEB-FINGERPRINT` 单元按 hostname 聚合 exact-origin outcome，
但它不是精确分母：真正的完整性仍由上述 barrier 逐
`scheme://host:port` 校验。因此同 hostname 的一个 HTTPS outcome 可以关闭父 target
的普通 WEB 单元，却不能替代仍缺失的 HTTP 或其他端口 origin；不同 hostname
（包括 `www` 与 apex）也绝不互相补格。

### 5.3 工具解析

- AI 只调用 `eas_probe_http_liveness` / `eas_discover_ports` /
  `eas_fingerprint_services` / `eas_fingerprint_web_stack` 四个业务 wrapper；
  `httpx`、`naabu`、`masscan`、`nmap`、WhatWeb 都是 wrapper 内固定实现，不向模型
  暴露自由命令执行。landing 必须携带 wrapper 注入的 hidden capability，不能只按底层
  tool name 猜语义。
- httpx 输出必须保留 URL、scheme、port、content type，并能从 URL 推导默认 80/443。
- WhatWeb 同时接受 `[200]`、`[200 OK]`、`[404 Not Found]`，无插件尾部也是合法已检查结果。
- 同一 target 的多个 exact origins 可以放在一个授权 batch 中；只拒绝完全重复的 origin。
- `eas_discover_ports` 的普通 nmap/naabu/masscan 结果只证明 PORT/LIVENESS：即使 nmap
  按 `/etc/services` 输出 `http`，也必须剥离 service/version/URL，不能生成 SERVICE 或
  WEB 真值。`eas_fingerprint_services` 的固定 `nmap -sV` 才能保存真实 service、product /
  version；确认的 HTTP(S) service 必须生成对应 exact origin。
- 每个已授权 IP/CIDR 输出的 `IP:port`（包括 nmap/naabu/masscan/httpx）都写
  `network_endpoints`；nmap `Host is up` 可写显式 liveness，零开放端口不再让 LIVENESS
  永久 pending。WhatWeb 与 httpx 写到同名 fingerprint 时，WhatWeb provenance 必须提升为
  terminal WEB producer。
- IPv6 host/port 接受 bracketed/unbracketed 工具输出并以 `IpAddr` 规范化匹配授权；
  target exact-value 授权还必须匹配 `target_type`。同 batch 的 IP+包含它的 CIDR或重叠
  CIDR 在任何网络调用前整批拒绝；CIDR 命中同 org/project 已存在的授权 IP 时复用该行，
  不要求相同 parent。masscan 不支持 IPv6，必须在 launch 前明确拒绝。

## 6. Wave 与闭环

每波最多 200 个资产。下一波候选定义为“当前 operation/org/stage 从未进入任何 wave 的资产”，不使用 `created_at > parent.started_at`。因此阶段开始前已有的第 201 个资产，以及阶段运行中新发现的资产，都能进入后续波；401 个资产必须得到 `200 + 200 + 1`，且不重复、不丢尾。

最后一次“没有未分配 target”的读取与 `org_stage_completions` 通过水位必须在同一短事务内完成，并对 wave 写入和 target 写入建立 DB 锁屏障。屏障前已经在途的 target writer 必须先完成并进入候选；屏障提交后的新 target 明确属于后续 operation/stage 生命周期。不能先做空 SELECT、再在另一个事务里发 completion/pass token。

completion 表虽然兼容保留 `(organization_id, stage_kind)` 唯一键，但写入的
`stage_run_id` 必须是当前 operation UUID；stage_run resume、pass-token 生成与
orchestrator 最终重算都必须匹配这个 UUID。并发 operation 可以互相覆盖兼容行并造成
安全的重跑/阻塞，但绝不能借另一个 operation 的 fresh timestamp 假 PASS。

### 6.1 Preflight、最终 gate 与持久化

- Target Intel / EAS 的 preflight 可对当前权威 worklist 的精确格子做
  `terminal_exceptions` 只读预演；仅允许 checked_empty、blocked、
  not_applicable，永远不允许模型声明 found。Enumeration 继续拒绝模型终态。
- checked_empty 必须带该技术真实 evidence；blocked/not_applicable 必须有具体
  note。预演不写库、不新增资产、不扩权，返回的 `coverage_to_submit` 原样提交。
- `submit_stage_deliverable=accepted` 是主 agent 与 specialist 的确定性终止信号；
  不再等待“空闲轮次”，也不得继续查 worklist、改状态或重复提交。
- 只有最终 per-org gate PASS 后，blocked/not_applicable 才物化到
  `technique_outcomes`。条件 upsert 仅能插入缺行或替换 partial/error，不能覆盖
  found/empty/已有终态；checked_empty 仍由直接 producer/source evidence 落地。
- 权威 snapshot 重读或任一终态 upsert 失败必须反转本次 org PASS；仅当并发 producer
  已先写入不可降级的真实 terminal truth 时，才把条件 upsert 的零更新视为成功。
- 同一个 assistant tool-call batch 中，`submit_stage_deliverable=accepted` 是即时 barrier：
  余下工具调用必须跳过但仍生成配对 ToolResult，不能在 accepted 之后继续改 target/state。
- DB/gate truth 查询必须纯读；DNS/subdomain refresh 只在成功
  `recon_map_assets` 的显式写路径发生，Scoping/query/preflight 不得触发 Intel。

## 7. 测试与验收

### 7.1 确定性测试

- apex + www + 同 hostname 多 A/AAAA 全保留；primary IP 稳定。
- DNS 关系不自动创建未授权 IP target，不自动标记 liveness alive。
- organization profile domains/ip_ranges 不能创建执行 root；wildcard 本身不执行，
  Target Intel 只有 SUBDOMAIN 格且 found 要求真实 child target，EAS 全轴 N/A。
- sibling org 同 value 不串 ownership；legacy null-org 可安全认领。
- provider 全失败为 error，真实空为 empty；error 不闭 checked-empty。
- provider/DNS mixed success 保存成功 records，但 exact outcome 保持 partial/error 并允许重试；同 host 多 service/port 不丢。
- IP + 多 vhost coverage 全保留，PORT/SERVICE 仅 IP 承担。
- 两 vhost 共 IP、多个 scheme/port 时 exact origins 全部分别闭格。
- discover-ports 与 fingerprint-services 语义隔离、nmap version/origin、IPv6 parse/auth、overlap preflight、WhatWeb provenance、host-up liveness 均有回归测试。
- WhatWeb reason phrase、httpx port/scheme、401 wave 分页均有回归测试。

### 7.2 CLI 验收

使用 `scripts/stage_smoke.py` 的隔离临时 DB 与 `--auto-approve`，无需 stdin 交互：

1. 本地 fixture 验证 EAS 精确 origin 与 gate；
2. 在用户授权的“默安科技 / moresec.cn”范围执行 Scoping→Intel→EAS；
3. 不传 `--include-subsidiaries`，objective 明确禁止扩展到子公司及 DNS-only 共享/CDN IP；
4. 用 `scripts/run_tree.py --full --db` 和 DB truth 复核每阶段 outcome、evidence、targets、DNS edges、origins 与 pass token；
5. 上游通过后核对 EAS 已留下 target-bound exact origins 与 WEB outcomes，证明
   Enumeration 的输入合同可用；本任务不重跑用户已修复的 Enumeration 阶段。

## 8. 非目标

- 本轮不新增/修改 migration，不改变 IPC generated types。
- 不把所有观察到的 IP 变成主动扫描 target。
- 不把 `organizations.domains/app_domains/ip_ranges` 当作可执行目标授权库。
- 不自动修复历史 sibling-org 错绑数据；新鲜隔离 DB 先验证新语义，历史 reconciliation 另立任务。
- 不扩展到默安科技子公司或未授权根域。
