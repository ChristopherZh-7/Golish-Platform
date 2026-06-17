# 测绘被动情报：域名↔IP 配对 + 探活 + 自动落库 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans` 逐任务实现此计划。每个任务单独 commit。TDD：先写失败测试，跑红，再实现转绿。纯函数（配对抽取 / scope 过滤 / liveness 映射）必须有单测；落库/探活为非致命旁路，失败只告警。

**目标：** 让 `recon_enrich_assets` 把测绘多源发现的资产**自动配对、过滤、入库为 target（带测绘 real_ip）并探活回填存活状态**，把"发现 189 / 落地 23"的缺口闭合成"发现即（过滤后）落地"。

**架构：** 在既有 asset_intel enrich 管线尾部追加三段纯增量步骤——③域名↔IP 配对（normalize 新增成对抽取）、⑤自动 profile→target 提升（复用 `persist_target_record` + `value_belongs_to_organization` scope 过滤、real_ip 取配对值）、⑥探活（httpx/dnsx 回填 `targets.status` + `dns_records`）。①多维查询改 provider JSON（运行时直读，无需重编译）。0.zone/quake 现有 http_json 路径不动，只扩 `requests`。

**技术栈：** Rust（`golish-recon-app` asset_intel / organization_recon）+ provider/toolsconfig JSON + Postgres（sqlx，复用既有 repo 写函数）+ 前端无改动（`buildHostTree` 已按 real_ip 归位）。

---

## 背景与关键证据（实现前必读，均已核对真实代码 / 运行时 2026-06-17）

| 事实 | 落点 |
|---|---|
| enrich 入口 | `golish-recon-app/src/asset_intel/agent_intel.rs::run_passive_intel(PassiveIntelPhase::Enrich)` → `run_providers_for_org` |
| runtime 分发 | `golish-recon-app/src/asset_intel/service/hydrate.rs`（`CliJson`/`HttpJson`/`NativeProvider`） |
| 归一化（配对要改这里） | `golish-recon-app/src/asset_intel/normalize.rs::extract_profile_field_entries`：按规则独立产 `(target_kind,target_field,value)`，**无成对** |
| profile 落库 | `golish-recon-app/src/asset_intel/profile_patch.rs`（折叠进 `organizations` 列） |
| 子公司提升（仅 org，**无 target 提升**） | `golish-recon-app/src/asset_intel/promote.rs::auto_promote_child_decisions` |
| 落 target（无 real_ip） | `golish-recon-app/src/organization_recon/persistence.rs::persist_target_record`（INSERT 不含 real_ip） |
| scope 过滤（复用） | `persistence.rs::value_belongs_to_organization` / `organization_owned_domains` / `normalized_host` |
| real_ip 写入 | `golish-db/src/repo/targets.rs::set_real_ip_by_id(pool,target_id,ip)` |
| DNS 落库（兜底） | `persistence.rs::land_dns_records`（LIMIT 128 / 3s）+ `refresh_per_asset_landing(pool,org_id)` |
| provider 查询声明 | `resources/intel-providers/{0-zone,quake,fofa,hunter}.json` 的 `runtime.requests`/`queries` + `normalize` |
| 探活工具 | `resources/toolsconfig/httpx.json`（已存在） |
| 前端归位（无改动） | `frontend/lib/target-panel/org-tree.ts::buildHostTree`（按 `real_ip` 挂 host 节点） |
| 运行时缺口 | DB(平安集团)：`domains=151/ip_ranges=86/asns=19`，`targets=23`；`cloud/job/yun.pingan.com` real_ip 空且 NXDOMAIN |

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-recon-app/src/asset_intel/normalize.rs` | 新增 `extract_host_ip_pairs(raw, rule)`：同一记录内成对抽 `(host, ip)` | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/types.rs` | 新增 `HostIpPair{host,ip}` + 在 provider 描述符加 `pair_rules` | 改 |
| `backend/crates/golish-pentest-domain/src/models/asset_intel.rs` | provider 描述符 `normalize` 加 `pairs: Vec<AssetIntelPairRule>`（可选，缺省空） | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/landing.rs` | **新建**：`promote_profile_assets_to_targets`（scope 过滤 + upsert + real_ip 取配对值） | 新建 |
| `backend/crates/golish-recon-app/src/asset_intel/probe.rs` | **新建**：`probe_targets_liveness`（httpx/dnsx → status/ports/title，非致命） | 新建 |
| `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs` | enrich 成功后依次调 promote → probe（非致命，仅告警） | 改 |
| `backend/crates/golish-recon-app/src/organization_recon/persistence.rs` | `value_belongs_to_organization` / `normalized_host` 提为 `pub(crate)` 供 landing.rs 复用 | 改 |
| `resources/intel-providers/quake.json` | `requests` 加 `domain:` / `cert:` 维；`normalize.pairs` 声明 `hostname↔ip` | 改 |
| `resources/intel-providers/fofa.json` | `queries` 加 `domain`/`cert`/`icp` 维 | 改 |
| `resources/intel-providers/0-zone.json` | `normalize.pairs` 声明 `domain↔msg.ip` | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/tests.rs` | 配对抽取 + scope 过滤 + liveness 映射单测 | 改 |

