# DB 真值驱动的 gate coverage 实现计划（首步 · target_intel 灰度）

> **状态更新（2026-06-22 · 核当前代码 + git log）**：✅ **已落地**——execute.rs stage-close hook 从业务表派生 facts（`db_truth_facts`）合并进 `GateContext.evidence_facts`，`coverage_complete` 经 `derive_from_evidence` 认 DB 真值；`golish-db` 业务表只读查询 + 哨兵过滤已就位。target_intel 灰度生效。

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/SKILL.md` 逐任务实现此计划，配 `.cursor/skills/test-driven-development/SKILL.md`（先红后绿）。每个写操作后 `ReadLints`，每任务后跑该任务的验证命令。

**目标：** 让 `target_intel` 阶段的 coverage 判定以**数据库业务表真实结构化数据**为准——`organizations.asns`/`.certificates` 专列非空 → 该资产的 ASN/CT 算 `Found`；`target_assets(asset_type='subdomain')` 存在 → 该资产 SUBDOMAIN 算 `Found`，不再只认 agent 自报 / 命令派生。

**架构：** 复用现有 `evidence_facts` 通道（设计 §5.3 最优雅取舍）。外层 `execute.rs` stage-close hook 在从 `audit_log` 派生 facts 之外，**再从业务表派生 facts**，合并后一起注入 `GateContext.evidence_facts`。gate 纯函数层（`coverage_complete`）与 `target_intel.json` **零改动**（`derive_from_evidence` 早已开）。全部工作落在：① `golish-db` 加一个只读查询；② `execute.rs` hook 多查业务表并合并；③ `synthesize_from_evidence` 加哨兵过滤守 fabricated 红线。

**技术栈：** Rust 2021 / `sqlx` + pg-embed / `cargo nextest` / golish-agent-kit harness gate。

---

## 0. 背景与现状（动手前必读，已逐行核对源码 2026-06-12）

设计文档：`docs/design/2026-06-12-db-truth-driven-gate-and-diagnostic-reflector.md`（§7 决策全拍板）。本计划只实现设计 §5.3 的「DB 业务表投影」（路线图里的 PR4 首步）。

**已核实的关键事实（决定方案可行性）：**

1. `resources/harness/stages/target_intel.json` **已开** `coverage_complete.derive_from_evidence: true`（第 47 行）+ `expected_techniques` 含 6 类（DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT）。→ **本 PR 不改这个 JSON。**
2. technique id 已注册于 `resources/harness/technique_taxonomy.json`：`GOLISH-INTEL-ASN`/`-CT`/`-SUBDOMAIN`。
3. `coverage_complete`（`rule_engine.rs:311`）的 asset 维度 = `ctx.in_scope_assets`（注入则用它，否则用 deliverable 自报）。
4. `coverage_complete` 的 `derive_from_evidence` 投影（`rule_engine.rs:390-400`）**只读** `f.asset` / `f.technique` / `f.outcome`，**不读** `f.evidence_id`。`Found` 事实 → `Found` 格，`Empty` 事实 → `CheckedEmpty` 格。
5. `coverage_corroborated`（`rule_engine.rs:512-526`）**只遍历 `d.coverage` 里 agent 自报的 `Found` cell**，要求有对齐 claim/finding；它**不消费 `ctx.evidence_facts`**。→ 业务表投影补的格（不在 `d.coverage` 自报集里）**天然绕过** corroborated，不会被它误 BLOCK。**这是方案安全的核心保证。**
6. `evidence_id` 仅在 `synthesize_from_evidence`（missing-deliverable 投影兜底，`execute.rs:2194`）路径被收进 `evidence_refs` + claims；`enforce_evidence_existence`（`execute.rs:1289`）随后校验这些 id 在 `audit_log` 真实存在。→ **唯一张力点**（见 §1 决策 D2）。
7. `fetch_evidence_facts_for_gate`（`execute.rs:1240`）调 `repo.evidence_facts_for_session(sid)` → 转 `EvidenceFact` → 注入。生产调用点有两处：`execute.rs:265` 与 `execute.rs:426`。
8. `harness_org_id`（`orchestrator.rs:54`）已是 orchestrator 字段；`fetch_in_scope_assets_for_gate`（`execute.rs:1184`）已用它查 `repo.in_scope_assets(org_id)`。
9. golish-db 范式：SQL 抽成 `build_*_sql()` 纯函数 + 单测断言 SQL 字符串（见 `targets.rs:148`/`:579`）；DB 实跑留集成/活体。
10. 业务表：`organizations.asns`/`.certificates` 为 `JSONB NOT NULL DEFAULT '[]'`；`target_assets.asset_type TEXT DEFAULT 'subdomain'`，有 `target_id` FK 但**无** `organization_id`（要 JOIN `targets`）。

---

## 1. 范围与关键设计决策

### In scope（本计划做）
- ASN / CT / SUBDOMAIN 三类的 DB 业务表投影（已有专列/专表）。
- `golish-db` 只读查询 + trait 方法 + app 实现 + hook 合并 + 哨兵红线守护 + TDD。

### Out of scope（本计划**不**做，明确边界）
- **DNS**：需新建 `dns_records` 专表（schema 改动，AGENTS.md §2.7 须先与用户确认）。本轮不碰。
- **WHOIS / OSINT**：埋在 `organizations.intel` JSONB catch-all，无稳定可查路径；留待设计 §5.2。
- **诊断式 reflector**（设计 §5.4 / 路线图 PR2）：独立 PR，不在本计划。
- **命令路径 / provider 路径 technique 标注**（设计 §5.1 / PR1）：独立，价值有限（设计 §5.3 核实修正已说明），不在本计划。

### 关键设计决策

**D1 · asset 维度对齐。** 业务表 fact 的 `asset` 字段必须与 gate 实际遍历的 asset 集（`ctx.in_scope_assets`）一致才能匹配投影。故：DB 查询接收 `in_scope_assets: &[String]`，**只为其中的 asset 产 fact**。ASN/CT 是 org 级存量 → 对 in-scope 集里**每个** asset 都产（org 有 ASN 数据 = 该 org 所有资产的 ASN 维度算覆盖）。SUBDOMAIN 是 target 级 → 仅对「真有子域资产行」的 target value 产。`in_scope_assets` 为空（GUI/chat 路径 `org_id=None` 且无注入）→ 不投影，退回纯账本行为（零回归）。

**D2 · 哨兵 `evidence_id = 0` + synthesize 过滤（守 fabricated 红线 §4.1）。** 业务表 fact 不指向某条 `audit_log` evidence 行，用哨兵 `evidence_id = 0` 标记「非账本来源」。
- 正常路径（agent 交了 deliverable）：业务表 fact 只经 `coverage_complete.derive_from_evidence` 补格——该路径**不看 evidence_id**，哨兵无影响。✅
- 兜底路径（agent 没交 deliverable，`synthesize_from_evidence`）：改为只收 `evidence_id > 0` 的 fact 进 `evidence_refs` / claims，哨兵被过滤 → 业务表 fact 绝不进 deliverable.evidence_refs → `enforce_evidence_existence` 不会把它当伪造 id 拦。✅

**D3 · 只产 `Found`（红线 §4.2 / I8）。** DB 查询只查「有数据」→ 一律 `EvidenceOutcome::Found`。**绝不**产 `Empty`：DB 无数据 = 「没测」或「测了空」无法区分，不在此推断 checked_empty。缺数据的 (asset × technique) → 无 fact → `coverage_complete` 照旧报 gap → BLOCK（这是 I8 的正确语义）。

**D4 · 本 PR 单独跑活体未必能让 `target_intel` PASS（必须让用户知道）。** 今天 live run BLOCK 在 `{DNS,WHOIS,ASN,CT,OSINT}` 五类 never attempted。本 PR 能补 ASN/CT（**前提是 `organizations.asns/.certificates` 真有数据**）+ SUBDOMAIN。DNS/WHOIS/OSINT 仍会 BLOCK（缺专表/埋 catch-all）。且业务表当前是否真有 ASN/CT 数据取决于 provider 落库路径（设计 §5.1(b) PR1）——若业务表为空，投影不出格仍 BLOCK，**这是正确行为**。本 PR 的成功标准是「DB 里真有的格被正确投影为 Found，DB 里没有的格继续正确 BLOCK」，**不是**「target_intel 一定 PASS」。

---

## 2. 文件结构（创建 / 修改）

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-db/src/repo/coverage_truth.rs` | **新建** | 业务表真值只读查询：`build_*_sql()` 纯函数 + `assemble_truth_facts()` 纯组装 + `coverage_truth_facts()` IO 入口 + 单测 |
| `backend/crates/golish-db/src/repo/mod.rs` | 改 | `pub mod coverage_truth;` |
| `backend/crates/golish-agent-kit/src/db_traits/repo.rs` | 改 | `DbRepoProvider` 加默认空方法 `db_truth_facts` |
| `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs` | 改 | `impl DbRepoProvider` 转发 `db_truth_facts` → `db_truth_facts_impl` |
| `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs` | 改 | `impl GolishDbRepoProvider` 加 `db_truth_facts_impl`（调 golish-db） |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 改 | `db_truth_facts_to_evidence` 纯函数 + `fetch_evidence_facts_for_gate` 合并 + 两处调用点 + `synthesize_from_evidence` 哨兵过滤 + 测试 |
| `resources/harness/stages/target_intel.json` | **零改动** | `derive_from_evidence` 已开（设计 §5.3 优雅点） |

