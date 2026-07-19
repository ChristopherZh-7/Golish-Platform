# 两级阶段模型：大阶段(Phase) × 小阶段(Stage)

> Routine post-Scoping phase-confirmation decisions are superseded by
> `docs/design/2026-07-18-scoping-only-routine-human-confirmation.md`.
> Phase grouping, risk metadata, per-stage deterministic Gates, and typed
> authorization boundaries remain current.

> **一句话**：把现在扁平的 12 个 stage 重构成「外圈大阶段(phase) + 内圈小阶段(stage)」两级。大阶段 = 风险/审批边界（锚定已有授权阶梯 L0–L5）；大阶段内部的小阶段无强制先后、可并行，跑完整个大阶段在出口过一道放行（审批 + 游标前进）。
>
> **基于**：`docs/design/2026-06-01-harness-explainer-and-decisions.md`（现状地图 + 决策清单）。本文是它点名的「下一份设计决策文档」。
>
> **状态**：草稿，待 review（2026-06-03）。定稿后转 `docs/superpowers/plans/` 出实现计划，再追加 `feature_list.json`。

---

## 1. 背景与问题

当前 harness 把一次渗透测试拆成 **12 个扁平 stage**（`StageKind`），靠一张**近似线性**的 DAG 串起来跑。实际使用中暴露三个痛点（用户原话）：

1. **太细**：12 个阶段，心智负担重；很多步骤本不该是「各自带闸的独立串行节点」。
2. **不该强制先后 / 能并行**：例如「被动情报 / 外部攻击面 / 主动枚举」本质是同一件事（资产探测），却被排成单链强制串行。
3. **改中间不方便**：调一个中间环节要动一堆 stage 文件 + 它们之间的边。

核心诉求 = **更粗 + 阶段内可并行 + 改中间只动一处**。

---

## 2. 目标 / 非目标

**目标**
- 引入显式的「大阶段(phase)」分层；大阶段 = 风险/审批边界。
- 大阶段内小阶段去掉**人为**先后约束，可并行（第一步：任意顺序）。
- 放行/人工审批只设在**大阶段边界**，数量收敛到 3 道。
- **复用**现有 per-stage gate 判定逻辑（零重写核心 gate）。

**非目标（本期不做）**
- 不重写 gate 校验内核（schema / vacuous / freshness / 证据账本交叉等一律保留）。
- 不做小阶段「真正同时并发执行」（运行时并发是更大改动，后置；本期只做「去掉强制顺序」）。
- 不改授权阶梯 L0–L5 的语义、不改 profile 的 stage 粒度配置（向后兼容）。

---

## 3. 现状（已核实，附文件位置）

| 概念 | 位置 | 现状（实测） |
|---|---|---|
| 12 个 stage 全集 | `backend/crates/golish-agent-kit/src/harness/types.rs` `enum StageKind` | scoping, target_intel, external_attack_surface, enumeration, vuln_triage, verification, access_validation, internal_discovery, objective_pathing, objective_simulation, reporting, cleanup |
| 授权阶梯 L0–L5 | `harness/profile.rs` `AuthorizationLevel` | ObserveOnly→PassiveIntel→ActiveRecon→VulnValidation→ControlledExploit→PostExploitRedTeam |
| Profile（选阶段 + 授权上限 + 审批策略） | `resources/harness/profiles/*.json` + `profile.rs` | `allowed_stage_kinds` / `forbidden_stage_kinds` / `max_authorization` / `approval_policy{before_active_scan, before_scope_expansion}` |
| Operation DAG | `resources/harness/graph/operation_graph.json` + `harness/operation_graph.rs` | 12 节点 / **15 边**＝11 条线性主干 + 4 条 bail-to-reporting 短路；加载时校验无环；`project()` 按 profile 投影出可达子图；`next_stages()` 给拓扑候选 |
| Stage Spec（每阶段合同） | `resources/harness/stages/*.json` + `harness/stage_spec.rs` | 字段含 `risk_level` / `requires_stages` / `allowed_next_stages` / `allowed_tool_types` / `required_checks` / `min_invocations` / `max_other_skips` / `human_approval.required_before` / `required_evidence_kinds` / `finding_verification` / `inherits_evidence_from` |
| Gate（放行闸） | `harness/gate/` | 结构性常跑（schema / contract / vacuous / freshness / finding_verification）+ 语义按 `required_checks` 选跑（scope / surface_coverage / min_invocations）+ 证据账本交叉 |
| Human Approval | profile 级 `approval_policy` + 阶段级 `human_approval.required_before` | 已实现；触发点经 `stage_transition` / `pre_action_authorizer` |

