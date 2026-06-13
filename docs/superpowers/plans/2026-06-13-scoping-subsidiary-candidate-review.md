# Scoping 子公司「候选→人工勾选→再建」（关闭 auto_promote）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** scoping 阶段不再自动把子公司建成子组织；改成「发现 → 出候选（含投资比/状态）→ `ask_human(unit_review)` 把候选列表展示给人勾选 → 只建人选中的」。母公司（root）仍按现状先建（已经过 lookup + 选规范名人工确认），不变。

**架构：** ① `enscan-go.json` 关掉 `discovery.auto_promote`，让 discover 只产候选不建子组织；② discover 工具输出新增 `subsidiaries` 候选列表（name + ownership_percent + status + meets_threshold，meets_threshold 用人给的阈值在后端算）；③ scoping methodology 重排 step 2-5：把候选（达标的预选）作为 JSON 数组传进 `ask_human(unit_review)` 的 `context`（顺带修复"占位符"bug）→ 人勾选 → 用 `manage_organizations(action="create", parent_id=root)` 逐个建选中的子公司。前端 unit_review 表无需改（候选名带投资比标签即可，MVP）。

**技术栈：** Rust（golish-recon-app asset_intel/agent_intel + promote），harness methodology markdown，JSON 配置，cargo nextest。

---

## 背景与现状（动手前必读）

- `run_passive_intel`（`agent_intel.rs`）当 `discovery_policy.auto_promote==true` 时调 `auto_promote_discovered_children` → **直接建子组织**，并把 `run.candidates` 清空 → `PassiveIntelSummary.organizations=0`、`promoted_children=N`。这就是"先建后审"。
- `PassiveIntelSummary`（`agent_intel.rs:47`）只有计数（`organizations/targets/promoted_children`），**没有候选名单**。
- 候选结构 `OrganizationCandidate`（`organizations/types.rs:52`）：`value`=名称、`evidence.raw.{scale,status,pid}` 含投资比/状态。
- `unit_review` UI（`ScopeReviewTable.tsx`）初始行来自 `ask_human` 的 `context`（JSON 数组）；context 空 → 占位符 "Acme Corp/Acme Subsidiary"。即"占位符 bug"=agent 没把候选传进 context。
- `manage_organizations`：`create` 支持 `parent_id`（建子公司用它）；`create_batch` 只建 root（parent_id=NULL）。
- 上一轮已加 `apply_ownership_threshold_override`（promote.rs）——它只在 auto_promote=true 时生效；本计划 auto_promote=false 后它对促晋升不再起作用，但**保留**（将阈值改为在 T2 里算 `meets_threshold` 用于候选预选；override 留作未来 auto 模式）。
- 不变量：I8（"不纳入子公司"=checked-empty 而非未检查；候选"跑了但没选"≠"没跑"）。

---

## 文件结构

- `resources/toolsconfig/enscan-go.json` — `discovery.auto_promote: false`。
- `backend/crates/golish-recon-app/src/asset_intel/promote.rs` — `parse_ownership_percent` 改 `pub(crate)`（复用做 meets_threshold）。
- `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs` — `PassiveIntelSummary` 增 `subsidiaries: Vec<SubsidiaryCandidate>`；新增 `SubsidiaryCandidate` 结构 + 填充逻辑 + 测试。
- `resources/harness/stages/scoping.methodology.md` — step 2-5 重排为「候选→勾选→建选中」。

---

## Task 1 — 关闭 auto_promote

**文件：** `resources/toolsconfig/enscan-go.json`

**步骤：** 把第一个 provider（`enscan-go`）的 `discovery.auto_promote` 由 `true` 改为 `false`：

```json
        "discovery": {
          "auto_promote": false,
          "auto_promote_note": "Scoping 改为候选→人工勾选→再建（不自动建子组织）；见 docs/superpowers/plans/2026-06-13-scoping-subsidiary-candidate-review.md",
          "dedupe_by": [ "pid", "name" ],
          "ownership_field": "scale",
          "promote_when": [ { "field": "scale", "op": "gte", "value": "51" } ]
        },
```
（注：`auto_promote_note` 若 schema 不允许未知键则去掉该行——`AssetIntelDiscoveryConfig` 用 serde 默认会忽略未知字段，安全；不放心就只改 `auto_promote`。）