---

## 任务 1 · golish-db：`coverage_truth.rs` 的 SQL 构造纯函数（TDD）

**文件：** 新建 `backend/crates/golish-db/src/repo/coverage_truth.rs`；改 `backend/crates/golish-db/src/repo/mod.rs`

### 步骤 1.1 — 先写失败的 SQL 断言测试

在 `coverage_truth.rs` 写文件骨架 + 测试（此时 `build_*_sql` 还不存在 → 编译失败 = 红）：

```rust
//! Coverage gate 的 DB 业务表真值查询（设计 2026-06-12 §5.3）。
//!
//! 只读地回答「某 org / in-scope 资产，在业务表里某类被动情报技术是否真有数据」，
//! 供 harness 外层 hook 转成 `Found` EvidenceFact 注入 coverage gate。
//!
//! 红线（设计 §4）：
//! - 只产「有数据」(Found 语义)；DB 无数据**绝不**推断 checked_empty (I8)。
//! - 只读 SELECT，不写库；gate 纯函数不变（查询在 golish-db，结果经 hook 注入）。

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

/// 注册于 `technique_taxonomy.json` 的被动情报 technique id（本 PR 灰度三类）。
pub const TECH_ASN: &str = "GOLISH-INTEL-ASN";
pub const TECH_CT: &str = "GOLISH-INTEL-CT";
pub const TECH_SUBDOMAIN: &str = "GOLISH-INTEL-SUBDOMAIN";

/// org 级情报存量：`asns` / `certificates` 专列是否非空（JSONB 数组长度 > 0）。
fn build_org_intel_presence_sql() -> String {
    "SELECT (jsonb_array_length(asns) > 0) AS has_asn, \
            (jsonb_array_length(certificates) > 0) AS has_ct \
       FROM organizations WHERE id = $1"
        .to_string()
}

/// 该 org 下 scope='in' 的 target 中，哪些 `value` 真有 `asset_type='subdomain'` 子资产行。
/// `$1 IS NULL` 时不按 org 过滤（退回全局 scope='in'，与 `list_in_scope_values` 同款）。
fn build_subdomain_target_values_sql() -> String {
    "SELECT DISTINCT t.value FROM targets t \
       JOIN target_assets ta ON ta.target_id = t.id \
       WHERE t.scope::text = 'in' \
         AND ($1 IS NULL OR t.organization_id = $1) \
         AND ta.asset_type = 'subdomain'"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_intel_presence_sql_reads_asn_and_cert_columns() {
        let sql = build_org_intel_presence_sql();
        assert!(sql.contains("jsonb_array_length(asns) > 0"));
        assert!(sql.contains("jsonb_array_length(certificates) > 0"));
        assert!(sql.contains("FROM organizations WHERE id = $1"));
    }

    #[test]
    fn subdomain_sql_filters_scope_org_and_asset_type() {
        let sql = build_subdomain_target_values_sql();
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("ta.asset_type = 'subdomain'"));
        assert!(sql.contains("JOIN target_assets ta ON ta.target_id = t.id"));
    }
}
```

