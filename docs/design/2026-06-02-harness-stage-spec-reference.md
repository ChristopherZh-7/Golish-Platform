# Operation Harness · 节点层参考（Stage spec）

> 目的：讲清楚 harness 第二层「**单个阶段怎么定义**」——Stage spec 是什么、字段逐个、闭环 5 段怎么落到字段、required_checks 怎么变成 gate 实际跑的检查、以及 **12 个阶段现状一览**（哪些定义得实、哪些还是占位）。这是一份**现状参考**（reference）。
>
> 证据来源（均已逐一核对真实文件）：
> - `harness/stage_spec.rs`（`StageSpec` 结构体 + loader）
> - `resources/harness/stages/*.json`（12 个阶段全部）
> - `harness/gate/`（7 个 check 文件名）、配合拓扑层文档
>
> 配套：上一层（Profile + DAG）见 `2026-06-02-harness-topology-reference.md`；总览见 `2026-06-01-harness-explainer-and-decisions.md`。日期：2026-06-02。

---

## 0. 这一层是什么

节点层 = 拓扑层选出的**每个节点内部的「合同」**。一个 stage = 一个 `stages/<名>.json`。它回答这个阶段的 5 个问题：进得来吗、能用啥工具、必须做+必须产什么、gate 查什么才放行、通过去哪（外加：要不要人工批、从上游继承哪些证据）。

> 这层就是「逐阶段定义做什么 + 闭环」的**主战场**——你真正要逐个填的是每个 stage.json 的 `required_checks` + `min_invocations` + 交付物约束。

---

## 1. Stage spec 字段逐个（对应 `StageSpec` 结构体）

| 字段 | 类型 | 含义 | 闭环核心 |
|---|---|---|---|
| `id` / `kind` | string / StageKind | 阶段名（要和 StageKind / DAG / 文件名对齐） | |
| `risk_level` | enum | low / medium / high / critical | |
| `requires_stages` | `Vec<StageKind>` | 进来前必须先完成的阶段 | ① 进得来 |
| `allowed_tools` | `Vec<String>` | 这阶段只准用的工具（纯字符串，不校验真伪） | ② 能用啥 |
| `forbidden_tools` | `Vec<String>` | 明令禁止的工具 | ② 能用啥 |
| `min_invocations` | `Map<String,u32>` | 哪些工具至少跑几次 | ③ 必须做 ⭐ |
| `deliverable_schema` | string | 要交的结构化交付物类型 | ③ 必须产 ⭐ |
| `required_checks` | `Vec<String>` | gate 查哪些才放行 | ④ 闸 ⭐ |
| `gate_validator` | string | 用哪个 gate 校验函数 | ④ 闸 |
| `allowed_next_stages` | `Vec<StageKind>` | 通过后能去哪（与 DAG 边一致） | ⑤ 去哪 |
| `human_approval.required_before` | `Vec<String>` | 做哪些动作前要人工批（配合 profile.approval_policy 才触发） | 人工闸 |
| `inherits_evidence_from` | `Vec<{stage_kind, evidence_kinds}>` | 从上游哪些阶段继承哪些证据 | 证据继承 |
| `max_other_skips` | `Option<u32>` | vacuous 检测：容忍几个「其它原因跳过」的检查 | |
| `agent_continuity` | enum | `single_session` / `multi_session_relay` | |

字段大多有 `#[serde(default)]`，所以缺省也能解析；`$schema` / `$comment` 注释字段被忽略。

---

## 2. 闭环 5 段怎么落到字段

```
① 进得来   requires_stages 满足
② 能用啥   allowed_tools / forbidden_tools（authorizer 在每次工具调用前拦）
③ 必须做+必须产   min_invocations（跑哪些工具×几次） + deliverable（claims/findings/evidence_refs）
④ 闸       gate 按 required_checks（+ gate_validator）校验 → PASS 推游标 / BLOCK 打回
⑤ 去哪     allowed_next_stages（要和 DAG 边一致）
旁路：human_approval（人工闸） + inherits_evidence_from（证据继承）
```

⭐ 真正的「放行标准」核心 = `min_invocations` + `required_checks` + 交付物约束。逐阶段要做的就是把这三块从占位写成真标准。

---

## 3. required_checks 选项 → gate 实际跑的 check

gate 永远跑 **4 个结构性 check**（`schema` / `contract` / `vacuous` / `freshness`），再按 `required_checks` 选跑 **3 个语义 check**：