**当前 DAG 边（逐条）**：scoping→target_intel→external_attack_surface→enumeration→vuln_triage→verification→access_validation→internal_discovery→objective_pathing→objective_simulation→cleanup→reporting（线性主干），外加 external_attack_surface / enumeration / vuln_triage / verification / cleanup → reporting 的 bail 短路。

**关键观察**：用户要的「大阶段 = 风险边界」其实**已隐含在授权阶梯 L0–L5 与三个审批语义键（scope_expansion / active_scan / exploit_validation）里**。本设计是把这层**扶正为一等概念**，而非从零发明。

---

## 4. 设计决策（已与用户逐条确认）

| # | 决策 | 选择 |
|---|---|---|
| D1 | 分层模型 | 两级：大阶段(phase) + 小阶段(stage)；大阶段**锚定授权阶梯 L0–L5** |
| D2 | 大阶段分组 | **5 个大阶段**（见 §5），按授权阶梯归类 |
| D3 | 「完整 / 确定」判定标准 | **复用现有 gate**：`required_checks` + `min_invocations` + `max_other_skips`(vacuous 防空壳)。判定语义 = 「该做的检查都做了，或明确标记 skipped + 原因」（不要求穷尽，遵守 I8「已查为空≠未查」） |
| D4 | 人工审批闸 | **3 道**，对齐授权跃迁（详见 §5）：`active_scan`＝①→②、`exploit_validation`＝②→③、`scope_expansion`＝① 段内扩范围事件触发 |
| D5 | 放行粒度 | **甲：保留每个小阶段各自 gate**；大阶段出口放行 = `AND(成员小阶段 gate 全 PASS)` + 大阶段边界审批。核心 gate 零重写 |
| D6 | 并行 | 第一步 = **去掉人为先后**（小阶段可任意顺序）；真正并发执行后置为独立工作项 |

---

## 5. 大阶段 ↔ 小阶段映射

> **采用「乙」分组**（2026-06-03 用户拍板，原 O3 已定）：被动情报 `target_intel` 归入 ① 准备段，让三道审批正好压在三次授权跃迁的边界上。

| 大阶段 | id | 授权档 | 含小阶段 | 入口审批（升入本阶段前要批） | profile |
|---|---|---|---|---|---|
| ① 准备与被动情报 | `prep` | L0–L1 | scoping · target_intel | —（入口即起点；`scope_expansion` 在段内**扩范围时**事件触发） | 全部 |
| ② 主动侦察 | `active_recon` | L2 | external_attack_surface · enumeration | `active_scan`（主动扫描前 = L1→L2） | 全部 |
| ③ 漏洞与利用 | `vuln` | L3–L4 | vuln_triage · verification | `exploit_validation`（真打前 = L2→L3） | pentest / red_team（assessment forbidden） |
| ④ 后渗透 | `post_exploit` | L5 | access_validation · internal_discovery · objective_pathing · objective_simulation | （红队授权 = L4→L5；assessment/pentest 不可达） | **仅 red_team** |
| ⑤ 收尾 | `closeout` | — | reporting · cleanup | —（出口 gate：报告可追溯 + 已清理） | reporting 全部；cleanup 仅 red_team |

