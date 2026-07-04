# 实现计划索引 · Implementation Plans Index

> **这是 `docs/superpowers/plans/` 的导航入口。** 实现计划是**按 `writing-plans` skill 写的分步执行计划**（point-in-time，日期前缀命名）。对应的设计决策在 [`../../design/`](../../design/INDEX.md)。
>
> 标题取自 slug；**状态**来自文内 `superseded` 标注的实读 grep（⚠️ = 文内含 superseded 关系）。按主题分组，组内按日期。计划是否落地以 [`feature_list.json`](../../../feature_list.json) 为准。

## 图例
- ✅ 计划在册 ｜ ⚠️ 文内含 superseded 关系 ｜ 🗂️ legacy（早于日期前缀约定、2026-06-07 从 docs 根归位、未注日期）

---

## 1. Agent Harness / Engine v2 / Stage / Gate / Plan-UX（核心）

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-20 | [golish-agent-harness-architecture](2026-05-20-golish-agent-harness-architecture.md) | ⚠️ |
| 2026-05-26 | [task-mode-refactor-to-harness](2026-05-26-task-mode-refactor-to-harness.md) | ⚠️ |
| 2026-05-27 | [sub-agent-nested-thinking-ui](2026-05-27-sub-agent-nested-thinking-ui.md) | ✅ |
| 2026-06-01 | [harness-full-impl](2026-06-01-harness-full-impl.md) | ✅ |
| 2026-06-01 | [harness-rebuild](2026-06-01-harness-rebuild.md) | ✅ |
| 2026-06-02 | [engine-v2-p0-evidence-loop](2026-06-02-engine-v2-p0-evidence-loop.md) | ✅ |
| 2026-06-02 | [engine-v2-p1-graph-checkpoint](2026-06-02-engine-v2-p1-graph-checkpoint.md) | ✅ |
| 2026-06-02 | [engine-v2-p2-metalcraft-graph-executor](2026-06-02-engine-v2-p2-metalcraft-graph-executor.md) | ✅ |
| 2026-06-02 | [engine-v2-p2-verification-eval-guardrails](2026-06-02-engine-v2-p2-verification-eval-guardrails.md) | ✅ |
| 2026-06-02 | [engine-v2-p3-rag-knowledge-graph](2026-06-02-engine-v2-p3-rag-knowledge-graph.md) | ✅ |
| 2026-06-03 | [harness-profile-driven-execution-p0](2026-06-03-harness-profile-driven-execution-p0.md) | ✅ |
| 2026-06-03 | [harness-profile-driven-execution-p1p2](2026-06-03-harness-profile-driven-execution-p1p2.md) | ✅ |
| 2026-06-03 | [lazy-per-stage-planning](2026-06-03-lazy-per-stage-planning.md) | ✅ |
| 2026-06-03 | [two-level-phase-stage-model](2026-06-03-two-level-phase-stage-model.md) | ✅ |
| 2026-06-04 | [per-stage-plan-cards](2026-06-04-per-stage-plan-cards.md) | ✅ |
| 2026-06-04 | [per-stage-plan-isolation](2026-06-04-per-stage-plan-isolation.md) | ✅ |
| 2026-06-04 | [plan-roadmap-ux-overhaul](2026-06-04-plan-roadmap-ux-overhaul.md) | ✅ |
| 2026-06-04 | [stage-internal-agent-todo-execution](2026-06-04-stage-internal-agent-todo-execution.md) | ✅ |
| 2026-06-04 | [task-resume-after-disconnect](2026-06-04-task-resume-after-disconnect.md) | ✅ |
| 2026-06-05 | [coverage-matrix](2026-06-05-coverage-matrix.md) | ✅ |
| 2026-06-05 | [gate-rule-engine](2026-06-05-gate-rule-engine.md) | ✅ |
| 2026-06-05 | [gate-rules-migration](2026-06-05-gate-rules-migration.md) | ✅ |
| 2026-06-05 | [remove-pipeline-feature](2026-06-05-remove-pipeline-feature.md) | ✅ |
| 2026-06-05 | [unified-ai-harness-observability](2026-06-05-unified-ai-harness-observability.md) | ✅ |
| 2026-06-05 | [vuln-triage-technique-matrix](2026-06-05-vuln-triage-technique-matrix.md) | ✅ |
| 2026-06-06 | [headless-single-stage-runner](2026-06-06-headless-single-stage-runner.md) | ✅ |
| 2026-06-06 | [intel-stage-ai-driven-p0](2026-06-06-intel-stage-ai-driven-p0.md) | ✅ |
| 2026-06-06 | [scoping-per-mode-gate-hitl-p0](2026-06-06-scoping-per-mode-gate-hitl-p0.md) | ✅ |
| 2026-06-25 | [stage-aware-db-refiner](2026-06-25-stage-aware-db-refiner.md) | ✅ |
| 2026-06-25 | [runtime-supervisor](2026-06-25-runtime-supervisor.md) | ✅ |
| 2026-07-02 | [gate-capability-ledger](2026-07-02-gate-capability-ledger.md) | ✅ |
| 2026-07-02 | [dead-asset-liveness-state](2026-07-02-dead-asset-liveness-state.md) | ✅ |
| 2026-07-02 | [asset-discovery-stage-and-delta-wave](2026-07-02-asset-discovery-stage-and-delta-wave.md) | ✅ |
| 2026-07-02 | [eas-worker-evidence-and-service-fingerprint](2026-07-02-eas-worker-evidence-and-service-fingerprint.md) | ✅ |
| 2026-07-05 | [stage-capability-tools](2026-07-05-stage-capability-tools.md) | ✅ |

