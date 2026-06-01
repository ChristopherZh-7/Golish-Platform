# Operation Harness 重建设计（2026-06-01）

- **Status**: **Approved**（2026-06-01 用户授权拍板 §11：D1=A / D2=A 精练 / D3=A / D4=A / 文档位置 OK）。实施计划：`docs/superpowers/plans/2026-06-01-harness-rebuild.md`。
- **Author**: MCP-agent-2（原草案）；§12 re-anchor 校正 + §11 拍板 + §13 执行记录 by MCP-agent-1（2026-06-01）
- **Date**: 2026-06-01
- **Branch**: `feat/harness-2026-06-01`（从 `feat/recon-service` HEAD `936350a` 切出）
- **Supersedes（部分）**: 2026-05-26 五份 harness doc + 2026-05-20 plan 的「现状/优先级/crate 前提」部分。**保留**它们的 gate 哲学、Evidence Ledger schema、§21 工程不变量（除 F3/F4，见 §6）。
- **Source of truth（仍引用）**: `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` §21

> **一句话**：Phase 1 harness 代码**已落地、分层合法、测试全绿**，但 ① 从未运行验证 ② 领域模型已从 SecurityView 转向 target-centric Surface Workbench ③ §21 的 F3/F4 不变量被后续架构决定推翻。所以这是 **re-anchor + 补验证**，不是推倒重写。

> **2026-06-01 重要校正（见 §12）**：经磁盘实证，harness gate 逻辑**与数据模型无关**，且 `query_target_data` 已返回 Surface Workbench 数据源——"落后"程度比原草案估计的小，re-anchor 工作面很窄。

---

## 1. 背景：为什么"落后"

2026-05-26 实现 Phase 1 后，代码库发生两件大事，把 harness 的前提抽走了：

1. **crate-per-service 拆分（M0–M5）+ S1-2 端口化**：新增 6 个 app 服务 crate（`golish-app-core` L5.0 / `golish-{vuln,recon,pentest,agent,platform}-app` L5.5–5.6），跨服务 repo 耦合全走 `app-core/ports/`（ALLOWLIST 28→0）。
2. **领域 pivot（2026-05-28）**：删除 ZAP + 整个 SecurityView 外层面板，转向 **target-centric**：`organization → target → Target Surface Workbench`（六 tab：Identity / Surface / Sitemap / JS-API / Sensitive / Evidence）。

harness 的 MVP stage `external_attack_surface` 当初锚在旧 recon/SecurityView 模型上，现在那个模型没了。再加上 Phase 1 默认 OFF、手动 E2E 从未做过，loop 实际能不能跑通是未知数。

---

## 2. 现状盘点（磁盘实证 2026-06-01）

| 组件 | 位置 | 层 | 状态 |
|---|---|---|---|
| Evidence Ledger 核心（EvidenceLedger / Sanitizer / types / repo） | `golish-pentest/src/evidence_ledger/` + `evidence_sanitizer.rs` | **L2.0 lib** | 在·分层合法 |
| `evidence_read` 命令 + facade + registry | `golish-pentest-app/src/evidence.rs` + `golish/commands_facade/evidence.rs` | **L5.6 命令** | 在·分层合法 |
| Harness module（15 文件：gate 5 check + intent_classifier + sprint_contract + stage_harness + profile/stage_spec） | `golish-agent-kit/src/harness/` | **L4.1 toolkit** | 在 |
| Orchestrator 接入 | `golish-agent-kit/.../subtask_phases/execute.rs::apply_harness_gate_hook` | L4.1 | flag 默认 OFF，仅识别 `ExternalAttackSurface` |
| Schema migration | `golish-db/migrations/20260601000001_evidence_ledger.sql` | L2 | 已落 |
| Resources | `resources/harness/{profiles,stages,graph,evidence_kinds}.json` | — | 在 |
| 测试 | `harness::*` 88 unit + `e2e_tests` 10 | — | feature_list 记录全绿 |

