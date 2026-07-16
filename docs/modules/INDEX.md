# 模块索引 · Module Index

> **这是给 AI agent 的导航入口。** 先读本索引看全貌，再按需打开具体模块卡，不要一次性吞整个代码库。每张卡用统一模板介绍一个模块：职责、公开接口、依赖关系、关键文件、坑、测试入口。
>
> 体系设计见 [`docs/design/2026-06-07-module-cards-system.md`](../design/2026-06-07-module-cards-system.md)。改任何模块都必须同步更新它的卡 + 本索引状态列（AGENTS.md §2.4 / §4 已强制）。

## 怎么用（给 AI）

1. 在下表按「一句话职责」定位相关模块
2. 打开对应「卡片」链接读细节
3. 卡内「何时该读这张卡」是触发提示；「依赖 / 被谁依赖」帮你判断改动影响面

## 图例

- 状态：✅ 已写卡 ｜ 🚧 进行中 ｜ ⬜ 待写
- 分层依据真实依赖图（`backend/crates/*/Cargo.toml` 内部依赖），从底层往上：基础 → 数据 → 执行/LLM → 工具 → 领域 → agent → app → 组合根。

---

## Backend（`backend/crates/`，共 57 crate）

### 基础层

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-platform | 跨平台抽象层，全仓库唯一允许写 cfg(target_os) 的地方 | [→](backend/golish-platform.md) | ✅ |
| golish-core | 最底层基础类型与 trait（Tool/会话/HITL/operation rollout + opaque Candidate context + safe terminal/consolidation trace wire） | [→](backend/golish-core.md) | ✅ |
| golish-settings | 集中式 TOML 配置（env 插值/原子写/类型安全 schema） | [→](backend/golish-settings.md) | ✅ |
| golish-events | AI 事件协调 + transcript/op_trace（含 Candidate Attempt terminal / Wave consolidation 安全摘要） | [→](backend/golish-events.md) | ✅ |
| golish-models | LLM 模型注册表与能力定义（metadata 取代字符串猜） | [→](backend/golish-models.md) | ✅ |
| golish-context | LLM 上下文窗口与 token 预算/压缩/截断 | [→](backend/golish-context.md) | ✅ |
| golish-cli-output | CLI 事件渲染（terminal/JSON/quiet 三模式） | [→](backend/golish-cli-output.md) | ✅ |
| golish-json-repair | 修复 LLM 畸形 JSON 工具参数，保证参数为 object | [→](backend/golish-json-repair.md) | ✅ |
| golish-udiff | 解析并应用 unified diff（多 hunk 外科式编辑） | [→](backend/golish-udiff.md) | ✅ |

### 数据 / 持久化

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-db | PostgreSQL 持久化层（runtime/attack V2 rollout + durable Stage Team/repair + Company Controller exact-Worker lease recovery + bounded crawler中断同链reconcile + Candidate TerminalIntent/recovery + typed FactDelta direct/pending route + hash-consistent Memory source invalidation） | [→](backend/golish-db.md) | ✅ |
| golish-graphiti | legacy GraphClient + scoped temporal node/edge Assertion-lineage graph（attested generation rebuild，旧 API 兼容） | [→](backend/golish-graphiti.md) | ✅ |
| golish-indexer | 代码索引基座（IndexerBackend + vtcode 后端 + git 工具） | [→](backend/golish-indexer.md) | ✅ |
| golish-artifacts | 自动维护项目文档提案（README/CLAUDE，未集成） | [→](backend/golish-artifacts.md) | ✅ |

### 执行 / Shell / LLM

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-pty | PTY/终端管理（会话/转义解析/渲染网格/shell 集成） | [→](backend/golish-pty.md) | ✅ |
| golish-shell-exec | shell 命令执行（PATH 继承 + 流式 + run_pty_cmd 工具） | [→](backend/golish-shell-exec.md) | ✅ |
| golish-sidecar | 后台被动捕获会话上下文（state.md/patches/artifacts） | [→](backend/golish-sidecar.md) | ✅ |
| golish-synthesis | LLM 生成 commit 消息/状态/会话标题 | [→](backend/golish-synthesis.md) | ✅ |
| golish-llm-providers | 统一 LLM provider 抽象（10+ provider） | [→](backend/golish-llm-providers.md) | ✅ |
| golish-prompts | prompt 组装系统（贡献者/registry/system prompt/摘要器） | [→](backend/golish-prompts.md) | ✅ |
| golish-session | AI 会话持久化（归档/双写文件+PG） | [→](backend/golish-session.md) | ✅ |
| golish-skills | Agent Skills 发现/解析/匹配（agentskills.io） | [→](backend/golish-skills.md) | ✅ |