| `required_checks` 里的字符串 | 走哪个 gate check | 性质 |
|---|---|---|
| `scope_status_present` | `scope_check` | 语义（选跑） |
| `out_of_scope_targets_excluded` | `scope_check` | 语义（选跑） |
| `surface_workbench_coverage` | `surface_coverage_check` | 语义（选跑） |
| `min_tool_invocations_per_check` | `min_invocations_check`（读 `min_invocations`） | 语义（选跑） |
| `evidence_non_empty` | 已被 schema/vacuous 覆盖 | 结构性兜底 |
| `unchecked_distinct_from_checked_empty` | 已被 schema/vacuous 覆盖 | 结构性兜底 |

> 含义：你在某阶段 `required_checks` 里写 `min_tool_invocations_per_check`，gate 才会去查 `min_invocations`；写 `surface_workbench_coverage` 才查攻击面覆盖度。`evidence_non_empty` / `unchecked_distinct_from_checked_empty` 即使列上，主要是结构性兜底（schema/vacuous 已覆盖）。

---

## 4. 12 个阶段现状一览（关键表）

| # | 阶段 | risk | requires | next | checks 数 | min_invocations | human_approval.required_before | continuity |
|---|---|---|---|---|---|---|---|---|
| 1 | scoping | low | — | target_intel | **1** | — | scope_expansion | single |
| 2 | target_intel | low | scoping | eas | 3 | dns_resolve:1 | active_scan | single |
| 3 | external_attack_surface | medium | scoping,target_intel | enumeration,reporting | **6** | dns_resolve:1, http_probe:1, subdomain_enum_passive:1 | active_scan, exploit_validation | single |
| 4 | enumeration | medium | eas | vuln_triage,reporting | 5 | http_probe:1 | active_scan | single |
| 5 | vuln_triage | high | enumeration | verification,reporting | 4 | — | active_scan, exploit_validation | single |
| 6 | verification | critical | vuln_triage | access_validation,reporting | 3 | — | exploit_validation | single |
| 7 | access_validation | critical | verification | internal_discovery | 3 | — | post_exploit, exploit_validation | relay |
| 8 | internal_discovery | critical | access_validation | objective_pathing | 3 | — | post_exploit, exploit_validation | relay |
| 9 | objective_pathing | critical | internal_discovery | objective_simulation | 3 | — | post_exploit, exploit_validation | relay |
| 10 | objective_simulation | critical | objective_pathing | cleanup | 3 | — | post_exploit, exploit_validation | relay |
| 11 | cleanup | medium | objective_simulation | reporting | 2 | — | — | relay |
| 12 | reporting | low | — | — | **1** | — | — | single |

> 注：除 `external_attack_surface` 用 `ExternalAttackSurfaceDeliverable` + `validate_external_attack_surface_gate` 外，**其余 11 个阶段全用通用 `StageDeliverable` + `validate_stage_gate`**。

各阶段 `required_checks` 明细：

- **scoping**：scope_status_present
- **target_intel**：scope_status_present, evidence_non_empty, out_of_scope_targets_excluded
- **external_attack_surface**：scope_status_present, evidence_non_empty, unchecked_distinct_from_checked_empty, out_of_scope_targets_excluded, min_tool_invocations_per_check, surface_workbench_coverage
- **enumeration**：上面前 5 个（无 surface_workbench_coverage）
- **vuln_triage**：scope_status_present, evidence_non_empty, unchecked_distinct_from_checked_empty, out_of_scope_targets_excluded
- **verification / access_validation / internal_discovery / objective_pathing / objective_simulation**：scope_status_present, evidence_non_empty, out_of_scope_targets_excluded（同样 3 条）
- **cleanup**：scope_status_present, evidence_non_empty
- **reporting**：evidence_non_empty

---

## 5. 定义「实」vs「占位」分级

| 等级 | 阶段 | 说明 |
|---|---|---|
| **实**（样板） | external_attack_surface | 6 条 check（含 surface 覆盖度）+ 3 个 min_invocations，放行标准具体 |
| **中等** | enumeration(5+1) · target_intel(3+1) · vuln_triage(4+0) | 有语义 check，但 min_invocations 偏薄或空 |
| **薄/占位** | scoping(1) · reporting(1) · verification(3+0) · cleanup(2) | 只有通用 check，无 min_invocations |
| **模板复制** | access_validation · internal_discovery · objective_pathing · objective_simulation | 4 个红队阶段几乎逐字段相同（3 check / 无 min / 同 approval / relay），明显占位 |

