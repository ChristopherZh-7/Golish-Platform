# PR1 · Recon 落库闭合 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans` 逐任务实现此计划。
> 关联设计：`docs/design/2026-06-15-db-truth-single-source-deliverable.md` §5 PR1、`docs/design/2026-06-14-target-intel-landing-and-tools.md` §2③。
> 决策：D1=抽共享函数（§8）、D3=WHOIS 走现成 RDAP。**D2（per-technique 可引用账本 fact）移到 PR2**——它只在 PR2 被消费（让投影 deliverable 引用真实 id），PR1 保持「数据落进业务表」这一可独立验证的干净单元。

**目标：** 让 AI agent 的被动情报路径（`recon_enrich_assets` → `run_passive_intel`）把侦察结果落进 coverage gate 实际读的业务表（`dns_records` / `target_assets(subdomain)` / `organizations.certificates` / `organizations.whois`），复用 GUI org-recon 路径**已实现**的 landing 钩子，不重写。
**架构：** 把 `organization_recon/persistence.rs` 里现有的三个私有 landing 钩子（`land_subdomain_assets` / `land_dns_records` / `land_ct_and_whois`）收敛为一个 `pub(crate)` 入口 `land_target_intel_coverage`，由 GUI 路径（`persist_normalized_records`）和 agent 路径（`run_passive_intel` 的 Enrich 阶段）**共用**。子域来源从 `&[NormalizedReconRecord]` 抽象为 `&[String]` host 列表（GUI 用 records、agent 用 `organizations.domains`）。
**技术栈：** Rust 2021（crate `golish-recon-app`）、sqlx、tokio；`cargo nextest`。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 改动 | 职责 |
|---|---|---|
| `backend/crates/golish-recon-app/src/organization_recon/persistence.rs` | 改 | 三 landing 钩子收敛为 `land_target_intel_coverage`；`collect_subdomain_pairs` / `land_subdomain_assets` 入参 records→hosts；`persist_normalized_records` 改调共享入口 |
| `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs` | 改 | Enrich 阶段：reload org → 取 `domains` host 列表 → 调 `land_target_intel_coverage` |
| `backend/crates/golish-recon-app/src/organization_recon/mod.rs`（或对应 mod 声明处） | 可能改 | 确保 `persistence::land_target_intel_coverage` 对 `asset_intel` 模块 `pub(crate)` 可见 |

> 不新增文件、不改 DB schema、不改 ts-rs/IPC（均为后端内部）。

---

## Task 1 · `collect_subdomain_pairs` 入参 records → hosts（TDD 先改测试）

**文件：** `persistence.rs`（函数 `:197`、单测 `:1211-1256`）

**步骤 1.1（测试先行）** 把 3 个现有单测改成传 host 列表而非 records。例（`collect_subdomain_pairs_maps_owned_subdomains_to_root`）：

```rust
#[test]
fn collect_subdomain_pairs_maps_owned_subdomains_to_root() {
    let org = org_with_domains(&["pingan.com"]); // 既有测试 helper
    let hosts = vec![
        "a.pingan.com".to_string(),
        "pingan.com".to_string(),   // 等于 root 本身 → 丢弃
        "x.notowned.com".to_string(), // 无 owned root → 丢弃
    ];
    let pairs = collect_subdomain_pairs(&org, &hosts);
    assert_eq!(pairs, vec![("pingan.com".to_string(), "a.pingan.com".to_string())]);
}
```

**步骤 1.2** 改函数签名 + 循环源：

```rust
fn collect_subdomain_pairs(
    organization: &golish_db::models::Organization,
    hosts: &[String],
) -> Vec<(String, String)> {
    let roots = organization_owned_domains(organization);
    if roots.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for raw in hosts {
        let Some(host) = normalized_host(raw) else { continue; };
        if roots.iter().any(|root| root == &host) { continue; }
        let Some(root) = roots
            .iter()
            .filter(|root| host.ends_with(&format!(".{root}")))
            .max_by_key(|root| root.len())
        else { continue; };
        if seen.insert((root.clone(), host.clone())) {
            pairs.push((root.clone(), host));
        }
    }
    pairs
}
```