> 范围拆分（每段独立可测、独立 commit）：**Phase A 配对**（Task 1-3）、**Phase B 自动入库**（Task 4-5）、**Phase C 探活**（Task 6-7）、**Phase D 多维查询**（Task 8）、**Phase E 端到端验证**（Task 9）、**Phase F 工具集精简 + stage spec 对齐**（Task 10-13）。A/B 是闭环主干，C/D 是质量增强，可分 PR；F 在 A/B 落地后做（让 enrich 闭环成为主路径，CLI 工具退化为 fallback）。

---

## Task 1：定义 HostIpPair 类型 + 描述符 pair 规则（先写类型，零行为）

**文件：** `golish-recon-app/src/asset_intel/types.rs`、`golish-pentest-domain/src/models/asset_intel.rs`

**步骤：**
1. 在 `golish-pentest-domain/src/models/asset_intel.rs` 的 provider `normalize` 描述符结构里加可选字段（serde 缺省空，旧配置兼容）：
```rust
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AssetIntelPairRule {
    pub path: String,        // JSONPath 选每条记录，如 "$..data[*]"
    pub host_field: Vec<String>, // 优先级取 host：如 ["domain","hostname","service.http.host"]
    pub ip_field: Vec<String>,   // 优先级取 ip：如 ["ip","msg.ip","ip_addr"]
}
```
   并在 `AssetIntelNormalizeConfig`（含 `target`/`profile_fields` 的那个结构）追加：
```rust
    #[serde(default)]
    pub pairs: Vec<AssetIntelPairRule>,
```
2. 在 `golish-recon-app/src/asset_intel/types.rs` 加纯数据类型：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostIpPair {
    pub host: String, // 归一化后的小写 host
    pub ip: String,   // 测绘观测 IP
}
```

**验证：** `cd backend && cargo check -p golish-recon-app -p golish-pentest-domain`，零错误（仅加类型）。

**提交：** `feat(asset-intel): add HostIpPair type and pair-rule descriptor`

---

## Task 2：写 `extract_host_ip_pairs` 的失败测试（TDD 红）

**文件：** `golish-recon-app/src/asset_intel/tests.rs`

**步骤：**
1. 加测试，喂一段 quake 形态 JSON，断言抽出成对 (host, ip)：
```rust
#[test]
fn extract_host_ip_pairs_quake_record_pairs_domain_and_ip() {
    let raw = serde_json::json!({"data":[
        {"domain":"bank.pingan.com","ip":"221.11.190.218"},
        {"hostname":"www.pingan.com","ip":"61.241.22.62"},
        {"domain":"","ip":"1.2.3.4"} // host 空 → 跳过
    ]});
    let rule = crate::asset_intel::types::pair_rule_for_test(
        "$..data[*]", &["domain","hostname"], &["ip"]);
    let mut pairs = crate::asset_intel::extract_host_ip_pairs(&raw, &rule);
    pairs.sort_by(|a,b| a.host.cmp(&b.host));
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].host, "bank.pingan.com");
    assert_eq!(pairs[0].ip, "221.11.190.218");
    assert_eq!(pairs[1].host, "www.pingan.com");
    assert_eq!(pairs[1].ip, "61.241.22.62");
}
```
   （`pair_rule_for_test` 是 `#[cfg(test)]` 小构造器，放 `types.rs`。）

**验证：** `cd backend && cargo test -p golish-recon-app extract_host_ip_pairs`，预期 **编译失败/红**（函数未实现）。

**提交：** `test(asset-intel): failing test for host↔ip pair extraction`

