# 后端超预算文件按域拆分 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 配套：`docs/superpowers/plans/2026-05-30-arch-health-backlog.md`（本计划是其 **P2 后端** 部分的详细展开）；姊妹计划 `docs/superpowers/plans/2026-05-30-frontend-oversized-component-split.md`（MCP-5 前端组件拆分）。
> 作者：MCP-3（backend_dev）。分工：本计划 后端 Rust 文件拆分 / MCP-5 前端组件拆分 / MCP-4 类型收敛(I5)。

**目标：** 把超过 500 行 Rust 预算（`scripts/check_file_sizes.sh`，`RUST_LIMIT=500`）的非测试业务文件按职责拆成小而专注的子模块，**行为零变更**，让 `arch-check.yml` 的 file-size gate 对后端转绿。
**架构：** 纯结构性重构——三种确定性套路：(A) 单个超大 `impl Trait for Type` → 把方法体下沉到按域分文件的 inherent `impl` 块，trait impl 退化为薄委托层；(B) 自由函数 / 类型聚合的模块 → 按职责把相关项原样搬到 sibling 子模块，`mod.rs` 用 `pub use` 维持对外路径不变；(C) 行内 `#[cfg(test)] mod tests` → 搬到 gate 豁免文件名（`tests.rs` / `*_tests.rs`）。模块声明（父 `mod foo;`）在 `foo.rs → foo/mod.rs` 转换后**无需改动**，对外 `use` 路径保持稳定。
**技术栈：** Rust 2021 + workspace（`backend/crates/` 50+ crate）+ sqlx + `cargo nextest` + clippy + `scripts/check_file_sizes.sh`。

---

## 背景与范围（证据）

`bash scripts/check_file_sizes.sh`（MCP-3 于 2026-05-30 实跑，exit=1）报告的 **Rust 超 500 行文件 = 28 个**（MCP-5 转述为「30 个」，以本次实跑为准，差异已对账）。完整清单（行数降序，来自本次实跑 stderr）：

| # | 行数 | 文件 | crate(`-p`) | 套路 |
|---|---|---|---|---|
| 1 | 998 | `golish-integrations/src/storage/external_file.rs` | golish-integrations | **C** |
| 2 | 960 | `golish-pipeline/src/engine/steps/single.rs` | golish-pipeline | **B+内部分解** |
| 3 | 888 | `golish-integrations/src/schema.rs` | golish-integrations | **B+C** |
| 4 | 766 | `golish/src/tools/organizations.rs` | golish | **B+C** |
| 5 | 747 | `golish/src/ai/db_bridge.rs` | golish | **A** |
| 6 | 739 | `golish-integrations/src/resolver.rs` | golish-integrations | **C** |
| 7 | 709 | `golish/src/ai/tracking_bridge.rs` | golish | **A** |
| 8 | 705 | `golish-pty/src/manager/session_create.rs` | golish-pty | **B+内部分解** |
| 9 | 679 | `golish-db/src/repo/audit.rs` | golish-db | **B+C** |
| 10 | 635 | `golish/src/tools/asset_intel/runtime/cli.rs` | golish | **B（需扫描）** |
| 11 | 620 | `golish-pty/src/manager/stdin_wait_detector.rs` | golish-pty | **C** |
| 12 | 611 | `golish-intel-providers/src/zone/mapper.rs` | golish-intel-providers | **C** |
| 13 | 590 | `golish-pentest/src/output_store/organizations.rs` | golish-pentest | **B+C** |
| 14 | 590 | `golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs` | golish-agent-runtime | **B（需扫描）** |
| 15 | 565 | `golish-pty/src/manager/core.rs` | golish-pty | **C** |
| 16 | 563 | `golish-agent-runtime/src/agentic_loop/tool_execution/direct.rs` | golish-agent-runtime | **B（需扫描）** |
| 17 | 558 | `golish-pipeline/src/engine/orchestrator.rs` | golish-pipeline | **B** |
| 18 | 557 | `golish-llm-providers/src/lib.rs` | golish-llm-providers | **C（lib→C2）** |
| 19 | 549 | `golish/src/tools/methodology.rs` | golish | **B** |
| 20 | 549 | `golish/src/tools/integrations/state.rs` | golish | **C** |
| 21 | 538 | `golish-pentest/src/evidence_sanitizer.rs` | golish-pentest | **C** |
| 22 | 531 | `golish-js-analyzer/src/lib.rs` | golish-js-analyzer | **C（lib→C2）** |
| 23 | 531 | `golish-agent-kit/src/planner/manager.rs` | golish-agent-kit | **B（需扫描）** |
| 24 | 527 | `golish/src/ai/commands/mod.rs` | golish | **需扫描** |
| 25 | 506 | `golish-agent-runtime/src/execution_mode/prompt_render.rs` | golish-agent-runtime | **C** |
| 26 | 504 | `golish-settings/src/schema/llm.rs` | golish-settings | **需扫描** |
| 27 | 504 | `golish-core/src/events/event.rs` | golish-core | **B（需扫描）** |
| 28 | 502 | `golish-agent-kit/src/tool_executors/graph.rs` | golish-agent-kit | **C（验证边界）** |

**关键发现（决定优先级）**：file-size gate 的 `find` 排除了文件名 `tests.rs` 与 `*_tests.rs`（脚本 `-not -name "tests.rs" -not -name "*_tests.rs"`），但**不排除行内 `#[cfg(test)] mod tests`**。多数大文件的体积来自行内测试模块。各文件「行内测试起点」与「去测试后真实代码行数」实测如下（`rg -n '#\[cfg\(test\)\]'`）：