在 `repo/mod.rs` 按字母序加（`coverage_truth` 在 `audit` 后）：

```rust
pub mod coverage_truth;
```

### 步骤 1.2 — 看它失败（编译错：测试引用的私有 fn 已存在则应直接绿；若骨架未贴全则红）

```bash
cd backend && cargo nextest run -p golish-db coverage_truth 2>&1 | tail -20
```

预期：函数贴全后这两个 SQL 测试 **2 passed**（SQL 是确定性字符串，先写测试是为了钉死列名/谓词，防后续改 SQL 漂移）。若红，对照步骤 1.1 补齐。

### 步骤 1.3 — Commit（待用户授权）

```bash
git add backend/crates/golish-db/src/repo/coverage_truth.rs backend/crates/golish-db/src/repo/mod.rs
git commit -m "feat(golish-db): coverage_truth SQL builders for DB business-table projection"
```

---

## 任务 2 · golish-db：纯组装 + IO 入口（TDD）

**文件：** `backend/crates/golish-db/src/repo/coverage_truth.rs`

### 步骤 2.1 — 先写失败的组装测试

在 `mod tests` 里追加（此时 `assemble_truth_facts` 不存在 → 红）：

```rust
    use std::collections::HashSet;

    fn subs(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn assemble_empty_in_scope_yields_no_facts() {
        let out = assemble_truth_facts(&[], true, true, &subs(&["a.com"]));
        assert!(out.is_empty(), "no in-scope asset → no fact (D1 维度对齐)");
    }

    #[test]
    fn assemble_org_intel_applies_to_every_in_scope_asset() {
        let assets = vec!["moresec.cn".to_string(), "sub.moresec.cn".to_string()];
        let out = assemble_truth_facts(&assets, true, false, &HashSet::new());
        // has_asn=true → 每个 in-scope asset 产 ASN；has_ct=false → 无 CT。
        assert_eq!(
            out,
            vec![
                ("moresec.cn".to_string(), TECH_ASN),
                ("sub.moresec.cn".to_string(), TECH_ASN),
            ]
        );
    }

    #[test]
    fn assemble_subdomain_only_for_targets_with_children() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let out = assemble_truth_facts(&assets, false, false, &subs(&["moresec.cn"]));
        // 只有 moresec.cn 有子域资产行 → 只它产 SUBDOMAIN。
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_SUBDOMAIN)]);
    }

    #[test]
    fn assemble_never_emits_empty_outcome_only_found_pairs() {
        // assemble 只返回 (asset, technique) 对 = Found 语义；无 outcome 字段可为 Empty。
        let out = assemble_truth_facts(&["a.com".to_string()], true, true, &subs(&["a.com"]));
        assert_eq!(
            out,
            vec![
                ("a.com".to_string(), TECH_ASN),
                ("a.com".to_string(), TECH_CT),
                ("a.com".to_string(), TECH_SUBDOMAIN),
            ]
        );
    }
```

### 步骤 2.2 — 看它失败

```bash
cd backend && cargo nextest run -p golish-db coverage_truth 2>&1 | tail -20
```

预期：4 个新测试编译失败（`assemble_truth_facts` 未定义）。

### 步骤 2.3 — 最少实现让测试通过

在 `coverage_truth.rs`（`mod tests` 之前）加组装纯函数 + IO 入口：

