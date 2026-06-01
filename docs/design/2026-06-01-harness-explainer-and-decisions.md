# Operation Harness 讲解 + 决策工作表

> 目的：在动手"逐阶段定义放行标准"之前，先把整套机制用**大白话 + 文件位置**讲清楚 —— 什么是阶段、每个东西定义在哪个文件、每个阶段怎么定义、哪个 AI 能用什么工具、现在实现到什么程度、**每个阶段你需要决定什么**。
>
> 这不是"设计决策文档"（那是下一份）。这是"现状地图 + 决策清单"，看完它你才能有依据地拍板。
>
> 证据来源：`resources/harness/**`、`backend/crates/golish-agent-kit/src/harness/**`、`task_orchestrator/**`、`golish-sub-agents/**`，以及 2026-06-01 对实时 DB（operation_state / stage_runs）的取证。

---

## 1. 先搞懂：什么是「阶段 / 节点」（大白话）

一次渗透测试**不是一下子搞定的**，而是**分成有先后顺序的好几步**来做。

- 每一步 = 一个**阶段（stage）**
- 在流程图上把这一步画成一个圈 = 一个**节点（node）**
- **阶段和节点是同一个东西**，只是一个从"流程"角度叫、一个从"图"角度叫，别被两个词搞晕。

**生活类比（看病）**：先确认看哪 → 问病史 → 体表检查 → 抽血化验 → 找出病因 → 确诊 → 开报告。不能跳着来（没化验就确诊是乱来）。渗透测试一样。

**你的 pentest 实际就这 7 步**（拿 example.com 举例）：

| 步 | 阶段 | 大白话：这步干嘛 |
|---|---|---|
| 1 | scoping | 先确认"我被允许打 example.com 吗、打哪些" |
| 2 | target_intel | 查公开情报：whois、它有哪些 IP/域名 |
| 3 | external_attack_surface | 找它对外暴露的入口：子域名、能访问的网站 |
| 4 | enumeration | 对这些入口细查：开了哪些端口、什么服务 |
| 5 | vuln_triage | 在这些服务上找漏洞（扫描、识别） |
| 6 | verification | 挑出的漏洞，受控地验证"真能利用吗"（这步会真打，要你批准） |
| 7 | reporting | 把发现写成报告 |

**为什么非要分阶段？**
1. **顺序依赖**：第 5 步找漏洞，前提是第 3、4 步先知道有哪些主机/服务。跳步就是瞎打。
2. **安全控制**：越往后越危险（第 6 步开始"真打"）。分阶段你才能在危险那步**卡一道闸**让 AI 停下等你批。

> "12 个阶段"是**全集**（含红队的横向移动、内网渗透那些）；你日常的 pentest profile **只用上面 7 个**，另外 5 个被禁了。

---

## 2. 这些东西都定义在哪个文件（速查表）

| 你想改 / 想看的东西 | 在哪 | 类比 |
|---|---|---|
| **这次用哪几个阶段**（选 7 个 / 禁 5 个） | `resources/harness/profiles/<profile>.json` 的 `allowed_stage_kinds` | 点餐：从菜单勾这次要哪几道 |
| **每个阶段本身的规则**（工具/放行标准/审批） | `resources/harness/stages/<阶段名>.json`（12 个文件） | 每道菜的菜谱 |
| **总共能有哪些阶段**（代码总清单） | `backend/crates/golish-agent-kit/src/harness/types.rs` 的 `enum StageKind`（12 个） | 这家店会做的全部菜 |
| **阶段之间的先后顺序**（DAG） | `resources/harness/graph/operation_graph.json`（节点+边） | 上菜顺序 |
| **每个 sub-agent 能用什么工具 / 能叫谁** | `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs` | 每个工种的工具包 |
| **主控 vs 子任务整体工具面** | `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs` | 谁能进哪些库房 |
| **gate（放行闸）校验逻辑** | `backend/crates/golish-agent-kit/src/harness/gate/` | 质检员的检查单 |

> 一句话：**profile 选阶段，stages/\*.json 定义阶段，types.rs 列全部阶段，operation_graph.json 排顺序。**

---

## 3. 七个核心概念（是什么 + 在哪 + 现状）