| 文件 | 总行 | 行内 `#[cfg(test)]` 起点 | 去测试后真实代码≈ | 仅靠 C 是否转绿 |
|---|---|---|---|---|
| `storage/external_file.rs` | 998 | 483 | ~482 | ✅ 是 |
| `resolver.rs` | 739 | 212 | ~211 | ✅ 是 |
| `tools/integrations/state.rs` | 549 | 379 | ~378 | ✅ 是 |
| `manager/stdin_wait_detector.rs` | 620 | 314 | ~313 | ✅ 是 |
| `manager/core.rs`（pty） | 565 | 407 | ~406 | ✅ 是 |
| `zone/mapper.rs` | 611 | 304 | ~303 | ✅ 是 |
| `evidence_sanitizer.rs` | 538 | 310 | ~309 | ✅ 是 |
| `execution_mode/prompt_render.rs` | 506 | 359 | ~358 | ✅ 是 |
| `llm-providers/src/lib.rs` | 557 | 437 | ~436 | ✅ 是（C2） |
| `js-analyzer/src/lib.rs` | 531 | 313 | ~312 | ✅ 是（C2） |
| `tool_executors/graph.rs` | 502 | 78 | ~77 | ✅ 是（先核对边界） |
| `schema.rs` | 888 | 576 | ~575 | ❌ 仍需 B |
| `tools/organizations.rs` | 766 | 602 | ~601 | ❌ 仍需 B |
| `db/repo/audit.rs` | 679 | 480 & 628 | ~601 | ❌ 仍需 B |
| `output_store/organizations.rs` | 590 | 523 | ~522 | ❌ 仍需 B |

> **执行顺序原则**：先做 **Wave 1（纯 C，11 个文件，风险最低、ROI 最高）**，再 **Wave 2（A，trait-impl）**，再 **Wave 3（B/B+C）**，最后 **Wave 4（内部函数分解 + 需扫描文件，风险最高）**。每文件独立 commit，逐文件 `cargo check -p <crate>` + `cargo nextest run -p <crate>` + `bash scripts/check_file_sizes.sh` 三道验证。

---

## 套路 A — 拆分超大单个 trait impl（`impl Trait for Type`）

**约束（必读）**：Rust **不允许**把同一个 `impl Trait for Type` 物理拆成多个块——一个 trait 的所有方法必须在同一个 impl 块内。因此不能直接「把 trait impl 切成几片放不同文件」。正确做法是：把方法**体**搬到按域分文件的 **inherent impl** 块（inherent impl 可以多块、可跨文件），trait impl 退化成**薄委托层**。

**机械步骤（以 `db_bridge.rs` 为模板）：**

1. `git mv` 把 `foo.rs` 变成目录模块：`foo.rs → foo/mod.rs`。父模块的 `pub mod foo;`（本仓库 `ai/mod.rs:10/15` 已是 `pub mod db_bridge; / pub mod tracking_bridge;`）**无需改动**，对外类型路径（如 `ai::db_bridge::GolishDbRepoProvider`）保持稳定。
2. 在 `foo/mod.rs` 顶部声明私有子模块：`mod wiki; mod recon; mod tasks; mod orchestration; mod convert;`
3. 每个域子模块写 **inherent impl**，方法名加 `_impl` 后缀（消除 inherent/trait 同名解析歧义），方法体**逐字搬移**：

```rust
// foo/wiki.rs
use super::GolishDbRepoProvider;
use golish_db::models::{NewWikiPage, NewWikiChangelog};

impl GolishDbRepoProvider {
    pub(super) async fn wiki_upsert_page_impl(&self, page: &NewWikiPage) -> anyhow::Result<()> {
        // ← 把原 trait 方法体逐字搬到这里（不改一行逻辑）
    }
    // …该域其余方法同法，全部加 _impl 后缀…
}
```

4. `foo/mod.rs` 只保留：`use` + 结构体定义 + `impl GolishDbRepoProvider { pub fn new(...) }` + trait impl（薄委托，按域用注释分段）：

```rust
// foo/mod.rs
#[async_trait::async_trait]  // ← 若原 trait 用了 #[async_trait]，保留此属性；否则用原生 async fn
impl DbRepoProvider for GolishDbRepoProvider {
    // ── wiki ──────────────────────────────────────────
    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<()> {
        self.wiki_upsert_page_impl(page).await
    }
    // …每个 trait 方法一行委托到对应 _impl…
}
```

5. 文件底部的自由函数（如 `convert_*`）搬到 `foo/convert.rs`，签名加 `pub(super)`，在用到处 `use super::convert::*;` 或 `convert::convert_task_status(...)` 调用。

**为什么行为零变更**：委托层调用的是同一段代码，仅多一层函数调用（通常被内联）；inherent 方法解析优先级高于 trait 方法，但因加了 `_impl` 后缀，无任何同名歧义。`dyn Trait` 调用走 trait 方法（委托）→ inherent，语义不变。

**async-trait 注意**：本仓库 trait 方法在 impl 中以 `async fn ...(&self)` 直写。若该 trait 标注了 `#[async_trait]`，`mod.rs` 的 trait impl 块需保留 `#[async_trait::async_trait]`；子模块的 `_impl` inherent 方法用**原生** `async fn`（inherent impl 的原生 async fn 自 Rust 1.75 稳定，无需宏）。