**三道审批 ↔ 授权跃迁（乙 的核心收益）**：`active_scan`＝①→②（被动转主动）、`exploit_validation`＝②→③（侦察转真打）、`scope_expansion`＝① 段内扩范围的事件触发。每道审批都落在两个大阶段的「门口」或明确的动作前，不再尴尬卡在某个大阶段内部。

说明：
- **assessment profile** 经 §6 投影后只剩 ①②⑤(reporting)，③④ 与 cleanup 被 `forbidden_stage_kinds` 裁掉——复用现有 profile 投影，无需为大阶段单独配置。
- **target_intel（被动情报）** 是「只查公开资料、不碰对方服务器」的零风险动作（L1），故归 ① 而非 ②；这样 `active_scan` 审批正好卡在「被动→主动」的门口。
- **reporting** 是「任意大阶段可提前 bail」的通用终点：归 ⑤，但保留各大阶段 → reporting 的 bail 边。
- `risk_level` 已核实：scoping=low / target_intel=low(L1 被动) / external_attack_surface=medium / enumeration=medium / vuln_triage=high / verification=critical(L4)；其余（红队 4 段 + reporting/cleanup）按授权阶梯归类，**写实现计划时逐个 stage JSON 校准**（遵守「未读不引」）。

---

## 6. 架构改动点（文件级）

1. **新增 phase 拓扑文件** `resources/harness/graph/phases.json`（推荐，与 operation_graph.json 并列）：
   ```jsonc
   { "phases": [
     { "id": "prep",         "stages": ["scoping","target_intel"] },
     { "id": "active_recon", "stages": ["external_attack_surface","enumeration"], "entry_approval": "active_scan" },
     { "id": "vuln",         "stages": ["vuln_triage","verification"],           "entry_approval": "exploit_validation" },
     { "id": "post_exploit", "stages": ["access_validation","internal_discovery","objective_pathing","objective_simulation"] },
     { "id": "closeout",     "stages": ["reporting","cleanup"] }
   ] }
   ```
   理由：一处看全结构（「改中间方便」），phase↔approval 显式可读。**备选**：给每个 stage JSON 加 `phase` 字段（分散，但少一个文件）——本设计取 phases.json，见 §11。
2. **`operation_graph.json` 改造**：去掉**大阶段内部的人为线性边**（如 ② 内 eas→enumeration 若判定为人为顺序），只保留 ①跨大阶段的边 + ②真实 `requires_stages` 依赖边（见 §8）+ ③ bail-to-reporting 短路。仍是 12 节点、仍无环、仍向后兼容。
3. **`operation_graph.rs` + 新 phase 层**：加载 phases.json；提供 phase-aware 遍历——「当前大阶段的全部成员小阶段同时可达；大阶段 done = 成员 gate 全 PASS」。`AllowedDag` 增加 phase 视图（phase 至少含一个 allowed stage 才出现）。
4. **审批触发点**（`harness/stage_transition.rs` / `pre_action_authorizer` 接线）：把 `human_approval` 从「每 stage 触发」收敛为「大阶段边界触发一次」；同一大阶段内多个 stage 声明同一 approval key 时 **de-dup**（只弹一次）。
5. **运行时游标**（orchestrator 的 `drive_stage_transition` 内联路径 + graph-flow 引擎路径，两路共用 DAG/branch_target）：改为 phase-aware——大阶段内小阶段任意序推进，全 PASS 才把游标推进到下一大阶段。
6. **Gate 不动**（D5/甲）：`validate_stage_gate` 仍逐 stage 跑；证据账本交叉、vacuous、freshness 全保留。
7. **前端**：在 chat / timeline 展示 phase 分组（折叠 + 进度）。复用既有 `WorkflowProgress` + 本人此前已合入的 `StageMarker`（commit `5fe447d`）。若 phase 透到前端需新增 TS 类型，走 `ts-rs`（I5）。
8. **Profile 不动结构**：`allowed_stage_kinds` 仍 stage 粒度（向后兼容）；大阶段可见性由投影派生。

