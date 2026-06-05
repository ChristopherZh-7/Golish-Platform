# 2026-06-05 · Gate 规则引擎（固定菜单 → 数据驱动 `gate_rules`）

> 把每阶段的「过关标准」从**写死在 Rust `match` 里的固定菜单**（`required_checks` 字符串只能命中预实现的几个 check，未命中的名字被 `_ => continue` 静默吞掉），升级成**一套可在 stage JSON 里用积木拼出的声明式规则** `gate_rules`，由一个通用、纯函数、可单测、DB-free 的解释器执行。新增过关标准 = 改 JSON，**零 Rust 改动**（除非要一块全新积木）。
>
> 关联：`gate/mod.rs::validate_stage_gate_with_skeleton`（调度入口）、`gate/finding_verification_check.rs`（已有的「配置驱动」原型，本设计的范本）、`stage_spec.rs::StageSpec`（加 `gate_rules` 字段）、`resources/harness/stages/*.json`（声明规则）、`types.rs::StageDeliverable`（规则求值的输入 contract）。
>
> 本文档是「第 2 步」设计；「第 1 步」（未知 `required_checks` fail-closed + 故意空跑白名单）是独立的便宜安全网，见 §9。
>
> **后续（2026-06-05 · B 彻底版）**：用户选择彻底迁移——`required_checks` 固定菜单已被 `docs/design/2026-06-05-gate-rules-migration.md` **整个删除**，`gate_rules` 成为过关标准唯一入口（新增 `named_check` 逃生舱承接 scope/surface_coverage/min_invocations）。本文中「与 `required_checks` 并存 / 渐进迁移」的表述已被该迁移取代。

---

## 1. 背景与问题（实测根因）

当前 `validate_stage_gate_with_skeleton`（`gate/mod.rs:107-143`）的判定分两层：

1. **结构性 check** 永远跑（与 stage 语义无关）：`schema` / `contract` / `vacuous` / `freshness` / `finding_verification`。
2. **语义 check** 按 `spec.required_checks` 字符串**选跑**——但映射写死在一个 `match`：

```rust
let check_id = match name.as_str() {
    "scope_status_present" | "out_of_scope_targets_excluded" => "scope",
    "surface_workbench_coverage" => "surface_coverage",
    "min_tool_invocations_per_check" => "min_invocations",
    _ => continue,            // ← 未知 check 名被静默忽略
};
```

**三个真实后果**：

- **静默失效**：`target_intel.json` / `external_attack_surface.json` 的 `required_checks` 里都写了 `evidence_non_empty`、`unchecked_distinct_from_checked_empty`——这俩在 `match` 里没有分支，全走 `_ => continue`。它们恰好被 `scope` / `vacuous` 覆盖所以"碰巧没出事"，但任何人写一个**真的**新 check 名（如 `coverage_matrix`），同样被静默吞掉，**写了等于没写**。
- **加标准必须改 Rust**：要加一个新过关标准，得三步——① 写 check 函数 ② 在 `match` 加分支 ③ JSON 引用。攻击阶段要加几十种漏洞各自的合格标准，每个都卡在程序员身上，迭代极慢。
- **声明与实现漂移**：`min_invocations: dns_resolve:1` 这种"声明了但没接线"的字段已经出现过（spec 写了，gate 不强制）。固定菜单天然鼓励这种漂移。

> 注：`StageSpec.gate_validator` 字符串字段当前也是"声明但未接线"——`stage_spec.rs:63` 有它、单测断言它的值，但调度永远走 `validate_stage_gate_with_skeleton`，该字符串不参与任何分发。本设计不依赖也不修复它（正交）。

**已有的正确范本**：`finding_verification_check`（`finding_verification_check.rs`）已经是"配置驱动"——它纯读 `spec.finding_verification` / `min_findings` / `min_claims`，按 stage JSON 声明强制，**零 per-stage 硬编码**，且 DB-free 可单测。本设计是把这个范本**泛化**成一套通用积木。

---

## 2. 不变量（本设计不改）

