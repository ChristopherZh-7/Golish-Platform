# 2026-06-05 · Gate 彻底数据驱动化（删 `required_checks` 固定菜单 → 单一 `gate_rules` 入口）

> **承接** `docs/design/2026-06-05-gate-rule-engine.md`（已实现 commit `d02dbb46`：新增 `gate_rules` 数据驱动层，与旧 `required_checks` 固定菜单**并存**）。本设计是用户拍板的「**第 2 步·彻底版（B）**」：把旧 `required_checks` 固定菜单 + `gate/mod.rs` 里那个 `_ => continue` 静默忽略的 `match` **整个删掉**，让 `gate_rules` 成为 stage 过关标准的**唯一声明入口**。
>
> 关联代码：`gate/mod.rs`（删 match + 改 eval 入参）、`gate/rule_engine.rs`（加 `named_check` 积木）、`stage_spec.rs`（删 `required_checks` 字段）、`gate/{scope_check,surface_coverage_check,min_invocations_check}.rs`（保留逻辑、改为经 named_check 调用）、`resources/harness/stages/*.json`（12 份全部迁移）。

---

## 1. 背景：为什么要「彻底删旧」

第 1 步设计（`2026-06-05-gate-rule-engine.md`）刻意选择**并存渐进迁移**：`gate_rules`（新、fail-closed）与 `required_checks`（旧、`_ => continue` 静默忽略）两套同时存在。这留下两个问题：

- **两套机制、两种心智**：作者要分辨「这个标准该写 `required_checks` 还是 `gate_rules`」。
- **旧路仍是静默失效雷区**：`gate/mod.rs:125-141` 的 `match` 对未知 check 名 `_ => continue`——写错/写一个没实现的名字，gate 当没看见。新 `gate_rules` 已 fail-closed，但旧路没堵。

用户决策：**B（彻底迁移删旧）**。终局 = 一个机制（`gate_rules`）+ 一个 fail-closed 入口；旧 `required_checks` 字段、那个 `match`、以及静默忽略的坑全部消失。

---

## 2. 调查结论（已逐文件核实，证据见 §11）

### 2.1 旧 `required_checks` 实际只驱动 3 个语义 check

| check（match 分支） | 触发名 | 实际逻辑 | 数据来源 | 用的 stage |
|---|---|---|---|---|
| `scope_check` | `scope_status_present` / `out_of_scope_targets_excluded` | **只做**「每个 claim 有非空 `evidence_ids` + 每个 finding 有非空 `evidence_refs`」；**`scoping` 阶段豁免**（authz-only，无扫描）。真实 scope-label 检查（Task 1c.5）**至今未实现** | 仅 deliverable | 除 reporting 外几乎全部 |
| `surface_coverage_check` | `surface_workbench_coverage` | 领域逻辑：用 `surface_mapping::from_kind`（关键词匹配）把 claim/finding 归类成 Surface/JsApi/Sitemap/…，硬要求 Surface+JsApi 覆盖，Sitemap 软要求（查 `skipped_checks`） | 仅 deliverable | **仅** external_attack_surface |
| `min_invocations_check` | `min_tool_invocations_per_check` | **弱 MVP**：对 `spec.min_invocations` 的每个工具名，检查它是否作为子串出现在 `deliverable.required_checks_done`（agent 自填）；**忽略 count 值**（只判存在） | deliverable + `spec.min_invocations` | **仅** external_attack_surface、enumeration |

### 2.2 两个触发名是「静默空跑」

`evidence_non_empty`、`unchecked_distinct_from_checked_empty` 出现在多份 spec 的 `required_checks` 里，但 `match` **无对应分支** → 全走 `_ => continue`。它们的语义已被 `schema`/`vacuous` 结构 check 覆盖，所以「碰巧没出事」。迁移时**直接丢弃**（不再保留这两个名字）。

### 2.3 交付物只有一种形状

`ExternalAttackSurfaceDeliverable` 是 `StageDeliverable` 的**类型别名**（`types.rs:175`）。所以全 gate 只有一个 deliverable contract，无类型转换。字段：`stage_id / stage_run_id / claims[] / evidence_refs[] / skipped_checks[] / findings[] / required_checks_done[]`。

