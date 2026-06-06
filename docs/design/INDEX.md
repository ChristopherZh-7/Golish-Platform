# 设计文档索引 · Design Docs Index

> **这是 `docs/design/` 的导航入口。** 设计文档是**决策记录**（point-in-time，按日期前缀命名，不覆盖旧文件——作废只在头部加 `> Superseded by …`，见 AGENTS.md I6）。配套实现计划在 [`../superpowers/plans/`](../superpowers/plans/INDEX.md)。
>
> 标题取自文件名 slug；**状态**来自文内 `superseded` 标注的实读 grep（⚠️ = 文内含 superseded 关系，定位后请打开确认方向）。按主题分组，组内按日期。

## 图例
- ✅ 现行 ｜ ⚠️ 文内含 superseded 关系（可能已被取代或取代他者）

---

## 1. Agent Harness / Engine / Stage / Gate / Evidence（核心）

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-20 | [agent-harness-strategy](2026-05-20-agent-harness-strategy.md) | ⚠️ |
| 2026-05-26 | [evidence-ledger-on-existing-audit-log](2026-05-26-evidence-ledger-on-existing-audit-log.md) | ⚠️ |
| 2026-05-26 | [harness-observability-plane](2026-05-26-harness-observability-plane.md) | ✅ |
| 2026-05-26 | [mcp-resource-evidence-summary](2026-05-26-mcp-resource-evidence-summary.md) | ✅ |
| 2026-05-26 | [operation-harness-profile-dag-lab](2026-05-26-operation-harness-profile-dag-lab.md) | ⚠️ |
| 2026-05-26 | [stage-harness-mvp-external-attack-surface](2026-05-26-stage-harness-mvp-external-attack-surface.md) | ⚠️ |
| 2026-06-01 | [harness-explainer-and-decisions](2026-06-01-harness-explainer-and-decisions.md) | ✅ |
| 2026-06-01 | [harness-rebuild](2026-06-01-harness-rebuild.md) | ⚠️ |
| 2026-06-02 | [golish-agent-engine-v2-design](2026-06-02-golish-agent-engine-v2-design.md) | ✅ |
| 2026-06-02 | [harness-execution-layer-reference](2026-06-02-harness-execution-layer-reference.md) | ✅ |
| 2026-06-02 | [harness-stage-spec-reference](2026-06-02-harness-stage-spec-reference.md) | ✅ |
| 2026-06-02 | [harness-topology-reference](2026-06-02-harness-topology-reference.md) | ✅ |
| 2026-06-02 | [harness-vs-mainstream-gap-analysis](2026-06-02-harness-vs-mainstream-gap-analysis.md) | ✅ |
| 2026-06-02 | [pentagi-engine-substrate-reference](2026-06-02-pentagi-engine-substrate-reference.md) | ✅ |
| 2026-06-02 | [stage-driven-required-stage-coverage](2026-06-02-stage-driven-required-stage-coverage.md) | ✅ |
| 2026-06-02 | [stage-spec-worksheet.csv](2026-06-02-stage-spec-worksheet.csv) | ✅ |
| 2026-06-02 | [stage-tool-whitelist-enforcement](2026-06-02-stage-tool-whitelist-enforcement.md) | ✅ |
| 2026-06-02 | [submit-stage-deliverable-tool](2026-06-02-submit-stage-deliverable-tool.md) | ✅ |
| 2026-06-03 | [background-tool-execution](2026-06-03-background-tool-execution.md) | ✅ |
| 2026-06-03 | [harness-profile-driven-execution](2026-06-03-harness-profile-driven-execution.md) | ✅ |
| 2026-06-03 | [lazy-per-stage-planning](2026-06-03-lazy-per-stage-planning.md) | ✅ |
| 2026-06-03 | [task-mode-lead-agent-triage](2026-06-03-task-mode-lead-agent-triage.md) | ✅ |
| 2026-06-03 | [two-level-phase-stage-model](2026-06-03-two-level-phase-stage-model.md) | ✅ |
| 2026-06-04 | [per-stage-plan-cards](2026-06-04-per-stage-plan-cards.md) | ✅ |
| 2026-06-04 | [plan-roadmap-ux-overhaul](2026-06-04-plan-roadmap-ux-overhaul.md) | ✅ |
| 2026-06-05 | [attack-surface-ceiling-raising](2026-06-05-attack-surface-ceiling-raising.md) | ✅ |
| 2026-06-05 | [coverage-matrix](2026-06-05-coverage-matrix.md) | ✅ |
| 2026-06-05 | [gate-rule-engine](2026-06-05-gate-rule-engine.md) | ✅ |
| 2026-06-05 | [gate-rules-migration](2026-06-05-gate-rules-migration.md) | ✅ |
| 2026-06-05 | [unified-ai-harness-observability](2026-06-05-unified-ai-harness-observability.md) | ⚠️ |
| 2026-06-05 | [vuln-triage-technique-matrix](2026-06-05-vuln-triage-technique-matrix.md) | ✅ |
| 2026-06-06 | [active-recon-coverage-matrix](2026-06-06-active-recon-coverage-matrix.md) | ✅ |
| 2026-06-06 | [headless-single-stage-runner](2026-06-06-headless-single-stage-runner.md) | ✅ |
| 2026-06-06 | [intel-stage-ai-driven-per-mode](2026-06-06-intel-stage-ai-driven-per-mode.md) | ✅ |
| 2026-06-06 | [scoping-per-mode-gate-hitl](2026-06-06-scoping-per-mode-gate-hitl.md) | ✅ |

