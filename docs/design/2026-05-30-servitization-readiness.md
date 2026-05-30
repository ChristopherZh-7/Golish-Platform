# Golish 平台服务化（模块化）就绪度设计

> 日期：2026-05-30
> 状态：Draft
> 来源：用户需求「评估项目哪些地方需要模块化，为未来把功能服务化搭好框架，看耦合 / 拆分」（2026-05-30）
> Relates to:
> - `docs/architecture.md`（6 层 DAG 现状，本文不变量基线）
> - `docs/design/2026-05-29-architecture-optimization.md`（内部代码健康路线图 P0/P1/P2，本文是其「服务化」上层视角，复用其全部结论）
> - `docs/superpowers/plans/2026-05-30-arch-health-backlog.md`（拆 / 合并 / 优化 backlog）
> - `AGENTS.md` §5 不变量 I1（错误码契约）/ I2（IDOR）/ I5（ts-rs 同步）/ I9（事务内不调外部）/ I10（schema 向后兼容）
> 范围：backend Rust workspace（45 内部 crate）+ 跨端接口层
> **本文件只记录设计与路线图，不改任何代码、不动 `frontend/lib/generated/`。所有结论均带 `文件:行号` 证据。**

---

## 1. 背景与目标

### 1.1 背景

Golish 当前是 Tauri 2 桌面端的 agentic 渗透测试平台，后端为 45 crate 的 Rust workspace（`backend/Cargo.toml:2-48`），前端 React 19。用户提出未来要把多个功能「服务化」——即把部分垂直域从桌面进程内抽出为可独立部署 / 独立伸缩的服务（同机多进程乃至远程 / 云端）。

本设计基于一次**只读架构体检**（纯静态阅读，未改代码、未跑编译），评估当前架构对「服务化」的就绪度：已有哪些资产可复用、哪些耦合会卡住拆分、应当**现在**就立哪些「框架规矩」让未来拆分变成机械操作。

**核心判断**：项目底子很好——已是 CI 强制的 6 层无环 DAG（`scripts/check_dag.py`、`.github/workflows/arch-check.yml`），远超普通单体。真正卡服务化的**不是 crate 划分**，而是 3 个跨域「框架性」耦合：① 共享单库 ② 接口只有 Tauri 进程内 IPC ③ 域间直接函数调用。

### 1.2 目标

1. 用**带 `文件:行号` 证据**的阻碍清单替代「感觉上能不能拆」的模糊判断。
2. 给出**服务边界划分**（bounded context → 候选服务）与**目标架构**（六边形 + 端口 + 数据所有权 + 传输无关接口）。
3. 定义**分阶段演进路线**（阶段 0-4），低风险优先、复用已有 P0/P1，每项含 **目标 / 影响面 / 验证 / 回滚**，可独立挑入 `feature_list.json`。

### 1.3 非目标

- 不在本文件内改任何代码 / 配置 / 生成物。
- 不重新设计业务语义（normalize 规则、pipeline 协议、provider fan-out 等保持不变）。
- 不立即启动微服务拆分；本轮只「搭框架 + 立规矩」，真正抽服务在阶段 4 且需用户拍板目标形态。
- 不与 `2026-05-29-architecture-optimization.md` 重复内部去重细节（B-D1…F-C2），本文聚焦**服务化结构**视角。

### 1.4 分期定位（2026-05-30 用户澄清）

> 用户明确：**现阶段只做模块化 / 解耦，最终形态是远程 / 云端服务**；且内层 harness 尚未开始，搭 harness 时发现「中间逻辑」需先理顺，故**先模块化、再搞 harness**。

| 维度 | 现阶段（本轮聚焦） | 最终北极星 |
|---|---|---|
| 目标 | **模块化 / 解耦**：数据所有权边界 + 端口化横向耦合 + 命令层六边形化（阶段 0-3） | **远程 / 云端服务**：垂直域独立部署、Tauri 变薄客户端（阶段 4） |
| 不做 | 不真正拆服务进程、不切独立库、不选传输 | —— |
| 设计约束 | 所有契约 / 端口 / DTO 从现在起按 **remote-ready** 设计（传输无关、网络友好、可序列化、无 `Arc` 共享语义泄漏） | 传输选型（**MCP-over-HTTP/SSE** 复用 `golish-mcp` ↔ **gRPC** 新引入）推迟到阶段 4 决策 |
| 与 harness 关系 | 模块化先行，给 harness 一个干净的依赖边界落地 | harness（stage gate / evidence ledger）在模块化稳定后再启 |

