# Scoping 企业实体确认与自主企业资产发现设计

> **状态**：Approved for implementation（用户于 2026-08-04 明确要求实施、跑实体、移除旧六轴兼容）
>
> **取代**：`2026-08-02-target-intel-goal-loop-and-audit.md` 中关于 Scoping 不变、Target Intel 不做可达性验证、保留旧六轴 compatibility publication 的决定。
>
> **范围**：Red Team 的 `Scoping` 与 `Target Intel`，以及 Target Intel → EAS 的正式资产晋升边界。后续 EAS/Enumeration/Vuln/AU/Investigation/Reporting 的既有安全语义不变。

## 1. 决策摘要

Scoping 不再只做单一企业注册信息查询，也不在 provider 无结果时立即要求用户手填。它由 Main AI 负责确认企业实体，按证据质量依次使用：

1. 现有组织库精确复用；
2. 企业注册信息工具（ENScan/企查查类能力）；
3. 0.zone 的企业画像查询；
4. 受控公开搜索与浏览器取证；
5. 仍有多个合理实体、证据冲突或无法确认时才请求用户选择。

Scoping 的终态是冻结 `Company Identity`，至少包含法定名称、常用名/品牌、可用企业标识、证据引用、确认方式和 scope policy。网页标题、搜索摘要或模型记忆不能单独成为法定实体权威。

Target Intel 不再补 DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT 六个格子。Main AI 得到 `Golish Corporate Asset Discovery Methodology v1`、冻结的 Company Identity、当前 observation/frontier 和受控工具，自己制定并调整计划。它可以从公司、品牌、域名、hostname、IP、ASN、证书、ICP备案、邮箱域、代码仓库、App 等 pivot 继续搜索，但模型永远不能提交 provider DSL、凭据、scope authority、evidence id 或直接写 Target。

每个发现先进入 `Asset Observation`，经过以下宿主确定性步骤后才可晋升为正式资产：

1. 规范化与去重；
2. 企业归属判定；
3. 共享/CDN/第三方基础设施隔离；
4. 低影响可达性验证；
5. 原子写入正式 Target、DNS/service 关系和完整 provider 元数据。

不可达资产、仅 DNS 可解析资产、归属不明确资产不会进入正式 Target；它们保留为 observation、Evidence 和 residual，供 AI 换 pivot 或审计。HTTP 的任何实际响应（包括 3xx/401/403）可证明 Web 可达；非 Web 资产需要协议握手或端口响应。DNS 响应本身不足以证明资产可访问。

## 2. Scoping 契约

### 2.1 Main AI 可用能力

- `manage_organizations(list)`：优先复用现有根组织。
- `recon_lookup_company`：统一企业实体查询入口，聚合所有已配置的企业注册/画像 adapter，不限定单一 provider。
- `recon_search_public` / 受控 browser：只有结构化 provider 不可用、checked-empty 或结果冲突时使用；结果必须落 raw artifact 和 Evidence。
- `ask_human(choice)`：只在确定性结果仍有歧义时使用，选项必须带可区分的企业标识和证据摘要。
- `manage_organizations(create|get-or-create)`：只写用户/证据确认后的法定实体。

0.zone 的 `org` 查询属于企业实体解析能力；`domain/site/code` 等资产搜索不在 Scoping 执行。Scoping 仍不探测目标主机，也不创建 Target。

### 2.2 Company Identity

每次 Scoping 终态必须形成 operation/org-scoped、不可变的 Company Identity receipt：

- `canonical_legal_name`
- `aliases` / `brands`
- `unified_social_credit_code`、注册号等可用标识
- 注册地、法定代表人等 disambiguation fields（有则保存，不强制伪造）
- `evidence_refs` 与 raw artifact refs
- `resolution_status = confirmed | needs_human | unresolved`
- `scope_policy`：默认只包含当前法定实体；子公司是否纳入仍是显式 scope 决定

只有 `confirmed` 可进入 Target Intel。provider 不可用、无结果和网络失败必须分别记录，不能合并成 checked-empty。

## 3. Golish Corporate Asset Discovery Methodology v1

