# 跨 IPC / 跨 crate 类型收敛到 ts-rs 单一真相源 (I5) 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 配套：`docs/superpowers/plans/2026-05-30-arch-health-backlog.md`（本计划是其 **P1-a / P1-b** 的详细展开）。
> 作者：MCP-4（data_dev）。分工：MCP-3 后端文件拆分 / MCP-5 前端组件拆分 / 本计划 类型收敛(I5)。
> 不变量：`AGENTS.md` I5（跨 IPC 类型用 `#[derive(ts_rs::TS)]` 同步，不手维护两份）、I6（设计变更走新 md）、§1.3（跨 crate / 改 IPC 改动先写设计文档）、§2.7（高风险先确认）。

**目标：** 消除「同一业务概念在 Rust 侧多份手写表示 + 前端再手写一份镜像」的 I5 历史债，让跨 IPC 边界的类型由 `ts-rs` 单源生成、`just check-types` 守门；对**同名异义**类型改名消歧（不合并）；对**真孪生**类型按依赖图安全收敛到 owner crate。
**架构：** 分四阶段、每阶段独立验证 + 独立 commit。Phase 0 先落设计文档并取得用户确认（§1.3/§2.7 硬门禁）；Phase 1 做零跨 crate 风险的「Finding 线 IPC DTO → ts-rs」单源化（最高收益、最低风险，作为模板）；Phase 2 纯改名消歧；Phase 3 在依赖图确认无环后收敛 `ToolConfig` 真孪生；Phase 4 评估 `StoreStats`/`ParseResult`/`PlanStep` 同型三胞胎是否上提领域 crate。**所有阶段行为零变更**（wire 格式保持），用 `just check-types` + serde round-trip 测试 + `pnpm typecheck` 四道守住。
**技术栈：** Rust 2021 + `ts-rs = 12.0.1`（features `serde-json-impl` + `chrono-impl`）+ `cargo nextest` + React 19 / TS 6 + Vitest + Biome；统一命令走 `just`。

---

## 背景与证据（2026-05-30 实跑核对）

ts-rs 管线已就位（**不需要新建**）：

- 现有 27 个生成文件在 `frontend/lib/generated/*.ts`（如 `PlanStep.ts` / `AuditEntry.ts` / `Note.ts`）。
- 生成方式：带 `#[ts(export)]` 的类型在 `cargo test --workspace export_bindings` 时作为 `export_bindings_*` 测试的副作用写盘。
  - `just gen-types` → `cd backend && cargo test --workspace export_bindings -q`
  - `just check-types` → `gen-types` 后 `git diff --exit-code -- frontend/lib/generated/`（这是 I5 漂移门禁）。
- 统一 incantation（**所有现有 crate 一字不差地用这一行**，与 crate / 文件深度无关）：
  ```rust
  #[ts(export, export_to = "../../../../frontend/lib/generated/")]
  ```
  参照样板：`backend/crates/golish-core/src/plan.rs:22-119`（`StepStatus` / `FailureKind` / `PlanStep` / `PlanSummary`）。

**已有先例（本计划遵循同一范式）**：存储层 `golish-db::models::Note`（`FromRow`）与 wire 层 `golish::tools::notes::Note`（`ts_rs::TS`）是「DB 行 vs IPC DTO」两类型并存且被接受 —— 本计划对 Finding 走完全相同的范式（**只给 wire DTO 加 ts-rs，不动 `FromRow` DB 结构**）。

### 证据快照

