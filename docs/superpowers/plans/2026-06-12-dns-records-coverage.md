# DNS dns_records 专表 + 落库 + coverage 投影 实现计划（PR-B）

> **面向 AI 代理的工作者：** 配 `.cursor/skills/test-driven-development/SKILL.md`（先红后绿）。承接 PR-A（DB 真值驱动 coverage，已建立 `coverage_truth.rs` + `db_truth_facts` 通道）。

**目标：** 给 DNS 类被动情报建结构化落点 `dns_records` 专表，承接 `dig` 全量输出（ANSWER SECTION），并扩 `coverage_truth` 让「in-scope 资产有 DNS 记录」投影为 `GOLISH-INTEL-DNS` 的 `Found`。

**架构：** 沿用既有 tool-output 落库管线（`toolsconfig output.detect/db_action/patterns` → `parse_output` → `OutputStore::store_*` → golish-db repo）。新增 `dns_record_add` db_action。coverage 侧复用 PR-A 的 `coverage_truth_facts`（加 DNS 维度），gate 纯函数 + target_intel.json 零改动。

**技术栈：** Rust / sqlx + pg-embed / regex output parser / cargo nextest。

---

## 0. 现状（已核对源码 2026-06-12）

1. `dig` 输出**当前根本没落库**：`resources/toolsconfig/dig.json` 的 `output.db_action = "host_add"`，而 `output_store/mod.rs` 的 dispatch 只认 `target_add/target_update_recon/directory_entry_add/finding_add/organization_update` → `host_add` 命中 `other => "Unknown db_action"` 分支（印证设计 §3.2「DNS 仅 targets.real_ip，无表」）。
2. 落库管线：`maybe_detect_and_store_via`（`output_store/mod.rs:148`）→ `detect_tool`（按 toolsconfig `output.detect` 正则）→ `parse_output`（`output_parser.rs`，text 模式按 `patterns[].regex` + `fields` 抽 `ParsedRecord{record_type, fields: HashMap}`）→ 按 `db_action` 分发到 `OutputStore::store_*`。
3. `OutputStore` trait（`output_store/store_trait.rs`）+ `PgPentestStore`（`pg_adapter.rs`，转发到 `pub(crate)` freestanding `store_*`）+ 测试用 `MockOutputStore`（dev-dependency，**加 trait 方法必须同步实现**）。
4. `find_or_create_target(pool, host_value, project_path) -> Uuid`（`targets.rs`）已存在，可复用。
5. PR-A 已建 `golish-db/src/repo/coverage_truth.rs`（`coverage_truth_facts` + `assemble_truth_facts` + `TECH_ASN/CT/SUBDOMAIN`）。
6. 最新 migration 时间戳 `20260611000002`；新文件用 `20260612000001`。

---

## 1. 范围与关键决策

### In scope
DNS 端到端：`dns_records` 表 + repo + `dns_record_add` 落库 + `dig.json` output 规则 + `coverage_truth` DNS 投影 + TDD。

### Out of scope
WHOIS / OSINT（设计 §5.2 留 `organizations` JSONB；非本 PR）。`dnsx` 落库（dig 跑通后同模式可加，本 PR 先 dig）。

### 关键决策
- **D-B1 · 只解析全量 banner 的 ANSWER SECTION，不碰 `+short`。** `dig x A +short` 只输出裸值（如 `1.2.3.4`），缺 `record_type`/`name`，无法可靠结构化 → 不落（歧义即不映射，红线）。`dig` 默认 / `+noall +answer` 输出含 `name TTL IN TYPE value` 全字段行 → 落。coverage 只需「有 DNS 记录」，落不下的 `+short` 场景 DNS 仍 BLOCK（正确，鼓励 agent 用全量 dig）。
- **D-B2 · `dns_records` owner = recon**（DNS 是 recon 资产数据）。写它的 `output_store`（golish-pentest）+ 读它的 `coverage_truth`（golish-db 内部）均不在 `check_repo_ownership.py` 的 command-层 SOURCE_ROOTS，不触发跨服务检查；只需在 `REPO_OWNER` 注册 `dns_records: recon`（满足 guard 行 314 declared 检查）。
- **D-B3 · 维度对齐。** `store_dns_record` 用 `find_or_create_target(name)` 把记录挂到 target；`coverage_truth` 的 DNS 投影对每个 in-scope asset 查它（作为 target）有无 `dns_records` 行 → 与 PR-A SUBDOMAIN 同款（JOIN targets on organization_id）。
- **D-B4 · I10 向后兼容。** 新表 `CREATE TABLE IF NOT EXISTS`（可重放）；纯新增、不改既有表、灰度开关无需（DNS 投影只在 `coverage_truth` 内对有数据的 asset 产 Found）。
- **红线（同 PR-A）：** 只产 Found；DB 无 DNS 记录 ≠ checked_empty（I8）；findings 永不投影；gate 纯函数（查库在 coverage_truth/hook）。