```rust
/// 纯组装（与 IO 解耦，便于单测）：对每个 in-scope asset，按业务表存量产 `(asset, technique)`。
/// 顺序确定（ASN→CT→SUBDOMAIN，外层按 in_scope_assets 顺序），便于断言。
pub(crate) fn assemble_truth_facts(
    in_scope_assets: &[String],
    has_asn: bool,
    has_ct: bool,
    subdomain_values: &HashSet<String>,
) -> Vec<(String, &'static str)> {
    let mut facts = Vec::new();
    for asset in in_scope_assets {
        if has_asn {
            facts.push((asset.clone(), TECH_ASN));
        }
        if has_ct {
            facts.push((asset.clone(), TECH_CT));
        }
        if subdomain_values.contains(asset) {
            facts.push((asset.clone(), TECH_SUBDOMAIN));
        }
    }
    facts
}

/// DB 业务表真值事实 `(asset, technique)`：业务表里 `asset` 上 `technique` 真有数据。
///
/// `in_scope_assets` 是 coverage gate 实际遍历的权威资产集（org 已隔离），保证与
/// `coverage_complete` 的 asset 维度对齐。`org_id=None` 时不查 org 级情报（ASN/CT 不
/// 投影），SUBDOMAIN 退回全局 scope='in'。空 in-scope → 直接返回空（D1）。
pub async fn coverage_truth_facts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    in_scope_assets: &[String],
) -> Result<Vec<(String, &'static str)>> {
    if in_scope_assets.is_empty() {
        return Ok(Vec::new());
    }
    let (has_asn, has_ct) = match org_id {
        Some(id) => sqlx::query_as::<_, (bool, bool)>(&build_org_intel_presence_sql())
            .bind(id)
            .fetch_optional(pool)
            .await?
            .unwrap_or((false, false)),
        None => (false, false),
    };
    let subdomain_values: HashSet<String> =
        sqlx::query_scalar::<_, String>(&build_subdomain_target_values_sql())
            .bind(org_id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    Ok(assemble_truth_facts(
        in_scope_assets,
        has_asn,
        has_ct,
        &subdomain_values,
    ))
}
```

### 步骤 2.4 — 看它通过 + clippy + fmt

```bash
cd backend && cargo nextest run -p golish-db coverage_truth 2>&1 | tail -20
cargo clippy -p golish-db --lib -- -D warnings 2>&1 | tail -5
cargo fmt -p golish-db -- --check
```

预期：6 passed（任务 1 的 2 + 本任务 4）；clippy 零告警；fmt 干净。`ReadLints` 改动文件 0 错误。

### 步骤 2.5 — Commit（待用户授权）

```bash
git add backend/crates/golish-db/src/repo/coverage_truth.rs
git commit -m "feat(golish-db): coverage_truth_facts queries business tables (ASN/CT/SUBDOMAIN, Found only)"
```

---

## 任务 3 · golish-agent-kit：`DbRepoProvider` trait 加 `db_truth_facts`（TDD）

**文件：** `backend/crates/golish-agent-kit/src/db_traits/repo.rs`

### 步骤 3.1 — 先确认 test double 默认行为不变（红：方法未定义）

在 `db_traits/repo.rs` 的 `#[cfg(test)]`（若无则不强求；该文件主要是 trait 定义，测试可放 execute.rs 任务 5）——本任务的「失败验证」用编译：调用方（任务 6）会引用 `repo.db_truth_facts(...)`，未定义则编译失败。先加方法即可。

### 步骤 3.2 — 加默认空方法

在 `evidence_facts_for_session`（`repo.rs:331`）附近加（紧随其后，保持「Evidence Ledger / coverage 投影」语义聚集）：

```rust
    /// 设计 2026-06-12 §5.3 · DB 业务表真值事实 `(asset, technique)`：业务表里
    /// `asset` 上 `technique` 真有结构化数据（`organizations.asns`/`.certificates`
    /// 专列非空、`target_assets(asset_type='subdomain')` 存在）。coverage gate 外层
    /// hook 把这些转成 `Found` EvidenceFact 合并注入，使 coverage 判定以 DB 真值为准。
    ///
    /// 只产「有数据」(Found 语义)；DB 无数据**绝不**推断 checked_empty (I8)。
    /// `in_scope_assets` 是 gate 的权威资产集（保证维度对齐）；空集 → 空结果。
    ///
    /// 默认空（test double 零改动）；app 层 `GolishDbRepoProvider` 覆写。
    async fn db_truth_facts(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
    ) -> anyhow::Result<Vec<(String, String)>> {
        let _ = (org_id, in_scope_assets);
        Ok(Vec::new())
    }
```

> 注：trait 返回 `Vec<(String, String)>`（technique 用 `String`，不依赖 golish-db 的 `&'static str` 常量，保持 crate 解耦）。

### 步骤 3.3 — 编译验证

```bash
cd backend && cargo check -p golish-agent-kit 2>&1 | tail -5
```

预期：exit 0（默认方法不破坏任何现有 impl）。

---

## 任务 4 · golish-agent-app：`GolishDbRepoProvider` 实现 `db_truth_facts`

**文件：** `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`

### 步骤 4.1 — `recon.rs` 加 `_impl`

在 `recon.rs` 的 `impl GolishDbRepoProvider { ... }` 块里加（紧邻 `in_scope_assets_impl`，技术相关）：

```rust
    /// 设计 2026-06-12 §5.3 · DB 业务表真值事实（转 String technique，trait 解耦）。
    pub(crate) async fn db_truth_facts_impl(
        &self,
        org_id: Option<uuid::Uuid>,
        in_scope_assets: &[String],
    ) -> anyhow::Result<Vec<(String, String)>> {
        let rows = golish_db::repo::coverage_truth::coverage_truth_facts(
            &self.pool,
            org_id,
            in_scope_assets,
        )
        .await?;
        Ok(rows.into_iter().map(|(a, t)| (a, t.to_string())).collect())
    }
```

> 执行前先 `Grep` 确认 `recon.rs` 里 `in_scope_assets_impl` 的确切位置与 `impl GolishDbRepoProvider` 块；若 `recon.rs` 无该 impl 块，则放到定义 `in_scope_assets_impl` 的同一文件的 impl 块内（搜 `fn in_scope_assets_impl`）。