### 2.4 12 份 spec 的 `required_checks` 全量清单

| stage | required_checks（去掉空跑名后的有效项） |
|---|---|
| scoping | `scope_status_present`（但 scoping 豁免 → **等于无**） |
| target_intel | scope |
| external_attack_surface | scope + **surface_coverage** + **min_invocations** |
| enumeration | scope + **min_invocations** |
| vuln_triage | scope |
| verification | scope |
| access_validation | scope |
| internal_discovery | scope |
| objective_pathing | scope |
| objective_simulation | scope |
| cleanup | scope |
| reporting | （只有 evidence_non_empty 空跑）→ **无有效语义 check** |

### 2.5 fail-closed 安全网已存在

`resources.rs::all_twelve_stage_specs_load_and_kind_matches` 遍历 12 个 `StageKind` 调 `load_embedded_stage_spec`（= serde 反序列化）。任何 spec 里写错的 `gate_rules` op/pred/named-check 名（typed enum）→ 反序列化报错 → 该测试当场失败。**这是 B 全程的 fail-closed 兜底。**

---

## 3. 不变量（B 不改）

- **I-A · 5 个结构 check 永远跑**：`schema / contract / vacuous / freshness / finding_verification` 触发与逻辑不变，不并入 `gate_rules`。它们是结构地基，不是「可选菜单」。
- **I-B · 确定性 + DB-free 主路**：`rule_engine::eval` 纯函数、无 IO；named_check 调用的 3 个旧 check 本就只读 deliverable(+spec 配置)。
- **I-C · 行为零变更**：B 是**结构性重构**——迁移后每个 stage 的 PASS/BLOCK 结论与迁移前**逐字节一致**（§7 证明）。这是安全闸删代码的硬约束。
- **I-D · fail-closed**：删掉 `_ => continue` 后，「过关标准」唯一来源是 typed-enum 的 `gate_rules`，写错名 = 加载期报错。

---

## 4. 目标 / 非目标

**目标**
- G1 删 `StageSpec.required_checks` 字段 + `gate/mod.rs` 的 `match`（含 `_ => continue`）。
- G2 `gate_rules` 成为过关标准**唯一**入口；简单标准用数据积木、领域/遗留逻辑用 `named_check` 逃生舱，二者都从这一个口子走。
- G3 行为零变更：12 份 spec 迁移后 gate 结论不变（§7 + 等价性测试）。
- G4 全程 fail-closed：未知 op/pred/named-check 名 → spec 加载失败（被 §2.5 测试抓）。

**非目标**
- 不重写 `surface_coverage` / `min_invocations` 的**领域逻辑**——它们经 `named_check` 原样保留（行为零变更优先）。
- 不**加固** min_invocations（它是弱 MVP，「按真实工具调用计数」是独立的后续语义改进，不与本结构迁移纠缠）。
- 不碰 `gate_validator` 字符串字段（声明但未接线的正交遗留；调度永远走 `validate_stage_gate_with_skeleton`）。
- 不动 5 个结构 check、不动 `finding_verification`/`min_findings`/`min_claims`/`required_evidence_kinds` 这套已是配置驱动的字段。

---

## 5. 设计

### 5.1 新积木：`named_check` 逃生舱（typed enum，fail-closed）

`GateRule` 加一个变体，用于从 `gate_rules` 里**按名调用**保留下来的 Rust 领域 check：

```rust
// rule_engine.rs · GateRule 新增变体
NamedCheck {
    check: NamedCheckKind,
    #[serde(default)] on_fail: Option<OnFail>,  // 可选：覆盖该 check 默认的 reason/recovery
},

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedCheckKind { Scope, SurfaceCoverage, MinInvocations }
```

JSON 写法（替代旧 `required_checks` 里的名字）：
```json
{ "op": "named_check", "check": "surface_coverage" }
{ "op": "named_check", "check": "min_invocations" }
```