**关键纠正**：Evidence Ledger 的「L2 lib 核心 + L5.6 命令层」分布是**正确分层**（与 servitization 一致），**不是** split-brain，**不需要搬**。harness（L4.1）`use golish_pentest::evidence_ledger`（L2）也是合法依赖（4.1 > 2.0）。因此本次重建**不提议**移动 evidence ledger 或 harness 的 crate 归属——避免无关重构（YAGNI）。

---

## 3. 重建决策（已拍板 · 见 §11）

> 每条给出选项 + 推荐 + 理由。**2026-06-01 用户授权全取推荐项**。

### D1 · 现有 Phase 1 代码处置 → **✅ A（re-anchor）**
- **(A) re-anchor【已选】**：保留代码，只解掉对已删领域模型的引用 + 重映射数据源。
- (B) 重写：丢弃 module 从零。
- (C) 删除回到设计阶段。

**理由**：代码分层合法、测试绿、DTO 设计扎实（`ExternalAttackSurfaceDeliverable` / `StageClaim` / `GateResult` / `HarnessRecoveryActions`）。重写浪费已验证资产，违反 YAGNI。

### D2 · MVP stage 锚点（核心 re-anchor 工作）→ **✅ A（精练版）**
旧 `external_attack_surface` 的 claims/evidence 锚在 SecurityView/recon 模型。新锚到 **Target Surface Workbench** 数据源（沿用现有 API，不改 schema）：`targetAssets / apiEndpoints / fingerprints / jsAnalysis / sitemap / sensitive`。

- **(A)【已选 · 精练】** stage "done" = `Surface`（端口/服务/指纹）+ `JS-API` 两类为**硬要求**（后端数据源经 `query_target_data` 确实存在）；`Sitemap` 为**软要求**（honest-empty 允许，但须显式落 `skipped_checks`，区分"已检查为空 vs 未检查"，对齐 I8）。
- (B) 仅 `Surface` 单 tab 即 pass（更窄、更快出 MVP）。
- (C) 六 tab 全覆盖才 pass（最严，但 MVP 过重）。

**精练理由（§12）**：`2026-05-28-target-surface-workbench.md` §8.6 明确 Sitemap/Sensitive tab 当前是 honest-empty（无保证后端数据源），把它设硬要求会让 gate 不可过。故硬 Surface+JS-API、软 Sitemap。

### D3 · 集成点 → **✅ A（保留 hook）**
- **(A)【已选】** 保留 `task_orchestrator` 末端 hook，但**先与正在进行的 legacy single-bridge 清理对齐**（2026-05-31/06-01 正在改 task/agent 路径），最小化改动。
- (B) 另起独立集成路径（脱离 task_orchestrator）。

### D4 · 验证策略 → **✅ A（validation-first）**
- **(A) validation-first【已选】**：**不加任何新 stage**，先 `GOLISH_HARNESS_STAGE_MODE=true` 把从未做过的手动 E2E 跑通，证明 loop 可用，再谈 Phase 2。
- (B) 边扩 stage 边验证。

---

## 4. 目标架构（重建后）

```text
[task 模式]
 用户 task
  └─ generator 给 subtask 标 harness_stage = external_attack_surface
       └─ subtask 执行（agent 调工具 → evidence 落 audit_log + evidence_classifications）
            └─ agent 交 deliverable JSON（ExternalAttackSurfaceDeliverable）
                 └─ StageHarness.validate_gate（6 确定性 check）
                      ├─ allowed  → 写 timeline（HarnessGateEvaluated）+ 进下一 subtask
                      └─ blocked  → recovery_actions 喂 refiner（≤3 重试 → paused_needs_user）
```

- **不搬** evidence ledger（L2 lib + L5.6 命令分层已正确）。
- harness 留 `golish-agent-kit`（L4.1 toolkit），由 `golish-agent-app`（L5.6）消费；跨服务读 evidence 走 `evidence_read` 命令（已有），需要时经 `app-core/ports`。
- profile / stage_spec / evidence_kinds 仍走 `resources/harness/*.json`（配置驱动，不硬编码）。
- **新增工作 = stage → Surface Workbench 数据映射层**（`harness/surface_mapping.rs` + 第 6 个 gate check `surface_coverage_check`，把 gate 的 claim/finding kind 对到 target-centric tab）。

---

## 5. 保留（不动·仍 sound）