**工程含义**：现在每写一个跨域接口，都按「将来会变成网络调用」来设计（DTO 可序列化、不传 `Arc<PgPool>` / 不传闭包、错误用 `{code,message}`），这样阶段 4 真要远程时，只换适配器、不动业务。

---

## 2. 现状架构：已具备的服务化资产

> 评估服务化前先盘点「不必推倒重来」的资产。证据：各 crate `Cargo.toml`、`docs/architecture.md`。

| 资产 | 现状 | 证据 |
|---|---|---|
| **6 层无环 DAG** | 45 crate 分 6 层，CI 禁回边 | `docs/architecture.md:44-88`、`scripts/check_dag.py`、`.github/workflows/arch-check.yml` |
| **纯领域模型 crate** | `golish-pentest-domain` / `golish-vuln-intel-domain`（纯类型，零内部依赖）—— 服务「契约层」雏形 | `docs/architecture.md:106-107` |
| **领域服务 crate 已拆** | `golish-pentest` / `-vuln-intel` / `-scan-runner` / `-integrations` / `-intel-providers` / `-pipeline` | `backend/Cargo.toml:31-47` |
| **网络传输机已具备** | `golish-mcp` 自带 SSE / HTTP 传输 + OAuth；`golish-pentest-mcp` 已把渗透工具暴露为 MCP tool | `golish-mcp/src/sse_transport.rs`、`golish-mcp/src/oauth/`、`docs/architecture.md:154` |
| **事件基础设施** | `golish-events` + `golish-core::EventEmitter`（已用于流式事件） | `golish-core/src/lib.rs:49`、`golish/src/tools/asset_intel/mod.rs:106-108` |
| **进行中的契约工作** | 错误码契约（I1）、ts-rs 类型同步（I5）、作用域 SQL 下沉（I2）已立项 | `docs/design/2026-05-29-architecture-optimization.md §4-5` |

> 结论：服务化所需的「契约层模式」「网络传输」「事件总线」**都已有先例**，无需从零造轮子。

---

## 3. 服务化阻碍清单（逐条带 `文件:行号`）

### 3.1 阻碍 ①（P0）· 共享单一数据库 —— 最大阻碍

| 项 | 证据 |
|---|---|
| 一个 `PgPool` 承载全部域 | `golish-db/src/repo/mod.rs:1-45` 列 **42 个 repo**：findings / targets / organizations / vuln_intel / vuln_scan / sessions / vault / methodology / pipelines / sitemap_store / fingerprints / api_endpoints / audit / conversation_store… 全在一库 |
| 全局共享 pool，无域边界 | `golish/src/state/db.rs:11-14`（`DbState` 只持有单个 `Arc<PgPool>` + `DbReadyGate`） |
| 跨域可随意 JOIN / 直查表 | 命令层与 repo 层散布裸 SQL（见 `2026-05-29` 文 B-D2：`tools/vault.rs`、`tools/findings/crud.rs`、`tools/methodology.rs`、`tools/pipeline/commands.rs`） |

**后果**：没有数据所有权边界，任意域可读写任意表 → 服务化时无法「一域一库」切分，是头号阻碍。

### 3.2 阻碍 ②（P0）· 接口只有进程内 Tauri IPC

| 项 | 证据 |
|---|---|
| ~550 个 `#[tauri::command]`（覆盖 `golish/src`；2026-05-29 体检注册 533 条） | 遍布 `golish/src/**`（如 `tools/pentest/runtime.rs` 24 个、`tools/security_analysis.rs` 15 个、`tools/wiki/vuln_links.rs` 14 个） |
| 业务逻辑写在命令体、与 Tauri / `AppState` 绑死 | 命令直接取 `tauri::State<'_, DbState>` / `AppState`（`golish/src/state/db.rs:22`） |
| 无传输无关的「服务方法」层 | 命令即接口，没有 `service(req: Dto) -> Result<Dto>` 纯函数可被 HTTP/gRPC 复用 |

**后果**：要把某域暴露为网络服务，必须重写其全部入口；业务与传输无法分离。

### 3.3 阻碍 ③（P1）· 域间横向直连 + god-crate / god-kernel

