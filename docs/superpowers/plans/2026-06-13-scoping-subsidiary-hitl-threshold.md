# Scoping 子公司范围人机确认（HITL 投资比阈值）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 scoping 阶段在发现子公司前**回来问人**——是否纳入子公司、投资比阈值多少、是否含分公司——并让人给的阈值真正驱动「自动晋升为子组织」的门槛，而不是写死的 51%。

**架构：** 三块联动。① `recon_discover_subsidiaries` 工具 schema 增可选 `min_ownership_percent` / `include_branches` 参数；② `run_phase` 解析这些参数并构造 `AssetIntelHydrateConfig`（不再恒为 default）；③ `run_passive_intel` 用运行时阈值**覆盖** discovery policy 的 `promote_when` scale 门槛，再促晋升。④ scoping 方法论 prompt 增加 discover 前的 `ask_human` 前置询问（不纳入→跳过并记 checked-empty）。

**技术栈：** Rust（golish-recon-app `agent_tools` / `asset_intel`），harness methodology markdown，cargo nextest。

---

## 背景与现状（动手前必读）

- 现状数据流：`ReconDiscoverSubsidiariesTool.parameters` → `passive_intel_parameters()`（**只有 `organization_id`**）→ `run_phase(... )` 里恒传 `AssetIntelHydrateConfig::default()` → `run_passive_intel(agent_intel.rs)` → `select_discovery_policy()` 取 provider 配置里的 `promote_when`（`enscan-go.json` 现为 `[{scale gte 51}]`）→ `auto_promote_discovered_children(.., &policy)` → `auto_promote_child_decisions`（`promote.rs`，`filter_passes` 跑 `promote_when`）。
- 所以：agent **没有任何参数**能把人选的阈值传进去；promote 门槛只来自 config。光改 prompt 不够。
- 关联事实：刚把 `enscan-go.json` 的 `promote_when` 状态条删掉（只剩 `scale gte 51`）；`filter_passes`（`normalize.rs`）的 `gte` 会 `trim_end_matches('%')`，所以 scale `"100%"` 能正确比较。
- 不变量：AGENTS.md I2（IDOR，`run_phase` 已做）、I8（「已检查为空」≠「未检查」——不纳入子公司要记 checked-empty 而非静默跳过）。

### 关键类型（已存在，勿重定义）
- `AssetIntelHydrateConfig { min_ownership_percent: Option<String>, depth: Option<String>, include_branches: Option<bool>, create_candidates: Option<bool> }`（`golish-recon-app` 内，`agent_tools` 已 `use`）。
- `golish_pentest::models::AssetIntelDiscoveryConfig { auto_promote: bool, promote_when: Vec<AssetIntelNormalizeFilter>, ownership_field: String, dedupe_by: Vec<String> }`。
- `golish_pentest::models::AssetIntelNormalizeFilter { field: String, op: AssetIntelNormalizeFilterOp, value: String }`。
- `golish_pentest::models::AssetIntelNormalizeFilterOp`（含 `Gte`/`Gt`/`Eq`/`Contains`/…）。

---

## 文件结构

- `backend/crates/golish-recon-app/src/agent_tools/mod.rs` — 子公司工具 schema 增参数 + `run_phase` 解析并构造 config。
- `backend/crates/golish-recon-app/src/asset_intel/promote.rs` — 新增 `apply_ownership_threshold_override` 纯函数 + 单测。
- `backend/crates/golish-recon-app/src/asset_intel/mod.rs` — 导出 `apply_ownership_threshold_override`。
- `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs` — `run_passive_intel` 用运行时阈值覆盖 policy。
- `resources/harness/stages/scoping.methodology.md` — step 2 改为 discover 前置 `ask_human`。

---

## Task 1 — 子公司工具 schema 增 `min_ownership_percent` / `include_branches`

**文件：** `backend/crates/golish-recon-app/src/agent_tools/mod.rs`

**步骤：**

1. 在 `passive_intel_parameters` 函数下方新增子公司专用 schema（enrich 仍用旧的）：