---

## 7. 数据流（一次 run 怎么走）

```
进入 ① prep（准备与被动情报）
  └ scoping · target_intel 任意序推进 → 各自交 StageDeliverable → 各自 gate PASS
  └ 段内若要扩范围 → scope_expansion 审批（事件触发）
  └ 大阶段 done = 两者 gate 全 PASS
→ 入口审批 active_scan（主动扫描前，弹一次）→ 进入 ② active_recon（主动侦察）
  └ external_attack_surface · enumeration 推进（顺序按 §8 重审结果）→ 各自 gate PASS
  └ 大阶段 done = 两者 gate 全 PASS
→ 入口审批 exploit_validation（真打前，弹一次）→ 进入 ③ vuln（漏洞与利用）
  └ vuln_triage → verification（真实依赖，§8 保留顺序）→ 各自 gate PASS
→ （仅红队）④ post_exploit ...
→ ⑤ closeout：reporting（+ 红队 cleanup）
```

任意大阶段中途「无收获」可走 bail 边直接到 reporting（保留现有短路语义）。

---

## 8. 并行性与 `requires_stages` 重审（关键子任务）

「阶段内并行」会撞上现有 `requires_stages` 硬依赖（均已读 stage JSON 核实）：
- ① prep：`target_intel.requires_stages`（被动情报）与 scoping 是否真有先后？大概率可并行（待 §8 重审）。
- ② active_recon：`external_attack_surface.requires_stages = [scoping, target_intel]`（都在 ① 段，跨阶段依赖天然满足）；`enumeration.requires_stages = [external_attack_surface]` → ② 段内 eas→enumeration 是否真实依赖待判（端口枚举是否非得先有攻击面测绘结果）。
- ③ vuln：`verification.requires_stages = [vuln_triage]`（先识别后利用）——**真实依赖，保留串行**。

因此并行不是「无脑去掉所有边」，而是**逐条重审 `requires_stages`**，分类为：
- **真实数据依赖**（下游必须用到上游产物）→ 保留，仍串行（如 verification←vuln_triage）。
- **人为顺序**（历史排版，实则可并行）→ 解除。

落地：复用已有的 `docs/design/2026-06-02-stage-spec-worksheet.csv` 工作表，逐边填「真实依赖 / 人为顺序」，据此重写 operation_graph.json 的边集与各 stage 的 `requires_stages`。

### C 重审结论（2026-06-03 执行 · 已读全部相关 stage JSON 实证）
逐条 `requires_stages`（实证值）+ 是否跨 phase + 分类：

| 边 | 实证 | 位置 | 分类 | 处理 |
|---|---|---|---|---|
| target_intel ← scoping | `["scoping"]` | prep 内 | real（先确认范围再被动情报，inherits scope_rule） | 保留 |
| external_attack_surface ← scoping,target_intel | `["scoping","target_intel"]` | prep→active_recon 跨 phase | real（跨 phase 天然满足） | 保留 |
| enumeration ← external_attack_surface | `["external_attack_surface"]` | **active_recon 内** | **存疑**（端口/目录枚举可在已知种子主机上与子域测绘并行；也可视为需 eas 先产 http_service） | **本期保留，标记为唯一并行候选，待用户/安全复审** |
| vuln_triage ← enumeration | `["enumeration"]` | active_recon→vuln 跨 phase | real | 保留 |
| verification ← vuln_triage | `["vuln_triage"]` | **vuln 内** | real（先识别后利用） | 保留 |

**结论**：当前 2-stage 分组下，唯一可争取的 intra-phase 并行是 active_recon 的 `eas ∥ enumeration`，但它涉及「枚举是否非得先有攻击面测绘结果」这一安全语义，属改变 gate 依赖契约，**不在本批静默改**（遵守 §2.7「改依赖边先确认」）。本批 `operation_graph.json` **不删边**；该并行候选待用户拍板后单独落（届时改 enumeration 的 requires + operation_graph 的 eas→enumeration 边）。其余 phase 内边为真实依赖，保留串行。

