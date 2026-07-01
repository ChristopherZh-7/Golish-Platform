# 枚举阶段「四轴拆分 + IP-web 纳入」实现计划

> **面向 AI 代理的工作者：** 使用 `.cursor/skills/executing-plans` 逐任务实现此计划。设计源：
> `docs/design/2026-07-01-enumeration-four-axis-and-ip-web.md`（用户已拍板 §12 全部默认决策）。

**目标：** 把枚举阶段「资产覆盖」矩阵从三轴（DIR/PARAM/JSAPI）扩为四轴（**JS → DIR → PARAM →
JSAPI**，JS 收集独立成轴），并让 EAS 探到 HTTP 的裸 IP（`targets.http_status` 非空）与域名同等纳入
内容枚举（全套四类）。

**架构：** 分两个可独立回滚的 PR。
- **PR-1 四轴拆分**：新增技术 `GOLISH-ENUM-JS`（真值 = `js_analysis_results` 有行），`GOLISH-ENUM-JSAPI`
  语义收窄为「从 JS/爬虫抽取 API 端点」（SQL 不变）。直接改 baseline / 清单 / spec（加法式扩展）；
  enumeration 是 non-authoritative（agent 自报 `checked_empty` 兜底），无 JS 站点由 methodology 引导自报，
  不引入运行时开关（灰度 = 分 PR + revert）。
- **PR-2 IP-web 纳入**：新增确定性判定 `build_web_capable_ip_values_sql`（`http_status` 非空 IP）；
  `technique_applies_web_aware` 让 web-capable IP 参与四类；经 `GateContext.web_capable_assets`（默认
  `None` = 旧行为）seam 注入 gate / UI / worklist；spec bool `enum_ip_web_coverage` 控制是否注入 +
  parity 测。改 gate 判定核心，遵循 host_aware_coverage/freshness_window 的 spec-bool 灰度模式。

**技术栈：** Rust（`golish-agent-kit` / `golish-db` / `golish-agent-app`）+ resource JSON
（`resources/harness/**`）+ React/TS（`StageAssetCoveragePanel.tsx`）；`cargo nextest` / `vitest`。

**关键既有事实（已核对现状，行号为 2026-07-01）：**
- gate 期望技术：`execute.rs::gate_expected_techniques` → `sprint_contract::expected_techniques_for_target_types`
  （`sprint_contract.rs:136`）→ `resolve_expected_techniques`（`technique_resolver.rs:41`）→ `stage_baseline`
  （`technique_resolver.rs:13`）。submit 预检与 stage-close 共用此函数，改 baseline 即两路一致。
- per-asset 裁剪：`coverage_complete`（`rule_engine.rs:534`）在 `spec.host_aware_coverage` 时用
  `technique_applies_to_value(spec.kind, class, asset, tech)`（`rule_engine.rs:652`）。
- UI/worklist：`stage_coverage.rs` 的 `techniques_for_stage`（941）、`coverage_cells_with_eas_parent_ips`
  （786，判定在 811）、`next_wave_coverage_cells_with_eas_parent_ips`（884，判定在 907）、
  `filter_enumeration_assets_by_eas_found`（374）、`technique_label`（1069）、`suggested_tools`（1087）。
- 真值：`coverage_truth.rs` 的 `TECH_ENUM_*`（52-54）、`build_*_values_sql`、`TruthInputs`（349）、
  `assemble_truth_facts_typed`（376）、`coverage_truth_facts`（470）、`IP_TYPE_IN_LIST`（44）、
  `build_in_scope_values_sql`（141）。
- GateContext：`rule_engine.rs:239`（`in_scope_assets/asset_types/expected_techniques/evidence_facts/
  source_queries`，**无** web_capable_assets）；组装入口 `GateContextBuilder`（`execute.rs:2479`）。
- gate hook 注入点：`apply_harness_gate_hook`（`execute.rs:2309`），DB 查询在其调用方（提供
  `in_scope_assets` / `asset_types`）。
- taxonomy fail-closed 测：`technique_taxonomy::tests::all_embedded_expected_techniques_are_recognized`
  会校验所有 spec 的 `expected_techniques` 已在 `technique_taxonomy.json` 注册 → spec 与 taxonomy 必须同 PR 改。
