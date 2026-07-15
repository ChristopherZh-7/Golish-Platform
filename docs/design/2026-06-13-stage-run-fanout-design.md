# Stage Run — 通用「按 org 并行执行一个阶段」的 chat 形态

> Execution ownership 与多 Agent 调度部分由
> `docs/design/2026-07-14-stage-run-multi-agent-team-scheduler.md` 取代；本文件保留为早期
> chat/UI/fan-out 决策历史，不能再据此实现共享 lease 的嵌套 sub-agent。

> 状态：草案（brainstorming 产出，待用户审查）
> 关联：`docs/design/2026-06-13-engagement-scoping-fanout-redesign.md`（engagement / fan-out 上层设计）、
> `docs/superpowers/plans/2026-06-13-engagement-phase-bc-worker-pool.md`（Phase B/C 工人池）。
> 适用对象：开发 Golish 的 agent。先读本文件再动手。

## 0. 一句话

把 CLI 已验证的 **`--stage-run --include-subsidiaries`（一个阶段按 org 逐个/并行执行、各过各的 gate）** 带进 chat：主 agent 对当前阶段**派出一个「stage 管理者 sub-agent」**（动作 `stage_run`），它在底下按 org 并行铺开收集、统一汇报；**主 agent 按 DB 真值证据过门，过不了把缺口丢回这个管理者自纠**（门禁闭环）。chat 里只留**一张紧凑卡片**，点开在**左侧详情面板**看「每个 org 一个该阶段专家、并行、各自证据过 gate」的实时视图。**通用**——12 个阶段共用同一套机制 + 视图，每阶段只填配置，不写新工具/新前端。

## 1. 背景与动机（为什么要做）

- 现状（chat）：单会话进 target_intel，主 agent 委托 **Pentester** 子 agent 跑被动收集（subfinder/dns）。两个问题：
  1. **Pentester 不应承担信息收集**——它该只做后续攻击/渗透；收集应由「Recon」专家做。
  2. 单会话只收「一个 org」，不会把 1 母 + 10 子**逐个**收（那是 CLI `--include-subsidiaries` 才有的行为）。
- 现状（CLI，已验证）：`stage_run/mod.rs` 在 scoping 过后，母 org 跑完该阶段切片 → `filter_child_orgs` 逐个子 org 各跑一遍（串行、按 org 隔离、各过 gate、失败隔离、engagement 聚合「EVERY org must pass」）。
- 目标：把 CLI 那套**正常逻辑**（逐 org 执行 + 每 org gate）变成 chat 里**可视、AI 驱动、并行**的形态，并且**对所有 12 个阶段通用**。

## 2. 关键决策（已与用户确认）

| # | 决策 | 结论 |
|---|---|---|
| D1 | 颗粒度 | **每个 org 一个执行单元**（11 家各一个），符合 methodology「每 root 一次」+ CLI 逐 org。**不是** per-asset/per-subdomain（methodology 明禁逐子域 dig）。 |
| D2 | 层级与执行单元 | 主 agent → **stage 管理者 sub-agent**（管本阶段）→ **per-org 收集（= 该阶段专家：intel=Recon、EAS=Prober、攻击=Pentester…）** → 工具。fan-out = 管理者把「当前阶段专家」按 org 复制 N 份并行。 |
| D3 | worker vs sub-agent | 用 **sub-agent**（嵌套、**不开新 tab**），不是独立 worker 会话。钻入用现有「详情视图」。 |
| D4 | Pentester 定位 | **退出信息收集**；只做后续攻击阶段。收集专家叫 **Recon**（从 Pentester 拆出）。 |
| D5 | 布局 | chat 里**只一张紧凑卡片**；点它 → **左侧 detail 面板**显示整套并行视图。复用现有 `detailViewMode`（`SubAgentInlineCard` 已是「卡 → setDetailViewMode → PaneLeaf 渲染详情」）。 |
| D6 | 触发 / 运行 | 主 agent 对当前 stage **派出一个「stage 管理者 sub-agent」**（动作名 `stage_run`，charter 提示）。管理者管本阶段、fan-out per-org、统一汇报。**不是**主 agent 逐个调 sub_agent（会冒 N 张卡）。 |
| D7 | 通用性 | **一个机制 + 一个视图 + 每阶段配置**，12 阶段共用。**不写 12 个工具/管理者代码**。 |
| D8 | 命名 | **stage run**（对齐 CLI `--stage-run`）。动作/工具 `stage_run`、详情模式 `"stage-run"`、视图 `StageRunView`。 |
| D9 | gate | **每个 org 各过各的 gate**（确定性 + DB 真值，按 org 隔离——现有能力）。全过 → 进下一阶段；任一 BLOCK/FAIL 隔离、聚合报缺口。 |
| D10 | 主 agent 角色 | **主 agent = engagement 大脑（模型 A）**：scoping/授权、阶段推进、跨 org 优先级、对聚合结果决策、人机交互、最终综合。它**派出 stage 管理者**并**过门**，不微操 per-org。模型 B（DAG 确定性自动串、几乎无 LLM 主脑）作为未来「全自动模式」，本设计不实现。 |
| D11 | 编排闭环 | **主 agent 门禁闭环**：派出 stage 管理者 → 等汇报 → 按 DB 真值证据过门 → 过则进下一 stage；**不过则把缺口发回同一个 stage 管理者 sub-agent 自纠**（只重跑缺口 org → 再汇报，循环到过或 `ask_human`）。每个 stage 都走这套，通用。 |