---

## Task 3：实现 `extract_host_ip_pairs`（TDD 绿）

**文件：** `golish-recon-app/src/asset_intel/normalize.rs`（+ `types.rs` 测试构造器）

**步骤：**
1. 在 `normalize.rs` 复用既有 `select_json_values` / `resolve_field_ref`，新增：
```rust
pub(crate) fn extract_host_ip_pairs(
    raw: &Value,
    rule: &golish_pentest::models::AssetIntelPairRule,
) -> Vec<crate::asset_intel::types::HostIpPair> {
    let mut out = Vec::new();
    for item in select_json_values(raw, &rule.path) {
        let host = rule.host_field.iter()
            .find_map(|f| resolve_field_ref(item, &std::slice::from_ref(f)))
            .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|h| !h.is_empty() && h.parse::<std::net::IpAddr>().is_err());
        let ip = rule.ip_field.iter()
            .find_map(|f| resolve_field_ref(item, &std::slice::from_ref(f)))
            .map(|s| s.trim().to_string())
            .filter(|s| s.parse::<std::net::IpAddr>().is_ok());
        if let (Some(host), Some(ip)) = (host, ip) {
            out.push(crate::asset_intel::types::HostIpPair { host, ip });
        }
    }
    out
}
```
   注意：`resolve_field_ref` 现签名取 `&[String]`，若只接受单 field 需按其真实签名调整（实现时核对 `normalize.rs` 现有调用）。
2. 在 `asset_intel/mod.rs` `pub(crate) use normalize::{..., extract_host_ip_pairs};`。
3. `types.rs` 加 `#[cfg(test)] pub(crate) fn pair_rule_for_test(...) -> AssetIntelPairRule`。

**验证：** `cd backend && cargo test -p golish-recon-app extract_host_ip_pairs`，预期 **绿**。

**提交：** `feat(asset-intel): implement host↔ip pair extraction (paired domain/ip)`

---

## Task 4：scope 过滤可见性 + landing 模块骨架 + 失败测试

**文件：** `organization_recon/persistence.rs`、`asset_intel/landing.rs`（新建）、`asset_intel/tests.rs`

**步骤：**
1. 把 `persistence.rs` 的 `value_belongs_to_organization`、`organization_owned_domains`、`normalized_host` 由 `fn` 提为 `pub(crate) fn`（仅可见性，不改逻辑）。
2. 新建 `asset_intel/landing.rs`，先放纯函数 + 失败测试目标：
```rust
/// 从 (host_map + 档案 ip_ranges) 里筛出"属本 org 自有"的待入库资产。纯函数，便于测试。
pub(crate) fn plan_promotable_assets(
    org: &golish_db::models::Organization,
    pairs: &[crate::asset_intel::types::HostIpPair],
    profile_ips: &[String],
) -> (Vec<(String, Option<String>)> /* (domain, real_ip?) */, Vec<String> /* ip targets */) {
    use crate::organization_recon::persistence::value_belongs_to_organization;
    let mut domains = Vec::new();
    for p in pairs {
        if value_belongs_to_organization(org, &p.host) {
            domains.push((p.host.clone(), Some(p.ip.clone())));
        }
    }
    let ips = profile_ips.iter()
        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
        .cloned().collect();
    (domains, ips)
}
```
3. 在 `tests.rs` 加失败测试：owned 域名带配对 ip 入选、第三方域名（如 `194.1.broad.ha.dynamic.163data.com.cn`）被过滤、非法 ip 被剔除。

**验证：** `cd backend && cargo test -p golish-recon-app plan_promotable_assets`，先红后（实现后）绿；`cargo check` 通过可见性改动。

**提交：** `feat(asset-intel): scope-filtered promotable-asset planner (pure)`

---

## Task 5：自动 profile→target 提升（带测绘 real_ip，非致命）