### 工具 / 集成

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| **golish-tools** | agent 工具执行系统（ToolRegistry + 文件/目录/shell/AST/联网/可信 Enumeration preflight schema） | [→](backend/golish-tools.md) | ✅ |
| golish-tools / file_ops | 文件读写增删改 5 工具，经沙箱校验 | [→](backend/golish-tools/file_ops.md) | ✅ |
| golish-tools / directory_ops | list_files / list_directory / grep_file | [→](backend/golish-tools/directory_ops.md) | ✅ |
| golish-tools / ast_grep | 结构化代码搜索与替换 | [→](backend/golish-tools/ast_grep.md) | ✅ |
| golish-tools / definitions | 工具 schema → LLM function declarations | [→](backend/golish-tools/definitions.md) | ✅ |
| golish-web | Web 搜索与内容抓取（Tavily/Brave + 抓取），封装成 agent 工具 | [→](backend/golish-web.md) | ✅ |
| golish-integrations | schema 驱动的外部服务凭据管理（FOFA/Quake/Hunter/Shodan/0.zone/ENScan/GitHub） | [→](backend/golish-integrations.md) | ✅ |
| golish-intel-providers | ASM/威胁情报 provider 抽象（0.zone/FOFA/Quake/Hunter/Shodan；typed DSL + exact-host mapping） | [→](backend/golish-intel-providers.md) | ✅ |
| golish-mcp | MCP 客户端集成（fail-closed 项目信任 + canonical builtin 来源、rmcp client、工具转换） | [→](backend/golish-mcp.md) | ✅ |
| golish-js-analyzer | JS bundle 静态分析（保留 callee/完整 receiver/byte span/相对 path 候选，兼容旧 Endpoint） | [→](backend/golish-js-analyzer.md) | ✅ |

### 领域（pentest / vuln / recon / scan）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-pentest-domain | pentest 领域层（纯类型/业务规则、共享 confirmed-target Web Origin identity、I/O 边界 trait） | [→](backend/golish-pentest-domain.md) | ✅ |
| golish-pentest | pentest 工具执行引擎 + trusted-tool scoped output landing + multi-origin fingerprint observations + producer-org guarded 证据账本 | [→](backend/golish-pentest.md) | ✅ |
| golish-pentest-mcp | MCP server 二进制（把 pentest 工具作为 MCP 工具暴露） | [→](backend/golish-pentest-mcp.md) | ✅ |
| golish-vuln-intel-domain | 漏洞情报领域层（纯类型 + I/O 边界 trait，无 I/O 依赖） | [→](backend/golish-vuln-intel-domain.md) | ✅ |
| golish-vuln-intel | 漏洞情报引擎（NVD/CISA/RSS 摄取 + GitHub PoC + Nuclei 发现） | [→](backend/golish-vuln-intel.md) | ✅ |
| golish-scan-runner | guarded Recon runner（WhatWeb/ferox）+ current-owner Nuclei 安全 template selector | [→](backend/golish-scan-runner.md) | ✅ |
| golish-projects | 项目配置存储 + `{project}/.golish/` 文件管理 + Unix dirfd / Windows capability-handle report storage（无 Tauri 依赖） | [→](backend/golish-projects.md) | ✅ |
| golish-memory-domain | Memory Fabric 纯领域契约（typed source/Episode/Assertion/event catalog + ContextPack values/layers + 1536 维 schema） | [→](backend/golish-memory-domain.md) | ✅ |
| golish-post-exploit-domain | Post-Exploit V2 纯领域契约（Foothold/Internal Asset/Attack Path/Action/Approval） | [→](backend/golish-post-exploit-domain.md) | ✅ |
| golish-cleanup-domain | Cleanup obligation/attempt/absence/waiver 纯状态机、exact frozen-scope CAS 与安全不变量 | [→](backend/golish-cleanup-domain.md) | ✅ |
| golish-reporting-domain | Canonical cited Report Read Model、typed EvidenceAudit/blocked-decision/sealed-handoff authority、双轴 revision 与 fail-closed validator | [→](backend/golish-reporting-domain.md) | ✅ |