---

## 2. 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-db/migrations/20260612000001_dns_records.sql` | 新建 | `dns_records` 表（target_id FK / record_type / name / value / source / 唯一约束） |
| `backend/crates/golish-db/src/repo/dns_records.rs` | 新建 | `upsert` 写入 + `has_records_for_targets`（按 org 查哪些 in-scope target value 有 DNS 记录）+ SQL builders + 单测 |
| `backend/crates/golish-db/src/repo/mod.rs` | 改 | `pub mod dns_records;` |
| `scripts/check_repo_ownership.py` | 改 | `REPO_OWNER` 加 `"dns_records": "recon"` |
| `backend/crates/golish-pentest/src/output_store/dns_records.rs` | 新建 | `store_dns_record` freestanding（find_or_create_target + repo upsert） |
| `backend/crates/golish-pentest/src/output_store/store_trait.rs` | 改 | `OutputStore` trait 加 `store_dns_record` |
| `backend/crates/golish-pentest/src/output_store/pg_adapter.rs` | 改 | `PgPentestStore` 实现转发 |
| `backend/crates/golish-pentest/src/output_store/mod.rs` | 改 | `mod dns_records;` + `pub(crate) use` + dispatch 加 `"dns_record_add"` |
| `backend/crates/golish-pentest/src/output_store/<mock 所在>` | 改 | `MockOutputStore` 实现新 trait 方法（搜 `MockOutputStore`） |
| `resources/toolsconfig/dig.json` | 改 | `output.db_action: host_add → dns_record_add` + 通用 ANSWER SECTION pattern |
| `backend/crates/golish-db/src/repo/coverage_truth.rs` | 改 | 加 `TECH_DNS` + `dns_record` 查询 + `assemble_truth_facts` 加 DNS 维度 |

---

## 任务 1 · migration（dns_records 表）

**文件：** 新建 `backend/crates/golish-db/migrations/20260612000001_dns_records.sql`

```sql
-- DNS records discovered by dig/dnsx (design 2026-06-12 §5.2 / §7 D-schema).
-- New table, additive, replayable (IF NOT EXISTS) — I10 backward-compatible.
-- Backs the coverage gate's GOLISH-INTEL-DNS truth projection (coverage_truth).
CREATE TABLE IF NOT EXISTS dns_records (
    id           BIGSERIAL PRIMARY KEY,
    target_id    UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path TEXT NOT NULL DEFAULT '',
    record_type  TEXT NOT NULL,          -- A / AAAA / NS / MX / TXT / CNAME / SOA / PTR
    name         TEXT NOT NULL,          -- queried name (hostname)
    value        TEXT NOT NULL,          -- record value (ip / target / text)
    source       TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (target_id, record_type, name, value)
);

CREATE INDEX IF NOT EXISTS idx_dns_records_target ON dns_records(target_id);
CREATE INDEX IF NOT EXISTS idx_dns_records_type   ON dns_records(record_type);
```

**验证：** `python3 -c "import re,sys; sys.exit(0)"`（语法）；真正校验在任务 2 编译期（sqlx 不离线校验本仓库，DB 实跑在活体）。

---

## 任务 2 · golish-db repo/dns_records.rs（TDD · SQL builders）

### 步骤 2.1 — 先写骨架（`todo!()`）+ SQL 断言测试 → 红

```rust
//! `dns_records` repository (design 2026-06-12 §5.2). Write-side upsert for
//! dig/dnsx output; read-side presence query backing coverage_truth's DNS
//! projection. Owner: recon (DNS = recon asset data).

use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

fn build_dns_upsert_sql() -> String {
    todo!()
}

/// 哪些 in-scope target `value` 真有 DNS 记录（org 隔离，与 coverage_truth 的
/// subdomain 查询同款）。`org_id=None` → 全局 scope='in'。
fn build_dns_present_target_values_sql() -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_sql_targets_dns_records_with_conflict_noop() {
        let sql = build_dns_upsert_sql();
        assert!(sql.contains("INSERT INTO dns_records"));
        assert!(sql.contains("target_id, project_path, record_type, name, value, source"));
        assert!(sql.contains("ON CONFLICT") && sql.contains("DO NOTHING"));
    }

    #[test]
    fn present_sql_filters_scope_and_org_and_joins_targets() {
        let sql = build_dns_present_target_values_sql();
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
    }
}
```