## 2. Asset Intel / Recon / Targets / Integrations

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-(05) | [recon-tool-belt-2026-05](recon-tool-belt-2026-05.md) | ✅ |
| 2026-05-20 | [asm-intel-providers](2026-05-20-asm-intel-providers.md) | ⚠️ |
| 2026-05-21 | [credential-capture-engine](2026-05-21-credential-capture-engine.md) | ✅ |
| 2026-05-21 | [integrations](2026-05-21-integrations.md) | ✅ |
| 2026-05-22 | [asset-intel-json-driven-providers](2026-05-22-asset-intel-json-driven-providers.md) | ✅ |
| 2026-05-22 | [asset-intel-provider-abstraction](2026-05-22-asset-intel-provider-abstraction.md) | ⚠️ |
| 2026-05-22 | [asset-intel-two-phase-hydrate](2026-05-22-asset-intel-two-phase-hydrate.md) | ✅ |
| 2026-05-22 | [engagement-workspace](2026-05-22-engagement-workspace.md) | ✅ |
| 2026-05-23 | [asset-intel-providers-flat](2026-05-23-asset-intel-providers-flat.md) | ✅ |
| 2026-05-25 | [extract-golish-asset-intel-crate](2026-05-25-extract-golish-asset-intel-crate.md) | ✅ |
| 2026-05-28 | [target-surface-workbench](2026-05-28-target-surface-workbench.md) | ✅ |
| 2026-05-28 | [target-topology-redesign](2026-05-28-target-topology-redesign.md) | ✅ |
| 2026-06-02 | [organization-recon-closed-loop](2026-06-02-organization-recon-closed-loop.md) | ✅ |

## 3. 架构 / Servitization / Crate-per-service

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-29 | [architecture-optimization](2026-05-29-architecture-optimization.md) | ✅ |
| 2026-05-30 | [servitization-readiness](2026-05-30-servitization-readiness.md) | ✅ |
| 2026-05-30 | [s1-2-port-horizontal-coupling](2026-05-30-s1-2-port-horizontal-coupling.md) | ✅ |
| 2026-05-30 | [s1-2b-recon-read-port](2026-05-30-s1-2b-recon-read-port.md) | ✅ |

## 4. LLM / Provider / Prompt / Tool-use

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-17 | [refiner-patch-protocol](2026-05-17-refiner-patch-protocol.md) | ✅ |
| 2026-05-25 | [llm-models-json-driven](2026-05-25-llm-models-json-driven.md) | ✅ |
| 2026-05-27 | [add-xiaomi-mimo-provider](2026-05-27-add-xiaomi-mimo-provider.md) | ✅ |
| 2026-05-27 | [agent-tool-use-compatibility-layer](2026-05-27-agent-tool-use-compatibility-layer.md) | ✅ |
| （无日期） | [system-prompt-design-input](system-prompt-design-input.md) | ✅ |

## 5. Pentest 数据 / 文档基建 / 其它

| 日期 | 文档 | 状态 |
|---|---|---|
| 2026-05-20 | [pentest-fields-tool-mapping](2026-05-20-pentest-fields-tool-mapping.md) | ✅ |
| 2026-06-05 | [remove-pipeline-feature](2026-06-05-remove-pipeline-feature.md) | ✅ |
| 2026-06-07 | [module-cards-system](2026-06-07-module-cards-system.md) | ✅ |

---

共 **60** 篇（含 1 csv）。设计→实现的配套计划见 [`../superpowers/plans/INDEX.md`](../superpowers/plans/INDEX.md)。