**验证：**
```bash
cd /Users/christopherzheng/WebstormProjects/Golish-Platform && \
python3 -c "import json;d=json.load(open('resources/toolsconfig/enscan-go.json'));print('auto_promote=',d['tool']['asset_intel_providers'][0]['discovery']['auto_promote'])"
```
预期：`auto_promote= False`。

**提交：** `feat(scoping): stop auto-promoting subsidiaries — candidates go to human review`

---

## Task 2 — discover 输出暴露候选子公司列表（name+投资比+状态+是否达标）

**文件：** `backend/crates/golish-recon-app/src/asset_intel/promote.rs`、`backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`

**步骤：**

1. `promote.rs`：把 `parse_ownership_percent` 改成 `pub(crate)`（签名不变）：
```rust
pub(crate) fn parse_ownership_percent(raw: &str) -> Option<f64> {
```
并在 `asset_intel/mod.rs` 的 `pub(crate) use promote::{...}` 里追加 `parse_ownership_percent`。

2. `agent_intel.rs`：在 `PassiveIntelSummary` 上方新增候选结构：
```rust
/// One discovered subsidiary candidate surfaced for human review (auto_promote
/// is off — nothing is created until the user picks). `meets_threshold` is
/// computed against the human-chosen `min_ownership_percent` (default 51).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsidiaryCandidate {
    pub name: String,
    pub ownership_percent: Option<String>,
    pub status: Option<String>,
    pub meets_threshold: bool,
}
```

3. `PassiveIntelSummary` 增字段（放在 `promoted_children` 后）：
```rust
    /// Subsidiaries phase: the discovered candidates (auto_promote off) so the
    /// agent can pass them into `ask_human(unit_review)` for the user to pick.
    /// Empty for enrich.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsidiaries: Vec<SubsidiaryCandidate>,
```

4. 在 `run_passive_intel` 构造 `Ok(PassiveIntelSummary { ... })` 之前，计算候选列表（仅 subsidiaries 阶段、未促晋升时 `run.candidates.organizations` 才非空）：
```rust
    let threshold = config
        .min_ownership_percent
        .as_deref()
        .and_then(parse_ownership_percent)
        .unwrap_or(51.0);
    let subsidiaries: Vec<SubsidiaryCandidate> = run
        .candidates
        .organizations
        .iter()
        .map(|c| {
            let raw = c.evidence.get("raw");
            let ownership = raw
                .and_then(|r| r.get("scale"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let status = raw
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let meets_threshold = ownership
                .as_deref()
                .and_then(parse_ownership_percent)
                .is_some_and(|v| v >= threshold);
            SubsidiaryCandidate {
                name: c.value.trim().to_string(),
                ownership_percent: ownership,
                status,
                meets_threshold,
            }
        })
        .collect();
```
   然后在 `PassiveIntelSummary { ... }` 字面量里加 `subsidiaries,`。

5. 更新 `summary_serializes_with_camel_friendly_fields` 测试字面量补 `subsidiaries: vec![]`，并新增一个测试覆盖候选映射：
```rust
    #[test]
    fn subsidiary_candidate_serializes_camel_and_threshold() {
        let s = SubsidiaryCandidate {
            name: "平安银行".into(),
            ownership_percent: Some("58%".into()),
            status: Some("在营".into()),
            meets_threshold: true,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["name"], "平安银行");
        assert_eq!(v["ownershipPercent"], "58%");
        assert_eq!(v["meetsThreshold"], true);
    }
```

**验证：**
```bash
cd backend && cargo build -p golish-recon-app 2>&1 | tail -5 && \
cargo nextest run -p golish-recon-app -E 'test(/subsidiary_candidate/) + test(/summary_serializes/)' 2>&1 | tail -10
```
预期：build 0；2 passed。

**提交：** `feat(recon): surface subsidiary candidates (name/ownership/status/meetsThreshold) in discover output`

---

## Task 3 — methodology：候选→勾选→建选中