## 2.5 编排模型：stage 管理者 sub-agent + 主 agent 门禁闭环（模型 A）

主 agent 不被架空，而是**上移成 engagement 级大脑**；每个 stage 交给一个**管理者 sub-agent**自管，主 agent 用**门禁闭环**收口。脑力上移、机械活下沉，是干净的分工：

- **主 agent（engagement 大脑）只做判断重的事**：
  - scoping / 授权：纠名、建 org 树、范围确认
  - 阶段推进与跳过；跨 org/跨阶段策略（intel 完了按薄弱度漏斗**挑哪些 org 优先打**）
  - **门禁闭环**：派出 stage 管理者 → 等汇报 → 按 DB 真值证据过门 → 过：进下一 stage；不过：把缺口发回该管理者自纠
  - 人机交互（`ask_human`、回答用户、汇报）；最终综合（报告）
- **stage 管理者 sub-agent（每阶段一个）**：`stage_run` 派出它；它在底下**按 org 并行 fan-out**专家收集、聚合、**统一汇报**给主 agent；收到主 agent 的缺口反馈后**只重跑缺口 org**再汇报。
- **per-org 专家（Recon/Prober/Pentester…）**：管理者底下，干本 org 本阶段的活；必要时再委托助手（Browser/Adviser）。
- **层级**：主 agent → stage 管理者 sub-agent → per-org 专家 → 工具。
- **模型 B（未来可选）**：harness DAG 确定性串各阶段、几乎无 LLM 主脑——更省更简单，但丢失上面那些自适应判断，作为后续「auto mode」，本期不做。

## 3. 架构

### 3.1 后端：stage 管理者 sub-agent + 分阶段专家

- **`stage_run` 动作 / 工具**（命名遵循 `<domain>_<verb>_<object>`，最终命名见实现）：主 agent 调它 = **为当前阶段派出一个「stage 管理者 sub-agent」**。
  - 入参：无 / 可选 `concurrency`（默认读当前阶段 + engagement org 树）。
  - 管理者行为：读「当前阶段」+ org 树 → 构造 per-org 单元 → 以并发 K 起每个 org 的**当前阶段专家**收集 → 各自按 org 隔离执行该阶段切片、book 证据、过本 org gate → **聚合并统一汇报**给主 agent。
  - **门禁闭环**：主 agent 按 DB 真值证据过门；不过 → 把缺口（哪些 org/技术没过）发回**同一个管理者** → 管理者只重跑缺口 org → 再汇报，循环到过或主 agent 决定 `ask_human`。
  - 复用：`stage_run/mod.rs` 的逐 org/隔离/聚合语义（`filter_child_orgs` / `build_child_objective` / per-org `set_harness_org_id` / 聚合「EVERY org must pass」）。
- **分阶段专家（sub-agent）**：把 `golish-sub-agents` 里 Pentester 承担的 recon 角色拆成独立 **Recon** 专家（带 `recon_*` + 被动 `pentest_run` + `manage_targets` + `submit_stage_deliverable`）。后续阶段同理映射（EAS=Prober…）。**stage → 管理者 + 专家** 是配置，不是每阶段写代码。

### 3.2 阶段配置（每阶段只声明，不写代码）

每个阶段 JSON 规格（`resources/harness/stages/<stage>.json`，已存在）已含：允许工具、覆盖契约、gate、最低调用、阶段切片。新增两项：

- `specialist`: 该阶段的专家角色 slug（intel→`recon`）。
- `coverage_axis`: 展示用的覆盖技术列表（intel→`[DNS,WHOIS,ASN,CT,SUBDOMAIN,OSINT]`；EAS→端口/服务/指纹/…）。

stage 管理者与视图读这两项分派/渲染，**无每阶段代码**。

### 3.3 前端：卡片 + 详情视图（通用）

- **`StageRunCard`**（chat 内联，复刻 `SubAgentInlineCard` 调性）：一张卡，显示阶段名 + 专家 chip + 进度（covered/active/blocked）+ 进度条；点击 `setDetailViewMode(sid, "stage-run")`。
- **`detailViewMode: "stage-run"`** → `PaneLeaf` 渲染 **`StageRunView`**（已落地的真组件，coverageAxis/roleLabel/stageLabel 均为 props）：每个 org 一张专家卡（状态字形 + 专家 chip + org 名 + 实时活动行 + `coverage_axis` 逐格 + 证据数 + gate），点某行**原地内联展开**该 org 的工具流（无新 tab）。
- 状态来源：管理者运行时发的进度事件（每 org 的状态/覆盖/证据/活动）→ store slice → 卡片与详情双订阅（运行时态叠加 DB 真值，复用 engagement §10 思路）。