- **I-A · 5 个结构性 check 永远跑**：`schema` / `contract` / `vacuous` / `freshness` / `finding_verification` 的触发与逻辑不变。`gate_rules` 是**附加**层，不替换它们。
- **I-B · gate 是确定性的**：规则求值必须是纯函数、无随机、无 IO；同输入同输出。绝不能因为"AI 自信说完成"而放行（沿用现有 deterministic gate 原则）。
- **I-C · DB-free 主路**：规则引擎 MVP **只读 `StageDeliverable`**，不查 EvidenceLedger（与 `finding_verification_check` 同款边界，保证可单测）。需要 ledger 的语义（证据 KIND、freshness age）维持现状 caller-side 强制（`execute.rs::enforce_evidence_kinds`），见 §6。
- **I-D · "已检查为空" ≠ "未检查"**（AGENTS.md I8）：规则可以引用 `skipped_checks` 作为"显式声明空"的证明，但默认缺失 = 未检查 = 不通过。
- **I-E · 向后兼容**：`gate_rules` 缺省为空数组（`#[serde(default)]`）；不写 `gate_rules` 的 stage 行为与今天**逐字节一致**。`required_checks` 继续工作，两者并存。

---

## 3. 目标 / 非目标

**目标**
- G1：stage JSON 能用一组**积木 op** 声明过关标准，新增标准纯改 JSON。
- G2：未知 op / 未知谓词 / 字段不匹配 → **fail-closed**（报错或 Block），绝不静默忽略（治 §1 的核心病）。
- G3：规则引擎是**纯函数 + 全单测**，输出复用现有 `GateCheckOutcome` / `HarnessRecoveryActions`，无缝并进现有聚合。
- G4：能用 `gate_rules` **复刻**现有声明式 check（`finding_verification` 的 deliverable 半、scope 的 evidence-非空、min_findings/min_claims），证明积木够用；并能表达攻击阶段示例（"每个 finding 必须挂证据"、"至少 N 个某类 finding"）。
- G5：**渐进迁移**——`gate_rules` 与 `required_checks` 并存，逐个 stage 搬，不要求一次性重写全部 12 个 spec。

**非目标**
- 不动 `required_checks` 现有 3 个内建语义 check（`scope` / `surface_coverage` / `min_invocations`）的实现——它们要么含领域逻辑（surface_coverage 查 `surface_mapping`）、要么依赖外部口径，暂留为"内建自定义 op"，不强行塞进通用积木（YAGNI）。
- 不引入脚本语言 / 表达式求值器（Rhai/Lua/CEL）——过度工程、引入沙箱与安全面，违背"确定性 + 可审计"。见 §4 备选。
- 不接 ledger KIND / freshness age 进规则引擎主路（MVP 范围内保持 caller-side），仅预留扩展点。
- 不碰 `gate_validator` 字符串字段（正交的历史遗留）。

---

## 4. 备选方案与取舍

| 方案 | 做法 | 取舍 | 结论 |
|---|---|---|---|
| **A 维持固定菜单** | 每个新 check 写 Rust + 加 match 分支 | 类型安全但每条标准都卡程序员；§1 的病不治 | ✗ 否决（就是现状） |
| **B 声明式规则引擎（本设计）** | JSON 用固定 op 集拼规则，通用解释器执行 | 新标准纯改 JSON；op 是 typed enum → fail-closed；积木有限但覆盖 80% 常见标准 | ✓ **采用** |
| **C 嵌脚本语言** | stage JSON 里写 Rhai/CEL 表达式 | 最灵活，但引入求值器 + 沙箱 + 安全审计；非确定性风险；难单测 | ✗ 否决（违背 I-B / 可审计） |

方案 B 的关键洞察：渗透 gate 的标准 95% 是"**对某集合做存在/计数/全称判断 + 字段比较**"，这是有限积木能覆盖的，不需要图灵完备。

---

## 5. 规则 DSL（积木集）

### 5.1 顶层规则 = `gate_rules: [GateRule]`

每条 `GateRule` 是一个**内部 tagged** 的 op（serde `#[serde(tag = "op")]`），求值产出**一个** `GateCheckOutcome`（Pass 或 Block）。MVP 两个顶层 op，覆盖"存在/计数"与"全称"：