---

## 套路 B — 按职责拆分自由函数 / 类型模块

1. `git mv foo.rs foo/mod.rs`。
2. 按职责建子模块（`foo/types.rs` `foo/validation.rs` …），把相关 `struct/enum/fn` **整段逐字**搬入；子模块顶部补 `use` 与 `use super::*;`（按需）。
3. **维持对外路径稳定**：凡是原来 `pub`、被外部以 `foo::Bar` 引用的项，在 `foo/mod.rs` 用 `pub use self::types::Bar;`（或 `pub use self::types::*;`）重导出，使 `foo::Bar` 仍可达。仅内部使用的项用 `pub(crate)` / `pub(super)`。
4. 跨子模块互相调用时按真实可见性补 `use`；`cargo check` 会精确报出缺失，逐条补齐。

**为什么行为零变更**：仅移动定义位置 + 重导出，无逻辑改写；可见性放宽（`pub use`）不改变运行语义。

---

## 套路 C — 抽出行内测试到预算豁免文件

gate 排除文件名 `tests.rs` 与 `*_tests.rs`。两种等价做法，二选一：

- **C1（目录模块，配合 B 一起做时用）**：`foo.rs → foo/mod.rs`，在 `mod.rs` 末尾加 `#[cfg(test)] mod tests;`，把原行内 `mod tests { … }` 的**花括号内部**搬到 `foo/tests.rs`，文件首行 `use super::*;`。
- **C2（`#[path]`，纯抽测试、不想建目录时用；lib.rs 必须用此法）**：保留 `foo.rs`，把行内 `#[cfg(test)] mod tests { … }` 替换为
  ```rust
  #[cfg(test)]
  #[path = "foo_tests.rs"]
  mod tests;
  ```
  把测试体搬到 sibling `foo_tests.rs`（命中 `*_tests.rs` 豁免），文件首行 `use super::*;`。

两法都让 tests 仍是原文件的**子模块**（`super::*` 仍能访问私有项），且把测试行数落到豁免文件名。

> **lib.rs 限制**：crate 根 `lib.rs` 不能改名为 `lib/mod.rs`（Cargo 约束），故 `golish-llm-providers/src/lib.rs`、`golish-js-analyzer/src/lib.rs` **必须用 C2**（`#[path = "lib_tests.rs"]`）。

---

## Task 0 — 基线

**文件：** 无（只读）
**步骤：**
```bash
bash scripts/check_file_sizes.sh; echo "exit=$?"        # 记录当前 28 个 Rust 违规
cargo --version && cargo nextest --version               # 确认工具链可用
```
**验证：** 输出含 28 行 `✗ Rust files over 500 lines`，与上表一致。
**提交：** 无。

---

# Wave 1 — 纯测试抽离（套路 C，11 个文件，各自独立 commit）

> 这一波**每个文件都是同一套机械操作**：定位行内 `#[cfg(test)] mod tests`，按 C2（或 lib 的 C2）搬到 `*_tests.rs`。下面给出 1 个完整样例 + 1 个 lib 样例，其余 9 个文件**完全同法**，仅文件名/行号不同（逐个列于表内，避免「类似任务 N」式占位）。

## Task 1.1 — `storage/external_file.rs`（998 → ~482）

**文件：**
- 改 `backend/crates/golish-integrations/src/storage/external_file.rs`
- 新建 `backend/crates/golish-integrations/src/storage/external_file_tests.rs`

**步骤：**
1. 打开 `external_file.rs`，定位第 483 行起的 `#[cfg(test)]\nmod tests {` 直到文件末尾的配对 `}`。
2. 把 `mod tests { … }` 花括号**内部**全部内容剪切到新文件 `external_file_tests.rs`，并在该文件首行加 `use super::*;`（若原 `mod tests` 内已有 `use super::*;` 则保留、不重复）。
3. 在 `external_file.rs` 原 483 行位置替换为：
```rust
#[cfg(test)]
#[path = "external_file_tests.rs"]
mod tests;
```

**验证：**
```bash
wc -l backend/crates/golish-integrations/src/storage/external_file.rs   # 预期 ~482
cargo check -p golish-integrations
cargo nextest run -p golish-integrations
bash scripts/check_file_sizes.sh    # external_file.rs 不再出现
```
**提交：** `refactor(integrations): extract external_file inline tests to sibling file`

## Task 1.2 — `golish-llm-providers/src/lib.rs`（557 → ~436，lib 用 C2）

**文件：**
- 改 `backend/crates/golish-llm-providers/src/lib.rs`
- 新建 `backend/crates/golish-llm-providers/src/lib_tests.rs`

**步骤：**
1. 定位 `lib.rs` 第 437 行起的 `#[cfg(test)] mod tests { … }`。
2. 花括号内部搬到 `lib_tests.rs`，首行 `use super::*;`。
3. 第 437 行替换为：
```rust
#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
```
**验证：**
```bash
cargo check -p golish-llm-providers && cargo nextest run -p golish-llm-providers
wc -l backend/crates/golish-llm-providers/src/lib.rs   # 预期 ~436
bash scripts/check_file_sizes.sh
```
**提交：** `refactor(llm-providers): move lib inline tests to lib_tests.rs`

## Task 1.3–1.11 — 其余 9 个纯 C 文件（逐个，同机械，独立 commit）