| 集群 | 位置 | 形态 | 结论 |
|---|---|---|---|
| **Finding（wire DTO）** | `golish/src/tools/findings/mod.rs:19` | `id: String` / `severity: Severity`(本地 lowercase enum) / `tags: Vec<String>` / `created_at: u64` / `references` / `evidence: Vec<Evidence>` | **加 ts-rs，前端改用生成类型**（Phase 1） |
| Finding（DB 行） | `golish-db/src/models/pentest.rs:96` | `id: Uuid` / `sev: Severity` / `tags: serde_json::Value` / `created_at: DateTime<Utc>` / 有 `project_path`、无 `target_id` | 存储型，**保留不动** |
| Finding（投影行） | `golish-db/src/repo/findings.rs:12` `FindingDetailRow` | `id: Uuid` / `sev: String` / `status: String` / 有 `target_id` | 投影型，**保留不动**；映射在 `findings/mod.rs:152 impl From` |
| 前端手写镜像 | `frontend/lib/api/findings.ts:7` `interface Finding` + `:31 Evidence` | `severity: string` / `status: string` / snake_case 字段 | **删除，改 re-export 生成类型**（Phase 1） |
| **Finding（同名异义 A）** | `golish-auth-probe/src/types.rs:107` | `endpoint` / `scenario` / `verdict` / `evidence: Evidence(round_1..3)` —— IDOR 探针结果 | **改名 `ProbeFinding`，不合并**（Phase 2） |
| **Finding（同名异义 B）** | `golish-agent-kit/src/harness/types.rs:131` | `finding_id: Uuid` / `kind` / `subject` / `evidence_refs` —— harness 交付物 | **改名 `HarnessFinding`，不合并**（Phase 2） |
| **ToolConfig（canonical）** | `golish-pentest-domain/src/models/tool_config.rs:65` | 全字段（`pentest_phase`/`asset_intel`/`params`/`output`/`skills`…） | **owner**；`golish-pentest::models` 已 `pub use golish_pentest_domain::models::*`（`models.rs:14`）**verbatim 再导出，非独立副本** |
| **ToolConfig（真孪生·子集）** | `golish-pentest-mcp/src/models.rs:16` | `pub(crate)`、`Deserialize` only、字段子集（id/name/executable/runtime/params/skills/jvm_options），由 `ToolConfigFile { tool }` 包裹 | **真孪生**：收敛到 domain（Phase 3） |
| **ToolConfig（同名异义）** | `golish-agent-kit/src/tool_definitions/config.rs:9` | `preset` / `additional` / `disabled` —— 工具**选择**预设，与「单工具 JSON 描述」无关 | **改名 `ToolSelectionConfig`，不合并**（Phase 2） |
| ToolConfig（域外） | `rig-gemini-vertex/src/types.rs:207` | Gemini API 请求体 | **out of scope**，不动 |
| 同型三胞胎 | `golish-core/src/plan.rs:85`(PlanStep, 已 ts-rs) ↔ `golish-db/src/models/session.rs:174` ↔ `golish-agent-kit/src/db_traits/types.rs:102`；`StoreStats`×3（`golish-pipeline/src/parser.rs:38` / `golish-pentest/src/output_store/mod.rs:51` / `golish/src/tools/output_parser.rs:294`）；`ParseResult`×3（`golish-pty/src/parser/types.rs:15` / `golish-pentest/src/output_parser.rs:13` / `golish/src/tools/output_parser.rs:14`） | 同名，shape 可能已分叉 | **先 diff 评估再决定**（Phase 4，可能部分 defer） |

### 依赖图（cycle 分析，已核对各 `Cargo.toml`）

- `golish-pentest-domain`：**零 `golish-*` 依赖**（叶子 / L1 域 crate）→ 最稳，作 owner 无环风险。
- `golish-pentest` → 依赖 `golish-pentest-domain`（`Cargo.toml:11`），且 `models.rs:14` 已 `pub use` 再导出 → ToolConfig **已收敛**到 domain。
- `golish-pentest-mcp` → 现依赖 `golish-core` + `golish-shell-exec`，**未**依赖 `golish-pentest-domain`。新增 `golish-pentest-mcp → golish-pentest-domain`：因 domain 是叶子 → **不可能成环**。✅
- `golish-agent-kit` → 依赖 `golish-pentest`（`Cargo.toml:23`）等 → 其 `ToolConfig` 是「选择预设」，改名即可，无跨 crate 合并。