```rust
/// JSON schema for `recon_discover_subsidiaries`. Adds the scope knobs the
/// scoping agent must ASK the human for (ownership threshold / branches) — see
/// scoping.methodology.md. Absent fields fall back to provider-config defaults.
fn subsidiary_intel_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "organization_id": {
                "type": "string",
                "description": "Organization UUID (the confirmed engagement subject to discover subsidiaries for). Create/select it first via manage_organizations."
            },
            "min_ownership_percent": {
                "type": "string",
                "description": "Ownership threshold (percent, no % sign), e.g. \"51\" or \"100\". A discovered subsidiary auto-promotes into an in-scope child org only when its ownership >= this value. ASK THE HUMAN for this during scoping and pass their answer. Omit to use the provider default (51)."
            },
            "include_branches": {
                "type": "boolean",
                "description": "Also collect branch offices (分公司). Default false. Ask the human whether branches are in scope."
            }
        },
        "required": ["organization_id"]
    })
}
```

2. 把 `ReconDiscoverSubsidiariesTool::parameters` 从 `passive_intel_parameters("to discover subsidiaries for")` 改为 `subsidiary_intel_parameters()`。

3. 更新 `ReconDiscoverSubsidiariesTool::description`，追加一句让 agent 先问人：在结尾加
   `" Before calling, ask the human (scoping) whether subsidiaries are in scope and at what ownership threshold; pass min_ownership_percent accordingly."`

**验证：**
```bash
cd backend && cargo build -p golish-recon-app 2>&1 | tail -5
```
预期：编译通过（0 error）。

**提交：** `feat(recon): add ownership-threshold/branches params to discover_subsidiaries tool schema`

---

## Task 2 — `run_phase` 解析参数并构造 `AssetIntelHydrateConfig`

**文件：** `backend/crates/golish-recon-app/src/agent_tools/mod.rs`

**步骤：**

1. 在 `run_phase` 里，IDOR 校验通过后、调用 `run_passive_intel` 前，解析可选参数（enrich 阶段这些字段不存在 → None，行为不变）：

```rust
    // Scope knobs (only the subsidiaries tool sends these; enrich omits them).
    let min_ownership_percent = args
        .get("min_ownership_percent")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let include_branches = args.get("include_branches").and_then(|v| v.as_bool());
    let config = AssetIntelHydrateConfig {
        min_ownership_percent,
        depth: None,
        include_branches,
        create_candidates: Some(true),
    };
```

2. 把 `run_passive_intel( Arc::clone(pool), tools.clone(), uid, phase, AssetIntelHydrateConfig::default(), )` 的最后一个实参 `AssetIntelHydrateConfig::default()` 改为 `config`。

**验证：**
```bash
cd backend && cargo build -p golish-recon-app 2>&1 | tail -5
```
预期：编译通过。

**提交：** `feat(recon): thread scope knobs from discover_subsidiaries args into hydrate config`

---

## Task 3 — `apply_ownership_threshold_override` 纯函数 + 单测

**文件：** `backend/crates/golish-recon-app/src/asset_intel/promote.rs`

**步骤：**

1. 在 `promote.rs` 顶部 `use` 区补充（若未引入）：
```rust
use golish_pentest::models::{
    AssetIntelDiscoveryConfig, AssetIntelNormalizeFilter, AssetIntelNormalizeFilterOp,
};
```
（注意：现有代码已用 `golish_pentest::models::AssetIntelDiscoveryConfig` 全路径；保持风格一致也可不加 use，直接全路径。二选一，保证编译。）

2. 新增纯函数（放在 `select_discovery_policy` 附近）：

