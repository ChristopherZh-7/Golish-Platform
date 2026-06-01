# Operation Harness · 拓扑层参考（Profile + DAG）

> 目的：把 harness 的**拓扑层**——两个紧耦合的概念 **Profile（交战画像）** + **Operation DAG（阶段流转图）**——讲清楚：各是什么功能、现在定义了哪些、每个字段什么意思、它俩怎么投影成「这次任务能走的路」。这是一份**现状参考**（reference），不是设计决策文档。
>
> 证据来源（均已逐一核对真实文件）：
> - `harness/profile.rs`（Profile 结构体 + AuthorizationLevel + loader）
> - `resources/harness/profiles/*.json`（5 个 profile）、`harness/resources.rs`（`EMBEDDED_PROFILE_IDS`）
> - `commands/mode.rs`（picker 列举/选择）、`commands/core/chat.rs`（选中→orchestrator）、`task_orchestrator/orchestrator.rs`（启动建 operation_state）
> - `harness/operation_graph.rs` + `resources/harness/graph/operation_graph.json`（DAG 加载/校验/投影/流转）
> - `harness/stage_transition.rs`（gate 后流转决策）
>
> 配套：节点层（单个 Stage spec）见 `2026-06-02-harness-stage-spec-reference.md`；总览见 `2026-06-01-harness-explainer-and-decisions.md`。日期：2026-06-02。

---

## 0. 这一层是什么

拓扑层回答一个问题：**这次任务能走哪些阶段、按什么顺序走**。它由两个紧耦合的东西组成：

- **Profile**（滤镜）：选哪些阶段 + 授权上限 + 审批策略
- **Operation DAG**（底图）：所有阶段 + 阶段间允许的流转方向

核心公式：

```
Profile（选哪些阶段） × DAG（排顺序/边） = 可达子图（这次真正能走的路）
                                              ↓ 子图里每个节点
                                       挂一份 Stage spec（下一层，见 2026-06-02-harness-stage-spec-reference.md）
```

本文 **Part A** 讲 Profile，**Part B** 讲 DAG，**Part C** 讲它俩怎么投影落地 + 完整性 + 现状。

---

# Part A · Profile（交战画像）

## A1. 这是什么功能

**Profile = 一次 operation（= 一个 Task）的「总开关」**。它不干活，只**框定边界**：能走哪些阶段、授权到什么程度、哪些动作要人工批、收尾要不要证据/清理。它通过 4 个杠杆起作用：

| 杠杆 | 运行时效果 |
|---|---|
| `allowed_stage_kinds` / `forbidden_stage_kinds` | **投影 DAG**（见 Part C）：决定这次能走哪些阶段 |
| `max_authorization` | **授权天花板**：每个 tool 调用前比较 `tool_required_level <= authz.rank()`，超限即拒 |
| `approval_policy` | **人工闸开关**：哪些动作（active_scan / scope_expansion）前阻塞等人批 |
| `cleanup_required` / `evidence_required` | 收尾是否强制清理 / 是否强制要证据 |

> Profile **不定义阶段本身**（阶段干啥/怎么验收在 `stages/*.json`），它只**点名引用**阶段名。

## A2. 字段逐个含义

对应 Rust 结构体 `Profile`（`harness/profile.rs`）。serde 默认忽略未知字段，所以 `$schema` / `$comment` 不报错。

| 字段 | 类型 | 含义 | 运行时效果 |
|---|---|---|---|
| `id` | string | profile 唯一标识 | 必须和文件名、`EMBEDDED_PROFILE_IDS`、picker id 一致 |
| `display_name` | string | 人类可读名 | 显示在前端 picker |
| `max_authorization` | enum（6 档，见 A3） | 授权天花板 | 每个 tool dispatch 前拦超授权工具 |
| `allowed_stage_kinds` | `Vec<StageKind>`（强类型） | 这次允许走的阶段 | 投影 DAG 的「保留节点集」；写错阶段名解析失败 |
| `forbidden_stage_kinds` | `Vec<StageKind>` | 明令禁止的阶段 | 文档化意图（投影按 allowed 为准） |
| `approval_policy.before_active_scan` | bool（默认 false） | 主动扫描前是否要批 | 触发 `waiting_approval` 阻塞 |
| `approval_policy.before_scope_expansion` | bool（默认 false） | 扩大范围前是否要批 | 触发 `waiting_approval` 阻塞 |
| `cleanup_required` | bool（默认 false） | 收尾是否强制清理 | 仅 red_team=true |
| `evidence_required` | bool（默认 false） | 是否强制要证据 | 当前 5 个全=true |

## A3. 授权天花板：AuthorizationLevel 6 档（L0–L5）

定义在 `harness/profile.rs`，每档有 `rank()`（L0=0..L5=5），比较即 `tool_required_level <= authz.rank()`。