CyberStrike 的可复用部分是“根据当前事实动态串联工具和调整计划”。其 Recon skill 从已知域名开始，不能直接承担 Golish 的公司名起点、资产归属和正式入库闭环，因此不复制其内容，只采用以下运行原则。

### 3.1 信息增益驱动的 pivot

Main AI 每轮读取 frontier，选择预期信息增益最高且仍在预算/授权内的 pivot，例如：

- 公司/品牌 → 官网、ICP备案、公开仓库、App、邮箱域、测绘平台组织字段；
- 已确认域名 → 子域、证书透明度、DNS 历史、测绘平台 host/service；
- 已确认 IP → 精确 IP 测绘、rDNS、证书、相邻服务；
- 已确认企业 ASN → ASN 内候选；ASN 只有企业所有权已有证据时才可扩展；
- 证书/ICP/favicon/repository → 反向发现候选域名或系统。

工具返回的新事实可改变 plan。没有固定 provider 顺序，也不要求 WHOIS、ASN 等每类都执行。高成本、低预期增益、缺凭据或重复 pivot 必须形成 typed disposition，而不是无限重试。

### 3.2 Semantic Query AST

模型只提交闭合语义结构：

- `pivot { kind, value }`
- `intent`
- 可选布尔条件：字段、操作符、值、AND/OR（有界深度与项数）

允许字段覆盖 organization、brand、domain、hostname、ip、cidr、asn、cert、icp、title、body、favicon、email_domain、github_org、repository、app_id；允许 exact/contains/suffix/prefix 等受控操作。宿主按 provider 能力编译 FOFA/Hunter/Quake/Shodan/0.zone 等 DSL，执行 literal escaping、credential/rate/cost policy 和 Tool Truth receipt。模型不能看到或生成 raw secret。

### 3.3 归属判定

每个 candidate 必须有一条可审计 disposition：

- `owned`：足够证据证明属于当前企业；
- `shared`：CDN、云共享、第三方 SaaS 等共享基础设施；
- `third_party`：供应商、客户、合作方或无关实体；
- `ambiguous`：证据不足/冲突，继续搜索或 residual；
- `rejected`：格式错误、噪声或被确定性规则排除。

归属证据可由已确认官网/严格子域关系、ICP备案主体、证书主体与 SAN 组合、企业代码仓库/应用声明、测绘平台组织字段和多个独立来源组成。单一 IP 邻居、同 ASN、同证书、相似标题或模型置信度均不能单独证明 owned。

### 3.4 去重与正式身份

- 域名：canonical FQDN；
- Web：`scheme + host + port`；
- 网络端点：`ip + transport + port`；
- 证书：fingerprint；
- ASN：canonical ASN。

重复 observation 合并来源而不丢 provenance。互相矛盾的字段保留版本和来源，不能 first-write-wins 静默覆盖。

### 3.5 可达性与晋升

资产晋升由宿主 typed operator 执行，AI 只提交候选和验证意图：

- Web：限定 HEAD/GET 或浏览器导航，任何真实 HTTP 响应为 reachable；
- 非 Web：受控 TCP/协议握手响应为 reachable；
- DNS-only、超时、连接拒绝和仅有测绘历史记录均不晋升；
- 验证必须受 frozen scope/policy、速率、并发、超时和目标边界保护。

晋升事务写入：Target canonical identity、liveness state/reason/time、provider/query/fetched_at、title/status/server/OS/ASN/location/cert/service 等已观测字段、DNS/service edges、Evidence/receipt/raw artifact hashes。核心字段结构化，provider 特有字段保存在 namespaced JSONB；raw 响应继续保存到 artifact store，不塞进业务表。

## 4. 完成条件与新 Gate

Target Intel 完成由 Goal review + deterministic finalizer 决定，不读取旧六轴 coverage：

1. 本 run 至少有一个真实外部 search receipt 或明确的全体 capability residual；
2. material frontier 全部是 terminal disposition；
3. 每个正式 Target 都绑定 owned disposition、fresh reachable receipt 和 Evidence；
4. shared/third_party/ambiguous/rejected 未被晋升；
5. dedupe/conflict 集合闭合；
6. 没有 active worker/tool、stale review 或 outcome-unknown receipt；
7. reviewer 的 PASS 绑定同一状态快照；
8. host 生成 final seal 与 Target Intel → EAS handoff。