- 前端矩阵列动态渲染（`techniques = assetRows[0]?.coverage`，`StageAssetCoveragePanel.tsx:1029`），后端多
  一格自动多一列；只需修 `techniqueKeyFromCell`（268）+ `techniqueShortLabel`（837）。

**DRY / YAGNI / TDD / 频繁 commit。** 每个 task 先写/改测试（红）→ 实现（绿）→ 跑验证 → commit。

---

## PR-1 · 四轴拆分（独立 `GOLISH-ENUM-JS`）

### Task 1.1 · coverage_truth：JS 真值 SQL + 投影（TDD）

**文件：** `backend/crates/golish-db/src/repo/coverage_truth.rs`

**步骤：**
1. 在 `TECH_ENUM_JSAPI`（54）后加常量：

```rust
/// 内容枚举 · JS 资产收集（design 2026-07-01 §4.1）。真值 = 该 host 已落 js_analysis_results 行。
pub const TECH_ENUM_JS: &str = "GOLISH-ENUM-JS";
```

2. 先加**测试**（`#[cfg(test)] mod tests` 内），确认 SQL 形状与投影（红）：

```rust
#[test]
fn js_values_sql_joins_js_analysis_results_and_scoping() {
    let sql = build_js_values_sql(false);
    assert!(sql.contains("JOIN js_analysis_results jar ON jar.target_id = t.id"));
    assert!(sql.contains("t.scope::text = 'in'"));
    assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
    assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
    // freshness window on ⇒ 只数本次 stage-run 落库。
    assert!(build_js_values_sql(true).contains("jar.analyzed_at >= $2"));
}

#[test]
fn assemble_projects_js_per_asset() {
    let empty = subs(&[]);
    let js = subs(&["a.com"]);
    let mut inputs = empty_inputs(&empty);
    inputs.js_values = &js;
    let assets = vec!["a.com".to_string(), "b.com".to_string()];
    let facts = assemble_truth_facts_typed(&assets, &[], &inputs);
    assert!(facts.contains(&("a.com".to_string(), TECH_ENUM_JS)));
    assert!(!facts.iter().any(|(a, t)| a == "b.com" && *t == TECH_ENUM_JS));
}
```

3. 实现 `build_js_values_sql`（放在 `build_dir_values_sql`(296) 附近）：

```rust
/// ENUM-JS：该 host 已收集到 JS 资产（browser_collect_js_api → js_analysis_results）。
/// Phase D 行级窗：apply_window ⇒ 只数本次 stage-run 落库（jar.analyzed_at >= $2）。
fn build_js_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN js_analysis_results jar ON jar.target_id = t.id",
        "",
        apply_window.then_some("jar.analyzed_at"),
    )
}
```

4. `TruthInputs`（349）加字段（放在 `dir_values` 前，保持 ENUM 分组）：`pub js_values: &'a HashSet<String>,`
5. `assemble_truth_facts_typed`（376）在 `if inputs.dir_values.contains(asset)`（431）**之前**加：

```rust
if inputs.js_values.contains(asset) {
    facts.push((asset.clone(), TECH_ENUM_JS));
}
```

6. `empty_inputs`（563 测试 helper）加 `js_values: empty,`。
7. `coverage_truth_facts`（470）：在 `dir_values` fetch（528）前加
   `let js_values = fetch_values(pool, &build_js_values_sql(aw), org_id, run_start).await?;`
   并在 `TruthInputs { ... }` 里加 `js_values: &js_values,`。
8. 更新 `assemble_combines_all_dimensions_in_stable_order`（1029）与 `assemble_each_active_dimension_only_for_matching_asset`（986）：为新 `js_values` 补 `&one`/`&HashSet::new()` 并在期望向量的 `TECH_ENUM_DIR` 前插入 `("a.com", TECH_ENUM_JS)`（顺序：JS 在 DIR 前）。同样更新 `active_dimension_sqls_off_omit_row_level_window` / `..._on_window_their_collection_timestamp` 若要覆盖 JS（可选：加 `build_js_values_sql`）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-db coverage_truth --status-level fail
```
预期：全绿（含新 2 测 + 更新的顺序测）。

**提交：** `feat(coverage): add GOLISH-ENUM-JS truth (js_analysis_results) [PR-1 1/6]`

---

### Task 1.2 · technique_resolver：baseline 加 JS（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`