| 档 | 枚举值（JSON snake_case） | rank | 含义 |
|---|---|---|---|
| L0 | `observe_only` | 0 | 仅查现有数据，无探测 |
| L1 | `passive_intel` | 1 | 仅被动收集（公开库 / passive DNS / CT log） |
| L2 | `active_recon` | 2 | 低风险探测（HTTP probe / DNS / 主动子域枚举）— assessment 顶 |
| L3 | `vuln_validation` | 3 | 非破坏性漏洞验证 — bug_bounty / cloud_assessment 顶 |
| L4 | `controlled_exploit` | 4 | 受控 exploit 验证 — pentest 顶 |
| L5 | `post_exploit_red_team` | 5 | 横移 / 后渗透 — red_team 顶 |

## A4. 现在定义了哪 5 个 profile

| profile | display_name | 授权顶 | 阶段数 | active_scan 需批 | cleanup_required | 关键特征 |
|---|---|---|---|---|---|---|
| `assessment` | Security Assessment | L2 active_recon | 5 | 是 | 否 | **当前默认**；只侦察，禁 vuln/verify |
| `bug_bounty` | Bug Bounty | L3 vuln_validation | 6 | **否** | 否 | 到 vuln_triage 止，禁 verification |
| `cloud_assessment` | Cloud Assessment | L3 vuln_validation | 6 | 是 | 否 | 阶段同 bug_bounty，差在 active_scan 需批 |
| `pentest` | Pentest | L4 controlled_exploit | 7 | 是 | 否 | 到 verification（受控利用），禁红队 5 段 |
| `red_team` | Red Team | L5 post_exploit_red_team | 12 | 是 | **是** | 全开（forbidden 空），强制清理 |

各 profile 的 `allowed_stage_kinds`：

- **assessment（5）**：scoping · target_intel · external_attack_surface · enumeration · reporting
- **bug_bounty（6）**：上面 4 个侦察段 + vuln_triage + reporting
- **cloud_assessment（6）**：与 bug_bounty 阶段集相同（差异只在 before_active_scan）
- **pentest（7）**：bug_bounty 6 个 + verification
- **red_team（12）**：全部 12 个阶段

> 观察：`bug_bounty` 与 `cloud_assessment` 阶段集完全一样，唯一区别是 `before_active_scan`——说明「同样阶段集靠 approval_policy 也能拉开差异」。

## A5. 怎么定义 / 注册 / 自动上架

1. **JSON**：`resources/harness/profiles/<id>.json`（字段见 A2）
2. **Rust 类型**：`harness/profile.rs` 的 `struct Profile` + `enum AuthorizationLevel`（一般不动）
3. **注册（内嵌）**：`harness/resources.rs` 的 `EMBEDDED_PROFILE_IDS`（`include_str!` 编译进二进制，新 id 要加进数组）
4. **自动上架 picker**：`commands/mode.rs` 的 `list_execution_modes()` 遍历 `EMBEDDED_PROFILE_IDS` 自动出一项——**加 JSON + 加 id 就够，前端零改动**

## A6. 前端选中 → 运行时链路

```
picker 选 <profile_id>
  → set_execution_mode(id)                      (commands/mode.rs)
        · "chat"        → Chat 引擎，无 profile
        · "<profile_id>"→ Task 引擎 + 该 profile
        · "task"(legacy)→ Task 引擎 + env 默认 profile
  → bridge.set_harness_profile(Some(id))
  → 起任务时 chat.rs: orchestrator.set_profile_override(get_harness_profile())
  → orchestrator.run():
        profile = profile_override ?? active_profile_id()(env) ?? "assessment"
        operation_state.insert(task_id, profile, scoping)   ← 起点 scoping
```

要点：用 picker 选 `pentest` 是**真生效**的；「默认 assessment」只在「没选 / legacy task / env 没设 / 选了未知 id」时兜底。

---

# Part B · Operation DAG（阶段流转图）

## B1. 这是什么功能

DAG 定义「**所有阶段 + 阶段间允许的流转方向**」。它**只管拓扑**：有哪些节点、谁连谁；**不管**阶段内部干啥、不管授权/审批（那是 Stage spec / authorizer / approval）。

- 静态定义：`resources/harness/graph/operation_graph.json`（`nodes` + `edges` 两个数组）
- 运行引擎：`harness/operation_graph.rs`（加载、校验、投影、算下一 stage）

## B2. 现在 nodes + edges 长啥样

**12 节点**（= 12 个 `StageKind`），**15 条边**（11 条线性 + 4 条早退到 reporting）。

主链（线性骨架）：

```
scoping → target_intel → external_attack_surface → enumeration
  → vuln_triage → verification → access_validation
  → internal_discovery → objective_pathing → objective_simulation → cleanup → reporting
```

4 条早退边（bail-to-reporting）：`external_attack_surface` / `enumeration` / `vuln_triage` / `verification` 各有一条 `→ reporting`。设计意图：任何阶段都能提前收尾出报告（现状见 B5）。