**文件：** `resources/harness/stages/scoping.methodology.md`

**步骤：** 把 step 2、step 3、step 4、step 5 调整为下述（root 仍在 step 5 建；子公司走候选→勾选→建）：

- step 2（已含 ask_human 询问是否纳入+阈值）末尾补一句：discover 现在**只产候选、不建子组织**；其输出的 `subsidiaries` 数组里 `meetsThreshold=true` 的就是达标候选。
- 新 step 3（替换原 propose）：
```markdown
3. **Show the discovered subsidiaries and let the user PICK — never auto-add them.**
   Take the `subsidiaries` array from the `recon_discover_subsidiaries` output and
   call `ask_human(input_type="unit_review", context=<JSON array>)` where context is
   the candidate list (pre-select the `meetsThreshold=true` ones; put ownership in the
   name label, e.g. `{"name":"平安银行股份有限公司 (58%)"}`). The user confirms/edits
   which subsidiaries are in scope. NEVER pass an empty context — if you have
   candidates, they MUST appear in the review table.
```
- step 4 改为「针对 root（A 模式无子公司时）或纯目标场景的单次 review」保留；子公司 review 已在 step 3。（若 root 已确认则不重复 review。）
- step 5：建组织时——root 用 `create` / `create_batch`（canonical 名）；**用户在 step 3 选中的子公司**用 `manage_organizations(action="create", name=<picked>, parent_id=<root org id>)` 逐个建（create_batch 只建 root，子公司必须带 parent_id 用 create）。只建用户选中的；没选的留作候选/checked-empty，不建。

**验证：**
```bash
rg -n "subsidiaries|unit_review|parent_id|meetsThreshold|PICK" resources/harness/stages/scoping.methodology.md | head
```
预期：能看到新增的候选→勾选→建选中相关行。

**提交：** `docs(harness): scoping shows subsidiary candidates for human pick, creates only chosen`

---

## Task 4 — 全量验证 + 收口

**步骤：**
```bash
cd backend && cargo build ./... 2>&1 | tail -5
cargo nextest run -p golish-recon-app -p golish-pentest-domain 2>&1 | tail -25
cargo clippy -p golish-recon-app -q -- -D warnings 2>&1 | tail -10
cargo fmt -p golish-recon-app
```
预期：build 0、nextest 全绿（注意：若有 fixture 断言 `auto_promote==true`，改成 false 并说明）、clippy 0、fmt 干净。

端到端（需重启/`just dev` 让 Rust 生效；methodology+json 运行时即读）：「搞一下平安」→ agent 问是否纳入+阈值 → discover 出候选 → **unit_review 表里列出那些子公司（达标预选）给你勾** → 你确认 → 只建你选的（挂在中国平安下）。

**提交：** 若各任务已分提交，此步只跑验证。

---

## 自检

1. **规格覆盖**：① 不自动建→Task 1；② 候选列表出到 discover 输出→Task 2；③ 候选进 unit_review 给人勾 + 只建选中→Task 3；④ 母公司先建不变→Task 3 step 5 root 路径保留。✅
2. **占位符 bug**：Task 3 强制把候选作为 JSON 数组传 context → 表格不再空。✅
3. **占位符扫描**：无 TODO；code step 均带完整代码。✅
4. **类型一致**：`SubsidiaryCandidate{name,ownership_percent,status,meets_threshold}` 定义(Task2)与序列化测试一致；`parse_ownership_percent` pub(crate) 复用一致。✅

## 注意 / 权衡
- 前端 `ScopeReviewTable` unit_review 列仍是 name/aliases/domains；本计划把投资比放进 name 标签（MVP，不改前端）。若要独立"投资比"列，另开小改（ScopeReviewTable COLUMNS + normalizeScopeRows）。
- `apply_ownership_threshold_override`（上一轮）在 auto_promote=false 下不参与促晋升，保留以备未来切回自动模式；阈值在本计划改由 `meetsThreshold` 驱动候选预选。
- 子公司建库走 `create`+`parent_id` 逐个；数量大（>几十）时可另加 `create_batch` 的 parent_id 支持，当前 YAGNI。