## 2. Crate-per-service / Servitization / 架构健康 / P0 契约

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-29 | [p0-error-code-contract](2026-05-29-p0-error-code-contract.md) | ✅ |
| 2026-05-29 | [p0-frontend-api-layer](2026-05-29-p0-frontend-api-layer.md) | ✅ |
| 2026-05-29 | [p0-scoped-sql-to-repo](2026-05-29-p0-scoped-sql-to-repo.md) | ✅ |
| 2026-05-29 | [p0-tsrs-type-sync](2026-05-29-p0-tsrs-type-sync.md) | ✅ |
| 2026-05-30 | [arch-health-backlog](2026-05-30-arch-health-backlog.md) | ✅ |
| 2026-05-30 | [backend-oversized-file-split](2026-05-30-backend-oversized-file-split.md) | ✅ |
| 2026-05-30 | [crate-per-service-split](2026-05-30-crate-per-service-split.md) | ✅ |
| 2026-05-30 | [frontend-oversized-component-split](2026-05-30-frontend-oversized-component-split.md) | ✅ |
| 2026-05-30 | [m2-recon-app](2026-05-30-m2-recon-app.md) | ✅ |
| 2026-05-30 | [p0-3b-idor-residual-sink-full](2026-05-30-p0-3b-idor-residual-sink-full.md) | ✅ |
| 2026-05-30 | [p1-1-golish-db-scoped-crud-helper](2026-05-30-p1-1-golish-db-scoped-crud-helper.md) | ✅ |
| 2026-05-30 | [s1-1-repo-data-ownership-boundary](2026-05-30-s1-1-repo-data-ownership-boundary.md) | ✅ |
| 2026-05-30 | [s1-2-portification](2026-05-30-s1-2-portification.md) | ✅ |
| 2026-05-30 | [type-dedup-tsrs](2026-05-30-type-dedup-tsrs.md) | ✅ |
| 2026-05-31 | [m3-pentest-app](2026-05-31-m3-pentest-app.md) | ✅ |
| 2026-05-31 | [m4-agent-app-feasibility](2026-05-31-m4-agent-app-feasibility.md) | ✅ |
| 2026-05-31 | [m4-proper-move-agent-commands](2026-05-31-m4-proper-move-agent-commands.md) | ✅ |
| 2026-05-31 | [m4a-appstate-decouple](2026-05-31-m4a-appstate-decouple.md) | ✅ |
| 2026-05-31 | [m5-platform-app](2026-05-31-m5-platform-app.md) | ✅ |
| 2026-05-31 | [s1-2b1-recon-port-agent-bridge](2026-05-31-s1-2b1-recon-port-agent-bridge.md) | ✅ |

## 3. Asset Intel / Recon / Targets / Integrations

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-21 | [credential-capture-engine](2026-05-21-credential-capture-engine.md) | ✅ |
| 2026-05-21 | [integrations](2026-05-21-integrations.md) | ✅ |
| 2026-05-22 | [asset-intel-json-driven-providers](2026-05-22-asset-intel-json-driven-providers.md) | ⚠️ |
| 2026-05-22 | [asset-intel-two-phase-hydrate](2026-05-22-asset-intel-two-phase-hydrate.md) | ✅ |
| 2026-05-23 | [asset-intel-providers-flat](2026-05-23-asset-intel-providers-flat.md) | ✅ |
| 2026-05-25 | [extract-golish-asset-intel-crate](2026-05-25-extract-golish-asset-intel-crate.md) | ✅ |
| 2026-05-28 | [target-surface-workbench](2026-05-28-target-surface-workbench.md) | ✅ |
| 2026-05-28 | [target-topology-redesign](2026-05-28-target-topology-redesign.md) | ✅ |
| 2026-06-02 | [organization-recon-closed-loop](2026-06-02-organization-recon-closed-loop.md) | ✅ |
| 2026-07-02 | [frontend-fingerprint-display](2026-07-02-frontend-fingerprint-display.md) | ✅ |

## 4. 平台 / 跨平台 / Provider / LLM

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-07 | [builtin-mcp-auto-init](2026-05-07-builtin-mcp-auto-init.md) | ✅ |
| 2026-05-07 | [cross-platform-finishing](2026-05-07-cross-platform-finishing.md) | ✅ |
| 2026-05-25 | [llm-models-json-driven](2026-05-25-llm-models-json-driven.md) | ✅ |
| 2026-05-27 | [agent-tool-use-compatibility-layer](2026-05-27-agent-tool-use-compatibility-layer.md) | ✅ |

## 5. Legacy（未注日期 · 2026-06-07 从 docs 根归位）

| 文档 | 状态 |
|---|---|
| [execution-mode-policy-plan](execution-mode-policy-plan.md) | 🗂️ |
| [mcp-implementation-plan](mcp-implementation-plan.md) | 🗂️ |
| [mcp-migration-plan](mcp-migration-plan.md) | 🗂️ |
| [prompt-generation-ui-plan](prompt-generation-ui-plan.md) | 🗂️ |
| [scan-workflow-implementation](scan-workflow-implementation.md) | 🗂️ |

---

共 **71** 篇（66 日期前缀 + 5 legacy）。对应设计决策见 [`../../design/INDEX.md`](../../design/INDEX.md)；落地状态以 `feature_list.json` 为准。
