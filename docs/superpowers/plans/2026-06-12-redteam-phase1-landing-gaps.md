# 红队 DB 真值闭环 Phase 1：补落点 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 设计：`docs/design/2026-06-12-redteam-phase1-landing-gaps.md`（1-C/1-D/1-E）+ `docs/design/2026-06-12-active-collection-db-truth-closure.md`（1-A/1-B，P0-A/B + P1-C）。

**目标：** 让 Phase 0 的权威 gate「有真数据可读」：主动工具产物真落库（endpoint_add/host_add 孤儿修复）、coverage_truth 从 4 维扩到 12 维（+WHOIS/OSINT/主动 6 维）、target_intel 灰度扩到全 6 被动类、EAS/enumeration 开 DB 投影补格。
**架构：** 落库 writer（golish-pentest output_store）→ 业务表 → coverage_truth 只读投影（golish-db）→ 既有 hook `fetch_evidence_facts_for_gate` 合并（零改动）→ coverage_complete 判定（零改动）。
**技术栈：** Rust（sqlx/PgPool）、JSONB、toolsconfig JSON、harness stage JSON。

## 实施期勘验结论（修正设计文档假设）

1. **ffuf/gobuster pattern 字段名是 `path` 非 `url`**（`resources/toolsconfig/ffuf.json`），且 ffuf 默认输出第一列可能是相对 token。处理：toolsconfig 字段改名 `url`；`store_directory_entry` 对绝对 URL 解析 host→target_id，相对 path 落 target_id=NULL（保留现状容错，不参与投影/去重）。命令行回填 base URL 记为后续。
2. **arjun pattern 只产 count 无 url** → endpoint writer 对缺 url 的 record 返 Err 进 errors[]（无害不落库）；arjun 真落库需命令回填，记为后续。
3. **masscan 字段 `ip/port/protocol`** 与 `store_target_update_recon` 的 `host||ip||url` 兜底 + `build_port_json` 的 `protocol` 默认 tcp 兼容，**零 Rust 改动**。
4. **主动阶段 Empty 事实源不存在**（命令派生仅映射 dig/whois/subfinder）→ EAS/enum 若开 authoritative_found，「真跑→空」的格无法 checked_empty（违 I8 精神）。**本期 EAS/enum 只开 `derive_from_evidence`（DB 投影补格，不收紧）**；authoritative 等主动命令派生（Empty 源）就绪后再开，记为 Phase 1.5。
5. **OSINT provider 路径（recon_enrich_assets）落账 facts=None**（subject=公司名非资产，runtime `direct/mod.rs:441` 注释已说明）→ OSINT 的 found 走 1-D(b) DB 投影（org 级 OSINT 列非空 → 每 in-scope 资产 found）；checked_empty 在 authoritative 下不可满足（enrich 真跑零产出时模型只能 blocked+note，受 max_other_skips 限制）——已知边角，提示词已引导 blocked 路径。
6. **JSAPI 投影本期只查 `api_endpoints`（source IN js_analysis/crawler）**；`js_analysis_results` 无 target_id 关联，记为后续。

## Task 1 · golish-db：2 migrations + coverage_truth 扩 12 维（TDD）

**文件：**
- 新 `backend/crates/golish-db/migrations/20260612000002_api_endpoints_unique.sql`
- 新 `backend/crates/golish-db/migrations/20260612000003_organizations_whois.sql`
- 改 `backend/crates/golish-db/src/repo/coverage_truth.rs`

**步骤：**
1. migration A（api_endpoints 去重锚，I10 可重放）：

```sql
-- endpoint_add writer 的 ON CONFLICT 锚（设计 2026-06-12-active-collection §3.1）。
CREATE UNIQUE INDEX IF NOT EXISTS uq_api_endpoint_target_url_method
    ON api_endpoints(target_id, url, method);
```

2. migration B（WHOIS 专列，I10 先扩可空）：

```sql
-- Phase 1 (设计 2026-06-12-redteam-phase1 §3): WHOIS 结构化专列
-- { registrar, created, expires, registrant, name_servers: [...], raw_ref }
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS whois JSONB;
```

