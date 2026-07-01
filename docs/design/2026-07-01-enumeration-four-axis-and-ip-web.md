# 2026-07-01 · 枚举阶段覆盖矩阵：四轴拆分（独立 JS 收集）+ IP-web 资产纳入

> Date: 2026-07-01
> Status: **DESIGN — 用户已拍板 §12 全部默认（2026-07-01）；下一步 writing-plans**
> Author: BajieAsk-agent-3（全栈工程师）
> Related:
> - `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`（baseline + `technique_applies(_to_value)` 矩阵）
> - `backend/crates/golish-db/src/repo/coverage_truth.rs`（DB 真值 SQL + `TruthInputs`）
> - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`（UI 覆盖快照 + 枚举 worklist 过滤）
> - `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`（`coverage_complete` gate）
> - `resources/harness/stages/enumeration/spec.json`、`resources/harness/technique_taxonomy.json`、`resources/harness/stages/enumeration/methodology.md`
> - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`（资产覆盖矩阵 UI）
> Invariants touched: I5（ts-rs 跨 IPC；本设计不新增类型，coverage 数组动态）、I7（证据可追溯）、I8（"已检查为空" ≠ "未检查"）、I10（迁移向后兼容/灰度）、§2.5（pentest evidence）、§2.7（harness 核心 + gate BLOCKING 变更需 sign-off）
> Supersedes: 无（独立新增；host-aware 2b 对 enumeration 的 IP 全排除规则在 §5.2 被本设计放宽）

---

## 1. 背景 / 问题

枚举阶段（Enumeration）的"资产覆盖"矩阵（UI: `StageAssetCoveragePanel`）当前只有 **三轴**：`DIR / PARAM / JSAPI`。真实使用中暴露两个问题：

### 问题 1 · 覆盖应是"四类"，JS 收集要独立成一轴

`GOLISH-ENUM-JSAPI` 把"**JS 资产收集**"与"**从 JS 抽取 API 端点**"绑在同一格：

- taxonomy 名称：`"JS Collection / API Endpoint Extraction"`（`technique_taxonomy.json:39`）。
- DB 真值只认"抽出了 API"：`build_jsapi_values_sql` = `JOIN api_endpoints ... ae.source IN ('js_analysis','crawler')`（`coverage_truth.rs:315-321`）。

后果：**抓到一大堆 JS 文件、但还没抽出 API** 时，JSAPI 格不会亮绿——而这些 JS 文件其实实实在在落库在 `js_analysis_results`（`migrations/20260415100002_security_analysis.sql:65-88`，有 `target_id` FK + `analyzed_at`）。"JS 收集"这个真实、可测的动作在覆盖矩阵里没有位置。

→ 目标四类：**JS 收集 / DIR / PARAM / JSAPI**。

### 问题 2 · 只有 IP 没有域名、但 IP 是 web 资产时也要纳入（至少 JS 收集）

枚举阶段对资产类型 **一刀切**：只有 `Domain | Url` 是内容枚举目标，任何 `Ip | Cidr` 全排除。证据：

- 矩阵谓词：`technique_resolver.rs:91` `StageKind::Enumeration => matches!(class, Domain | Url)`；单测 `enumeration_is_web_only_per_asset` 断言 `techniques_for(Enum, Ip).is_empty()`。
- **worklist 更早就把 IP 滤掉**：`stage_coverage.rs::filter_enumeration_assets_by_eas_found`（374-403）只保留 `class ∈ {Domain,Url}` 且有 `GOLISH-EAS-LIVENESS` 的资产；单测 `enumeration_worklist_read_model_keeps_eas_live_web_roots`（1166-1207）明确断言 `live_ip` 被丢弃。

后果：一个 `http://115.28.135.55/` 这种纯 IP 的 web 服务（EAS 已探到 http），在枚举阶段既进不了 worklist，覆盖格也全是 not_applicable——JS 收集被漏做。

→ EAS 探活时 httpx 会把 HTTP 响应写进 `targets.http_status`（`build_liveness_values_sql` 就用 `t.http_status IS NOT NULL` 判活，`coverage_truth.rs:252/259`），这是"IP 是不是 web 资产"的现成、确定性判定源。

