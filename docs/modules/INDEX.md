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

## Backend（`backend/crates/`，共 50 crate）

### 基础层

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-platform | 跨平台抽象层，全仓库唯一允许写 cfg(target_os) 的地方 | [→](backend/golish-platform.md) | ✅ |
| golish-core | 最底层基础类型与 trait（Tool/会话/事件/计划/HITL） | [→](backend/golish-core.md) | ✅ |
| golish-settings | 集中式 TOML 配置（env 插值/原子写/类型安全 schema） | [→](backend/golish-settings.md) | ✅ |
| golish-events | AI 事件协调 + transcript（DomainEvent/Coordinator/Writer） | [→](backend/golish-events.md) | ✅ |
| golish-models | LLM 模型注册表与能力定义（metadata 取代字符串猜） | [→](backend/golish-models.md) | ✅ |
| golish-context | LLM 上下文窗口与 token 预算/压缩/截断 | [→](backend/golish-context.md) | ✅ |
| golish-cli-output | CLI 事件渲染（terminal/JSON/quiet 三模式） | [→](backend/golish-cli-output.md) | ✅ |
| golish-json-repair | 修复 LLM 畸形 JSON 工具参数，保证参数为 object | [→](backend/golish-json-repair.md) | ✅ |
| golish-udiff | 解析并应用 unified diff（多 hunk 外科式编辑） | [→](backend/golish-udiff.md) | ✅ |

### 数据 / 持久化

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-db | PostgreSQL 持久化层（嵌入式 PG + scoped/atomic repo CRUD + current-owner evidence reads + attempt-generation CAS + terminal outcome/checkpoint atomic publish） | [→](backend/golish-db.md) | ✅ |
| golish-graphiti | PG 图知识库（pentest 发现的实体/关系图） | [→](backend/golish-graphiti.md) | ✅ |
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
| golish-intel-providers | ASM/威胁情报 provider 抽象（0.zone/FOFA/Quake/Hunter/Shodan） | [→](backend/golish-intel-providers.md) | ✅ |
| golish-mcp | MCP 客户端集成（fail-closed 项目信任 + canonical builtin 来源、rmcp client、工具转换） | [→](backend/golish-mcp.md) | ✅ |
| golish-js-analyzer | JS bundle 静态分析（抽取 API 端点调用点，省 LLM token） | [→](backend/golish-js-analyzer.md) | ✅ |
| golish-auth-probe | API 授权探测（消费 js-analyzer 端点，跑匿名/IDOR/越权 3 轮检测） | [→](backend/golish-auth-probe.md) | ✅ |

### 领域（pentest / vuln / recon / scan）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-pentest-domain | pentest 领域层（纯类型/业务规则、共享 confirmed-target Web Origin identity、I/O 边界 trait） | [→](backend/golish-pentest-domain.md) | ✅ |
| golish-pentest | pentest 工具执行引擎 + scoped output landing + producer-org guarded 证据账本 | [→](backend/golish-pentest.md) | ✅ |
| golish-pentest-mcp | MCP server 二进制（把 pentest 工具作为 MCP 工具暴露） | [→](backend/golish-pentest-mcp.md) | ✅ |
| golish-vuln-intel-domain | 漏洞情报领域层（纯类型 + I/O 边界 trait，无 I/O 依赖） | [→](backend/golish-vuln-intel-domain.md) | ✅ |
| golish-vuln-intel | 漏洞情报引擎（NVD/CISA/RSS 摄取 + GitHub PoC + Nuclei 发现） | [→](backend/golish-vuln-intel.md) | ✅ |
| golish-scan-runner | guarded 扫描器调度（current-owner/exact-origin launch + WhatWeb/Nuclei/ferox guarded landing） | [→](backend/golish-scan-runner.md) | ✅ |
| golish-projects | 项目配置存储 + `{project}/.golish/` 文件目录管理（无 Tauri 依赖） | [→](backend/golish-projects.md) | ✅ |

### Agent

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-agent-kit | agent runtime 底层构件（guarded harness gate + exact-origin worklist + bounded recovery projection） | [→](backend/golish-agent-kit.md) | ✅ |
| golish-agent-runtime | 高层 agent runtime（L4b：流式 run_agentic_loop + 压缩 + evals + request-scoped stage retry breaker + context-limit fail-stop + Enumeration bounded capacity continuation + structured/legacy worker chain identity wiring + failed-dispatch truth） | [→](backend/golish-agent-runtime.md) | ✅ |
| golish-agent-bridge | app↔runtime 桥接层（stable generation owner + cancel epoch + abort-safe history/background-note handoff + DB chain session injection） | [→](backend/golish-agent-bridge.md) | ✅ |
| golish-sub-agents | sub-agent 系统（定义/registry/执行器 + initial/atomic checkpoint addressability + provider-budgeted exact-chain replay + typed context-limit failure + full-data guards + bounded recovery projection） | [→](backend/golish-sub-agents.md) | ✅ |