---

## 三类问题分流（执行前必须认清）

| 类别 | 判据 | 处理 | 本计划 |
|---|---|---|---|
| **真合并** | 同概念、同 wire 形态（或子集），shape 未本质分叉 | 收敛到 owner crate，其余 `pub use` | ToolConfig（mcp 子集 → domain）：Phase 3 |
| **I5 单源化** | 业务概念跨 IPC，前端有手写镜像 | wire DTO 加 ts-rs，前端 import 生成物 | Finding 线：Phase 1；其余手写镜像：附录 A |
| **改名不合并** | 同名但语义不同（"已检查为空 ≠ 未检查" 式陷阱，违 I8 风险） | 重命名消歧，**禁止合并** | ProbeFinding / HarnessFinding / ToolSelectionConfig：Phase 2 |
| **评估上提** | 同型多份，shape 是否一致未知 | 先 diff，再决定上提或保留 | StoreStats / ParseResult / PlanStep：Phase 4 |

> **反面教训（写进设计文档）**：原 backlog「ToolConfig ≥4 份孪生」需修正——`golish-pentest` 是 domain 的 `pub use` 再导出（非副本），`golish-agent-kit` 与 `rig-gemini-vertex` 是同名异义。**真正可合并的只有 `golish-pentest-mcp` 一份**。盲目「合并 4 份」会把语义不同的类型强行揉成一个，是 bug。

---

## 目标文件结构

```text
docs/design/
  2026-05-30-type-dedup-tsrs.md          # Phase 0 新建：设计确认（owner/cycle/shape-fork/合并 vs 改名）

backend/crates/golish/src/tools/findings/
  mod.rs                                  # Finding/Evidence/Severity/FindingStatus 加 ts-rs derive（Phase 1）

frontend/lib/generated/                   # 由 ts-rs 生成（勿手改）：新增 Finding.ts / Evidence.ts / Severity.ts / FindingStatus.ts
frontend/lib/api/findings.ts             # 删手写 Finding/Evidence，改 re-export 生成类型（Phase 1）

backend/crates/golish-auth-probe/src/types.rs        # Finding → ProbeFinding（Phase 2）
backend/crates/golish-agent-kit/src/harness/types.rs # Finding → HarnessFinding（Phase 2）
backend/crates/golish-agent-kit/src/tool_definitions/config.rs  # ToolConfig → ToolSelectionConfig（Phase 2）

backend/crates/golish-pentest-mcp/
  Cargo.toml                             # + golish-pentest-domain 依赖（Phase 3）
  src/models.rs                          # 删本地 ToolConfig 子集，pub use domain 的（Phase 3）
```

---

## Phase 0 — 设计确认（硬门禁，未通过不得进 Phase 3）

### Task 0.1 — 写设计文档 `docs/design/2026-05-30-type-dedup-tsrs.md`

**文件：** 新建 `docs/design/2026-05-30-type-dedup-tsrs.md`
**步骤：** 按以下骨架填写（内容用本计划「证据快照 + 依赖图 + 三类问题分流」三节的结论，逐条落字）：

```markdown
# 设计：跨 IPC/跨 crate 类型收敛到 ts-rs 单源（I5）

## 1. 问题陈述
（Finding 线 3 表示 + FE 镜像；ToolConfig 真孪生 vs 同名异义；同型三胞胎）

## 2. 每个集群的三问答复（§1.3 必答）
| 集群 | ① owner crate | ② 依赖图是否成环 | ③ shape 是否已分叉 | 决策 |
| Finding wire DTO | golish（IPC 层） | 否（不跨 crate 移动） | 与 DB 行/投影行本就分层 | 加 ts-rs，FE 改生成类型 |
| ToolConfig | golish-pentest-domain（叶子） | 否（mcp→domain 安全） | mcp 是子集，未分叉 | mcp 收敛到 domain |
| ProbeFinding/HarnessFinding | 各自原 crate | n/a | 同名异义 | 改名，不合并 |
| ToolSelectionConfig | golish-agent-kit | n/a | 同名异义 | 改名，不合并 |
| StoreStats/ParseResult/PlanStep | 待 Phase 4 diff | 待评估 | 待评估 | 先测量 |

## 3. 行为兼容性
- wire JSON 字段名/取值零变更（serde 形态保持）
- FindingStatus serde(rename_all=lowercase) 产出 "falsepositive"，与手写 as_str() 的 "false_positive" 不一致——见 §4 风险

## 4. 风险与回滚
- 每阶段独立 commit；回滚 = revert 单个 commit
- check-types 漂移门禁兜底

## 5. 决策记录
（待用户在 chat 确认 Phase 2 改名命名 + Phase 3 合并）
```