### Agent

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-agent-kit | agent runtime 底层构件（typed stage/runtime contracts + org-bound evidence seam + trusted-target active-recon barrier + runtime tool owner fence + Candidate/whole-source resume selector） | [→](backend/golish-agent-kit.md) | ✅ |
| golish-agent-runtime | 高层 agent runtime（流式 loop + source-pinned worker restore + 每公司持久 Company Controller + 动态 sibling SubAgent + 中断crawler的worklist-first同链恢复 + 同 Controller Gate repair/final turn + Candidate submit-only/terminal recovery） | [→](backend/golish-agent-runtime.md) | ✅ |
| golish-agent-bridge | app↔runtime 桥接层（stable request owner + request-local resume source + trusted lease/runtime-memory/UoW/ContextPack 注入） | [→](backend/golish-agent-bridge.md) | ✅ |
| golish-sub-agents | sub-agent 系统（source-pinned V2 lifecycle + trusted Company Controller plan/dispatch controls + foreground Nuclei specialist + Wave-read-only analyst + opaque Attempt verifier） | [→](backend/golish-sub-agents.md) | ✅ |

### App 层（Tauri command facades）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-app-core | 应用边界共享类型（L5：GolishError/DbState/scoping/runtime + opaque trusted operator + exact trusted-target promotion + generation-guarded recon ports） | [→](backend/golish-app-core.md) | ✅ |
| golish-cleanup-app | Cleanup P7b exact terminal truth、backoff/fair DB-global worker、Gate/residual 与可恢复两阶段组织删除 | [→](backend/golish-cleanup-app.md) | ✅ |
| golish-agent-app | agent 服务命令面（GUI/CLI shared task-operation kernel + Stage Team read/operator recovery + Candidate Verification/pending-enrichment queue + Memory/Cleanup/Reporting authority） | [→](backend/golish-agent-app.md) | ✅ |
| golish-pentest-app | pentest 服务命令面（AI 工具桥 + operation-bound EAS/Enumeration/Vuln guarded producers + deterministic JS/API contextual promotion + lease-fenced Post-Exploit/Cleanup wrappers） | [→](backend/golish-pentest-app.md) | ✅ |
| golish-recon-app | recon 服务命令面（stable candidate/unit-review ts-rs contracts + existing-child identity projection + exact-target transaction dedup/query-seed 分层 asset-intel landing） | [→](backend/golish-recon-app.md) | ✅ |
| golish-vuln-app | vuln-intel 服务命令面（feed/搜索/匹配/PoC·Nuclei 富化 + wiki） | [→](backend/golish-vuln-app.md) | ✅ |
| golish-platform-app | platform 服务命令面（vault/audit/notes/recordings） | [→](backend/golish-platform-app.md) | ✅ |
| golish-memory-app | Memory Fabric 应用服务（atomic canonical outbox + deterministic projectors + opaque-auth scoped ContextPack retrieval） | [→](backend/golish-memory-app.md) | ✅ |
| golish-post-exploit-app | Post-Exploit V2 app services（canonical facts + cleanup-bound P6b action/approval fence） | [→](backend/golish-post-exploit-app.md) | ✅ |
| golish-reporting-app | Reporting single-RR build/redaction + verified-artifact/outbox finalization protocol（无 RAG/KG authority） | [→](backend/golish-reporting-app.md) | ✅ |