> **实现注记（2026-06-17 执行）：** `run_providers_for_org` 的返回值**不携带** provider 原始文档
> （raw 只落 artifact 文件 + 候选 `evidence.raw`），故本步「把各 provider raw 用 extract_host_ip_pairs 汇成
> Vec<HostIpPair>」改为在 `agent_intel.rs` enrich 分支从 `run.candidates[*].evidence.raw` 抽取——
> http_json provider（0.zone/quake）的 `normalize.target` 规则会为每条 `data[*]` 记录产出 target 候选，
> 其 `evidence.raw` 保留整条记录（domain+ip 同在）。新增纯函数 `landing::pairs_from_candidates`，按 provider
> `normalize.pairs` 字段（缺则默认字段集）对单条记录（path 置 `$`）抽 (host, ip)，按 host 去重。这样**零改动**
> 14 处 `finish_provider_run` / 9 处 `normalize_json_*` 调用点（改它们会显著放大爆炸半径）。**已知边界：**
> native provider（fofa/hunter/shodan）候选 evidence 不带 raw，其 pairs 暂不覆盖（会员 key 门控，记为后续）。

**文件：** `asset_intel/landing.rs`、`asset_intel/agent_intel.rs`

**步骤：**
1. 在 `landing.rs` 加 IO 落库（复用 `persist_target_record` 的 upsert 语义 + `set_real_ip_by_id`）：
```rust
pub(crate) async fn promote_profile_assets_to_targets(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    pairs: &[crate::asset_intel::types::HostIpPair],
) -> usize {
    let profile_ips = crate::asset_intel::landing::profile_ip_strings(org); // 读 org.ip_ranges
    let (domains, ips) = plan_promotable_assets(org, pairs, &profile_ips);
    let mut landed = 0usize;
    for (domain, real_ip) in domains {
        match upsert_target(pool, org, &domain, "domain", real_ip.as_deref()).await {
            Ok(_) => landed += 1,
            Err(e) => tracing::warn!(%domain, %e, "promote domain→target failed (non-fatal)"),
        }
    }
    for ip in ips {
        if let Err(e) = upsert_target(pool, org, &ip, "ip", None).await {
            tracing::warn!(%ip, %e, "promote ip→target failed (non-fatal)");
        } else { landed += 1; }
    }
    landed
}
```
   `upsert_target`：SELECT 既有（`value=$1 AND project_path IS NOT DISTINCT FROM $2`）→ 有则 `COALESCE(organization_id)` + 有 real_ip 则 `set_real_ip_by_id`；无则 INSERT（`source='asset_intel'`, `scope='in'`，real_ip 直接进 INSERT）。对照 `persist_target_record` 写法，差异是**带 real_ip 列**。
2. 在 `agent_intel.rs::run_passive_intel` 的 Enrich 分支、`run_providers_for_org` 成功之后：把各 provider raw 用 `extract_host_ip_pairs` 汇成 `Vec<HostIpPair>`（按 host 去重，取首个 ip），调 `promote_profile_assets_to_targets`，失败只 warn。

**验证：** `cd backend && cargo test -p golish-recon-app`（含 plan 测试）+ `cargo clippy -p golish-recon-app -- -D warnings`。端到端在 Task 9 验。

**提交：** `feat(asset-intel): auto-promote scoped discovered assets to targets with surveyed real_ip`

---

## Task 6：探活模块 + liveness 状态映射失败测试

**文件：** `asset_intel/probe.rs`（新建）、`asset_intel/tests.rs`

**步骤：**
1. `probe.rs` 先放纯映射（httpx JSON 行 → 我们的 status），写失败测试：
```rust
pub(crate) fn liveness_from_httpx(line: &serde_json::Value) -> &'static str {
    match line.get("status_code").and_then(|v| v.as_i64()) {
        Some(code) if (100..600).contains(&code) => "live",
        _ if line.get("failed").and_then(|v| v.as_bool()).unwrap_or(false) => "dead",
        _ => "unknown",
    }
}
```
   测试：`{"status_code":200}`→live、`{"failed":true}`→dead、`{}`→unknown。

**验证：** `cd backend && cargo test -p golish-recon-app liveness_from_httpx`（红→绿）。

**提交：** `test(asset-intel): liveness status mapping (httpx → target.status)`

---

## Task 7：探活落库（httpx/dnsx，非门槛、非致命）

> **被 Task 13 改判（2026-06-17 执行）：** 为保 target_intel 的 zero-touch 契约，Phase C 在本阶段**只实现
> `probe::liveness_from_httpx` 纯映射 + 单测**，**不**在 enrich 主路径调 httpx。下文「在 `agent_intel.rs`
> Enrich 尾部 promote 之后调 probe」一步**不执行**；主动探活 IO 下沉 EAS specialist 接管。`liveness_from_httpx`
> 以 `#[cfg_attr(not(test), allow(dead_code))]` 标注，作为待 EAS 接线的稳定契约。详见 Task 13。