3. coverage_truth.rs：新 8 个 technique 常量（id 必须与 `technique_taxonomy.json` / stage JSON 逐字一致）：`GOLISH-INTEL-WHOIS`、`GOLISH-INTEL-OSINT`、`GOLISH-EAS-LIVENESS`、`GOLISH-EAS-PORT`、`GOLISH-EAS-SERVICE-FINGERPRINT`、`GOLISH-ENUM-DIR`、`GOLISH-ENUM-PARAM`、`GOLISH-ENUM-JSAPI`。
4. org 级查询扩展（单条 SQL 返回 4 bool）：`has_whois`=`whois IS NOT NULL AND whois <> 'null' AND whois <> '{}'::jsonb`；`has_osint`=`jsonb_array_length(intel->'records')>0 OR jsonb_array_length(contacts)>0 OR jsonb_array_length(social_accounts)>0 OR jsonb_array_length(business_systems)>0`（COALESCE 包裹防 NULL）。
5. per-asset 查询 6 条（全部 org 隔离 + scope='in'，模式抄 `build_subdomain_target_values_sql`）：LIVENESS=`http_status IS NOT NULL OR real_ip <> ''`；PORT=`jsonb_array_length(ports)>0`；SERVICE-FP=`JOIN fingerprints`；DIR=`JOIN directory_entries`（target_id 非 NULL 行天然生效）；PARAM=`JOIN api_endpoints WHERE jsonb_array_length(params)>0`；JSAPI=`JOIN api_endpoints WHERE source IN ('js_analysis','crawler')`。
6. `assemble_truth_facts` 入参重构为结构体 `TruthInputs`（2 个旧 bool + 2 新 bool + 8 个 HashSet），逐维 push，顺序稳定；现有 5 个单测改用结构体 + 新增每维 only-this 单测 + 全维组合单测。
7. `coverage_truth_facts` 串联全部查询（org_id=None 时 org 级四 bool 全 false，per-asset 退全局 in-scope，与现状一致）。

**验证：** `cargo nextest run -p golish-db -E 'test(coverage_truth)'` 全绿；`cargo clippy -p golish-db --all-targets -- -D warnings`。

## Task 2 · golish-pentest：endpoint writer + dirent target_id + dispatch（TDD）

**文件：**
- 新 `backend/crates/golish-pentest/src/output_store/endpoints.rs`
- 改 `findings.rs`（store_directory_entry 补 target_id）、`store_trait.rs`、`pg_adapter.rs`、`mod.rs`

**步骤：**
1. `endpoints.rs`：纯函数 `endpoint_parts(url) -> Option<(host, path, params_json)>`（`url::Url` 解析；查询参数名去重成 JSONB 数组）+ `source_for_tool(tool) -> &str`（katana→crawler、gau/waybackurls→historical、arjun→param、其他→crawler）+ writer：

```rust
pub(crate) async fn store_endpoint(
    pool: &PgPool, fields: &HashMap<String, String>,
    tool_name: &str, project_path: Option<&str>,
) -> PentestResult<()> {
    let url = fields.get("url").ok_or(... "No url field for endpoint")?;
    let (host, path, params) = endpoint_parts(url).ok_or(... "endpoint url not absolute")?;
    let target_id = find_or_create_target(pool, &host, project_path).await?;
    let method = fields.get("method").cloned().unwrap_or("GET".into());
    // INSERT INTO api_endpoints (target_id, project_path, url, method, path, params, source)
    // VALUES ... ON CONFLICT (target_id, url, method) DO NOTHING
}
```

2. `store_directory_entry`：url 以 http(s):// 开头 → `find_or_create_target` 填 target_id（让 `UNIQUE(url,tool) WHERE target_id IS NOT NULL` 去重生效）；否则 NULL 保持现状。
3. trait + adapter + dispatch：`OutputStore::store_endpoint`；`PgPentestStore` 转发；`mod.rs` dispatch 加 `"endpoint_add" =>` 分支。
4. 单测（同文件 `#[cfg(test)]`）：`endpoint_parts` 的 host/path/params 提取（带 query、无 query、相对 path→None、非法 URL→None）；`source_for_tool` 映射。

**验证：** `cargo nextest run -p golish-pentest -E 'test(endpoint)'` 全绿；`rg '"endpoint_add"' backend/crates/golish-pentest/src/output_store/mod.rs` 命中 dispatch。

## Task 3 · resources/toolsconfig：4 文件修正