### 4.1 Main AI 短期工作记忆

Main AI 必须在同一条 durable message chain 中持续使用 `update_plan` 维护当前计划，并通过实际工具调用留下“尝试了什么、得到了什么、哪些为空/失败、哪些事实已落库、剩余方向是什么”的结构化轨迹。这里的短期记忆是宿主可回放的消息、plan tool call/result、工具 receipt、Worker output 和 checkpoint，不采集或依赖模型不可见的隐藏推理。

申请审查时，宿主把当前 Controller 的 exact message chain、checkpoint/version 与 frontier、Worker outputs、Tool Truth receipts、Observation/Target 实际落库一起冻结为 `controller_work_memory`。审查 AI 因此既能看到 Main 如何调整计划，也能核对其声称的动作是否真的发生、结果是否真的落库；自然语言 completion claim 不能覆盖这些事实。

### 4.2 审查结论与 same-chain 续跑

独立 reviewer 只读上述冻结快照，输出 `PASS | REWORK | NEEDS_HUMAN`：

- `PASS`：无 critical/major open finding，允许进入确定性 finalizer；
- `REWORK`：每个 material finding 必须携带 evidence refs、action kind 和 close condition。宿主原子创建下一 Goal epoch，解除旧 review freeze，把完整 findings/residuals 作为 trusted continuation message 追加到**同一个** Controller message chain，并把原 WorkerRun 停在可续跑状态；随后 Main 重新读取自己的短期记忆、修改 plan、补跑缺口并再次申请审查；
- `NEEDS_HUMAN`：只允许 credential、scope/subject confirmation、provider recovery 或无 material delta 的 review fixed point 等 typed hold。

REWORK 不生成固定 lane 或六轴 worklist。相同 finding 在没有新的状态、行动或落库变化时再次出现，宿主转 typed human hold，禁止无限自动循环。

旧 `GOLISH-INTEL-DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT` 不再出现在新 prompt、spec denominator、repair worklist、coverage projection 或 publication path。历史 DB 行保留为不可变审计数据，但运行时不读取兼容分支；用户明确接受旧 operation 不可按旧六轴恢复。

## 5. 失败语义

- provider 缺凭据：`unavailable`
- provider 成功且零结果：`checked_empty`
- 网络/解析/限流/服务错误：`failed` 或 `blocked`
- receipt outcome unknown：`recovery_required`
- 候选不可达：`unreachable`
- 归属证据不足：`ambiguous`

这些状态都保留 Evidence/receipt 和重试边界；任何一个都不能伪装为 found 或 checked-empty。

## 6. 安全与非目标

- Scoping 公开搜索只用于确认企业，不把搜索结果直接变成 Target。
- Target Intel 的可达性验证仅低影响，不含端口全扫、爆破、漏洞利用、写入或持久化。
- IP/ASN/CDN/证书关系不会自动扩大 scope。
- AI 不获得 raw provider DSL、secret、裸 shell 或无证据 browser。
- 不复制或再分发 CyberStrike AGPL 内容；只实现 Golish 自有方法论。

## 7. 验收

1. Scoping 在 ENScan/企查查类、0.zone、公开搜索三层分别有 found/empty/unavailable/failure 测试。
2. Scoping 只有 confirmed Company Identity 可 seal。
3. production Target Intel Main AI 能连续发出不同 semantic pivots，并根据结果调整 plan。
4. FOFA/Hunter/Shodan native 与 Quake/0.zone HTTP 都在 `receipt_v1` 下可审计；缺凭据诚实 residual。
5. 所有候选先 observation，未通过 owned + reachable 前正式 Target exact-set 不变。
6. provider 字段、raw artifact、Evidence、receipt 和 promotion lineage 可回放。
7. 代码与 prompt 中不存在旧六轴 Target Intel publication/repair 路径。
8. 受控 fixture 完整跑至 Reporting；fresh moresec.cn operation 完整经过 Scoping→Target Intel→EAS→Enumeration→Vuln→AU→Investigation→Reporting。