**文件：** `asset_intel/probe.rs`、`asset_intel/agent_intel.rs`

**步骤：**
1. `probe_targets_liveness(pool, org)`：取本 org `scope='in'` 且 `status` 未探活的 target（限流，复用 `land_dns_records` 的 LIMIT 128 风格），对域名/IP 跑 httpx（经 `golish_pentest` 工具运行器，工具 id `httpx`，`-json -silent`），把 `liveness_from_httpx` 结果 `update targets set status=$.. , http_title/http_status/webserver/cdn_waf=...`；**dead 也写 status，不删行**（D2）。
2. 测绘没给 IP 的域名：调既有 `refresh_per_asset_landing(pool, org.id)` 做一次 DNS 兜底（已幂等）。
3. 在 `agent_intel.rs` Enrich 尾部 promote 之后调 probe，失败只 warn。

**验证：** `cd backend && cargo test -p golish-recon-app` + `cargo clippy ... -D warnings`。端到端在 Task 9。

**提交：** `feat(asset-intel): non-gating liveness probe (httpx) backfills target status, never drops dead assets`

---

## Task 8：provider 多维查询（配置，运行时直读）

**文件：** `resources/intel-providers/quake.json`、`fofa.json`、`0-zone.json`

**步骤：**
1. `quake.json` `runtime.requests` 在现有 `org` / `icp_unit` 外，追加（`{{company_name}}` 渲染处保留，新增 `{{root_domain}}` 由 runner 对每个自有根域展开；若 runner 暂不支持多值，先加 `cert` 维）：
```json
{ "id": "cert", "method": "POST", "url": "https://quake.360.net/api/v3/search/quake_service",
  "headers": {"Content-Type":"application/json","X-QuakeToken":"{{secret:api_key}}"},
  "json": { "query": "cert: \"{{company_name}}\"", "size": 100, "latest": true,
    "include": ["ip","port","domain","hostname","asn","service.cert","service.http.host"] } }
```
   并在 `quake.json` `normalize` 加：`"pairs":[{"path":"$..data[*]","host_field":["domain","hostname","service.http.host"],"ip_field":["ip"]}]`。
2. `fofa.json` `runtime.queries` 由仅 `org=` 扩为多维（runner 逐个根域展开 `{{root_domain}}`）：
```json
"queries": [
  { "query_type": "site", "template": "org=\"{{company_name}}\"" },
  { "query_type": "cert", "template": "cert.subject.cn=\"{{root_domain}}\"" },
  { "query_type": "domain", "template": "domain=\"{{root_domain}}\"" }
]
```
3. `0-zone.json` `normalize` 加 `pairs`：`[{"path":"$..data[*]","host_field":["domain","url","host"],"ip_field":["ip","msg.ip","ip_addr"]}]`。
4. 若 runner 尚不支持 `{{root_domain}}` 多值展开：本任务仅落 `cert`/`pairs`（单值），`{{root_domain}}` 展开拆到后续小任务（在计划里显式记，不留 TODO 字样）。

**验证：** `cd backend && cargo test -p golish-recon-app`（provider JSON 解析单测覆盖 `pairs` 字段反序列化）；手动 `just check-fe` 不涉及。运行期：配了 quake 会员 key 后跑一次 enrich，直查 `targets` 含证书维新域名。

**提交：** `feat(intel-providers): add domain/cert query dims + host↔ip pair rules (quake/fofa/0.zone)`

---

## Task 9：端到端验证 + 证据记录

**文件：** 无代码改动（验证 + 写 `agent-progress.md` 证据）

**步骤：**
1. `./init.sh` 绿；`just precommit` 绿。
2. 起 `just dev`，对 pingan 工程跑一次 `target_intel` enrich（或最小复现：直接调 `recon_enrich_assets{org=平安集团}`）。
3. 直查 DB 对比前后：
```bash
python3 - <<'PY'
import psycopg2
c=psycopg2.connect("postgres://golish:golish_local@localhost:15432/golish");q=c.cursor()
q.execute("select count(*) from targets"); print("targets",q.fetchone())
q.execute("select count(*) from targets where target_type::text='domain' and real_ip<>''"); print("domain w/ real_ip",q.fetchone())
q.execute("select count(*) from targets where status is not null"); print("probed",q.fetchone())
q.execute("select value from targets where target_type::text='domain' and (real_ip is null or real_ip='')"); print("still unresolved",q.fetchall())
PY
```
   预期：`targets` 从 ~23 升到"档案过滤后规模"（数量级↑），多数域名有 real_ip，`status` 多为 live/dead（无大量 NULL），"still unresolved" 仅剩真正测绘+DNS 都无 IP 者。