`NamedCheckKind` 是闭合 enum → 写错名字 serde 报错 → fail-closed。逃生舱**故意只有 3 个固定值**：它不是给用户随便加新 check 用的（那是数据积木的活），只是「把无法纯数据化的现存领域逻辑挂回统一入口」。将来某个 named_check 被数据化后，就从这个 enum 删掉。

### 5.2 `scope` → 纯数据规则（实现用户「标准写进 JSON」的目标）

`scope_check` 只是「claim/finding 证据非空」，**可纯数据化**。迁移为两条数据规则（放进除 scoping/reporting 外的 10 份 spec）：

```json
{ "op": "for_all", "over": "claims",
  "require": { "pred": "non_empty", "field": "evidence_ids" },
  "on_fail": { "reason": "every claim must cite evidence",
               "hints": ["add evidence_refs to each claim via prior tool calls"] } },
{ "op": "for_all", "over": "findings",
  "require": { "pred": "non_empty", "field": "evidence_refs" },
  "on_fail": { "reason": "every finding must cite evidence",
               "hints": ["add evidence_refs to each finding via prior tool calls"] } }
```

- `scoping`：authz-only、旧 scope_check 豁免它 → **不加这两条**（行为一致）。
- `reporting`：旧无 scope_status_present → **不加**（行为一致）。
- 其余 10 份：加这两条 = 复刻 scope_check 的 evidence-非空语义。

> 这条是 B 的「净收益」：scope 从 Rust 黑箱变成每份 spec 里**看得见、可改**的数据规则——正是用户要的「过关标准在 JSON 里定义」。

### 5.3 `surface_coverage` / `min_invocations` → `named_check`

- `surface_coverage`：领域关键词归类逻辑（`surface_mapping.rs`），纯数据化需要新谓词 + 把关键词表搬进 JSON，得不偿失 → **保留 Rust，经 `named_check:surface_coverage` 调用**。只 external_attack_surface 用。
- `min_invocations`：弱 MVP（读 `spec.min_invocations`）→ **保留 Rust，经 `named_check:min_invocations` 调用**，行为零变更；加固另开。external_attack_surface + enumeration 用。

### 5.4 `eval` 入参变化（named_check 需要 deliverable + spec）

`min_invocations_check::run(deliverable, spec)` 需要 `spec`。故把引擎入口从 `eval(deliverable, rules)` 改为：

```rust
// rule_engine.rs
pub fn eval(deliverable: &StageDeliverable, spec: &StageSpec, rules: &[GateRule]) -> Vec<GateCheckOutcome>;
```

- 数据 op（count_at_least/for_all）忽略 `spec`。
- `NamedCheck` 分支 dispatch 到现有 Rust check：
  - `Scope` → `scope_check::run(deliverable)`
  - `SurfaceCoverage` → `surface_coverage_check::run(deliverable)`
  - `MinInvocations` → `min_invocations_check::run(deliverable, spec)`
  - 若 rule 带 `on_fail` 且该 check 返回 Block → 用 rule 的 reason/recovery 覆盖（否则用 check 自身的）。
- 模块依赖：`rule_engine` 反向引用 `stage_spec::StageSpec`。`stage_spec` 已引用 `rule_engine::GateRule`——同 crate 内模块互引类型/函数合法（仅 crate 间不可成环），无编译问题。

> 备选（若想让 `rule_engine` 保持不依赖 `StageSpec`）：named_check 的 dispatch 留在 `gate/mod.rs`（那里 deliverable+spec 都在手），`rule_engine::eval` 只管数据 op。两种都可，计划阶段二选一；本设计取「单一 eval 入口」以贯彻 G2。

### 5.5 删旧路

- `gate/mod.rs`：删除 `required_checks` 的 `for name in &spec.required_checks { match ... }` 整段（含 `_ => continue`）与 `HashSet ran` 去重逻辑；保留 5 个结构 check；把 `outcomes.extend(rule_engine::eval(deliverable, spec, &spec.gate_rules))` 作为唯一语义层。
- `stage_spec.rs`：删 `pub required_checks: Vec<String>` 字段。
- 12 份 spec：删 `required_checks` 数组，按 §6 写入等价 `gate_rules`。
- `min_invocations` 仍读 `spec.min_invocations`（该字段保留），`surface_coverage` 不需额外字段。