每个文件：在「`#[cfg(test)]` 起点」处把行内 `mod tests` 体搬到指定 `*_tests.rs`，原处替换为 `#[cfg(test)] #[path="…"] mod tests;`，然后 `cargo check -p <crate>` + `cargo nextest run -p <crate>` + gate。

| 文件 | `-p` crate | test 起点 | 新测试文件 | 预期残余行 | commit message |
|---|---|---|---|---|---|
| `golish-integrations/src/resolver.rs` | golish-integrations | 212 | `resolver_tests.rs` | ~211 | `refactor(integrations): extract resolver inline tests` |
| `golish/src/tools/integrations/state.rs` | golish | 379 | `state_tests.rs` | ~378 | `refactor(tools): extract integrations/state inline tests` |
| `golish-pty/src/manager/stdin_wait_detector.rs` | golish-pty | 314 | `stdin_wait_detector_tests.rs` | ~313 | `refactor(pty): extract stdin_wait_detector inline tests` |
| `golish-pty/src/manager/core.rs` | golish-pty | 407 | `core_tests.rs` | ~406 | `refactor(pty): extract manager/core inline tests` |
| `golish-intel-providers/src/zone/mapper.rs` | golish-intel-providers | 304 | `mapper_tests.rs` | ~303 | `refactor(intel): extract zone/mapper inline tests` |
| `golish-pentest/src/evidence_sanitizer.rs` | golish-pentest | 310 | `evidence_sanitizer_tests.rs` | ~309 | `refactor(pentest): extract evidence_sanitizer inline tests` |
| `golish-agent-runtime/src/execution_mode/prompt_render.rs` | golish-agent-runtime | 359 | `prompt_render_tests.rs` | ~358 | `refactor(runtime): extract prompt_render inline tests` |
| `golish-js-analyzer/src/lib.rs` | golish-js-analyzer | 313 | `lib_tests.rs`（C2） | ~312 | `refactor(js-analyzer): move lib inline tests to lib_tests.rs` |
| `golish-agent-kit/src/tool_executors/graph.rs` | golish-agent-kit | 78 | `graph_tests.rs` | ~77 | `refactor(agent-kit): extract tool_executors/graph inline tests` |

> **graph.rs 特别注意**：test 起点在第 78 行、文件 502 行，说明真实代码极少（~77 行）。先用 `Read` 核对 78 行确实是 `#[cfg(test)] mod tests {` 的开头（而非测试专用 `use`）；若 502 行内有**多个** `#[cfg(test)]` 段或测试夹在代码中间，按 audit.rs 的多段合并法（见 Task 3.4）处理。

---

# Wave 2 — 超大 trait impl 拆分（套路 A）

## Task 2.1 — `ai/db_bridge.rs`（747，单个 `impl DbRepoProvider`）

**文件：**
- `git mv backend/crates/golish/src/ai/db_bridge.rs backend/crates/golish/src/ai/db_bridge/mod.rs`
- 新建 `db_bridge/{wiki,recon,tasks,orchestration,convert}.rs`

**已核实的方法→域映射**（行号来自 `db_bridge.rs` 实测；`impl DbRepoProvider for GolishDbRepoProvider` 跨 24–646）：

| 子模块 | 方法（trait 方法名） | 原行段 |
|---|---|---|
| `wiki.rs` | `wiki_upsert_page` `wiki_link_cve` `wiki_delete_refs_from` `wiki_upsert_page_ref` `wiki_add_changelog` `wiki_search_fts` `wiki_search_by_category` `wiki_search_by_tag` `wiki_list_cves_with_pocs` `wiki_list_unresearched_cves` `wiki_poc_stats` `wiki_upsert_poc_full` | 26–131 |
| `recon.rs` | `vuln_intel_search` `audit_log_operation` `api_endpoints_insert` `js_analysis_insert` `js_analysis_update_file_path` `fingerprints_upsert` `passive_scans_insert` `query_target_data` | 132–346 |
| `tasks.rs` | `task_create` `task_get` `task_update_status` `task_set_result` `subtask_create` `subtask_update_status` `subtask_set_result` `subtask_next_pending` `subtask_list_by_task` `subtask_delete_pending` | 347–475 |
| `orchestration.rs` | `message_chain_create` `message_chain_update_chain` `message_chain_update_usage` `plan_list_active` `plan_update_steps` `plan_create` `dispatch_record_start` `dispatch_record_finish` `dispatch_list_running` | 476–646 |
| `convert.rs` | 自由函数 `convert_dispatch_status_back` `convert_task_status` `convert_task_status_back` `convert_subtask_status` `convert_subtask_status_back` `convert_agent_type` `convert_agent_type_back` `convert_plan_status` `convert_plan_status_back` | 647–746 |

**步骤：**
1. `git mv` 成目录模块（见上）。
2. 按套路 A 步骤 3：每个域子模块写 `impl GolishDbRepoProvider { pub(super) async fn <name>_impl(&self, …) { <原体逐字> } }`，子模块顶部补 `use super::GolishDbRepoProvider;` + 该域用到的 `golish_db::models::*` / `uuid::Uuid` / `serde_json` 等 `use`（参照原文件顶部 import）。
3. `convert.rs`：把 9 个 `convert_*` 自由函数原样搬入，前缀 `pub(super)`。
4. `mod.rs` 保留：原文件顶部 import（去掉只被搬走方法用到的）+ `pub struct GolishDbRepoProvider { … }`（13–16）+ `impl GolishDbRepoProvider { pub fn new(pool) {…} }`（17–22）+ `mod wiki; mod recon; mod tasks; mod orchestration; mod convert;` + `impl DbRepoProvider for GolishDbRepoProvider { …全部方法一行委托到 *_impl…，convert 处调 convert::convert_* }`。
5. 委托层按域用 `// ── wiki ──` 注释分段，便于审阅。