### 3.1 Profile（交战画像）
- **是什么**：一次任务的"总开关"。决定这次允许走哪些阶段、授权上限、哪些动作要审批。
- **在哪**：`resources/harness/profiles/*.json`（pentest / red_team / assessment / cloud_assessment / bug_bounty）。
- **关键字段**：`max_authorization`（授权天花板，如 `controlled_exploit`）、`allowed_stage_kinds`（允许阶段）、`forbidden_stage_kinds`、`approval_policy`、`cleanup_required`、`evidence_required`。
- **现状**：✅ 已实现。

### 3.2 Operation DAG（阶段流转图）
- **是什么**：所有阶段之间"能不能从 A 走到 B"的有向图。profile 在它上面投影出"这次可达子图"。
- **在哪**：`resources/harness/graph/operation_graph.json`（12 节点 + 边）。
- **现状**：✅ 加载+投影+选下一阶段已实现。⚠️ 分支选择策略（多下家时怎么选）需复核。

### 3.3 Stage Spec（阶段规格）
- **是什么**：单个阶段的"合同/菜谱"。详见 §4。
- **在哪**：`resources/harness/stages/*.json`（12 个）。
- **现状**：✅ 字段齐。⚠️ `required_checks`/`min_invocations` 大多是占位（见 §4）。

### 3.4 Gate（放行闸 / 证据校验）
- **是什么**：阶段做完后，确定性 Rust 代码按 stage spec 校验 AI 交的 deliverable，PASS 才推游标进下一阶段，BLOCK 就打回重做。
- **在哪**：`golish-agent-kit/src/harness/gate/`。
- **7 类 check**：结构性永远跑（schema / contract / vacuous / freshness）；语义按 `required_checks` 选跑（scope / surface_coverage / min_invocations）。
- **现状**：✅ 实现+有单测。⚠️ 2026-06-01 前从不触发（已修）。

### 3.5 Deliverable（阶段交付物）
- **是什么**：AI 每阶段结束要交的结构化 JSON（claims / findings / evidence_refs），是 gate 的唯一输入。
- **在哪**：类型 `StageDeliverable`（`harness/types.rs`）。
- **现状**：✅ 类型完整。⚠️ 提交链之前断裂（已修为"末尾吐 ```json 块"）。

### 3.6 Evidence Ledger（证据台账）
- **是什么**：本应是所有工具产出落库的地方，每条有 `evidence_audit_id`，deliverable 靠这些 id 引用真实证据。
- **现状**：❌ **未实现**。`evidence_audit` 表没建，gate 目前只能信 AI 自报的 evidence_refs，不能交叉验证。**最大未落地块。**

### 3.7 Human Approval（人工审批）
- **是什么**：高危动作/阶段切换前阻塞，等你回"批准"才放行。你**手动**控制的闸。
- **两层**：profile 级 `approval_policy`（如 before_active_scan）+ 阶段级 `human_approval.required_before`（如 verification before exploit_validation）。
- **现状**：✅ 机制实现。⚠️ 因 gate 之前不触发，从没被触发过。

---

## 4. 每个阶段怎么定义（stage spec 字段逐个）

每个阶段 = `resources/harness/stages/<名>.json` 一个文件。拿最简单的 **scoping** 逐字段大白话：

| 字段 | scoping 的值 | 大白话：管什么 |
|---|---|---|
| `id` / `kind` | scoping | 阶段名 |
| `risk_level` | low | 风险级别 |
| `requires_stages` | [] | 进来前必须先完成哪些阶段（它是第 1 个，无前置） |
| `allowed_next_stages` | [target_intel] | 通过后能去哪 |
| `allowed_tools` | query_target_data, log_operation… | 这阶段**只准用**这些工具 |
| `forbidden_tools` | dns_resolve, http_probe, exploit… | **明令禁止**的工具（scoping 不准任何探测） |
| `required_checks` | [scope_status_present] | ⭐**放行标准**：gate 查什么才放行 |
| `min_invocations` | {} | ⭐**最少必须跑**哪些工具几次（scoping 不需要） |
| `human_approval.required_before` | [scope_expansion] | 做哪个动作前要你人工批 |
| `deliverable_schema` | StageDeliverable | 要交的结构化交付物 |
| `inherits_evidence_from` | [] | 从上游哪些阶段继承证据 |
| `max_other_skips` | 2 | 最多容忍几个"其它原因跳过"的检查 |
| `agent_continuity` | single_session | 同一会话内连续执行 |

⭐ 两个带星的（`required_checks` + `min_invocations`）就是**"放行标准"**的核心。