| 项 | 证据 |
|---|---|
| 平级域互掏内部 | `golish/src/tools/asset_intel/mod.rs:27` `use crate::tools::organizations::{upsert_organization_candidates_for_org, OrganizationCandidates}`；`:30` `use crate::tools::pentest::PentestState`；`:22` `use golish_pentest::models::ToolConfig` |
| `golish` god-crate | out-degree 30，直接装配几乎全部垂直域（`backend/crates/golish/Cargo.toml:22-59`） |
| `golish-core` god-kernel | in-degree 22，把 events / session / tool / pentest_context / vault / prompt / plan / hitl 全塞一个 crate（`golish-core/src/lib.rs:18-74`） |
| 类型四处孪生 | `ToolConfig` 在 `pentest` / `pentest-domain` / `pentest-mcp` / `agent-kit` 至少 4 份（`docs/superpowers/plans/2026-05-30-arch-health-backlog.md` P1-a） |

**后果**：编译期强耦合，任一域无法独立编译 / 独立抽出；契约漂移风险。

---

## 4. 目标架构

### 4.1 六边形（端口-适配器）：业务与传输解耦

```text
        ┌──────────────────────────────────────┐
入站适配器 │            Domain Service Core         │ 出站适配器
          │                                       │
Tauri  ──▶│  InboundPort:  service(req) -> resp   │──▶ Repo (本域 DB)
HTTP   ──▶│  ───────────────────────────────     │──▶ OtherDomainPort (trait)
MCP/SSE──▶│  纯业务逻辑（不依赖 Tauri / sqlx 具体）  │──▶ EventBus
gRPC   ──▶│                                       │
          └──────────────────────────────────────┘
```

- **入站端口**：`#[tauri::command]` 退化为薄适配器，仅做「解析 → 调 service 方法 → 格式化响应」。同一个 service 方法将来可同时挂 HTTP / MCP-over-SSE / gRPC，业务零改动。
- **出站端口**：跨域调用走 `trait`（如 `OrganizationsPort`），现在给进程内实现，将来换网络实现（client stub）。
- **DTO 单源**：请求 / 响应类型用 `#[derive(ts_rs::TS)]`（I5），既喂前端又当服务契约。

### 4.2 数据所有权：一域一 schema → 一域一库

- 每个候选服务**只拥有自己的表**；禁止跨域 JOIN / 跨域直查。
- 过渡期：同一物理库内按域分 schema + repo facade（域命令只能调本域 repo）；可加一条 `check_dag.py` 同款的 CI 守卫。
- 终态：抽服务时把该 schema 整体迁出为独立库，跨域数据需求改走出站端口 / 事件。

### 4.3 域间通信：同步走端口，异步走事件

- 强一致 / 即时读：出站端口 trait（in-proc impl → network impl）。
- 最终一致 / 通知：复用 `golish-events`（如「候选已批准 → pentest 响应」）。遵守 I9（事务内不发外部调用）→ 用 outbox 模式落地。

---

## 5. 服务边界划分（bounded context → 候选服务）

> 按现有 `tools/*`（14 个域）与 `golish-db` repo 的自然接缝归并。

| 候选服务 | 现有 crate / 模块 | 主要拥有的表（repo） |
|---|---|---|
| **资产 / 攻面 Recon** | `tools/asset_intel`、`organizations`、`targets`、`golish-intel-providers`、`golish-integrations`(capture)、`golish-scan-runner`、`golish-auth-probe` | targets、target_assets、organizations、api_endpoints、sitemap_store、directory_entries、fingerprints、js_analysis、passive_scans、sensitive_scan、screenshots |
| **漏洞情报 Vuln-Intel** | `tools/vuln_intel`、`golish-vuln-intel(-domain)`、`tools/wiki` | vuln_intel、vuln_scan、scan_queue、wiki_kb/poc、kb_research |
| **渗透引擎 / 流水线** | `tools/pentest`、`pentest_ai`、`golish-pipeline`、`tools/methodology`、`tools/findings` | findings、methodology、stage_runs、execution_plans、evidence_classifications、subtasks、tasks |
| **Agent / LLM 编排** | `golish-agent-kit/-runtime/-bridge`、`golish-sub-agents`、`golish-prompts`、`golish-llm-providers`、`golish-session` | sessions、conversation_store、message_chains、agent_logs、tool_calls、sub_agent_dispatches、memories |
| **平台 / 凭据 / 审计** | `tools/vault`、`golish-integrations`(creds)、`golish-settings`、`golish-projects`、`tools/audit` | vault、audit、operation_state、prompt_templates |

> **最适合第一个抽出去的是「漏洞情报 Vuln-Intel」**：以拉取 / 读取为主、入站依赖最少、已有 `golish-vuln-intel-domain` 契约 crate、DB 表最独立，适合当「跑通服务化模式」的试金石。

