# 2026-06-05 · 攻击面「上限」提升方向：让 AI 真会打，而不只是过 gate

> **Status**: Direction Draft（方向记录，非实现计划；每个杠杆落地前按 AGENTS.md 走 design → plan → TDD → `just precommit`）
>
> **承接**：`docs/design/2026-06-05-coverage-matrix.md`（下限·覆盖矩阵 gate）、`docs/design/2026-06-05-vuln-triage-technique-matrix.md`（下限·技术矩阵 + 分母）、`docs/design/2026-05-20-agent-harness-strategy.md`（总策略）
>
> **一句话**：gate / coverage / `expected_techniques` 保的是**下限**（防偷懒、防造假）；本文记录**上限**（让 AI 成为更强的攻击者）的方向。

---

## 1. 核心认知：gate 是判别器，不是生成器

| | 下限（floor） | 上限（ceiling） |
|---|---|---|
| 是什么 | 约束 / 校验 | 生成 / 能力 |
| 载体 | gate、`coverage_complete`、`coverage_denominator`、`expected_techniques` | 知识 + 工具 + 推理深度 + 上下文 + 攻击链 + 持续学习 |
| 性质 | 确定性，只 PASS/BLOCK，只能「拒绝不合格」 | 概率性，靠喂给 AI 的「弹药」 |
| 极限 | best case = AI 正好做到及格线 | 取决于弹药厚度与编排质量 |

- **推论**：拧 gate 永远只动下限；提上限要投另一套子系统。
- **设计自证**：覆盖矩阵设计已写明「矩阵是**地板**（最低必测），不是天花板」。

---

## 2. 当前已有的「上限」接缝（2026-06-05 核码实况）

| 机制 | 文件锚点 | 状态 |
|---|---|---|
| wiki PoC/writeup 注入 prompt | `task_orchestrator/subtask_phases/execute.rs:123`（`retrieve_wiki_prior(title)` + `render_prior_knowledge`） | ✅ 已接（但**只 wiki** + query 粗） |
| stage charter / 继承证据 / 上游 handoff | `execute.rs:104-117` + `task_orchestrator/prompts/` | ✅ 已接 |
| 图谱 prior 检索 | `harness/rag_prior.rs::retrieve_graph_prior` | ⚠️ 已实现·**未接**（orchestrator 无 graph handle） |
| 发现写回图谱（持续学习） | `harness/rag_prior.rs::feed_findings_to_graph` | ⚠️ 已实现·**只在测试**·未接 stage 流 |
| 攻击链查询 | `find_attack_paths`（`golish-graphiti` client + graph tool + `graph_bridge`） | ⚠️ 已实现·**图谱没人喂 → 闲置** |

> 关键判断：上限的「半成品」已经不少，瓶颈在**接线 + 喂数据 + 内容厚度**，不是从零造轮子。

---

## 3. 七个杠杆（按 ROI 排序）

### 杠杆 1 · 知识厚度 + 精准检索（最大杠杆，半成品在跑）
- 上限 ∝ wiki/PoC 库厚度 × 检索准度。
- 动作：① 把库喂厚（n-day PoC / HackerOne writeup / 每类技术的深度打法）；② 检索 query 从 `planned.title` 换成「指纹 + 技术类」（如 `Struts2 OGNL RCE`）。
- 锚点：`execute.rs:123`、`rag_prior.rs`、wiki KB。
- ✅ **不依赖同事资产库**。

### 杠杆 2 · 图谱 prior + 持续学习（复利）
- 给 orchestrator 传 graph handle → 接 `retrieve_graph_prior`；stage 收尾调 `feed_findings_to_graph`。
- 效果：本次发现喂回图谱，下阶段 / 下任务能检索到 → 跨任务复利。
- 锚点：`rag_prior.rs` P3-b / P3-c。

### 杠杆 3 · 攻击链（scanner ↔ pentester 的分水岭）
- 地板矩阵逐格独立判定；真进攻是**串格子**：信息泄露 → 凭据 → 越权 → RCE。
- 动作：① 图谱喂满（靠杠杆 2）；② 攻击阶段把 `find_attack_paths` 的路径喂给 agent 做规划。
- 锚点：`find_attack_paths`（graphiti）。

### 杠杆 4 · 工具深度
- 固定 `allowed_tool_types` 白名单之外，让 AI **受控地**自己拼 payload / 写一次性脚本，而非只调固定工具。

### 杠杆 5 · 迭代深度
- inner loop 的假设驱动 / 反思 / 回溯质量（PentAGI 式 subtask / refine）：打不通就换路子，而非一遍过。

### 杠杆 6 · 动态向上扩 expected
- 按指纹自动加技术类（看到 GraphQL → 加 GraphQL 专项；Struts → 加 Struts CVE 链），超出静态地板。
- 与 coverage-matrix Phase 2 ③「skeleton 动态生成」同源，但方向是**向上扩**而非补全下限。

### 杠杆 7 · 严重度 / 新颖度记分
- gate 只记「完整度」（够不够全）；再加一个**基于证据**的严重度 / 链深信号，把 AI 往**高处**拉，而不只是越过及格线。
- 注意：必须证据驱动，**不能 LLM 自评**（防幻觉自夸）。

---

## 4. 反直觉但关键：地板会封顶上限

地板拧太死（巨型强制矩阵 / 死清单）→ AI 去「逐格打勾」而非「挖深洞」→ **反而封顶**（即 coverage-matrix 设计点名的「矩阵爆炸 / 被淹没」风险）。

**原则**：gate 当**精简安全网**，不当**紧身衣**；留巨大空间让 AI 超出地板，并用杠杆 7 **奖励它超出**。

---

## 5. 建议起步次序

1. **杠杆 1 + 2**（知识 / 检索 / 持续学习）：半成品在跑、ROI 最高、且**完全不依赖同事的资产库**（不卡 coverage Phase 2 的 DB 授权）。
2. 再上 **杠杆 3**（攻击链）——依赖杠杆 2 先把图谱喂起来。
3. **杠杆 6 / 7** 作为「把 AI 往高处拉」的增强，随后接。

> 每个杠杆落地按 AGENTS.md：先 `docs/design/` 细化 → `docs/superpowers/plans/`（writing-plans）→ TDD → `just precommit` 全绿 → `feature_list.json` 登记。本文只是**方向锚**，不是实现计划。