## 4. 数据流

```
stage charter ──提示──▶ 主 agent 调 stage_run（派出当前 stage 管理者 sub-agent）
   │
   ▼
stage 管理者 sub-agent：读当前阶段 + org 树 → 构造 per-org 单元
   │ 并发 K，fan-out
   ├─▶ 专家@org1 (recon_enrich→subfinder→gau→填coverage→submit) → org1 gate(DB真值)
   ├─▶ 专家@org2 …                                              → org2 gate
   └─▶ …                                                        → orgN gate
   │  进度事件(每 org: status/coverage/evidence/activity)
   ▼
store(stage-run slice) ──▶ StageRunCard(摘要) + StageRunView(每org详情)
   │  统一汇报
   ▼
主 agent 按 DB 真值证据过门
   ├─ 全过 → 进下一阶段（再 stage_run 派下一个管理者）
   └─ 有缺口 → 把缺口发回同一管理者 → 只重跑缺口 org → 再汇报（闭环，直到过 / ask_human）
```

## 5. 错误处理

- 单 org `blocked`/`failed`：**隔离**，不连累兄弟；详情该行标红/黄；管理者聚合在卡片与汇报里报缺口。
- 门禁不过：主 agent 把缺口发回管理者自纠（闭环）；多轮仍不过 → `ask_human`。
- 工具/事件异常：详情视图三态（loading/error/empty）；可重试单 org。
- AI 未调 `stage_run`：charter 强约束 + 兜底（进入阶段一段时间无 stage_run → 提示/降级单会话收集）。

## 6. 风险与开放问题

- **并行机制**：stage 管理者在底下并行起 N 个 per-org 专家——后端编排（新增并发能力）还是复用前端池（`runPool.ts`）？倾向后端编排（与 CLI 语义一致、不依赖前端在场），需确认 sub-agent 并发隔离（每个独立 bridge/session）+ 管理者如何收集子专家进度。
- **成本**：K 个 org × LLM loop（设计 §11.2 风险）+ 每阶段一个管理者 LLM 层（比「纯工具」模型多一层）。K 默认 3 可调；被动收集机械，未来可对部分阶段降级为确定性（非本设计目标）。
- **门禁闭环回灌**：主 agent 把「哪些 org/技术没过」结构化发回管理者，管理者要能只重跑缺口（而非整阶段重来）——需定义缺口反馈的数据形状。
- **charter 强制度**：靠 prompt 让 AI 必调 `stage_run`，需要兜底防漏调。
- **specialist 拆分影响面**：从 Pentester 拆 Recon 要核对现有委托/路由规则（`orchestration.rs` mandatory_routing_rules）+ 工具清单测试。

## 7. 不变量对照（AGENTS.md §5）

- I2/IDOR：per-org 隔离（`set_harness_org_id` + gate 按 org），批量同样隔离。
- I4：命令命名 `<domain>_<verb>_<object>`。
- I5：跨 IPC 新类型（StageRun 进度 DTO、缺口反馈 DTO）用 `ts-rs` 同步前端。
- I7/I8：阶段交付必须有证据；「已检查为空(checked_empty)」≠「未检查」。
- I9：事务内不调外部 HTTP/长耗。

## 8. 测试

- 纯函数：per-org 单元构造、coverage 映射、聚合/缺口判定、StageRunView 行模型（单测）。
- 管理者 / 工具：stage_run 派管理者、fan-out N、按 org 隔离、聚合、失败隔离、缺口回灌只重跑缺口（集成/可行则单测）。
- 前端：StageRunCard / StageRunView 渲染冒烟 + 卡→详情切换。

## 9. 分期（YAGNI：先通用骨架 + intel 接入）

1. **通用骨架**：`stage_run`（派 stage 管理者 sub-agent）+ 管理者 per-org fan-out + 主 agent 门禁闭环 + `StageRunCard` + `detailViewMode:"stage-run"` + `StageRunView` + stage config 两字段。
2. **intel 接入**：Recon 专家拆分 + `target_intel.json` 填 `specialist/coverage_axis`；端到端跑通平安 11 家。
3. 后续阶段（EAS/enumeration…）：只填配置 + 必要的专家映射，0 新工具/前端。

## 10. 现有可复用件（避免重复造）

- 视图：`frontend/components/Engagement/StageRunView.tsx` + `StageRunCard.tsx`（已落地的真组件，预览 `StageRun.preview.tsx` / `?preview=intel`）、`SubAgentInlineCard`（卡调性）、`PaneLeaf`/`detailViewMode`（详情面板，待接 `"stage-run"` 模式）。
- 后端：`stage_run/mod.rs`（逐 org/隔离/聚合）、`golish-sub-agents`（专家/管理者体系）、现有 per-org gate + DB 真值覆盖判定。
- 阶段规格：`resources/harness/stages/*.json` + `*.methodology.md`。