**验证：** 文档存在且 §2 表格四列填满。`ls docs/design/2026-05-30-type-dedup-tsrs.md`
**提交：** `docs(design): type-dedup to ts-rs single source (I5) design`

### Task 0.2 — 取得用户确认（§2.7 高风险确认）

**文件：** 无（chat 交互）
**步骤：** 在 chat 明确请用户拍板两件事，**得到确认前不得执行 Phase 2/3**：
1. Phase 2 改名命名：`ProbeFinding` / `HarnessFinding` / `ToolSelectionConfig` 是否采纳（或用户另定名）。
2. Phase 3 是否执行 `golish-pentest-mcp::ToolConfig` → `golish-pentest-domain` 收敛（新增一条 crate 依赖）。
**验证：** chat 中有用户「同意/调整」回复，记入 Task 0.1 文档 §5。
**提交：** 无（仅更新设计文档 §5，可并入下一个 commit）。

---

## Phase 1 — Finding 线 IPC DTO → ts-rs 单源（I5，零跨 crate 风险）

> 不依赖 Phase 0 用户确认即可执行（不跨 crate、不改名、wire 不变）。作为后续阶段的模板。

### Task 1.1 — 基线

**文件：** 无（只读）
**步骤：**
```bash
cd backend && cargo check -p golish -q; echo "check=$?"
just gen-types && git diff --quiet -- ../frontend/lib/generated/ && echo "generated clean baseline"
rg -n "lib/api/findings" ../frontend --glob '*.ts' --glob '*.tsx'   # 记录 Finding 的消费面
```
**验证：** `check=0`；generated 干净；记录所有 import `findings.ts` 的文件。
**提交：** 无。

### Task 1.2 — 给 wire DTO 加 ts-rs derive

**文件：** `backend/crates/golish/src/tools/findings/mod.rs`
**步骤：** 对 `Finding` / `Evidence` / `Severity` / `FindingStatus` 四个类型加 derive 与 export 属性（**只加注解，字段一字不改**）：

```rust
// 现 18-19 行
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    #[serde(default)]
    #[ts(optional = nullable)]            // 现 cvss: 产出 `cvss?: number | null`，对齐手写镜像的 cvss?
    pub cvss: Option<f64>,
    // …中间字段不变…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]            // target_id 产出 `target_id?: string`
    pub target_id: Option<String>,
    // …其余字段不变…
    #[ts(type = "number")]               // u64 → number（防 ts-rs 产出 bigint）
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

// 现 58-59 行
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Evidence {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub caption: String,
    #[ts(type = "number")]
    pub added_at: u64,
}

// 现 67-68 行（Severity 已有 #[serde(rename_all = "lowercase")]，ts-rs 会产 "critical"|"high"|…）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum Severity { /* 变体不变 */ }

// 现 98-99 行
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum FindingStatus { /* 变体不变 */ }
```