**步骤：**
1. 改测试 `enumeration_is_web_only_per_asset`（433）：三处 `.len()==3` 改 `==4`（Domain/Url/Other）。
2. `stage_baseline`（28）Enumeration 臂改（JS 首位）：

```rust
StageKind::Enumeration => vec![
    "GOLISH-ENUM-JS",
    "GOLISH-ENUM-DIR",
    "GOLISH-ENUM-PARAM",
    "GOLISH-ENUM-JSAPI",
],
```

3. `enumeration_drops_param_when_no_web_asset`（183）仍成立（JS 不受 PARAM 裁剪影响，无需改），跑测确认。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver --status-level fail
```
预期：`enumeration_is_web_only_per_asset` 绿（len 4），其余零回归。

**提交：** `feat(harness): enumeration baseline adds JS axis [PR-1 2/6]`

---

### Task 1.3 · taxonomy + spec + methodology（resource）

**文件：** `resources/harness/technique_taxonomy.json`、`resources/harness/stages/enumeration/spec.json`、
`resources/harness/stages/enumeration/methodology.md`

**步骤：**
1. taxonomy（39）：把 `GOLISH-ENUM-JSAPI` 名改 `"API Endpoint Extraction (from JS/crawler)"`，并在其前加：

```json
  "GOLISH-ENUM-JS": { "name": "JS Asset Collection", "standard": "GOLISH custom" },
```

2. spec.json：`expected_techniques`（55-57）加 `"GOLISH-ENUM-JS"` 于首位；`coverage_axis`（61）改
   `["JS", "DIR", "PARAM", "JSAPI"]`；`$comment_expected_techniques` / `$comment_stage_run` 补一句四轴说明。
3. methodology.md：在「Coverage + stop condition」列表把 `GOLISH-ENUM-JS` 列为独立目标（JS 收集
   = `browser_collect_js_api` 落 `js_analysis_results`），并说明「跑了 `browser_collect_js_api` 但 0 JS →
   自报 `checked_empty`（I8）」；JSAPI 描述收窄为「从 JS/爬虫抽取 API 端点（`js_extract_apis`）」。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit technique_taxonomy --status-level fail
# all_embedded_expected_techniques_are_recognized 必须绿（spec+taxonomy 同步）
python3 -c "import json,sys; [json.load(open(p)) for p in ['resources/harness/technique_taxonomy.json','resources/harness/stages/enumeration/spec.json']]; print('JSON VALID')"
```

**提交：** `feat(harness): register GOLISH-ENUM-JS, enumeration coverage_axis=4 [PR-1 3/6]`

---

### Task 1.4 · stage_coverage：清单 B + label + hints（TDD）

**文件：** `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

**步骤：**
1. `techniques_for_stage`（941）Enumeration 臂（956）加 `TECH_ENUM_JS` 首位：

```rust
StageKind::Enumeration => vec![
    golish_db::repo::coverage_truth::TECH_ENUM_JS,
    golish_db::repo::coverage_truth::TECH_ENUM_DIR,
    golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
    golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
],
```

2. `technique_label`（1069）：加 `TECH_ENUM_JS => "JS"`；把 `TECH_ENUM_JSAPI => "JS/API"` 改 `=> "API"`。
3. `suggested_tools`（1087）：加 `TECH_ENUM_JS => vec!["browser_collect_js_api".to_string()]`；把
   `TECH_ENUM_JSAPI` 臂（1115）收窄为 `vec!["js_extract_apis".to_string()]`。
4. 加/改测试：在该文件 `#[cfg(test)]` 内新增，断言四轴顺序 + label：

```rust
#[test]
fn enumeration_exposes_four_axes_js_first() {
    let t = techniques_for_stage(StageKind::Enumeration);
    assert_eq!(
        t,
        vec![
            golish_db::repo::coverage_truth::TECH_ENUM_JS,
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
        ]
    );
    assert_eq!(technique_label(golish_db::repo::coverage_truth::TECH_ENUM_JS), "JS");
    assert_eq!(technique_label(golish_db::repo::coverage_truth::TECH_ENUM_JSAPI), "API");
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail
```