---

## 6. 逐 stage 迁移表（行为等价）

> 「scope×2」= §5.2 的两条 for_all 非空规则。

| stage | 旧 required_checks（有效项） | 新 gate_rules |
|---|---|---|
| scoping | （豁免，无） | `[]`（保持现有 gate_rules 为空） |
| target_intel | scope | scope×2 |
| external_attack_surface | scope + surface_coverage + min_invocations | scope×2 + `named_check:surface_coverage` + `named_check:min_invocations` |
| enumeration | scope + min_invocations | scope×2 + `named_check:min_invocations` |
| vuln_triage | scope | scope×2 |
| verification | scope | scope×2 +（**保留现有** high+ finding 证据样例规则） |
| access_validation | scope | scope×2 |
| internal_discovery | scope | scope×2 |
| objective_pathing | scope | scope×2 |
| objective_simulation | scope | scope×2 |
| cleanup | scope | scope×2 |
| reporting | （无有效项） | `[]` |

注：verification 已有的 `gate_rules`（high+ finding 必须挂证据）**保留并叠加** scope×2；二者不冲突。

---

## 7. 行为零变更论证（安全闸删代码的硬要求）

> **范围澄清**：「零变更」指 gate 的 **PASS/BLOCK 决策**逐字节不变。少数 BLOCK 的
> **reason 文案**会变（scope_check 的逐元素消息 `finding[i] ... has empty evidence_refs`
> → 声明式规则的 `every finding must cite evidence`）——这是把真相源从 Rust check 迁到
> 数据规则的必然结果，不影响是否放行。受影响断言：`e2e_finding_missing_evidence_refs_*`
> 已更新为新文案（仍验证「缺证据 finding 被 Block」）。

逐 stage 对照「旧 gate 跑了哪些 check」vs「新 gate 跑了哪些」：

1. **5 个结构 check**：两边都永远跑，未动。✓
2. **scope**：旧 = scope_check（claim/finding 证据非空，scoping 豁免）。新 = §5.2 两条 for_all 非空，加在「旧会跑 scope_check 的同一批 stage」（= 有 scope_status_present 且非 scoping = 10 份）。`for_all non_empty` 与 scope_check 的逐元素非空判定**语义逐字节相同**；reporting/scoping 两边都不跑。✓（等价性测试 §10.2 锁死）
3. **surface_coverage / min_invocations**：新走 `named_check` 调**同一个 Rust 函数**，入参相同 → 输出必然相同。✓
4. **空跑名**（evidence_non_empty / unchecked_distinct）：旧 = `_ => continue` 无效果；新 = 删掉。两边都不产生 outcome。✓
5. **聚合**：`aggregate` 仍是 AND，未动。✓

结论：迁移是纯结构搬运，gate 结论不变。任何差异都会被 §10 的全 12-spec 回归 + 等价性测试抓住。

---

## 8. 删除步骤与回滚

**删除顺序**（先加新、再切、后删——避免中间态 gate 失效）：
1. 加 `named_check` 积木 + 改 `eval` 入参（旧 match 仍在，gate_rules 仍附加跑）。
2. 12 份 spec 写入新 `gate_rules`（此时 required_checks 与 gate_rules **同时存在**会双跑 scope —— 双跑同一非空判定结论不变，仅 reasons 可能重复，属安全的中间态）。
3. 删 `gate/mod.rs` 的 match + `stage_spec.required_checks` 字段 + 12 份 spec 的 `required_checks` 数组。
4. 跑全套回归。

**回滚点**：每步独立 commit；任一步 `just test-harness` 红即 `git revert` 该步。`required_checks` 字段删除是最后一步，之前所有步骤可单独回退。

---