**验证：** 把上述命令输出复制进 `agent-progress.md` 的"已记录证据"。`just precommit` 全绿。

**提交：** `docs(progress): record passive-intel landing closure e2e evidence`

---

# Phase F：工具集精简 + stage spec 与 enrich 闭环对齐（Task 10-13）

> **背景（实现前必读，均已对照真实代码核实 2026-06-17）：** Phase A/B 落地后，`recon_enrich_assets`
> 的「测绘 → 配对 → scope 过滤 → 自动入库（带 real_ip）→ landing」已经把 6 类被动情报技术
> （DNS/SUBDOMAIN/ASN/CT/WHOIS/OSINT）写进 gate 读的 DB 真相表。`target_intel` 的真正完成门控是
> `coverage_complete`（`authoritative_found: true`，**读 DB 真相**，见 `resources/harness/stages/target_intel.json:48-58`），
> 而**不是** CLI 工具的调用次数。因此 intel 阶段的 CLI 工具（dig/subfinder/amass/asnmap/ctfr）从
> 「必跑」退化为「enrich 没把某格子落库时才补的 fallback」。Phase F 把 spec 与 methodology 对齐到这个事实，
> 避免 agent 重复跑已被 enrich 覆盖的 CLI，又不放松完整性门控。
>
> | 关键事实 | 落点（已核实） |
> |---|---|
> | 真正的完成门控 = `coverage_complete`（authoritative，读 DB） | `target_intel.json:48-58`；`gate_rules` 数组里**没有** `min_invocations` named_check |
> | `min_invocations`（dns_resolve/subdomain_enum_passive）只驱动**提示词** | `task_orchestrator/prompts/mod.rs:98-111`（`spec.min_invocations` 空时打印 "(no per-tool minimum)"） |
> | `min_invocations` 在 `vacuous_check` 的 FakePattern 里**被 `facts_from_db_truth` 跳过** | `harness/gate/vacuous_check.rs:52`（`if !db_truth_backed && !spec.min_invocations.is_empty()`）；target_intel `facts_from_db_truth: true` |
> | `min_invocations_check` 用 `required_checks_done` 自报名单做 `.contains()` 包含匹配 | `harness/gate/min_invocations_check.rs:15-23` |
> | httpx 在 target_intel 被**禁用**（属 EAS） | `target_intel.methodology.md:3-4,50-52`；`tool_taxonomy.rs` httpx→`recon/http` 不在 allowed_tool_types |

---

## Task 10：失败测试——target_intel 不应再声明硬性工具下限（TDD 红）

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/min_invocations_check.rs`（`#[cfg(test)] mod tests`）

**步骤：**
1. 追加测试，断言 target_intel 不再声明 `min_invocations`（enrich landing + coverage_complete 已保证完整性）：
```rust
#[test]
fn target_intel_declares_no_hard_tool_floor() {
    // enrich(测绘+配对+自动入库+landing) 产出 DB 真相覆盖；coverage_complete
    // (authoritative, target_intel.json:48-58) 是真正的完整性门控。dns_resolve/
    // subdomain_enum_passive 自报下限既冗余、又会让提示词逼 agent 重复跑已被 enrich
    // 覆盖的 CLI。故 target_intel 不应再声明硬性工具下限。
    let spec = load_embedded_stage_spec(StageKind::TargetIntel).unwrap();
    assert!(
        spec.min_invocations.is_empty(),
        "target_intel must not declare hard tool floors; coverage_complete (DB truth) enforces completeness"
    );
}
```

**验证：** `cd backend && cargo test -p golish-agent-kit target_intel_declares_no_hard_tool_floor`，预期 **红**（当前 spec 仍有 `dns_resolve:1, subdomain_enum_passive:1`）。

**提交：** `test(harness): target_intel should declare no hard tool floor (enrich closure)`

---

## Task 11：放宽 target_intel `min_invocations` 至空（TDD 绿）

**文件：** `resources/harness/stages/target_intel.json`