### 步骤 4.2 — `mod.rs` 的 `impl DbRepoProvider` 转发

在 `db_bridge/mod.rs` 的 `impl DbRepoProvider for GolishDbRepoProvider` 块里，紧随 `in_scope_target_types`（`mod.rs:295`）加：

```rust
    async fn db_truth_facts(
        &self,
        org_id: Uuid,
        in_scope_assets: &[String],
    ) -> anyhow::Result<Vec<(String, String)>> {
        self.db_truth_facts_impl(Some(org_id), in_scope_assets).await
    }
```

> ⚠️ 类型一致性自查：trait 签名是 `org_id: Option<Uuid>`。这里**必须**与 trait 完全一致 → 写成 `org_id: Option<Uuid>` 并 `self.db_truth_facts_impl(org_id, in_scope_assets)`（不要包 `Some`）。改正为：

```rust
    async fn db_truth_facts(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
    ) -> anyhow::Result<Vec<(String, String)>> {
        self.db_truth_facts_impl(org_id, in_scope_assets).await
    }
```

### 步骤 4.3 — 编译验证

```bash
cd backend && cargo check -p golish-agent-app 2>&1 | tail -5
cargo clippy -p golish-agent-app --lib -- -D warnings 2>&1 | tail -5
```

预期：exit 0。`ReadLints` 改动文件 0 错误。

### 步骤 4.4 — Commit（待用户授权）

```bash
git add backend/crates/golish-agent-kit/src/db_traits/repo.rs \
        backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs \
        backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs
git commit -m "feat(harness): DbRepoProvider::db_truth_facts wired to golish-db coverage_truth"
```

---

## 任务 5 · execute.rs：`db_truth_facts_to_evidence` 纯函数（TDD · 哨兵 id + 只 Found）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

### 步骤 5.1 — 先写失败的纯函数测试

在 execute.rs 的测试区（紧跟现有 `synthesize_from_evidence 纯函数` 测试 mod，约 `execute.rs:3109` 区域）新增 mod：

```rust
    // ── db_truth_facts_to_evidence 纯函数（设计 2026-06-12 §5.3 / 约束 §4） ──
    mod db_truth_projection_tests {
        use super::super::*;
        use crate::harness::gate::rule_engine::EvidenceOutcome;

        #[test]
        fn maps_pairs_to_found_facts_with_sentinel_id() {
            let pairs = vec![
                ("moresec.cn".to_string(), "GOLISH-INTEL-ASN".to_string()),
                ("moresec.cn".to_string(), "GOLISH-INTEL-SUBDOMAIN".to_string()),
            ];
            let facts = db_truth_facts_to_evidence(pairs);
            assert_eq!(facts.len(), 2);
            for f in &facts {
                assert_eq!(f.outcome, EvidenceOutcome::Found, "DB 投影只产 Found (I8)");
                assert_eq!(f.evidence_id, 0, "业务表 fact 用哨兵 id=0 (D2)");
            }
            assert_eq!(facts[0].asset, "moresec.cn");
            assert_eq!(facts[0].technique, "GOLISH-INTEL-ASN");
        }

        #[test]
        fn empty_input_yields_empty() {
            assert!(db_truth_facts_to_evidence(vec![]).is_empty());
        }
    }
```

### 步骤 5.2 — 看它失败

```bash
cd backend && cargo nextest run -p golish-agent-kit db_truth_projection 2>&1 | tail -15
```

预期：编译失败（`db_truth_facts_to_evidence` 未定义）。

### 步骤 5.3 — 实现纯函数

在 execute.rs 紧邻 `synthesize_from_evidence`（`execute.rs:2194` 之前）加：

```rust
/// 设计 2026-06-12 §5.3 · 把 DB 业务表真值 `(asset, technique)` 转成 `Found`
/// EvidenceFact，供与账本 facts 合并注入 coverage gate。
///
/// 红线：
/// - outcome 恒 `Found` —— 业务表「有数据」即 Found；本函数永不产 `Empty`
///   （checked_empty 只能由账本「跑了→空」的真实 outcome 显式产生，I8）。
/// - `evidence_id` 用哨兵 `DB_TRUTH_EVIDENCE_ID`（0）标记「非账本来源」。
///   `coverage_complete` 投影只看 asset/technique/outcome（不看 id），哨兵无影响；
///   `synthesize_from_evidence` 用 `id > 0` 过滤哨兵，业务表 fact 绝不进
///   `evidence_refs` / claims，fabricated-evidence 校验天然不误伤（§4.1）。
const DB_TRUTH_EVIDENCE_ID: i64 = 0;

fn db_truth_facts_to_evidence(
    facts: Vec<(String, String)>,
) -> Vec<crate::harness::gate::rule_engine::EvidenceFact> {
    use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
    facts
        .into_iter()
        .map(|(asset, technique)| EvidenceFact {
            asset,
            technique,
            outcome: EvidenceOutcome::Found,
            evidence_id: DB_TRUTH_EVIDENCE_ID,
        })
        .collect()
}
```

### 步骤 5.4 — 看它通过

```bash
cd backend && cargo nextest run -p golish-agent-kit db_truth_projection 2>&1 | tail -15
```

预期：2 passed。

---

## 任务 6 · execute.rs：`fetch_evidence_facts_for_gate` 合并业务表 facts

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

### 步骤 6.1 — 改方法签名 + 合并逻辑

把 `fetch_evidence_facts_for_gate`（`execute.rs:1240`）改成接收 `in_scope_assets` 并合并业务表 facts。完整替换该方法体为：