**提交：** `feat(coverage-ui): enumeration snapshot exposes JS axis, JSAPI→API [PR-1 4/6]`

---

### Task 1.5 · 前端 Panel：JS/API 精确区分（TDD）

**文件：** `frontend/components/Engagement/StageAssetCoveragePanel.tsx`
（+ 测试 `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`，若不存在则建）

**步骤：**
1. `techniqueKeyFromCell`（268）：把 `if (text.includes("JSAPI") || text.includes("JS")) return "JSAPI";`
   （277）拆成先具体后一般（`GOLISH-ENUM-JSAPI` 含 `JSAPI`，`GOLISH-ENUM-JS` 只含 `JS`）：

```ts
  if (text.includes("JSAPI")) return "JSAPI";
  if (text.includes("JS")) return "JS";
```

2. `techniqueShortLabel`（837）：在 `parameter` 分支后加 JS/API 显式映射（避免 fallback 歧义）：

```ts
  if (normalized === "js") return "JS";
  if (normalized === "api" || normalized.includes("jsapi")) return "API";
```

3. 测试（新增/补）：渲染一个含四轴的 `assetRows[0].coverage` 快照（technique 分别为
   `GOLISH-ENUM-JS`/`-DIR`/`-PARAM`/`-JSAPI`，label `JS`/`Directory`/`Parameter`/`API`），断言表头出现四列且
   `JS` 与 `API` 两列独立（`techniqueKeyFromCell` 分别返回 `"JS"` / `"JSAPI"`）。

**验证：**
```bash
pnpm test:run -- frontend/components/Engagement/StageAssetCoveragePanel
just check-fe
```

**提交：** `feat(coverage-ui): panel distinguishes JS vs JSAPI columns [PR-1 5/6]`

---

### Task 1.6 · 模块卡 + PR-1 收口

**文件：** `docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
（如涉及）、`docs/modules/frontend/components.md`、`agent-progress.md`、`feature_list.json`

**步骤：**
1. 更新模块卡：coverage_truth 新增 `TECH_ENUM_JS` / `build_js_values_sql`；前端 Panel 四轴。
2. `feature_list.json` 追加/更新本功能条目（`in_progress`）。
3. `agent-progress.md` 记录 PR-1 会话证据。

**验证（PR-1 整体门禁）：**
```bash
cd backend && cargo nextest run -p golish-db -p golish-agent-kit -p golish-agent-app --status-level fail
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
pnpm test:run -- frontend/components/Engagement/StageAssetCoveragePanel
```

**提交：** `docs: PR-1 four-axis enumeration coverage notes [PR-1 6/6]`

---

## PR-2 · IP-web 资产纳入内容枚举

### Task 2.1 · technique_resolver：`technique_applies_web_aware`（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`

**步骤：**
1. 先加测试（红）：

```rust
#[test]
fn enumeration_ip_web_capable_gets_all_four() {
    use StageKind::Enumeration as Enum;
    for tech in ["GOLISH-ENUM-JS","GOLISH-ENUM-DIR","GOLISH-ENUM-PARAM","GOLISH-ENUM-JSAPI"] {
        // web-capable IP ⇒ 纳入全套四类。
        assert!(technique_applies_web_aware(Enum, AssetClass::Ip, "1.2.3.4", tech, true));
        // 非 web IP ⇒ 维持排除（not_applicable）。
        assert!(!technique_applies_web_aware(Enum, AssetClass::Ip, "1.2.3.4", tech, false));
    }
    // 域名不受 web_capable 影响（委托原判定，parity）。
    assert!(technique_applies_web_aware(Enum, AssetClass::Domain, "a.com", "GOLISH-ENUM-DIR", false));
    // 非 Enumeration 阶段：委托原判定，web_capable 不改变行为。
    assert!(technique_applies_web_aware(StageKind::TargetIntel, AssetClass::Domain, "a.com", "GOLISH-INTEL-WHOIS", true));
}
```

2. 实现（放在 `technique_applies_to_value`(118) 后）：

