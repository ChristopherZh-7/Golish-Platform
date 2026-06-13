# Engagement：Scoping 前置 + 会话工人池 Fan-out（chat-native 批量红队）— 设计

> 日期：2026-06-13
> 状态：设计（设计级 · 已与用户对齐并批准方向，待审本文档 → writing-plans 出实现计划）。
> 总纲：`2026-06-12-redteam-db-truth-master.md`（Phase 0/1/2/3 地基）。
> 取代关系：本设计**取代** headless 批处理路线 `2026-06-12-engagement-fleet-orchestration.md`（该实现已 revert，存 `git stash`）。复用其调度内核与数据契约，但**入口与执行形态完全不同**（见 §9）。
> 不变量：AGENTS.md I2（IDOR/范围所有权）、I5（ts-rs 同步）、I7（阶段交付有 evidence）、I8（已检查为空 ≠ 未检查）、M2（前端走 `lib/api/<domain>.ts`，禁裸 invoke）、§2.7（schema/安全语义先确认）。

---

## 1. 一句话

把「一次红队打几十~上百家公司」做成 **chat-native** 流程：**scoping 抽成独立的人机互动前置阶段**（AI 跟人把范围定死），范围锁定后**按受控并发 fan-out 成多个真·独立 AI 会话**当工人；**信息收集按母公司家族一起做（保关联），到攻击阶段才拆成每 org 一个会话**。入口永远是 chatpanel，不是 CLI、不是独立 dashboard 页。

---

## 2. 背景与动机

### 2.1 为什么推翻 headless fleet

上一版 `engagement-fleet-orchestration` 把批量做成了：① `golish --fleet-run` CLI 入口 ② 一个 headless 进程内两波调度器 ③ 一个只读 dashboard 独立页面。用户的反馈（逐字）：

- 「命令行入口是为了方便测试，不是让你搞一个页面在前端。」
- 「我想要的是 chatpanel 比如 100 个公司的资产进去了，那么就可以分配 3 个 tab 或者 session 排队同步做数个。而不是搞个工具批量。」

即：批量编排应该是**前端 chat 原生的「会话工人池」**，复用现有多 conversation/tab 机制，而不是后端批处理工具 + 独立观察页。

### 2.2 为什么现有 chatpanel 撑不住（实读结论）

- `conversation ↔ terminal` 是 **1:1**（`store/slices/conversation.ts`：「each conversation has exactly one terminal」）。
- 同一时刻**只渲染 `activeConversationId` 那一个**会话；conversation tab 是顶部细横条、标题 `max-w-[120px]` 截断，为「人手开的少数几个 chat」设计。
- 故 N 个工人并行时只能一次看一个、来回点 tab，没有「N 路进度 / 队列剩余 / 每 org PASS·BLOCK」的总览。**chatpanel 需要新增一层「总览 + 分组」**，但总览要长在 chat 流里、不是断开的独立页。

---

## 3. 用户已确认的模型（brainstorming 结论）

| # | 决策 | 选择 |
|---|---|---|
| Q1 | 一个「工人」是什么 | **A**：每个工人 = 一个完整独立 AI 会话（各自 agent + 终端） |
| Q2 | 会话/公司/名单关系 | **C**：每家一个独立会话（共 N 个、同时只 K 个活、完成归档） |
| 入口 | 怎么发起 | **把 scoping 抽成独立的人机互动阶段**；不是新「指挥」概念 |
| Q5 | 看全局放哪 | **A**：scoping 对话定完范围后**自己升级成 engagement 总览** |
| 颗粒度 | 工人何时拆 | **甲 + 推迟到攻击**：信息收集按家族一起做；**攻击阶段才拆成每 org 一个会话** |

> Q2(C) 定的是方向「按公司、而非母公司汇总，来切会话」；「颗粒度」行进一步细化：这个「按公司拆」**从攻击阶段才生效**，recon 阶段仍按母公司家族一起跑（保关联）。二者是「方向 → 何时拆」的递进，不矛盾。

待用户最终确认的次级默认（本设计已替用户拟定，§9.3）：并发 K 默认 3；工人挂了标 FAILED、工位领下一个、收官汇总缺口。

---

## 4. 目标 / 非目标

**目标**
- scoping 成为可独立运行、可人机互动的前置阶段：纠名 → 建母 org → 议子公司 → 建 org 树（范围锁定）。
- 范围锁定后，前端按 K 受控并发把工作 fan-out 成真·独立 AI 会话；信息收集按家族、攻击按 org。
- scoping 对话原地升级为 engagement 总览：org 树 + 活跃 K + 队列剩余 + 每单元状态；点一行钻进对应工人会话；100 工人会话在 tab 区按 engagement 分组折叠。