---

## 6. 几个值得注意的现状点

1. **只有 eas 有专属 deliverable_schema + gate_validator**；其余 11 个用通用 `StageDeliverable` + `validate_stage_gate`。要给某阶段强化交付物结构，需要像 eas 那样定专属 schema/validator。
2. **min_invocations 只有 3 个阶段有**（eas / target_intel / enumeration）；其余全空。空的阶段即便 `required_checks` 里列了 `min_tool_invocations_per_check` 也没东西可查（vuln_triage 没列、是一致的）。
3. **红队 4 阶段是同一模板复制**（access_validation → objective_simulation）：3 条通用 check、无 min_invocations、相同 human_approval、relay。去模板化是红队 profile 真正可用前的前置。
4. **verification 是 critical 却放行标准偏弱**：只 3 条通用 check + 无 min_invocations + **没有「必须有 exploit_proof / PoC 证据」的强校验**。受控利用的验收目前主要靠通用 evidence 非空，不够。
5. **allowed_tools 是纯字符串**（不校验工具真伪）→ 写错变「幽灵工具」。强类型的是阶段名（StageKind），不是工具名。
6. **agent_continuity**：侦察/漏洞段是 `single_session`；cleanup + 红队 4 阶段是 `multi_session_relay`（跨会话接力），但接力的实际落地程度需另行核对（拓扑/gate 层不负责）。

---

## 7. 这层的待决策（衔接「逐阶段填 contract」）

逐阶段把 `required_checks` + `min_invocations` + 交付物约束从占位写成真标准，优先级建议：

1. **scoping**：把「授权闭环」做实（in-scope 非空 + out-of-scope 列明 + 授权来源登记），现在只 1 条 check。
2. **verification**：受控利用必须有 exploit_proof 证据的强校验 + 允许程度边界（只读验证 vs 拿 shell）。
3. **红队 4 阶段**：去模板化，各自定义达成/验收标准；或先确认这次要不要开 red_team。
4. **cleanup**：补「清理是否真做了」的强校验（red_team 的 cleanup_required 才有意义）。
5. **reporting**：每条 finding 可追溯 evidence + 整改建议的校验。

> 注：很多强校验依赖 **Evidence Ledger（evidence_audit 表）**，目前未建，gate 只能信 AI 自报 evidence_refs——这是把放行标准做「实」的最大前置（见 `2026-06-01-harness-explainer-and-decisions.md`）。

---

## 8. `gate_rules` DSL 速查（2026-06-05）

声明式过关标准，与 `required_checks` 并存；缺省空数组（不写则行为不变）。引擎：`gate/rule_engine.rs::eval`（纯函数、DB-free、确定性）。完整设计见 `docs/design/2026-06-05-gate-rule-engine.md`。

**顶层 op**
- `count_at_least`：`{ op, over, where?, min, on_fail }` — 满足 `where` 的元素 ≥ `min`。
- `for_all`：`{ op, over, where?, require, on_fail }` — 满足 `where` 的每个元素都满足 `require`（空集合为真）。

**over**：`claims` | `findings`

**pred**（用于 `where` / `require`）
- `{ "pred":"non_empty", "field":<f> }`
- `{ "pred":"eq", "field":<f>, "value":"<s>" }`
- `{ "pred":"severity_at_least", "min":"info|low|medium|high|critical" }`（仅 findings）

**field**：`kind` | `subject` | `summary` | `evidence_refs`(finding) | `evidence_ids`(claim) | `severity`(finding)

**on_fail**：`{ reason, hints?, repair_tool_calls?, missing_evidence_kinds? }` → 映射到 `GateCheckOutcome::Block` + `HarnessRecoveryActions`。

**fail-closed**：未知 op/pred/over/field → spec 反序列化报错（被 `all_twelve_stage_specs_load` 抓）；字段-集合不匹配（如对 claims 取 severity）→ 求值期返回 `gate_rule config error` Block。绝不静默忽略。

**样例**（`resources/harness/stages/verification.json`）：每个 high+ finding 必须挂证据
```json
"gate_rules": [
  { "op":"for_all", "over":"findings",
    "where":{"pred":"severity_at_least","min":"high"},
    "require":{"pred":"non_empty","field":"evidence_refs"},
    "on_fail":{"reason":"verification: every high/critical finding must carry evidence"} }
]
```