---

## 6. 演进路线图（阶段 0-4）

> 每项含 **目标 / 影响面 / 验证 / 回滚**。低风险优先，复用 `2026-05-29` 路线图的 P0/P1。

### 阶段 0 —— 稳定契约（进行中，前置条件）

复用 `2026-05-29-architecture-optimization.md` 的 **P0-1 错误码契约 / P0-2 ts-rs 同步 / P0-3 作用域 SQL 下沉**。这三项本就是服务化前置：稳定的 `{code,message}` 错误契约 + ts-rs DTO 单源 + repo 收口的作用域守卫，是服务接口的基础。

### 阶段 1 —— 地基：数据所有权 + 端口化横向耦合

#### S1-1 数据所有权边界（防跨域直查）
- **目标**：`golish-db` 按域归组 repo（recon / vuln / pentest / agent / platform），加「repo 只能被本域命令调用」的约束；命令层禁止裸跨域 SQL。
- **影响面**：`golish-db/src/repo/*`、`golish/src/tools/*`、`scripts/check_dag.py`（可加守卫）。
- **验证**：`just test-rust`；grep 确认 `tools/` 下无跨域裸 SQL；新增越权读写返回 `NotFound` 单测（承接 I2）。
- **回滚**：归组为纯重排 + 增量守卫，逐文件迁移，未迁移路径保持原状。

#### S1-2 横向直连改出站端口 trait
- **目标**：消除 `asset_intel → organizations / pentest` 直接 import，引入 `OrganizationsPort` / `PentestPort` trait（进程内实现），放进 `*-domain` 或新 `*-contract` crate。
- **影响面**：`tools/asset_intel/mod.rs:27,30`、`organizations`、`pentest` 模块。
- **验证**：`just test-rust`；确认 `asset_intel` 不再 `use crate::tools::{organizations,pentest}::*`。
- **回滚**：trait 为新增抽象，旧直连可在迁移完成前并存。

#### S1-3 `ToolConfig` 收敛单源（消契约漂移）
- **目标**：确定 owner crate（建议 `golish-pentest-domain`），其余改 re-export；先写设计确认依赖图不成环（承接 backlog P1-a，属 AGENTS.md §1.3 必须先设计）。
- **影响面**：`pentest` / `pentest-domain` / `pentest-mcp` / `agent-kit`。
- **验证**：`just check`；类型 diff 确认 shape 一致。
- **回滚**：单源失败可回退到各自定义。

### 阶段 2 —— 接口：命令层六边形化 + 事件驱动

#### S2-1 命令体下沉为 service 方法
- **目标**：每个 `#[tauri::command]` 退化为薄适配器，业务移入 `service(req: Dto) -> Result<Dto, GolishError>`；DTO 用 ts-rs。
- **影响面**：按域逐批迁移 `golish/src/tools/<域>/`。
- **验证**：`just check` + 对应 nextest；适配器无业务分支。
- **回滚**：逐命令迁移，未迁移命令保持原样。

#### S2-2 域间异步通知改事件
- **目标**：把跨域同步副作用（如候选批准触发 pentest）改 `golish-events` 事件 + 消费者，配 outbox 落地（I9）。
- **影响面**：相关域 + `golish-events`。
- **验证**：`just test-rust`；事件投递 / 幂等消费单测。
- **回滚**：事件路径与旧直调可短期并存。

### 阶段 3 —— 解耦内核 + 碎 god-crate

#### S3-1 拆 `golish-core` god-kernel
- **目标**：把 session/agent 类型与 pentest/tool/vault 类型分到小契约 crate，降低 in-degree 22 的「全量拖入」。
- **影响面**：`golish-core` 及其下游（量大，分批）。
- **验证**：`just check`；DAG 守卫通过。
- **回滚**：保留旧 re-export 一个版本周期。

#### S3-2 碎 `golish` god-crate 为按域 app crate
- **目标**：把命令模块抽成 `golish-recon-app` / `golish-pentest-app` / `golish-vuln-app` 等，主二进制只做组装与 Tauri 绑定。
- **影响面**：`golish` crate 拆分；`commands_registry.rs` 组装方式。
- **验证**：`just check`；各 app crate 可独立 `cargo check`。
- **回滚**：纯结构性拆分，单 PR revert。

### 阶段 4 —— 真抽第一个服务（Vuln-Intel）