**非目标**
- 不改 Phase 0 的 gate 判定逻辑（DB 真值权威对每 org 自动生效）。
- 不重做收集/门禁/证据引擎——复用既有 stage 链 + `stage_run::orchestrate`。
- 不保留 `--fleet-run` headless 入口为产品形态（仅在需要时留作测试/无头跑的旁路，见 §9）。
- 不做跨公司家族之间的关联收集（关联只在同一母公司家族内）。

---

## 5. 架构：三面一流 + 阶段化工人颗粒度

```
┌─ ① Scoping 面（交互前置 · engagement 级 · 一个 chat）────────────┐
│  你粘 N 个公司名                                                  │
│  AI: 企查查纠名/规范化(以企查查为准) → 建 N 个母 org              │
│      议「子公司纳不纳入 / 投资比例阈值」(缺数据回头问你)          │
│      发现子公司 → 挂母 org 下，建权威 org 树                      │
│  原则: 输入够就一路自动做完; 缺了才回头找人要                     │
│  产物: DB org 树(母+合格子) = 锁定的范围                          │
└───────────────────────────────┬─────────────────────────────────┘
                                 │  范围锁定 → fan-out（前端会话工人池，K 受控并发）
        ┌────────────────────────┴────────────────────────┐
        ▼                                                   ▼
② 信息收集（recon: target_intel→EAS→enumeration）   （家族之间独立、可并发）
   单元 = 母公司家族（母 + 其合格子 一起做）
   一个「家族 recon 会话」：母先收 → 子逐个收（复用 Phase 3 orchestrate，停在攻击前）
   保留 A/B 关联数据（共享基建/关联漏洞不被拆散）
        │ 家族 recon 过 gate
        ▼
③ 攻击 → 报告（per-org 拆分）
   单元 = 每个 org（母、每个子各一个「攻击会话」）
   各自打 → 各自过「最后报告 gate」→ 各自一份报告
   共享 recon 数据从 DB 按 org 读

④ Engagement 总览（scoping 对话定完范围后自己升级成这个）
   org 树按家族分组 | 活跃 K / 队列剩余 | 每单元 阶段·PASS·BLOCK·过没过最后 gate
   点一行 → 钻进那个 recon/攻击会话（tab 区按 engagement 分组折叠）
```

**阶段化工人颗粒度（核心创新点）**：工作单元随阶段演进——scoping 是 engagement 级（一个交互 chat），recon 是家族级（保关联），attack→report 是 org 级（并行 + 每实体独立报告）。

---

## 6. 组件分解

### 6.1 前端：会话工人池编排器（净新，主要工作量）

一个前端模块（store slice + 调度 hook），负责把 scoping 产物变成运行中的会话池：

- **队列**：从锁定的 org 树构造工作项。recon 工作项按家族（母+子一组）；attack 工作项按 org（recon 过 gate 后再入队）。
- **受控并发 K**：同时最多 K 个「活跃工人会话」（recon 家族 + attack org 不分种类共用 K）。一个会话过其 gate（recon gate / 最后报告 gate）→ 释放工位 → 出队下一个。
- **spawn 工人会话**：程序化创建 conversation（复用 `addConversation` + `createTerminalTab` + 自动 seed 初始任务 prompt + 自动开跑），不靠用户手敲。每个会话绑 org/family + 任务目标。
- **归档**：完成的工人会话从活跃区移到 engagement 分组的「已完成」，tab 折叠。
- **失败处理**：工人会话失败 → 标 FAILED、工位照常领下一个、收官汇总列缺口（不停摆）。

> 与旧 headless fleet 的本质差异：调度从「后端一个进程的两波 `buffer_unordered`」搬到**前端会话池**；运行时状态因为每个会话是 UI 里活的，**可直接可见**（不再像 headless 那样只能读 DB 真值、GUI 状态退化为 `pending`）。

### 6.2 后端：scoping 独立化 + 纠名（净新 + 复用 Phase 2）

- **scoping 作为可独立交互的阶段**：`--to scoping` 已是 stage 边界（现成）；本设计要让它在 chat 里人机互动跑、产物落 DB、可被前端读取为「范围已锁定」信号。
- **企查查纠名/规范化（净新一步）**：scoping 第一步对每个输入公司名跑企查查 → 取权威规范名（以企查查为准）→ 去重 → 建母 org。这是 Phase 2 之前没有的前置子步骤。
- **子公司发现 + 阈值筛 + org 树落库**：复用 Phase 2 设计（`recon_discover_subsidiaries` 企查查/TYC/KC + 投资比例阈值 + parent-child 落 `organizations`）。