> ⚠️ **FindingStatus 取值审查（必做）**：枚举上 `#[serde(rename_all = "lowercase")]` 会让 `FalsePositive` 序列化为 `"falsepositive"`，而手写 `as_str()`（`mod.rs:117`）落库用 `"false_positive"`。两条路径取值不一致是**既有**情况（DB 落库走 `as_str`，serde wire 走 rename_all）。ts-rs 生成的 union 跟随 serde → `"falsepositive"`。**本 Task 不改行为**，但在 Task 0.1 设计文档 §4 记录该不一致；若决定统一，另开任务（不混入本计划）。

`golish` crate 已声明 `ts-rs`（`golish/Cargo.toml:135`），无需改 `Cargo.toml`。

**验证：**
```bash
cd backend && cargo check -p golish -q; echo "check=$?"
```
预期 `check=0`。
**提交：** `feat(findings): derive ts_rs::TS on wire Finding DTO (I5)`

### Task 1.3 — 生成 TS 并核对

**文件：** 生成 `frontend/lib/generated/{Finding,Evidence,Severity,FindingStatus}.ts`
**步骤：**
```bash
just gen-types
ls frontend/lib/generated/Finding.ts frontend/lib/generated/Evidence.ts \
   frontend/lib/generated/Severity.ts frontend/lib/generated/FindingStatus.ts
```
人工核对 `Finding.ts` 字段与 `frontend/lib/api/findings.ts` 现有手写 `interface Finding` 等价（差异点应仅为 `severity`/`status` 由 `string` 收紧为生成 union —— 这是预期增强）。
**验证：** 4 个 .ts 文件存在；字段集一致。
**提交：** `chore(types): regenerate ts-rs bindings for Finding cluster`

### Task 1.4 — 前端改用生成类型，删手写镜像

**文件：** `frontend/lib/api/findings.ts`
**步骤：**
1. 删除本地 `export interface Finding {…}`（现 7-29）与 `export interface Evidence {…}`（现 31-37）。
2. 顶部改为 re-export 生成类型（保持对外 import 路径 `@/lib/api/findings` 不变）：
```ts
import { invoke } from "@/lib/api/client";
import type { Finding } from "@/lib/generated/Finding";
import type { Evidence } from "@/lib/generated/Evidence";

export type { Finding, Evidence };

export interface FindingsStore {
  findings: Finding[];
}
```
3. `pnpm exec biome check --write frontend/lib/api/findings.ts` 整理 import。

**验证：**
```bash
pnpm typecheck
```
若报错来自把 `severity`/`status` 当宽松 `string` 用的旧代码 → 按生成 union（`Severity`/`FindingStatus`）窄化（如比较字面量、或在边界处 `as`）。逐处修到 `pnpm typecheck` 通过。
**提交：** `refactor(findings-fe): consume ts-rs generated Finding/Evidence (I5)`

### Task 1.5 — 漂移门禁回归

**文件：** 无
**步骤：**
```bash
just check-types                 # gen-types + git diff --exit-code
cd backend && cargo nextest run -p golish -q
```
**验证：** `check-types` exit 0（无漂移）；golish 测试全过。
**提交：** 若前序已 commit，本步无新改动。

---

## Phase 2 — 同名异义类型改名（无合并，编译器驱动）

> 依赖 Phase 0 Task 0.2 用户确认命名。三个子任务**互相独立**，各自独立 commit。纯改名，wire 形态不变（这些类型当前都**未** ts-rs 导出，不影响前端）。

### Task 2.1 — `golish-auth-probe` Finding → ProbeFinding

**文件：** `backend/crates/golish-auth-probe/src/types.rs` + 本 crate 内所有引用
**步骤：**
1. 在 `types.rs:107` 把 `pub struct Finding` 改名 `pub struct ProbeFinding`。
2. 编译器驱动改引用：
```bash
cd backend && cargo check -p golish-auth-probe -q 2>&1 | rg "cannot find|expected" | head
rg -n "\bFinding\b" crates/golish-auth-probe/src   # 确认仅剩本意为 ProbeFinding 的处
```
逐处把指向该类型的 `Finding` 改 `ProbeFinding`（注意：同文件可能有无关的 `Evidence`，勿误改）。
**验证：** `cd backend && cargo check -p golish-auth-probe -q; echo $?` → 0。下游若有引用（`rg -n "auth_probe::.*Finding|ProbeFinding" crates`）一并改并 `cargo check` 各下游 crate。
**提交：** `refactor(auth-probe): rename Finding -> ProbeFinding (disambiguate I8)`