**对比"定义得好"的样子** —— `external_attack_surface.json`：
- `required_checks` 有 6 条（scope_status_present / evidence_non_empty / unchecked_distinct_from_checked_empty / out_of_scope_targets_excluded / min_tool_invocations_per_check / surface_workbench_coverage）
- `min_invocations`：`dns_resolve≥1, http_probe≥1, subdomain_enum_passive≥1`（必须真跑这 3 个）

→ scoping 的放行标准很单薄（只 1 条），eas 的就具体。**你逐阶段要做的，就是把每个 stage.json 的这两块从占位改成像 eas 这样的真标准。**

### 现成的 required_checks 选项（gate 已实现的）
- `scope_status_present` / `out_of_scope_targets_excluded` → 走 scope_check
- `surface_workbench_coverage` → 走 surface_coverage_check（攻击面覆盖度）
- `min_tool_invocations_per_check` → 走 min_invocations_check
- `evidence_non_empty` / `unchecked_distinct_from_checked_empty` → 已被 schema/vacuous 覆盖

> 注意：写在 `allowed_tools` / `min_invocations` 里的工具名，必须是真实存在的工具（见 §5），否则会变成"幽灵工具"（`submit_stage_deliverable` 之前就是这毛病）。

---

## 5. 哪个 AI 能用什么工具 / sub-agent（分 4 层）

### 层 1 · 按角色/深度选工具 —— `execution_mode/modes/task.rs`
- **主控 primary（depth=0）= 只编排**：只能看到 `sub_agent_*` 派发工具 + `ask_human`，没有文件/shell/pentest 工具。
- **子任务 subtask（depth>0）= 全套工具箱**：文件/shell/pentest/bridge 全开，还能继续派子代理。

### 层 2 · 工具总目录 + 开关 —— `golish-agent-kit/src/tool_definitions.rs` + `tool_policy.rs`
所有工具定义、预设（minimal/standard/full）、启停、只读模式、URL/路径黑名单。

### 层 3 · 渗透阶段的工具闸（per-stage） —— `stages/*.json` 的 `allowed_tools`/`forbidden_tools`
执行时由 `agentic_loop/turn/phases/tool_dispatch.rs` 的 `gate_tool_call_for_dispatch` 拦。

### 层 4 · 每个 sub-agent 的工具箱 + 能派谁（最核心） —— `golish-sub-agents/src/defaults/builder/registry.rs`
每个用 `SubAgentDefinition::new(name).with_tools([...]).with_delegatable_agents([...])` 定义。当前 13 个：

| sub-agent | 能用的工具（节选） | 能派谁 |
|---|---|---|
| pentester | pentest_run, run_pipeline, flow_compose, manage_targets, record_finding, graph_*, search_exploits, vault, auth_probe… | coder/researcher/memorist/installer/enricher/browser |
| coder | read_file, list_files, grep_file, ast_grep(_replace) | — |
| researcher | web_search, web_fetch, knowledge, ingest_cve, save_poc… | memorist |
| installer | read/write_file, web_fetch, pentest_list_tools, pentest_run | researcher/memorist |
| browser | js_collect, js_extract_apis, web_*, record_finding | — |
| memorist | search/store/list_memories, graph_*, knowledge | — |
| adviser | web_*, read_file, memories, knowledge | researcher/memorist |
| reporter | read/write_file, memories, knowledge, poc_stats | memorist |
| enricher | memories, knowledge, graph_*, search_exploits | — |
| orchestrator/planner/refiner/reflector | 内部流水线用，**不暴露给 LLM** | — |

> ⚠️ 层 1（角色工具面）和层 3（harness 阶段 allowed_tools）是**两套独立过滤**，目前没完全打通 —— 规划时要拉通：`stage.allowed_tools` 里写的工具名，必须真的在某 sub-agent 的 `with_tools` 里存在。

---

## 6. 当前实现状态总表

| 组件 | 状态 | 说明 / gap |
|---|---|---|
| Profile | ✅ 已实现 | 5 个 profile |
| Operation DAG | ✅ 已实现 | ⚠️ 分支选择策略需复核 |
| Stage Spec（字段） | ✅ 已实现 | ⚠️ required_checks/min_invocations 多为占位 |
| Gate（7 check） | ✅ 已实现 | ⚠️ 之前不触发（已修）；min_invocations 近似 |
| Deliverable 提交 | ✅ 已修 | evidence_refs 仍是 AI 自报 |
| Backfill 贴 stage | ✅ 已修 | 词序无关；总是打 tagged X/Y |
| Human Approval | ✅ 已实现 | ⚠️ 从未被触发过 |
| Evidence Ledger | ❌ 未实现 | evidence_audit 表未建 |
| operation_state 游标 | ✅ 已实现 | 之前永远卡 scoping（已修，待活体验证） |
| stage_runs 记录 | ⚠️ 空 | 表在，从没写过行 |