## 9. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| 迁移漏 / 写错某 stage 的 gate_rules，悄悄放宽门槛 | 中 | §7 等价性 + §10.2 逐 stage 对照测试；§2.5 加载期 fail-closed |
| 删字段破坏其它读 `required_checks` 的代码 | 中 | 先全仓 grep `required_checks`（字段 vs `required_checks_done` 不同，勿误删）；编译器兜底 |
| named_check 的 on_fail 覆盖语义没对齐旧 recovery | 低 | 默认沿用 check 自身 recovery；on_fail 仅按需覆盖 |
| 中间态双跑 scope 产生重复 reasons | 低 | 仅 step 2→3 之间短暂存在；结论不变；step 3 后消失 |
| 属 gate 核心链路 | — | TDD + `just test-harness` + clippy -D + `just precommit`；AGENTS.md §2.5/§2.7（删代码已获用户授权方向 B） |

---

## 10. 验证计划（证据优先）

1. **单测（rule_engine.rs）**：新增 `named_check` 三种 kind 的 dispatch 测试（Scope/SurfaceCoverage/MinInvocations 各一 Pass + 一 Block）；未知 named-check 名 serde Err。
2. **等价性测试**：对每个曾用 scope/surface/min_invocations 的 stage，构造会 Block 的 deliverable，断言「迁移前 spec」与「迁移后 spec」gate 结论一致（可用 git 历史 spec 或内联两份 JSON）。
3. **回归**：`all_twelve_stage_specs_load` + `just test-harness` 全绿（12 份新 spec 全部解析 + gate 行为）。
4. **删字段后编译**：`cargo check -p golish-agent-kit` 通过（无残留 `required_checks` 读取点）。
5. **门禁**：`cargo nextest -p golish-agent-kit -p golish-agent-app` + `cargo clippy ... -D warnings` + `cargo fmt --check` 全绿；命令与退出码留痕到 `agent-progress.md`。

---

## 11. 调查证据（file:line）

- 旧 match + `_ => continue`：`gate/mod.rs:125-141`。
- 3 个 check 逻辑：`scope_check.rs:9-68`（仅证据非空 + scoping 豁免）、`surface_coverage_check.rs:16-90`（surface_mapping 领域逻辑）、`min_invocations_check.rs:12-44`（读 spec.min_invocations + required_checks_done 子串匹配，忽略 count）。
- 归类逻辑：`surface_mapping.rs:38-95`（from_kind 关键词）+ `:132-146`（D2_REQUIRED = Surface+JsApi）。
- deliverable 别名：`types.rs:175`（`ExternalAttackSurfaceDeliverable = StageDeliverable`）。
- StageSpec 字段：`stage_spec.rs:42-104`（`required_checks` :66、`min_invocations` :68、`gate_rules` :103）。
- fail-closed 网：`resources.rs:122-142`（all_twelve_stage_specs_load）。
- 12 份 spec required_checks：`resources/harness/stages/*.json`（grep 实测，§2.4 表）。

---

## 12. 实现补遗（删字段时发现的额外消费者）

调查删除 `StageSpec.required_checks` 时，发现 gate match 之外**还有两个消费者**（设计初稿漏列，实现时一并迁移、行为保持）：

1. **`vacuous_check::run`（永远跑的结构 check 之一）** 用 `if !spec.required_checks.is_empty()` 作为 FakePattern 子检查的外门，真正阈值是 `sum(spec.min_invocations)`。等价改为 `if !spec.min_invocations.is_empty()`——对全 12 spec 逐字节一致（凡有 min_invocations 的 stage 旧时 required_checks 也非空；无 min_invocations 时 `required_total>0` 本就为假）。
2. **`task_orchestrator/prompts/mod.rs::stage_charter`** 用 `spec.required_checks.join(", ")` 给 agent 拼「gate 会检查哪些」的提示行。改为 `spec.gate_rules.iter().map(GateRule::summary)`——为此给 `GateRule` 加 `summary()`（数据 op 返 `on_fail.reason`；named_check 返其 reason 或 `<kind> check`）。这是 agent 面向**提示文案**变化，不影响 gate 决策。

其余 `required_checks` 引用均为注释/字符串文案，已顺手更新（`types.rs` / `stage_harness.rs` doc comment、`vacuous_check` reason 串）。

**实现验证（本机实跑全绿）**：`cargo nextest -p golish-agent-kit -p golish-agent-app` → 509 passed / 0 failed；`cargo clippy ... -D warnings` → 0；`cargo fmt --check` → clean。