> 旁证（de-dup 设计被实测验证）：`active_scan` 审批同时声明在 target_intel / external_attack_surface / enumeration 三个 stage 上 —— phase 级 entry_approval 正好把它们合并成「跨入 active_recon 前弹一次」。

---

## 9. 风险、回滚、灰度

- **flag 灰度**：新增 `GOLISH_HARNESS_TWO_LEVEL`（或复用现有 harness flag）。关闭 = 回退到现状线性 DAG + per-stage 审批；开启 = phase-aware。回滚 = 关 flag。
- **向后兼容**：operation_graph.json 仍 12 节点；profile 投影逻辑不变；gate 不动 → 证据契约（I7）与 vacuous（I8）不受影响。
- **DB**：phase 是**拓扑/编排层**，不引入新持久化结构；若需记录「当前大阶段」，仅存 `phase_id` 字符串（向后兼容地加字段，遵守 I10 先扩字段）。
- **审批收敛风险**：从「每 stage 可审批」收敛到「大阶段边界对齐授权跃迁」，要确保不会把某个高危 stage 的审批漏掉——以 §5 映射 + de-dup 规则覆盖；测试用例显式校验 `active_scan`/`exploit_validation` 两道边界审批在 pentest 主路各触发一次且仅一次，`scope_expansion` 在扩范围事件时触发。

---

## 10. 不变量遵守（AGENTS.md §5）

| 不变量 | 影响 | 处理 |
|---|---|---|
| I4 命名 | 不新增 Tauri command | 不涉及 |
| I5 ts-rs 同步 | phase 若透前端 | 新增类型走 `#[derive(ts_rs::TS)]`，不手维护两份 |
| I6 设计走新文件 | 本文件 | 新建，不覆盖 explainer |
| I7 证据交付 | gate 不动 | 保留 |
| I8 已查为空≠未查 | vacuous 不动 | 保留 |
| I10 schema 向后兼容 | 若加 phase_id 列 | 先扩字段、再上代码 |

---

## 11. 开放问题（待 review 拍板）

- **O1** phase 定义放 `phases.json`（本设计取）还是 stage JSON 加 `phase` 字段？
- **O2（随 O3=乙 一并解决）** scoping 与 target_intel 同组成 ① prep（不再单独成段，也不并入主动侦察）；与 explainer §7「侦察段」的差异即此。
- **O3（已定 = 乙，2026-06-03）** target_intel(被动 L1) 归入 ① 准备段；② 主动侦察只含 eas+enumeration(L2)。三道审批对齐授权跃迁。§5/§6/§7/§8 均已按乙更新。
- **O4** 真正并发执行（非仅去顺序）何时做、做到什么程度？

---

## 12. 验证计划

- **单测**：phases.json 加载/校验/投影；phase-aware `next`（成员同时可达）；大阶段 done = `AND(成员 gate PASS)`；同一边界审批 de-dup（只弹一次）；assessment 投影后 phase 数正确（①②⑤）。
- **集成**：一次 recon run 走 ① prep（scoping·target_intel 任意序）→(active_scan)→ ② active_recon（eas·enumeration）→(exploit_validation)→ ③；bail-to-reporting 仍可达。
- **真机**：重跑 example.com 外部攻击面侦察，盯 `~/.golish/backend.log`：`active_scan`/`exploit_validation` 两道边界审批各触发一次（`scope_expansion` 仅在扩范围时触发）、② 段内无人为强制顺序、gate 仍逐 stage PASS、游标按大阶段前进。
- **门禁**：`just precommit` 全绿（fmt + lint + 前后端 test）。

> 没有新鲜验证证据，不宣称完成（AGENTS.md §3）。