```rust
/// Evidence-aware 扩展：在 [`technique_applies_to_value`] 之上叠一层 web-capability
/// 上下文（design 2026-07-01 §5.2）。仅对 Enumeration + Ip/Cidr 生效：`web_capable`
/// （EAS httpx 探到 http，见 coverage_truth::build_web_capable_ip_values_sql）为真 ⇒
/// 该 IP 与域名同等纳入四类；为假 ⇒ 维持 IP 全排除。其它 stage/class 委托原判定
/// （`web_capable` 不传或为默认时逐字节一致）。
pub fn technique_applies_web_aware(
    stage: StageKind,
    class: AssetClass,
    value: &str,
    tech: &str,
    web_capable: bool,
) -> bool {
    if matches!(stage, StageKind::Enumeration) && matches!(class, AssetClass::Ip | AssetClass::Cidr) {
        return web_capable;
    }
    technique_applies_to_value(stage, class, value, tech)
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver --status-level fail
```

**提交：** `feat(harness): technique_applies_web_aware for web-capable IP [PR-2 1/6]`

---

### Task 2.2 · coverage_truth：`build_web_capable_ip_values_sql`（TDD）

**文件：** `backend/crates/golish-db/src/repo/coverage_truth.rs`

**步骤：**
1. 先加测试（红）：

```rust
#[test]
fn web_capable_ip_sql_filters_http_status_and_ip_types() {
    let sql = build_web_capable_ip_values_sql();
    assert!(sql.contains("t.http_status IS NOT NULL"));
    assert!(sql.contains("t.target_type::text IN ('ip', 'ipv4'"));
    assert!(sql.contains("t.scope::text = 'in'"));
    assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
    assert!(!sql.contains("$2"));
}
```

2. 实现（放在 `build_ipwhois_values_sql`(337) 附近）：

```rust
/// web-capable IP（design 2026-07-01 §5.1）：in-scope Ip/Cidr 且 EAS httpx 探到 HTTP
/// 响应（targets.http_status 非空）。内容枚举把这类 IP 视同 web 资产。判定只用
/// http_status（比 LIVENESS 的 ping/端口更精确，避免仅 ping 通、无 web 的 IP 误纳入）。
pub fn build_web_capable_ip_values_sql() -> String {
    build_in_scope_values_sql(
        "",
        &format!("AND t.http_status IS NOT NULL AND t.target_type::text IN {IP_TYPE_IN_LIST}"),
        None,
    )
}
```

3. 新增只读查询封装（供 gate hook / UI / worklist 求集合，org 隔离）：

```rust
/// 便捷入口：查该 org 的 web-capable IP `value` 集合（design 2026-07-01 §5.3）。
pub async fn web_capable_ip_values(
    pool: &PgPool,
    org_id: Option<Uuid>,
) -> Result<HashSet<String>> {
    fetch_values(pool, &build_web_capable_ip_values_sql(), org_id, None).await
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-db coverage_truth --status-level fail
```

**提交：** `feat(coverage): build_web_capable_ip_values_sql (http_status IP) [PR-2 2/6]`

---

### Task 2.3 · StageSpec：`enum_ip_web_coverage` 灰度字段

**文件：** `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`、
`resources/harness/stages/enumeration/spec.json`

**步骤：**
1. `StageSpec`（42）在 `host_aware_coverage`（162）附近加 `#[serde(default)] pub enum_ip_web_coverage: bool,`
   （默认 false = 旧行为，I10 逐字节一致）。
2. `enumeration/spec.json` 加 `"enum_ip_web_coverage": true`（本 PR 目标启用）+ `$comment_enum_ip_web`
   说明「EAS http_status 非空的 IP 纳入内容枚举四类；默认 false 逐字节一致」。