**验证：** `cd backend && cargo nextest run -p golish-recon-app collect_subdomain_pairs`（3 测试通过）。
**提交：** `refactor(recon): collect_subdomain_pairs takes host list`

---

## Task 2 · `land_subdomain_assets` 入参 records → hosts

**文件：** `persistence.rs`（函数 `:235`）

**步骤 2.1** 改签名，内部调新版 `collect_subdomain_pairs`：

```rust
async fn land_subdomain_assets(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    subdomain_hosts: &[String],
) -> Result<usize, GolishError> {
    let pairs = collect_subdomain_pairs(organization, subdomain_hosts);
    // ……（其余 root_targets 解析 + target_assets::upsert 循环逐字不变）
}
```

**验证：** `cargo nextest run -p golish-recon-app`（编译过 + 既有测试不回归）。
**提交：** `refactor(recon): land_subdomain_assets takes host list`

---

## Task 3 · 收敛为共享入口 `land_target_intel_coverage`

**文件：** `persistence.rs`

**步骤 3.1** 新增 summary 结构 + `pub(crate)` 入口（三钩子各自非致命，逐个 warn 不阻断）：

```rust
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CoverageLandingSummary {
    pub subdomains: usize,
    pub dns_records: usize,
    pub certificates: usize,
    pub whois: bool,
}

/// 把一次被动情报采集的结果落进 coverage gate 读的业务表（design
/// 2026-06-15 §5 PR1）。GUI org-recon 与 agent enrich 两条路径共用。
/// 每个钩子非致命：失败只 warn，绝不回滚已提交的 recon 持久化。
pub(crate) async fn land_target_intel_coverage(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    subdomain_hosts: &[String],
) -> CoverageLandingSummary {
    let subdomains = land_subdomain_assets(pool, organization, run_id, subdomain_hosts)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(organization_id = %organization.id, %error, "subdomain landing failed");
            0
        });
    let dns_records = land_dns_records(pool, organization).await.unwrap_or_else(|error| {
        tracing::warn!(organization_id = %organization.id, %error, "dns_records landing failed");
        0
    });
    let (certificates, whois) =
        land_ct_and_whois(pool, organization).await.unwrap_or_else(|error| {
            tracing::warn!(organization_id = %organization.id, %error, "ct/whois landing failed");
            (0, false)
        });
    CoverageLandingSummary { subdomains, dns_records, certificates, whois }
}
```

**步骤 3.2** `persist_normalized_records`（`:144-189` 原三段 inline）替换为：

```rust
let subdomain_hosts: Vec<String> = records
    .iter()
    .filter(|r| matches!(r.kind, ReconRecordKind::Domain))
    .map(|r| r.value.clone())
    .collect();
let landed = land_target_intel_coverage(pool, organization, run_id, &subdomain_hosts).await;
tracing::info!(
    organization_id = %organization.id,
    subdomains = landed.subdomains,
    dns_records = landed.dns_records,
    certificates = landed.certificates,
    whois = landed.whois,
    "target_intel coverage landing (org-recon path)"
);
```

**验证：** `cargo nextest run -p golish-recon-app`（org-recon 路径测试不回归）。
**提交：** `refactor(recon): unify coverage landing into land_target_intel_coverage`

---

## Task 4 · agent 路径接入（`run_passive_intel` Enrich 阶段）

**文件：** `asset_intel/agent_intel.rs`（`run_passive_intel`，在 `run_providers_for_org` 之后、构造 `PassiveIntelSummary` 之前）

**步骤 4.1** Enrich 阶段：reload org 取新写入的 `domains` 作为子域来源，调共享入口。`run_id` 用本 run 已有的 id（与 `PassiveIntelSummary.run_id` 同源）。