**验证：**
```bash
cargo check -p golish
cargo nextest run -p golish
wc -l backend/crates/golish/src/ai/db_bridge/*.rs    # 各文件均 < 500，mod.rs ~ struct+new+委托(~120)
bash scripts/check_file_sizes.sh                      # db_bridge 不再出现
cargo clippy -p golish --no-deps                      # 零 warning（just lint-rust 子集）
```
**提交：** `refactor(ai): split db_bridge DbRepoProvider impl by domain (wiki/recon/tasks/orchestration)`

## Task 2.2 — `ai/tracking_bridge.rs`（709）

**文件：**
- `git mv … tracking_bridge.rs … tracking_bridge/mod.rs`
- 新建 `tracking_bridge/{records,memory,chain,rows,ready_gate}.rs`

**已核实的类型/方法→域映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `records.rs` | `impl DbTrackingBackend` 的记录类方法 → inherent `_impl`：`record_tool_call_start` `record_tool_call_finish` `record_token_usage` `record_terminal_output` `record_search_log` `record_audit` `record_agent_call` `record_msg_log` `record_vecstore_op` | 28–221 |
| `memory.rs` | 记忆/计划类方法 → inherent `_impl`：`store_memory` `store_memory_with_tool` `search_memories_text` `search_memories_semantic` `search_memories_by_doc_type` `search_memories_text_with_category` `search_memories_semantic_with_category` `fetch_memories_by_keyword` `fetch_active_plans` `list_recent_memories` `ensure_session` `load_prompt_template_overrides` | 222–549 |
| `chain.rs` | 整段搬移：`pub struct PgChainPersistence` + `impl PgChainPersistence { new }` + `impl SubAgentChainPersistence for PgChainPersistence { chain_create/chain_update/chain_update_usage/load_prompt_template_overrides }` | 551–635 |
| `rows.rs` | 整段搬移：`struct PgMemoryHitRow` + `From` + `struct PgScoredRow` + `struct PgBriefingPlanRow` + `From` | 636–692 |
| `ready_gate.rs` | 整段搬移：`pub struct CoreDbReadyGate(...)` + `impl DbReadinessGate for CoreDbReadyGate { is_ready/is_failed/wait/clone_box }` | 693–末 |

**步骤：**
1. `git mv` 成目录模块。
2. `records.rs` / `memory.rs`：套路 A（inherent `_impl` + `mod.rs` 委托）。
3. `chain.rs` / `rows.rs` / `ready_gate.rs`：这些是**独立类型**（非 `PgTrackingBackend` 的方法），按套路 B **整段搬移**，无需委托；`mod.rs` 对外 `pub` 的 `PgChainPersistence`、`CoreDbReadyGate` 用 `pub use self::chain::PgChainPersistence; pub use self::ready_gate::CoreDbReadyGate;` 重导出（保持 `ai::tracking_bridge::PgChainPersistence` 路径稳定）。`rows.rs` 内部类型用 `pub(super)`。
4. `mod.rs` 保留：import + `pub struct PgTrackingBackend{…}`（16–19）+ `impl PgTrackingBackend { pub fn new }`（20–26）+ 子模块声明 + `impl DbTrackingBackend for PgTrackingBackend { …委托… }` + 上述 `pub use`。

**验证：** 同 Task 2.1（`-p golish`）。`bash scripts/check_file_sizes.sh` 中 tracking_bridge 不再出现。
**提交：** `refactor(ai): split tracking_bridge by domain (records/memory/chain/rows/ready_gate)`

---

# Wave 3 — 按职责拆分（套路 B / B+C）

## Task 3.1 — `integrations/schema.rs`（888 → B+C）

**文件：** `git mv … schema.rs … schema/mod.rs`，新建 `schema/{storage,test_kind,capture,tests}.rs`

**已核实的类型→域映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `mod.rs`（core） | `IntegrationSchema` `IntegrationGroup` `Field` `FieldType`(+impl) `SelectOption` | 55–218 |
| `storage.rs` | `Storage`(+impl) `VaultStorage` `ExternalFileStorage` `ExternalFileFormat` `SettingsStorage` + `default_yaml_format` + `default_true` | 219–303 |
| `test_kind.rs` | `TestKind` + `default_timeout_30` + `default_ok_range` | 304–362 |
| `capture.rs` | `CaptureRecipe` + `default_capture_timeout` + `CaptureRule`(+impl) + `default_true_capture` `default_cookie_sep` `default_cookie_fmt` `default_wait_ms` | 363–575 |
| `tests.rs` | 行内 `mod tests` 体（C1） | 576–888 |

**步骤：** 套路 B 整段搬移 + 套路 C1 抽测试。`mod.rs` 用 `pub use self::storage::*; pub use self::test_kind::*; pub use self::capture::*;`（这些类型多为外部可见的 schema 模型，全部重导出以保 `schema::Storage` 等路径稳定）。子模块顶部按需 `use serde::{Deserialize, Serialize};` 等（照搬原顶部 import）。
**验证：** `cargo check -p golish-integrations && cargo nextest run -p golish-integrations && bash scripts/check_file_sizes.sh`。各子文件 < 500。
**提交：** `refactor(integrations): split schema into core/storage/test_kind/capture modules`