- 确定性优先 **gate check**：schema / scope / contract / vacuous / freshness（反"agent 动了工具就算完成"反模式的核心价值）。2026-06-01 新增第 6 个 `surface_coverage`。
- **Evidence Ledger on audit_log**：bitemporal + IFC 三态（InScope / OutOfScope / DerivedFromOutOfScope）+ `scope_version` snapshot。
- **MCP Resource sanitizer**：evidence 不直接进 LLM 上下文，走 `evidence_read(eid)` 拿脱敏 structured summary（防 prompt injection）。
- **Sprint Contract**：确定性生成（P1），cross-vendor LLM 推后。
- **feature-flag 渐进并行**：默认 OFF，与旧路径共存。
- §21 工程不变量 **F1 / F2 / F5 / F6 / F7 / F8**。

---

## 6. 变更（re-anchor 清单）

| # | 旧 | 新 |
|---|---|---|
| C1 | §21 **F3**「不新增 4 个 crate」 | **作废**（servitization 已加 6 app crate）。改为：harness 不强行新建 crate，但允许归入既有服务 crate。 |
| C2 | §21 **F4**「不重构 task_orchestrator」 | **放宽**：legacy-bridge 清理正在动 task 路径，harness 集成需与之协调；仍最小化改动、不大改 orchestrator。 |
| C3 | MVP stage 锚 SecurityView/recon 模型 | 重映射到 Target Surface Workbench 数据源（D2 · 见 §12/§13 实现）。 |
| C4 | 5 份 doc + Gen1 分散 | 本文 supersede「现状/优先级/crate 前提」；§21 决策与 schema 仍引用。 |

---

## 7. 测试策略

- **回归**：先确认现有 `harness::*` 88 unit + `e2e_tests` 10 在本分支仍绿（`cargo nextest run -p golish-agent-kit -E 'test(harness::)'`）。
- **新增**：stage → Surface Workbench 映射单测（`surface_mapping` 6 + `surface_coverage_check` 4，pass + fail 各覆盖）。
- **手动 E2E（D4，从未做过）**：`GOLISH_HARNESS_STAGE_MODE=true` → 新建 target → task「评估 \<target\> 的 attack surface」→ 观察 ① Stage banner ② evidence_read 出现在 timeline ③ Gate decision JSON；④ 故意交 vacuous deliverable（不调工具）→ gate 必须 BLOCK + 出 recovery_actions。

---

## 8. 分阶段执行

| 阶段 | 内容 | 验证 |
|---|---|---|
| **P0** | 本设计确认 | ✅ 用户拍 §11 决策（2026-06-01） |
| **P1** | re-anchor：解 `external_attack_surface` 对旧模型引用 + 映射到 Surface Workbench 数据源 | ✅ 代码已落（§13）；现有测试 + 新映射单测待 `just check` 确认 |
| **P2** | validation：flip flag + 手动 E2E（证明 loop） | ⏳ **需运行时**（`just dev`）·见 plan Task 7 |
| **P3** | 文档收敛 + `feature_list.json` 加重建条目 + Superseded 标注（§10） | ✅ 进行中（§13） |
| **P4**（之后） | 扩第二 stage（`target_intel` 或 `enumeration`） | 另起 plan |

> P1 实施计划已落 `docs/superpowers/plans/2026-06-01-harness-rebuild.md`（writing-plans skill）。

---

## 9. 风险

| 风险 | 缓解 |
|---|---|
| legacy-bridge 清理与 harness hook 冲突 | D3-A 保留 hook、不改 orchestrator 签名，冲突面最小 |
| target-centric 数据源 schema 未稳 | 复用现有 `targetAssets/apiEndpoints/...` API，**不改 schema** |
| flag flip 后真实 LLM 产 deliverable 格式不稳 | 复用 `golish-json-repair`（旧 plan 风险 1 缓解） |
| 重建 scope 蔓延 | 严守 D4 validation-first：loop 没证明可跑前不加新 stage |
| Sitemap 硬要求致 gate 不可过 | D2 精练：Sitemap 软要求 + honest-empty skip 通道（§12） |

---

## 10. Superseded 标注

