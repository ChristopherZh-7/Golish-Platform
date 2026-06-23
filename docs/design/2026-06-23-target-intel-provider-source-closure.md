# target_intel provider source closure

## 背景

`target_intel` 已经走向 DB-truth 裁决：agent 负责运行采集工具并让数据落库，gate 从
`target_assets`、`dns_records`、`organizations.*` 和 evidence ledger 投影 found。
但当前阶段边界仍混着两类路径：

- registry 工具：`recon_list_providers`、`recon_map_assets`、`recon_lookup_whois`
- scan-tool wrapper：`dig`、`ctfr`、`asnmap`、`gau`、`waybackurls`、历史上还会漂到
  subdomain CLI fallback

这会带来两个问题：

1. agent 会把 provider 未覆盖的格子补成 CLI 查询，导致 target_intel 的证据口径变成
   provider + 临时 fallback 的混合体。
2. gate 现在能证明“某个 coverage cell 有终态”，但不能严格证明“每个 provider/source
   都查过、没有重复查询、被动数据完整”。

## 目标

本轮收紧 stage contract，不再把 `target_intel` 设计成“provider 不够就跑 CLI”的阶段；
同时把 provider/source terminal row 接入 gate context，并在同一 run 内阻断重复 registry
recon action。

## 非目标

- 不证明互联网被动数据“完整”。provider 本身不是全网穷举；本设计只保证平台记录哪些
  provider/source 被查询、产生了什么终态。
- 不新增 migration。现有 `technique_outcomes` 与 `source_query_log` 已能承接第一版。
- 不把 active scan 或 live probing 搬回 `target_intel`。
- 不把 `source_query_log` 当 found 真值。它只证明 source/provider 已尝试并终态；found
  仍由 DB truth / evidence ledger 决定。

## 决策

### D1. target_intel 不再允许 scan-tool wrapper

`resources/harness/stages/target_intel/spec.json` 的 `allowed_tool_types` 收成空数组。
这只影响 scan wrapper/`pentest_run`/shell 类工具，不影响 registry 工具。

允许的 target_intel 路径变成：

- `recon_list_providers`：只读 provider 可用性。
- `recon_map_assets`：主 provider survey/landing path。
- `recon_lookup_whois`：RDAP WHOIS，一次 org 级查询。
- `recon_discover_subsidiaries`：红队/需要子公司发现时使用。

如果某个 technique 没有 provider/source、provider 不可用、或运行失败，agent 应提交
`blocked` / `checked_empty` / `not_applicable` 终态，而不是切到 CLI fallback。

### D2. 过 gate 的三层证据

短期 gate 仍以 coverage cell 终态为准：

- DB truth：能从真实落库数据投影出的 found。
- deliverable terminal cells：DB 不能投影的 `checked_empty` / `blocked` /
  `not_applicable`。
- evidence ledger：所有 claim 与 checked_empty 必须有证据；found 由 DB truth 补。

provider/source 审计现在是 gate context 的一部分，但只负责证明 source 尝试：

- `technique_outcomes`：每个 `(run, asset, technique)` 的终态。
- `source_query_log`：每个 `(run, source, query, target)` 的查询审计。
  Phase 2 已把 `recon_map_assets` / `recon_discover_subsidiaries` 的
  `providerStatus` 和 `recon_lookup_whois` 的 RDAP 结果写入该表。

`source_coverage` 规则只看 terminal source row 是否存在：

- `recon_map_assets` 的 providerStatus rows 证明 DNS/SUBDOMAIN/ASN/CT/OSINT provider
  survey 已尝试。
- `recon_lookup_whois` 的 RDAP row 证明 WHOIS 已尝试。
- `blocked` / `not_applicable` coverage cell 带非空 note 时，表示没有可调用 source/provider，
  不强制再要求 source row。
- source row 不投影 found；即使 source row status=`found`，`coverage_complete`
  `authoritative_found` 仍必须从 DB/ledger truth 看到真实数据。

### D3. provider 数据不等于完整被动收集

`recon_map_assets` 是 provider survey + DB landing，不是“完整收集证明”。它只能证明：

- 哪些 provider 被选中/运行；
- provider 返回的数据已经按当前 landing 规则写入 DB；
- 未返回/不可用/失败的维度需要显式终态。

完整性必须以后从 source contract 里定义：哪些 source 对哪些 technique 负责、每个 source
是否 terminal、重复查询是否被拦截。

### D4. 同 run 重复 action 阻断

`source_query_log` 的唯一键是 `(run_id, source, query, target)`，写入侧天然幂等。
runtime 现在还在 registry tool 执行前查同一 run/action 是否已有 terminal source row：

- `recon_map_assets` → `query=map_assets`
- `recon_lookup_whois` → `query=lookup_whois`
- `recon_discover_subsidiaries` → `query=discover_subsidiaries`

若存在 terminal row，则返回 `skipped_duplicate=true` 和已有 `evidence_ids`，不再调用
provider/registry tool。

## 后续分期

1. Phase 1：收紧 target_intel 工具边界和提示，不再建议/允许 CLI fallback。
2. Phase 2：把 `recon_map_assets` / `recon_lookup_whois` 的 provider/source 状态更完整地写入
   `source_query_log`。（已完成第一版：providerStatus/RDAP terminal rows）
3. Phase 3：gate 增加 source coverage 只读检查：每个 expected technique 至少有一个
   applicable source terminal，且 failed/blocked 与 checked_empty 分开。（已完成）
4. Phase 4：dispatch/provider runner 增加 duplicate guard，避免同 run 重复查询同一
   recon action。（已完成）

## 验收

- target_intel spec 不再暴露 scan-tool selectors。
- stage charter / refiner hint 不再引导 `dig`、`ctfr`、`asnmap`、URL-history CLI 或
  subdomain CLI fallback。
- 单测覆盖 target_intel provider-only 边界。
- gate context 读取 `source_query_log`，`source_coverage` 规则验证 source terminal，但不把
  source row 当 found。
- runtime duplicate guard 遇到同 run/action 已 terminal 时跳过 provider 调用。
- 文档明确：provider-only 不是完整性证明，只是当前证据闭环的第一版边界。