```rust
    async fn fetch_evidence_facts_for_gate(
        &self,
        planned: &PlannedSubtask,
        in_scope_assets: Option<&[String]>,
    ) -> Option<Vec<crate::harness::gate::rule_engine::EvidenceFact>> {
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        planned.harness_stage.as_ref()?;
        let sid = self.chat_session_id.as_deref()?;

        // ① 账本派生（现有路径）：audit_log 三列齐全的行 → EvidenceFact。
        let mut facts: Vec<EvidenceFact> = match self.repo.evidence_facts_for_session(sid).await {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "evidence-facts lookup failed; coverage gate runs without ledger projection"
                );
                Vec::new()
            }
        };

        // ② DB 业务表真值派生（设计 2026-06-12 §5.3）：org 已隔离的 in-scope 资产集上，
        // 业务表真有数据的 (asset × technique) 作为 Found 合并（只产 Found，哨兵 id=0）。
        // in_scope_assets 缺失（GUI/chat 路径 org_id=None 且无注入）→ 跳过，退回纯账本
        // 投影（零回归）。
        if let Some(assets) = in_scope_assets {
            match self.repo.db_truth_facts(self.harness_org_id, assets).await {
                Ok(truth) if !truth.is_empty() => {
                    let n = truth.len();
                    facts.extend(db_truth_facts_to_evidence(truth));
                    tracing::info!(
                        target: "harness::hook",
                        db_truth_facts = n,
                        org_id = ?self.harness_org_id,
                        "merged DB business-table truth facts into coverage gate (Found only)"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        target: "harness::hook",
                        error = %e,
                        "db-truth-facts lookup failed; coverage gate runs without DB projection"
                    );
                }
            }
        }

        if facts.is_empty() {
            return None;
        }
        tracing::info!(
            target: "harness::hook",
            fact_count = facts.len(),
            "injecting merged ledger+DB evidence facts into coverage gate (projection)"
        );
        Some(facts)
    }
```

### 步骤 6.2 — 改两处生产调用点

**调用点 A（`execute.rs:265`）：** `in_scope_assets` 变量在 `:262` 已是 `Option<Vec<String>>`，且 `:270` 会 move 进 `apply_harness_gate_hook`。借用先于 move，把 `:265` 改为：

```rust
                    let evidence_facts = self
                        .fetch_evidence_facts_for_gate(planned, in_scope_assets.as_deref())
                        .await;
```

**调用点 B（`execute.rs:426`）：** 先 `Read` `execute.rs:415-435` 确认该处 `in_scope_assets` 变量名与是否在此调用前已查得。照调用点 A 同样传入 `in_scope_assets.as_deref()`（若该处尚未查 in_scope_assets，则在调用前补 `let in_scope_assets = self.fetch_in_scope_assets_for_gate(planned).await;` 并同样传给后续 `apply_harness_gate_hook`）。

### 步骤 6.3 — 编译验证

```bash
cd backend && cargo check -p golish-agent-kit 2>&1 | tail -10
```

预期：exit 0。若报「`fetch_evidence_facts_for_gate` 参数数量不符」，说明还有调用点未改 → 按报错位置补 `, in_scope_assets.as_deref()` 或 `, None`（测试桩用 `None`）。

---

## 任务 7 · execute.rs：`synthesize_from_evidence` 哨兵过滤（TDD · 守 fabricated 红线）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

### 步骤 7.1 — 先写失败的过滤测试

在现有 `synthesize_from_evidence 纯函数` 测试 mod（`execute.rs:3109` 区域）追加：

```rust
    #[test]
    fn synthesize_excludes_db_truth_sentinel_from_refs_and_claims() {
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        let facts = vec![
            // 真实账本 fact（id>0）→ 应产 claim + 进 refs。
            EvidenceFact {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-DNS".to_string(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 42,
            },
            // 业务表投影 fact（哨兵 id=0）→ 不产 claim、不进 refs（D2 / §4.1）。
            EvidenceFact {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-ASN".to_string(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 0,
            },
        ];
        let d = synthesize_from_evidence(StageKind::TargetIntel, &facts);
        // 只有真实账本 fact 产 claim。
        assert_eq!(d.claims.len(), 1, "哨兵 fact 不产 claim");
        assert_eq!(d.claims[0].technique.as_deref(), Some("GOLISH-INTEL-DNS"));
        // evidence_refs 只含真实 id 42，不含哨兵 0。
        let refs: Vec<i64> = d.evidence_refs.iter().map(|e| e.as_i64()).collect();
        assert_eq!(refs, vec![42], "evidence_refs 排除哨兵 0（防 fabricated 误判）");
    }
```

> 注：`EvidenceAuditId` 取值方法若不是 `as_i64()`，先 `Grep "impl EvidenceAuditId"` 确认（设计已知 `synthesize_from_evidence` 用 `EvidenceAuditId::new(...)` 构造）。断言按真实 getter 调整。

### 步骤 7.2 — 看它失败

```bash
cd backend && cargo nextest run -p golish-agent-kit synthesize_excludes_db_truth 2>&1 | tail -15
```

预期：失败（当前实现把哨兵 0 收进 refs，且为 Found fact 产 claim）。

### 步骤 7.3 — 改 `synthesize_from_evidence` 过滤哨兵

在 `synthesize_from_evidence`（`execute.rs:2201`）：claims 的 `.filter` 加 `&& f.evidence_id > 0`：

```rust
    let claims: Vec<crate::harness::StageClaim> = facts
        .iter()
        .filter(|f| f.outcome == EvidenceOutcome::Found && f.evidence_id > 0)
        .map(|f| crate::harness::StageClaim {
            // ... 不变 ...
        })
        .collect();
```

