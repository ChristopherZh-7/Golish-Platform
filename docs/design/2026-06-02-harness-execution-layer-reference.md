# Operation Harness · 执行层参考（Deliverable / 三道闸 / Evidence Ledger）

> 目的：讲清楚 harness 第三层「**阶段怎么真跑、怎么把关**」——AI 交的 Deliverable 长啥样、三道闸（动作前 authorizer / 阶段末 gate / 人工 approval）各判什么、gate 的 7 个 check 逐个怎么判、以及为什么 Evidence Ledger 是这层的最大前置。现状参考。
>
> 证据来源（均已逐一核对真实文件）：
> - `harness/types.rs`（StageDeliverable / StageClaim / HarnessFinding / GateResult / HarnessRecoveryActions / IntentAxis）
> - `harness/gate/mod.rs` + 7 个 check 文件、`harness/pre_action_authorizer.rs`、`harness/stage_transition.rs`
> - `task_orchestrator/subtask_phases/execute.rs`（hook + approval hold→wait→resume）、`resources/harness/evidence_kinds.json`
>
> 配套：拓扑层见 `2026-06-02-harness-topology-reference.md`；节点层见 `2026-06-02-harness-stage-spec-reference.md`；总览见 `2026-06-01-harness-explainer-and-decisions.md`。日期：2026-06-02。

---

## 0. 这一层是什么

拓扑层定「走哪些阶段」，节点层定「每阶段的合同」，**执行层就是阶段真正跑 + 把关的地方**：

```
AI 在阶段里干活 → 产出 Deliverable（结构化 JSON）
        ↑ 每次工具调用先过 [闸1 authorizer]
   阶段末 → [闸2 gate] 按 7 个 check 校验 Deliverable → PASS 推游标 / BLOCK 打回
   切阶段/高危 → [闸3 human approval] 阻塞等人批
   所有 check 的"真证据"本应回查 [Evidence Ledger]（最大缺口，见 §6）
```

---

## 1. Deliverable（gate 的唯一输入）

`StageDeliverable`（`types.rs`，所有 stage 通用；旧名 `ExternalAttackSurfaceDeliverable` 是别名）。AI 在阶段末尾吐一段 ```json``` 块，hook 解析成它喂给 gate。

| 字段 | 类型 | 含义 |
|---|---|---|
| `stage_id` | string | 阶段名（gate 会和 spec.id 比对） |
| `stage_run_id` | uuid | 本次阶段运行 id（非 nil） |
| `claims` | `[StageClaim]` | 观察声明：`{kind, subject, summary, evidence_ids[]}` |
| `findings` | `[HarnessFinding]` | 结构化发现：`{finding_id, kind, subject, severity, evidence_refs[]}` |
| `evidence_refs` | `[EvidenceAuditId]` | 本交付引用的所有证据 id |
| `skipped_checks` | `[SkippedCheckRecord]` | 显式跳过的检查 `{check, reason=SkipReason}`（"检查为空"≠"没查"） |
| `required_checks_done` | `[String]` | **app-level hint**（agent 自报跑过的 check/tool）；gate 以 `spec.required_checks` 为准 |

> 关键：`evidence_ids` / `evidence_refs` 的类型是 `EvidenceAuditId`（来自 `golish_pentest::evidence_ledger`）——**类型存在，但 evidence_audit 真表未建**，所以 gate 现在只能信 AI 自报的这串 id，没法回查"这 id 真对应一条工具产出的真证据吗"（见 §6）。

---

## 2. 三道闸总览

| 闸 | 时机 | 谁拦 | 判什么 | 不过的后果 |
|---|---|---|---|---|
| **① PreActionAuthorizer** | 每次工具调用前 | 代码（确定性） | 工具在阶段 allowed 吗 + intent 没超授权吗 | 拒绝这次工具调用 |
| **② Gate** | 阶段做完时 | 代码（确定性） | Deliverable 满足 7 个 check 吗 | BLOCK 打回返工 |
| **③ Human Approval** | 切阶段 / 高危动作前 | 人（你） | 你批不批 | hold 阻塞，不推进 |

---

## 3. 闸① PreActionAuthorizer（动作前·工具级）

`pre_action_authorizer.rs`，每个 tool call dispatch 前跑，3 种 deny（按顺序）：

1. `tool ∈ spec.forbidden_tools` → **ToolForbidden**
2. `tool ∉ spec.allowed_tools` → **ToolNotAllowed**
3. intent 超授权 → **IntentExceedsAuthorization**

intent → 所需授权档映射：

| IntentAxis | 所需 AuthorizationLevel |
|---|---|
| `passive_observe` | L1 passive_intel |
| `active_probe` | L2 active_recon |
| `vuln_validation` | L3 vuln_validation |
| `exploit_validation` | L4 controlled_exploit |

若 `所需档.rank() > profile.max_authorization.rank()` → 拒。例：assessment（L2 顶）下，带 `exploit_validation` intent 的工具即使在 allowed_tools 里也被拒。C3 把 `HarnessAuthz{max_authorization, intent}` 穿到 per-tool dispatch，免得每次重载 profile。

---

## 4. 闸② Gate（阶段末·交付级）= 4 结构 + 3 语义

`validate_stage_gate`：**4 个结构性 check 永远跑** + 按 `spec.required_checks` 选跑 **3 个语义 check**（去重），最后 `aggregate` 合并所有 BLOCK 的 reasons + recovery → `GateResult{allowed, reasons, recovery_actions}`。