**步骤：**
1. 把 `min_invocations` 由两条下限改为空对象，并改写 `$comment_min_invocations` 记录原因（保留历史决策线索，不删旧注释语义）：
```json
  "$comment_min_invocations": "2026-06-17 passive-intel-closure Phase F：移除 dns_resolve/subdomain_enum_passive 硬下限。enrich(测绘多源→域名↔IP配对→scope过滤→自动入库→landing) 已把 DNS/SUBDOMAIN 等写进 gate 读的 DB 真相表，完整性由 coverage_complete(authoritative_found, 读 DB) 强制；旧硬下限只驱动提示词(prompts/mod.rs:98-111)逼 agent 重复跑 dig/subfinder，且在 vacuous_check FakePattern 里已被 facts_from_db_truth 跳过(vacuous_check.rs:52)。CLI(dig/subfinder/amass/asnmap/ctfr) 退化为 enrich 未落某格子时的 fallback(见 methodology 第 5 点)。如未来要把某工具重新设为硬下限，先评估它是否真的无法由 enrich landing 覆盖。",
  "min_invocations": {},
```
   注意：`min_invocations` 的 JSON value 必须是对象 `{}`（`spec.min_invocations: HashMap`），不可写 `null`。

**验证：**
```bash
cd backend && cargo test -p golish-agent-kit target_intel_declares_no_hard_tool_floor   # 绿
cargo test -p golish-agent-kit min_invocations   # 既有用例不回归
cargo test -p golish-agent-kit -- harness::resources   # 内嵌 spec 反序列化通过
```
预期：目标测试转绿；`min_invocations_check` 既有用例（用 Enumeration/EAS spec）不受影响。

**提交：** `feat(harness): drop target_intel hard tool floors; coverage_complete (DB truth) governs completeness`

---

## Task 12：methodology 对齐——enrich 为主、CLI 降为 fallback（文档，agent 行为）

**文件：** `resources/harness/stages/target_intel.methodology.md`

**步骤：**
1. 把「Recommended sequence」改成「enrich(含自动配对+入库+real_ip) 为唯一主路径，CLI 仅在某覆盖格子 enrich 没落库时按需补」，并删除「Passive subdomain enumeration 必跑」「URL history 必跑」这类硬性措辞（已无硬下限）。替换第 2-3 点为：
```markdown
2. CLI 仅作 fallback（zero-touch）— enrich 已把 SUBDOMAIN→target_assets、DNS→dns_records、
   ASN/CT/WHOIS→organizations.asns/.certificates/.whois、OSINT→organizations.intel 落库（Phase A/B
   起还会自动把测绘配对资产入库并带 real_ip）。**只对 coverage 仍为空的格子**按需补一次对应 CLI：
   SUBDOMAIN→`subfinder -all`/`amass enum -passive`、CT→`ctfr -d <root>`、ASN→`asnmap -d <root>`、
   DNS→`dig`（仅当某 in-scope 域名 enrich+解析都没拿到记录时）。每个工具每根域**最多跑一次**；跑完仍空
   则把该格子 submit 成 `checked_empty+evidence` 或 `blocked+note`，不要反复重试同一工具。
3. URL history（`gau`/`waybackurls`）按需用于历史端点；它与测绘正交，非完整性门控要求。
```
2. 保留并强化既有红线：`dig` 不逐子域跑、不跑 `httpx`/`nmap`/端口扫描（仍属 EAS）——这条与 Phase F 的「enrich 为主」一致，无需改。

**验证：** `rg -n "subfinder -all -recursive|run each technique ONCE" resources/harness/stages/target_intel.methodology.md`（确认旧「必跑」措辞已替换）；人工通读确认「enrich 主、CLI fallback」表述无歧义。无代码改动，无需编译。

**提交：** `docs(harness): target_intel methodology — enrich is primary, CLI tools are fallback-only`

---

## Task 13：定夺探活落点——保住 target_intel zero-touch（设计决策 + 文档）

> **解决冲突：** 设计文档 §2⑥/§4 把「httpx 轻量探活」写进 enrich，但 enrich 跑在 `target_intel`，
> 而该阶段方法论明确是 **zero-touch / 被动**、禁用 httpx（methodology.md:3-4,50-52）。Phase C（Task 6-7）
> 若在 enrich 尾部调 httpx，会让被动阶段对目标主机发包，违反阶段契约与 `human_approval.required_before:["active_scan"]`。
> **决策：** target_intel 保持 zero-touch——real_ip 只由「测绘配对值（Phase A）+ 被动 DNS 解析（land_dns_records）」得到；
> **主动 httpx 探活下沉到 EAS**（httpx 本就是 EAS 指派工具，见 `execute.rs:2748-2751`）。