### 组合根 + rig forks

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish | 组合根 + Tauri 桌面应用（DB-ready lifecycle + AppState + ~300 命令 + typed-provider/typed-HITL CLI） | [→](backend/golish.md) | ✅ |
| golish / stage_run | headless 单/区间实跑 + whole-source exact claim/recovery + per-invocation direct-active target authority + Company Controller/AI-call/runtime/attack DB diagnostics | [→](backend/golish/stage_run.md) | ✅ |
| rig-anthropic-vertex | rig fork：Claude on Vertex AI（CompletionModel + GCP 认证 + server tools） | [→](backend/rig-anthropic-vertex.md) | ✅ |
| rig-gemini-vertex | rig fork：Gemini on Vertex AI（CompletionModel + GCP 认证 + 流式） | [→](backend/rig-gemini-vertex.md) | ✅ |
| rig-openai-responses | rig fork：OpenAI Responses API（显式 reasoning 事件，o1/o3/gpt-5.x） | [→](backend/rig-openai-responses.md) | ✅ |
| rig-zai-sdk | rig fork：Z.AI GLM 原生 SDK（SSE 流式 + 伪 XML tool call 解析） | [→](backend/rig-zai-sdk.md) | ✅ |

## Frontend（`frontend/`）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| components | React UI 组件（fail-closed Task/Profile commit + DB-backed Stage Team/Candidate recovery；trace 仅触发 exact refresh） | [→](frontend/components.md) | ✅ |
| hooks | React hooks（Tauri 事件订阅/终端/补全/主题/键盘等） | [→](frontend/hooks.md) | ✅ |
| lib | 非-UI 基础设施（typed Stage Team + Candidate Verification/recovery/pending-enrichment API + generated safe wire） | [→](frontend/lib.md) | ✅ |
| pages | 独立页面（ComponentTestbed；主 shell 在 App.tsx） | [→](frontend/pages.md) | ✅ |
| services | 事件服务（ai-events 注册表 + Candidate review/Attempt/Wave consolidation refresh-only trace + terminal-events） | [→](frontend/services.md) | ✅ |
| store | Zustand 全局 store（12 slice + selectors + types；Candidate/Reporting refresh hint + backend-first atomic conversation clear） | [→](frontend/store.md) | ✅ |
| styles | 终端/xterm 特化 CSS（通用走 Tailwind 4） | [→](frontend/styles.md) | ✅ |

---

## 进度

- 已写卡：**187**（52 backend crate 卡 + 128 目录子模块卡 + 7 前端子系统卡）／ 预计 ~187 — **全部 3 波完成** 🎉
- **Wave 1 完成** ✅：全部 52 个 backend crate 的 crate 卡。
- **Wave 2 完成** ✅：全部 backend crate 的目录子模块卡（**128 张**，每张实读 `mod.rs`/入口）。
  - 基础 15 · 数据 4 · 执行/LLM 17 · 工具/集成 12 · 领域 12 · agent 25（含 agent-kit 13）· app 17 · 组合根/rig 22（含 golish 17）· golish-tools 4（Wave 0）
- **Wave 3 完成** ✅：前端 7 个子系统卡（components/hooks/lib/pages/services/store/styles），均实读 `frontend/` 真实结构 + 入口文件。
- 一致性：0 个 ⬜ 残留（backend + frontend 全 ✅）；卡间链接 0 broken。后续维护见 AGENTS.md §2.4/§4（改模块同步更新卡 + 本索引）。

## Card synchronization notes