---

## 2. Decision（TL;DR，用户已拍板 §12）

1. **四轴拆分**：新增技术 `GOLISH-ENUM-JS`（JS 资产收集），真值 = `js_analysis_results` 有行；`GOLISH-ENUM-JSAPI` 收窄语义为"从 JS/爬虫抽取 API 端点"（真值/SQL 不变）。枚举 `coverage_axis` = `["JS","DIR","PARAM","JSAPI"]`，顺序 JS → DIR → PARAM → JSAPI。
2. **IP-web 纳入**：EAS `http_status` 非空的 IP/CIDR 视为 web-capable，进入枚举 worklist 并参与内容枚举；**与域名同等，纳入全套四类**（用户决策）。无 http 服务的 IP 维持全 not_applicable（守 I8）。
3. **三处共用同一 web-capability 判定**：gate `coverage_complete`、UI `coverage_cells`、worklist 过滤，避免 UI/gate/worklist 漂移。
4. **均为 gate BLOCKING 行为变更**，走灰度开关 + parity 测，分两个可独立回滚的 PR，合并前 §2.7 sign-off。

---

## 3. 现状证据（三份"枚举技术清单"必须同步）

枚举技术清单在代码里有 **三份**，四轴拆分必须一起改，否则矩阵/真值/gate 会漂移：

| # | 位置 | 现值 | 作用 |
|---|---|---|---|
| A | `technique_resolver.rs::stage_baseline(Enumeration)` (28) | `[DIR, PARAM, JSAPI]` | gate 的期望技术基线 |
| B | `stage_coverage.rs::techniques_for_stage(Enumeration)` (956-960) | `[DIR, PARAM, JSAPI]` | UI 快照逐格渲染的技术列 |
| C | `coverage_truth.rs` `TECH_ENUM_*` (52-54) + `build_*_values_sql` + `TruthInputs` | DIR/PARAM/JSAPI | DB 真值投影 |

另有 `resources/harness/stages/enumeration/spec.json`（`expected_techniques` + `coverage_axis`）与 `technique_taxonomy.json` 两份 resource 声明。

gate 逐资产过滤入口（问题 2 的关键）：`rule_engine.rs::coverage_complete`（534-）在 `spec.host_aware_coverage` 为 true 时，对每个 asset 用 `AssetClass::classify` + `technique_applies_to_value` 过滤期望技术（637-660）。

---

## 4. 问题 1 设计：四轴拆分（独立 `GOLISH-ENUM-JS`）

### 4.1 技术定义与真值

- **新 id**：`GOLISH-ENUM-JS`，taxonomy 名 `"JS Asset Collection"`。
- **JSAPI 收窄**：`GOLISH-ENUM-JSAPI` 名改 `"API Endpoint Extraction (from JS/crawler)"`；**SQL/真值不变**（仍 `api_endpoints.source IN ('js_analysis','crawler')`），只是语义与文案不再包含"收集"。
- **JS 真值 SQL**（`coverage_truth.rs` 新增，镜像 `build_dir_values_sql` 的 per-asset + freshness window 写法）：