`repo/mod.rs` 加 `pub mod dns_records;`（按字母序，`directory_entries` 后、`endpoint_tests` 前）。

跑红：`cd backend && cargo nextest run -p golish-db dns_records`（todo panic）。

### 步骤 2.2 — 实现 → 绿

```rust
fn build_dns_upsert_sql() -> String {
    "INSERT INTO dns_records \
       (target_id, project_path, record_type, name, value, source) \
       VALUES ($1, $2, $3, $4, $5, $6) \
       ON CONFLICT (target_id, record_type, name, value) DO NOTHING"
        .to_string()
}

fn build_dns_present_target_values_sql() -> String {
    "SELECT DISTINCT t.value FROM targets t \
       JOIN dns_records dr ON dr.target_id = t.id \
       WHERE t.scope::text = 'in' \
         AND ($1 IS NULL OR t.organization_id = $1)"
        .to_string()
}

/// 写入一条 DNS 记录（幂等）。
pub async fn upsert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: &str,
    record_type: &str,
    name: &str,
    value: &str,
    source: &str,
) -> Result<()> {
    sqlx::query(&build_dns_upsert_sql())
        .bind(target_id)
        .bind(project_path)
        .bind(record_type)
        .bind(name)
        .bind(value)
        .bind(source)
        .execute(pool)
        .await?;
    Ok(())
}

/// in-scope target value 中真有 DNS 记录的集合（org 隔离）。
pub async fn present_target_values(
    pool: &PgPool,
    org_id: Option<Uuid>,
) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_dns_present_target_values_sql())
        .bind(org_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}
```

跑绿 + `cargo clippy -p golish-db --lib -- -D warnings` + `cargo fmt -p golish-db`。

### 步骤 2.3 — guard：`REPO_OWNER` 加 `"dns_records": "recon"`（在 `"directory_entries": "recon",` 附近）。跑 `python3 scripts/check_repo_ownership.py`（dns_records 不应出现在违规里——无 command-层调用）。

---

## 任务 3 · output_store：dns_record_add 落库（TDD via MockOutputStore）

### 步骤 3.1 — `output_store/dns_records.rs`（新）freestanding writer

```rust
//! DNS-record persistence: dig/dnsx ANSWER-SECTION rows → `dns_records`
//! (design 2026-06-12 §5.2). Mirrors `targets::store_target_*` shape.

use std::collections::HashMap;

use sqlx::PgPool;

use super::targets::find_or_create_target;
use crate::error::{PentestError, PentestResult};

pub(crate) async fn store_dns_record(
    pool: &PgPool,
    fields: &HashMap<String, String>,
    project_path: Option<&str>,
    tool_name: &str,
) -> PentestResult<()> {
    let name = fields
        .get("name")
        .ok_or_else(|| PentestError::OutputStore("No name field for dns_record".into()))?;
    let record_type = fields
        .get("record_type")
        .ok_or_else(|| PentestError::OutputStore("No record_type field for dns_record".into()))?;
    let value = fields
        .get("value")
        .ok_or_else(|| PentestError::OutputStore("No value field for dns_record".into()))?;
    let target_id = find_or_create_target(pool, name, project_path).await?;
    golish_db::repo::dns_records::upsert(
        pool,
        target_id,
        project_path.unwrap_or_default(),
        record_type,
        name,
        value,
        tool_name,
    )
    .await
    .map_err(|e| PentestError::OutputStore(format!("dns_records upsert failed: {e}")))?;
    Ok(())
}
```

> 执行前确认 golish-pentest 的 `Cargo.toml` 依赖 `golish-db`（targets.rs 等已用 `golish_db::repo`？——实测：`output_store` 现有 freestanding 用 `sqlx::PgPool` 直接写，未必依赖 golish-db）。**若 golish-pentest 不依赖 golish-db**（很可能，避免环依赖），则 `dns_records` 的写入 SQL 直接内联在本文件（用 `sqlx::query`，与 `store_target_add` 同款），不调 `golish_db::repo`。先 `Grep "golish_db" backend/crates/golish-pentest/` 判定；无依赖则内联 SQL：