### Task 2.2 — `golish-agent-kit::harness` Finding → HarnessFinding

**文件：** `backend/crates/golish-agent-kit/src/harness/types.rs` + 引用
**步骤：**
1. `types.rs:131` `pub struct Finding` → `pub struct HarnessFinding`。
2. 改 `ExternalAttackSurfaceDeliverable.findings: Vec<Finding>`（现 `:150`）为 `Vec<HarnessFinding>`，及其它引用。
```bash
cd backend && rg -n "\bFinding\b" crates/golish-agent-kit/src/harness
```
**验证：** `cd backend && cargo check -p golish-agent-kit -q; echo $?` → 0。
**提交：** `refactor(agent-kit): rename harness Finding -> HarnessFinding (disambiguate I8)`

### Task 2.3 — `golish-agent-kit::tool_definitions` ToolConfig → ToolSelectionConfig

**文件：** `backend/crates/golish-agent-kit/src/tool_definitions/config.rs` + 引用
**步骤：**
1. `config.rs:9` `pub struct ToolConfig` → `pub struct ToolSelectionConfig`；同步 `impl ToolConfig`（`with_preset`/`main_agent`/`is_none_preset` 等）改 `impl ToolSelectionConfig`。
2. 改本 crate 与下游引用：
```bash
cd backend && rg -n "tool_definitions::ToolConfig|ToolConfig::(with_preset|main_agent)" crates
```
**验证：** `cd backend && cargo check -p golish-agent-kit -q; echo $?` → 0；受牵连下游 `cargo check` 通过。
**提交：** `refactor(agent-kit): rename selection ToolConfig -> ToolSelectionConfig (disambiguate I8)`

---

## Phase 3 — ToolConfig 真孪生收敛（gated on Phase 0 Task 0.2）

> 仅当用户在 Task 0.2 确认后执行。把 `golish-pentest-mcp::ToolConfig`（deserialize-only 子集）收敛到 owner `golish-pentest-domain::ToolConfig`。

### Task 3.1 — 加依赖

**文件：** `backend/crates/golish-pentest-mcp/Cargo.toml`
**步骤：** 在 `[dependencies]` 增（与现有 `golish-core = { path = "../golish-core" }` 同风格）：
```toml
golish-pentest-domain = { path = "../golish-pentest-domain" }
```
**验证：**
```bash
cd backend && cargo check -p golish-pentest-mcp -q; echo "check=$?"
cargo tree -p golish-pentest-mcp -i golish-pentest-domain   # 确认依赖建立、无环报错
```
预期 `check=0`，无 cyclic dependency 报错（domain 是叶子）。
**提交：** `build(pentest-mcp): depend on golish-pentest-domain`

### Task 3.2 — 用 domain 类型替换本地子集

**文件：** `backend/crates/golish-pentest-mcp/src/models.rs`
**步骤：**
1. 删本地 `pub(crate) struct ToolConfig`（现 16-38）及其私有伴随 `ToolParam`/`ParamOption`/`ToolSkill`（现 40-95，若与 domain 等价）。
2. 保留 `ToolConfigFile { tool: ToolConfig }` 包裹，但 `tool` 改用 domain 类型：
```rust
use golish_pentest_domain::models::ToolConfig;

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct ToolConfigFile {
    pub(crate) tool: ToolConfig,
}
```
> domain 的 `ToolConfig` 是 `Serialize + Deserialize` 超集；mcp 之前只读子集，反序列化同一份 toolsconfig JSON，serde 对多余字段默认忽略 → **读取行为完全保持**。若 mcp 代码访问了 domain 版没有的字段，编译会报错→按 domain 字段名调整访问点（domain 字段更全，通常只是 `pub(crate)`→`pub` 可见性差异，不会缺字段）。