### App 层（Tauri command facades）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish-app-core | 应用边界共享类型（L5：GolishError/DbState/scoping/runtime + generation-guarded recon ports + exactly-once pump-drained job terminal） | [→](backend/golish-app-core.md) | ✅ |
| golish-agent-app | agent 服务命令面（stable lifecycle/listener handoff + shared-origin fresh read model + scoped durable-chain DB adapter） | [→](backend/golish-agent-app.md) | ✅ |
| golish-pentest-app | pentest 服务命令面（AI 工具桥 + exact-origin producer binding + scope-classified JS endpoints + v8 finite route recovery + guarded evidence publish） | [→](backend/golish-pentest-app.md) | ✅ |
| golish-recon-app | recon 服务命令面（targets/current-owner directory/资产情报/组织/扫描队列/intel/capture） | [→](backend/golish-recon-app.md) | ✅ |
| golish-vuln-app | vuln-intel 服务命令面（feed/搜索/匹配/PoC·Nuclei 富化 + wiki） | [→](backend/golish-vuln-app.md) | ✅ |
| golish-platform-app | platform 服务命令面（vault/audit/notes/recordings） | [→](backend/golish-platform-app.md) | ✅ |

### 组合根 + rig forks

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| golish | 组合根 + Tauri 桌面应用（apex：bootstrap + AppState + ~300 命令 + CLI） | [→](backend/golish.md) | ✅ |
| golish / stage_run | headless 单/区间阶段实跑 + exact session/task/chain recovery（含孤儿 claim 与首 stage checkpoint repair） | [→](backend/golish/stage_run.md) | ✅ |
| rig-anthropic-vertex | rig fork：Claude on Vertex AI（CompletionModel + GCP 认证 + server tools） | [→](backend/rig-anthropic-vertex.md) | ✅ |
| rig-gemini-vertex | rig fork：Gemini on Vertex AI（CompletionModel + GCP 认证 + 流式） | [→](backend/rig-gemini-vertex.md) | ✅ |
| rig-openai-responses | rig fork：OpenAI Responses API（显式 reasoning 事件，o1/o3/gpt-5.x） | [→](backend/rig-openai-responses.md) | ✅ |
| rig-zai-sdk | rig fork：Z.AI GLM 原生 SDK（SSE 流式 + 伪 XML tool call 解析） | [→](backend/rig-zai-sdk.md) | ✅ |

## Frontend（`frontend/`）

| 模块 | 一句话职责 | 卡片 | 状态 |
|---|---|---|---|
| components | React UI 组件（~39 功能域：聊天/终端/设置/findings/pane 等） | [→](frontend/components.md) | ✅ |
| hooks | React hooks（Tauri 事件订阅/终端/补全/主题/键盘等） | [→](frontend/hooks.md) | ✅ |
| lib | 非-UI 基础设施（api 客户端/generated ts-rs/events/ai/pentest 等） | [→](frontend/lib.md) | ✅ |
| pages | 独立页面（ComponentTestbed；主 shell 在 App.tsx） | [→](frontend/pages.md) | ✅ |
| services | 事件服务（ai-events 处理器注册表 + terminal-events） | [→](frontend/services.md) | ✅ |
| store | Zustand 全局 store（12 slice + selectors + types；backend-first atomic conversation clear） | [→](frontend/store.md) | ✅ |
| styles | 终端/xterm 特化 CSS（通用走 Tailwind 4） | [→](frontend/styles.md) | ✅ |

---

## 进度

- 已写卡：**185**（50 backend crate 卡 + 128 目录子模块卡 + 7 前端子系统卡）／ 预计 ~185 — **全部 3 波完成** 🎉
- **Wave 1 完成** ✅：全部 50 个 backend crate 的 crate 卡。
- **Wave 2 完成** ✅：全部 backend crate 的目录子模块卡（**128 张**，每张实读 `mod.rs`/入口）。
  - 基础 15 · 数据 4 · 执行/LLM 17 · 工具/集成 12 · 领域 12 · agent 25（含 agent-kit 13）· app 17 · 组合根/rig 22（含 golish 17）· golish-tools 4（Wave 0）
- **Wave 3 完成** ✅：前端 7 个子系统卡（components/hooks/lib/pages/services/store/styles），均实读 `frontend/` 真实结构 + 入口文件。
- 一致性：0 个 ⬜ 残留（backend + frontend 全 ✅）；卡间链接 0 broken。后续维护见 AGENTS.md §2.4/§4（改模块同步更新卡 + 本索引）。