1. `ffuf.json`/`gobuster.json`：`db_action` → `directory_entry_add`；pattern fields 的 `path` 改 `url`（status/size/words/lines 不动）。
2. `masscan.json`：`db_action` → `target_update_recon`（pattern 字段 ip/port/protocol 零改动）。
3. `katana.json`/`gau.json`/`waybackurls.json`/`arjun.json`：`db_action` 保持 `endpoint_add`（接 Task 2 新 writer）。

**验证：** 每个改动文件 `python3 -m json.tool` 合法；`rg '"db_action"' resources/toolsconfig/{ffuf,gobuster,masscan}.json` 值正确。

## Task 4 · golish-pentest organizations：WHOIS 专列路由（TDD）

**文件：** `output_store/organizations/{writers,mod,tests}.rs`

1. `writers.rs` 加 `merge_whois`：`UPDATE organizations SET whois = COALESCE(whois,'{}'::jsonb) || $2::jsonb WHERE id = $1`（对象 merge，幂等）。
2. `mod.rs`：ROUTED_KEYS 加 `registrar`/`whois_created`/`whois_expires`/`registrant`/`name_servers`；路由块把这些键组装为 `{registrar, created, expires, registrant, name_servers}` 对象（name_servers 按 `,` 切数组）调 `merge_whois`。
3. tests.rs 按现有测试模式补 whois 路由断言。

**验证：** `cargo nextest run -p golish-pentest -E 'test(organization)'` 全绿。

## Task 5 · evidence_facts.rs：WHOIS empty 判定收紧（小）

`passive_intel_outcome` 对 `GOLISH-INTEL-WHOIS` 增加确定性 empty 形态：`No match for`/`NOT FOUND`/`No Data Found`（大小写不敏感）→ `"empty"`（I8：whois 真查了无记录）。同步改 `nonempty_output_is_found` 单测 + 新增 whois empty 单测。

**验证：** `cargo nextest run -p golish-agent-kit -E 'test(evidence_facts) | test(whois)'` 全绿。

## Task 6 · harness stages：灰度扩维 + 守卫

1. `target_intel.json`：`authoritative_techniques` 从 4 类扩到全 6 类（+`GOLISH-INTEL-WHOIS`、`GOLISH-INTEL-OSINT`）；`$comment_authoritative` 与 on_fail hints 同步更新（WHOIS→organizations.whois 或 whois 命令账本、OSINT→enrich 落 org 列）。
2. `external_attack_surface.json` + `enumeration.json`：`coverage_complete` 加 `"derive_from_evidence": true`（DB 投影补格；**不开 authoritative**——主动 Empty 事实源未就绪，见勘验结论 4）+ hints 加一句「真值投影来自 DB 落库，跑真工具落真数据」。
3. `stage_spec.rs` 守卫断言：EAS/enum 各加 derive_from_evidence 断言（仿 target_intel 现有断言）。

**验证：** `python3 -m json.tool` 三文件合法；`cargo nextest run -p golish-agent-kit -E 'test(stage_spec)'` 全绿（守卫断言实读 JSON）。

## Task 7 · 收口验证

- `cargo nextest run -p golish-db -p golish-pentest -p golish-agent-kit --status-level fail` 全绿
- `cargo clippy -p golish-db -p golish-pentest -p golish-agent-kit --all-targets -- -D warnings` 零告警 + `cargo fmt` 净
- `cargo check -p golish` exit 0（下游链）
- `python3 scripts/check_repo_ownership.py` 无新违规
- progress/feature_list 更新（含勘验结论 4/5 的取舍记录）

## 红线（继承总纲 §8 + active-closure §0）

- coverage_truth 只产 Found 语义；DB 无数据绝不推 checked_empty（I8）
- gate 纯函数零改动；hook 注入链零改动（只动 coverage_truth.rs 内部）
- findings 链路不碰；migration I10 可重放；writer 只写真实工具产物
- EAS/enum 本期不开 authoritative（防真空结果卡死）

## 后续（不在本期）

- 主动工具命令派生（Empty 事实源）→ EAS/enum 开 authoritative（Phase 1.5）
- ffuf/arjun 命令行回填 base URL；js_analysis_results 接 JSAPI 投影
- OSINT provider 落账打 per-asset technique 标注（A1 follow-up）