- **目标**：给 Vuln-Intel 独立 schema / 库 + 复用 `golish-mcp` 的 HTTP/SSE（或 gRPC）暴露为服务；Tauri 端通过出站端口的 network impl 调用，变薄客户端。
- **影响面**：`golish-vuln-intel`、`tools/vuln_intel`、新服务二进制、传输层。
- **验证**：服务独立启动 + 健康检查；Tauri 端经网络端口跑通 vuln 查询；契约测试（consumer-driven）。
- **回滚**：保留 in-proc impl，端口可切回进程内实现。

---

## 7. 风险与回滚

| 风险 | 说明 | 缓解 |
|---|---|---|
| 切库破坏跨域 JOIN | 现有跨域查询在切 schema/库后断裂 | 阶段 1 先建端口 + 事件替代跨域读，再切库；grep 兜底 |
| 命令名 / 类型变更断前端 | 六边形化 / DTO 单源可能改签名 | 遵守 I5：ts-rs 生成 + 前后端同 commit；先加新名 alias |
| god-crate 拆分 scope 蔓延 | 拆分类任务易顺手改无关代码 | 每项一个 feature，遵守 AGENTS.md §3「不引入 scope 外改动」 |
| schema 迁移回滚 | 阶段 4 切库触及 migration | 遵守 I10：先扩字段 / 双写 → 再上代码 → 再清旧 |
| 传输选型反复 | MCP-over-HTTP vs gRPC 未定 | 由 §10 判断点先定目标形态，再选传输 |

**统一回滚原则**：所有项设计为「增量叠加 + 逐文件/逐域迁移」，未迁移路径保持旧行为，单项可独立 revert。

---

## 8. 验证方式

| 层 | 命令 | 用途 |
|---|---|---|
| 全套门禁 | `just precommit` | commit 前必跑，全绿才提交（AGENTS.md §2.6） |
| 静态 + 单测 | `just check` | fmt + check-fe + test-fe + lint-rust + test-rust-all |
| 后端 | `just lint-rust` / `just test-rust` | clippy 零 warning / cargo nextest |
| DAG | `python3 scripts/check_dag.py` | 层级约束 + 新增的「域 repo 隔离」守卫 |
| 契约 | （阶段 4 新增）consumer-driven contract test | 服务边界契约不漂移 |

**完成定义**（对齐 AGENTS.md §3）：每项落地必须有实际跑过的验证命令 + 证据记录到 `agent-progress.md`，并把 `feature_list.json` 对应条目的 `verification` 逐条核对、填 `evidence`。**没有新鲜验证证据不许宣称完成。**

---

## 9. 不在本次范围（deferred）

- **真正的微服务部署 / 编排**（K8s、服务发现、网关）：等阶段 4 跑通单服务后另起设计。
- **内层 domain harness**（stage gate / evidence ledger）：见 `docs/design/2026-05-20-agent-harness-strategy.md`，仍 deferred。
- **业务语义重设计**：normalize 规则、provider fan-out、pipeline 执行语义不变。
- **前端拆分**：巨型面板拆分见 `2026-05-29` 文 P1-4/P1-5，本文不重复。

---

## 10. 关键判断点

### 已定（2026-05-30）
- **目标形态 = 远程 / 云端**（最终北极星），但**现阶段只做模块化 / 解耦**（阶段 0-3），不真正拆服务。见 §1.4。
- **传输选型推迟**到阶段 4；现阶段一切按 remote-ready 设计即可，不锁 MCP-over-HTTP / gRPC。
- **顺序 = 先模块化、再 harness**。

### 待拍板（决定下一步实现）
1. 现阶段从哪块**先动手**？建议顺序：**S1-1 数据所有权边界**（性价比最高、纯加固）→ **S1-2 端口化横向耦合**（消 `asset_intel→organizations/pentest`）→ **S1-3 ToolConfig 收敛**。
2. 是否把**阶段 1（S1-1/S1-2/S1-3）**挑入 `feature_list.json` 作为第一批 `not_started`，并为首项在 `docs/superpowers/plans/` 写实现计划？
3. 用户提到「搭 harness 时发现中间逻辑有问题」——是否需要我先定位 / 记录那处逻辑问题（systematic-debugging），把它纳入模块化范围一起理顺？

---

> 本文档为只读架构体检的固化产物。后续每挑一项进入实现，请先在 `docs/superpowers/plans/` 写实现计划（`.cursor/skills/writing-plans/`），再按 `executing-plans` 推进，遵守 AGENTS.md §3 完成定义。