```rust
    // 无 golish-db 依赖时内联（与 store_target_add 同款 raw sqlx）：
    sqlx::query(
        "INSERT INTO dns_records \
           (target_id, project_path, record_type, name, value, source) \
           VALUES ($1, $2, $3, $4, $5, $6) \
           ON CONFLICT (target_id, record_type, name, value) DO NOTHING",
    )
    .bind(target_id)
    .bind(project_path.unwrap_or_default())
    .bind(record_type)
    .bind(name)
    .bind(value)
    .bind(tool_name)
    .execute(pool)
    .await?;
    Ok(())
```

### 步骤 3.2 — trait + adapter + mock + dispatch
- `store_trait.rs`：`OutputStore` 加 `async fn store_dns_record(&self, fields, project_path, tool_name) -> PentestResult<()>;`
- `pg_adapter.rs`：`PgPentestStore` 实现转发 + `super::` use 加 `store_dns_record`
- `mod.rs`：`mod dns_records;` + `pub(crate) use dns_records::store_dns_record;` + dispatch 加：
  ```rust
  "dns_record_add" => store.store_dns_record(&record.fields, project_path, tool_name).await,
  ```
- `MockOutputStore`（搜 `MockOutputStore` 定位）：加 `store_dns_record` 实现（记录调用，同其它 mock 方法模式）。

### 步骤 3.3 — 编译 + golish-pentest 测试无回归
`cargo check -p golish-pentest` / `cargo nextest run -p golish-pentest` / `clippy -D warnings` / `fmt`。

---

## 任务 4 · dig.json output 规则（dns_record_add + ANSWER SECTION pattern）

把 `resources/toolsconfig/dig.json` 的 `output` 改为：

```json
    "output": {
      "format": "text",
      "produces": ["dns_record"],
      "detect": "ANSWER SECTION|IN\\s+(A|AAAA|NS|MX|TXT|CNAME|SOA|PTR)\\s",
      "db_action": "dns_record_add",
      "patterns": [
        {
          "type": "dns_record",
          "regex": "^([\\w.-]+?)\\.?\\s+\\d+\\s+IN\\s+(A|AAAA|NS|MX|TXT|CNAME|SOA|PTR)\\s+(.+)$",
          "fields": { "1": "name", "2": "record_type", "3": "value" }
        }
      ]
    }
```

**验证：** `python3 -m json.tool resources/toolsconfig/dig.json`（合法 JSON）。pattern 正确性在任务 5 用 `output_parser` 单测钉死（不靠肉眼）。

---

## 任务 5 · output_parser dig 解析单测（TDD · pattern 正确性）

在 `output_parser.rs` 的 `mod tests` 加 dig 全量 banner 解析测试（先写 → 红前先确认 pattern；本质是钉死 dig.json 的 regex 对真实 dig 输出工作）：

```rust
    fn dig_config() -> OutputConfig {
        OutputConfig {
            format: "text".to_string(),
            produces: vec!["dns_record".to_string()],
            patterns: vec![OutputPattern {
                data_type: "dns_record".to_string(),
                regex: r"^([\w.-]+?)\.?\s+\d+\s+IN\s+(A|AAAA|NS|MX|TXT|CNAME|SOA|PTR)\s+(.+)$"
                    .to_string(),
                fields: HashMap::from([
                    ("1".to_string(), "name".to_string()),
                    ("2".to_string(), "record_type".to_string()),
                    ("3".to_string(), "value".to_string()),
                ]),
            }],
            fields: HashMap::new(),
            detect: Some(r"ANSWER SECTION|IN\s+(A|AAAA|NS|MX|TXT|CNAME|SOA)\s".to_string()),
            db_action: Some("dns_record_add".to_string()),
            transform: None,
        }
    }

    #[test]
    fn test_dig_answer_section_parse() {
        let output = "\n;; ANSWER SECTION:\nmoresec.cn.\t\t600\tIN\tA\t1.2.3.4\nmoresec.cn.\t\t600\tIN\tMX\t10 mail.moresec.cn.\n";
        let result = parse_output(output, &dig_config());
        assert_eq!(result.db_action, Some("dns_record_add".to_string()));
        assert_eq!(result.records.len(), 2);
        assert_eq!(result.records[0].fields.get("name").unwrap(), "moresec.cn");
        assert_eq!(result.records[0].fields.get("record_type").unwrap(), "A");
        assert_eq!(result.records[0].fields.get("value").unwrap(), "1.2.3.4");
        assert_eq!(result.records[1].fields.get("record_type").unwrap(), "MX");
    }
```