### 6.3 后端：org-run 执行单元（复用 Phase 3，零改动）

每个工人会话内部跑 `stage_run::orchestrate(bridge, pool, session, profile, entry, allowlist, objective, Some(org_id), include_subsidiaries, threshold)`：

- recon 家族会话：`entry=target_intel`、`to=enumeration`（或攻击前最后一个 recon 阶段），母先收→子逐个收。
- attack org 会话：`entry=攻击阶段`、`to=report`，单 org。
- gate 按 org 隔离（Phase 0/06-10 已实现，DB 真值权威自动继承）。

### 6.4 调度内核与数据契约（复用 stash 里的逻辑）

stash 里这些是**与执行形态解耦的纯逻辑/契约**，可直接搬来给前端池或一个轻后端编排命令用：

- `scheduler.rs`：`OrgRunTask` / `OrgRunStatus`(Passed/Blocked/Failed/SkippedAlreadyComplete) / `FleetMode`(Checklist/Funnel) / `FleetConfig` / `order_tasks`（纯函数排序）/ 三个注入 trait（`OrgRunExecutor`/`OrgCompletionOracle` 续跑/`WeaknessScorer`）/ `scheduler_is_stage_agnostic` 守卫。
- `weakness.rs`：薄弱度评分 + `org_stage_has_truth`（续跑判定：DB 真值已覆盖→跳过）。
- `import.rs`：粘名单/CSV → 批量建根 org（scoping 的「建母 org」可复用）。
- `query.rs` + ts-rs 契约：`EngagementSnapshot`(projectPath/mode/rootCount/totalOrgs/covered/blocked/failed/tree) / `OrgTreeNode` / `OrgRunStatusDto`(passed|blocked|failed|skippedAlreadyComplete|**pending**) / `OrgWeaknessScore` —— 正好当总览的读模型契约。

---

## 7. 数据流

```
[Scoping chat] 粘 N 名单
   │ 企查查纠名 → organizations(母, 规范名)
   │ 议阈值 → recon_discover_subsidiaries → 按投资比例筛 → organizations(子, parent_id)
   ▼ scoping gate(DB 真值): org 树落库 → 范围锁定
[前端会话池] 读 org 树 → 构造 recon 家族队列
   │ K 受控并发: 每家族 spawn 一个 recon 会话 → orchestrate(target_intel..=enum, org_id=母, 含子)
   │   写 target_assets/dns_records/api_endpoints/... (按 org_id 归属，含子)
   ▼ 家族 recon 过 gate → 把该家族每个 org 入「attack 队列」
   │ K 受控并发: 每 org spawn 一个攻击会话 → orchestrate(attack..=report, org_id=该 org)
   │   读该 org 的 recon 真值 → 攻击 → 过最后报告 gate → 落报告
   ▼
[Engagement 总览] = 升级后的 scoping 对话: 读 EngagementSnapshot(DB 真值 + 池运行时态) 渲染
```

---

## 8. 复用映射 vs 净新

| 块 | 复用 | 净新 |
|---|---|---|
| org-run 执行 | Phase 3 `stage_run::orchestrate`（零改动） | — |
| 子公司发现/阈值/org 树 | Phase 2 设计 + `recon_discover_subsidiaries` | scoping 独立化编排 |
| 纠名 | — | 企查查规范名前置子步骤（待核 adapter 是否返规范名，见 §11） |
| gate / DB 真值 | Phase 0 + 06-10 org 隔离 | — |
| 调度逻辑/契约 | stash: scheduler 内核 / weakness / import / query / 4 ts-rs 类型 | 从 headless 改造为前端池驱动 |
| 工人 = 会话 | 现有 conversation/terminal/tab 机制 | 程序化 spawn + 自动 seed + K 池 + 分组折叠 + 总览 |
| 入口 | 现有 chatpanel | scoping 对话升级为总览 |

---

## 9. 与旧 headless fleet 的关系（取代 + 复用）

### 9.1 取代

- 旧：`--fleet-run` CLI 入口 + headless 两波调度进程 + 独立只读 dashboard 页。
- 新：chatpanel 原生（scoping 交互 → 前端会话池 fan-out → scoping 升级成总览）。
- 已 revert 全套到 `git stash@{0}`，可恢复。

### 9.2 复用

§6.4 列的调度内核 / weakness / import / query / ts-rs 契约是**与入口无关的纯逻辑**，从 stash 取回搬进新形态。

### 9.3 本设计替用户拟定的默认（§3 待最终确认）

- 并发 K 统一卡「同时最多 K 个活跃工人」（recon 家族 + attack org 不分种类），默认 3。
- 工人失败 → 标 FAILED、工位领下一个、收官汇总列缺口。
- recon 单元 = 母公司家族；attack 单元 = 每个 org。