3. 加/改 spec 测试：断言 enumeration spec `enum_ip_web_coverage==true`、其它 stage 默认 false。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit stage_spec --status-level fail
python3 -c "import json; json.load(open('resources/harness/stages/enumeration/spec.json')); print('JSON VALID')"
```

**提交：** `feat(harness): enum_ip_web_coverage spec flag (default off, enumeration on) [PR-2 3/6]`

---

### Task 2.4 · gate：`GateContext.web_capable_assets` + web-aware 判定 + parity（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
（+ `GateContextBuilder` 定义处，grep `impl GateContextBuilder`）

**步骤：**
1. `GateContext`（239）加字段：

```rust
    /// design 2026-07-01 §5.3：web-capable IP `value` 集合（EAS http_status 非空）。
    /// `None` = 不注入 = 旧行为（enumeration IP 仍全排除）。仅当 spec.enum_ip_web_coverage
    /// 时由 stage-close/预检外层从 coverage_truth::web_capable_ip_values 求值填入。
    pub web_capable_assets: Option<std::collections::HashSet<String>>,
```

2. `coverage_complete`（637-660）：把 `technique_applies_to_value(...)`（652）调用改为：

```rust
                    crate::harness::technique_resolver::technique_applies_web_aware(
                        spec.kind,
                        class,
                        asset,
                        t.as_str(),
                        ctx.web_capable_assets
                            .as_ref()
                            .is_some_and(|s| s.contains(*asset)),
                    )
```

   （`web_capable_assets=None` ⇒ `is_some_and` 恒 false ⇒ `technique_applies_web_aware` 走
   `technique_applies_to_value` 委托 = 逐字节一致。）
3. `GateContextBuilder`：加 `web_capable_assets(HashSet<String>)` setter + `build()` 填字段（默认 None）。
4. parity + 行为测（新增）：

```rust
#[test]
fn coverage_complete_web_capable_ip_requires_four_when_injected() {
    // enum spec + host_aware + web_capable_assets 含该 IP ⇒ IP 需四类终态，缺则 BLOCK。
    // （构造：expected=4 轴，in_scope=["1.2.3.4"]，asset_types 该 IP=ip，coverage 空）
    // ...断言 Block 且理由含四类。
}

#[test]
fn coverage_complete_ip_web_parity_when_not_injected() {
    // web_capable_assets=None ⇒ 该 IP 仍被 technique_applies_to_value 全排除（not_applicable）
    // ⇒ 不因缺四类而 BLOCK（与旧行为逐字节一致）。
}