## Task 3.2 — `tools/organizations.rs`（766 → B+C）

**文件：** `git mv … organizations.rs … organizations/mod.rs`，新建 `organizations/{types,candidates,validation,tests}.rs`

**已核实映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `types.rs` | `Organization` `OrganizationCandidateKind` `OrganizationCandidate` `OrganizationCandidates` `OrganizationProfilePatch` + `impl From<OrganizationProfilePatch> for ProfilePatch` | 32–98, 333–385 |
| `candidates.rs` | `normalize_candidate` `read_candidates_from_intel` `upsert_candidates_into_intel` `upsert_organization_candidates_for_org` | 136–221 |
| `validation.rs` | `domain_regex` `asn_regex` `is_valid_cidr` `is_valid_domain` `is_valid_asn` `iter_strings` `validate_profile_patch` | 386–553 |
| `mod.rs`（commands） | `to_org` `now_millis` + `organization_list/get/create/update/move/delete/update_profile/candidates_list/candidates_upsert` | 99–135, 222–332, 554–601 |
| `tests.rs` | 行内 `mod tests` 体（C1） | 602–766 |

**步骤：** 套路 B + C1。`types.rs` 的 5 个类型多为 ts-rs/对外类型 → `mod.rs` 全部 `pub use self::types::*;`。`candidates.rs` / `validation.rs` 的自由函数用 `pub(super)`（仅 crate 内/模块内调用）；`upsert_organization_candidates_for_org` 原为 `pub(crate)` → 在 `mod.rs` 加 `pub(crate) use self::candidates::upsert_organization_candidates_for_org;` 保持 crate 内路径。
**验证：** `cargo check -p golish && cargo nextest run -p golish && bash scripts/check_file_sizes.sh`。
**提交：** `refactor(tools): split organizations into types/candidates/validation modules`

## Task 3.3 — `tools/methodology.rs`（549 → B，无行内测试）

**文件：** `git mv … methodology.rs … methodology/mod.rs`，新建 `methodology/{types,templates}.rs`

**已核实映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `types.rs` | `MethodologyTemplate` `Phase` `CheckItem` `ProjectMethodology` | 10–45 |
| `templates.rs` | `built_in_templates`（~350 行硬编码模板数据） + `check` 辅助 | 55–417 |
| `mod.rs`（commands） | `templates_dir` + `method_list_templates/start_project/list_projects/load_project/update_item/delete_project` | 46–54, 418–549 |

**步骤：** 套路 B。`types.rs` 4 个类型 `pub use self::types::*;`。`templates.rs`：`built_in_templates` 设 `pub(super)`，`check` 设 `fn`(私有)。`mod.rs` 命令函数保持原 `pub`/`pub async`（它们是 Tauri command 的实现，路径不能变）。
**验证：** `cargo check -p golish && cargo nextest run -p golish && bash scripts/check_file_sizes.sh`。
**提交：** `refactor(tools): split methodology built-in templates into submodule`

## Task 3.4 — `db/repo/audit.rs`（679 → B+C，⚠ golish-db crate）

> **⚠ 风险提示（I10 / AGENTS.md §2.7）**：本任务在 `golish-db` crate 内操作，但**仅为文件内代码重组，无任何 schema/migration/SQL 语义变更**——不属于「改 schema」的高风险类别。仍建议改完请 `code_audit` 角色复审一遍，确认无 SQL 字符串被误改。

**文件：** `git mv … repo/audit.rs … repo/audit/mod.rs`，新建 `audit/{pentest,queries,timeline,tests}.rs`

**已核实映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `mod.rs` | `reclaim_abandoned_audits` `reclaim_cutoff` `log` `log_operation` `log_operation_with_lineage` + 子模块声明 + 重导出 | 25–160 |
| `pentest.rs` | `pub struct PentestAudit` + `impl PentestAudit { started/completed/failed }` + `ensure_schema_v` + `lookup_run_id` + `impl PentestAudit { lookup_parent_by_detail_kv }` | 161–323 |
| `queries.rs` | `list` `list_by_category` `list_by_target` `list_by_session` `search` `count` `clear` + `build_list_by_project_exact_sql` `build_clear_by_project_exact_sql` `list_by_project_exact` `clear_by_project_exact` | 324–479 |
| `timeline.rs` | `pub struct TimelineEntry` + `target_timeline` | 510–627 |
| `tests.rs` | 合并两个行内测试段 `sql_tests`(481–509) + `reclaim_tests`(629–679)（C1，合到一个 `tests.rs`，内部可保留两个 `mod sql_tests {} mod reclaim_tests {}` 子模块） | 481–509, 629–679 |

**步骤：** 套路 B + C1。注意 `audit.rs` 有**两段**行内测试且中间夹着业务代码（`TimelineEntry`/`target_timeline` 在 510–627，位于两测试段之间）——搬移时按上表行段精确剪切，不要漏掉 510–627 这段业务代码。`PentestAudit`、`TimelineEntry` 为对外类型 → `mod.rs` `pub use self::pentest::PentestAudit; pub use self::timeline::TimelineEntry;`；查询函数原 `pub` → 逐个 `pub use self::queries::{list, list_by_category, …};`。
**验证：** `cargo check -p golish-db && cargo nextest run -p golish-db && bash scripts/check_file_sizes.sh && cargo clippy -p golish-db --no-deps`。
**提交：** `refactor(db): split repo/audit into pentest/queries/timeline modules`