```jsonc
// op 1 · count_at_least：满足 where 的元素至少 min 个
{
  "op": "count_at_least",
  "over": "findings",                                  // MVP 集合：claims | findings
  "where": { "pred": "eq", "field": "kind", "value": "subdomain" },  // 可选；省略=全体
  "min": 1,
  "on_fail": { "reason": "至少需要 1 个 subdomain finding" }
}

// op 2 · for_all：满足 where 的每个元素都必须满足 require
{
  "op": "for_all",
  "over": "findings",
  "where": { "pred": "severity_at_least", "min": "high" },  // 可选过滤
  "require": { "pred": "non_empty", "field": "evidence_refs" },
  "on_fail": {
    "reason": "每个 high+ finding 必须挂证据，未验证结论不过关",
    "missing_evidence_kinds": ["poc", "exploit_verified"]
  }
}
```

> `exists` = `count_at_least` 且 `min:1` 的语义糖；按 YAGNI **不单列 op**，作者写 `min:1` 即可（文档给别名提示）。

### 5.2 叶子谓词 `Pred`（对单个元素求值，内部 tagged `#[serde(tag = "pred")]`）

| `pred` | 字段 | 含义 | 适用集合 |
|---|---|---|---|
| `non_empty` | `field` | 该字段（数组/字符串）非空 | 全部 |
| `eq` | `field`, `value` | 该字段字符串等于 value | 全部 |
| `severity_at_least` | `min` | finding.severity rank ≥ min（用 `FindingSeverity::rank`） | findings |

`field` 是 typed enum `ItemField`：`kind` / `subject` / `summary` / `evidence_refs` / `evidence_ids` / `severity`。求值时若字段对当前集合不适用（如对 `claims` 取 `severity`）→ 返回 Block「rule references field `severity` not valid for collection `claims`」（运行时 fail-closed，不静默）。

### 5.3 `on_fail` → 直接映射现有恢复结构

```rust
struct OnFail {
    reason: String,                          // → GateCheckOutcome::Block.reasons
    #[serde(default)] hints: Vec<String>,    // → HarnessRecoveryActions.hints
    #[serde(default)] repair_tool_calls: Vec<String>,
    #[serde(default)] missing_evidence_kinds: Vec<String>,
}
```

### 5.4 Rust 类型（新增 `gate/rule_engine.rs`，纯函数 + 单测）

```rust
use serde::{Deserialize, Serialize};
use crate::harness::types::{FindingSeverity, HarnessRecoveryActions, StageDeliverable};
use super::GateCheckOutcome;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GateRule {
    CountAtLeast {
        over: Collection,
        #[serde(default, rename = "where")] filter: Option<Pred>,
        min: u32,
        on_fail: OnFail,
    },
    ForAll {
        over: Collection,
        #[serde(default, rename = "where")] filter: Option<Pred>,
        require: Pred,
        on_fail: OnFail,
    },
}

// MVP 只含有"可寻址字段"的两个集合；evidence_refs / skipped_checks 的计数已被
// vacuous_check 覆盖，故不纳入 MVP（未来要加是 +2 个枚举臂的小改动）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection { Claims, Findings }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "pred", rename_all = "snake_case")]
pub enum Pred {
    NonEmpty { field: ItemField },
    Eq { field: ItemField, value: String },
    SeverityAtLeast { min: FindingSeverity },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemField { Kind, Subject, Summary, EvidenceRefs, EvidenceIds, Severity }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnFail {
    pub reason: String,
    #[serde(default)] pub hints: Vec<String>,
    #[serde(default)] pub repair_tool_calls: Vec<String>,
    #[serde(default)] pub missing_evidence_kinds: Vec<String>,
}

/// 纯函数：逐条规则求值，每条产出一个 outcome。无 IO、无 DB、确定性。
pub fn eval(deliverable: &StageDeliverable, rules: &[GateRule]) -> Vec<GateCheckOutcome> {
    rules.iter().map(|r| eval_one(deliverable, r)).collect()
}
```

**fail-closed 由类型保证**：`op` / `pred` / `over` / `field` 全是 serde enum。stage JSON 写错名字（`"op":"coverage_matrix"`）→ `serde_json::from_str` 解析 `StageSpec` 时直接报错 → stage spec **加载即失败**，被现有 `resources.rs::all_twelve_stage_specs_load` 单测当场抓住。**这正是第 1 步想要的"未知 = 报错"，在第 2 步里由类型系统天然达成**，且比 runtime 更早暴露。