> 若 regex 因 `extract_capture_fields` 的 `"1"->"name"` legacy 反转映射或 `.?` 贪婪/尾点处理不如预期 → 调 regex 直到测试绿（这正是「不靠肉眼、用测试钉死 pattern」的价值）。`name` 尾点 `moresec.cn.` 的 `\.?` 吸收 + `extract_capture_fields` `.trim()`，断言用无尾点 `moresec.cn`。

跑：`cargo nextest run -p golish-pentest test_dig_answer_section`（先红→调 regex→绿）。**保持 dig.json 的 regex 与测试里的字符串逐字一致。**

---

## 任务 6 · coverage_truth DNS 投影扩展（TDD）

### 步骤 6.1 — 测试先行（assemble 加 DNS 维度）

在 `coverage_truth.rs` 的 `mod tests` 加：

```rust
    #[test]
    fn assemble_dns_only_for_targets_with_records() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let out = assemble_truth_facts(
            &assets, false, false, &HashSet::new(), &subs(&["moresec.cn"]),
        );
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_DNS)]);
    }
```

并把现有 4 个 `assemble_*` 测试的调用补上新参数 `&HashSet::new()`（DNS 维度，空）。

### 步骤 6.2 — 实现：`assemble_truth_facts` 加 `dns_values` 参数 + `TECH_DNS`

```rust
pub const TECH_DNS: &str = "GOLISH-INTEL-DNS";

pub(crate) fn assemble_truth_facts(
    in_scope_assets: &[String],
    has_asn: bool,
    has_ct: bool,
    subdomain_values: &HashSet<String>,
    dns_values: &HashSet<String>,
) -> Vec<(String, &'static str)> {
    let mut facts = Vec::new();
    for asset in in_scope_assets {
        if has_asn { facts.push((asset.clone(), TECH_ASN)); }
        if has_ct { facts.push((asset.clone(), TECH_CT)); }
        if subdomain_values.contains(asset) { facts.push((asset.clone(), TECH_SUBDOMAIN)); }
        if dns_values.contains(asset) { facts.push((asset.clone(), TECH_DNS)); }
    }
    facts
}
```

`coverage_truth_facts` 里加 DNS 查询并传入：

```rust
    let dns_values: HashSet<String> = sqlx::query_scalar::<_, String>(
        &build_dns_present_target_values_sql(),
    )
    .bind(org_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    Ok(assemble_truth_facts(
        in_scope_assets, has_asn, has_ct, &subdomain_values, &dns_values,
    ))
```

加 `build_dns_present_target_values_sql()`（同任务 2 的查询，但在 coverage_truth.rs 内联——或复用 `crate::repo::dns_records` 的 `present_target_values`；优先复用：`crate::repo::dns_records::present_target_values(pool, org_id)`，避免 SQL 重复）。**决策：复用 repo::dns_records::present_target_values**（DRY）。

### 步骤 6.3 — 跑绿 + clippy + fmt（golish-db）。

---

## 任务 7 · 全 crate 验证 + commit（待最后统一 precommit）

```bash
cd backend && cargo nextest run -p golish-db -p golish-pentest 2>&1 | tail
cargo clippy -p golish-db -p golish-pentest --lib -- -D warnings
cargo fmt -p golish-db -p golish-pentest -- --check
cd .. && python3 scripts/check_repo_ownership.py   # dns_records 注册后无新违规
```

commit（与其它 PR 一起在最后统一，按文件精确 add）。

---

## 自检
- DNS 落点（设计 §5.2）→ 任务 1-4。✅
- coverage DNS 投影（设计 §5.3）→ 任务 6。✅
- 只产 Found / I8 → DNS 无记录=无 fact=BLOCK，不产 Empty。✅
- gate 纯函数 → 查库在 coverage_truth/output_store，gate 不变。✅
- 占位符：`MockOutputStore` 位置、golish-pentest 是否依赖 golish-db 标注「执行前 Grep 确认」（真未知点，非偷懒）。
- 类型一致：`assemble_truth_facts` 加参数后所有调用点（4 现有测试 + coverage_truth_facts）同步——任务 6.1 已列出。