#[test]
fn coverage_complete_domain_unaffected_by_web_capable() {
    // 注入 web_capable_assets 后，域名资产的期望技术集不变（parity 硬测）。
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine coverage_complete --status-level fail
```

**提交：** `feat(gate): GateContext.web_capable_assets seam + web-aware coverage [PR-2 4/6]`

---

### Task 2.5 · execute.rs：注入 `web_capable_assets`（TDD）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
（+ `harness_submit_tool.rs` 预检 gate_context，保持两路一致）

**步骤：**
1. `apply_harness_gate_hook`（2309）签名加参数 `web_capable_assets: Option<std::collections::HashSet<String>>,`；
   `GateContextBuilder`（2479）链上加 `.web_capable_assets(web_capable_assets.unwrap_or_default())`。
2. 在 `apply_harness_gate_hook` 的**调用方**（提供 `in_scope_assets`/`asset_types` 的 DB 求值处，grep
   `apply_harness_gate_hook(`）：当 `harness.stage_spec.enum_ip_web_coverage` 时调
   `golish_db::repo::coverage_truth::web_capable_ip_values(pool, org_id).await` 求集合传入；否则 `None`。
   （DB 查询失败 fail-safe 退 `None` + warn，不误纳入。）
3. `harness_submit_tool.rs` 的 `gate_context`（grep）同步：预检也按 spec flag 注入 web_capable_assets，
   保证预检与 stage-close 口径一致（避免预检假 PASS / close BLOCK 分歧）。
4. 测试：为注入函数写纯函数单测（spec flag on/off → 注入/None），或复用既有 gate hook 测双分支。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail
```

**提交：** `feat(harness): inject web_capable_assets into enumeration gate + submit preview [PR-2 5/6]`

---

### Task 2.6 · stage_coverage：UI/worklist web-aware（TDD）+ methodology + 收口

**文件：** `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、
`resources/harness/stages/enumeration/methodology.md`、模块卡 / `feature_list.json` / `agent-progress.md`

**步骤：**
1. 组装快照的入口（`coverage_cells_with_eas_parent_ips`(786) / `next_wave_*`(884) 调用方）先查
   `web_capable_ip_values(pool, org_id)`（当 enumeration + spec flag on），得集合传入。
2. `coverage_cells_with_eas_parent_ips`(811) 与 `next_wave_*`(907) 的
   `technique_applies_to_value(stage, class, &asset.value, technique)` 改为
   `technique_applies_web_aware(stage, class, &asset.value, technique, web_capable.contains(&asset.value))`
   （新增参数 `web_capable: &BTreeSet<String>`，非 enumeration/未注入时传空集 = 旧行为）。
3. `filter_enumeration_assets_by_eas_found`（374）追加放行：`Ip|Cidr` 且 `value ∈ web_capable` 且有
   `GOLISH-EAS-LIVENESS` found → 保留进 worklist（现只保 Domain/Url）。为该函数新增 `web_capable: &BTreeSet<String>`
   入参，调用方传集合。
4. 更新单测 `enumeration_worklist_read_model_keeps_eas_live_web_roots`（原断言 live_ip 被丢）：新增一个
   web-capable IP 用例断言被保留；无 http 的 IP 仍丢。为 `coverage_cells` 加 web-capable IP 四格非
   not_applicable 的断言。
5. methodology.md：加「EAS 确认有 HTTP 的 IP（无域名）按 web 根同等做 JS/DIR/PARAM/JSAPI」。
6. 收口：模块卡（coverage_truth / stage_coverage / gate）、`feature_list.json` 状态、`agent-progress.md` 证据。

**验证（PR-2 整体门禁）：**
```bash
cd backend && cargo nextest run -p golish-db -p golish-agent-kit -p golish-agent-app --status-level fail
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
just precommit
```

**提交：** `feat(coverage): web-capable IP enters enumeration worklist + UI [PR-2 6/6]`

---

## 验证（整体 DoD，对齐设计 §9）

```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-db --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-db --all-targets -- -D warnings
pnpm test:run -- frontend/components/Engagement/StageAssetCoveragePanel
just check
```

**关键断言清单：**
- `technique_resolver`：`techniques_for(Enum, Domain).len()==4`；`technique_applies_web_aware(Enum, Ip, web=true)` 四类全 true、`web=false` 全 false；域名 parity。
- `coverage_truth`：`build_js_values_sql` join `js_analysis_results` + `analyzed_at>=$2`；`build_web_capable_ip_values_sql` 含 `http_status IS NOT NULL` + IP 类型；assemble 产 `TECH_ENUM_JS` fact。
- `stage_coverage`：四列 JS 首位；worklist 放行 web-capable IP、仍丢无 http 的 IP。
- `rule_engine`：注入 web_capable_assets 后 web-capable IP 需四类、非 web IP 仍豁免；**无域名资产决策变化**（parity 硬测）。
- 前端：JS 与 JSAPI 两列独立渲染。

---

## 自检（规格覆盖度 / 占位符 / 类型一致性）

- **规格覆盖**：设计 §4（四轴）→ Task 1.1-1.5；§4.4 空态 → Task 1.3 methodology（non-authoritative 自报兜底，
  不做代码自动派生，理由见架构段）；§5（IP-web）→ Task 2.1-2.6；§6 契约（无新 ts-rs）→ coverage cell 动态数组，
  `web_capable_assets`/`js_values` 后端内部类型；§7 files 全覆盖；§8 不变量 I5/I7/I8/I10/§2.7 见各 task。
- **占位符**：无 TODO；每 code step 有实际代码；Task 2.4/2.6 的测试骨架标注了构造要点（实现时按既有
  `coverage_complete_*` 测模式填 `GateRule`/`StageDeliverable`）。
- **类型一致**：`TECH_ENUM_JS`（db 常量）贯穿 coverage_truth / stage_coverage；`technique_applies_web_aware`
  签名（stage,class,value,tech,web_capable:bool）在 rule_engine 与 stage_coverage 调用一致；
  `web_capable_ip_values(pool,org_id)` 返回 `HashSet<String>`，gate 用 `HashSet`、UI/worklist 用 `BTreeSet`
  （调用方 `.into_iter().collect()` 转换）。