```rust
/// Override the discovery policy's ownership threshold with a runtime value
/// (the human-chosen `min_ownership_percent` from scoping). Sets the value on
/// the existing `ownership_field` gte/gt/eq clause, or appends a `gte` clause
/// when none exists. Empty/whitespace threshold is a no-op (keeps config
/// default). Does NOT touch the status/other clauses.
pub(crate) fn apply_ownership_threshold_override(
    policy: &mut golish_pentest::models::AssetIntelDiscoveryConfig,
    threshold: &str,
) {
    use golish_pentest::models::AssetIntelNormalizeFilterOp as Op;
    let t = threshold.trim();
    if t.is_empty() {
        return;
    }
    let field = policy.ownership_field.clone();
    if let Some(clause) = policy
        .promote_when
        .iter_mut()
        .find(|c| c.field == field && matches!(c.op, Op::Gte | Op::Gt | Op::Eq))
    {
        clause.value = t.to_string();
    } else {
        policy
            .promote_when
            .push(golish_pentest::models::AssetIntelNormalizeFilter {
                field,
                op: Op::Gte,
                value: t.to_string(),
            });
    }
}
```

3. 在 `promote.rs` 的 `#[cfg(test)] mod policy_tests` 内追加两个测试：

```rust
    #[test]
    fn override_replaces_existing_ownership_gte_value() {
        use golish_pentest::models::{
            AssetIntelDiscoveryConfig, AssetIntelNormalizeFilter, AssetIntelNormalizeFilterOp,
        };
        let mut policy = AssetIntelDiscoveryConfig {
            auto_promote: true,
            ownership_field: "scale".into(),
            promote_when: vec![AssetIntelNormalizeFilter {
                field: "scale".into(),
                op: AssetIntelNormalizeFilterOp::Gte,
                value: "51".into(),
            }],
            ..Default::default()
        };
        super::apply_ownership_threshold_override(&mut policy, "100");
        assert_eq!(policy.promote_when.len(), 1);
        assert_eq!(policy.promote_when[0].value, "100");
    }

    #[test]
    fn override_appends_clause_when_missing_and_noops_on_empty() {
        use golish_pentest::models::AssetIntelDiscoveryConfig;
        let mut policy = AssetIntelDiscoveryConfig {
            auto_promote: true,
            ownership_field: "scale".into(),
            promote_when: vec![],
            ..Default::default()
        };
        super::apply_ownership_threshold_override(&mut policy, "  ");
        assert!(policy.promote_when.is_empty(), "blank threshold is a no-op");
        super::apply_ownership_threshold_override(&mut policy, "51");
        assert_eq!(policy.promote_when.len(), 1);
        assert_eq!(policy.promote_when[0].field, "scale");
        assert_eq!(policy.promote_when[0].value, "51");
    }
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-recon-app -E 'test(/override_/)' 2>&1 | tail -15
```
预期：2 passed。

**提交：** `feat(recon): add apply_ownership_threshold_override pure fn + tests`

---

## Task 4 — 导出 + 在 `run_passive_intel` 应用阈值覆盖

**文件：** `backend/crates/golish-recon-app/src/asset_intel/mod.rs`、`backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`

**步骤：**

1. `asset_intel/mod.rs`：把 `apply_ownership_threshold_override` 加入 promote 的再导出：
```rust
pub(crate) use promote::{
    apply_ownership_threshold_override, auto_promote_child_decisions,
    clear_engagement_candidates_from_intel, select_discovery_policy,
};
```

2. `agent_intel.rs`：在 `use super::{ ... }` 列表加入 `apply_ownership_threshold_override`。

3. `agent_intel.rs` `run_passive_intel`：把 discovery_policy 的构造改成「先选 policy，再按 config 阈值覆盖」：

```rust
    let discovery_policy = (phase == PassiveIntelPhase::Subsidiaries).then(|| {
        let mut policy = select_discovery_policy(
            selected
                .iter()
                .filter_map(|tool| tool.asset_intel.as_ref())
                .map(|asset| &asset.discovery),
        );
        if let Some(threshold) = config.min_ownership_percent.as_deref() {
            apply_ownership_threshold_override(&mut policy, threshold);
        }
        policy
    });
```

   （`config` 此前已是 `run_passive_intel` 的入参；确认闭包能捕获 `config` 的引用——它在函数作用域内，OK。）