---

## 7. 12 个阶段逐一：用途 + 现状 + 你要决定什么

> `[pentest]` = 当前 pentest profile 会用；`[仅红队]` = pentest 禁用、只有 red_team 用。

### A. 侦察段（低/中风险，建议自动流转）
- **① scoping**（ROE/授权边界）`[pentest]` low —— 确认授权范围，不准任何探测。现仅 `scope_status_present`。**你要决定**：算"完成"需 AI 证明什么（目标清单确认 / 授权登记 / in+out scope 列明）？要不要人工确认范围？
- **② target_intel**（被动情报）`[pentest]` low —— whois/ASN/DNS。min: dns_resolve≥1。**你要决定**：必须收集哪些情报项？
- **③ external_attack_surface**（外部攻击面）`[pentest]` medium —— 子域名/DNS/HTTP 探测。目前定义最完整。**你要决定**：覆盖度标准是否符合预期？
- **④ enumeration**（主动枚举）`[pentest]` medium —— 端口/服务/目录。before active_scan 需审批。**你要决定**：主动扫描前要不要你批？必跑哪些？强度上限？

### B. 漏洞段（高/危，建议从这里开始人工把关）
- **⑤ vuln_triage**（漏洞识别）`[pentest]` high —— 非破坏性识别+分级。**你要决定**：放行需要什么？最少确认几条？
- **⑥ verification**（漏洞验证/受控利用）`[pentest]` critical —— 受控 PoC，**before exploit_validation 强制审批**。**你要决定**：利用前一定要你批吗（强烈建议是）？验证成功要什么证据？允许到什么程度（只读验证 vs 拿 shell）？

### C. 红队段（critical，pentest 默认禁用，仅 red_team）`[仅红队]`
- **⑦ access_validation ⑧ internal_discovery ⑨ objective_pathing ⑩ objective_simulation ⑪ cleanup** —— 拿到访问后的横向/纵深/目标达成/清理。放行标准都是占位；cleanup 没有"清理是否真做了"的强校验。**你要决定**：这次要不要开红队 profile？每个阶段的达成/清理验收标准是什么？

### D. 收尾
- **⑫ reporting**（报告）`[pentest]` low —— 从证据汇总报告，任何阶段可提前跳来收尾。**你要决定**：报告放行需要什么（每条 finding 可追溯 evidence + 整改建议）？

---

## 8. 你要做的决策清单（工作表）

### 全局（先定，是总纲）
- [ ] **D0 控制姿态**：AI 自动跑到哪、哪里停等你批？（A 一进 verification 就停 / B 只最危险才停 / C 每阶段都批 / D 全自动）
- [ ] **D0.1 默认 profile**：日常用 pentest（7 阶段）还是别的？
- [ ] **D0.2 证据台账**：现在就把 evidence_audit 表落地（让 gate 能真校验），还是先用"AI 自报 evidence_refs"跑通闭环？

### 每个阶段（逐阶段填 stage contract，主要工作量）
1. 进入前置（requires_stages 够不够）
2. 允许 / 禁止工具（现有对不对、工具名真存在吗）
3. **放行标准**：必跑哪些工具×几次（min_invocations）+ 必须产出什么证据/claim/finding（required_checks）
4. 要不要人工审批、卡在哪个动作前
5. 通过后允许去哪（allowed_next_stages）

---

## 9. 建议推进顺序
1. 你定 **D0 / D0.1 / D0.2** 三个全局决策。
2. 我据此起草 `docs/design/2026-06-01-harness-stage-contracts.md`（含"每阶段 contract 表"骨架）。
3. 从 **scoping** 开始，一阶段一阶段填 contract（每填一个你确认一个）。
4. contract 定稿后转 writing-plans 出实现计划：改 stage spec json + 落地 evidence ledger + 补 gate 真校验。

> 下一步只需要你回 **D0 控制姿态**（A/B/C/D）即可启动。