**验证：**
```bash
cd backend && cargo check -p golish-pentest-mcp -q; echo $?      # 0
cargo nextest run -p golish-pentest-mcp -q                        # 既有测试全过
```
**提交：** `refactor(pentest-mcp): converge ToolConfig onto golish-pentest-domain (I5)`

### Task 3.3 — round-trip 兜底测试

**文件：** `backend/crates/golish-pentest-mcp/src/models.rs`（`#[cfg(test)]`）
**步骤：** 加一条「用真实 toolsconfig JSON 反序列化进 `ToolConfigFile` 不报错」的测试，锁定收敛后读取行为：
```rust
#[cfg(test)]
mod tests {
    use super::ToolConfigFile;

    #[test]
    fn deserializes_real_toolconfig_subset() {
        // 取一份带 mcp 关心字段的最小 JSON（与 resources/toolsconfig/*.json 同形）
        let json = r#"{"tool":{"id":"nmap","name":"nmap","executable":"nmap",
            "runtime":"native","launchMode":"cli","params":[],"skills":[],
            "pentestPhase":["enum"]}}"#;
        let parsed: ToolConfigFile = serde_json::from_str(json).expect("must parse");
        assert_eq!(parsed.tool.id, "nmap");
        assert_eq!(parsed.tool.name, "nmap");
    }
}
```
**验证：** `cd backend && cargo nextest run -p golish-pentest-mcp deserializes_real_toolconfig_subset -q` → 1 passed。
**提交：** `test(pentest-mcp): lock ToolConfig converge round-trip`

---

## Phase 4 — 同型三胞胎评估（测量优先，可部分 defer）

> 目的：**先测量再决定**，禁止无 diff 直接合并（shape 可能已分叉）。

### Task 4.1 — diff 三组同型类型，产出决策表

**文件：** 无（只读 + 把结论写进 `docs/design/2026-05-30-type-dedup-tsrs.md` §2 表）
**步骤：** 对每组逐字段对比：
```bash
cd backend
# PlanStep：golish-core(已 ts-rs，canonical) vs golish-db vs golish-agent-kit
rg -n -A30 "struct PlanStep" crates/golish-core/src/plan.rs \
  crates/golish-db/src/models/session.rs crates/golish-agent-kit/src/db_traits/types.rs
# StoreStats ×3
rg -n -A12 "struct StoreStats" crates/golish-pipeline/src/parser.rs \
  crates/golish-pentest/src/output_store/mod.rs crates/golish/src/tools/output_parser.rs
# ParseResult ×3
rg -n -A12 "struct ParseResult" crates/golish-pty/src/parser/types.rs \
  crates/golish-pentest/src/output_parser.rs crates/golish/src/tools/output_parser.rs
```
判定规则：
- **字段完全一致 + 同语义** → 标记「可上提」，owner 取依赖图最稳的叶子 crate（PlanStep 首选 `golish-core`，它已是 ts-rs canonical）。
- **shape 已分叉**（字段不同 / `golish-pty::ParseResult` 是 PTY 解析、与工具输出解析不同概念）→ 标记「保留 + 文档说明为何不合并」（同名异义，勿合并）。
**验证：** 设计文档 §2 表新增三行，每行填 owner / 成环 / 分叉 / 决策。
**提交：** `docs(design): record same-shape triple dedup decisions`

### Task 4.2 — 仅对「可上提且零分叉」者执行收敛（逐个独立 commit）

**文件：** 视 Task 4.1 结论而定
**步骤：** 对判定「可上提」的（如确认 `golish-db::PlanStep` / `golish-agent-kit::PlanStep` 与 `golish-core::PlanStep` 等价）：删副本、`pub use golish_core::plan::PlanStep`，编译器驱动改引用。**判定「分叉/异义」者本计划不动**，留文档备案。
**验证：** 每收敛一个 → 该 crate + 下游 `cargo check` + `cargo nextest` 通过；涉 ts-rs 的 → `just check-types` 0。
**提交：** 每个收敛单独 commit，如 `refactor(core): converge PlanStep onto golish-core::plan (I5)`。