evidence_refs 收集（`execute.rs:2218`）加 `id > 0` 过滤：

```rust
    let mut ids: Vec<i64> = facts
        .iter()
        .map(|f| f.evidence_id)
        .filter(|id| *id > 0)
        .collect();
    ids.sort_unstable();
    ids.dedup();
```

并把该函数文档注释（`execute.rs:2186-2187`）的 "evidence_refs: ALL fact ids" 一句更新为：「只收真实账本 id（`> 0`）；业务表投影 fact 的哨兵 id（0）排除，故 `enforce_evidence_existence` 不误伤（设计 2026-06-12 §4.1）」。

### 步骤 7.4 — 看它通过 + 全量回归

```bash
cd backend && cargo nextest run -p golish-agent-kit synthesize_excludes_db_truth 2>&1 | tail -10
cargo nextest run -p golish-agent-kit 2>&1 | tail -8
cargo clippy -p golish-agent-kit --lib -- -D warnings 2>&1 | tail -5
cargo fmt -p golish-agent-kit -- --check
```

预期：新测试 pass；golish-agent-kit 全量绿（基线 580+，**0 failed**）；clippy 零告警；fmt 干净。`ReadLints` 改动文件 0 错误。

### 步骤 7.5 — Commit（待用户授权）

```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs
git commit -m "feat(harness): merge DB business-table truth facts into coverage gate (Found only, sentinel-guarded)"
```

---

## 任务 8 · hook 级集成测试（合并 facts → 投影补格 + corroborated 不误伤）

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`（测试区）

> 目的：用纯函数级 `eval_with_context` 钉死「业务表投影 fact（哨兵 id=0）能补 `coverage_complete` 的格，且不被 `coverage_corroborated` 误 BLOCK」——这是方案安全的核心保证（§0 事实 5）。无需 DB。

### 步骤 8.1 — 写测试

在 `rule_engine.rs` 测试区（紧随 `coverage_complete_derive_*` 测试群）加：

```rust
    #[test]
    fn db_truth_found_fact_fills_coverage_gap_without_corroboration_block() {
        use super::{CoverageStatus, EvidenceFact, EvidenceOutcome, GateContext};
        // 期望技术只一类 ASN；in-scope 资产只 moresec.cn；deliverable 自报 coverage 为空。
        let spec = stage_spec_with_expected(&["GOLISH-INTEL-ASN"]); // 见下方 helper 说明
        let rules = vec![
            coverage_complete_rule_derive_from_evidence(), // derive_from_evidence=true
            GateRule::CoverageCorroborated {
                on_fail: simple_on_fail("uncorroborated"),
            },
        ];
        let deliverable = empty_coverage_deliverable(); // claims/coverage/findings 全空
        let ctx = GateContext {
            in_scope_assets: Some(vec!["moresec.cn".to_string()]),
            expected_techniques: None,
            evidence_facts: Some(vec![EvidenceFact {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-ASN".to_string(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 0, // 业务表哨兵
            }]),
        };
        let outcomes = eval_with_context(&deliverable, &spec, &rules, &ctx);
        // coverage_complete：投影补格 → Pass（不报 ASN gap）。
        assert!(
            matches!(outcomes[0], GateCheckOutcome::Pass),
            "DB 投影 Found 补了 (moresec.cn × ASN) 格"
        );
        // coverage_corroborated：只查 d.coverage 自报 Found cell（此处为空）→ Pass（不误伤投影格）。
        assert!(
            matches!(outcomes[1], GateCheckOutcome::Pass),
            "投影格不在自报 coverage 集，corroborated 不检查它"
        );
    }
```

> **执行前**：`Grep` 该文件已有测试 helper（如 `coverage_complete_rule`（`:1035`）、构造空 deliverable 的方式、`GateContext` 构造）。复用现有 helper，不要新造重复构造器；上面的 `stage_spec_with_expected` / `coverage_complete_rule_derive_from_evidence` / `empty_coverage_deliverable` / `simple_on_fail` 若已有等价 helper 直接用其名，否则按现有 `coverage_complete_*` 测试（`:1252`/`:1640`）的构造方式内联。`derive_from_evidence=true` 的规则可参照 `:1640` 的派生测试构造。

### 步骤 8.2 — 看它通过

```bash
cd backend && cargo nextest run -p golish-agent-kit db_truth_found_fact_fills 2>&1 | tail -10
```

预期：pass。若 `coverage_corroborated` 误 BLOCK，说明对 §0 事实 5 的理解有误 → **停手复盘**，重读 `rule_engine.rs:512-526` 再调方案（不要硬改 corroborated）。

### 步骤 8.3 — Commit（待用户授权）

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs
git commit -m "test(harness): DB-truth projected cell fills coverage gap, bypasses corroboration"
```

---

## 任务 9 · 全量验证 + 收尾

### 步骤 9.1 — `just precommit` 全绿（commit / push 前必跑）

```bash
just precommit
```

预期：`✓ All checks passed!`（fmt + check-fe + test-fe + lint-rust + test-rust-all 全绿）。**未全绿不得宣称完成（AGENTS.md §3）。** 失败则按输出修，重跑。

### 步骤 9.2 — 活体对照（按 AGENTS.md §3：有验证命令实际跑过且证据被记录）

> 复用 progress 里 2026-06-12 的活体跑法（setsid 脱管，避免会话被杀连带清进程）：