**验证：**
```bash
cd backend && cargo build -p golish-recon-app 2>&1 | tail -5 && \
cargo nextest run -p golish-recon-app -E 'test(/asset_intel/) + test(/override_/)' 2>&1 | tail -20
```
预期：编译通过；asset_intel + override 全部 pass。

**提交：** `feat(recon): apply human ownership threshold to subsidiary auto-promotion`

---

## Task 5 — scoping 方法论：discover 前置人机询问

**文件：** `resources/harness/stages/scoping.methodology.md`

**步骤：** 把当前 step 2 整段替换为「先问人，再按答案决定」：

```markdown
2. **Subsidiaries are a SCOPE decision — ask before you discover.** Before
   calling `recon_discover_subsidiaries`, ask the human with `ask_human`
   (`input_type="choice"`, options like ["不纳入子公司", "纳入：≥51% 控股", "纳入：≥100% 全资", "纳入：自定义比例"]):
   whether subsidiaries/holdings are in scope and at what ownership threshold
   (and whether branch offices 分公司 are included).
   - **Not in scope →** skip discovery; record subsidiaries as checked-empty +
     evidence (NOT unchecked). Do NOT fabricate a tree.
   - **In scope →** call `recon_discover_subsidiaries` with
     `min_ownership_percent` set to the human's threshold (and
     `include_branches: true` if they asked for branches). Holdings at or above
     the threshold auto-promote into child orgs; the rest stay as candidates for
     the step-4 human review. Found none clearing the threshold? checked-empty +
     evidence — never invent a subsidiary.
```

**验证：**
```bash
rg -n "ask_human|min_ownership_percent|checked-empty" resources/harness/stages/scoping.methodology.md | head
```
预期：能看到新增的 `ask_human` / `min_ownership_percent` / `checked-empty` 行。

**提交：** `docs(harness): scoping asks human for subsidiary scope + ownership threshold`

---

## Task 6 — 全量验证 + 收口

**步骤：**
```bash
cd backend && cargo build ./... 2>&1 | tail -5
cargo nextest run -p golish-recon-app -p golish-pentest-domain 2>&1 | tail -25
cargo clippy -p golish-recon-app -q -- -D warnings 2>&1 | tail -10
cargo fmt -p golish-recon-app
```
预期：build 0、nextest 全绿、clippy 0 warning、fmt 干净。

可选端到端（需在 Golish 里跑 scoping）：「搞一下平安」→ 期望 agent 先弹「要不要纳入子公司 / 阈值」选择，选「≥51%」后再 discover，子公司按 51% 晋升。

**提交：** 若上面任务已分提交，则此步仅跑验证；如需统一收口可 `chore(recon): scoping subsidiary HITL threshold — verification`。

---

## 自检

1. **规格覆盖度**：① prompt 前置询问→Task 5；② 工具阈值参数→Task 1/2；③ promote 阈值生效→Task 3/4。「不纳入子公司」分支→Task 5（checked-empty）。✅
2. **占位符扫描**：无 TODO/待定；每个 code step 含完整代码。✅
3. **类型一致性**：`apply_ownership_threshold_override(&mut AssetIntelDiscoveryConfig, &str)` 在 Task 3 定义、Task 4 调用一致；`AssetIntelHydrateConfig` 字段名与既有 struct 一致（min_ownership_percent/depth/include_branches/create_candidates）。✅

## 注意 / 权衡
- 本计划只让阈值驱动 **promote**（晋升），不另设 `-invest` 抓取过滤；若希望 enscan 抓取阶段也按阈值收窄（减少候选），可在 Task 2 同时让 `min_ownership_percent` 经 `arg_bindings` 渲染 `-invest`（已有 binding），但会让低于阈值的子公司不出现在候选里。默认保留全部候选供人工复核。
- 「活跃状态」语义（排除 注销/吊销）是上一轮选的 B 方案遗留的精细化项（option C），不在本计划内；如需另开计划。