| # | check | 类型 | 实际判什么 | 现状严格度 |
|---|---|---|---|---|
| 1 | `schema_check` | 结构·永远 | stage_id 非空 + stage_run_id 非 nil + stage_id==spec.id | skeleton（仅 sanity，未做完整 schema 比对） |
| 2 | `contract_check` | 结构·永远 | 有 SprintContract 则 status 必 active；有 skeleton 则按 finding kind 的 expected_count_range 校数量 + evidence 总数≥min 和 | **默认 hook 不传 contract/skeleton → 基本 pass** |
| 3 | `vacuous_check` | 结构·永远·**防糊弄** | (a) 全空(claims+findings+skipped 都空)→block (b) FakePattern: evidence_refs 数 < sum(min_invocations)→block (c) SkipPattern: Other 类 skip 数 > max_other_skips→block | **真硬**；以 spec 为准，不读 agent 自报的 required_checks_done（防清空绕过） |
| 4 | `freshness_check` | 结构·永远 | sanity：claim/finding 引用的 evidence_id 必须都在 evidence_refs 里 | 默认只 sanity；真 max_age 比较需 Ledger 喂 age（未接） |
| 5 | `scope_check` | 语义（scope_status_present / out_of_scope_targets_excluded） | 每个 claim 有非空 evidence_ids、每个 finding 有非空 evidence_refs | skeleton（只查非空；真 InScope label 查 evidence_classifications 未做） |
| 6 | `surface_coverage_check` | 语义（surface_workbench_coverage） | 硬要求 Surface + JsApi 类别都有 evidence 支撑的 claim/finding，缺→block；Sitemap 没覆盖且没显式 skip→仅 hint | **真硬**（仅 eas 用它）；体现 I8「checked-empty≠unchecked」 |
| 7 | `min_invocations_check` | 语义（min_tool_invocations_per_check） | spec.min_invocations 每个 tool，看 deliverable.required_checks_done 里有没有包含该 tool 名 | Phase B 近似：**靠 agent 自报字符串包含匹配**；真计数推 Phase C |

**BLOCK 之后**：gate 的 `recovery_actions`（hints / repair_tool_calls / missing_evidence_kinds）被渲染成 correction 回灌 reflector retry，agent 重做并重交 deliverable（C4）。

> 小结：真正"硬"的是 **vacuous**（防全空/造假/滥用 skip）和 **surface_coverage**（Surface+JsApi 硬覆盖，且只有 eas 用）。其余目前多是 skeleton/近似级，等接 Evidence Ledger 才能做实。

---

## 5. 闸③ Human Approval（人工）

`stage_transition.rs::stage_entry_requires_approval(next_spec, profile)`：当**下一阶段声明了 `human_approval.required_before`（非空）** 且 **profile.approval_policy 任一开**（before_active_scan / before_scope_expansion）→ 需批。

`execute.rs` 的流转里：需批 → 发 `waiting_approval` 事件 → **阻塞等 `user_input_rx`** → 你回 yes/approve/继续 才 `advance_stage` 推游标；否则保持 hold 不推进。

> 现状：机制打通，但因为默认 profile=assessment 且分支策略不会自动走到 verification，approval 在日常默认流程里几乎不会被触发（见拓扑层文档 B5 / C4）。

---

## 6. Evidence Ledger（这层最大前置）

- **类型在**：`EvidenceAuditId`（`golish_pentest::evidence_ledger`）、`SkipReason`、`evidence_kinds.json` + `EvidenceKindRegistry`（每种证据的 max_age 词典）都已存在。
- **表没建**：`evidence_audit` 真表未落地，工具产出没有逐条入账。
- **后果**：gate 只能信 AI 自报的 `evidence_refs` 数量 + `required_checks_done` 字符串，无法交叉验证真证据。直接被卡住做不"实"的有：
  1. `contract_check` 的真 tool_call_counts（现用 evidence_refs 长度近似）
  2. `freshness_check` 的真 max_age 比较（现默认只 sanity）
  3. `min_invocations_check` 的真工具调用计数（现靠 required_checks_done 字符串匹配）
  4. `scope_check` 的真 InScope label（现只查非空）

> 一句话：**执行层的"闸"骨架齐全且确定性，但多数 check 的"严肌肉"要等 Evidence Ledger 落地才能长出来。** 这是把放行标准从"信自报"升到"可交叉验证"的总开关。

---

## 7. 现状小结 + 待决策

- **闸① authorizer**：✅ 完整（forbidden / not-allowed / intent 超授权三判都实）。
- **闸② gate**：✅ 7 check 骨架 + 单测齐；其中 vacuous / surface_coverage 真硬，其余 skeleton/近似。
- **闸③ approval**：✅ hold→wait→resume 机制实；默认流程少触发。
- **Evidence Ledger**：❌ 表未建 = 全层最大缺口。

待决策（衔接"逐阶段填 contract"与放行标准做实）：

1. **先落 Evidence Ledger 还是先用自报跑通闭环**？（对应总览 D0.2）
2. gate 各 skeleton check 要不要现在就升级（schema 完整比对 / scope 真 label / min_invocations 真计数）？
3. `contract_check` 的 SprintContract / skeleton 默认要不要传进来（现在 hook 传 None，等于没在校 finding 数量区间）？

> 这层定稿后，配合节点层逐阶段的 `required_checks` + `min_invocations`，才能把"每个阶段的闭环"真正焊死。