---

## 全量验证与收尾

**步骤/验证：**
```bash
just precommit        # = check (fmt + check-fe + test-fe + lint-rust + test-rust-all) + test
just check-types      # I5 漂移门禁单独再确认一次
```
预期：`precommit` 全绿；`check-types` exit 0。
**收尾：** 更新 `agent-progress.md`（本轮证据）；`feature_list.json` 对应条目状态置 `passing` 并填 `evidence`；按 `clean-state-checklist.md` 核对。

---

## 自检

1. **范围覆盖度：**
   - backlog P1-a（ToolConfig 收敛）→ Phase 0 设计 + Phase 3（已修正为「仅 mcp 一份真孪生」）。✓
   - backlog P1-b（前端手写镜像 → ts-rs）→ Phase 1（Finding 线）+ 附录 A（其余镜像）。✓
   - 同名异义（ProbeFinding/HarnessFinding/ToolSelectionConfig）→ Phase 2。✓
   - 同型三胞胎 → Phase 4（测量优先）。✓
   - §1.3「跨 crate 先设计」+ §2.7「高风险先确认」→ Phase 0 前置门禁。✓
2. **占位符扫描：** 各 Task 给真实路径、真实行号、真实 derive/属性代码、真实命令与期望退出码；无 TODO/待定。✓
3. **类型一致性：** Phase 1 的 `Finding`/`Evidence`/`Severity`/`FindingStatus` 在加 derive 与 FE re-export 两端命名一致；Phase 2 改名（`ProbeFinding`/`HarnessFinding`/`ToolSelectionConfig`）在 struct 定义、`impl` 块、下游引用三处一致；Phase 3 `golish_pentest_domain::models::ToolConfig` 全程同名。✓
4. **行为保持：** 全程 wire JSON 形态零变更；用 `just check-types`（漂移门禁）+ serde round-trip 测试 + `cargo nextest` + `pnpm typecheck` 四道守住；唯一「增强而非变更」是 FE `severity`/`status` 由 `string` 收紧为生成 union（Task 1.4 已列处理）。✓
5. **依赖安全：** Phase 3 唯一新增依赖 `golish-pentest-mcp → golish-pentest-domain`，domain 为零 `golish-*` 依赖的叶子 crate，`cargo tree -i` 验证无环。✓

---

## 附录 A — 其余前端手写镜像（backlog P1-b 续，非阻塞）

`frontend/lib/pentest/types.ts` 等手写镜像（agent-progress 多轮提到）按 Phase 1 同范式逐型收敛：后端对应 wire DTO 加 ts-rs → `just gen-types` → 前端 import 生成物 → 删手写。**每型独立 commit**，验证三连：`just check-types` + `pnpm typecheck` + 相关 `vitest`。建议优先级：先收敛被多处 import、字段最易漂移的（如 `PortInfo` / `ToolConfig` 前端镜像，若存在），逐个推进，避免一次性大 diff。

## 附录 B — 不在本计划 scope（明确划界）

- `rig-gemini-vertex::ToolConfig`：Gemini API 类型，与 pentest 工具配置无关，**不动**。
- `golish-db::models::{Finding, Note, AuditEntry}`（`FromRow` 存储型）与 `FindingDetailRow`（投影型）：与 wire DTO 分层是有意设计（先例：`Note` 已是 DB+wire 双类型并存），**保留不动**。
- FindingStatus 的 `serde(rename_all=lowercase)` 产 `"falsepositive"` vs `as_str()` 的 `"false_positive"` 不一致：**既有行为**，仅在设计文档记录；若要统一另开任务，不混入本计划（避免 scope 蔓延）。