```rust
// Coverage-gate landing (design 2026-06-15 §5 PR1): enrich 把域名/子域写进
// organizations.domains，但 gate 读 per-asset 业务表。复用 org-recon 的 landing
// 钩子把它们落进 dns_records / target_assets / certificates / whois。
// 仅 Enrich 阶段（Subsidiaries 阶段不产被动情报覆盖）；非致命。
if phase == PassiveIntelPhase::Enrich {
    match golish_db::repo::organizations::get_one(pool.as_ref(), organization_id).await {
        Ok(Some(fresh)) => {
            let subdomain_hosts: Vec<String> = fresh
                .domains
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let landed = crate::organization_recon::persistence::land_target_intel_coverage(
                pool.as_ref(),
                &fresh,
                &run_id,
                &subdomain_hosts,
            )
            .await;
            tracing::info!(
                run_id = %run_id,
                subdomains = landed.subdomains,
                dns_records = landed.dns_records,
                certificates = landed.certificates,
                whois = landed.whois,
                "target_intel coverage landing (agent path)"
            );
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "reload org for coverage landing failed"),
    }
}
```

**步骤 4.2** 若 `crate::organization_recon::persistence` 对 `asset_intel` 不可见，把 `mod persistence` 提升为 `pub(crate) mod persistence`（`organization_recon/mod.rs`），并确认 `land_target_intel_coverage` + `CoverageLandingSummary` 为 `pub(crate)`。确认 `Organization.domains` 字段类型（`serde_json::Value`，与 `land_ct_and_whois` 里 `organization.certificates.as_array()` 同款）；若为 `Option<Value>` 则相应 `.as_ref().and_then(|v| v.as_array())`。

**验证：** `cd backend && cargo check -p golish-recon-app`（编译过）。
**提交：** `feat(recon): land target_intel coverage on agent enrich path`

---

## Task 5 · 验证（端到端 + 回归）

**步骤 5.1** 单测回归：`cd backend && cargo nextest run -p golish-recon-app`。
**步骤 5.2** 端到端（手动，复现 pingan target_intel）：跑一次 `recon_enrich_assets`，对该 org 直查库：

```sql
SELECT count(*) FROM dns_records dr JOIN targets t ON dr.target_id=t.id
  WHERE t.organization_id = '<org>';                    -- 期望 >0
SELECT count(*) FROM target_assets ta JOIN targets t ON ta.target_id=t.id
  WHERE t.organization_id = '<org>' AND ta.asset_type='subdomain'; -- 期望 >0
SELECT (certificates <> '[]'::jsonb), (whois IS NOT NULL)
  FROM organizations WHERE id = '<org>';                -- 期望 t,t（有域名时）
```

**步骤 5.3** gate 行为对照：跑一次 target_intel，确认 `coverage_complete`（authoritative）对已落库技术不再判 `never attempted`（仍会因 PR2 未做的坎 2 在 vacuous/corroborated 处 BLOCK——这是预期，PR2 解决）。
**步骤 5.4** `just precommit` 全绿；证据（命令 + 退出码 + 关键输出）抄进 `agent-progress.md`。
**提交：** （文档）`docs: record PR1 landing-closure validation evidence`

---

## 自检（writing-plans 规范）

- **规格覆盖**：设计 §5 PR1 的两项（① 复用 landing 钩子进 agent 路径；② WHOIS/CT 经现成 RDAP/crt.sh）→ Task 3+4 覆盖①、land_ct_and_whois 现成覆盖②。per-technique fact（D2）已显式移至 PR2，不在本计划。
- **占位符扫描**：无 TODO/待定；每步有代码或精确命令。
- **类型一致性**：`collect_subdomain_pairs`/`land_subdomain_assets` 改 `&[String]` 后，Task 3 的 `subdomain_hosts` 与 Task 4 的 `subdomain_hosts` 同型；`CoverageLandingSummary` 字段在 Task 3 定义、Task 3/4 日志使用一致。
- **边界**：landing 全非致命（warn 不阻断）；仅 Enrich 阶段；org 隔离由各钩子既有 SQL 保证；reload org 失败只 warn。