## B3. 加载即校验（保证它真的是 DAG）

`load_operation_graph_from_json` 读入后立刻 `validate()`：

1. **每条边两端都必须在 `nodes[]`** —— 否则 `UnknownNodeInEdge`
2. **无环**（Kahn 拓扑排序）—— 否则 `Cycle`

所以图保证是合法有向无环图；非法图在启动期就报错，不会带病运行。

## B4. gate 之后怎么决定去哪（decide_transition）

`stage_transition.rs` 的 `decide_transition(current, gate_allowed, dag)` 是**纯确定性**决策：

| 情况 | 决定 | 含义 |
|---|---|---|
| gate 没过 | `Hold` | 留当前 stage 返工 |
| 过 + 0 个后继 | `Complete` | operation 完成（终点） |
| 过 + 1 个后继 | `Advance(s)` | 直接推进到 s |
| 过 + N>1 个后继 | `Branch(候选)` | 多下家，候选按**边声明顺序** |

`next_stages(current)` 给的就是沿边可达的候选，顺序 = `operation_graph.json` 里边的声明顺序。

## B5. ⚠️ 分支策略现状（你标的「需复核」根因）

虽然 `decide_transition` 会区分 `Branch`，但运行时实际推进游标调的是 `advance_target()`，它对 `Branch` **直接取 `candidates.first()`（第一候选）**（代码注释明写：「Phase 1 默认取第一候选；后续由 agent / 策略选」）。

而边声明顺序里，「**继续深入**」的边总排在「**bail→reporting**」之前（例：`eas → [enumeration, reporting]`，enumeration 在前）。结论：

> **现状 = 永远走第一条边 = 永远继续深入，永远不会自动提前 bail 到 reporting。**
> 那 4 条早退边目前**形同虚设**（除非人工干预或未来加策略/agent 选择）。

这就是「分支选择策略需复核」的根因。**待决策**：要不要让 agent/规则来选分支？什么条件下该 bail 到 reporting（比如没找到攻击面就别硬往下）？

## B6. 入口与终点

- `entry_points()`：无入边的节点（operation 起点候选）
- `terminals()`：无出边的节点（终点）
- 例：assessment 投影后 entry = `scoping`，terminal = `reporting`

---

# Part C · Profile × DAG 怎么落地 + 完整性 + 现状

## C1. 投影规则（project）

`graph.project(profile.allowed_stage_set())`：

- **节点**：只留 `allowed_stage_kinds` 里的
- **边**：只留**两端都在** allowed 的（任一端被 forbidden 连带剪掉）

## C2. 三个 profile 投影对照（真实数据）

| profile | 投影后节点数 | 边数 | 备注 |
|---|---|---|---|
| 全量（base） | 12 | 15 | 不投影时的全图 |
| assessment | 5 | 5 | `enumeration→vuln_triage` 因 vuln_triage 被剪而消失 |
| pentest | 7 | 9 | `verification→access_validation` 被剪；保留 verification→reporting |
| red_team | 12 | 15 | 全开，等于 base |

```
pentest 投影：
scoping → target_intel → eas → enumeration → vuln_triage → verification → reporting
（外加 eas/enum/vuln_triage 各自的 →reporting 早退边；verification→access_validation 被剪）
```

## C3. 完整性约束（容易踩的对齐问题）

同一个阶段名出现在四处，**必须对齐**：

```
types.rs 的 StageKind 枚举 ↔ operation_graph.json 的 nodes ↔ stages/<名>.json 文件名 ↔ profile 的 allowed_stage_kinds
```

- `allowed_stage_kinds: Vec<StageKind>` 与 DAG 的 `nodes/edges` 都是**强类型**（`StageKind`）：写一个不存在的阶段名 → serde 解析失败 → 加载失败回退。所以**阶段名不会变「幽灵」**。
- DAG 还多一层 `validate()`：边端点不在 nodes、或成环，都在加载期拒绝。
- 但若 profile 允许某阶段、`stages/` 里却没对应 JSON → 走到那阶段载 spec 会失败。新增/改阶段时四处同步。
- 对照：阶段内部 `allowed_tools` 是**纯字符串**（不校验工具是否真存在），写错会变「幽灵工具」——这是拓扑层之外、Stage spec 里的另一类隐患（详见节点层文档）。

## C4. 现状与待决策

- **默认 profile = assessment**：不显式选/设就只做侦察（到不了 vuln_triage/verification）。待决策：默认是否改、是否「记住上次选择」。
- **⚠️ 分支策略 = 取第一候选**（B5）：早退边未被自动启用。待决策：分支怎么选、何时该 bail。
- **拓扑层本身**：✅ Profile 加载 / DAG 加载+校验 / 投影 / 流转决策 / picker 链路均已打通。
- 与拓扑层关系较弱、但同属 harness 的缺口（`evidence_audit` 未建、`stage_runs` 空）见 `2026-06-01-harness-explainer-and-decisions.md`。