## Task 3.5 — `pipeline/engine/orchestrator.rs`（558 → B，无行内测试）

**文件：** `git mv … orchestrator.rs … orchestrator/mod.rs`，新建 `orchestrator/run.rs`

**已核实映射：**

| 子模块 | 内容 | 原行段 |
|---|---|---|
| `mod.rs` | `pub struct PipelineRunner<'a>` + `impl PipelineRunner { new/with_emitter/with_optional_emitter/with_parent_audit_id/with_ai_tools/with_optional_ai_tools }` + 3 个 `execute_pipeline_headless*` 包装函数 + `mod run;` + `pub(super) use run::execute_pipeline_inner;`（按需） | 37–172 |
| `run.rs` | `pub(super) async fn execute_pipeline_inner`（~385 行核心编排） | 173–558 |

**步骤：** 套路 B 整段搬移。`run.rs` 顶部 `use super::PipelineRunner;` + 照搬原顶部 import 中 `execute_pipeline_inner` 用到的部分。`execute_pipeline_inner` 保持 `pub(super)` 可见性（它被 `mod.rs` 的包装函数调用）。
**验证：** `cargo check -p golish-pipeline && cargo nextest run -p golish-pipeline && bash scripts/check_file_sizes.sh`。
**提交：** `refactor(pipeline): move execute_pipeline_inner into orchestrator/run.rs`

## Task 3.6 — `pentest/output_store/organizations.rs`（590 → B+C，需先扫描）

**文件：** `git mv … organizations.rs … organizations/mod.rs`（+ 子模块按扫描结果定）

**步骤：**
1. 先扫描结构：
```bash
rg -n '^\s*(pub(\([^)]*\))? )?(async )?(fn|impl|struct|enum|trait) ' \
  backend/crates/golish-pentest/src/output_store/organizations.rs
```
2. 已知行内测试在 523 行（C1 抽到 `tests.rs`，省 ~67 行 → 真实 ~522，仍超 500 ~22 行）。按扫描出的职责分组（预期：类型定义 / 读写函数 / 转换辅助）抽 1–2 个子模块即可达标。
3. 套路 B + C1，`mod.rs` 对原 `pub` 项 `pub use` 重导出。
**验证：** `cargo check -p golish-pentest && cargo nextest run -p golish-pentest && bash scripts/check_file_sizes.sh`。
**提交：** `refactor(pentest): split output_store/organizations by responsibility`

---

# Wave 4 — 内部函数分解 + 需扫描文件（风险最高，逐个谨慎）

> 这一波的文件**没有可一键搬走的测试/类型大块**，体积来自**单个超大函数**或需先扫描确认。原则：**纯提取**（extract function/method），把原函数体内的连续语句段原样移入新的私有 helper，通过参数/返回值传递局部变量，**不改控制流、不改语义**；每提取一个 helper 就 `cargo check` 一次。

## Task 4.1 — `pipeline/engine/steps/single.rs`（960，4 个函数）

**已核实结构**（无行内测试）：`run_single_step`（17–600，~583 行，`pub(in super::super)`）、`run_ai_tool_step`（601–872，~271）、`ai_tool_failure`（873–947）、`ai_tool_stored_count`（948–960）。

**步骤（两阶段，各独立 commit）：**
- **4.1a**：`git mv single.rs single/mod.rs`，新建 `single/ai_tool.rs`，把 `run_ai_tool_step` + `ai_tool_failure` + `ai_tool_stored_count` 整段搬入（套路 B），可见性保持 `pub(super)`/私有；`mod.rs` `use self::ai_tool::run_ai_tool_step;`（`run_single_step` 内对其调用不变）。验证后此时 `mod.rs` 仅剩 `run_single_step` ~583 行，**仍超 500**。
  - commit：`refactor(pipeline): move ai_tool step helpers into single/ai_tool.rs`
- **4.1b**：分解 `run_single_step`。先 `Read` 17–600 全文，按其内部阶段（实测开头：emit "running" 事件 → `sub_pipeline`/`foreach` 分派 → 解析 tool 命令 → 运行 → 解析输出 → 存储 → 收尾）找 3–4 个**连续语句块**，各提取为 `single/exec.rs` 中的私有 `async fn`（如 `resolve_and_run(...)`、`parse_and_store(...)`），把块内引用的局部变量作为参数传入、把后续需要的值作为返回值传出。每提取一个就 `cargo check -p golish-pipeline`。目标每个 helper < 200 行、`run_single_step` 收口为「按阶段调用 helper」的骨架 < 200 行。
  - commit：`refactor(pipeline): decompose run_single_step into phase helpers`

**验证（4.1b 末）：**
```bash
cargo check -p golish-pipeline && cargo nextest run -p golish-pipeline
wc -l backend/crates/golish-pipeline/src/engine/steps/single/*.rs   # 均 < 500
bash scripts/check_file_sizes.sh
```

## Task 4.2 — `pty/manager/session_create.rs`（705，无行内测试）

**已核实结构**：`hex_encode`（20–49）、`dispatch_parsed_events`（50–108，`pub(super)`）、`impl PtyManager { create_session_internal }`（109–705，其中方法体 114–705 ~591 行）。