### 5.5 接入 `validate_stage_gate_with_skeleton`（仅 +1 行）

```rust
// gate/mod.rs，现有 required_checks 循环之后、aggregate 之前：
outcomes.extend(rule_engine::eval(deliverable, &spec.gate_rules));
aggregate(outcomes)
```

`StageSpec` 加字段：

```rust
#[serde(default)]
pub gate_rules: Vec<rule_engine::GateRule>,
```

---

## 6. 用 `gate_rules` 复刻 / 表达现有标准（证明积木够用 · G4）

| 现有/目标标准 | 今天怎么实现 | `gate_rules` 写法 |
|---|---|---|
| 每个 claim 有证据（scope 的一半） | `scope_check` 硬编码 | `for_all over claims require {non_empty evidence_ids}` |
| 每个 finding 有证据 | `scope_check` 硬编码 | `for_all over findings require {non_empty evidence_refs}` |
| high+ finding 必须有证据 | `finding_verification_check` 读 spec | `for_all over findings where {severity_at_least high} require {non_empty evidence_refs}` |
| 至少 N 条 finding / claim | `min_findings` / `min_claims` | `count_at_least over findings min N` / `over claims min N` |
| 至少 1 个 subdomain（攻击面） | 无（要 Rust） | `count_at_least over findings where {eq kind subdomain} min 1` |
| 每个漏洞挂"已验证"证据（攻击阶段目标） | 无（要 Rust + ledger） | **见下：MVP 部分覆盖** |

**领域 / ledger 边界（明确不在 MVP 主路）**：

- `surface_coverage`（查 `surface_mapping` 的必覆盖类目）含领域逻辑，**保留为内建 check**，由 `required_checks` 继续驱动；不强塞进通用积木。
- **证据 KIND**（"证据必须是 poc / exploit_verified 类"）：deliverable 只有 evidence ID，没有 kind。维持现状——KIND 校验 caller-side（`execute.rs::enforce_evidence_kinds` 查 ledger）。规则引擎只能表达"有/没有证据 ID 挂上"这一 deliverable 层。
- **扩展点（Phase 2，不在本设计实现）**：`eval` 可重载为 `eval_with_context(deliverable, rules, ctx)`，`ctx: { evidence_kinds: HashMap<EvidenceAuditId, Vec<String>> }` 由 caller 预先查 ledger 填好，再新增一个 `evidence_of_kind` 谓词。这样 KIND 判断也能进 JSON，且仍保持 `eval` 纯函数（context 作为入参注入，不在引擎内做 IO）。本设计只**预留**，不实现。

---

## 7. 集成点（文件级落点）

| # | 改动 | 位置 |
|---|---|---|
| 7.1 | 新增 `GateRule` / `Pred` / `Collection` / `ItemField` / `OnFail` + `eval` + 单测 | 新增 `golish-agent-kit/src/harness/gate/rule_engine.rs` |
| 7.2 | `pub mod rule_engine;` + `outcomes.extend(rule_engine::eval(...))` | `gate/mod.rs`（模块声明 + `validate_stage_gate_with_skeleton` 内 +1 行） |
| 7.3 | `StageSpec` 加 `#[serde(default)] gate_rules: Vec<GateRule>` | `stage_spec.rs` |
| 7.4 | （可选·验证用）给 1 个 stage（建议 `verification.json`）补一条样例 `gate_rules` 复刻其 finding_verification | `resources/harness/stages/verification.json` |
| 7.5 | 文档：DSL 速查（op / pred / field 表）追加到 stage-spec reference | `docs/design/2026-06-02-harness-stage-spec-reference.md` 末尾补一节 |

依赖方向：`rule_engine` 只依赖 `harness::types` + `gate::GateCheckOutcome`，与现有 check 同层，无新 crate 依赖、无环。

---

## 8. 边界与风险