```bash
cd backend && cargo build -p golish --bin golish   # 含本批改动
# 参照 agent-progress.md 2026-06-11 活体段的 setsid 双 fork 脱管方式启动：
./target/debug/golish --stage-run -p xiaomi -m mimo-v2.5-pro --profile assessment \
  --to target_intel --org 默安科技 --target moresec.cn --auto-approve --verbose
```

观察（日志 `/tmp/golish-stage-run.log` + transcript）：
- grep `merged DB business-table truth facts`：确认业务表 facts 被合并注入（出现 = 业务表真有 ASN/CT/SUBDOMAIN 数据）。
- 若 `organizations.asns/.certificates` 为空（业务表无数据）→ 不出现该行，ASN/CT 仍 BLOCK = **正确**（D4：本 PR 不负责让数据落库）。
- coverage_complete 的 BLOCK reason 里 ASN/CT/SUBDOMAIN 是否从「never attempted」列表消失（取决于 DB 实际存量）。

**把跑的命令、退出码、关键 grep 片段复制到 `agent-progress.md`「已记录证据」。** 活体无法逼出 DB 有数据时，至少用 §0 事实 + 单测证据 + 一条手工 `INSERT organizations.asns` 后重投影的对照（可选）说明投影链路通。

### 步骤 9.3 — 更新 `feature_list.json`

`db-truth-driven-gate-coverage-2026-06-12` 条目：
- `status`：若 precommit 全绿 + 活体证据齐 → `passing`；若活体只验到「链路通、DB 暂无数据」→ 留 `in_progress` 并在 `notes` 写清「投影链路 TDD 全绿，活体待业务表有 ASN/CT 数据（依赖 PR1 provider 落库）」。
- `plan`：填本文件路径 `docs/superpowers/plans/2026-06-12-db-truth-driven-gate-coverage.md`。
- `verification` / `evidence`：填实际跑过的命令与输出。

### 步骤 9.4 — 更新 `agent-progress.md`

新建一条 2026-06-12 会话记录：本轮目标、已完成（任务 1-9）、跑过的验证、已记录证据、提交记录、未提交半成品、下一步（PR1 provider 落库 / PR2 诊断式 reflector / DNS 专表需 §2.7 确认）。

### 步骤 9.5 — `clean-state-checklist.md` 逐项核对，确认无 scope 外改动。

---

## 自检（writing-plans skill）

**1. 规格覆盖度（对照设计 §5.3 + feature_list `remember`）：**
- 「golish-db 加只读查询」→ 任务 1+2。✅
- 「按 org/target 查某 technique 在业务表有无数据」→ `coverage_truth_facts`（ASN/CT 走 org，SUBDOMAIN 走 target JOIN）。✅
- 「organizations.asns/.certificates 非空→ASN/CT found」→ `build_org_intel_presence_sql` + `assemble_truth_facts`。✅
- 「target_assets asset_type=subdomain→SUBDOMAIN」→ `build_subdomain_target_values_sql`。✅
- 「execute.rs hook 调它、转 EvidenceFact 合并注入 ctx.evidence_facts」→ 任务 5+6。✅
- 「只产 Found」→ D3 + 任务 5（outcome 恒 Found）。✅
- 「checked_empty 不靠 DB 无数据推断（I8）」→ D3（无数据=无 fact，不产 Empty）。✅
- 「findings 永不投影」→ 未碰 findings 路径；`synthesize_from_evidence` findings 恒空（不变）。✅
- 「gate 保持纯函数（查库只在外层 hook）」→ 查询在 golish-db，hook 在 execute.rs，`coverage_complete`/`target_intel.json` 零改动。✅
- 「先只 target_intel 单阶段灰度」→ `derive_from_evidence` 只在 target_intel.json 开（其它阶段未开，自动不投影）。✅
- 「DNS 先不碰」→ Out of scope 明确。✅
- fabricated 红线（设计 §4.1）→ D2 哨兵 + 任务 7 过滤 + 任务 8 验证。✅

**2. 占位符扫描：** 无「TODO / 待定 / 类似任务 N / 为上述代码编写测试」。每个代码步骤有完整代码块；引用的类型（`EvidenceFact`/`EvidenceOutcome`/`GateContext`/`StageDeliverable`/`StageClaim`/`EvidenceAuditId`）均为已存在类型，已注明执行前需 `Grep` 确认的 getter/helper（`EvidenceAuditId::as_i64`、测试 helper）。

**3. 类型一致性：**
- golish-db `coverage_truth_facts` 返回 `Vec<(String, &'static str)>`；trait `db_truth_facts` 返回 `Vec<(String, String)>`；app `_impl` 用 `.map(|(a,t)| (a, t.to_string()))` 转换。✅一致。
- trait `db_truth_facts(org_id: Option<Uuid>, ...)` 与 `impl` 签名一致（任务 4.2 已修正 `Some` 误用为直接 `Option<Uuid>`）。✅
- `fetch_evidence_facts_for_gate` 新增参数 `in_scope_assets: Option<&[String]>`；调用点传 `in_scope_assets.as_deref()`（`Option<Vec<String>>` → `Option<&[String]>`），测试桩传 `None`。✅
- 哨兵常量 `DB_TRUTH_EVIDENCE_ID = 0`，`synthesize_from_evidence` 过滤用 `f.evidence_id > 0` / `*id > 0`，与哨兵 0 一致。✅

**风险复盘：** 唯一行为风险是「业务表 fact 进 synthesize 兜底路径」，已由 D2 哨兵 + 任务 7 双过滤 + 任务 7.1 测试封死；若任务 8 发现 corroborated 误伤（与 §0 事实 5 矛盾）→ 停手复盘，不硬改 gate。