**步骤（两阶段）：**
- **4.2a**：`git mv session_create.rs session_create/mod.rs`，新建 `session_create/util.rs`，把 `hex_encode` + `dispatch_parsed_events` 搬入（保持 `pub(super)`）；`mod.rs` `use self::util::{hex_encode, dispatch_parsed_events};`。验证。commit：`refactor(pty): move session_create helpers into util.rs`
- **4.2b**：分解 `create_session_internal`（~591 行）。`Read` 114–705，按阶段（实测语义：构造 PTY/命令 → spawn 子进程 → 装配 emitter/解析器 → 注册 session 到 manager → spawn reader 循环 → spawn wait-detector）提取为 `impl PtyManager` 的私有方法（放 `session_create/spawn.rs`，inherent impl 可跨文件）。每提取一个 `cargo check -p golish-pty`。commit：`refactor(pty): decompose create_session_internal into stage methods`
**验证：** `cargo check -p golish-pty && cargo nextest run -p golish-pty && bash scripts/check_file_sizes.sh`。

## Task 4.3 — 需扫描文件（先 `rg` 结构，再判套路）

下列文件未在前面波次确证结构，**执行前必须先扫描**再决定 A/B/C。命令模板：
```bash
rg -n '^\s*(pub(\([^)]*\))? )?(async )?(fn|impl|struct|enum|trait) ' <file>
rg -n '#\[cfg\(test\)\]' <file>
```
判定规则：① 有行内大测试且去测试后 <500 → 套路 C；② 单个超大 `impl Trait` → 套路 A；③ 多类型/多自由函数 → 套路 B；④ 单个超大函数 → 内部分解（同 4.1/4.2）。

| 文件 | `-p` crate | 行数 | 预判 | commit 前缀 |
|---|---|---|---|---|
| `golish/src/tools/asset_intel/runtime/cli.rs` | golish | 635 | 无测试 → B 或内部分解 | `refactor(tools): split asset_intel runtime/cli` |
| `golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs` | golish-agent-runtime | 590 | 已是 mod.rs → 抽子模块 B | `refactor(runtime): split stream_processor` |
| `golish-agent-runtime/src/agentic_loop/tool_execution/direct.rs` | golish-agent-runtime | 563 | 无测试 → B/内部分解 | `refactor(runtime): split tool_execution/direct` |
| `golish-agent-kit/src/planner/manager.rs` | golish-agent-kit | 531 | 无测试 → B | `refactor(agent-kit): split planner/manager` |
| `golish/src/ai/commands/mod.rs` | golish | 527 | 已是 mod.rs → 抽子模块 B | `refactor(ai): split ai/commands` |
| `golish-settings/src/schema/llm.rs` | golish-settings | 504 | 多为配置类型 → B | `refactor(settings): split schema/llm` |
| `golish-core/src/events/event.rs` | golish-core | 504 | 多为事件类型/enum → B（若单一大 enum 不可拆则记 blocked 并在 backlog 说明） | `refactor(core): split events/event` |

> **event.rs 边界情形**：若 `event.rs` 是**一个**巨型 `enum Event { … }`（变体无法跨文件拆），则不能用套路 B 强拆。此时记为 `blocked`，在 backlog 写明「单一大 enum，需先与 code_review 讨论是否按域拆成多个 enum + 顶层聚合」，**不要**为了过 gate 做破坏类型语义的强拆（违 I5/I6 精神）。

---

## 全局验证与收尾

每个 Task 完成后（逐文件）：
```bash
cargo check -p <crate>
cargo nextest run -p <crate>
cargo clippy -p <crate> --no-deps      # 零 warning
bash scripts/check_file_sizes.sh       # 该文件不再出现
git add -A && git commit -m "<上表 message>"
```

全部 Wave 完成后（一次）：
```bash
bash scripts/check_file_sizes.sh; echo "exit=$?"   # 期望 Rust 段 0 violation（前端另见 MCP-5 计划）
just precommit                                       # fmt + check-fe + test-fe + lint-rust + test-rust-all 全绿
```

> **commit 纪律**：每文件一个独立 commit，commit 前 `git status` 确认未混入跨文件改动；不「顺手优化」无关代码（AGENTS.md §3.4）。Wave 间互不依赖，可按 crate 并行推进，但**同一 crate 内的文件建议串行**（避免 import 冲突）。

---

## 自检（对照 writing-plans）

1. **范围覆盖度：** gate 报告的 28 个 Rust 文件 → 全部在「背景表」列出并归入 Wave 1–4，每个都有 `-p` crate、套路、commit message。✓
2. **占位符扫描：** 无 TODO/待定；各 Task 给真实路径、实测行段、真实验证命令。「逐字搬移原体」是合法重构指令（非占位），与姊妹前端计划同约定。需扫描文件（Task 4.3）给出了**确定性扫描命令 + 判定规则**而非「后续实现」式占位。✓
3. **类型一致性：** 套路 A 的 `_impl` 后缀在 db_bridge/tracking_bridge 一致使用；`pub use` 重导出在所有套路 B 任务一致表述；委托方法名与原 trait 方法名一一对应（来自实测方法清单）。✓
4. **行为保持：** 全程「移动 + 重导出/委托」，无逻辑改写；四道闸守住：`cargo check` + `cargo nextest`（含搬移后的原测试）+ `clippy` + file-size gate。trait-impl 套路特别核对了「不可物理拆分」约束与 inherent/trait 解析优先级。✓
5. **风险点已标注：** golish-db（Task 3.4，非 migration 但建议 code_audit 复审）、event.rs 单一大 enum 边界情形（Task 4.3，可能 blocked）、两个 lib.rs 必须用 C2。✓