- **积木覆盖不全**：MVP 只有 `count_at_least` / `for_all` + 3 个谓词。无 `and/or` 组合、无跨集合关联、无 KIND/age。缓解：覆盖当前 80% 标准；缺的按需加积木（每加一块是一次小、隔离、带单测的改动），不是推倒重来。
- **fail-closed 太"硬"**：一条规则 JSON 写错 → 整个 stage spec 加载失败。这是**刻意**的（治静默失效），且被 `all_twelve_stage_specs_load` 单测兜底，开发期当场暴露，不会漏到线上。风险：误把生产 spec 写挂——缓解：迁移时一次只动一个 stage + 跑 `just test-harness`。
- **双轨期认知负担**：`required_checks`（内建语义 check）与 `gate_rules`（声明式）并存。缓解：文档明确分工——领域/外部口径 check 留 `required_checks`；可声明的标准走 `gate_rules`；不要求强迁。
- **字段-集合不匹配**：靠 runtime Block 兜底（非 panic），但属"作者错误"。可选增强：加一个 `gate_rules_well_formed` 单测，对所有内嵌 spec 跑一遍 `eval`（空 deliverable）确保不出现"字段不适用"类 Block。
- **风险等级**：**中**。属 gate 核心链路（AGENTS.md I7 / 收口规则）。落地必须 TDD + `just test-harness` + clippy 全绿 + `just precommit`；按 AGENTS.md §2.5「安全/核心链路」先设计（本文件）后实现。

---

## 9. 与「第 1 步」的关系

- **第 1 步**（未知 `required_checks` fail-closed + `evidence_non_empty`/`unchecked_distinct_from_checked_empty` 白名单）：改 `gate/mod.rs` 那个 `match` 的 `_ => continue`，是**便宜、低风险**的安全网，**先做也行、不做也不阻塞第 2 步**。
- **第 2 步**（本设计）：`gate_rules` 是**新字段、typed enum**，其 fail-closed 由 serde 在 spec 加载期天然达成（§5.4），**不经过**第 1 步那个 `match`。
- 两步**正交可独立推进**。建议：若想快速堵住"静默失效"风险，先做第 1 步（半天）；本设计（第 2 步）随后按计划实现。第 2 步落地后，老的 `required_checks` 静默忽略问题随着各 stage 逐步迁移到 `gate_rules` 而自然收敛。

---

## 10. 验证计划（证据优先）

1. **单测（`rule_engine.rs`）**：
   - `count_at_least`：min 命中/未命中、有/无 `where` 过滤、空集合。
   - `for_all`：全满足 Pass、有一个不满足 Block、`where` 过滤后为空集合时 Pass（全称空真）。
   - 谓词：`non_empty`（空/非空）、`eq`（命中/不命中）、`severity_at_least`（边界 rank）。
   - fail-closed：字段-集合不匹配返回 Block 且 reason 明确。
   - serde：未知 `op` / 未知 `pred` → `from_str` Err（spec 加载失败）。
2. **复刻等价性单测**：构造一个 deliverable，分别用 `finding_verification`（现状）与等价 `gate_rules` 跑，断言 Block/Pass 结论一致（证明积木确实复刻了现有标准）。
3. **回归**：`resources.rs::all_twelve_stage_specs_load` 仍全绿（含 7.4 新增样例规则的 stage）。
4. **门禁**：`just test-harness` + `cargo clippy`（零 warning）+ `just precommit` 全绿后才可标 `passing`。
5. **证据留痕**：上述命令输出 + 退出码复制进 `agent-progress.md`「已记录证据」段（AGENTS.md §3）。

---

## 11. 落地次序（供后续 writing-plans 展开）

> 本节是计划骨架，正式实现计划按 `.cursor/skills/writing-plans` 落到 `docs/superpowers/plans/2026-06-05-gate-rule-engine.md`。

1. 新增 `rule_engine.rs`：先写谓词 + `eval_one` 的失败测试（TDD），再实现。
2. 加 `count_at_least` / `for_all` 求值 + 全分支单测。
3. `StageSpec` 加 `gate_rules` 字段（`#[serde(default)]`），跑 `all_twelve_stage_specs_load` 确认零破坏。
4. `gate/mod.rs` 接 `eval`（+1 行），加一个集成测试：带 `gate_rules` 的 spec 能 Block/Pass。
5. 给 `verification.json` 补一条样例 `gate_rules`，写复刻等价性测试。
6. 文档：stage-spec reference 补 DSL 速查表。
7. `just test-harness` + `just precommit` 全绿，留证据，更新 progress + feature_list。