**文件：** `docs/superpowers/plans/2026-06-17-passive-intel-pairing-probe-landing.md`（Task 7 注记）、`docs/design/2026-06-17-passive-intel-pairing-probe-landing.md`（§2⑥/§4 注记）、`backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`（不在 Enrich 调 probe）

**步骤：**
1. **Task 7 改写（本计划内）：** 把 Task 7 的「在 `agent_intel.rs` Enrich 尾部 promote 之后调 probe」一步**移除**；Task 7 仅保留 `liveness_from_httpx`（Task 6 的纯映射）作为**可复用工具函数**，其 IO 调用方改为 EAS specialist 路径（不在本计划 target_intel scope 内落地，作为显式 scope 边界记录，留待 EAS 增强计划接管）。即：Phase C 在 target_intel 内**只做到纯函数 + 单测**，不接 enrich 主路径。
2. **agent_intel.rs：** 确认 `run_passive_intel(PassiveIntelPhase::Enrich)` 尾部**只调 `promote_profile_assets_to_targets`（Phase B）+ `refresh_per_asset_landing` 被动 DNS 兜底**，**不调** httpx 探活。若 Task 7 早期版本已接，删除该调用。
3. **设计文档注记（不删旧设计，按 AGENTS.md §2.4 注记）：** 在 `docs/design/2026-06-17-passive-intel-pairing-probe-landing.md` 的 §2 管线 ⑥ 行与 §4 行各加一行：
```markdown
> Phase F 决策（2026-06-17）：⑥ 主动 httpx 探活下沉到 EAS（target_intel 保持 zero-touch）；
> 本阶段 real_ip 仅来自测绘配对 + 被动 DNS。liveness/端口/指纹属 EAS。
```

**验证：** `rg -n "httpx" backend/crates/golish-recon-app/src/asset_intel/` 确认 enrich 主路径无 httpx 调用；`cd backend && cargo test -p golish-recon-app liveness_from_httpx`（纯映射单测仍绿）；`cd backend && cargo check -p golish-recon-app` 通过。

**提交：** `refactor(asset-intel): keep target_intel zero-touch — defer active httpx liveness to EAS`

---

## 自检

1. **规格覆盖度**：设计 §2 七步 → ①Task8 / ②既有 merge / ③Task1-3 / ④Task4 / ⑤Task5 / ⑥**Phase F Task13 改判**（httpx 探活下沉 EAS，target_intel 保持 zero-touch；Phase C 在本阶段仅留纯映射 Task6 + 单测）/ ⑦前端既有。§3 不变量：D1=Task5（real_ip 取配对）、D2=Task7（dead 不丢，随探活迁 EAS 一并生效）、D3=Task4（scope 过滤）、D4=Task5（warn 不回滚）、D5=Task8（多维）、D6=Task8（会员字段退化）。全覆盖。
   - **Phase F 覆盖（工具集精简 + spec 对齐）**：Task10-11=移除 target_intel 硬性工具下限（coverage_complete/DB 真相为唯一完整性门控）；Task12=methodology 改 enrich 为主、CLI(dig/subfinder/amass/asnmap/ctfr) 退化 fallback；Task13=定夺探活落点保 zero-touch。回答「intel 阶段哪些工具变冗余」：asnmap/ctfr/dig/subfinder/amass 由「必跑」降为「enrich 未落格子时的 fallback」，whois（refresh 故意跳过）与 gau/waybackurls（与测绘正交）保留，httpx 不属本阶段（EAS）。
2. **占位符扫描**：Task8 第 4 点已显式说明"`{{root_domain}}` 多值展开若 runner 不支持则拆后续小任务"，非 TODO 占位；其余步骤均含真实代码块与命令。
3. **类型一致性**：`HostIpPair{host,ip}`（Task1）→ `extract_host_ip_pairs`（Task3）→ `plan_promotable_assets`/`promote_profile_assets_to_targets`（Task4-5）签名一致；`AssetIntelPairRule{path,host_field,ip_field}` 在 Task1 定义、Task3/Task8 使用一致。