---

## 10. 错误处理 / 边界态

- **scoping 缺数据**：回头问人（HITL），不臆造范围；范围未锁定不允许 fan-out。
- **企查查查不到/限流**：scoping 标该公司「纠名失败/待人工」，不静默丢弃（I8：区分「查了→无」vs「没查」）。
- **工人会话失败/BLOCK**：标 FAILED/BLOCK、释放工位、收官汇总缺口；不连累兄弟工人。
- **杀进程/重启续跑**：池启动前用 `org_stage_has_truth` 跳过 DB 已覆盖的单元（不重跑）。
- **三态 UI**：总览 + 每个工人 tab 的 loading/error/empty 都要画（AGENTS.md §2.3）。
- **IDOR/范围（I2）**：fan-out spawn 的每个工人会话绑 org_id，后端命令校验 org 归属当前 engagement，批量操作同样校验。

---

## 11. 风险 / 待验证

1. **企查查规范名字段**：必须核实 adapter（`golish-intel-providers` enscan/zone）返回里有「权威规范名 + 投资比例」可用——若无，纠名/阈值筛退化，需换源或调整（Phase 2 §6 已挂同款风险）。
2. **N 路真并发 LLM 成本/限流**：每个工人是真 AI 会话 → K 路 = K 倍 token/连接（kimi 429 教训）。K 默认保守（3），并发安全需 per-run bridge 隔离（stash FleetConfig 注释提到「concurrency 目标 2 需 per-run bridge 隔离后」）。
3. **100+ 工人会话的 tab 体验**：必须分组/折叠/虚拟化，否则 tab 区爆炸（连 Phase 4 前端虚拟化风险）。
4. **运行时态 vs DB 真值**：前端池可见运行时态，但刷新/重开后只能从 DB 真值重建（`OrgRunStatusDto.pending` 语义）——总览要能从 DB 重建 + 活会话覆盖。
5. **`sub_agent_models` 在 stage-run 下 override 失效**（Phase 3 §6 记录的前置 bug）：多 org 重 agent 跑前需确认已修，否则全压主模型。
6. **攻击/报告阶段边界**：需在 stage spec（`resources/harness/stages/*.json`）核实攻击/exploitation→report 阶段与「最后报告 gate」的实际定义（本设计按其存在编写，细节进 plan 核）。

---

## 12. 分期（多子系统，按依赖切）

> 本需求是多子系统活儿；建议分 3 期，每期独立可验、独立 commit。

- **Phase A — Scoping 独立化 + 纠名**：scoping 可在 chat 交互跑；企查查纠名前置；org 树落库 + scoping gate（复用 Phase 2）。产物：范围锁定信号可被前端读。**前置依赖：Phase 0/1/2 稳定。**
- **Phase B — 会话工人池 fan-out**：前端编排器（队列 + K 并发 + spawn 会话 + 续跑 + 失败处理）；recon 家族会话 / attack org 会话两类工人；复用 stash 调度内核 + `stage_run::orchestrate`。
- **Phase C — Engagement 总览 UI**：scoping 对话升级成总览（org 树分组 + 活跃/队列 + per-org 状态 + 钻入）；tab 分组折叠；复用 ts-rs 契约 + query 读模型。

---

## 13. 验证（DoD 雏形）

- **Phase A**：单测纠名去重 + 阈值筛纯函数；活体 `scoping`（含子公司）→ `organizations` 出现母+合格子规范名、gate 在 org 树落库后才 PASS、不带 include 时零回归。
- **Phase B**：调度内核纯单测（K 并发上限、完成出队、续跑跳过、失败不连累、stage-agnostic 守卫）；前端 spawn/归档/分组单测；活体 `母+2 子`→ recon 一个家族会话、attack 3 个 org 会话、K 限流生效。
- **Phase C**：前端总览渲染 + 钻入 + 分组折叠测试；三态 UI；从 DB 真值重建总览。
- 每期 `just precommit` 全绿 + 证据写 `agent-progress.md`。

---

## 14. 待用户拍板

1. 并发 K 默认值（拟 3）。
2. recon 单元 = 母公司家族、attack 单元 = 每 org（已口头确认，文档留痕）。
3. 工人失败处理（拟 FAILED + 领下一个 + 收官汇总）。
4. 分期顺序 A→B→C 是否可接受（A 依赖 Phase 0/1/2 稳定）。
5. 旧 stash 的复用块是「搬回主干」还是「重写」（拟搬回纯逻辑：scheduler/weakness/import/query/ts-rs 契约）。