确认设计后给以下文件头部加**部分** supersede 注释（AGENTS.md §2.4；只 supersede「现状/优先级/crate 前提」，gate 哲学/Evidence schema/§21 仍有效）：
2026-05-20 plan + 2026-05-26 的 operation-harness-profile-dag-lab / stage-harness-mvp / evidence-ledger / mcp-resource / observability-plane / task-mode-refactor plan。

---

## 11. 决策记录（已拍板 2026-06-01）

| # | 决策 | 取值 |
|---|---|---|
| D1 | 现有代码 | **A · re-anchor** |
| D2 | MVP stage done 判定 | **A 精练**：Surface+JS-API 硬要求，Sitemap 软要求 |
| D3 | 集成点 | **A · 保留 task_orchestrator hook + 对齐 legacy 清理** |
| D4 | 验证 | **A · validation-first** |
| 文档位置 | `docs/design/` + `docs/superpowers/plans/` | **OK** |

---

## 12. Re-anchor 校正（磁盘实证 2026-06-01 · MCP-agent-1）

原草案 §1 称 harness「落后」是因为 MVP stage 锚在已删的 SecurityView/recon 模型。**逐文件核实后校正如下**：

1. **harness gate 逻辑与数据模型无关**：5 个 gate check（`schema/scope/contract/vacuous/freshness`）只吃 `ExternalAttackSurfaceDeliverable`（claims/findings/evidence_refs）+ Evidence Ledger（audit_log）。**没有任何一处 import/引用 SecurityView**。
2. **stage spec 的 `allowed_tools` 已是 target-centric**：含 `query_target_data / dns_resolve / subdomain_enum_passive / http_probe / fingerprint_target / shodan_query`——都写入 recon 表，不是 SecurityView。
3. **`query_target_data_impl`（`golish-agent-app/src/ai/db_bridge/recon.rs`）已返回 Surface Workbench 数据源**：`assets / endpoints / fingerprints / js_analysis / scan_logs`，正是六 tab 的后端。

**结论**：真实"锚在旧模型"的面**很窄**，只有两处：
- (a) `evidence_kinds.json` 缺 target-centric kind（只有 dns_a/ct_log/nmap... 等通用 recon kind）；
- (b) D2 的"stage done = Surface Workbench 覆盖"判定**从未编码**（contract_check 只比 finding 数量范围）。

因此 re-anchor = ① 补 evidence_kinds；② 新增 `surface_mapping` + `surface_coverage_check` 编码 D2——**不是**大改。`EvidenceKindRegistry` 是开放 map（跳过 `$` 键、未知 kind 走 7 天 fallback），加键安全，不破坏既有断言。

---

## 13. 本轮已执行（2026-06-01 · 代码落地，验证待 `just check` + E2E）

按 plan Task 1–6 落地（**P1 代码完成**，flag 仍默认 OFF）：

| Task | 文件 | 动作 |
|---|---|---|
| 1 | `resources/harness/evidence_kinds.json` | 补 6 个 target-centric kind（target_asset/fingerprint/api_endpoint/js_analysis/sitemap/sensitive_exposure） |
| 2 | `golish-agent-kit/src/harness/surface_mapping.rs` | **新建**：`SurfaceCategory` 映射 + `SurfaceCoverage` + `missing_required_categories` + `D2_REQUIRED_CATEGORIES`（6 单测） |
| 3 | `golish-agent-kit/src/harness/mod.rs` | 声明 + re-export surface_mapping |
| 4 | `golish-agent-kit/src/harness/gate/surface_coverage_check.rs` | **新建**：第 6 个 gate check（4 单测） |
| 5 | `gate/mod.rs` + `e2e_tests.rs` | 接进 6-check 流水线；happy fixture 补 1 个 JS-API finding |
| 6 | `external_attack_surface.json` + `stage_spec.rs` | `required_checks` 加 `surface_workbench_coverage`，count 断言 5→6 |

**待办**：① `just check` 全绿（本轮结尾跑一次）；② P2 手动 E2E（需 `just dev` 运行时，见 plan Task 7）；③ E2E 证据齐后 `feature_list.json` 对应条目转 `passing`。