```rust
pub const TECH_ENUM_JS: &str = "GOLISH-ENUM-JS";

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

- `TruthInputs` 加 `js_values: &HashSet<String>`；`assemble_truth_facts_typed` 加一条 push（`if inputs.js_values.contains(asset) { facts.push((asset.clone(), TECH_ENUM_JS)); }`）；`coverage_truth_facts` `fetch_values(build_js_values_sql(aw))`。

### 4.2 baseline / 快照 / spec / label / hints

- `stage_baseline(Enumeration)` → `["GOLISH-ENUM-JS","GOLISH-ENUM-DIR","GOLISH-ENUM-PARAM","GOLISH-ENUM-JSAPI"]`（清单 A）。
- `stage_coverage.rs::techniques_for_stage(Enumeration)` 同步加 `TECH_ENUM_JS` 于首位（清单 B）。
- `technique_label`：`TECH_ENUM_JS => "JS"`；`TECH_ENUM_JSAPI` 由 `"JS/API"` 改 `"API"`（避免与新 JS 轴混淆）。
- `suggested_tools`：`TECH_ENUM_JS => ["browser_collect_js_api"]`；`TECH_ENUM_JSAPI => ["js_extract_apis"]`（收窄为抽取工具）。
- `spec.json`：`expected_techniques` 加 `GOLISH-ENUM-JS`；`coverage_axis` = `["JS","DIR","PARAM","JSAPI"]`；相关 `$comment` 更新。
- `technique_taxonomy.json`：加 `GOLISH-ENUM-JS`，改 JSAPI 名。

### 4.3 前端（`StageAssetCoveragePanel.tsx`）

矩阵列本身是 **动态渲染**（`techniques = assetRows[0].coverage`，1029-1031/1334-1342），后端多一格 UI 自动多一列。只需补两处映射，否则短标签/筛选会退化：

- `techniqueShortLabel`（837-855）：把 `directory→DIR / parameter→PARAM` 之外补 `js/api` 区分——`label==="JS"→"JS"`、`label==="API"→"API"`（现在 JSAPI label 走 fallback）。
- `techniqueKeyFromCell`（268-285）：现在 `JS`/`JSAPI` 都命中 `text.includes("JS")→"JSAPI"`。要按 `technique` 精确区分：`GOLISH-ENUM-JS→"JS"`、`GOLISH-ENUM-JSAPI→"JSAPI"`，避免 live-work 徽标错配到同一列。
- 测试 `StageAssetCoveragePanel.test.tsx` 补四列渲染断言。

### 4.4 BLOCKING 与 I8 兜底

新增 JS 轴 = 每个 web 资产多一格要求。若站点确实无 JS，需要"跑了 `browser_collect_js_api` 但 0 JS → `checked_empty`"的证据派生（I8），否则会假 BLOCK：

- `harness/evidence_facts.rs`：`browser_collect_js_api` outcome 为空 → 派生 `GOLISH-ENUM-JS` 的 `Empty` 事实（镜像既有 EAS/ENUM 的 empty 派生）。
- methodology.md：把"JS 收集"列为独立 coverage 目标，并说明空态记 `checked_empty`。

---

## 5. 问题 2 设计：IP-web 资产纳入内容枚举

### 5.1 web-capability 判定源（确定性）

新增 `coverage_truth.rs::build_web_capable_ip_values_sql`：in-scope `Ip|Cidr` 类型且 `http_status IS NOT NULL` 的 target `value` 集合。

```rust
/// web-capable IP：EAS httpx 对该 IP 探到 HTTP 响应（targets.http_status 非空）。
/// 内容枚举把这类 IP 视同 web 资产（design 2026-07-01 §5）。
fn build_web_capable_ip_values_sql() -> String {
    build_in_scope_values_sql(
        "",
        &format!("AND t.http_status IS NOT NULL AND t.target_type::text IN {IP_TYPE_IN_LIST}"),
        None,
    )
}
```

> 判定只用 `http_status`（httpx 探到 HTTP）——比 `GOLISH-EAS-LIVENESS`（ping/端口也算活）更精确，避免把"仅 ping 通、无 web"的 IP 误纳入。`IP_TYPE_IN_LIST` 复用 2c-3 已有常量。

### 5.2 evidence-aware 的 `technique_applies`

现 `technique_applies(stage, class, tech)` 纯 class-based（DB-free）。**不改其签名**（避免动全部调用点），改为在其上叠一层 web-capability 上下文：

- 新增 `technique_applies_web_aware(stage, class, value, tech, web_capable: bool) -> bool`：
  - 非 Enumeration / 非 Ip|Cidr：委托 `technique_applies_to_value`（行为不变）。
  - `Enumeration` + `Ip|Cidr` + `web_capable==true`：返回 `true`（纳入全套四类）。
  - `Enumeration` + `Ip|Cidr` + `web_capable==false`：`false`（维持排除，not_applicable）。
- `technique_applies(Enumeration)` 的 `matches!(class, Domain|Url)` **保持不变**（这是 `web_capable==false` 的默认路径，保证不传上下文时逐字节一致 / 灰度关时零回归）。

### 5.3 三处接入（共用 web-capable 集合）

| 接入点 | 现状 | 改动 |
|---|---|---|
| gate `rule_engine.rs::coverage_complete` (637-660) | `technique_applies_to_value(kind,class,asset,t)` | `GateContext` 加 `web_capable_assets: Option<HashSet<String>>`（seam，默认 None=旧行为）；判定改 `technique_applies_web_aware(..., web_capable_assets.contains(asset))` |
| UI `stage_coverage.rs::coverage_cells_with_eas_parent_ips` (811-816) + `next_wave_*` (907-912) | 同上 | 组装快照时先查 `build_web_capable_ip_values_sql` 得集合，判定改 web-aware |
| worklist `stage_coverage.rs::filter_enumeration_assets_by_eas_found` (374-403) | 仅 `Domain|Url` + LIVENESS | 追加放行：`Ip|Cidr` 且 value ∈ web-capable 集合 |

gate 侧的 `web_capable_assets` 由 stage-close 外层（`execute.rs` gate hook，与 `asset_types`/`expected_techniques` 同处注入）从 `build_web_capable_ip_values_sql` 求值后填入 `GateContext`，保持 gate 纯函数 / DB-free（沿用既有 seam 模式）。

### 5.4 IP 覆盖的四格如何满足

纳入后，web-capable IP 的四格真值走既有 per-asset SQL（`js_analysis_results` / `directory_entries` / `api_endpoints` 均以 `target_id` join，对 IP target 同样成立）——**无需额外真值改动**，只要工具对 IP base_url 落库即可。methodology.md 增补："EAS 确认有 http 的 IP（无域名）按 web 根同等做 JS/DIR/PARAM/JSAPI。"

---

## 6. 数据模型 / 契约（ts-rs）

- **不新增 ts-rs 类型**：`StageAssetCoverageCell`（`stage_coverage.rs:61-72`）已是 `{technique,label,state,...}` 动态数组，多一格 JS 自动流到前端 `frontend/lib/generated/`。
- `GateContext.web_capable_assets`、`TruthInputs.js_values` 均为后端内部类型，不跨 IPC。
- I10：`GateContext` 新字段默认 `None`、`coverage_truth` 新 SQL 加法式，灰度关/不注入时逐字节一致。

---

## 7. Files（预计改动）

| File | Change | 问题 |
|---|---|---|
| `resources/harness/technique_taxonomy.json` | +`GOLISH-ENUM-JS`，改 JSAPI 名 | 1 |
| `resources/harness/stages/enumeration/spec.json` | `expected_techniques` +JS；`coverage_axis`=4 轴 | 1 |
| `resources/harness/stages/enumeration/methodology.md` | JS 收集独立目标 + IP-web 说明 + 空态 checked_empty | 1/2 |
| `.../harness/technique_resolver.rs` | baseline +JS；`technique_applies_web_aware` + 单测 | 1/2 |
| `.../golish-db/src/repo/coverage_truth.rs` | `TECH_ENUM_JS`+`build_js_values_sql`+`build_web_capable_ip_values_sql`+`TruthInputs.js_values`+assemble+fetch + 单测 | 1/2 |
| `.../golish-agent-app/src/ai/commands/stage_coverage.rs` | `techniques_for_stage`+JS；label/suggested_tools；web-aware 判定；worklist 放行 IP + 单测 | 1/2 |
| `.../harness/gate/rule_engine.rs` | `GateContext.web_capable_assets`；`coverage_complete` 用 web-aware + parity 测 | 2 |
| `.../task_orchestrator/subtask_phases/execute.rs`（gate hook） | 注入 `web_capable_assets` | 2 |
| `.../harness/evidence_facts.rs` | `browser_collect_js_api` 空 → `GOLISH-ENUM-JS` Empty | 1 |
| `frontend/components/Engagement/StageAssetCoveragePanel.tsx` | `techniqueShortLabel`/`techniqueKeyFromCell` 加 JS/API 区分 + 测 | 1 |
| `docs/modules/...` 卡 + `feature_list.json` + `agent-progress.md` | 同步（I6/§2.4） | all |

---

## 8. 不变量对齐

- **I5**：无手写跨 IPC 类型；coverage cell 动态数组，ts-rs 自动导出。
- **I7/§2.5**：JS 轴真值来自落盘的 `js_analysis_results`；IP-web 判定来自 EAS `http_status` 证据——均可追溯。
- **I8**：JS 空态、IP 无 http 均记 `checked_empty`/`not_applicable`，不与"未检查"混淆。
- **I10**：`GateContext` 新字段默认 None、SQL 加法式、灰度开关；关时逐字节一致。
- **§2.7**：harness baseline + gate BLOCKING 变更，合并前 sign-off（用户已就设计拍板；实现后仍需 parity 测通过）。

---

## 9. Verification（DoD）

```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-db --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-db --all-targets -- -D warnings
pnpm test:run -- frontend/components/Engagement/StageAssetCoveragePanel
just check
```

关键测试：
- `technique_resolver`：`techniques_for(Enum, Domain).len()==4`；`technique_applies_web_aware(Enum, Ip, web=true)` 四类全 true、`web=false` 全 false。
- `coverage_truth`：`build_js_values_sql` join `js_analysis_results` + `analyzed_at>=$2`；`build_web_capable_ip_values_sql` 含 `http_status IS NOT NULL` + IP 类型；assemble 产 `TECH_ENUM_JS` fact。
- `stage_coverage`：四列渲染；worklist 放行 web-capable IP、仍丢无 http 的 IP；web-capable IP 四格非 not_applicable。
- `rule_engine`：parity——注入 `web_capable_assets` 后 web-capable IP 需四类、非 web IP 仍豁免；**无域名资产决策变化**（parity 硬测）。

---

## 10. 增量实现顺序（2 个 PR，各自可回滚）

1. **PR-1 四轴拆分**（问题 1，低风险）：taxonomy/spec/baseline/清单 B/coverage_truth JS 真值/label/hints/前端 + evidence 空态派生。灰度：`GOLISH_ENUM_JS_AXIS`（默认关→观察→开）。
2. **PR-2 IP-web 纳入**（问题 2，harness 核心）：`technique_applies_web_aware` + `build_web_capable_ip_values_sql` + 三处接入 + gate seam + parity 测。灰度：`GOLISH_ENUM_IP_WEB`（默认关）。

每 PR 自带 TDD（先红后绿）+ `just precommit` 绿 + 模块卡/feature_list/progress 更新。

---

## 11. 错误处理 / 回滚

- JS 真值：`js_analysis_results` 空 → JS 格 pending，由 evidence 空态派生转 `checked_empty`；查询异常回退空集（不误 found）。
- web-capable 判定：`http_status` 为空 → IP 维持 not_applicable（不误纳入）。
- 回滚：两 PR 独立 revert；灰度开关关闭即回旧行为（逐字节一致）。

---

## 12. 决策记录（用户已拍板 2026-07-01：全部采用建议默认）

1. **JS 收集真值** = `js_analysis_results` 有行（抓到 JS 即 found）；JSAPI 维持"抽 API"。✅
2. **命名 / 顺序** = `GOLISH-ENUM-JS`，四轴 JS → DIR → PARAM → JSAPI。✅
3. **IP-web 纳入范围** = 与域名同等，纳入全套四类（不是只 JS）。✅
4. **IP 是否 web** = `targets.http_status` 非空判定；无 http 维持不适用。✅
5. **流程** = 设计文档 → 计划 → TDD + sign-off；harness 核心 + gate BLOCKING 走灰度 + parity 测。✅

> 下一步：以本设计为准用 writing-plans 出 `docs/superpowers/plans/2026-07-01-enumeration-four-axis-and-ip-web.md`（按 §10 两个 PR 逐个 TDD 落地）。本设计独立新增，不覆盖旧文档（I6）。
