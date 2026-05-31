# Crate-per-Service 拆分实现计划（god-crate `golish` → 按域 app crate）

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans/` 逐里程碑实现；每个里程碑（M1 起）落地前先按 `.cursor/skills/writing-plans/` 写该里程碑的**细粒度子计划**（本文是序列 + 第一步骨架，不是全 5 服务的逐行代码）。

**目标：** 把顶层 god-crate `golish`（out-degree 30、308 个 Tauri 命令塞在一起）按 5 个服务域拆成独立 app crate（`golish-vuln-app` / `-recon-app` / `-pentest-app` / `-agent-app` / `-platform-app`），主二进制只做组装 + Tauri 绑定，为「每个功能独立模块、类微服务」的北极星铺路。

**架构：** 沿用 `docs/design/2026-05-30-servitization-readiness.md` 的六边形 + 数据所有权 + 端口化方案。本计划把该设计的**阶段 3（S3-2 碎 god-crate）** 具体化为「按耦合 DAG 叶子→根」逐个抽 crate，并为每个抽取标注其**端口前置**（哪些跨服务耦合必须先用端口剪断，否则抽出来会成循环依赖）。

**技术栈：** Rust 2021 workspace（`backend/crates/`，45 crate）+ Tauri 2（`generate_handler!` 命令注册）+ sqlx/`golish-db` + `cargo nextest` + clippy + `scripts/check_repo_ownership.py`（数据所有权守卫）+ `scripts/check_dag.py`（层级守卫）。

---

## 0. 定位：本计划在整体路线里的位置

| 整体阶段（servitization-readiness §6） | 状态 | 本计划关系 |
|---|---|---|
| 阶段 0 稳定契约（错误码 I1 / ts-rs I5 / 作用域 SQL I2） | 进行中 | 前置，非本计划范围 |
| 阶段 1 数据所有权 + 端口化（S1-1 / S1-2） | S1-1 ✅、S1-2a ✅、S1-2b 设计完 | **本计划的"端口前置"直接消费这一阶段产物** |
| **阶段 3 碎 god-crate 为按域 app crate（S3-2）** | not_started | **← 本计划就是 S3-2 的可执行化** |
| 阶段 4 真抽第一个网络服务 | deferred | 本计划完成后，任一 app crate 可按阶段 4 升级为网络服务 |

> **已完成的"地基"（本计划不再重复）**：① 文件级模块化（`scripts/check_file_sizes.sh` 全绿，Rust ≤500 / TS ≤800）；② S1-1 数据所有权守卫（5 服务表归属 + CI 闸门）；③ S1-2a `VaultReadPort` 走路骨架（platform 读取侧已解耦）。

---

## 1. 核心洞察：抽取顺序由"耦合 DAG"决定，不能乱抽

### 1.1 五服务 + 当前跨服务耦合（证据：`scripts/check_repo_ownership.py`）

守卫把系统划成 5 个服务，每个拥有自己的 DB 表（`REPO_OWNER`，§该脚本 36-84 行）。`ALLOWLIST`（127-158 行）冻结了 **27 处跨服务直读**（一个服务的命令层直接读另一个服务的表）。按"谁读谁"归并出的服务依赖边：

```text
platform ──▶ agent      (audit.rs 读 agent_logs / search_logs)
platform ──▶ recon      (audit.rs 读 passive_scans)
agent    ──▶ recon      (ai/db_bridge/recon.rs 读 api_endpoints/fingerprints/js_analysis/passive_scans/target_assets)
agent    ──▶ vuln       (ai/db_bridge/recon.rs 读 vuln_intel；wiki.rs 读 wiki_kb)
agent    ──▶ pentest    (ai/db_bridge/orchestration.rs 读 execution_plans)
pentest  ──▶ recon      (pentest_bridge/* + security_analysis.rs + pipeline/storage.rs，共 16 处，读 targets/js_analysis/sitemap_store/...)
recon    ──▶ vuln       (tools/scan_queue.rs 读 scan_queue)
```

### 1.2 这是一个 DAG（无环），拓扑序为：

```text
platform → agent → pentest → recon → vuln
（所有边都指向右侧；vuln 是叶子，不依赖任何其它服务）
```

**结论（抽取顺序）**：从**叶子**往**根**抽，被抽出的 crate 只依赖"已经抽出"的下游，永不产生循环：

| 顺序 | 服务 | 出向耦合（它读别人） | 入向耦合（别人读它） | 抽取难度 |
|---|---|---|---|---|
| 1 | **vuln** | 0（只读自己 + scan_queue 属自己） | 3（agent×2、recon×1） | **最低（叶子）** |
| 2 | **recon** | 1（读 vuln 的 scan_queue） | **21**（被最多服务读） | 中（出向极少，可先抽 crate，再补 ReconPort 给上游） |
| 3 | **pentest** | 16（重度读 recon） | 1（agent 读 execution_plans） | 高（须先有 ReconPort，否则硬依赖 recon-app） |
| 4 | **agent** | 8（读 recon/vuln/pentest） | 2（platform 读 agent_logs/search_logs） | 高 |
| 5 | **platform** | 3（audit.rs 读 agent/recon） | 0（vault 端口已做完） | 中 |

### 1.3 两个层次的目标（务必让用户知情）

- **层次 A（crate 拆分）**：按上面 DAG 顺序抽成 5 个 app crate，crate 间用**编译期依赖链**（`pentest-app` → `recon-app` → `vuln-app`）。可在端口未全做完时就拿到"5 个独立 crate"的形态，但它们仍硬耦合成一条链 = **被切片的单体**，非真独立。
- **层次 B（真独立 / 类微服务）**：每个 app crate 只依赖**契约/端口 crate**（trait），不直接依赖兄弟 app crate。需要 S1-2 系列端口（`ReconPort` 等）落地。**这才是用户要的"像微服务"**。

> 本计划按"先 A 见效、再用端口逐步升 B"推进：每抽一个服务，**先用编译期依赖链让它跑起来**，再把"别人读它的表"那些入向耦合替换成它的端口，最终切断兄弟依赖。

---

## 2. 结构性前置（M0）：让 `generate_handler!` 能跨 crate 收集命令

### 2.1 问题（证据：`backend/crates/golish/src/commands_registry.rs:1-36`）

当前所有 `#[tauri::command]` 的 `__cmd__$name` 宏 `#[macro_export]` 到 **`golish` crate 根**，`tauri::generate_handler!` 在 `golish` 内统一引用。命令模块一旦搬到独立 crate，宏会导出到**那个 crate 的根**，`golish` 的 `generate_handler!` 看不到 → 注册断裂。**这是抽 app crate 的头号机械障碍，必须先解决。**

### 2.1.1 机制已核实（2026-05-30 web 调研，Tauri 官方 Discussion #5378 + docs.rs）

- **`Builder::invoke_handler` 全应用只能调一次**——多次调用后者覆盖前者。故"每个 crate 一个 `get_handlers()` 闭包再拼接"的路子走不通（`generate_handler!` 产出的闭包不可合并）。
- **正解 = 单个聚合 `generate_handler!` 按路径引用**：`generate_handler![golish_vuln_app::vuln_search, golish_recon_app::..., ...]`，被引用命令在其 crate 内 `pub` 即可。**这与 golish 现状（`commands_registry.rs` 单个大 `generate_handler!`）完全吻合**——app crate 只需把命令暴露成 `pub` 路径，golish 的聚合宏按路径列出。
- **不引第三方**（`tauri-helper` / `tauri-interop` 可自动收集，但增依赖，不采用）。
- **M0 spike 仍需实证**：本仓库的 Tauri 版本 + `#[macro_export]`-to-crate-root 现状下，路径式跨 crate 引用是否一次通过（若 `generate_handler!` 仍要求 `__cmd__` 宏在调用点 in-scope，则 app crate 需 `pub use` 或 golish 侧 `use golish_vuln_app::__cmd__vuln_search`）——这正是 M0 要用最小样本验证的点。

### 2.2 M0 任务（spike + 设计，先证明机制再批量搬）

- **文件（新建/修改）：**
  - 新建临时验证 crate `backend/crates/golish-vuln-app/`（仅含 1 个真实命令做样本，如把 `vuln_intel/commands/search.rs` 的 `vuln_search` 移入）
  - `backend/crates/golish/src/commands_registry.rs`（验证 `generate_handler![golish_vuln_app::vuln_search, ...]` 路径式引用是否可行）
  - `backend/Cargo.toml`（members + workspace.deps 加 `golish-vuln-app`）
- **步骤：**
  1. 读 Tauri 2 文档确认跨 crate `#[tauri::command]` 的注册写法（path 式 vs collect 式；Tauri 2 支持 `generate_handler![other_crate::cmd]`，需被调命令在其 crate 内 `pub`）。
  2. 把 `golish-vuln-app` 需要的共享类型（`DbState`、`AppState`、`GolishError`、`EventEmitter` 等）来源确认：要么这些 State 类型本就在可被依赖的下层 crate（`golish-core` / 新 `golish-app-core`），要么 M0 顺带把 State 定义下沉到一个 `golish-app-core` crate。**这是 M0 的第二个设计决策点。**
  3. 让样本命令在新 crate 编译通过 + 在 `golish` 的 `generate_handler!` 里注册通过 + 前端能 `invoke('vuln_search')`。
- **验证：** `cargo check -p golish-vuln-app && cargo check -p golish`；`just dev-fe` mock 下 invoke 样本命令返回正常；`bash scripts/check_dag.py`（新 crate 层级合法）。
- **回滚：** M0 是新增 crate + 1 命令搬移，未跑通前 `golish` 内旧命令保留；失败则删新 crate、命令搬回。
- **产出：** 一份《多 crate 命令注册机制》结论（写进本目录新 design 或 architecture.md），M1-M5 全部复用。

> **M0 是真正的下一步可执行项**：它把"能不能拆 crate"从假设变成已证机制。在 M0 跑通前，不要批量搬任何命令。

---

## 3. 里程碑序列（M1–M5：每服务一个，按 §1.2 叶子→根）

> M1 在下面给出**任务级骨架**；M2–M5 给出**范围 + 端口前置 + 解锁门槛**，各自在被选中时按 writing-plans 写细粒度子计划（writing-plans §范围检查：多子系统须各自独立计划）。

### M1 —— 抽 `golish-vuln-app`（叶子服务，proof of pattern）

- **为什么先抽它**：出向耦合 = 0（只碰自己的表 vuln_intel/vuln_scan/scan_queue/wiki_kb/kb_research），是 DAG 叶子；`golish-vuln-intel(-domain)` 契约 crate 已存在；与 servitization-readiness §5 "Vuln-Intel 最适合第一个抽" 一致。
- **搬迁内容（证据：本计划调研）：**
  - `golish/src/tools/vuln_intel/`（mod + commands/{search,matching,fetching,feeds,enrichment,shared,mod}）→ `golish-vuln-app/src/vuln_intel/`
  - `golish/src/tools/wiki/`（mod + pages/* + vuln_links/search/dashboard/kb_research）→ `golish-vuln-app/src/wiki/`
  - `golish/src/commands_facade/{vuln_intel,wiki}.rs` → 调整为 re-export 新 crate
- **端口前置：** vuln 的**出向**为 0 → **无端口前置**，可直接搬（这正是选它当第一刀的原因）。注意：`recon`/`agent` 仍读 vuln 的表（入向 3 处），M1 阶段让 `golish`/recon/agent 侧**编译期依赖 `golish-vuln-app`**（层次 A）即可；这 3 处入向耦合留到 M-后期补 `VulnReadPort` 时切断（层次 B）。
- **步骤：**
  1. M0 机制就绪后，建 `golish-vuln-app` crate（Cargo.toml 依赖：`golish-core`/`golish-db`/`golish-vuln-intel`/`golish-vuln-intel-domain`/`golish-app-core`(若 M0 引入)/`tauri`/`sqlx`...）。
  2. `git mv` 上述模块目录到新 crate；批量改 `use crate::tools::...` → `use crate::...` / `use golish_vuln_app::...`。
  3. `golish` 侧：`commands_facade/{vuln_intel,wiki}.rs` 改 `pub use golish_vuln_app::...`；`commands_registry.rs` 的 `use commands_facade::{vuln_intel,wiki}::*` 保持不变（facade 转发）。
  4. `RAW_SQL_ALLOWLIST` / `ALLOWLIST` 路径迁移：守卫扫的是 `golish/src`，搬走的文件路径从 allowlist 移除（搬到新 crate 后不再被该守卫覆盖——需在 M1 子计划里决定守卫是否扩展扫描 `golish-vuln-app/src`）。
- **验证：** `cargo check -p golish-vuln-app && cargo nextest run -p golish-vuln-app`；`cargo check -p golish`；`bash scripts/check_dag.py`；`bash scripts/check_repo_ownership.py`（确认无新违规、allowlist 同步）；`just precommit` 全绿；`just dev` 手测 vuln 面板 + wiki 功能正常。
- **回滚：** 纯结构性搬移，单 PR `git revert`；未跑通前 `golish` 内旧路径可短期并存（facade 双指）。

### M2 —— 抽 `golish-recon-app`（被依赖最多，端口收益最高）

- **端口前置（关键）：** recon 入向 = 21（pentest 16 + agent 5 + platform 1 各读 recon 表）。要让 recon 抽出后**上游不硬依赖 recon-app**（层次 B），须先落 **S1-2b `ReconPort`**（已设计：`docs/design/2026-05-30-s1-2b-recon-read-port.md`，22 条 allowlist、6 子片 b1–b6）。
- **两种推进：** ① 先按层次 A 抽 recon-app（让 pentest/agent/platform 编译期依赖它）快速见形态；② 再做 S1-2b 把 21 处入向改走 `ReconPort`，切断硬依赖升层次 B。
- **范围：** `tools/{asset_intel,organizations,targets,custom_rules,sensitive_scan,scan_runner,scan_queue,intel_providers,integrations}` + 相关 facade。
- **解锁门槛：** M1 完成 + M0 机制稳定；层次 B 还需 S1-2b 落地。**抽中时写 `2026-05-30-m2-recon-app.md` 子计划。**

### M3 —— 抽 `golish-pentest-app`

- **端口前置：** pentest 出向 = 16（重度读 recon）→ **必须先有 `ReconPort`（S1-2b）**，否则 pentest-app 硬依赖 recon-app。入向仅 1（agent 读 execution_plans）→ 后补 `PentestPlanReadPort`。
- **范围：** `tools/{pentest,pentest_ai,pentest_bridge,findings,methodology,pipeline,execution_plans,evidence,security_analysis}` + facade。
- **解锁门槛：** M2 + S1-2b 完成。子计划另写。

### M4 —— 抽 `golish-agent-app`

- **端口前置：** agent 出向 = 8（读 recon 6 + vuln 1 + pentest 1）→ 须 `ReconPort` + `VulnReadPort` + `PentestPlanReadPort` 就位。入向 2（platform 读 agent_logs/search_logs）→ 后补 `AgentLogReadPort`。
- **范围：** `ai/`（db_bridge/session_bridge/tracking_bridge…）+ `tools/conversation_store` + facade `ai`。注意 ai 层最厚、与 `golish-agent-*` crate 交织最深。
- **解锁门槛：** M2/M3 端口齐 + M0。子计划另写。

### M5 —— 抽 `golish-platform-app`

- **端口前置：** platform 出向 3（audit.rs 读 agent_logs/search_logs/passive_scans）→ 须 `AgentLogReadPort` + recon 的 passive_scans 端口。入向 0（vault 已端口化，S1-2a ✅）。
- **范围：** `tools/{vault,audit,notes,recordings}` + `ports/platform/*`（已存在）+ facade `vault`。
- **解锁门槛：** M4 端口齐。子计划另写。

---

## 4. 端口前置映射总表（"抽哪个 crate 之前必须先剪哪些线"）

| 抽取 | 必须先有的端口（切出向硬依赖） | 抽后待补的端口（切入向、升层次 B） | 当前端口状态 |
|---|---|---|---|
| M1 vuln | 无（叶子） | `VulnReadPort`（被 agent/recon 读） | 待建 |
| M2 recon | 无（出向只读 vuln scan_queue，归属待校正） | **`ReconPort`（S1-2b，21 处）** | **已设计** |
| M3 pentest | `ReconPort` | `PentestPlanReadPort`（被 agent 读） | 依赖 S1-2b |
| M4 agent | `ReconPort`+`VulnReadPort`+`PentestPlanReadPort` | `AgentLogReadPort`（被 platform 读） | 依赖 M1/M2/M3 |
| M5 platform | `AgentLogReadPort`+recon passive_scans 端口 | 无（入向已 0） | 依赖 M4 |

> 端口落地范式见 S1-2a 已跑通的走路骨架：`golish/src/ports/<service>/<port>.rs`(trait) + `Pg<Service>Adapter`(impl) + 构造点注入 + 守卫拔 ratchet（`check_repo_ownership.py` 的 `DOMAIN_RULES` 加 `ports/<service>`、删对应 `ALLOWLIST` 条目）。

---

## 5. 验证 & 回滚（全局）

- **每个里程碑**：相关 crate `cargo check` + `cargo nextest`；`bash scripts/check_dag.py`（层级无环）；`bash scripts/check_repo_ownership.py`（耦合不回升）；收尾 `just precommit` 全绿（AGENTS.md §3）。
- **每里程碑独立 commit/PR**；不跨里程碑混 diff（AGENTS.md §2.8）。
- **回滚原则**：每步都是「新增 crate + `git mv` + facade 转发」的结构性变更，未跑通前旧路径并存，单 PR 可 revert。
- **高风险点**（AGENTS.md §2.7 须用户确认）：新增 crate 改 `backend/Cargo.toml` members、命令注册机制变更（影响全量 IPC）。

---

## 6. 关键决策（2026-05-30 用户按推荐全部拍板）

| # | 决策 | 结论 |
|---|---|---|
| 1 | State 共享方式 | ✅ 引入 `golish-app-core` crate 收纳 `DbState`/`AppState`/`GolishError` 等共享类型，各 app crate 依赖它 |
| 2 | 守卫扫描范围 | ✅ `check_repo_ownership.py` 扩展扫描 `golish-*-app/src`，搬走的命令仍受耦合守卫覆盖 |
| 3 | 层次 A vs B 节奏 | ✅ A 先行（按编译期依赖链抽出 5 crate 见形态）→ 再逐个补 S1-2 端口升真独立 B |
| 4 | 复用 extract-golish-asset-intel-crate | ✅ 并入 M2 recon 作为子项 |

> 决策已定，M0 可开工。**M0 第一步硬约束**：先核实 Tauri 2 跨 crate `#[tauri::command]` 注册机制（承重假设），核实通过再建 crate / 改 Cargo.toml。

---

## 7. 自检（writing-plans §自检）

- **规格覆盖**：servitization §6 阶段 3（S3-2 碎 god-crate）→ 本文 M0-M5 全覆盖；阶段 4（网络服务）显式 deferred 到各 app crate 抽出后。✓
- **占位符扫描**：M2-M5 标注为"被选中时写子计划"，符合 writing-plans §范围检查（多子系统各自独立计划），非 TODO 占位。M0/M1 含具体文件 + 命令 + 验证。✓
- **类型/路径一致**：`golish-vuln-app` / `golish-app-core` / `ReconPort` 等命名跨章节一致；端口范式与 S1-2a 既有实现对齐。✓
- **顺序依赖**：M0（机制）→ M1（叶子 vuln）→ M2（recon+ReconPort）→ M3/M4/M5（依端口链），与 §1.2 DAG 拓扑序一致。✓