- `frontend/components` is current as of 2026-07-14 for persistent AIChatPanel context-compaction visibility; its module status remains unchanged.
- `frontend/components` is current as of 2026-07-15 for fail-closed AIChatPanel execution-profile commit/send/restore semantics; its module status remains ✅.
- `golish-db/repo`, `golish-agent-runtime/agentic_loop`, `golish-agent-app/ai`, `frontend/lib`, and `frontend/components` are current as of 2026-07-14 for durable Stage Team repair/operator recovery and the exact Candidate Verification/pending-enrichment queue; `golish-db` additionally records the 2026-07-15 audited checksum/forward-migration recovery contract; all module-card statuses remain ✅.
- `golish-agent-app/ai` and `golish-agent-kit/task_orchestrator` are current as of 2026-07-14 for unit-less coordinator pass-token normalization and DB-authoritative fan-out closeout; statuses remain ✅.
- `golish-agent-app/ai`, `golish-agent-kit/task_orchestrator`, and `golish/stage_run` are current as of 2026-07-15 for the typed fresh-target authority tri-state, explicit CLI `--org` confirmed-identity fast path (target-empty and held before historical-target DB reads), deferred V2 Scoping finalization before `stage_passed` with deterministic replay/identity checks, headless exact-resume missing-marker fail-closed behavior, GUI prompt-only `UnconfirmedSubject`, and current-invocation exact-target shape validation; statuses remain ✅.
- `golish-agent-app/ai`, `golish/cli`, and `frontend/components` are current as of 2026-07-15 for GUI/CLI shared provider and context-config normalization; statuses remain ✅.
- `golish-agent-app/ai` is current as of 2026-07-15 for fresh GUI/CLI launch profile validation; unknown embedded profiles fail closed before operation creation instead of silently selecting another DAG.
- `golish/stage_run` is current as of 2026-07-15 for typed phase-boundary approval parity: headless confirmation stays fail closed unless the invocation explicitly combines `--auto-approve` with `--approve-phase-boundaries`, the CLI equivalent of the GUI Confirm card.
- `golish-agent-bridge/agent_bridge` is current as of 2026-07-15 for the GUI Task success-path stack boundary: completed primary turns bypass the provider failover future, while real primary errors retain the existing fallback policy; status remains ✅.
- `golish-agent-runtime/agentic_loop` and `frontend/components` are current as of 2026-07-15 for Company Controller child-output protocol/dependency retry semantics, exact-pointer legacy/Team mutual exclusion, and the compact output-authoritative Team status view; statuses remain ✅.
- `frontend/components` is current as of 2026-07-15 for Target delete convergence: single/bulk deletes update local state immediately, stale target-list responses cannot restore removed rows, all destructive entries use an in-app confirmation dialog, and two-phase organization deletion waits for the durable row removal before refresh; status remains ✅.
- `golish-db/repo`, `golish-agent-runtime/agentic_loop`, `frontend/lib`, and `frontend/components` are current as of 2026-07-15 for Stage Team WorkerRun UI isolation, ordered reasoning flush/timing, transient tool-fence transaction retry, and narrow local-provider split-state recovery; statuses remain ✅.
- `golish-db/repo`, `golish-agent-runtime/agentic_loop`, `golish-agent-kit/tool_executors`, `golish-sub-agents/defaults`, and `frontend/components` are current as of 2026-07-15 for controller-only Stage Team seeding, Codex-compatible chain-local `update_plan`, composite parallel plan steps, exact expired-Controller reclaim, preview/submit `reason_kind` parity, same-Controller final submission/Gate repair continuity, and a single current Controller plan card whose failed/superseded raw updates remain diagnostic-only; statuses remain ✅.
- `golish-db/repo`, `golish-agent-runtime/agentic_loop`, `golish-sub-agents/executor`, and `golish/stage_run` are current as of 2026-07-15 for structured Controller chain authority, claimed-Unit refresh, router-only coordination barriers, operation-fresh aggregate closeout claims, and terminal-aware CLI/run-tree execution selection; statuses remain ✅.
- `golish-agent-app/ai` is current as of 2026-07-15 for AttackCandidate submit-preview evidence membership against the exact frozen work item manifest; status remains ✅.
- `golish-agent-kit/task_orchestrator`, `golish-agent-app/ai`, and `frontend/components` are current as of 2026-07-16 for the operation-bound TargetIntel→EAS exact target review: one explicit non-empty subset confirmation replaces the duplicate generic active-scan approval, while direct EAS and provider-derived authority remain fail closed; statuses remain ✅.
- `golish-agent-runtime/agentic_loop` and `frontend/components` are current as of 2026-07-16 for terminal Company Controller progress convergence: new runs emit exact passed/blocked Unit progress, while successful historical `stage_run` results repair stale request-scoped running summaries during replay; statuses remain ✅.
- `golish-db/repo`, `golish-agent-runtime/agentic_loop`, and `golish-agent-kit/task_orchestrator` are current as of 2026-07-16 for interrupted EAS wrapper same-chain reconciliation and Gate-blocked UI semantics: bounded exact-scope wrappers resume from durable worklist gaps, while blocked stages no longer emit `SubtaskCompleted` or promise unconditional recovery; statuses remain ✅.
- `golish-agent-kit/harness`, `golish-agent-runtime/agentic_loop`, `golish-agent-app/ai`, `golish-pentest-app/pentest_bridge`, `golish/stage_run`, and `frontend/components` are current as of 2026-07-16 for downstream Company Controller convergence: Target Intel/EAS/Enumeration/Vuln share one durable Team contract with frozen stage specialists and DB-backed UI; Controller park/heartbeat, one-round no-migration Gate repair, anonymous eligible-id recovery, and legacy telemetry role folding are documented; statuses remain ✅.
- `golish-agent-runtime/agentic_loop`, `golish/stage_run`, and `frontend/components` are current as of 2026-07-16 for company-stage legacy retirement: non-V2 operations are typed rerun-required, generic specialist fallback and legacy collector cards are unavailable, exact resume accepts validated Team children while selecting the unique Controller, and Candidate/Verification/later typed views remain separate; statuses remain ✅.
- `frontend/components` is current as of 2026-07-16 for Codex-style Company Controller drill-in: exact `stage_team_dispatch_workers` children replace the generic tool row with clickable SubAgent cards, retain stack-based back navigation, and keep the Controller header live while delegated children run; status remains ✅.
- `frontend/components` is current as of 2026-07-16 for passed-stage Controller plan convergence: exact Team lead identities resolve the matching organization row, and only server-authored `passed` truth projects a stale final plan snapshot to display-complete; blocked/error/stopped states remain unfinished; status remains ✅.
- `golish-agent-runtime/agentic_loop` and `frontend/components` are current as of 2026-07-16 for Stage Team dispatch fan-out identity: every accepted WorkerRun derives a distinct `${dispatch_tool_request_id}::worker:<worker_run_id>` transcript/UI key, while the Controller immediately shows one numbered assignment card per dispatch request and keeps not-yet-started children visible as queued; statuses remain ✅.
- `golish-db/repo` and `golish-agent-app/ai` are current as of 2026-07-16 for Codex-style Thread/Turn continuation: a stable Task/Operation appends durable `operation_turns` from either `running` or `waiting`, while expired clean Controller/child workers resume the same WorkItem, WorkerRun and message chain with an incremented attempt epoch; outcome-unknown active tools remain recovery-required. Existing worker-keyed frontend cards therefore continue on one timeline without a new IPC/UI branch; statuses remain ✅.
- `golish-db/repo` and `golish-agent-app/ai` are current as of 2026-07-16 for exact Company Controller chain ownership: `company_stage_controller` is a server-owned `pentester` at both persistence and resume validation boundaries, so a valid same-session/same-operation Controller chain is not rejected as cross-owned; unknown roles and real ownership drift remain fail closed. Statuses remain ✅.
- `golish-db/repo` is current as of 2026-07-16 for Stage Team dynamic-child replay canonicalization: accepted child creation and replay validation share the same objective/hash/ref material, including the Controller assignment envelope, so a continued operation validates existing children without rewriting them. Status remains ✅.
- `golish-agent-runtime/agentic_loop` is current as of 2026-07-16 for Company Controller operator-recovery propagation: a parked Controller with an outcome-unknown child returns a typed DB-backed recovery blocker and closes same-request reentry instead of reporting a generic failed Controller or falling back to direct tools. Status remains ✅.
- `golish-agent-runtime/agentic_loop` is current as of 2026-07-16 for terminal Stage Run request control: closed operator-recovery/reentry ToolResults become a typed dispatch halt, suppress later same-batch calls, and end the current model loop after result persistence without changing successful scheduler-read semantics. Status remains ✅.
- `golish-sub-agents/executor`, `golish-agent-runtime/agentic_loop`, and `frontend/components` are current as of 2026-07-16 for Stage Team child completion/retry truth: ordinary children receive a typed `stage_worker_output.v1` barrier, invalid output corrects the exact failed WorkerRun UI state before retry, and retry generations of one WorkItem render as one assignment with attempt history rather than fake sibling cards; statuses remain ✅.
- `golish-db/repo` and `golish-agent-runtime/agentic_loop` are current as of 2026-07-16 for reconciled-child CLI continuation closure: seed replay preserves historical WorkItem epochs while validating dynamic/gate-repair authority; ordinary Company Controller Gate BLOCK and aggregate-token unavailability close only the current request; an authorized successor-Turn authority atomically restores the same plan/unit/leader/Controller WorkerRun/message chain without replacing the Agent or rewriting historical gaps. Migration-time legacy no-gap witnesses are immutable and post-migration creation remains forbidden; statuses remain ✅.
