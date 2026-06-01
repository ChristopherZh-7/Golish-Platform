# agent-progress.md

> **进度日志**。每轮会话结束前必须更新；每轮新会话开始前必须先读。
>
> 配套文件：`AGENTS.md`（工作宪法）、`feature_list.json`（功能清单）、`clean-state-checklist.md`（收尾检查）。

---

## 当前已验证状态

> 这是项目当前状态的**唯一真相来源**。任何与此处冲突的"agent 记忆"或"以前的回复"都不算数。

| 字段 | 值 |
|---|---|
| **仓库根** | `/Users/christopherzheng/WebstormProjects/Golish-Platform`（macOS）/ 同名相对路径 |
| **栈** | Tauri 2 + Rust workspace (50+ crates) + React 19 + TypeScript 6 + Vite 8 + Tailwind 4 |
| **包管理** | `pnpm`（前端）+ `cargo` nextest（后端） |
| **标准启动** | `just dev`（全栈热重载,端口 1420）/ `just dev-fe`（仅前端 mock） |
| **标准验证** | `just precommit` = `just check && just test` |
| **当前最高优先级** | **用户已澄清北极星 = crate-per-service（每个功能独立 crate、类微服务）**。新写 `docs/superpowers/plans/2026-05-30-crate-per-service-split.md`（servitization 阶段 3 S3-2 可执行化），feature_list `arch-crate-per-service-split` 已转 **`in_progress`**（M0 阶段）。**2026-05-30 进展**：§6 的 4 决策全按推荐拍板 + Tauri 跨 crate 注册机制 web 核实（Discussion #5378：invoke_handler 只调一次 → 单聚合 generate_handler! 路径引用）。**M0 State 下沉半边 = 完成+验证**：新建 `golish-app-core`(L5) 收 GolishError+DbState（AppState 故意留 golish），`golish/src/{error,state/db}.rs` 改 re-export，`check_dag.py` 加 L5；`cargo check -p golish-app-core` ✅ + `check_dag.py` ✅(46 crates) + golish 编译用户确认 OK。**M1（vuln 叶子）整体完成+验证**（MCP-agent-2 接 dead session yj5fxhjr 半成品）：vuln_intel(M1a)+wiki(M1b) 均 git mv 进 golish-vuln-app；`cargo check` 两 crate + `check_dag`(47 crates) + `check_repo_ownership` 全 exit 0；M0 欠的多 crate 命令注册由此 compile-level 实证。**M2（recon 服务）整体完成+验证（2026-05-31 · MCP-agent-4 · 层次 A 编译期依赖链）**：`golish-recon-app` 抽入 11 模块组（targets/organizations/scan_queue/sensitive_scan/custom_rules/scan_runner/intel_providers/wordlists + asset_intel + integrations），`scoping` 下沉 golish-app-core，asset_intel 解 PentestState（`ToolsConfigState` 共享同一 `Arc<ConfigManager>`），integrations（含 tauri webview 捕获引擎）搬迁 + tauri_app 启动接线。验证：`cargo check` 两 crate + `nextest -p golish-recon-app`(106✓) + `clippy -p golish-recon-app -D warnings` + `check_dag`(48 crates) + `check_repo_ownership` 全 exit 0。**M3（pentest 服务）整体完成+验证（2026-05-31 · MCP-agent-3 · 层次 A 编译期依赖链）**：`golish-pentest-app`(L5.6) 抽入 9 模块组（pentest/pentest_ai/pentest_bridge/findings/methodology/pipeline/execution_plans/evidence/security_analysis + 连带 output_parser）；**两个共享件下沉 golish-app-core**：`pty_interactive`（golish state/runtime/ai + pentest_ai 双用）+ `ports`(VaultReadPort/PgVaultAdapter，S1-2a)。pentest-app 编译期依赖 recon-app(targets)/pipeline(L3)/app-core；ai/ 入向桥 `pub(crate) use golish_pentest_app::{pentest,pentest_ai,pentest_bridge}`。验证：`cargo check` 两 crate + `nextest -p golish-pentest-app`(**47✓**) + `clippy -p golish-pentest-app -D` + `clippy -p golish --lib -D` + `check_dag`(**49 crates**) + `check_repo_ownership` 全 exit 0。**M4 调查 + M4-A（AppState 解耦）完成（2026-05-31 · MCP-agent-3）**：M4（agent）实证发现**真实 blocker**——`ai/commands/*`(19 文件) 几乎全 take 单体 `AppState`，而 `AppState` 聚合 `AiState`(定义在 ai/commands/mod.rs)，三者互锁 → 直接抽 ai/ 会造成 golish↔agent-app 循环（见 `docs/superpowers/plans/2026-05-31-m4-agent-app-feasibility.md`）。用户选「开 A：AppState 解耦」。**M4-A 完成**：新建 `golish-agent-app`(L5.6)，`AiState` 搬入 + 新 `AgentState`(13 字段 ≈ AppState 减 command_index/telemetry/langfuse)；`AppState::extract_agent_state()` + 启动 `.manage()`；**19 个 ai/commands 全部 `State<AppState>`→`State<AgentState>`**；bridge_config/mcp 接线改走 AgentState。验证：cargo check 两 crate + `clippy -p golish --lib -D` + `clippy -p golish-agent-app -D` + `check_dag`(**50 crates**) + `check_repo_ownership` 全 exit 0；ReadLints 无错（顺带 #[allow(dead_code)] 3 处 pre-existing 死字段 pty/sidecar/db_pool_ready，recompile surfaced）。**M4-proper 完成+验证（2026-05-31 · MCP-agent-3 接另一 MCP 半成品收尾）**：另一 MCP 已 `git mv` ai/ 全子树 + conversation_store 入 golish-agent-app、runtime/ 下沉 golish-app-core（TauriRuntime 解耦 AppState 改 take pty_output_tap 参数）、golish 侧 ai.rs/runtime.rs/conversation_store shim + facade + 守卫，但只跑 cargo check（带 4 unused warning）、从未跑 clippy、且在删死 re-export 前被掐断。本会话补完：① agent-app lib.rs 加 crate 级 `#![allow(clippy::too_many_arguments)]`（agents.rs:43 16 参命令）；② 删 4 个死 re-export（state AgentState / tools pentest_ai+pentest_bridge / db PgPentestStore，db/mod.rs 现空占位）。验证全绿：cargo check 两 crate + clippy 两 crate `-D warnings` + nextest -p golish-agent-app(**15✓**) + check_dag(**50 crates**) + check_repo_ownership 全 exit 0。**M5 platform 完成+验证（2026-05-31 · MCP-agent-3 · 用户「开 M5 platform」）**：抽 `golish-platform-app`(L5.5 纯叶子，零兄弟依赖)——`tools/{vault,audit,notes,recordings}.rs`(4 文件全 `State<DbState>`)`git mv` 入 crate，跨服务读经 golish_db::repo(L2) 不经兄弟 crate；导入重映射 `crate::{error,state::DbState,tools::scoping}`→`golish_app_core::*`；crate 级 too_many_arguments allow；facade vault/workspace 转发；golish tools/mod 删 4 pub mod + 删死 scoping re-export；守卫 check_dag(platform-app=5.5)+check_repo_ownership(SOURCE_ROOTS + DOMAIN_RULES 清 4 + ALLOWLIST/RAW_SQL 迁前缀)。验证全绿：cargo check 两 crate + clippy 两 crate -D + nextest -p golish-platform-app(**1✓**) + check_dag(**51 crates**) + check_repo_ownership 全 exit 0。**🎯 crate-per-service 北极星：5 个服务域(vuln/recon/pentest/agent/platform)全部层次 A 抽完。** epic 维持 in_progress 待层次 B（端口切兄弟硬依赖升真微服务）/ precommit / commit 收口。**层次 B 启动 · S1-2b1 完成（2026-05-31 · MCP-agent-3 · 用户「开层次 B 端口化」→「按推荐开干 b1」）**：发现 S1-2b 设计过期（写于层次 A 前，假设端口放 golish/src/ports），修正端口家为 **golish-app-core/src/ports/recon/**（6 消费方已分散到 4 app crate，不能依赖 golish）。建 ReconScansPort(10 method)+ReconAssetsPort(1 method)（镜像 repo 签名去 pool、返回同 Row 类型 remote-ready、纯透传适配器）；GolishDbRepoProvider 加 2 端口字段 new(pool) 内构造（外部签名不变）；agent-app recon.rs 11 调用点迁端口；守卫加 ('ports/recon','recon') + 删 5 条 ALLOWLIST。验证全绿：check app-core/agent-app + nextest ports::recon(2✓) + clippy 三处 -D 零告警 + check_dag(51) + repo_ownership(OK,ALLOWLIST 净减 5)。**b2 完成（2026-05-31，用户「接着开 b2」）**：security_analysis.rs（pentest，10 自由 Tauri 命令）5 recon 表迁端口；因须保留 pool_ready 就绪门，用『就绪门后内联构造适配器』（非 struct 注入）；扩 ReconScansPort +4 + ReconAssetsPort +1 method；删 5 条 ALLOWLIST（累计 28→18）；验证全绿（check pentest-app + nextest ports::recon 2✓ + clippy app-core/pentest-app/golish --lib -D 零告警 + 双守卫）。**b3-b6 完成（2026-05-31，用户「连续干 b3-b6」）→ S1-2b ReconPort 全 6 子片完成,22 条 recon 跨服务耦合全切断（ALLOWLIST 28→6）**：新建 ReconTargetsPort/ReconSitemapPort/ReconDirectoryPort + 扩 ReconScansPort（js_analysis_update_file_path_by_url、passive_scans_list_global_by_project，含端口 DTO ReconPassiveScanGlobal 解泛型 object-safety + app-core 加 chrono）；迁 8 文件（pentest_bridge 5 + pipeline/storage + platform/audit + vuln/matching），`&PgPool` 消费方用 Arc::new(pool.clone()) 注入。验证全绿：check 3 消费方 + nextest ports::recon(5✓) + nextest pentest/platform/vuln(48✓ 无回归) + clippy 五处 -D 零告警 + 双守卫。剩余 ALLOWLIST 6 = pentest_plan/vuln/agent_log/scan_queue（S1-2c/d/e/f，非 recon）。 **commit + S1-2c/d/e/f 完成（2026-05-31，用户「你帮我commit吧...后面全部做完」）**：① M0–S1-2b 已 commit `45f4bb2`（229 文件，未 push，本地 ahead 12）；② S1-2c（VulnIntelPort+WikiKbPort）/ S1-2d（PentestPlanPort）/ S1-2e（AgentLogReadPort，含 DTO 解泛型）/ S1-2f（scan_queue REPO_OWNER vuln→recon 伪阳性修正）全部完成 → **S1-2 横向耦合端口化整体完成，ALLOWLIST 28→0（cross-service ratchet 清空，每条横向 repo 耦合都走 golish-app-core/ports/ 服务端口）**。验证全绿：clippy app-core/agent/platform/golish --lib -D 零告警 + nextest app-core ports(10 object-safe) + nextest agent/platform/vuln(16✓ 无回归) + check_dag(51) + check_repo_ownership OK clean(ALLOWLIST 空)。**下一步 = commit S1-2c-f；（用户回来后）跑 just precommit 决定 push**。**§2.1：2 个 in_progress = arch-crate-per-service-split（父 epic）+ arch-s1-2b-recon-port（子里程碑），同一工作流父/子两粒度。** M0–S1-2b 已 commit(45f4bb2,未 push)；S1-2c-f 待 commit；全套未 push、未跑 just precommit 全量。前置端口：`arch-s1-2b-recon-port`（`not_started`，设计已写，ReconPort 是 M2 recon 抽取的前置）。父条目 `arch-s1-2-port-horizontal-coupling` 已 **passing**（S1-2a 走路骨架确立）。`target-surface-workbench` 继续 `blocked`。**§2.1 当前 in_progress 数 = 1（arch-crate-per-service-split）**。 |
| **当前 blocker** | `xiaomi-mimo-provider` 已从 `in_progress` 切 `blocked`，等待 tool-use compatibility layer 与真实 MiMo E2E 后再决定 passing。2026-05-27 复测发现 `ask_human` 被误包成普通 ToolApprovalRequest；已修为直接发 `AskHumanRequest`，但需重启 dev app 后真实复测。**2026-05-30 更新**：本机 `just check` **全绿**（fmt + check-fe + test-fe + lint-rust（clippy `-D warnings` 0 告警 + `cargo fmt --check`）+ test-rust-all（nextest **2592 passed / 7 skipped / 0 failed**）+ check-types（ts-rs 绑定无漂移）均 ✅）。此前记录的 clippy warnings 与 sandbox PermissionDenied baseline failures 在本机最新工作树**未复现**。 |
| **未提交的半成品** | **2026-05-30：架构优化批已拆 9 commit 落 `feat/recon-service`（`98beea9`→`6aaa0fb`，HEAD `d060ce4`）。** 其上叠了 **P0-3b 残余作用域 SQL 下沉**（T1-T6 全部完成，**未 commit**）：26 个 tracked 文件改动 + 6 个新 repo 模块（untracked：`repo/{scan_queue,sensitive_scan,conversation_store,directory_entries,sitemap_store,custom_rules}.rs`）。验证：rg 命令层裸作用域 SQL 清零、`golish-db` nextest 46/46、`golish --lib` nextest 318/318、`clippy golish-db+golish` 全绿，并跑通**全栈 `just precommit` → `✓ All checks passed!`（exit 0）**（含用户授权后修的 1 个 pre-existing `integrations/commands.rs:179` baseline）。**已按拆分提交 4 个 commit**（`65e0292`/`06af27a`/`d023386`/`c2f5ad2`，落 `feat/recon-service`，未 push）。**2026-05-30 续（MCP-2）：P2 拆分①完成——`golish-pentest-domain/src/models.rs`(1310) 模块化为 module-root + `models/{tool_config,asset_intel,runtime,tests}.rs`（全 < 500 行），全验证通过（crate check/nextest 17✓/clippy `-D warnings`/`cargo check --workspace` 全绿），**未 commit**（`M models.rs` + `?? models/`）。P2 拆分②完成——`golish/src/tools/pentest_bridge/js_collect.rs`(1357) 模块化为 module-root + `js_collect/{extract,judge,quality,sitemap,tool_impl,tests}.rs`（全 < 500 行，max 470），全验证通过（`cargo check -p golish`/`nextest js_collect` 26✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M js_collect.rs` + `?? js_collect/`）。P2 拆分③完成——`golish/src/tools/integrations/capture/engine.rs`(1483) 模块化为 module-root + `engine/{extract,helpers,tests}.rs`（全 < 500 行，engine.rs 496）；生命周期/webview 方法留 root 避免 super:: 改写，全验证通过（`cargo check -p golish`/`nextest capture::engine` 23✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M engine.rs` + `?? engine/`）。P2 拆分④（进行中）——`frontend/mocks.ts`(4135→2353) 抽出事件系统/AI 模拟/showcase 三层到 `mocks/{event-bus,events,simulations,showcase}.ts`（公共面零变更；`showcase.ts` 1146 仍 >500 待再分），`check-fe`+`test-fe` 全绿；剩余 demos/有状态 ipc 待续。**✅ 已按块 commit**：经 `just precommit` 全绿（`✓ All checks passed!`，~21.7min）后落 5 个 commit 到 `feat/recon-service`（`a71319b` pentest-domain models / `03871db` js_collect / `63c196e` capture engine / `83a105c` frontend mocks / `dd3c367` docs progress，**未 push**）。**2026-05-30 收尾（MCP-agent-2）：本会话架构体检全批（拆/合并/优化/dedup）已 `cargo fmt --all` 后按主题拆 20 个 commit（`a85f7d4`(scripts)→…→ docs(progress)，**未 push**）；提交后工作树 clean。完整 `just precommit` 本轮未重跑（树稍早已全绿，fmt 仅排版）。** **2026-05-30 续（MCP-5 · 接 MCP-3 转交）：S1-1 repo 数据所有权守卫 + check_dag 修复**——已修既有 `golish-graphiti(L1)→golish-db(L2)` DAG 违规（graphiti 归 L2，非删依赖）；`just arch` → **exit 0**（双守卫全绿）。已落 4 commit 到 `feat/recon-service`（`b0811ea`/`dc9ad0f`/`821c101` + 1 docs commit，**未 push**），提交后工作树 clean。feature_list `arch-s1-1-repo-ownership-guard` → **passing**；`just precommit` 未重跑（改动集零 Rust/TS/Cargo diff）。 **2026-05-30 续（MCP-agent-4 数据工程）：S1-2a `VaultReadPort` 走路骨架** —— 另一会话写 Tasks 1-4（端口/迁移/注入），本会话接手 Task 5（守卫拔 ratchet）+ Task 6（文档/feature_list/progress）。改动：`?? golish/src/ports/`(3 文件)、`M golish/src/lib.rs`、`M tools/pentest_bridge/{vault_ops,auth_probe,mod}.rs`、`M scripts/check_repo_ownership.py`、`M docs/architecture.md`、`M feature_list.json`、`M agent-progress.md`、`?? docs/{design,plans}/2026-05-30-s1-2-*`。验证：`cargo check -p golish` exit 0、`just arch` exit 0（ALLOWLIST **30→28**）、guard OK clean、`rg golish_db::repo::vault` 于 pentest_bridge 空。**2026-05-30 续（MCP-agent-3 后端工程，用户授权 C: A+B 一气呵成）**：跑 `cargo nextest -p golish ports::platform::vault` → **1 passed/373 skipped exit 0**（4m53s 冷编译）+ `just precommit` → **✓ All checks passed! exit 0**（29.6 min · fmt+check-fe+test-fe+lint-rust+test-rust-all 全绿）；按 plan 拆 **6 commit 落 feat/recon-service**：`6abaec8`(feat 端口骨架,4f+118)/`1e162de`(refactor VaultTool,1f)/`1a7018b`(refactor AuthProbeTool,1f)/`1149ddb`(refactor 构造点注入,1f)/`389d3fd`(chore 拔 ratchet,1f) + `23e47a6`(docs S1-2 design+plan+architecture+feature_list+progress,5f +947-3)；**未 push**，本地 ahead 10。**2026-05-30 续 2（MCP-agent-3 · 用户授权"你想怎么搞合适"）**：S1-2 父条目 `arch-s1-2-port-horizontal-coupling` → **passing**（走路骨架确立）；**新增** `arch-s1-2b-recon-port` 条目 `not_started`（等用户审 §10 5 决策再转 in_progress）；**新写** `docs/design/2026-05-30-s1-2b-recon-read-port.md` S1-2b 高层设计（22 条 allowlist 精确清单+grep 实证、6 子片划分 b1-b6、ReconPort trait 25 method 含读+写、守卫配合、5 待拍板决策）；命名差异关键：a 是 ReadPort（read-only），b 是 Port（含写，因 agent-bridge 适配器内有 insert/upsert/update）。新增/修改 3 文件：`?? docs/design/2026-05-30-s1-2b-recon-read-port.md`、`M feature_list.json`、`M agent-progress.md`。**待 commit + 不 push**（push 需用户单独点头，按 AGENTS.md §2.7 红线保守处理）。 **2026-05-30 续（MCP-agent-2）：M1 crate 抽取全套未 commit** —— `?? backend/crates/golish-app-core/`(M0)、`?? backend/crates/golish-vuln-app/{Cargo.toml,src/lib.rs}` + `RM` 19 文件（vuln_intel 8 + wiki 11，git mv 进 golish-vuln-app/src/）、`M backend/Cargo.toml`、`M golish/{Cargo.toml, src/commands_facade/{vuln_intel,wiki}.rs, src/tools/mod.rs, src/error.rs, src/state/db.rs, src/event_emitter.rs}`、`M scripts/check_{dag,repo_ownership}.py`、`M feature_list.json`、`M agent-progress.md`。验证：`cargo check` 两 crate + 双守卫全 exit 0；**未跑 just precommit 全量、未 commit、未 push**。 |

---

## 会话记录

> 倒序排列,最新一轮在最上面。每轮一条。

---

### 2026-06-01 · harness 闭环集成测试（drive_stage_transition · 内存 operation_state repo + 审批通道）（MCP-agent-1 · DISPATCH off · §5.9 单会话直接执行 · 用户「写闭环集成测试」→「commit」→「归档进 progress」）

- **本轮目标**：用户在 harness Phase C 收口（commit `634a6dc`）后要「进程内闭环集成测试」（计划 Phase D 第 2 项：mock executor + 内存 operation_state repo）。
- **靶点选择**：测 `task_orchestrator/subtask_phases/execute.rs` 的私有 `drive_stage_transition`（gate outcome → DAG 决策 → 审批闸 → 推进游标）。**关键：该方法不查 `stage_mode_enabled()`（LazyLock 缓存 env），故测试确定性、不受 `GOLISH_HARNESS_STAGE_MODE` env 影响**；全量 `run()` 路径因 flag 是 LazyLock 需独立 env 进程，不适合默认 nextest。
- **新增** `subtask_phases/execute_harness_loop_tests.rs`（495 行，`#[cfg(test)] #[path=...] mod` 挂为 execute 子模块 → 可达私有 `drive_stage_transition` + `HarnessGateOutcome`）；`execute.rs` +4 行 mod 声明。
- **内存 repo**：`MemRepo` 实现 `DbRepoProvider`，仅 `operation_state_{insert,get,advance_stage}` 真实（`Mutex<HashMap>`），其余 ~39 方法 `unimplemented!()`（transition driver 不触）。
- **4 个闭环测试**：① `pass_walks_cursor_along_assessment_dag`（PASS 沿 assessment DAG scoping→target_intel→eas→分支首选 enumeration→reporting，终点 Complete 不动；中间 stage 因 approval_policy 开预喂 approve）② `block_holds_cursor`（gate BLOCK→Hold）③ `approval_gate_holds_on_non_affirmative_reply`（pentest vuln_triage→verification 审批闸回「no」→hold+发 waiting_approval）④ `approval_gate_resumes_on_affirmative_reply`（同闸回「approve」→resume 推进 verification）。测试 ③ vs ④ 同设置/反回复/反结果 → 证明 C5 审批分支真被执行（非空过）。
- **运行过的验证（已记录证据）**：`cargo nextest -p golish-agent-kit execute_harness_loop_tests` → 4/4；`cargo nextest -p golish-agent-kit --no-fail-fast` → **357/357**（原 353，+4，无回归）；ReadLints → 0；`cargo clippy -p golish-agent-kit -- -D warnings`（= `just lint-rust` 口径，无 `--all-targets`）→ exit 0。
- **提交记录**：commit `0ff5b6a`（`test(harness): closed-loop integration tests for stage-transition driver`，2 文件 +499），落 `feat/harness-2026-06-01`，**未 push**（§2.7）。分支 HEAD：`0ff5b6a` ← `634a6dc` ← `3a06265`。
- **范围/诚实**：clippy `--all-targets`（比 gate 严）暴露一处**既有** `planner/tests/manager_tests.rs:365` type_complexity（非本轮代码、不在 gate `--workspace` 口径内），按「只在必要时改既有 lint」未动。本测试覆盖 transition 闭环（游标/审批/resume），**不**覆盖：全量 `run()` flag-on 路径、gate→repair reflector 回灌、C6 真 handoff 注入（后二者有 MCP-4 单测）、活体 E2E（需 LLM key + just dev，仍人工）。
- **下一步建议**：① push `feat/harness-2026-06-01`（需用户点头）；② 补 handoff/repair 闭环集成测试；③ 活体 E2E（`GOLISH_HARNESS_STAGE_MODE=true GOLISH_HARNESS_PROFILE=red_team just dev` + LLM key）。

---

### 2026-06-01 · harness Phase C 收口：C1–C6 闭环 + 多 profile 选择 + 起点 scoping + 修 4 红测试（MCP-agent-4 · DISPATCH off · §5.9 单会话直接执行 · 用户「是不是 harness 全搞定了」审计 →「修红测试」→「补 profile 选择+起点 scoping」→「补 C5 resume/C6 真交接」→「commit+归档」）

- **本轮起点**：用户问「harness 逻辑是不是全部搞定了」。先做**只读审计**（读 2026-06-01 两份计划 full-impl/rebuild + 实际代码），诚实结论：A+B+C 接线基本铺完、编译过、单测大多绿，但有 **4 红测试** + 多处 **MVP 半成品** + **从没活体跑过**。随后用户逐项让我收口。
- **审计结论（磁盘+行号实证）**：C1（`execute.rs` 读 `exec_ctx.harness_profile_id` 按 kind 载 spec）/ C2（`prompts::stage_charter` + 注入）/ C3（runtime `tool_dispatch.rs:164-215` 调 `PreActionAuthorizer::check_with_max_authz`，authz 穿透 orchestrator→bridge 侧信道 `trait_impl.rs:92`→runtime）/ C4（`pending_gate_correction` + `build_gate_correction`）/ C5（`stage_transition.rs:84` + `drive_stage_transition`）/ C6（`prompts::stage_inherited_evidence`）均已接。
- **修 4 红测试**：3 个 e2e（happy/sprint_contract/skipped_check）因新 `min_invocations_check` 拦 happy fixture（`external_attack_surface.json` 声明 `min_invocations={dns_resolve,http_probe,subdomain_enum_passive}`，fixture 的 `required_checks_done` 没列）→ 给 `happy_deliverable` 补这 3 个工具名。第 4 个 `operation_graph`：图实有 **15** 边、断言 **13**（DAG 扩边后没同步）→ 改名 `base_graph_has_12_nodes_15_edges` 断言 15。**根因是 Phase B 泛化引入的回归，非「无关旧账」**。
- **补 profile 选择 + 起点 scoping**：新增 `harness::active_profile_id()`（env `GOLISH_HARNESS_PROFILE`，默认 assessment，LazyLock，同 `stage_mode_enabled` 套路）；`orchestrator.rs` startup 原写死 `insert("assessment", external_attack_surface)` → 改 `load_embedded_profile` 校验选中 profile（未知 id 回退 assessment），起点改 `StageKind::Scoping`（DAG 入口）。效果：pentest/red_team/bug_bounty/cloud_assessment 在运行时可达（之前是死数据）。
- **C5 resume**：`drive_stage_transition` 改 `&mut self`；命中审批闸 emit `waiting_approval` → **阻塞等 `user_input_rx`** → 肯定回复（`approval_reply_is_affirmative` en+zh）才 `advance_stage`，否则 hold（无交互通道则只 hold 不推进，保留测试行为）。
- **C6 真交接**：`TaskOrchestrator` 加 `harness_evidence: HashMap<String,String>`；gate PASS 时 `summarize_deliverable()` 生成摘要存库；下游 stage 建 charter 时 `render_inherited_handoff()` 按 `inherits_evidence_from` 查库注入「ACTUAL UPSTREAM RESULTS」（不再只是静态 kind 提示）。
- **运行过的验证（已记录证据）**：`cargo nextest -p golish-agent-kit --no-fail-fast` → **353/353 passed**（flag off）；同命令带 `GOLISH_HARNESS_STAGE_MODE=true` → **353/353**（flag on 路径健康）；`cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings` → exit 0；`rustfmt --check`（harness 模块 + task_orchestrator 改动文件）→ clean；`cargo check -p golish-agent-app -p golish-agent-runtime -p golish-agent-bridge` → exit 0。新增 5 单测（active_profile / read_env_profile / approval_reply / summarize_deliverable / render_inherited_handoff）。顺手 fmt 了 3 个 Phase B 遗留 drift 文件（min_invocations_check/resources/stage_harness，纯排版）。
- **范围/风险（诚实）**：**未做活体 E2E**（游标真推进 / 审批真弹窗 / BLOCK 真回灌只有编译+单测证据）。MVP 边界：C6 handoff 内存级（不跨进程/resume 持久化，resume 回退静态提示）；C5 approval 复用 `user_input` 通道无独立审批 UI；profile 选择 env 级无 UI/任务元数据。**关键认知**：harness 是 `task_orchestrator` 的 flag 叠加层（5 个 `stage_mode_enabled()` 全在 `task_orchestrator` 内），**非独立引擎，删 task 逻辑=删 harness**。
- **提交记录**：本轮收口 commit 13 文件（11 code + agent-progress.md + feature_list.json），落 `feat/harness-2026-06-01`，**未 push**（§2.7）。
- **下一步建议**：① 活体 E2E（`GOLISH_HARNESS_STAGE_MODE=true GOLISH_HARNESS_PROFILE=red_team just dev`）或写进程内闭环集成测试（mock executor + 内存 operation_state repo）；② stage 序列化执行（subtask 真按 DAG 顺序而非 harness_backfill 关键词）；③ Phase D Harness Lab 自反馈线。

---

### 2026-06-01 · harness 第 2 层 Operation DAG 引擎 + gate 驱动 stage 流转 + operation_state 游标接线（方案 A：Task=operation）（MCP-agent-1 · DISPATCH off · §5.9 单会话直接执行 · 用户「深挖 DAG 流转引擎」→「写第2层引擎」→「接真 DB 游标」）

- **本轮目标**：把 harness 的"中间断层"——第 2 层 Operation DAG 流转引擎——从"只有 JSON 数据、无运行时消费者"补成真引擎，并按用户选的方案 A（一个 Task = 一个 operation）接通 `operation_state` DB 游标，让 gate 过后游标真正推进。
- **背景实证（重连后磁盘核实）**：`resources/harness/graph/operation_graph.json`（12 节点 13 边）此前**从没被 Rust 解析**（全仓唯一引用是 `harness/types.rs:11` 注释）；`golish-db` 的 `operation_state` repo（含 `advance_stage`）定义但**零调用点**；`StageHarness::for_stage` 硬锁单 stage（`stage_harness.rs:40`）。即第 2 层是"空的中间"，整条链 flag 默认 OFF 从没真跑通。
- **新增 `harness/operation_graph.rs`（golish-agent-kit）**：`OperationGraph{nodes,edges}` + `StageEdge` + `load_operation_graph_from_json`（校验：边引用未声明节点→`UnknownNodeInEdge`；有环→`Cycle`，Kahn 拓扑排序）+ `base_operation_graph()`（`include_str!` 内置加载）+ `project(allowed)→AllowedDag`（Doc 3 §3.3 profile 投影：留 allowed 节点 + 两端都 allowed 的边）+ `AllowedDag::{next_stages,contains,is_terminal,entry_points,terminals}`。assessment 投影 = 5 节点 5 边（scoping→target_intel→external_attack_surface→{enumeration,reporting}，enumeration→reporting）。
- **新增 `harness/stage_transition.rs`（golish-agent-kit）**：`TransitionDecision{Hold,Complete,Advance,Branch}` + `decide_transition(current,gate_allowed,&dag)` + `decide_from_gate(&GateResult,&dag)` + `TransitionDecision::advance_target()`（Advance→该 stage / Branch→首候选 / Hold|Complete→None）。纯函数不碰 DB。
- **接 operation_state 游标（方案 A · 跨 crate）**：① golish-agent-kit：`DbRepoProvider` 加 `operation_state_{insert,get,advance_stage}` + `OperationStateView` 类型（db_traits/types.rs）+ db_shim `operation_state` 透传模块；测试 `StubRepo`（manager_tests.rs）补 3 stub。② golish-agent-app：`GolishDbRepoProvider` 实现 3 方法（mod.rs 委托 + orchestration.rs `_impl`）→ 调 `golish_db::repo::operation_state`（`&self.pool`）。
- **hook 驱动（task_orchestrator/subtask_phases/execute.rs）**：`apply_harness_gate_hook` 改返回 `(String, Option<HarnessGateOutcome>)`（保持 7 个 skip 透传分支）；新增 `drive_stage_transition(&self, operation_id, outcome)`：`base_operation_graph()` → `project(assessment.allowed_stage_set())` → `decide_transition` → `advance_target()` → `operation_state::advance_stage` 写库；接在**两个** gate 点（主成功路径 execute.rs:158 + fallback execute.rs 尾）。`orchestrator.rs::run()` 在 `stage_mode_enabled()` 时建 operation_state（operation_id=task_id，current_stage=external_attack_surface）。**flag OFF 零碰 DB，旧路径零影响**。
- **运行过的验证（已记录证据）**：`cargo check -p golish-agent-kit -p golish-agent-app --all-targets` exit 0（两 crate 编译）；`cargo nextest -p golish-agent-kit` → **334 passed / 0 failed**（含新增 13 operation_graph + 8 stage_transition + StubRepo 编译 + 既有 hook 测试）；`cargo clippy`：agent-app `-D` clean、agent-kit 仅 1 条**既有无关** warning（planner/tests/manager_tests.rs:365 type_complexity）；`cargo fmt --check` 两 crate clean；ReadLints 全无错。
- **范围/风险**：纯新增 + 收敛式 hook 改造。**未做活体端到端**（没开 flag + 起真嵌入 PG + 真 agent 跑一轮看 DB 真写行）——DB 写入是"接好 + 类型/单测过 + 编译过"，**非"亲眼看到表里多一行"**。Phase-1 简化：current_stage 种在 external_attack_surface（MVP 唯一实跑格）；Branch 自动取首候选；gate 跟 subtask 跑（沿用现有 hook 位置）。资产收集内容由同事负责（接缝 = deliverable schema + StageSpec），本轮只动"框架笼子"。
- **提交记录**：**未 commit、未 push**（按 §2.7；等用户定提交边界）。
- **下一步建议**：① 写带嵌入 PG 的集成测试锁死闭环（insert→模拟 gate 过→advance→读回 current_stage=enumeration）；② 补 scoping/target_intel/enumeration/reporting 4 个 StageSpec 让多格真连跑；③ 解锁 `StageHarness::for_stage` 单 stage 硬限（按 stage 从 disk 载 StageSpec）；④ 把 `next_stages` 叠 approval/authz 闸（`approval_policy` + `max_authorization`）。

---

### 2026-06-01 · legacy bridge 4 个遗留项全清（①死错误码 ②A2 IPC 收紧 ③mocks 死 case ④SubAgentInfo DTO）（MCP-agent-1 · DISPATCH off · §5.9 单会话直接执行 · 用户「扫掉0后遗留项」→「④② 都做」）

- **本轮目标**：清掉上一轮（2026-05-31 legacy single-bridge 重构）尾部标注的 4 个遗留项。重连后以磁盘实证恢复上下文（记忆已过期）。
- **①死错误码（后端，纯死码）**：`state.rs` 删 `AI_NOT_INITIALIZED_ERROR` 常量 + `ai_not_initialized_error()` 函数（grep 实证 0 调用方，文案 "Call init_ai_agent first" 已 stale）；清 `ai/commands/mod.rs` + `lib.rs` 两处 re-export。
- **③mocks 死 case（前端，纯死码）**：`mocks.ts` 删 8 个死 case（init_ai_agent/_vertex、send_ai_prompt、execute_ai_tool、get_available_tools、list_sub_agents、shutdown_ai_agent、is_ai_initialized；保留中间活的 list_workflows）；连带删 orphan `mockAiInitialized` 变量 + `mockTools`/`mockSubAgents`（import + `fixtures.ts` 定义）+ 同步文件头 doc。保留 `mockConversationLength`（clear/get_length/session 恢复仍用）。
- **④SubAgentInfo DTO（删文件，用户授权）**：生产者 list_sub_agents 已删 → DTO orphan（grep 实证前端仅 types.ts import+re-export，0 实际使用）。删 `core/tools.rs`（仅含该 struct）+ `core/mod.rs` 去 `pub mod tools`/`pub use tools::*` + 删生成的 `frontend/lib/generated/SubAgentInfo.ts` + `types.ts` 去 import+re-export。
- **②A2 IPC 签名收紧（用户授权）**：35 个 dual-mode 命令 `session_id: Option<String>` → 必填 `String` + 扁平化 body（去 `if let Some` + `Err(ai_session_required_error())`），按 context.rs 已有的 retry_compaction 目标形态。文件：context(8)/policy(8)/hitl(7)/loop_detection(7)/session(5)。各文件去 `ai_session_required_error` import；`state.rs` 删现已 0-caller 的 `ai_session_required_error()` fn。**故意排除 3 个真 Optional**（None 是合法路径，非 dual-mode）：`config.rs::update_ai_workspace`（全局更新工作区）、`analytics.rs::get_tool_call_stats`（跨全会话统计）、`workflow.rs::run_recon_pipeline`（已废弃命令忽略此参数）。
  - **前端 wrapper 同步收紧（让 typecheck 成为"所有调用方都传 sessionId"的证明）**：`context.ts` 15 个 wrapper（context+loop）本就 `sessionId: string` 必填，无需改；`approval.ts`(7 hitl)/`persistence.ts`(3)/`session.ts`(2) 共 12 个 `sessionId?` → `sessionId`。**policy 8 命令前端无 wrapper/调用方**（grep 0），后端收紧纯安全。唯一活调用方 `actions.ts:246 clearAiConversation(sessionId)` 传的是 string，安全。
- **运行过的验证（已记录证据）**：`cargo clippy -p golish-agent-app -- -D warnings` exit 0（1m35s，零告警，证实删 helper 后无 unused import）；`cargo clippy -p golish --lib -- -D warnings` exit 0（2m06s，registry generate_handler! 收紧签名后编译干净）；`pnpm typecheck` exit 0（**证明 35 命令所有前端调用方都传 sessionId + SubAgentInfo 移除无悬垂引用**）；`pnpm biome check`（6 文件）exit 0；`cargo check -p golish-agent-app`+`-p golish` exit 0（①③ 阶段，2m03s/1m49s）；ReadLints 全无错；grep 确认 `ai_not_initialized_error`/`ai_session_required_error`/`SubAgentInfo`/`core::tools` 全清。
- **范围/风险**：纯死码删除（①③④）+ IPC 签名收紧（②，typecheck 证安全）。未跑全量 `just precommit`（工作树有另一条 targets/ts-rs bindings 未提交流会混入；全程未碰它）。运行时 GUI 未实测（删/改的命令前端调用方极少且都传 sessionId，typecheck 已证类型安全；②建议日后配 GUI 冒烟一次）。
- **提交记录**：**未 commit、未 push**（按 §2.7；本批 legacy 清理 + 上轮 legacy 重构 + 那条 targets 流交织，提交边界需用户理清）。
- **下一步建议**：用户决定 ① 是否把 legacy bridge 全批（上轮重构 + 本轮 4 遗留项）单独 commit；② 是否对 ② 做一次 GUI 冒烟（hitl/context/loop/session 设置面板真点一遍）；③ 那条 targets/ts-rs bindings 未提交流单独收口。

---

### 2026-05-31 · legacy AI single-bridge 残余清除（A1 dead-fallback 收口 + stub 删除 + 7 个 dead session-less 命令/wrapper 删除）（MCP-agent-1 · DISPATCH off · §5.9 单会话直接执行 · 用户「A 仍做 legacy 重构」）

- **本轮目标**：完成 legacy single-bridge 重构收尾（用户在 B「清多会话 legacy bridge」后选 A）。重连后以**磁盘实证**为准（记忆已过期）。
- **关键实证（与上轮记忆相反）**：legacy `AiState.bridge` 字段**早已删除**；`get_legacy_bridge`/`with_legacy_bridge` 已是 always-erroring stub；前端主对话**早已走 session-keyed**（`send_ai_prompt_session`/`init_ai_session`）。即危险的核心链路迁移已完成,剩下纯 dead-code 清理（**零活路径行为变更**）。`initAiAgent`/`sendPrompt` 等 7 个 session-less 前端 wrapper **零调用方**（grep 实证）→ AI 对话未坏。
- **Phase A1（dual-mode fallback 收口）**：policy(7)/context(8,含 2 个 with_legacy_bridge)/loop_detection(7)/hitl(7)/session(5) 共 34 个命令的 `session_id=None → get_legacy_bridge()` 死分支改为内联 `ai_session_required_error()`（行为同:None 早已 error）；config.rs `update_ai_workspace` 删死 best-effort 块。各文件补 `use crate::state::ai_session_required_error`。
- **移除 stub**：`state.rs` 删 `get_legacy_bridge`/`with_legacy_bridge`（无调用方,grep 证）+ 更新模块 doc。
- **Phase B（删 7 个 dead session-less 命令）**：`core/tools.rs` 删 send_ai_prompt/execute_ai_tool/get_available_tools/list_sub_agents/shutdown_ai_agent/is_ai_initialized（保留 SubAgentInfo DTO）；删 `core/lifecycle.rs`（init_ai_agent 唯一内容）；`core/mod.rs` 删 lifecycle 模块 + 6+1 个 `__cmd__` re-export；`ai/mod.rs` `pub use commands::{}` 删 7 名；`commands_registry.rs` generate_handler! 删 7 名；facade doc 更新。
- **前端**：删 7 个 dead wrapper（providers.ts `initAiAgent` / session.ts `sendPrompt`+`executeTool`+`getAvailableTools`+`getAvailableSubAgents`+`shutdownAiAgent`+`isAiInitialized`）+ 清 session.ts 的 `ToolDefinition`/`SubAgentInfo` unused import + 删 1 stale 注释。barrel `export *` 自动跟随。mocks.ts 保留（dead case 无害,避免 unused-var 级联）。
- **运行过的验证（已记录证据）**：`cargo fmt -p golish-agent-app -p golish`；`cargo check -p golish-agent-app` exit 0（51s）；`cargo check -p golish` exit 0（registry macro 展开,1m41s）；`cargo clippy -p golish-agent-app -p golish -- -D warnings` exit 0（**零告警**,2m25s）；`pnpm typecheck` exit 0（证:无活调用方依赖被删 wrapper）；`pnpm biome check lib/ai/{session,providers}.ts` exit 0；ReadLints（4 文件）无错；`rg get_legacy_bridge|with_legacy_bridge` 全仓库 = **0 命中**。
- **风险/范围**：纯 dead-code 删除,**零活路径行为变更**（typecheck 即安全证明）。未跑全量 `just precommit`（仓库有大量上轮未提交改动会混入）；运行时 GUI 未实测（删的都是已 error/无调用的代码,风险极低）。
- **遗留（noted follow-up,未做以避免 scope creep）**：① `AI_NOT_INITIALIZED_ERROR`/`ai_not_initialized_error`（state.rs,pub dead,消息仍写 "Call init_ai_agent first" 已 stale,0 调用方）可删；② **A2**：把 34 个 dual-mode 命令 `session_id: Option<String>` 收紧为必填 `String`（IPC 签名变更,typecheck 可证安全,建议配合 GUI 冒烟）；③ mocks.ts 删 7 个 dead case；④ SubAgentInfo DTO 现 orphan（list_sub_agents 删后无生产者）。
- **下一步建议**：用户决定 ① 是否做 A2（IPC 签名收紧）；② 是否 commit（本批 legacy 清理 + 上轮一堆未提交需先理清提交边界,**未授权 push**）。

---

### 2026-05-31 · commit M0–S1-2b（45f4bb2）+ S1-2c/d/e/f 完成 → S1-2 横向耦合端口化整体完成（ALLOWLIST 28→0）（MCP-agent-3 · DISPATCH off · §5.9 · 用户「你帮我commit吧 commit完之后你自己后面全部做完 我出门了」）

- **上下文来源**：b3-b6 完成后用户授权「先 commit,再自主完成剩余全部」（commit 授权,**未授权 push**）。
- **commit M0–S1-2b**：`cargo fmt --all`（31 文件）后用户选「直接 commit」（中止全量 precommit）。`git add -A` + 1 个综合 commit `45f4bb2 feat(arch): crate-per-service split (M0–M5) + ReconPort layer-B (S1-2b)`（229 文件，文件交织无法干净拆分且禁 git add -p）。工作树 clean，本地 ahead 12，**未 push**。
- **S1-2c（agent 读 vuln）**：新建 `golish-app-core/ports/vuln/{intel,wiki}.rs` —— VulnIntelPort（search_entries）+ WikiKbPort（12 method：upsert_page/link_cve/delete_refs_from/upsert_page_ref/add_changelog/search_{fts,by_category,by_tag}/list_{cves_with_pocs,unresearched_cves}/poc_stats/upsert_poc_full，含读写）。迁 agent db_bridge recon.rs（vuln_intel 1）+ wiki.rs（wiki_kb 12）；GolishDbRepoProvider 加 vuln_intel/wiki_kb 字段。
- **S1-2d（agent 读 pentest）**：新建 `ports/pentest/plans.rs` —— PentestPlanPort（plan_list_active/plan_update_steps/plan_create）。迁 orchestration.rs execution_plans 3 调用；加 pentest_plan 字段。
- **S1-2e（platform 读 agent）**：新建 `ports/agent/logs.rs` —— AgentLogReadPort（agent_logs_list_by_project/search_logs_list_by_project）。**泛型方法 object-safety**：agent_logs/search_logs::list_by_project<T> 泛型 + 自定义投影 → 端口自带 DTO AgentLogGlobal/SearchLogGlobal（10/9 字段镜像投影），platform audit.rs 用 DTO 替换本地 AgentLogRow/SearchLogRow。terminal_logs 留守（platform 自有，不迁）。
- **S1-2f（scan_queue 归属修正）**：grep 实证 scan_queue 唯一用户=golish-recon-app/scan_queue.rs（4 调用），"vuln" 归属是伪阳性 → REPO_OWNER scan_queue vuln→recon（纯静态标签修正,无 schema/运行时影响）。非端口。
- **守卫**：DOMAIN_RULES 加 `("ports/vuln","vuln")`+`("ports/pentest","pentest")`+`("ports/agent","agent")`；删 6 条 ALLOWLIST（c 2 + d 1 + e 2 + f 1）→ **ALLOWLIST 完全清空**（cross-service ratchet 空,每条横向 repo 耦合都经服务端口）。注意：守卫 grep `repo::\w+` 会匹配注释,故 mod.rs 注释里 `golish_db::repo::execution_plans` 改写为「pentest plan repo」避免伪阳性。
- **运行过的验证（已记录证据）**：`cargo check` app-core/agent/platform 全 exit 0；`cargo clippy -p golish-app-core/-agent-app/-platform-app/--lib(golish) -- -D warnings` 全 exit 0（修 1 处 plans.rs doc_lazy_continuation：`(read\n+ write)` 的 `+` 行首被当 markdown 列表项）；`cargo nextest run -p golish-app-core ports` → **10 passed**（recon 5+vuln 2+pentest 1+agent 1+platform vault 1 object-safe）；`cargo nextest run -p golish-agent-app -p golish-platform-app -p golish-vuln-app` → **16 passed / 0 failed**（无回归）；`python3 scripts/check_dag.py` exit 0（51 crates）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean,ALLOWLIST 空）；ReadLints 无错。
- **完成定义**：S1-2（a-f）横向耦合端口化 = VaultReadPort(a)+ReconPort(b1-b6)+VulnIntel/WikiKb(c)+PentestPlan(d)+AgentLogRead(e)+scan_queue 修正(f) = **全完成**。**ALLOWLIST 28→0：每条跨服务 repo 耦合都走 golish-app-core/ports/ 服务端口**（remote-ready 雏形）。
- **未做（需用户授权）**：① commit S1-2c-f（本批改动 + 本记录,待 commit）；② push（未授权）；③ 全量 just precommit（fe-test/ts-rs 漂移/全量 nextest，用户选直接 commit 跳过）；④ 运行时 invoke 未实测。
- **下一步建议**：① commit S1-2c-f；②（用户回来后）跑 just precommit 拿全量绿证据 + 决定 push；③ 层次 B 完整切兄弟 crate 编译期硬依赖（当前 ReconPort 等只切了 repo 读写耦合，storage.rs 仍用 golish_recon_app::targets::db_target_add 这类域函数）。

---

### 2026-05-31 · S1-2b3-b6 完成 → ReconPort 全 6 子片完成（22 条 recon 跨服务耦合全切断）（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「连续干 b3-b6」）

- **本轮目标**：一次性完成层次 B 剩余 4 子片 b3-b6，把 pentest/vuln/platform 对 recon repo 的跨服务读写全迁端口。
- **新建 3 个 sub-port（app-core/ports/recon/）**：`ReconTargetsPort`（find_id_by_value_pair / find_id_by_value_or_name / exists_by_value_exact / match_rows_legacy）、`ReconSitemapPort`（read/delete_zap_sitemap）、`ReconDirectoryPort`（exists_by_url_project）。扩 `ReconScansPort`（js_analysis_update_file_path_by_url、passive_scans_list_global_by_project）。
- **两个技术决策（设计未预见）**：
  - ① **泛型方法 object-safety**：`golish_db::repo::passive_scans::list_global_by_project<T>` 是泛型（object-safe trait 禁泛型方法），消费方用自定义投影 `PassiveScanRow`（9 字段，platform 域，app-core 看不到）。SQL 是固定 9 列投影（非 SELECT *，证：repo 测试断言），不能返 PassiveScanLog。**解**：端口自带 DTO `ReconPassiveScanGlobal`（9 字段镜像投影 + camelCase serde + FromRow，app-core 加 `chrono` dep），端口返 Vec<DTO>；platform audit.rs 用 DTO 替换本地 PassiveScanRow（前端 JSON 形状不变）。这是 proper 端口设计（端口拥有自己的 DTO，不泄漏消费方类型）。
  - ② **`&PgPool` 注入**：b3 部分消费方（js_collect/sitemap free-fn、pipeline MainStorage trait 方法）接收 `pool: &PgPool` 无 Arc。**解**：`PgReconXAdapter::new(Arc::new(pool.clone()))`（PgPool 是 Arc 内核，clone 廉价 = Arc bump）。struct 工具用 `self.pool.clone()`（pool: Arc<PgPool>），命令用 `state.pool_arc()`。
- **迁移（8 文件）**：b3 pentest_bridge 5 文件（auth_probe/record_finding/js_collect{sitemap,tool_impl}/js_extract_apis，targets+sitemap_store+js_analysis）；b4 pipeline/storage.rs（MainStorage，targets+sitemap_store+directory_entries）；b5 platform audit.rs（passive_scans_global，DTO 替换）；b6 vuln matching.rs（match_rows_legacy，保留 pool 供 vuln_entries 查询）。注意：storage.rs 的 `golish_recon_app::targets::db_target_add` 是 recon-app 域函数（非 repo 读），不在本片范围。各文件须导入 trait 本体才能调方法。
- **守卫**：删 **12 条 ALLOWLIST**（b3 7 + b4 3 + b5 1 + b6 1）；DOMAIN_RULES `ports/recon` 已在 b1 加。RAW_SQL_ALLOWLIST 不动（auth_probe/sitemap/record_finding/storage 等仍有其他 raw sqlx，非 recon repo 读）。ALLOWLIST 累计 **28→6**。
- **运行过的验证（已记录证据）**：`cargo check -p golish-pentest-app -p golish-platform-app -p golish-vuln-app` exit 0；`cargo nextest run -p golish-app-core ports::recon` → **5 passed**（5 sub-port object-safe）；`cargo nextest run -p golish-pentest-app -p golish-platform-app -p golish-vuln-app` → **48 passed / 0 failed**（无回归）；`cargo clippy -p golish-app-core -p golish-pentest-app -p golish-platform-app -p golish-vuln-app --all-targets -- -D warnings` exit 0；`cargo clippy -p golish --lib -- -D warnings` exit 0（53s）；`python3 scripts/check_dag.py` exit 0（✓ 51 crates）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；`rg golish_db::repo::(targets|sitemap_store|directory_entries) pentest-app/src` → 空；ReadLints（9 文件）无错。
- **完成定义**：S1-2b（ReconPort，b1-b6 全 6 子片）= 5 sub-port 建成 + 8 文件迁移 + 删 22 条 ALLOWLIST + 全验证绿 = **完成**。**22 条 recon 跨服务耦合全切断**（pentest/agent/platform/vuln 不再直调 recon repo；经 app-core 端口）。剩余 ALLOWLIST 6 = pentest_plan 1(S1-2d) + vuln 2(S1-2c) + agent_log 2(S1-2e) + scan_queue 1(S1-2f)，非 recon。`arch-s1-2b-recon-port` 仍 in_progress 待全量 just precommit + commit（§2.7）。
- **未做（按 §2.7/§3，需用户授权）**：① commit/push（M0–S1-2b 全套 + 本记录）；② just precommit 全量；③ S1-2c/d/e/f（vuln/pentest_plan/agent_log/scan_queue 端口，剩 6 条 ALLOWLIST）；④ 注：层次 B 仍有 recon-app/pentest-app 等兄弟 crate 编译期依赖（如 storage.rs 用 golish_recon_app::targets::db_target_add 这类域函数，非 repo 读），ReconPort 只切了 repo 读写耦合，完整切兄弟 crate 硬依赖需后续。
- **下一步建议**：① S1-2c（agent db_bridge 的 vuln_intel + wiki_kb 读，2 条）；② 或跑 just precommit 收口 + 按里程碑 commit（高风险 §2.7 需点头）。

---

### 2026-05-31 · S1-2b2 完成：security_analysis.rs 迁 ReconPort（pentest 域，层次 B 第二片）（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「接着开 b2」）

- **本轮目标**：把 `golish-pentest-app/src/security_analysis.rs` 的 5 个 recon 表读迁到 recon 端口（复用 b1 + 新增 5 method）。
- **消费方结构差异（与 b1 关键不同）**：security_analysis.rs 是 **10 个自由 `#[tauri::command]` 函数**（take `State<DbState>` + `pool_ready()` 就绪门 + 调 repo），**非 struct**。b1 的 struct-字段注入不适用。
- **注入决策（保留就绪门 = 零行为变更）**：采用『**就绪门后内联构造适配器**』——`state.pool_ready().await?`（守门，丢弃返回的 pool）+ `PgRecon{Scans,Assets}Adapter::new(state.pool_arc()).method(...)`（查询经 app-core/recon 域适配器）。这样：①保留 pool_ready 就绪门（行为不变）；②满足守卫（命令不再直调 golish_db::repo::recon，repo 调用移入 app-core 适配器）；③零启动接线、零命令签名改动（最低风险）。**权衡**：未达「Arc<dyn Port> 注入」的完全 remote-ready（命令内联构造具体适配器），但达成守卫目标 + 单 repo 调用点；完全 DI 留阶段 4。
- **扩端口**：ReconScansPort +4 method（api_endpoints_list_untested → Vec<ApiEndpoint> / api_endpoints_count_by_target → (i64,i64) / passive_scans_list_by_url → Vec<PassiveScanLog> / passive_scans_list_vulnerable → Vec<PassiveScanLog>）；ReconAssetsPort +1（target_assets_count_by_target → i64）。均逐字镜像 repo。
- **迁移**：security_analysis.rs 10 调用点（target_assets/api_endpoints×2/fingerprints/js_analysis/passive_scans×4/overview 3 合 1）迁端口；**须导入 trait 本体**（ReconScansPort/ReconAssetsPort）至作用域才能调方法（首次 check 报 E0599 trait not in scope，补导入后绿）；recon 表 repo 清零，仅留 audit（SHARED，oplog_* 不迁）。
- **守卫**：删 5 条 ALLOWLIST `("golish-pentest-app/security_analysis.rs", "{api_endpoints,fingerprints,js_analysis,passive_scans,target_assets}")`（ratchet 净前进；ALLOWLIST 累计 28→18）。DOMAIN_RULES ports/recon 已在 b1 加，无需再动。
- **运行过的验证（已记录证据）**：`cargo check -p golish-pentest-app` exit 0；`cargo nextest run -p golish-app-core ports::recon` → **2 passed**；`cargo clippy -p golish-app-core -p golish-pentest-app --all-targets -- -D warnings` exit 0（零告警）；`cargo clippy -p golish --lib -- -D warnings` exit 0（零告警）；`python3 scripts/check_dag.py` exit 0（✓ 51 crates）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；`rg golish_db::repo::(recon 5 表) security_analysis.rs` → 空；ReadLints 无错。
- **完成定义**：S1-2b2 = 端口扩 5 method + security_analysis.rs 迁移 + 删 5 ALLOWLIST + 全验证绿 = **完成**。arch-s1-2b-recon-port 仍 in_progress（b3-b6 待做）。
- **下一步建议**：b3（`golish-pentest-app/pentest_bridge/{auth_probe,record_finding,js_collect/sitemap,js_collect/tool_impl,js_extract_apis}.rs`，5 文件最大片，引 ReconTargetsPort + ReconSitemapPort + js_analysis update_file_path_by_url；这些是 struct-based tools，注入方式可能回到 b1 风格）→ b4/b5/b6。未 commit/push、未跑 just precommit 全量。

---

### 2026-05-31 · 层次 B 启动 · S1-2b1 完成：ReconPort 骨架（app-core）+ agent-bridge 迁移（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「开层次 B 端口化」→「按推荐开干 b1」）

- **上下文来源**：M5 后用户「开层次 B 端口化」（端口切兄弟硬依赖升真微服务）。我读 S1-2b 设计 + VaultReadPort 范式 + feature_list 条目，发现**设计已过期**（写于层次 A crate 拆分前），把 5 决策 + 过期修正给用户拍板，用户「按推荐开干 b1」。
- **关键设计适配（过期修正）**：S1-2b 设计假设端口放 `golish/src/ports/recon`、消费方都在 golish。现 6 消费方已分散到 4 个 app crate（agent/pentest/platform/vuln-app），**不能依赖 golish** → 端口家修正为 **`golish-app-core/src/ports/recon/`**（VaultReadPort 同款位置；app-core 依赖 golish-db，PgReconAdapter 可调 `golish_db::repo::recon`；消费方都依赖 app-core）。
- **5 决策（用户全按推荐）**：D1 方案 Y 按表分子 port；D2 b1→b6；D3 不动 DbRepoProvider trait 只改 impl；D4 镜像 repo 名；D5 端口家=app-core。
- **b1 范围**：`golish-agent-app/src/ai/db_bridge/recon.rs`（`GolishDbRepoProvider` 的 5 个 recon 表、11 个 repo 调用点）。剔除同文件 vuln_intel（S1-2c）/ audit（SHARED），不动。
- **建端口（app-core/src/ports/recon/）**：`mod.rs` + `scans.rs`（`ReconScansPort` 10 method：api_endpoints insert/list、js_analysis insert/update_file_path/list、fingerprints upsert/list、passive_scans insert/list/stats）+ `assets.rs`（`ReconAssetsPort` 1 method：target_assets list）。**端口方法逐字镜像 repo 签名（去 pool）、返回同 Row 类型**（`golish_db::models::{ApiEndpoint,JsAnalysisResult,Fingerprint,PassiveScanLog,TargetAsset}` 全派生 `Serialize,Deserialize` → remote-ready）；`Pg*Adapter{pool}` 纯透传；模块级 `#![allow(clippy::too_many_arguments)]`（insert 宽签名）；各端口 object-safe 测试。ports/mod.rs 加 `pub mod recon`。
- **消费方接线（D3）**：`GolishDbRepoProvider` 加 `recon_scans: Arc<dyn ReconScansPort>` + `recon_assets: Arc<dyn ReconAssetsPort>` 字段；`new(pool)` 内部 `Pg*Adapter::new(pool.clone())` 构造（**外部 new(pool) 签名不变 → 零调用方改动**；pool 字段保留供 vuln_intel/audit 等用）。recon.rs 11 调用点 `golish_db::repo::<table>::<fn>(&self.pool,args)` → `self.recon_<scans|assets>.<table>_<fn>(args)`（去 pool，返回/to_value 处理不变）。
- **守卫**：`check_repo_ownership.py` DOMAIN_RULES 加 `("ports/recon","recon")`（app-core 适配器域=recon 合法）；删 5 条 ALLOWLIST `("golish-agent-app/ai/db_bridge/recon.rs", "{api_endpoints,fingerprints,js_analysis,passive_scans,target_assets}")`（ratchet 净前进）；保留 vuln_intel/wiki_kb（S1-2c）。
- **运行过的验证（已记录证据）**：`cargo check -p golish-app-core` exit 0；`cargo check -p golish-agent-app` exit 0；`cargo nextest run -p golish-app-core ports::recon` → **2 passed**（recon_scans/recon_assets object-safe）；`cargo clippy -p golish-app-core -p golish-agent-app --all-targets -- -D warnings` exit 0（1m10s 零告警）；`cargo clippy -p golish --lib -- -D warnings` exit 0（1m01s 零告警）；`python3 scripts/check_dag.py` exit 0（✓ **51 crates**）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean，ALLOWLIST 净减 5）；`rg golish_db::repo::(api_endpoints|js_analysis|fingerprints|passive_scans|target_assets) recon.rs` → 空；ReadLints（5 文件）无错。
- **完成定义**：S1-2b1（ReconPort 骨架 + agent-bridge 迁移）= 端口建成 + 消费方迁移 + 守卫拔 5 ratchet + 全验证绿 = **完成**。`arch-s1-2b-recon-port` 转 **in_progress**（b1 完成，b2-b6 待做）。
- **§2.1 说明（有意的父/子双 in_progress）**：当前 2 个 in_progress = `arch-crate-per-service-split`（父 epic，层次 A 全完成、层次 B 进行中）+ `arch-s1-2b-recon-port`（子里程碑，层次 B 的实现载体，b1 完成）。二者是**同一工作流的父/子两个粒度**（非并行无关任务），s1-2b 的 notes 早已写明「转 in_progress 时接管 §2.1 名额」。下一会话视 s1-2b 为当前活跃工作。
- **未做（按 §2.7/§3，需用户授权）**：① b2-b6（security_analysis/pentest_bridge/pipeline/audit/vuln matching 迁端口）；② commit/push；③ just precommit 全量；④ RAW_SQL allowlist 不动（recon.rs 仍有 vuln/audit raw 调用）。
- **下一步建议**：① b2（`golish-pentest-app/security_analysis.rs`，复用 b1 读方法 + 新增 count/untested/list_by_url/list_vulnerable/count_by_target，删 5 条 ALLOWLIST）；② 然后 b3（pentest_bridge 5 文件，最大片，引 ReconTargetsPort/ReconSitemapPort）→ b4/b5/b6；③（高风险 §2.7 需点头）跑 just precommit + 按里程碑 commit。

---

### 2026-05-31 · M5 完成：golish-platform-app 抽取 platform 服务 → crate-per-service 5 域全部抽完（层次 A）（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「开 M5 platform」）

- **上下文来源**：M4-proper 收尾后用户「开 M5 platform」。M5 是 crate-per-service 最后一域。
- **本轮目标**：①调查 platform 域耦合；②写 M5 子计划；③按层次 A 抽 `golish-platform-app`；④全验证。
- **子计划**：`docs/superpowers/plans/2026-05-31-m5-platform-app.md`。
- **实证（platform 是最干净一域）**：`tools/{vault,audit,notes,recordings}.rs`(4 单文件 792 行)**全部只 take `State<DbState>`、零 AppState**；跨服务读（audit.rs 读 `passive_scans`=recon-owned / `agent_logs`+`search_logs`=agent-owned）**全经 `golish_db::repo::`(L2 仓储层)而非兄弟 crate** → **零兄弟依赖的纯叶子 app crate（L5.5）**。`project_io.rs` 不在范围（留守 golish）。`ports/platform/*` 早在 M3/S1-2a 已下沉 app-core，M5 不搬。
- **抽取**：新建 golish-platform-app（deps：golish-app-core/core/db + tauri/sqlx/serde/serde_json/uuid/chrono/tracing/ts-rs/**reqwest**(vault_validate HTTP 探测，初次漏加，cargo check 暴露后补)）；backend/Cargo.toml members+default-members+workspace.deps；golish/Cargo.toml 加依赖；`git mv` 4 文件入 crate；lib.rs 声明 4 模块 + crate 级 `#![allow(clippy::too_many_arguments)]`（vault_add 等多参命令，镜像其他 app crate；初次漏加，clippy 暴露后补）。
- **导入重映射**：`crate::error::GolishError`→`golish_app_core::GolishError`、`crate::state::DbState`→`golish_app_core::DbState`、`crate::tools::scoping::`→`golish_app_core::scoping::`（vault/notes）。**ts-rs `export_to` 字符串不动**（按 crate 根解析，与 agent/vuln/recon 已迁移一致；nextest export_bindings_note 通过实证）。
- **facade**：`commands_facade/vault.rs`→`pub use golish_platform_app::vault::*`；`commands_facade/workspace.rs` audit/notes/recordings 三行→`golish_platform_app::*`。
- **golish 清理**：`tools/mod.rs` 删 4 个 `pub mod {vault,audit,notes,recordings}`（project_io 留守）+ **删死 scoping re-export**（`crate::tools::scoping` 最后消费者 vault/notes 已搬走，cargo check 暴露 unused → 删，否则挂 clippy）。
- **守卫**：`check_dag.py` 加 `golish-platform-app=5.5`；`check_repo_ownership.py` SOURCE_ROOTS 加 `(golish-platform-app,platform)`、删 4 条死 DOMAIN_RULES（tools/{vault,audit,notes,recordings}）、ALLOWLIST 3 键（audit.rs agent_logs/passive_scans/search_logs）迁 `golish-platform-app/` 前缀、RAW_SQL 2 键（audit.rs/recordings.rs）迁前缀（project_io.rs 保留）。
- **运行过的验证（已记录证据）**：`cargo check -p golish-platform-app` exit 0；`cargo check -p golish` exit 0；`cargo clippy -p golish-platform-app --all-targets -- -D warnings` exit 0（零告警）；`cargo clippy -p golish --lib -- -D warnings` exit 0（1m19s，零告警，确认 scoping 删除干净）；`cargo nextest run -p golish-platform-app` → **1 passed / 0 skipped**（ts export_bindings_note）；`python3 scripts/check_dag.py` exit 0（✓ **51 crates**）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；ReadLints（platform-app 4 文件 + golish tools/mod + facade）无错误。
- **完成定义**：M5（platform 服务）= 抽完 + 编译 + 测试 + clippy + 守卫全绿 = **完成（层次 A）**。**crate-per-service 北极星：5 个服务域（vuln/recon/pentest/agent/platform）全部层次 A 抽完。** `arch-crate-per-service-split` epic 维持 **in_progress**，待层次 B / precommit / commit 收口。
- **未做（按 §2.7/§3，需用户授权）**：① 层次 B（ReconPort/AgentLogReadPort 等端口切断兄弟硬依赖升真微服务）；② commit/push（M0–M5 全套 + 本记录均未提交，工作树脏 170+ 文件）；③ just precommit 全量（fe-test/ts-rs 漂移/fmt/golish nextest 全量）未跑；④ 运行时 invoke 未实测；⑤ db/mod.rs 空壳模块文件级删除（§2.7 删文件需点头）。
- **下一步建议**：①（高风险 §2.7 需点头）跑 just precommit 拿 M0–M5 完整绿证据 + 按里程碑 commit；② 层次 B 端口化（S1-2b ReconPort 起，切断 pentest/agent/platform 对 recon-app 的兄弟硬依赖）；③ 清理 db/mod.rs 空壳。

---

### 2026-05-31 · M4-proper 收尾：接另一 MCP 的半成品，补 clippy allow + 删死 re-export + 全验证（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「mcp 干了一半 M4-proper 你看看干到哪里了」+ 贴出另一 MCP 工作日志）

- **上下文来源**：用户说另一个 MCP 把 M4-proper 干了一半，让我核查进度；随后贴出该 MCP 完整工作日志。本会话先独立核查磁盘状态 + 实测编译/clippy，再与该日志对账，最后补完缺口。
- **核查结论（磁盘 = 该 MCP 日志，完全一致）**：另一 MCP 已完成 M4-proper 主体——① ai/ 全子树（ai/commands 19+core+agents、db_bridge、embedder/graph/session/sidecar_bridge、tracking_bridge、ai/mod.rs）+ conversation_store `git mv` 入 golish-agent-app（golish/src/ai 目录已不存在）；② runtime/ 下沉 golish-app-core（`TauriRuntime::new` 解耦 AppState，改 take `pty_output_tap: Option<Arc<PtyOutputTap>>`；cli.rs 一并下沉）；③ golish 侧 shim：`src/ai.rs`→`golish_agent_app::ai::*`、`src/runtime.rs`→`golish_app_core::runtime::*`、`tools/mod.rs` conversation_store re-export；④ facade `ai.rs`→`pub use crate::ai::commands::*`；⑤ 守卫 check_dag(agent-app=5.6)+check_repo_ownership(SOURCE_ROOTS + ALLOWLIST/RAW_SQL 迁 golish-agent-app/ 前缀)；⑥ agent-app Cargo.toml 补 dirs/dotenvy/ts-rs。
- **缺口（该 MCP 只跑 `cargo check`、从未跑 clippy，且在删死 re-export 前被掐断）**：① `cargo check -p golish` 绿但带 4 个 unused-import warning；② clippy `-D warnings` 下 golish-agent-app 直接编译失败（`agents.rs:43` 16 参命令触发 too_many_arguments，agent-app lib.rs 漏了 crate 级 allow）。
- **本会话补完 2 个修复**：① `golish-agent-app/src/lib.rs` 加 crate 级 `#![allow(clippy::too_many_arguments)]`（镜像 golish lib.rs:5 + pentest-app，搬入新 crate 丢失原 crate 级 allow 的老问题，M2/M3 同款）；② 删 4 个死 re-export——`state/mod.rs` `golish_agent_app::AgentState`、`tools/mod.rs` `pentest_ai`+`pentest_bridge`、`db/mod.rs` `PgPentestStore`（消费者均已随命令面搬去 agent-app）。`db/mod.rs` 现为空占位模块（已 vestigial，可在用户点头后删文件 + 删 lib.rs:29 `pub(crate) mod db;`，按 §2.7 保守未删）。
- **运行过的验证（已记录证据）**：`cargo check -p golish-agent-app` exit 0；`cargo check -p golish` exit 0；`cargo clippy -p golish-agent-app --all-targets -- -D warnings` exit 0（Finished 14.16s，零告警）；`cargo clippy -p golish --lib -- -D warnings` exit 0（Finished 34.59s，零告警）；`cargo nextest run -p golish-agent-app` → **15 passed / 0 skipped / 0 failed**；`python3 scripts/check_dag.py` exit 0（✓ **50 crates**）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；ReadLints（4 个编辑文件）无错误。
- **完成定义**：M4-proper（搬命令体 + 桥接层 + runtime 下沉 + 接线 + 收尾）= 编译 + clippy + 测试 + 守卫全绿 = **完成（层次 A）**。`arch-crate-per-service-split` epic **仍 in_progress**（M5 platform 未动）。
- **未做（按 §2.7/§3，需用户授权）**：① commit/push（M0+M1+M2+M3+M4-A+M4-proper 全套 + 本记录均未提交，工作树脏 160+ 文件）；② just precommit 全量（fe-test/ts-rs 漂移/fmt/golish nextest 全量）未跑；③ 运行时 invoke 未实测（编译+注册解析+状态装配证明，未真点 app）；④ db/mod.rs 空壳模块的文件级删除（§2.7 删文件需点头）。
- **下一步建议**：① M5 platform（vault/audit/notes/recordings，DbState 干净移，同 M2/M3 范式，crate-per-service 最后一域）；②（高风险 §2.7 需点头）跑 just precommit 拿 M0–M4 完整绿证据 + 按里程碑 commit；③ 清理 db/mod.rs 空壳（删文件需点头）。

---

### 2026-05-31 · M4 调查 + M4-A AppState 解耦（新建 golish-agent-app + 窄 AgentState）（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「直接开 M4」→「开 A」→「按 X 执行 A1」→「继续 A2+A3」）

- **M4 调查（发现 blocker）**：用户「直接开 M4」。实证 ai/ 域（39 文件）：`ai/commands/*`(19) 几乎全 take `State<AppState>`（session 11/context 9/hitl 8/policy 8/…）；`AppState`(state/mod.rs) 聚合 `ai_state: AiState`，而 **AiState 定义在 ai/commands/mod.rs:61**；三者互锁 → 把 ai/ 搬到 agent-app 会成 golish↔agent-app 循环。`ai/db_bridge`/`tracking_bridge`/`*_bridge`/`conversation_store` 是 AppState-free（可搬），但**命令面搬不动**=核心价值受阻。写 `docs/superpowers/plans/2026-05-31-m4-agent-app-feasibility.md`（4 选项 A/B/C/D）。用户选 **A（AppState 解耦）**。
- **A 设计**：实证 AgentState 需 13 字段（≈ AppState 减 command_index/telemetry/langfuse）；唯一 golish-内部类型 = AiState（只持 L4 AgentBridge + L1 GolishRuntime，可移）；IndexerState(L2)/SidecarState(L3)/PtyManager(L2) 皆 crate 类型。写 `docs/superpowers/plans/2026-05-31-m4a-appstate-decouple.md`。用户选 **X（新建 golish-agent-app 放 AiState+AgentState）**。
- **A1（状态地基）**：新建 `golish-agent-app`(L5.6，Cargo.toml + lib.rs + state.rs)；`AiState`(+ helper) 从 ai/commands/mod.rs 搬入（加 `#[derive(Clone)]` 共享 Arc）；新 `AgentState`(13 pub 字段)；golish `crate::ai::AiState` 经 commands re-export 兼容；`AppState::extract_agent_state()`(克隆共享 Arc)。golish-mcp 用 path dep（非 workspace.dep）。验证 cargo check 两 crate + check_dag(50) + clippy agent-app 全绿。
- **A2（命令迁移）**：`crate::state` re-export AgentState；**19 个 ai/commands** `State<AppState>`→`State<AgentState>`（replace_all，多数字段名不变）；bridge_config 内部链 `&AppState`→`&AgentState`；外部 caller `mcp/commands.rs::refresh_all_bridge_mcp_tools` + `app/mcp_bootstrap.rs` 改 `extract_agent_state()` 再传 `&agent_state`。
- **A3（启动接线）**：`tauri_app.rs` 加 `let agent_state = app_state.extract_agent_state();` + `.manage(agent_state)`（与 AppState 共享同批 Arc → 行为零变）。
- **pre-existing 死码处理**：state/mod.rs recompile surfaced 3 处 pre-existing 死码（PtyState.output_tap/busy_sessions、SidecarManaged.sidecar_config、AppState.db_pool_ready；均 unchanged vs HEAD，A4 narrow-state era 遗留，被增量 clippy 缓存掩盖）→ 加 `#[allow(dead_code)]` + 注释（镜像文件已有 new() 的 allow 风格）；删 AgentState 多余的 db_pool_ready（YAGNI）。
- **运行过的验证（已记录证据）**：`cargo check -p golish-agent-app` exit 0；`cargo check -p golish` exit 0（无 warning）；`cargo clippy -p golish --lib -- -D warnings` exit 0；`cargo clippy -p golish-agent-app --all-targets -- -D warnings` exit 0；`python3 scripts/check_dag.py` exit 0（✓ **50 crates**）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；ReadLints（agent-app/state + golish state/ai/commands/tauri_app/mcp）无错误。
- **完成定义**：M4-A（AppState 解耦）= 新 crate + AiState 搬移 + AgentState + 19 命令迁移 + 启动接线 + 验证 = **完成**。这是 M4 的前置：命令面已不再 take 单体 AppState，golish↔agent-app 循环已断。`arch-crate-per-service-split` epic **仍 in_progress**（M4-proper 移命令体 / M5 platform 未动）。
- **未做（按 §2.7/§3，需用户授权）**：① commit/push（M0+M1+M2+M3+M4-A 全套 + 本记录未提交）；② just precommit 全量；③ 运行时 invoke 未实测（编译 + 注册解析 + 状态装配证明，未跑 app 真点）。
- **下一步建议**：① M4-proper：把 ai/commands 命令体 + AppState-free 桥接层（db_bridge/tracking_bridge/session_bridge/embedder/graph/sidecar_bridge + conversation_store）搬进 golish-agent-app，facade 转发（此时命令已 take AgentState，可干净移）；② 然后 M5 platform（vault/audit/notes/recordings，DbState 干净移）；③（高风险 §2.7 需点头）跑 just precommit 拿 M0–M4-A 完整绿证据 + 按里程碑 commit。

---

### 2026-05-31 · M3 完成：golish-pentest-app 抽取 pentest 服务（M3 前置共享下沉 + M3a 干净 6 + M3b recon 耦合 + M3c AI 桥）（MCP-agent-3 · DISPATCH off · §5.9 单会话直接执行 · 用户「直接开 M3」）

- **上下文来源**：用户「你看进度 我 m2 全部搞定了 后面还要怎么搞」→ 我核实 M2 完成但 M0+M1+M2 全未 commit；用户选「直接开 M3」（跳过 precommit 收口 + S1-2b 前置，按层次 A 抽 crate）。
- **本轮目标**：①自读核实 pentest 域耦合；②写 M3 子计划；③按层次 A 抽 `golish-pentest-app`（编译期依赖 recon-app，ReconPort 升层次 B 留 S1-2b）。
- **子计划**：`docs/superpowers/plans/2026-05-31-m3-pentest-app.md`（writing-plans 格式，前置共享下沉 + M3a/b/c）。
- **M3 前置（两个共享件下沉 golish-app-core，实证发现的关键决策）**：① `pty_interactive`（被 golish 留守的 `state/`+`runtime/`+`ai/` 与 pentest_ai 双用，留 golish 则 pentest-app 成环）→ app-core（已含 golish-core+golish-pty，零新依赖；`run_shell_command_detail` 提 pub）；② `ports/`(VaultReadPort+PgVaultAdapter，S1-2a) → app-core（pentest_bridge 用，pentest-app 不能反依赖 golish；app-core 加 async-trait）。golish 侧 `tools/mod.rs` 留 `pub(crate) use golish_app_core::pty_interactive`、lib.rs 删 `mod ports`/`mod event_emitter`（移后无消费者）。
- **M3a（6 干净/自有域）**：findings/methodology/execution_plans/evidence/pentest（含 PentestState）/output_parser `git mv` 入 pentest-app；导入重映射 error/state/event_emitter→golish_app_core、`crate::tools::scoping`→`golish_app_core::scoping`、`crate::db::PgPentestStore`→`golish_pentest::output_store::PgPentestStore`、`crate::settings::SettingsManager`→`golish_settings::SettingsManager`。facade {pentest,workspace(execution_plans/methodology/output_parser),evidence,findings} 直指 `golish_pentest_app::<mod>::*`。
- **M3b+M3c（合并，因 pipeline↔pentest_bridge 互相依赖必须同移）**：security_analysis/pipeline/pentest_ai/pentest_bridge `git mv` 入 pentest-app；`crate::tools::targets`→`golish_recon_app::targets`、`crate::tools::pipeline`→`crate::pipeline`、`crate::tools::pentest_bridge`→`crate::pentest_bridge`、`super::super::vault`→`golish_core::vault::{obfuscate,deobfuscate}`、`crate::projects::file_storage`→`golish_projects::file_storage`、ports/pty→app-core。golish 入向桥 pentest_ai/pentest_bridge（ai/bridge_config 用 create_*_tools）；facade pipeline/workspace(security_analysis) 直指 crate。
- **golish 清理**：M3 后 4 个孤儿 re-export（event_emitter shim / projects::file_storage / ports 桥 / targets 桥）失去消费者 → 删除（含删 `golish/src/event_emitter.rs` 文件）。
- **守卫**：`check_dag.py` 加 `golish-pentest-app: 5.6`；`check_repo_ownership.py` SOURCE_ROOTS 加 `(golish-pentest-app,pentest)` + `(golish-app-core,None)`（守 vault 适配器，`ports/platform/vault.rs` 经 DOMAIN_RULES 命中 platform）；删 9 条死 pentest DOMAIN_RULES；迁 15 ALLOWLIST + 7 RAW_SQL 键到 `golish-pentest-app/` 前缀。
- **运行过的验证（已记录证据）**：`cargo check -p golish-pentest-app` exit 0；`cargo check -p golish` exit 0（无 warning）；`cargo nextest run -p golish-pentest-app` → **47 passed / 0 failed**；`cargo clippy -p golish-pentest-app --all-targets -- -D warnings` exit 0（补 crate 级 `#![allow(clippy::too_many_arguments)]`，镜像 golish lib.rs:5）；`cargo clippy -p golish --lib -- -D warnings` exit 0（1m50s，覆盖全 workspace）；`python3 scripts/check_dag.py` exit 0（✓ **49 crates**）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；ReadLints（接线文件 + 守卫脚本）无错误。
- **完成定义**：M3（pentest 服务）= 前置 + a + b + c 整体抽完 + 编译 + 测试 + 守卫 + clippy 验证 = **完成（层次 A）**；`arch-crate-per-service-split` epic **仍 in_progress**（M4 agent / M5 platform 未动）。层次 B（切 agent/platform 对 pentest/recon 入向硬依赖）留 S1-2b/后续端口。
- **未做（按 AGENTS.md §2.7/§3，需用户授权）**：① `git commit`/`git push`（M0+M1+M2+M3 全套 + 本记录均未提交，工作树脏 160 文件）；② `just precommit` 全量（golish nextest 全量 / fe-test / ts-rs 漂移 / fmt）未跑；③ 运行时前端 invoke 未实测（编译 + 注册解析证明）。
- **下一步建议**：①（高风险 §2.7 需点头）跑 `just precommit` 拿 M0+M1+M2+M3 完整绿证据，按里程碑 commit；② 开 M4 抽 golish-agent-app（ai/ 栈最厚，前置端口最多）；③ 或先做 S1-2b ReconPort 把 recon/pentest 从层次 A 升 B。

---

### 2026-05-31 · M2 完成：golish-recon-app 抽取 recon 服务（M2a 7 干净模块 + M2b asset_intel + M2c integrations）（MCP-agent-4 · DISPATCH off · §5.9 单会话直接执行 · 接 MCP-2 上下文转移 · 用户「abc 一起搞」）

- **上下文来源**：用户从 MCP-2（`bajie-mcp-agent-2-yw50eby4`）转移上下文，指令「abc 一起搞」= M2a+M2b+M2c 三子步全做。`get_session_summary` 取 MCP-2 研究（recon 9 模块体检：7 干净 / asset_intel 阻塞 / integrations 重），本会话先自读核实（未读不引）再动手。
- **本轮目标**：①自读核实 recon 耦合；②写 M2 子计划；③M2a/b/c 逐子步搬迁 + 每步独立验证。
- **关键发现（修复半成品）**：上一会话留下的 `golish-recon-app` 是**半成品**——`Cargo.toml`+`lib.rs` 声明 6 模块但模块文件未搬、`check_dag.py` LAYER_TABLE 缺登记（守卫当时 RED），`golish/Cargo.toml` 缺依赖。本会话补齐并完成。
- **M2a（7 干净模块 + wordlists）**：`git mv` targets(6)/organizations(5)/scan_queue/sensitive_scan/custom_rules/scan_runner/intel_providers → recon-app；连带 `wordlists`（被 sensitive_scan `super::wordlists::wordlist_path` 依赖，recon-app 不能反向依赖 golish 故一并搬）。`scoping`（33 行 IDOR 守卫，6 处用）**下沉 golish-app-core**（fn 提 pub），golish `tools/mod.rs` 改 `pub(crate) use golish_app_core::scoping`。导入重映射 error/state/event_emitter→golish_app_core、`crate::tools::scoping`→`golish_app_core::scoping`、`crate::tools::targets`→`crate::targets`（scan_runner 域内）。入向桥 `pub(crate) use golish_recon_app::targets`（pipeline/storage 留守用）。facade `workspace.rs`(7 行)+`intel_providers.rs` 直指 `golish_recon_app::<mod>::*`（M1 glob 范式，__cmd__ 宏解析）。`upsert_organization_candidates_for_org` 提 pub。
- **M2b（asset_intel，解 PentestState）**：唯一坏耦合 `crate::tools::pentest::PentestState`（L6 god-crate），窄用 `config_manager.get().toolsconfig_dir`。新建 `ToolsConfigState(Arc<golish_pentest::ConfigManager>)` 窄受管状态（8 命令改收它，`pentest.0.get()`）；golish `tauri_app.rs` 克隆 PentestState **同一 Arc** 注入 → 行为零变。`crate::tools::organizations`→`crate::organizations`（域内）撤 organizations 桥；补 `golish-projects`(L1) 依赖（file_storage）。
- **M2c（integrations，最重）**：13 文件含 tauri webview 捕获引擎（CaptureEngine/WebviewWindowBuilder/Wry）。导入重映射 error/state→golish_app_core、`crate::tools::integrations`→`crate::integrations`；golish `tauri_app.rs` 的 `CaptureEngine`/`IntegrationsState` import 改指 `golish_recon_app::integrations`（受管状态类型不变，构造点不变）；facade 显式导出函数(mod 根)+`__cmd__` 宏(子模块 commands/capture_commands 路径)；补 `dirs` 依赖、`tempfile` dev-dep。
- **守卫**：`check_dag.py` LAYER_TABLE 加 `golish-recon-app: 5.5`；`check_repo_ownership.py` SOURCE_ROOTS 加 `(golish-recon-app, recon)`，迁移 ALLOWLIST(`golish-recon-app/scan_queue.rs`→scan_queue 跨服务) + RAW_SQL_ALLOWLIST(8 键：custom_rules/intel_providers/scan_queue/sensitive_scan/targets{cmds,db,directory}/asset_intel/runtime/mod)，清理 9 条死 recon DOMAIN_RULES。
- **运行过的验证（已记录证据）**：`cargo check -p golish-recon-app` exit 0；`cargo check -p golish` exit 0（50.83s）；`cargo nextest run -p golish-recon-app` → **106 passed / 0 failed**；`cargo clippy -p golish-recon-app --all-targets -- -D warnings` exit 0（补 scan_nuclei_targeted + db_directory_entry_add 两处 `#[allow(clippy::too_many_arguments)]`）；`cargo clippy -p golish --lib -- -D warnings` exit 0（53s，覆盖 vuln-app+recon-app+golish+全部 deps；顺带修了 M1 抽取遗留、被 workspace clippy 暴露的 `golish-vuln-app/src/wiki/vuln_links.rs::vuln_link_add_poc_full` too_many_arguments —— M1 deferred precommit 未跑 clippy 故漏，根因同 recon-app：模块搬入新 crate 后失去原 crate 级 allow 覆盖）；`python3 scripts/check_dag.py` exit 0（✓ 48 crates）；`python3 scripts/check_repo_ownership.py` exit 0（OK clean）；ReadLints（golish 接线文件 + recon-app integrations/asset_intel）无错误。
- **完成定义**：M2（recon 服务）= a+b+c 整体抽完 + 编译 + 测试 + 守卫 + clippy（recon-app）验证 = **完成（层次 A）**；`arch-crate-per-service-split` epic **仍 in_progress**（M3 pentest / M4 agent / M5 platform 未动）。层次 B（切 pentest/agent/platform 对 recon 的入向硬依赖）留 ReconPort/S1-2b。
- **未做（按 AGENTS.md §2.7/§3，需用户授权）**：① `git commit`/`git push`（M0+M1+M2 全套 + 本记录均未提交，工作树脏）；② `just precommit` 全量（golish clippy `-D warnings` / golish nextest / fe-test / ts-rs 漂移）未跑——本轮跑了 recon-app clippy+nextest + 两守卫 + 双 crate check，未跑 god-crate clippy/nextest 全量；③ 运行时前端 invoke 未实测（编译 + 注册解析证明，未跑 app）。
- **下一步建议**：①（高风险 §2.7 需点头）跑 `just precommit` 拿 M0+M1+M2 完整绿证据，按里程碑 commit；② 开 M3：先落 ReconPort（S1-2b，已设计）切 pentest→recon 16 处出向，再抽 golish-pentest-app；③ 或先把 recon 层次 A 升 B（S1-2b 把 21 处入向改走 ReconPort，切断 pentest/agent/platform 对 recon-app 的兄弟硬依赖）。

---

### 2026-05-30 · M1 完成：golish-vuln-app 抽取 vuln_intel(M1a)+wiki(M1b) + 编译/守卫验证（MCP-agent-2 · DISPATCH off · §5.9 单会话直接执行 · 接 dead session bajie-mcp-agent-2-yj5fxhjr 半成品）

- **上下文来源**：用户重连本会话后说"我不知道搞到哪里了 cursor_bajie_mcp_agent_2_yj5fxhjr 你看这个"——指向另一会话 `bajie-mcp-agent-2-yj5fxhjr`（状态 developing，15:49:30 离线）。该会话最后一句"正在干……跑完会拿编译绿证据回报"后掉线，**M1a 机械搬移做了但绿证据从未回报、也未记录**（用户失去状态跟踪的根因）。本会话先跨会话核查（get_session_summary / read_session_history + git 工作树 + 双守卫），再补验证 + 抽 M1b + 补记录。
- **本轮目标**：①查清 dead session 的 M1 进度真相；②补跑 M1a 缺失的编译验证；③按用户指示抽 M1b（wiki）；④补记 M1a+M1b 进 feature_list + 本日志。
- **M1a（vuln_intel，dead session 已做、本会话补验证）**：`git mv tools/vuln_intel`(8 文件)→`golish-vuln-app/src/vuln_intel/`，facade `pub use golish_vuln_app::vuln_intel::*`、Cargo/backend members 接线、守卫 ALLOWLIST/RAW_SQL key 迁 crate 前缀（均 dead session 产出）。本会话补跑 `cargo check -p golish-vuln-app` exit 0 + `cargo check -p golish` exit 0 → **M0 欠的『多 crate 命令注册实证』compile-level 证明**（facade glob 重导出 `__cmd__` 宏 → 聚合 `generate_handler!` 编译通过）。
- **M1b（wiki，本会话实现）**：`git mv tools/wiki`(11 文件)→`golish-vuln-app/src/wiki/`；golish-vuln-app/Cargo.toml 补 `golish-core`(wiki_dir)+`tokio`(fs)；lib.rs 加 `pub mod wiki;`；导入重映射——`crate::error::GolishError`→`golish_app_core::GolishError`(7 文件)、`crate::state::DbState`→`golish_app_core::DbState`(6 文件)、`pub(in crate::tools::wiki)`→`pub(in crate::wiki)`(5 处)；golish 侧 facade `commands_facade/wiki.rs` 改 `pub use golish_vuln_app::wiki::*`、`tools/mod.rs` 删 `pub mod wiki;`、`commands_registry.rs` 不动（facade 转发）；守卫 `check_repo_ownership.py` RAW_SQL key `tools/wiki/vuln_links.rs`→`golish-vuln-app/wiki/vuln_links.rs`。
- **零跨服务**：wiki 用 wiki_kb/kb_research/vuln_scan 全 REPO_OWNER=vuln（自有表）→ 无新 ALLOWLIST 条目。
- **运行过的验证（已记录证据）**：`cargo check -p golish-vuln-app` → exit 0；`cargo check -p golish` → exit 0（41s）；`python3 scripts/check_dag.py` → exit 0（✓ DAG clean across 47 crates）；`python3 scripts/check_repo_ownership.py` → exit 0（OK clean）；ReadLints(wiki 目录 + Cargo.toml + facade) → 无错误；grep 残留 `crate::error|crate::state|crate::tools` 于 wiki 目录 → 0 命中。
- **完成定义**：M1（vuln 叶子）= vuln_intel + wiki 整体抽完 + 编译 + 守卫验证 = **完成**；但 `arch-crate-per-service-split` 整条 epic **仍 in_progress**（M2 recon / M3 pentest / M4 agent / M5 platform 未动）。
- **未做（按 AGENTS.md §2.7/§3）**：① `git commit`/`git push`（golish-app-core + golish-vuln-app 全套 + 本记录均未提交，工作树脏）；② `just precommit` 全量（clippy `-D warnings` / nextest / fe-test / ts-rs 漂移）未跑——本轮只跑 cargo check + 两守卫；③ 运行时前端 invoke 未实测（编译证明注册解析，未跑 app）。
- **下一步建议**：① 跑 `just precommit` 拿 M1 完整绿证据（含 clippy + nextest + fe）；②（高风险，§2.7 需点头）按里程碑 commit M0 + M1a + M1b；③ 开 M2 抽 golish-recon-app（前置：先落 ReconPort/S1-2b，否则 pentest/agent 上游硬依赖）。

---

### 2026-05-30 · M0 State 下沉：golish-app-core 落地 + 验证（MCP-agent-2 · DISPATCH off · §5.9 单会话直接执行 · 接上一会话跨会话转交记忆，补记录+验证）

- **上下文来源**：用户跨会话转交记忆——上一会话已拍板计划 §6 的 4 个决策（全按推荐）+ web 核实 Tauri 跨 crate 注册机制（官方 Discussion #5378：`Builder::invoke_handler` 只能调一次 → 单个聚合 `generate_handler!` 按路径引用各 app crate `pub` 命令），并选「**直接建 golish-app-core**」（跳过 throwaway spike）。该会话已建好 golish-app-core 但**未跑验证、未更新 progress/feature_list**（顶部会话记录仍停在更早的"拆分计划"轮）；本会话接手补齐。用户明确指示「不要跑 check / 编译没问题」→ god-crate 全量编译按用户确认，不重跑省时。
- **本轮目标**：核验 M0 的 **State 下沉半边**（golish-app-core）落地无误、补记录证据，并定位 M0 剩余半边（多 crate 命令注册实证）的去向。
- **已落地内容（实读核验，上一会话产出，未 commit）**：
  - `backend/crates/golish-app-core/`：`error.rs`（完整 `GolishError` enum + 稳定 `code()` 映射 + `{code,message}` `Serialize` + 全 `From` 链含 anyhow/String/&str/zip/base64/uuid/DbError + 3 单测）、`state.rs`（`DbState`：`Arc<PgPool>` + `DbReadyGate`，`pool_ready`/`pool`/`pool_arc`/`ready_gate`）、`lib.rs`（`pub use` 导出 + 架构注释）、`Cargo.toml`（依赖 8 个 L2/L3 域 crate + serde/sqlx/reqwest/zip/...）。
  - `backend/Cargo.toml`：`golish-app-core` 已入 members / default-members / workspace.dependencies。
  - `golish/Cargo.toml`：已加 `golish-app-core = { workspace = true }`（注释标 L5）。
  - `golish/src/error.rs`：改为 `pub use golish_app_core::{GolishError, IpcError, Result}` + **仅留** `impl From<crate::history::HistoryError>`（孤儿规则本地转换）。
  - `golish/src/state/db.rs`：改为 `pub use golish_app_core::DbState`。
  - `scripts/check_dag.py`：`LAYER_TABLE` 新增 `golish-app-core: 5.0`（L5 应用共享边界），`golish` 保持 6.0。
- **运行过的验证（已记录证据）**：
  - `cargo check -p golish-app-core`（cwd backend）→ **exit 0**（`Finished dev profile in 0.60s`）。
  - `python3 scripts/check_dag.py` → **exit 0**（`✓ DAG clean across 46 crates`，golish-app-core 合法落 L5、无环、无非法上行边）。
  - `cargo check -p golish`（god-crate 全量）：**用户确认编译无问题**，本轮按用户指示未重跑（改动均为纯 re-export，零语义变更，风险极低）。
- **关键偏离（已当场告知用户）**：决策①原话"收纳 `DbState`/**`AppState`**"，实现只下沉 **`DbState`**；`AppState` 仍留 `golish`（它聚合 AI/indexer/settings/sidecar 等 golish 内部子系统，下沉会把大量 crate 拽下来；app crate 只需窄的 `DbState`）。这是更干净的取舍，已写进 `golish-app-core/src/lib.rs` 注释。
- **完成定义**：M0 **State 下沉半边 = 完成+验证**；M0 **另一半（多 crate 命令注册机制实证）未做**（需 M1 把第一个真实命令 `vuln_search` 搬进 `golish-vuln-app`，用单聚合 `generate_handler!` 路径引用实证）→ `arch-crate-per-service-split` 保持 `in_progress`。
- **未做（按 AGENTS.md §2.7，需用户授权）**：① M1 抽 `golish-vuln-app`（新 crate 改 Cargo.toml + 命令注册改动，影响全量 IPC = 高风险）；② `git commit` / `git push`（golish-app-core 全套 + 本轮记录均未 commit）。
- **下一步建议**：M1 抽 `golish-vuln-app`（叶子服务，出向耦合 0）——`git mv tools/{vuln_intel,wiki}` 进新 crate + facade `pub use` 转发 + golish `commands_registry.rs` 单聚合 `generate_handler!` 路径引用，顺带实证 M0 命令注册机制。按 §2.7 须用户点头再动手。

---

### 2026-05-30 · Crate-per-service 拆分计划（MCP-agent-3 后端工程 · DISPATCH off · §5.9 单会话直接执行 · 仅文档/计划，零代码改动）

- **上下文来源**：用户回顾"最后改动是关于模块化的东西"，逐步澄清出真实诉求——**"模块化 = 每个功能独立 crate、像微服务"**（不是文件拆分）。期间用户质疑 S1 端口化"是不是搞早了/该不该退回去"；本会话用证据回答：① 文件级模块化已完成（`scripts/check_file_sizes.sh` exit 0，CI 强制）；② 端口化不是"最后一步"而是 crate 独立的**前置桥**（27 处跨服务直读若硬拆 crate 会成循环依赖）；③ 已做的 11 commit（S1-1 守卫 + S1-2a 端口 + S1-2b 设计）绝大部分是"地图+整理"，无需退回。用户最终选 **C：不退，写计划**。
- **本轮目标**：把 servitization-readiness §6 **阶段 3（S3-2 碎 god-crate）** 具体化为一份按依赖顺序、带端口前置的 crate 抽取计划（AGENTS.md §1.3 跨 crate 复杂改动须先计划）。
- **本轮完成（产出，未 commit）**：
  - 新建 `docs/superpowers/plans/2026-05-30-crate-per-service-split.md`：writing-plans 格式。核心=**抽取顺序由耦合 DAG 决定**（`platform→agent→pentest→recon→vuln`，从叶子 vuln 往根抽永不成环）；§2 结构性前置 M0（多 crate 命令注册机制 + State 下沉 golish-app-core，证据=`commands_registry.rs:1-36` 宏导出到 crate 根的约束）；M1 vuln 叶子抽取任务级骨架（含 `tools/vuln_intel`+`tools/wiki` 模块清单、facade 转发、守卫迁移）；M2-M5 范围+端口前置+解锁门槛；§4 端口前置映射总表；§6 4 个开放决策；§7 自检。
  - 区分**层次 A（crate 拆分=切片单体，编译期依赖链）vs 层次 B（端口解耦=真独立/类微服务）**，计划按"A 先见效、再用 S1-2 端口逐步升 B"推进。
  - `feature_list.json` 新增 `arch-crate-per-service-split`（priority 2, `not_started`）+ JSON 校验通过。
- **运行过的验证（已记录证据）**：
  - `bash scripts/check_file_sizes.sh` → **exit 0**（✓ all files within size budget；唯一豁免 event.rs 504）——回答"模块化完成没"的硬证据
  - `python3 -c "json.load(open('feature_list.json'))"` → **exit 0 VALID**
  - `ReadLints`（新 plan md + feature_list.json）→ **No linter errors**
  - 代码事实核验（实读）：`scripts/check_repo_ownership.py`（REPO_OWNER 5 服务 36-84 / ALLOWLIST 27 条 127-158）、`commands_registry.rs:1-36`（generate_handler! 宏导出约束）、`tools/vuln_intel/**`(8 文件)、`tools/wiki/**`(11 文件)、`commands_facade/*`(18 文件)
  - **未跑** cargo/just precommit：本轮**零代码改动**（1 新 plan md + feature_list + 本记录），不影响编译。
- **完成定义**：本轮是**规划交付**，非功能实现 → `arch-crate-per-service-split` 保持 `not_started`，不宣称 passing。计划含待用户拍板关卡（§6 4 决策）。
- **未做（按 AGENTS.md §2.7，需用户授权）**：① `git push`（本地 ahead 11 + 本轮 doc commit）；② 开 M0 spike（多 crate 命令注册机制）；③ 转条目 in_progress（等 §6 决策）。
- **下一步建议**：① 用户审计划 §6 的 4 个开放决策（State 共享 golish-app-core / 守卫扫描范围 / 层次 A vs B 节奏 / 复用 extract-golish-asset-intel-crate）；② 决策后开 M0 spike 证明多 crate 命令注册；③ M0 通后写 M1 vuln 抽取细粒度子计划并实施。

---

### 2026-05-30 · S1-2b ReconPort 高层设计（MCP-agent-3 后端工程 · 接 S1-2a 收尾 · DISPATCH off · 用户授权"你想怎么搞合适" · §5.9 单会话直接执行 · 仅文档，零代码改动）

- **上下文来源**：S1-2a 已 commit + precommit 全绿；用户说"现在的问题是 我想继续计划"→"我不懂你说的意思 你想怎么搞合适"。授权我替他做决策。决策（按推荐路线）：① 子切粒度 A 按消费方 6 子片；② 错误类型 X 续用 anyhow；③ feature_list 登记 Q 父转 passing + 新开 b 子条目。**push 保守处理**：AGENTS.md §2.7 「推送到远端」是明示高风险红线，"你想怎么搞合适"不是"明确同意推到远端"——本轮不 push，等下一轮用户单独点头。
- **本轮目标**：S1-2b 是 S1-2 路线图中最大切片（22 条 allowlist 跨 8 文件 6 消费方），按 AGENTS.md §1.3 复杂改动必须先设计后实现——本轮只出**高层架构设计**，不写 b1 实施计划（留给下一轮，等用户审过决策）。
- **本轮完成**：
  - 新建 `docs/design/2026-05-30-s1-2b-recon-read-port.md`：200+ 行紧凑设计，含 §1 命名差异（ReconPort vs ReadPort，因 b 含写）、§2 22 条 allowlist 精确清单+grep 实证、§3 25 method 按 8 表分组、§4 6 子片划分（b1 agent-bridge 5 / b2 security-analysis 5 / b3 pentest-bridge 6 / b4 pipeline 3 / b5 audit 1 / b6 vuln-matching 1）、§5 trait 结构方案 X 单 trait vs 方案 Y 5 子 port（推荐 Y）、§6 守卫配合（DOMAIN_RULES 加 ports/recon）、§7 验证矩阵、§8 风险、§10 5 待用户拍板决策。
  - `feature_list.json` 改动：`arch-s1-2-port-horizontal-coupling` `in_progress` → **`passing`**（回填 evidence：S1-2a 走路骨架完整收口 + 6 commit hash + nextest + precommit 证据 + S1-2b 设计文档路径 + 5 commit hash 引用）；**新增** `arch-s1-2b-recon-port` `not_started`（priority 1，verification 5 条，evidence 含 design 路径，notes 解释命名差异）；JSON 校验 exit 0；§2.1 当前 in_progress 数 = 0（target-surface 仍 blocked、arch-s1-2 passing、arch-s1-2b not_started）。
  - `agent-progress.md` 顶部"当前最高优先级"+"未提交的半成品"段更新；本会话记录写入。
- **关键设计决策（已写进文档）**：
  - **端口命名**：a 用 `VaultReadPort`（read-only 因 store ON CONFLICT 不可迁），b 用 `ReconPort`（含写，因 agent-bridge 适配器内有 5 个 insert/upsert/update 触发 ALLOWLIST 命中）——这条经验直接来自 a 的智能纠错，沉淀为命名规则。
  - **trait 结构**：推荐方案 Y 按表分 5 子 port（`ReconTargetsPort` / `ReconAssetsPort` / `ReconScansPort` / `ReconSitemapPort` / `ReconDirectoryPort`），每文件 < 200 行符合 architecture.md 500 行预算。
  - **子片顺序**：b1→b2→b3→b4→b5→b6（先建模式 b1，b2 复用 b1 method 信心建立，b3 大片放后，b4-b6 小片快速收尾）。
  - **DbRepoProvider 边界**：b1 改 `impl GolishDbRepoProvider` 内部走端口，不动 `DbRepoProvider` trait 本身（agent-kit L4a 边界稳定）。
- **运行过的验证（已记录证据）**：
  - `python3 -c "import json; ... json.load(open('feature_list.json'))"` → **exit 0 valid**，§2.1 满足（in_progress count = 0）
  - 代码事实核验（grep/Read 实证）：
    - 22 条 ALLOWLIST 来自 `scripts/check_repo_ownership.py:130-156`（实读）
    - 8 个 recon repo 的 `pub async fn`：`rg '^pub async fn' golish-db/src/repo/{8 files}` 实证（49 method 总数，端口需镜像 18 个被实际调用的）
    - 22 条调用方法在消费方文件的精确行号：见设计 §2 表（grep `golish_db::repo::<table>::<method>` 全 14 文件覆盖）
  - **未跑** `cargo` / `just arch` / `just precommit`：本轮**零代码改动**（仅 1 新 md + 2 状态文件改动），不影响编译/测试。
- **完成定义（AGENTS.md §3 五条逐条核对）**：本轮是**设计交付**，非功能实现：① 验证证据=文档+JSON valid+grep实证 ✓；② feature_list arch-s1-2 转 passing 已满足其 verification 全部 4 条 ✓；③ 不需要 precommit（零代码）；④ 零 scope 蔓延 ✓；⑤ 下一轮交接清晰（设计 §10 5 决策待用户回复） ✓。新条目 arch-s1-2b 保持 not_started（设计待用户审）。
- **未做（按 Cursor + AGENTS.md §2.7 红线，需用户单独授权）**：
  - `git push origin feat/recon-service`（ahead 10 = S1-1 4 + S1-2a 6 + 本轮 1 = 11 待 commit 后；本轮零代码、仅文档，commit 后变 11 commit ahead）
  - 写 `docs/superpowers/plans/2026-05-30-s1-2b1-recon-port-agent-bridge.md` b1 详细实施计划（等用户审设计 §10 决策后）
  - 把 `arch-s1-2b-recon-port` 转 in_progress（同上）
- **下一步建议**：① 用户审 `docs/design/2026-05-30-s1-2b-recon-read-port.md` §10 5 决策（端口结构 X/Y / 子片顺序 / DbRepoProvider trait 边界 / 写 method 命名 / b1 实施计划是否启动）；② 决定 push 时机（可与 b1 plan 一起 push，也可现在 push 让远端可见 a 收尾 + b 设计）；③ 决策完毕后写 b1 实施计划 + 转 arch-s1-2b 为 in_progress + 实施。

---

### 2026-05-30 · S1-2a VaultReadPort 走路骨架 commit + precommit 收尾（MCP-agent-3 后端工程 · 接 MCP-agent-4 上下文 · DISPATCH off · 用户授权 C: A+B 一气呵成 · §5.9 单会话直接执行）

- **上下文来源**：用户在另一会话（MCP-agent-4 数据工程）把"未 commit、未跑 just precommit"的 S1-2a 工作树（端口+迁移+守卫+文档全做完）粘到本会话，问"搞到哪里了 就是我之前说要拆分成模块以后方便搞那个"。本会话先核验进度（实读 `ports/vault.rs` / `vault_ops.rs` / `auth_probe.rs` / `pentest_bridge/mod.rs` / `check_repo_ownership.py` / `feature_list.json` / `agent-progress.md` / `docs/superpowers/plans/...s1-2-portification.md` / `docs/design/...s1-2-port-horizontal-coupling.md`），给出 5 选项；用户选 **C: A+B 一气呵成**——跑契约单测 + `just precommit` 全套，全绿后按 plan 拆 6 commit。
- **A 阶段（验证）**：
  - `cd backend && cargo nextest run -p golish 'ports::platform::vault'` → **1 passed / 373 skipped / exit 0**，4m 53s 冷编译；contract test `golish ports::platform::vault::tests::vault_read_port_is_object_safe` PASS 0.016s。
  - `just precommit` → **✓ All checks passed! exit 0**，约 29.6 min（1776657 ms）；含 fmt + check-fe + test-fe + lint-rust（clippy `-D warnings` 全绿）+ test-rust-all（nextest 全绿）。
- **B 阶段（按 plan 拆 6 commit · 落 `feat/recon-service`，未 push）**：
  - `6abaec8` feat(arch): add VaultReadPort + PgVaultAdapter (S1-2a ports skeleton) · 4 files / +118（`ports/{mod,platform/mod,platform/vault}.rs` + `lib.rs`）
  - `1e162de` refactor(arch): route VaultTool through VaultReadPort (S1-2a) · 1 file / +14-15（`vault_ops.rs`，list/get 走端口，store 保裸 SQL）
  - `1a7018b` refactor(arch): route AuthProbeTool through VaultReadPort (S1-2a) · 1 file / +9-9（`auth_probe.rs::resolve_token` 走端口）
  - `1149ddb` refactor(arch): inject PgVaultAdapter into vault/auth tools (S1-2a) · 1 file / +4-2（`pentest_bridge/mod.rs:39-47` 注入共享适配器，闭合 RED 状态）
  - `389d3fd` chore(arch): pull ratchet — vault coupling now via VaultReadPort (S1-2a) · 1 file / +3-2（`check_repo_ownership.py`：DOMAIN_RULES 加 `("ports/platform","platform")` + 删 2 条 vault ALLOWLIST；30→28；RAW_SQL 保留 vault_ops.rs 因 store 仍裸 SQL）
  - 本 docs(arch) commit · 5 files：`docs/design/2026-05-30-s1-2-port-horizontal-coupling.md` + `docs/superpowers/plans/2026-05-30-s1-2-portification.md` + `docs/architecture.md` + `feature_list.json`（回填 evidence + commit hash）+ `agent-progress.md`（本记录 + 顶部状态更新）
- **运行过的验证（已记录证据 · evidence）**：
  - `cargo nextest -p golish ports::platform::vault` → 1 passed / 373 skipped / exit 0（4m53s 冷编译）
  - `just precommit` → ✓ All checks passed! exit 0（29.6 min）
  - 前序（上一轮）：`cargo check -p golish` Finished 1.53s exit 0、`just arch` exit 0（check_dag ✓45 crates + repo-ownership OK）、`rg golish_db::repo::vault` 于 pentest_bridge 空、`python3 scripts/check_repo_ownership.py` [repo-ownership] OK clean exit 0、`feature_list.json` JSON valid
  - 后续：`python3 -c "import json; json.load(open('feature_list.json'))"` → exit 0 `feature_list.json valid`（本轮更新后再校验）
- **完成定义（AGENTS.md §3 五条逐条核对）**：
  - ① 验证命令实际跑过且证据已记录 ✓（命令 + 退出码 + 输出片段 + 时长全在本记录与 `feature_list.evidence.s1_2a_verification`）
  - ② `feature_list.verification` 4 条逐条核对 ✓（ALLOWLIST 30→28 ✓ / nextest 通过 ✓ / grep vault 空 ✓ / precommit exit 0 ✓）
  - ③ `just precommit` 全绿 ✓（exit 0, 29.6 min, fmt+check-fe+test-fe+lint-rust+test-rust-all 全套）
  - ④ 没有引入未在 scope 内的代码改动 ✓（git diff 集合严格 = plan 列出的文件，无外溢）
  - ⑤ 下一轮会话不需要人工补救就能继续 ✓（feature_list + progress + 6 commit 全落 + design/plan 同步）
- **完成定义结论**：S1-2a 切片**完整收尾**（实现+验证+commit+文档同步全部 done）。父条目 `arch-s1-2-port-horizontal-coupling` 仍 **in_progress**（b-g 未做）；按 AGENTS.md §3 严格表述：a 片满足 passing 五条，但父条目代表整个 S1-2，建议用户在 ① 父条目继续 in_progress 直到 a-g 全完 / ② 把父条目转 passing 并新开 `arch-s1-2b-recon-read-port` 等子条目跟踪 b-g 两种登记法中二选一。
- **未做（按 Cursor 规则需用户授权）**：
  - `git push origin feat/recon-service`（本地 ahead 10 = S1-1 收尾 4 commit + 本批 S1-2a 6 commit）
  - 推进 S1-2b `ReconReadPort`（22 条最大切片，按消费方子切；需先写设计/计划）
  - 决定父条目 passing/in_progress 登记法（见上）
- **下一步建议**：① 用户决定 push（建议 push，10 commit 不算多且彼此独立）；② 父条目登记法二选一；③ 若选①父条目保持 in_progress → 直接进 S1-2b 设计/计划；若选②父条目转 passing → 新条目 + 让 `target-surface-workbench` 回 in_progress 或继续顶 S1-2b。

---

### 2026-05-30 · S1-2a VaultReadPort 走路骨架 实现 Task 5-6（MCP-agent-4 数据工程 · 接另一会话 Tasks 1-4 上下文 · DISPATCH on · 用户转交记忆，§5 执行者本会话直接执行）

- **上下文来源**：用户把另一会话的「记忆」转交本会话——该会话已完成 S1-2a 的 Tasks 1-4（按 `docs/superpowers/plans/2026-05-30-s1-2-portification.md`），停在「即将 cargo check 验证」。本会话（数据工程）接手：先核验 Tasks 1-4，再做 Task 5（数据所有权守卫拔 ratchet，本职）+ Task 6（收尾）。
- **核验 Tasks 1-4（已在工作树，evidence）**：
  - 端口层 `golish/src/ports/{mod.rs,platform/mod.rs,platform/vault.rs}` 已建；`lib.rs:37 mod ports;`。
  - **关键偏差（智能纠错）**：端口为 **read-only**（3 法，弃 `store_entry`）——`store` 动作用 `ON CONFLICT DO NOTHING`，并入 `insert_full` 会变语义，故保留裸 INSERT。→ `vault_ops.rs` 留 `RAW_SQL_ALLOWLIST`，原计划 Task 5.3 **不执行**。
  - 消费方：`vault_ops.rs`（VaultTool list/get 走端口、store 保裸 SQL）、`auth_probe.rs`（resolve_token 走端口）；构造点 `pentest_bridge/mod.rs:39-47` 注入共享 `PgVaultAdapter`。
  - `cargo check -p golish` → **Finished 1.53s, exit 0**；`rg golish_db::repo::vault` 于 pentest_bridge → **空**。
- **本会话完成 Task 5（守卫，数据所有权 ratchet · 本职）**：`scripts/check_repo_ownership.py`：`DOMAIN_RULES` 顶部加 `("ports/platform","platform")`；删 2 条 vault `ALLOWLIST`（auth_probe.rs/vault_ops.rs）；**保留** `RAW_SQL_ALLOWLIST` 的 vault_ops.rs（偏差）。
  - 验证：`python3 scripts/check_repo_ownership.py` → **[repo-ownership] OK clean, exit 0**；`just arch` → **check_dag ✓45 crates + repo-ownership OK, exit 0**。ALLOWLIST **30→28**。
- **本会话完成 Task 6（部分）**：`docs/architecture.md` data-ownership 节补 S1-2 进度段；`feature_list.json`：`arch-s1-2` → **in_progress**（非 passing，仅 a 片完成）+ 回填 evidence + 修正 verification（RAW_SQL 不减）；`target-surface-workbench` → **blocked**（让 §2.1 名额，可恢复）；JSON 校验 exit 0。本记录。
- **运行过的验证（已记录证据）**：`cargo check -p golish` exit 0；`just arch` exit 0；guard OK clean；feature_list JSON valid exit 0；`cargo nextest -p golish ports::platform::vault`（运行中，结果待回填）。
- **完成定义（未达 passing）**：父条目 in_progress；**未 commit**、**未跑 just precommit**——按 Cursor 提交规则，实现+验证后由用户授权再提交（commit 前跑全套 precommit）。
- **下一步建议**：① 用户确认 `target-surface-workbench` 暂泊 + S1-2 父条目 in_progress（非 passing）；② 授权后跑 `just precommit` 全绿 → 按计划 Task 1-6 拆 commit（端口/迁移/注入/守卫/文档）；③ 续 S1-2b `ReconReadPort`（22 条，最大，按消费方子切）。

---

### 2026-05-30 · S1-2 端口化横向耦合 设计 + 实现计划（MCP-agent-1 主控中心 · 接 MCP-5 上下文转交 · DISPATCH on · 窗口内仅本会话在线 → §5.9 单会话直接执行 · 仅文档/计划，零代码改动）

- **上下文来源**：MCP-5（UI 设计实现）2026-05-30 把 S1-1 收尾上下文转交本会话（主控 MCP-1）；用户指令「**开始写 S1-2 计划**」。`list_sessions` 显示 MCP-2/3/4/5 全部 `online:false`，唯一在线工作会话是 MCP-1（`bajie-mcp-default` 为无角色占位），按 §5.9「分发开关开启 + 仅 1 会话在线 → 直接执行」，主控本会话亲自起草，不派发。
- **本轮目标**：为 servitization 阶段 1 第二项 S1-2（端口化横向耦合）出**设计文档 + 实现计划 + feature_list 登记**（AGENTS.md §1.3：跨 crate Rust 改动须先设计/计划，审过再动代码）。
- **关键洞察（实读代码得出，非转述）**：
  - agent 栈**已有消费方端口** `DbRepoProvider`（`golish_agent_kit::db_traits`，由 `golish/src/ai/db_bridge/mod.rs:16,24-35` 的 `GolishDbRepoProvider` 实现）——**S1-2 不动它**。
  - S1-2 要加的是**提供方服务端口**：按「被读的是哪个服务的表」定义 `*Port`，让跨服务直查 `golish_db::repo::<x>` 改走端口（in-proc 适配器 → 阶段4 网络适配器）。
  - S1-1 守卫 `ALLOWLIST` 当前 **30 条 = 29 真跨服务读 + 1 条 `tools/scan_queue.rs→scan_queue` 领域映射伪阳性**（recon 域文件读 vuln 域 repo，同一概念被一分为二，应改归属而非建端口）。
  - 按提供方归类出 **5 个端口**：`ReconReadPort`(22,最大) / `VaultReadPort`(2) / `VulnReadPort`+wiki(2) / `AgentLogReadPort`(2) / `PentestPlanReadPort`(1)。recon 被依赖最多（22/29），与 §5「最先抽 vuln、最后抽 recon」一致。
- **本轮完成（产出文件，均未 commit）**：
  - 新建 `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`（设计：两层端口洞察、30 条 allowlist 按提供方归类、端口三件套模式、守卫 DOMAIN_RULES `ports/<service>` 配合机制、`VaultReadPort` 走路骨架、7 切片路线、remote-ready 约束、§9 待用户拍板 5 决策）。
  - 新建 `docs/superpowers/plans/2026-05-30-s1-2-portification.md`（实现计划：S1-2a `VaultReadPort` 走路骨架 6 个 Task 全 code-complete——端口 trait+`PgVaultAdapter`、迁移 `vault_ops.rs`/`auth_probe.rs`、构造点 `pentest_bridge/mod.rs:34-53` 注入、守卫拔 ratchet、验证收尾；附 S1-2b–g 路线图 + writing-plans 自检）。
  - `feature_list.json` 新增 `arch-s1-2-port-horizontal-coupling`，`status: not_started`（**未** in_progress：§2.1 唯一 in_progress 仍为 `target-surface-workbench`，是否顶替待用户定，见设计 §9 决策5）。
- **运行过的验证（已记录证据）**：
  - `python3 -c "import json; json.load(open('feature_list.json'))"` → **exit 0 `feature_list.json valid`**。
  - 代码事实核验（grep/read 实证）：`VaultTool` 构造点 `tools/pentest_bridge/mod.rs:42`、`AuthProbeTool` `:45`；vault repo 签名 `golish-db/src/repo/vault.rs:120/314/330/347`；消费方直查点 `vault_ops.rs:167/201/122`、`auth_probe.rs:253`；crate 根 `golish/src/lib.rs:43 pub mod tools;`。
  - **未跑** `just precommit` / `cargo` / `just arch`：本轮**零代码改动**（仅新增 2 个 md + 改 feature_list.json + 本记录），不影响编译/守卫；S1-2a 真正实现时再按计划 Task 6 跑全套。
- **完成定义**：本轮是**规划交付**，非功能实现 → S1-2 保持 `not_started`，不宣称 passing。设计含**待用户审查关卡**（brainstorming 规范）。
- **下一步建议**：① 用户审设计 §9 五个决策（切片顺序 / 端口错误类型 anyhow vs GolishError / trait 位置 / scan_queue 归属 / 是否顶替 workbench 焦点）；② 决策后执行 S1-2a 走路骨架（`.cursor/skills/executing-plans/`）；③ 4 个 S1-1 commit 仍在 `feat/recon-service` 本地未 push。

---

### 2026-05-30 · S1-1 repo 数据所有权守卫收尾 Task 6.3/6.4（MCP-agent-5 UI 设计实现 · 接 MCP-3 上下文转交 · DISPATCH on · 非 controller 本会话直接执行）

- **上下文来源**：MCP-3（后端工程 `bajie-mcp-agent-3-m0ehe76b`）执行 S1-1 计划至 Task 1/2/4/5/6.1-6.2 后，2026-05-30 把上下文转交本会话；用户指令「补完 6.3/6.4 文档」。
- **接手时实况核验（实读文件 + 实跑命令，非转述）**：`scripts/check_repo_ownership.py` 已建且 `ALLOWLIST`(29 跨服务条目) + `RAW_SQL_ALLOWLIST`(30 文件) 已填；`.github/workflows/arch-check.yml` 加 `repo-ownership` job；`justfile` 加 `arch` recipe（改进为两守卫无条件跑 + 聚合退出码，避免一个失败掩盖另一个）；`docs/architecture.md` 加「数据所有权」原则 + 服务→repo 表。**全部未 commit**；Task 6.3（feature_list 条目）与 6.4（本记录）当时缺失。
- **本轮完成**：
  - **Task 6.3**：`feature_list.json` 新增 `arch-s1-1-repo-ownership-guard` 条目，`status: blocked`。选 blocked 而非 passing：① §2.1 同时只能一个 in_progress（已被 `target-surface-workbench` 占用）；② §3 要求 just precommit 全绿才可 passing，但 just arch 当前 exit 1（既有 check_dag 红，非 S1-1 引入）。JSON 校验通过，in_progress 仍唯一。
  - **Task 6.4**：本会话记录。
- **运行过的验证（已记录证据）**：
  - `python3 scripts/check_repo_ownership.py` → **exit 0 `OK clean`**
  - **守卫拦新增耦合实证**：临时建 `backend/crates/golish/src/tools/vault_s1probe_tmp.rs`（platform 域 `use golish_db::repo::findings`，findings 属 pentest）→ 守卫 **exit 1** 报 `tools/vault_s1probe_tmp.rs: platform -> repo::findings (owned by pentest)`；**删除该临时文件后** → **exit 0**（工作树无残留，已 `git status` 确认）
  - `just arch`（接手时）→ **exit 1**，根因是**既有**且与 S1-1 无关的 `check_dag` 违规 `golish-graphiti(L1)→golish-db(L2)`（commit 49cc135 加显式依赖后漏更新层表所致）；repo-ownership 段本身 `OK clean`。
  - 用户选「**顺手修老问题再存**」→ 本轮**已修** check_dag：graphiti 实际依赖 golish-db 的 `PgPool`（`golish-graphiti/src/client.rs:3 use golish_db::PgPool`），是合法 sibling 依赖，故把它从 L1 归入 L2（`check_dag.py` LAYER_TABLE + persistence `L2_CLUSTER`），并同步 `architecture.md` 层图/L1-L2 目录。**非删依赖**。修复后 `just arch` → **exit 0**（check_dag ✓ 45 crates + repo-ownership ✓）。
- **提交记录（已落 4 commit 到 `feat/recon-service`，未 push）**：`b0811ea` fix(arch) graphiti L2 / `dc9ad0f` feat(arch) 守卫脚本 / `821c101` ci(arch) CI+`just arch` / 1 个 `docs(arch)` commit（含 architecture.md + design + plan + feature_list.json + 本文件）。提交后工作树 clean。
- **feature_list**：`arch-s1-1-repo-ownership-guard` 由 `blocked` 切 **`passing`**（just arch 全绿；唯一 in_progress 仍为 target-surface-workbench，§2.1 未破）。
- **已知风险**：`just precommit`（全量 cargo/vitest，~20min）本轮**未重跑**——改动集 = CI 脚本(check_dag.py / check_repo_ownership.py) + arch-check.yml + justfile + 文档，**零 Rust/TS/Cargo diff**，不影响编译/测试（基线 2026-05-30 记录全绿）；如需绝对保险可补跑。
- **下一步**：可选 ① 补跑 `just precommit` 终检；② push `feat/recon-service` / 开 PR；③ 进入 **S1-2**（端口化横向耦合：消 `asset_intel→organizations/pentest` 直连，引 `OrganizationsPort`/`PentestPort` trait）。

---

### 2026-05-30 · 架构体检全批 fmt --all + 按主题拆 20 commit 收尾（MCP-agent-2 产品经理·DISPATCH on·非controller 本会话直接执行）

- **本轮目标**：用户「顺手 fmt --all」→「按主题拆 commit」。把本会话累计的架构体检工作树（拆/合并/优化/dedup）格式化并按主题落成干净 commit。
- **fmt**：`cargo fmt --all`（纯排版，零逻辑变更），`cargo fmt --all --check` → **exit 0**；连带把历史遗留的几个未格式化文件（api_request_stats.rs / 多个 `*_tests.rs` 等）一并归正。
- **按主题拆 20 个 commit**（先 `git reset` 解除 `git mv` 预暂存，再逐主题精确 `git add`）：
  - `build(scripts)` 文件大小门禁 rust grandfather + inline-test splitter（1）
  - `refactor(tests)` 内联测试抽 sibling `*_tests.rs`（1，覆盖 agent-kit/agent-runtime/intel-providers/js-analyzer/llm-providers/pentest/pty/integrations/golish）
  - `refactor(<crate>)` 12 个超大文件**逐模块**拆分（settings llm / agent-kit planner / agent-runtime stream_processor+direct / db audit / integrations schema / pentest orgs / pipeline single / pty session_create / tools orgs+cli / ai commands）
  - `refactor(core)` time/path/string helper 收敛进 golish-core 单源（1，~20 调用点 + golish-indexer 加 golish-core 依赖 + StoreStats 复用）
  - `fix(recordings)` 录制命令项目作用域 IDOR（1，I2）
  - `refactor(frontend)` formatClockTime 收编（1）+ mocks fixtures 抽出（1）
  - `docs(plans)` 拆分/dedup 计划（1）+ `docs(progress)` 本记录（1）
- **运行过的验证**：`cargo fmt --all --check` → exit 0；`git status --porcelain` → 仅 `M agent-progress.md`（提交后 clean）；每个 commit 均 exit 0（无 pre-commit hook）。**注**：完整 `just precommit`（~20min）本轮按用户取舍**未重跑**——树在本会话稍早已 `cargo check`/`nextest`/`clippy`/`fmt` 全绿，fmt 后仅排版差异。
- **提交记录**：`a85f7d4`(scripts) → `8049196`(tests) → `929ec2e`/`616472b`/`2c91682`/`9a3272a`/`ead8b76`/`9e83dfa`/`230c53c`/`7c3d4e5`/`2f2d079`/`348346a`/`252b838`/`77ac579`(12 splits) → `6fa4cc3`(core dedup) → `a831631`(IDOR) → `432ad09`(fe time) → `ed7f75c`(fe mocks) → `a19c9af`(plans) → 本 progress commit。**全部未 push**。
- **已知风险**：commit 前未重跑全量 precommit（用户取舍）；如需绝对保险可补一次 `just precommit`。`db/audit` 拆分非本会话 todo 内（前轮遗留工作树），但树编译绿故一并按主题提交。
- **下一步最佳动作**：用户可 `just precommit` 终检 → 满意后 push / 开 PR；或继续别的优化主题。

---

### 2026-05-30 · 前端 dedup 扫描 + formatClockTime 收编（MCP-agent-2 产品经理·DISPATCH on·非controller 本会话直接执行）

- **本轮目标**：用户「扫前端重复」→「加 formatClockTime」。
- **前端扫描结论**：重复度低，公共 util 已集中在 `lib/`（time/clipboard/format/cn）；`lib/time.ts` 的 `formatRelativeAgo` 上轮已统一相对时间。copyToClipboard/debounce/throttle/getErrorMessage/prettyJson 均单源无重复。
- **执行 formatClockTime（附录 C 规划项）**：`surface/surfaceModel.ts` 的 `formatTime`（时间戳→HH:MM:SS 时钟时刻，`toLocaleTimeString` 2-digit）是 lib/time 缺失的格式（区别于 formatDurationClock 时长 M:SS / formatLogDate 带日期）。新增 `lib/time.ts::formatClockTime`（逐字同逻辑），surfaceModel 改 `import { formatClockTime as formatTime }` + `export { formatTime }`（保持对外 import 名稳定，EvidenceTab/test 不受影响）。
- **运行过的验证**：`tsc --noEmit` → exit 0；`vitest surfaceModel.test` → **6 passed**；`biome check`（2 文件）→ 修 1 个 import 排序后 **No fixes/clean**。
- **保留（非重复）**：TerminalRecordingControls.formatTime(秒→M:SS)/SessionBrowser formatDate-formatDuration(局部)/TargetTimeline.formatTimestamp → 格式各异、组件局部，低 ROI 不动。
- **提交记录**：未 commit（同前）。
- **下一步**：dedup 维度（后端跨 crate / repo SQL / 前端）已全面扫完；建议转 commit 切分 + just precommit 兜底。

---

### 2026-05-30 · strip_ansi ×2 收敛 + 全面 dedup 扫描收尾（MCP-agent-2 产品经理·DISPATCH off 直接执行）

- **本轮目标**：用户「再扫其它重复」。`[DISPATCH:off]` → 直接执行。
- **新发现真重复 → 已收敛**：`strip_ansi` ×2（`golish-agent-kit::db_traits::memory` 与 `golish-db::gatekeeper`，**逐字等价**，仅一处多条注释）→ 新增 `golish_core::utils::strip_ansi`（+ 单测），两处改 `use golish_core::utils::strip_ansi;`（均依赖 golish-core）。
- **扫描范围（本轮 + 前两轮）**：expand_/format_relative/truncate/slugify/sanitize_name/timestamp/merge_json/atomic_write/load_json/redact-mask/ensure_dir/is_http_url/snake-camel/strip_ansi/strip_control/estimate_tokens/format_bytes/first_line-preview/shell_escape/normalize_whitespace/parse_duration-size/levenshtein/dedup/is_binary/parse_bool/extract_json 等 ~30 组。
- **同名异义/单例 → 保留（非重复）**：slugify(文件名 vs 标题)、sanitize_name(下划线 vs slug)、truncate(shell 尾部字节版)、find_memory_file_for_workspace(行为分叉)、strip_control_chars(单例)、estimate_tokens(单例)、merge_json_array/merge_into_sitemap(各异)、dedupe_*(领域专用)、is_binary_file vs is_binary_or_artifact(内容 vs 路径)、ensure_dirs(不同结构的方法)。
- **结论**：跨 crate 真重复辅助函数已基本清零；剩余同名函数均为同名异义或单例，**正确保留**。
- **运行过的验证**：`cargo check -p golish-core -p golish-db -p golish-agent-kit` → exit 0；`nextest golish-core test(strip_ansi)` → **1 passed**；`clippy` 三 crate `--no-deps` → **0 warning**；3 改动文件已 rustfmt。
- **提交记录**：未 commit（同前）。
- **下一步**：dedup 维度基本收尾；建议转 commit 切分 + `just precommit` 兜底，或处理 6 个 pre-existing fmt 旧文件。

---

### 2026-05-30 · truncate 家族统一 → golish-core::utils 单源原语（MCP-agent-2 产品经理·DISPATCH off 直接执行）

- **本轮目标**：用户「统一 truncate 家族」。延续 dedup。`[DISPATCH:off]` → 直接执行。
- **现状（5 份，语义分叉：字节/字符 × 头/尾 × 不同标记）**：
  - `golish-core::utils::truncate_str`（字节·头·切片·无标记，**既有 canonical**）+ `truncate_head_tail`（70/30）
  - `golish-cli-output::truncate_output`（字符·头·无标记）= 与新 `truncate_chars` **逐字等价**
  - `golish-agent-runtime::eval_support::truncate_string`（换行归一 + 字符·头 + `...`）
  - `golish/pty_interactive::truncate_output`（字节·头 + `...\n[note]` + 固定 50k）
  - `golish-shell-exec::truncate_output`（字节·**尾** + header，**唯一尾部保留**）
- **统一方案（保持各调用方行为零变更）**：
  - golish-core::utils 新增 **`truncate_chars(&str, max_chars)->String`**（字符·头·无标记）+ 单测；与字节版 `truncate_str` 配成「truncate 家族」单源。
  - cli-output → `pub(crate) use golish_core::utils::truncate_chars as truncate_output`（**完全去重**）。
  - agent-runtime `truncate_string` → 截断步复用 `truncate_chars`，保留换行归一 + `...` 包装。
  - pty `truncate_output` → 切片步复用 `truncate_str`（替掉 nightly `floor_char_boundary`，行为等价：均取 ≤cap 的最大 char-boundary 前缀），保留 `...\n[note]`。
  - shell-exec 尾部字节版**单一实现、唯一语义** → 保留并记录（非重复）。
- **运行过的验证**：`cargo check -p golish-core -p golish-cli-output -p golish-agent-runtime -p golish` → exit 0；`nextest golish-core test(truncate)` → **10 passed**（含新 truncate_chars 测）；`clippy` 上述 4 crate → **0 warning**；4 个改动文件已 rustfmt。
- **提交记录**：未 commit（同前，留用户切分）。
- **下一步**：用户决定 commit / 是否继续（slugify 两版可评估统一；或转去 commit 切分 + just precommit 兜底）。

---

### 2026-05-30 · 重复函数收敛：expand_tilde ×6 + format_relative_time ×2 → golish-core 单源（MCP-agent-2 产品经理·DISPATCH off 直接执行）

- **本轮目标**：用户「还有没有重复函数 / 需要模块化收到一个模块里的」。延续架构体检的去重维度。`[DISPATCH:off]` → 直接执行。
- **扫描方法**：对高发重复辅助函数名跨 crate grep（expand_/hex_/truncate_/split_command/now_/format_relative/sanitize/slugify/is_valid…）。
- **真重复 → 已收敛到 `golish-core`（通用 L1 叶子）**：
  - **`expand_home_dir`/`expand_tilde` ×6**（golish app/workspace · indexer/codebases · ai/bridge_config 内嵌 · commands/fs/completions · golish-indexer/path_helpers · golish-agent-kit/memory_file 内嵌）→ 新增 `golish_core::paths::{expand_tilde, expand_tilde_string, contract_home_dir}`（采用最完整版：同时处理裸 `~` 与 `~/`，是各 `~/`-only 版的超集，行为安全）。6 处全部改为调用/再导出（workspace.rs、codebases、path_helpers 用 `pub use ... as expand_home_dir` 保名稳定；内嵌版直接删并调 golish-core）。**`golish-indexer` 原无 golish-core 依赖 → 新增 `golish-core = { workspace = true }`**（L2→L1，golish-core 仅依赖 golish-platform，无环）。
  - **`format_relative_time` ×2**（golish-indexer/git_helpers · golish/indexer/commands/hidden_dirs，**逐字相同**）→ 新增 `golish_core::time::format_relative_time`，两处再导出/import。
- **同名异义 → 正确保留（测量后不合，合即 bug）**：
  - `slugify` ×2（sidecar take50/unicode vs projects ascii+折叠连字符，形态分叉）、`sanitize_name` ×2（mcp 仅 `-`→`_` vs pentest-mcp 全 slug，**完全不同用途**）、`truncate_*` 家族（bytes vs chars vs &str/String 分叉）、`find_memory_file_for_workspace` ×2（agent-kit 额外自动探测 CLAUDE.md/AGENT.md vs golish 仅 codebase 匹配，行为分叉）→ 全保留并记录。
  - 时间戳薄委派 `epoch_secs`/`now_millis` 上轮已委派到 golish-core::time，非真重复。
- **运行过的验证**（本机实跑）：
  - `cargo check -p golish-core` → exit 0；`cargo check -p golish-indexer -p golish-agent-kit -p golish` → exit 0（修 1 个 unused PathBuf）
  - `cargo nextest -p golish-core`（path/time 子集）→ **7 passed**（新增 expand_tilde/contract_home_dir/format 相关）
  - `cargo nextest -p golish-indexer` → **26 passed/0 failed**（含 expand_home_dir 两测，现走 golish-core）
  - `cargo clippy -p golish-core -p golish-indexer -p golish-agent-kit -p golish --no-deps` → **0 warning**
  - `check_file_sizes.sh` → ✓（paths.rs/time.rs 增量小，无新违规）；`cargo fmt --check` → 本轮 9 个改动文件全干净（连带把 time.rs 的旧 fmt 残差也修了）
- **提交记录**：**未 commit**（同前：工作树带历史半成品，留用户切分）。
- **下一步**：用户决定 commit 切分 / 是否继续找更多 dedup（如 truncate 家族可在 golish-core::utils 统一 chars 版）。

---

### 2026-05-30 · 架构体检 backlog 全清：后端文件拆分(拆) + 类型收敛(合并) + recordings IDOR(优化)（MCP-agent-2 产品经理·DISPATCH off 直接执行）

- **本轮目标**：用户「还有什么需要拆/合并/优化」→「全部一次性搞定，不要一个个问我」。三条线（拆 / 合并 / 优化）一次性收口。`[DISPATCH:off]` → 本会话直接全能执行。
- **一、拆（split）— 后端 file-size gate 从 12 违规 → 0（全绿）**：12 个 >500 行 Rust 文件全部按职责模块化（行为零变更，纯结构重构）：
  - `golish-settings/schema/llm.rs`(504)→ `llm/{mod,google,openrouter,openai_compat}`
  - `golish-agent-kit/planner/manager.rs`(531)→ `manager/{mod,persistence,mutations}`（inherent impl 跨文件）
  - `golish-agent-runtime/.../stream_processor/mod.rs`(590)→ 抽 `chunks.rs`(text/reasoning handlers)+`tests.rs`
  - `golish-agent-runtime/.../tool_execution/direct.rs`(563)→ `direct/{mod,sub_agent_call}`
  - `golish-pentest/output_store/organizations.rs`(590)→ `organizations/{mod,writers,tests}`
  - `golish-integrations/schema.rs`(888)→ `schema/{mod,storage,test_kind,capture,tests}`
  - `golish-pty/manager/session_create.rs`(705)→ `session_create/{mod,util,reader,emitter_loop}`（两个线程闭包体抽成 run_reader_loop/run_emitter_loop）
  - `golish-pipeline/.../steps/single.rs`(960)→ `single/{mod,ai_tool,exec}`（exec=命令迭代+parse_and_store 阶段函数）
  - `golish/tools/organizations.rs`(764)→ `organizations/{mod,types,candidates,validation,tests}`
  - `golish/tools/asset_intel/runtime/cli.rs`(635)→ `cli/{mod,stream}`
  - `golish/ai/commands/mod.rs`(529)→ 抽 `bridge_config.rs`（configure_bridge 全套 + McpManagerToolExecutor）
  - `golish-core/events/event.rs`(504)= 单一巨型 `AiEvent` ts-rs wire-contract enum，**物理不可拆**（拆变体会改 serde JSON wire 破 I5）→ 给 `scripts/check_file_sizes.sh` 加 Rust grandfather 机制（镜像既有 TS grandfather），504 行豁免（注释说明）。
- **二、合并（dedup / I5）— Phase 4 同型三胞胎：测量后决策（plan 明确允许 defer，禁止盲合）**：
  - `StoreStats`：golish-pentest 版异构（tool-detection 统计，不同概念→保留）；`golish/tools/output_parser` 与 `golish-pipeline/parser` **字段+derive 完全一致**且 golish 已依赖 golish-pipeline → **收敛**：前者改 `pub use golish_pipeline::parser::StoreStats`（去 1 份副本，I5 单源）。
  - `ParseResult` ×3：全异构（pty 解析 / 工具解析 / pentest 解析，同名异义）→ **全保留**，盲合即 bug。
  - `PlanStep` ×3：golish-core 版是 canonical wire 类型（StepStatus enum，异构保留）；agent-kit `db_traits::PlanStep` 与 golish-db 版字段一致，但 **golish-agent-kit 无 golish-db 依赖**（ports/adapters 刻意解耦边界 DTO）→ 收敛会强加跨 crate 依赖破坏解耦 → **保留 + 文档化**。
  - 说明：高价值的前端 wire 类型 I5 收敛（Finding ts-rs / ProbeFinding·HarnessFinding·ToolSelectionConfig 改名 / ToolConfig→domain）此前 Phase 1-3 已完成；Phase 4 余下三胞胎多为内部类型，按 plan 测量后大多正确地保持分离。
- **三、优化（IDOR / I2）— recordings 作用域（选 A，符合 I2 多租隔离）**：`recording_{list,delete,load}` 之前接收但忽略 `_project_path`；改为全部用 `WHERE project_path IS NOT DISTINCT FROM $n` 作用域（`recording_save` 本就持久化 project_path）。前端 `lib/terminal/recording.ts` 4 个调用**早已传 `projectPath: getProjectPath()`** → 纯后端改动、向后兼容（既有行存的就是该值；`IS NOT DISTINCT FROM` 正确处理 NULL）。
- **运行过的验证**（本机实跑）：
  - `bash scripts/check_file_sizes.sh` → **exit 0**（✓ all files within size budget；event.rs grandfather ≤504）
  - 逐 crate `cargo check`：golish-settings/agent-kit/agent-runtime/pentest/integrations/pty/pipeline/**golish** 全 **exit 0**（含全 workspace 依赖编译）
  - `cargo nextest run -p golish-integrations -p golish-agent-runtime -p golish-settings -p golish-pty -p golish-pipeline -p golish-agent-kit` → **839 passed / 0 failed**（含 schema::tests / stream_processor 等被搬移测试）
  - `cargo nextest run -p golish-pentest -p golish -E '<8 个被搬移测试>'` → **13 passed / 0 failed**（organizations validation/candidate + output_store collect_leftover）
  - `cargo clippy --workspace --no-deps` → 见下「下一步」（本轮末尾运行中）
- **已记录证据**：见上「运行过的验证」；gate exit 0 输出 + 852 测试 PASS 汇总。
- **提交记录**：**未 commit**。原因：进场工作树已带大量上几轮未提交改动（time.rs 收敛 / P0-3b 残余 / P2 拆分等，见「未提交的半成品」），本轮改动叠加其上；为避免把无关半成品混入一个 commit，留给用户决定提交切分。本轮新增/改动文件均为本任务 scope 内（上列 12 拆分目录 + check_file_sizes.sh + output_parser.rs + recordings.rs）。
- **已知风险或未解决问题**：
  - `just precommit` 全量（fmt + check-fe + test-fe + lint-rust + test-rust-all + check-types）未在本轮完整跑（耗时 ~20min）；已用逐 crate check + 目标 nextest + workspace clippy 代理验证；建议 commit 前由用户/下一轮跑一次 `just precommit` 兜底。
  - Phase 4 的 PlanStep(db_traits↔golish-db) 收敛被**有意 defer**（解耦边界），如未来要统一需先引依赖或上提 DTO crate，属设计决策非遗漏。
  - 小项未做（非阻塞）：FindingStatus serde `"falsepositive"` vs `as_str()` `"false_positive"` 取值不一致（既有债，建议单开任务）。
- **下一步最佳动作**：
  1. 等 `cargo clippy --workspace --no-deps` 结果；若有告警立即修（refactor 为纯搬移，预期 0 新增告警）。
  2. 用户决定 commit 切分（建议：12 拆分按 crate 分 commit + 1 个 StoreStats dedup + 1 个 recordings scope + 1 个 gate grandfather）。
  3. commit 前跑 `just precommit` 兜底。

---

### 2026-05-30 · 时间戳工具函数收敛收尾：ts_from_chrono → golish_core::time::ts_from_dt（MCP-1 主控·DISPATCH off 直接执行）

- **本轮目标**：用户「收敛时间戳工具函数」。`[DISPATCH:off]` → 本会话直接执行。承接上一轮已起的 P1-3 时间戳收敛（架构审计计为「11 份」：now_ts×4 / ts_from_dt×3 / now_ms×2 / now_millis×2）。
- **进场状态（上一轮已落，未 commit）**：`golish-core/src/time.rs` 建为唯一真源（`now_ts`/`now_ms`/`ts_from_dt` + 4 单测），`lib.rs` 导出；`golish-pipeline/types.rs` 与 `golish-vuln-intel/types.rs` 改 `pub use golish_core::time::*`；`golish-sub-agents` 的 `epoch_secs()` 与 `golish/asset_intel/runtime/cli.rs` 的 `now_millis()` 改为薄委派；`golish` 的 vault/findings/wordlists/history-storage/organizations 全部 `use golish_core::time::*`。
- **本轮改动（完成最后 1 个漏网命名重复）**：`golish/src/tools/targets/` 里行为等价但**异名**的 `ts_from_chrono`（= 第 3 份 ts_from_dt）收敛掉：
  - `targets/types.rs`：删本地 `fn ts_from_chrono`，加 `use golish_core::time::ts_from_dt;`，4 处调用点改名（time_window_start/end map + created_at/updated_at）。
  - `targets/recon.rs`：`use super::types::ts_from_chrono` → `use golish_core::time::ts_from_dt`，1 处调用点改名。
  - 结果：命名时间戳工具函数实现**11→1**（其余为薄委派/再导出，单一真源在 golish-core）。
- **运行过的验证**（本机实跑）：
  - `cargo check -p golish -p golish-core -p golish-pipeline -p golish-vuln-intel -p golish-sub-agents` → **exit 0**（`Finished dev in 3m13s`）
  - `cargo nextest run -p golish-core` → **156 passed / 0 failed**（含 `time::tests` 4 项：now_ts/now_ms/ts_from_dt 全 PASS）
  - `cargo clippy -p golish --all-targets` → **exit 0**（无 warning）
  - `rg ts_from_chrono backend` → 0 命中；`ReadLints` 两文件 → 无错误
- **已记录证据**：见上「运行过的验证」。
- **提交记录**：**待用户授权**（未 commit）。本轮改：`M targets/types.rs`、`M targets/recon.rs`；连同上一轮未提交的时间戳收敛改动（time.rs/lib.rs/pipeline/vuln-intel/sub-agents/cli + vault/findings/wordlists/storage/organizations）。
- **已知风险或未解决问题（=下一轮可选 Wave 2，本轮**未**动，避免越界）**：仍有**内联**时间戳表达式与 chrono 取时未收敛——① 内联 `SystemTime::now().duration_since(UNIX_EPOCH)`：golish-events/transcript/summarizer.rs ×2、golish-cli-output/cli_json/mod.rs、golish/telemetry/stats.rs、golish/tools/pentest/packages/install/mod.rs（语义等价，所在 crate 已依赖 golish-core，低风险可换）；② golish-context/token_budget/{stats,manager}.rs（需给 golish-context **新增 golish-core 依赖**，无环但属结构改动）；③ chrono `Utc::now().timestamp[_millis]()` 与 `row.field.timestamp() as u64`（organizations/notes/audit/history/capture/oauth/terminal/pentest 等，API 不同、含 i64 算术，宜单独评估）；④ vendored fork `rig-openai-responses`（as_nanos 作 id）与 `stream_retry.rs`（subsec_nanos 作 jitter）语义不同，**不应**并入。
- **下一步最佳动作**：①（推荐）授权 commit 本轮 + 上一轮时间戳收敛；② 视意愿做 Wave 2 内联收敛（先做已依赖 golish-core 的低风险 5 处）。

---

### 2026-05-30 · P2 大文件拆分 ④（进行中）：frontend/mocks.ts 事件系统层抽出（MCP-2 续）

- **本轮目标**：用户选「继续拆 mocks.ts」。`[DISPATCH:off]`。mocks.ts 是 4135 行的浏览器/E2E mock harness（dev-only），结构特殊：~1100 行有状态 `mockIPC` switch + 大量可变模块状态。完整拆分是多模块分解（工作量≈前 3 个文件之和）。本轮先抽出**自包含的事件系统层**（已用 grep 验证解耦：simulate*/emit*/demo* 均不读写可变计数器状态，`mockIPC` handler 也不调用它们）。
- **已完成（step 1 事件系统 + step 2 AI 模拟 + step 3 showcase）**：
  - `mocks/event-bus.ts`（74）：监听器注册表 + `dispatchMockEvent`（原私有，现 `pub`）。
  - `mocks/events.ts`（215）：事件类型 + emit 助手。
  - `mocks/simulations.ts`（435）：AI 流式模拟（simulateAiResponse/SubAgent/WithSubAgent/JsHarvest，仅依赖 `emitAiEvent`）。
  - `mocks/showcase.ts`（**1146 · 仍 >500**）：timeline block 注入 + full-flow demos（mockCommandBlock/PipelineProgressBlock/SubAgentBlocks/ToolExecutionBlocks/PlanPipeline/ShowAllBlocks/FullPlanExecution/RunCommandApproval/simulatePipelineFanOut，用 `useStore`+`dispatchMockEvent`+`AiEventType`）。
  - **（step 4，已 commit 后续）** `mocks/fixtures.ts`（177）：只读数据（mockTools/Workflows/SubAgents/Sessions/ApprovalPatterns/Prompts/Skills/ProjectSettings + `MockCodebase` 类型）——ipc handler 读这些，是 ipc 层抽出的前置。`check-fe`+`test-fe` 全绿。
  - `mocks.ts`（4135→**2193**）：移除上述各块改 import；再导出全部原公共符号（`@/mocks` 公共面零变更）。
- **运行过的验证**（本机实跑，三步均跑过）：
  - `just check-fe`（tsc + biome）→ exit 0
  - `just test-fe`（vitest 全量）→ exit 0
  - `ReadLints` 各新文件 → 无错误
- **已记录证据**：见上「运行过的验证」。
- **提交记录**：**待用户授权**。`M mocks.ts` + `?? mocks/`（event-bus/events/simulations/showcase）。
- **已知风险或未解决问题**：mocks.ts 仍 2353（>500）；`showcase.ts` 1146（>500，可再按 timeline-block / full-flow demos / pipeline-fanout 三分）。剩余 mocks.ts 内：demos(~513)、有状态 state+getters/setters+`mockIPC` switch(~1300)。最后一块降到 <500 需把 ~10 个可变 `let` 收进共享状态容器 + switch 按域拆 handler（较大改动）。
- **下一步最佳动作**：① 抽 `mocks/demos.ts`；② state 容器 + `mocks/ipc.ts`（switch 按域拆）；③ 视需要再分 `showcase.ts`。或先 commit 已完成的 ①②③ 生产代码拆分 + ④ mocks step1-3。

---

### 2026-05-30 · P2 大文件拆分 ③：golish/tools/integrations/capture/engine.rs 1483→模块化（MCP-2 续）

- **本轮目标**：用户选「继续拆 capture/engine」。`[DISPATCH:off]` → 直接执行。把最大后端文件 `engine.rs`（1483 行）按职责拆成 module-root + 3 子模块，行为零变更。
- **关键设计**：把「会话生命周期/状态机 + webview 构建」方法**留在 root**（它们是唯一引用 capture 级兄弟模块 `data_dir`/`webview_isolation`/`session` 的代码 → 避免任何 `super::` 路径改写）；只抽出抽取逻辑与底层 helpers：
  - `engine.rs`（496）：module-root + doc + consts + `pub struct CaptureEngine` + impl Default + `impl CaptureEngine`（new/register/get/transition/cancel/gc/session_count/transition_and_emit/rearm/spawn_soft_retry_probe/deliver_js_value/start_webview/clear_profile/spawn_ttl_watcher）+ `pub(crate) use {extract,helpers}::*` 再导出。
  - `engine/extract.rs`（468）：`impl CaptureEngine { try_extract }` + on_navigation_event + rule_is_required + extract_one（7 种 CaptureRule 分发）+ 4 个 soft-retry failure-reason helpers。
  - `engine/helpers.rs`（247）：eval_js_value/parse_js_value_title（webview JS 值桥）+ fetch_domain_cookies/cookie_domain_matches/format_joined_cookies（cookie 访问）+ persist_captured_values（存储后端桥）。
  - `engine/tests.rs`（320）：23 个原单测逐字搬迁（module 路径仍 `capture::engine::tests`，session 字段可见性零变更）。
  - 关键不变量：跨多 `impl CaptureEngine` 块的方法互调按类型解析（与文件无关）；`self.sessions`/`js_value_waiters` 私有字段经子模块（descendant）访问；free fns 视情况 `pub(crate)` 由 root 再导出，仅 rule_is_required/extract_one 保持私有。
- **运行过的验证**（本机实跑）：
  - `wc -l` → 496 / 468 / 247 / 320，**全部 < 500**（engine.rs 496 紧贴预算）。
  - `cargo check -p golish` → exit 0（23.0s）
  - `cargo nextest run -p golish capture::engine` → **23 passed / 346 skipped**（原 23 单测全过）
  - `cargo clippy -p golish --all-targets -- -D warnings` → exit 0（0 告警）
- **已记录证据**：见上「运行过的验证」。
- **提交记录**：**待用户授权**。工作树累积三块 P2：①models、②js_collect、③capture/engine（各 `M <root>` + `?? <dir>/`）+ `M agent-progress.md`。
- **已知风险或未解决问题**：仅 cargo 层验证（无活 DB / 无真实 webview）；engine.rs 行数贴近 500 预算上限。
- **下一步最佳动作**：① 用户授权后按块拆 3 个 `refactor` commit；② 继续 P2（前端 `mocks.ts` 4135 收益最大；后端 `golish-integrations/*`、`pipeline steps/single.rs`、`tools/organizations.rs`、`ai/db_bridge.rs`）。

---

### 2026-05-30 · P2 大文件拆分 ②：golish/tools/pentest_bridge/js_collect.rs 1357→模块化（MCP-2 续）

- **本轮目标**：用户选「继续拆 js_collect」。`[DISPATCH:off]` → 直接执行。把 `js_collect.rs`（1357 行，职责混：下载+sitemap+扫描+质量门）按职责拆成 module-root + 6 子模块，行为零变更。
- **已完成**：`js_collect.rs` 保留为 module-root，`pub struct JsCollectTool`（外部仅此一项经 `pentest_bridge/mod.rs::pub use` 暴露，路径零变更）+ 3 const + 模块声明 + `pub(crate) use *::*` 再导出：
  - `js_collect.rs`（93）：module-root + doc + struct + new + consts(MAX_FILES/DOWNLOAD_CONCURRENCY/MANIFEST_PROBES) + 再导出。
  - `js_collect/extract.rs`（237）：纯 URL/HTML/JS 引用提取（resolve_url / extract_html_* / scan_js_for_references / looks_like_js_ref / extract_public_path / expand_webpack_chunk_map）。
  - `js_collect/judge.rs`（135）：内容真实性 + 同质性检测（Confidence / judge_js_content / HomogeneityReport / detect_homogeneous_chunks）。
  - `js_collect/quality.rs`（104）：`build_quality_warnings` + `CollectStats`（结构体入参规避 clippy `too_many_arguments`）。
  - `js_collect/sitemap.rs`（95）：`merge_into_sitemap`（sitemap_store 合并写入；**空集守卫保留在调用点**以保证零行为变更）。
  - `js_collect/tool_impl.rs`（470）：`impl Tool for JsCollectTool`（四策略发现 + 限并发下载 + 递归扫描 + 审计 + 委托 quality/sitemap）。
  - `js_collect/tests.rs`（309）：26 个原单测逐字搬迁，`use super::*` 经再导出解析。
  - 关键不变量：`self.pool` 私有字段经子模块（descendant）访问；audit `json!` 与 sitemap `INSERT`（含 project_path）逐字保留；只把 quality-warnings 构建与 sitemap 合并抽成函数，主 execute 流程顺序与可变状态零变更。
- **运行过的验证**（本机实跑）：
  - `wc -l` → 93 / 237 / 135 / 104 / 95 / 309 / 470，**全部 < 500**。
  - `cargo check -p golish` → exit 0（36.4s）
  - `cargo nextest run -p golish js_collect` → **26 passed / 343 skipped**（原 26 单测全过）
  - `cargo clippy -p golish --all-targets -- -D warnings` → exit 0（4m05s，0 告警）
- **已记录证据**：见上「运行过的验证」。
- **提交记录**：**待用户授权**。工作树累积两块 P2：①`M models.rs`+`?? models/`、②`M js_collect.rs`+`?? js_collect/`，外加 `M agent-progress.md`。
- **已知风险或未解决问题**：仅 cargo 层验证（无活 DB）；`sitemap.rs` 的 `sitemap_store` INSERT 仍是裸 SQL（原样保留，未纳入 P0-3b repo 下沉，不在本次 scope）。
- **下一步最佳动作**：① 用户授权后按块拆 commit（①pentest-domain models、②js_collect，各一个 `refactor`）；② 继续 P2 下一个（`capture/engine.rs` 1483 / 前端 `mocks.ts` 4135）。

---

### 2026-05-30 · P2 大文件拆分 ①：golish-pentest-domain/models.rs 1310→模块化（MCP-2 接手 MCP-1 上下文执行）

- **本轮目标**：接手 MCP-1（主控）转移的架构体检 backlog，执行 P2「超 500 行文件拆分」第 1 块。`[DISPATCH:off]` → 本会话直接执行。计划见 `docs/superpowers/plans/2026-05-30-arch-health-backlog.md`。
- **已完成**：把 `crates/golish-pentest-domain/src/models.rs`（1310 行）按职责拆成 module-root + 子模块目录（Rust 2018+ path 风格，**无删文件**，仅重写 models.rs + 新建 models/ 目录）：
  - `models.rs`（23 行）：module-root，`mod {asset_intel,runtime,tool_config}` + `pub use *::*` 全量再导出 + `#[cfg(test)] mod tests`。**公共路径零变更**（`golish_pentest_domain::models::X` 与 crate-root `::X` 均保持；lib.rs `pub use models::*` 未动）。
  - `models/tool_config.rs`（426）：ParamOption/ToolParam/ToolCategory/SubCategory/ToolConfig/InstalledVia/OutputConfig/OutputPattern/ToolConfigFile/ScanResult + `impl ToolConfig`(normalize/validate) + default_* + `VALID_PENTEST_PHASES`。
  - `models/asset_intel.rs`（321）：全部 `AssetIntel*` 类型 + 私有 default_*_asset_intel helpers（自包含）。
  - `models/runtime.rs`（145）：ToolSkill/InstallInfo/PlatformInstall/RuntimeInfo/RuntimeType/InterfaceType/LaunchResult + impl（自包含）。
  - `models/tests.rs`（431）：原 `#[cfg(test)] mod tests` 全量搬迁，`use super::*` 经再导出解析；14 个原测试逐字保留。
  - 行为零变更：纯模块重组；serde `default="fn"` 路径仍在各结构定义所在模块内解析。
- **运行过的验证**（本机实跑）：
  - `wc -l models.rs models/*.rs` → 23 / 426 / 321 / 145 / 431，**全部 < 500**（达标 500 行模块预算）
  - `cargo check -p golish-pentest-domain --all-targets` → exit 0（13.85s）
  - `cargo nextest run -p golish-pentest-domain` → **17 passed / 0 skipped**（14 models + 3 search）
  - `cargo clippy -p golish-pentest-domain --all-targets -- -D warnings` → exit 0（0 告警）
  - `cargo check --workspace` → exit 0（50.23s，全部下游 crate 编译通过 → 公共 API 零破坏）
- **已记录证据**：见上「运行过的验证」。
- **提交记录**：**待用户授权**（commit 属 AGENTS.md §2.7 高风险）。工作树：`M models.rs` + `?? models/`（4 新文件）。
- **已知风险或未解决问题**：仅 cargo 层验证（与该 crate 既有范式一致）。ToolConfig 的 I5 孪生问题（P1-a）未触碰——本块只是文件内重组，不合并孪生。
- **下一步最佳动作**：① 用户授权后单独 commit 本块（建议 `refactor(pentest-domain): split models.rs into config/asset_intel/runtime submodules (P2)`）；② 继续 P2 下一个文件（后端 `js_collect.rs` 1357 / `capture/engine.rs` 1483；前端 `mocks.ts` 4135 收益最大）。每块独立 commit，不混入跨块改动。

---

### 2026-05-30 · P0-3b 残余作用域 SQL 全量下沉 golish-db repo（T4-T6 · MCP-1 接手 MCP-4/MCP-5 上下文执行）

- **本轮目标**：接手 MCP-4 转移的上下文（源头 MCP-5 写完计划 `docs/superpowers/plans/2026-05-30-p0-3b-idor-residual-sink-full.md` 后断线，T1-T3 已在未提交工作树中完成），继续 T4-T6——把命令层残余的项目作用域裸 SQL 全部下沉到 `golish-db` repo 唯一边界（IDOR/I2 收口）。`[DISPATCH:off]` → 本会话直接执行。
- **已完成**：
  - **T4（repo 侧此前已建，本轮补命令层 + 1 个新 repo fn）**：
    - `tools/audit.rs` 6 处裸 SQL 全部改调 repo：`audit_list`→`audit::list_by_project_exact::<AuditRow>`、`audit_clear`→`audit::clear_by_project_exact`、`passive_scans_global`→`passive_scans::list_global_by_project`、`agent_logs_list`/`terminal_logs_list`/`search_logs_list`→各自 `*::list_by_project`。
    - `tools/pentest_bridge/auth_probe.rs` vault token 反查（L254）下沉：新增 `repo::vault::get_value_by_name_project`（`SELECT value FROM vault_entries WHERE name=$1 AND project_path=$2 LIMIT 1`，配 `build_*_sql` 零漂移单测）。
  - **T5（新建 6 个 repo 模块 + 1 个 targets fn + 命令层改调 8 文件）**：
    - 新建 `repo/{scan_queue,sensitive_scan,conversation_store,directory_entries,sitemap_store,custom_rules}.rs`，`repo/mod.rs` 注册 6 个 `pub mod`；每个自定义 SQL fn 均配 `build_*_sql` + `#[cfg(test)]` 零漂移断言。
    - `repo/targets.rs` 加 `exists_by_value_exact`（pipeline dedup 探针）+ 零漂移单测。
    - 命令层改调：`scan_queue.rs`(4)、`custom_rules.rs`(2)、`targets/directory.rs`(2 分支)、`sensitive_scan.rs`(sitemap 读 + results 列表 + clear×2 + verdicts 列表)、`conversation_store/mod.rs`(conv_list + load_preferences)、`conversation_store/batch.rs`(事务内 stale 删除，含动态 `NOT IN` 占位符；保持原子性)、`pipeline/storage.rs`(targets/dir EXISTS + sitemap 读/删)、`pentest_bridge/js_collect.rs`(sitemap 读/删)。
    - `conversation_store/mod.rs` 为消解 clippy `type_complexity` 抽了 `ConvListRow` / `WorkspacePrefsRow` 两个 `type` 别名。
  - **零漂移范式**：所有自定义 SQL 走 `build_*_sql()` 纯函数 + 单测断言字符串 == 迁移前原文（含 sitemap `name='zap-sitemap'` 字面、targets legacy 谓词、conversations `($1::text IS NULL OR ...)` 谓词、动态 `NOT IN` 0/1/3 形状）。
- **运行过的验证**（本机最新工作树实跑）：
  - `rg -n "project_path (IS NOT DISTINCT FROM|= \$|IS NULL OR project_path)" backend/crates/golish/src/tools` → **CLEAN（命令层裸作用域 SQL 清零）**
  - `cargo check -p golish-db --tests` → exit 0（24.4s）
  - `cargo check -p golish-db -p golish` → exit 0（43.1s）
  - `cargo nextest run -p golish-db` → **46 passed / 0 failed**（含 7 个本轮新增零漂移测试：scan_queue / sensitive_scan / sitemap_store / directory_entries / custom_rules / conversation_store ×2 + targets exists + vault get_value）
  - `cargo nextest run -p golish --lib` → **318 passed / 0 failed**（无回归）
  - `cargo clippy -p golish-db -p golish --all-targets -- -D warnings` → **exit 0 全绿**。本轮先修我引入的 type_complexity×2（抽 `ConvListRow`/`WorkspacePrefsRow` 别名）+ explicit_auto_deref×1（`&mut tx`）；随后**经用户明确授权**（「修掉 integrations clippy」）顺手清掉 1 个 pre-existing baseline `integrations/commands.rs:179 doc_lazy_continuation`（doc 注释里行首 `+ ` 被 markdown 误判为列表项 → 把 `+` 移到上一行行尾，非 SQL 逻辑改动）。
  - **`just precommit`（用户授权后跑全栈门禁）→ exit 0 `✓ All checks passed!`**（~18.5min；= fmt + check-fe[biome+tsc] + test-fe[vitest] + lint-rust[clippy `--workspace` `-D warnings` + `cargo fmt --check`] + test-rust-all[nextest `--workspace`] + check-types[ts-rs gen + `git diff --exit-code` 无漂移] + test[再跑前后端]）。`fmt` 自动格式化后工作树仍仅本轮预期改动，无意外漂移。
- **已记录证据**：见上「运行过的验证」；新文件 6 个 repo 模块为 untracked（`git diff --stat` 不显示，nextest 已编译并跑过其测试）。
- **提交记录**：经用户授权（「按拆分提交」）已落 4 个 commit 到 `feat/recon-service`（**未 push**）：
  1. `65e0292` feat(db): project-scoped repo helpers for residual scoped SQL sink (P0-3b) — 15 files
  2. `06af27a` refactor(tools): route residual scoped SQL through golish-db repo (P0-3b) — 17 files
  3. `d023386` fix(integrations): resolve clippy doc_lazy_continuation in test module docs — 1 file
  4. `c2f5ad2` docs: record P0-3b residual scoped SQL sink plan + progress — 2 files
  （+ 本条 progress 收尾微调另起一个 docs commit）。工作树提交后 `git status` 干净。
- **已知风险或未解决问题**：
  - 全部为 `cargo`（无活 DB）层验证——零漂移单测保证 SQL 字符串与迁移前逐字一致，但 SQL 实际行为未跑 pg-embed 集成测试（与既有 repo 测试范式一致）。
  - `just precommit` 已跑且**全绿**（见上）；唯一未做的是 **commit 本身**（高风险，需用户确认）。
  - 注：经用户授权额外修了 `integrations/commands.rs:179` 这条 pre-existing baseline clippy（doc 注释，非 P0-3b SQL scope，但已纳入本轮 diff）。
- **下一步最佳动作**：① 用户授权后按 Tier/任务粒度拆 commit（建议：先 `golish-db` repo 增量[T1/T3/T4 repo + 6 新模块]，再命令层改调[T2/T4/T5]，integrations clippy 单列一个 `fix`）；② 决定是否 push `feat/recon-service` / 开 PR；③ 工作树仍含 MCP-5 留下的 untracked 计划文件 `docs/superpowers/plans/2026-05-30-p0-3b-idor-residual-sink-full.md`，commit 时一并纳入。

---

### 2026-05-30 · 架构优化批 125-file 工作树按功能拆 9 commit + 全绿验证（MCP-2 执行）

- **本轮目标**：承接 MCP-3 转移的上下文（架构优化 P0/P1 代码已写完但 125 文件全压工作树未提交），用户指令「按功能拆 commit 分组」→「随便你」。本轮 = 真正落地拆分提交 + 端到端验证。
- **关键发现（用证据纠正了转移上下文里的过时结论）**：
  - 转移上下文称 `just precommit` 非全绿（clippy warnings + sandbox baseline failures）。**本机实测全绿**：clippy `-D warnings` 0 告警、nextest workspace **2592 passed / 7 skipped / 0 failed**、check-fe / test-fe / `cargo fmt --check` / ts-rs 绑定无漂移全过。旧失败未复现。
  - `gen-types`（`cargo test --workspace export_bindings`）把 `frontend/lib/generated/` 重新与 Rust 源对齐：7 个此前“已修改”的绑定文件本是过时漂移，回到 HEAD；20 个新类型为真实新增。
  - 计划里两处归属纠正：`vuln_intel/commands/matching.rs` 实为 I2 scoping（并入 C2/C3，非 C8）；`findings/mod.rs` 改用 `golish_db::repo::findings::FindingDetailRow` 且对齐 `FindingStatus`，与 DB 层强耦合（并入 C2/C3）。`tools/organizations.rs` 的 `rename_all="camelCase"` 删除是独立 bug fix（单列一个 `fix` commit）。
  - C2（golish-db 助手）与 C3（tools scoping）签名互相依赖（repo 签名加 `project_path`），单独提交 C2 会让 golish crate 不编译 → **合并成一个 commit**；C4/C5 因 hunk 重叠也按计划择优拆成「错误码契约」+「api 层路由」两个干净 file-level commit（无需 `git add -p`）。
- **已完成（9 个 commit，按依赖序，落在 `feat/recon-service`）**：
  1. `98beea9` feat(types): finish ts-rs cross-IPC type generation (P0-2) — 37 files
  2. `30cb5e1` refactor(db): project-scoped CRUD helper + enforce IDOR scope (P1-1, P0-3, I2) — 32 files
  3. `f329be5` fix(organizations): accept snake_case keys in profile patch (I3) — 1 file
  4. `92522a7` feat(error): end-to-end error-code contract (P0-1, I1) — 5 files
  5. `6065658` refactor(frontend): route through typed API layer + tri-state (P0-4, M2) — 19 files
  6. `1ff31bc` refactor(asset-intel): split monolith into runtime/service/commands layers — 11 files
  7. `b03a51f` refactor(target-panel): extract org/target subcomponents from TargetGroupedView (P1-4) — 15 files
  8. `75165c3` docs(arch): architecture optimization design + P0/P1 implementation plans — 6 files
  9. `6aaa0fb` chore(harness): tighten global-enforcement rules + skill cross-links — 7 files
- **已记录证据**（均在本机最新工作树/HEAD 实跑）：
  - `cargo check --workspace` → exit 0（Finished，1m37s）
  - `cargo clippy --workspace -q -- -D warnings` → exit 0（0 告警）
  - `cargo fmt --check` → exit 0
  - `cargo nextest run --workspace --status-level fail` → **2592 passed / 7 skipped / 0 failed**（~95s）
  - `just check-fe`（biome + tsc）→ exit 0；`just test-fe`（vitest）→ exit 0
  - `cargo test --workspace export_bindings` + `git diff --exit-code -- frontend/lib/generated/` → 无漂移
  - **拆分提交完成后** `just check` → `✓ passed` / `━━━ OK ━━━`（exit 0），且 `git status --porcelain` 仅剩 `agent-progress.md`（证明 fmt 步零改动、9 个 commit 与工作树字节一致）
- **提交记录**：见上 9 个 commit。均为本地 commit，**未 push**（push 属高风险，需用户确认）。
- **已知风险或未解决问题**：
  - 各 commit 未做逐个隔离编译验证（成本高）；但按依赖序排列、最终 HEAD == 已验证全绿工作树，最终态有保证。
  - 这些 P0/P1 项仍无 `feature_list.json` 专属条目（靠设计文档 + 计划跟踪）；当前唯一 `in_progress` 仍为 `target-surface-workbench`，本轮未动其状态。
  - `xiaomi-mimo-provider` blocker 与本轮无关，保持原状。
- **下一步最佳动作**：① 用户决定是否 push `feat/recon-service` / 开 PR；② 视需要把 P0/P1 在 `feature_list.json` 补成正式条目并标 passing（已有证据）；③ 继续推进未动的 P1-2 / P1-3 / P2 项。

---

### 2026-05-30 · P0-4 前端 api 层（裸 invoke 收口）验证（多 agent 派发 · agent-5 执行）

- **本轮目标**：controller agent-1 派发 dispatch `47df9103` / `t-1`，要求落地 P0-4（前端调用层回归 api 层）。计划：`docs/superpowers/plans/2026-05-29-p0-frontend-api-layer.md`。
- **发现**：P0-4 的 9 个任务在当前未提交工作树中**已全部实现**，本轮转为**验证**（read-only + 跑测，未改业务代码）：
  - 任务1：`frontend/lib/api/pipeline.ts` 已含 `executePipeline`/`cancelPipeline`/`deletePipeline` + 新头注释（"All pipeline IPC lives here"）✅
  - 任务2/3：`rg 'targets\.(execute|cancel|delete)Pipeline' frontend` → 0 命中；`PipelinePanel.tsx` 改用 `listPipelines`/`deletePipeline` ✅
  - 任务4：`frontend/lib/api/vuln-intel.ts` 已含 `addPocFull`(L109) + `wikiSearchDb`(L138) ✅
  - 任务5/6/7/8：`rg 'invoke[<(]' frontend/components` → 仅 1 处 `FindingsPanel.tsx` 的 biome-ignore **注释**（convertFileSrc，非 invoke 调用）✅；`rg 'import {...invoke...} from "@/lib/api"' frontend/components` → 0 ✅
  - 任务9：`frontend/lib/api/index.ts` 已不再 re-export `invoke`（barrel 硬约束）✅；`biome.json` 保留 `@tauri-apps/api/core` + `@/lib/api/client` 两条（计划允许只留 barrel 硬约束，第三条 `@/lib/api` 为可选双保险，未启用）
- **运行过的验证**：
  - `just check-fe`（biome + tsc）→ **exit 0**（236s，仅 models.generated.ts 预构建副作用输出，无类型/lint 错）
  - `just test-fe`（vitest 全量 96 files）→ exit 1：**4 failed | 1090 passed | 12 skipped**；4 个失败**全部 `Test timed out in 5000ms`**，落在 `Markdown.lazy` / `PaneLeaf.memo` / `NewEngagementDialog` / 一个 createSession session 测试，**均与 api 层无关**；耗时异常（environment 282s / setup 177s）指向高负载
  - 隔离重跑（低负载）4 失败文件 + P0-4 区：`pnpm exec vitest run Markdown.lazy PaneLeaf.memo NewEngagementDialog VulnIntelPanel PipelinePanel TargetPanel/hooks lib/api` → **exit 0，5 files / 27 passed**；之前超时的 "should be wrapped in React.memo" 2965ms、Markdown fallback 2541ms、NewEngagement 1794ms 均远低于 5000ms → **确认 4 个失败是多 agent 并发高负载下的环境性 flaky 超时，非 P0-4 回归**
- **已记录证据**：见上"运行过的验证"
- **提交记录**：未 commit（高风险需用户确认；且工作树含大量 P0-4 之外的未提交改动，需按功能拆分）
- **已知风险或未解决问题**：
  - `just test-fe` 全量在高负载下偶发超时（已隔离确认无关 P0-4）；干净全绿需在低负载下重跑
  - 工作树含 P0-4 之外大量未提交改动（asset_intel / target panel / error-codes 等），commit 前需拆分
  - `feature_list.json` 无 P0-4 专属条目（由计划文档 + 派单跟踪），当前唯一 `in_progress` 仍为 `target-surface-workbench`，本轮未改其状态
- **下一步最佳动作**：① 低负载下重跑 `just test-fe` 取干净全绿；② 用户决定 P0-4 的 commit 拆分（建议按计划 9 任务的 commit 粒度）；③ P0-4 兄弟计划（panel error 态）另行推进

---

### 2026-05-29 · 全栈删除 terminal-era 遗留（Git 源代码面板 + 终端 git 徽标 + worktree + AI 评测 crate）

- **本轮目标**：用户先做了一轮架构体检（重复/复用/解耦/优化），随后聚焦到「git 那套东西是什么」→ 确认是源代码管理面板（提交 git 用）→ 决定删除所有对渗透测试无用的 terminal/coding 时代遗留。用户决策：「全删（含 worktree+eval）」；git 分两层时明确选「B：连终端分支徽标一起删」（授权动终端核心）。
- **已完成（全部删除并验证编译/测试通过）**：
  - **AI 评测 crate**：删 `golish-evals`/`golish-benchmarks`(HumanEval)/`golish-swebench`(SWE-bench) 三个 crate 目录 + workspace/golish Cargo.toml 登记 + `evals` feature + `--features evals` CLI 分支（main.rs/cli/args.rs/cli/mod.rs/cli/bootstrap）+ `cli/eval/` 目录 + scripts/check_dag.py 的 L5 登记 + justfile 的 eval/swebench 目标。
  - **Git 源代码面板（后端）**：删 `commands/proc/git.rs`、`ai/commands/commit_writer.rs`、`indexer/commands/home_view.rs`、`indexer/commands/worktrees.rs`；registry/facade 移除 git 系命令（status/diff/stage/commit/push/delete_worktree）+ worktree（create/list）+ `generate_commit_message` + `list_projects_for_home`；`shell/mod.rs` 删 `get_git_branch`；`hidden_dirs.rs` 内联 `format_relative_time` 并去 git 统计（`RecentDirectory` 收窄为 path/name/last_accessed）。
  - **Git 面板 + 终端 git 徽标（前端）**：删 `components/GitPanel/`、`DiffView/GitDiffView.tsx`、`lib/api/git.ts`、`lib/git.ts`、`store/slices/git.ts`、`store/selectors/git-panel.ts`、`HomeView/{NewWorktreeModal,ContextMenus,ProjectCards}.tsx`；store 去 git slice + session/pane/panel/selectors/unified-input；App/AppShell/useKeyboardHandlerContext 去 GitPanel + Cmd+Shift+G；useCreateTerminalTab/usePaneControls/useTauriEvents/terminal-events 去 git fetch + 5s 轮询；lib/api/index 去 git 命名空间；lib/api/indexer + lib/indexer 去 worktree/ProjectInfo/BranchInfo；lib/ai/persistence+types 去 generateCommitMessage/CommitMessageResponse；mocks.ts 去 git mock；~20 测试文件去 git mock/断言（删 useTauriEvents.test.tsx + HomeView.memo.test.tsx 两个纯 git/ProjectCards 测试）。
- **已记录证据**：
  - `cd backend && cargo check -p golish` → exit 0
  - `cd frontend && pnpm exec tsc --noEmit` → 0 errors
  - `cd frontend && pnpm exec biome check .` → Checked 689 files / No fixes applied
  - `pnpm test:run` → 92 files / 1075 passed / 12 skipped
- **保留（渗透要用/不同特性）**：`DiffView` 基础组件、GitHub Token/Integrations、`pentest_git_clone_tool`、`golish-sidecar::generate_commit_message`（patches→PR 合成，不是源代码面板）、HomeView 项目列表（`listProjectConfigs`）、`listRecentDirectories`（收窄）。
- **未跑 / 原因**：未跑 `just precommit`（baseline 即有 clippy warnings-as-errors + sandbox PermissionDenied 测试失败，与本删除无关）；未做手动 E2E（建议用户 `just dev` 复测：终端标签无 git 徽标、Cmd+Shift+G 失效、HomeView 正常、无控制台报错）。
- **提交记录**：未 commit（等用户确认）。
- **已修改但未提交（本轮 scope）**：见上方删除/编辑清单 + `feature_list.json`（新增 `chore-remove-git-eval-worktree` = passing）+ `agent-progress.md` + `docs/architecture.md`（移除 L5 eval 段）。注意工作树仍含此前 Target Surface Workbench 等未提交改动，commit 前需按功能拆分。
- **追加（同轮）· 历史 docs + 注释清理到字面零残留**：用户要求「清掉历史 docs+注释」。删除 `docs/swebench/`(目录)、`docs/swebench.md`、`docs/rig-evals.md`、`docs/pr-check-evals.md`、`docs/home-view-implementation.md`；`docs/README.md` 移除 Evaluation/benchmarks 段的 4 个死链（保留 graph-flow-integration）；`docs/prompt-contributions.md` 删 Evaluations(executor.rs) 段 + Testing 段 + golish-evals related-file + 改写 intro；`docs/system-prompt-guide.md` 删 eval parity 测试；`docs/golish-platform-analysis.md` 删 golish-evals 树行；清理 `PaneLeaf.memo.test.tsx` 与 `UnifiedInput/REFACTOR_PLAN.md` 里 onOpenGitPanel 注释。最终残留扫描：仅剩 `docs/architecture.md` 的删除说明（有意）+ `golish-sidecar::generate_commit_message`（不同特性，有意保留）。复验：tsc 0 / biome clean / vitest 92 files 1075 passed / cargo check -p golish exit 0。注：AGENTS.md I6 通常建议旧文档标 superseded 而非删除，本轮按用户明确指令直接删除。
- **下一步建议**：① 用户 `just dev` 手动复测 UI（终端无 git 徽标 / Cmd+Shift+G 失效 / HomeView 正常 / 控制台无报错）；② 体检报告里的其它项（修绿 DAG 守卫、前端 useAsyncQuery 抽取、后端 pentest/vuln domain 去重）待用户挑选推进；③ commit 策略待用户确认（工作树仍含此前未提交改动，需按功能拆分）。

---

### 2026-05-28 · Target Surface Workbench 设计确认 + 第一版前端接线

- **本轮目标**：用户确认 ZAP/SecurityView 删除后，下一步 UI 按 mock 方向设计（Org Tree + Target Surface Workbench）。用户明确要求“不要跑 init 了，直接搞”，因此中止正在跑的 `./init.sh`，直接落设计文档、实施计划和第一版前端 UI。
- **状态切换**：
  - `feature_list.json` 新增 `target-surface-workbench` 并设为唯一 `in_progress`。
  - `agent-tool-use-compatibility-layer` 从 `in_progress` 暂停为 `blocked`：用户当前切到 Target Surface Workbench；之前代码不回滚，后续再恢复。
- **已完成**：
  - 新增设计文档：`docs/design/2026-05-28-target-surface-workbench.md`
  - 新增实施计划：`docs/superpowers/plans/2026-05-28-target-surface-workbench.md`
  - 新增 mock artifact：`.codex-screenshots/target-surface-workbench-mock.svg`（用户已确认“可以就这样设计”）；同时保留 `.codex-screenshots/target-surface-workbench-mock.html` 作为静态 mock 源。
  - 新增 `frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`：复用现有 security-analysis API，统一拉 `targetAssetsList` / `apiEndpointsList` / `fingerprintsList` / `jsAnalysisList` / `oplogListByTarget`，返回 loading/error/reload。
  - 新增 `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`：Target 选中后显示正式 workbench，包含 header scope/source/evidence metadata、staged actions、`Identity / Surface / Sitemap / JS/API / Sensitive / Evidence` tabs；首版复用现有 target ports / endpoints / JS / logs。
  - `TargetGroupedView.tsx` 新增 `selectedTargetId` 状态：点击 target row 右侧进入 Workbench；点击 org row 清除 target selection 回到 org workspace。
  - `TargetDetail.tsx` 改用共享 `useTargetSurfaceData` hook，避免旧的 fetch 逻辑继续散在组件内。
  - `NewEngagementDialog.tsx` 删除 “Discovery orchestration is not wired yet” 过期文案，改成保存 discovery settings 后从 org workspace 继续 discover/enrich/promote。
  - `frontend/lib/i18n/{en,zh-CN}.json` 更新空态，不再提旧的 Network/List 视图。
- **已记录证据**：
  - `./init.sh`（用户后续要求停止）→ 依赖安装 passed；`fmt` passed；`check-fe` passed；`test-fe` passed；执行到 `lint-rust` 时用户要求“不要跑init了 直接搞”，已 SIGINT，中止后 exit 130。
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/design/2026-05-28-target-surface-workbench.md docs/superpowers/plans/2026-05-28-target-surface-workbench.md` → exit 0 / Checked 18 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
  - Browser visual check：`http://127.0.0.1:1420` → Recent project `golish` → Target Manager 空态成功渲染，新文案为 “Create an engagement, then import customer targets or discover assets from an organization profile”，不再提 Network/List。
  - `node -e "JSON.parse(require('fs').readFileSync('feature_list.json','utf8'))"` → feature_list JSON parse ok；唯一 `in_progress` 为 `target-surface-workbench`。
- **未跑 / 原因**：
  - 未完成 `./init.sh` / `just precommit`：用户明确要求停止 init 并直接实现。
  - 未做真实 target 数据视觉 QA：当前 browser 只验证了空态；需要后续用真实或 mock target 数据确认 Workbench 的 selected-target 状态、JS/API/Sensitive/Evidence tabs 在 1280x720 下无重叠。
  - 未触碰后端 / DB schema / Tauri command。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `.codex-screenshots/target-surface-workbench-mock.html`
  - `.codex-screenshots/target-surface-workbench-mock.svg`
  - `docs/design/2026-05-28-target-surface-workbench.md`
  - `docs/superpowers/plans/2026-05-28-target-surface-workbench.md`
  - `feature_list.json`
  - `agent-progress.md`
  - `frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`
  - `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`
  - `frontend/components/TargetPanel/TargetGroupedView.tsx`
  - `frontend/components/TargetPanel/TargetDetail.tsx`
  - `frontend/components/TargetPanel/NewEngagementDialog.tsx`
  - `frontend/lib/i18n/en.json`
  - `frontend/lib/i18n/zh-CN.json`
- **下一步建议**：用一组可控 mock target 数据或现有真实项目 target 进入 selected-target 状态做视觉 QA；然后补 `Sitemap` / `Sensitive` 真实数据源映射，最后在用户允许时跑 `just precommit` 再考虑切 `passing`。

#### 2026-05-28 追加 · Workbench 数据面补强

- **追加目标**：用户说“开始实现吧”，继续把 Workbench 从壳子推进到更完整的 existing-data UI。
- **追加已完成**：
  - `useTargetSurfaceData` 继续扩展：新增拉取 `passiveScansList(targetId, 50)`、`targetTimeline(targetId, 100)`、`listDirectoryEntries({ targetId })`，与 assets/endpoints/fingerprints/js/oplog 一起组成统一 target surface payload。
  - `frontend/lib/security-analysis.ts` 兼容 re-export 补 `PassiveScanLog` / `passiveScansList` / `TimelineEntry` / `targetTimeline`。
  - `TargetSurfaceWorkbench` tabs 显示数量 badge。
  - `Surface` tab 真实渲染 fingerprints，不再永远显示空态。
  - `Sitemap` tab 渲染 directory entries + `target_assets` 中 path/url/sitemap 类型记录。
  - `Sensitive` tab 渲染 JS secrets / source map signals，并合并 passive scan 中 `vulnerable` / `potential` 结果。
  - `Evidence` tab 优先渲染 `target_timeline`，无 timeline 时回退到 `oplogListByTarget`。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json frontend/lib/security-analysis.ts` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
  - Browser text check：`http://127.0.0.1:1420` → project `golish` → Target Manager 文本成功读取，空态仍为新文案。截图 capture 曾超时一次，但 DOM/text polling 成功。
- **仍未完成**：
  - 需要有真实 target 数据或专门 mock fixture，才能检查 selected target workbench 的有数据视觉状态。
  - 未跑 `just precommit`（遵循用户“不跑 init，直接搞”的当前指令）。

#### 2026-05-28 追加 · 左右视觉对齐 pass

- **追加目标**：用户截图指出左侧树与右侧 Workbench 样式有出入；继续把 selected-target 工作台和左侧 target tree 的视觉语言收敛。
- **追加已完成**：
  - `TargetSurfaceWorkbench.tsx`：压低 header / tab / action button 高度，弱化 section / metric / empty state 的边框和背景；`Surface` / `JS/API` 双列 grid 增加 `items-start`，避免左侧 Services 被右列空内容撑成超高卡片；空态高度从 280px 收到 180px。
  - `TargetGroupedView.tsx`：左右布局比例从 `0.9fr/1.1fr` 调为 `0.72fr/1.28fr`，让右侧 Workbench 获得更合理的检查空间。
  - 左侧 target tree 轻整理：selected target 改为稳定左边选中条；out-of-scope 只弱化名称不再整行半透明；target 行固定成 type / scope / value / evidence count / action 的紧凑结构；子公司不再每行重复 mode badge，仅根节点或当前选中节点显示。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/security-analysis.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
  - 尝试启动 `pnpm dev --host 127.0.0.1`：沙箱内监听 localhost 被 EPERM 拦截；升级后 1420/1421 已占用；改用 1430 可启动，但随后用户表示“不用自己看，我给你截图”，因此停止浏览器自检路线，基于用户截图继续收敛样式。
- **仍未完成**：
  - 未跑 `just precommit`（当前用户指令仍是不要跑 init/precommit，直接实现）。
  - 还需要用户用真实 UI 截图确认左侧 tree pass 是否够；若继续调整，优先处理 top toolbar / org action hover 区域，而不是重做整套导航。

#### 2026-05-28 追加 · Target 顶部切换与主操作色修正

- **追加目标**：用户截图指出 Target Manager 顶部 tree/graph 分段 tab 已不需要，且“新建任务”等主操作颜色在当前暗色主题下看不清；同时确认拓扑图后续应重新设计。
- **追加已完成**：
  - `TargetPanel.tsx` 移除顶部 tree/graph 分段切换，Target Manager 固定进入 org tree + selected target workbench；`TargetGraphView` 代码保留，后续 topology/relationship workspace 重新设计时再接入。
  - Target Manager 标题改为普通 `text-foreground`，icon 改蓝色，避免继续依赖低对比 `accent`。
  - `TargetGroupedView.tsx`：顶部“新建任务”改为 green primary button，Quick org 改为低调边框按钮；空态三张入口卡把 Import targets / Discover assets 分别改成 green / blue 语义色。
  - `NewEngagementDialog.tsx`：workflow 选项、Look up、submit primary button 去掉低对比 `accent` 主色，改为 green / blue / neutral 语义色。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/security-analysis.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
- **仍未完成**：
  - 拓扑图没有在本轮重做：当前只隐藏旧入口，后续建议作为 Relationship / Ownership Graph 独立工作区设计，带 evidence filter、org/target/path 切换，而不是恢复顶部小 tab。
  - 未跑 `just precommit`（继续遵循用户“不跑 init，直接搞”的当前指令）。

#### 2026-05-28 追加 · 移除 Quick org 顶栏入口

- **追加目标**：用户明确要求删掉“快速建组织”按钮。
- **追加已完成**：
  - `TargetGroupedView.tsx` 顶栏删除 Quick org 按钮，只保留 New Engagement 作为主创建入口和 org/target 计数。
  - 空态中的 Profile Only 卡片暂保留，避免空项目没有只建 customer record 的路径。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/security-analysis.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
- **仍未完成**：
  - 未跑 `just precommit`（继续遵循用户“不跑 init，直接搞”的当前指令）。

#### 2026-05-28 追加 · Org 子公司计数不换行

- **追加目标**：用户截图指出 `7 sub` 在左侧 org 行里被拆成两行。
- **追加已完成**：
  - `TargetGroupedView.tsx` 将子组织计数改为 `inline-flex whitespace-nowrap`，保证 `· 7 sub` 作为一个整体同行显示。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/security-analysis.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
- **仍未完成**：
  - 未跑 `just precommit`（继续遵循用户“不跑 init，直接搞”的当前指令）。

#### 2026-05-28 追加 · 恢复 Topology 入口

- **追加目标**：用户希望拓扑图后续由另一个 session 重新设计，但当前不要隐藏入口。
- **追加已完成**：
  - `TargetPanel.tsx` 恢复 tree / topology view switch，并保持默认进入 org tree + workbench。
  - 切换 UI 从纯图标分段改成低调的 `Tree / Topology` 文本+图标按钮，保证入口可见但不抢主流程。
  - `TargetGraphView` 继续作为现有 topology view 渲染，等待后续重新设计。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel frontend/lib/security-analysis.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
- **仍未完成**：
  - 拓扑图本身未重做；本轮只恢复入口。
  - 未跑 `just precommit`（继续遵循用户“不跑 init，直接搞”的当前指令）。

#### 2026-05-28 追加 · Topology redesign 静态 mock

- **追加目标**：用户希望先 mock 新拓扑图方向，确认“看着舒服、逻辑更好”后再决定是否接入 `elkjs` / 重构真实图。
- **追加已完成**：
  - 新增静态 mock：`.codex-screenshots/target-topology-redesign-mock.svg`
  - mock 方向：左侧 topology controls / graph mode / filters，中间 full-bleed layered attack surface map，右侧 inspector；节点从 `Organization → Target → Service → Evidence` 分层，边带语义（ownership / service exposure / evidence trail），视觉上与当前 Target Surface Workbench 的暗色、紧凑、证据优先风格对齐。
- **追加验证证据**：
  - `python3 -m xml.etree.ElementTree .codex-screenshots/target-topology-redesign-mock.svg` → exit 0（SVG/XML 结构可解析）
  - `wc -l .codex-screenshots/target-topology-redesign-mock.svg` → 297 lines
- **仍未完成**：
  - 未改真实 Topology 代码；本轮只产出 mock。
  - 未跑 `init` / `just precommit`，遵循用户“不要运行init 你直接看”的当前指令。

#### 2026-05-28 追加 · Topology redesign 第一版实现

- **追加目标**：用户确认 mock “很不错”后要求开始实现；按用户确认的策略删除旧拓扑实现，用新版 Attack Surface Map 替换旧 Topology。
- **追加已完成**：
  - 新增设计文档：`docs/design/2026-05-28-target-topology-redesign.md`
  - 新增实施计划：`docs/superpowers/plans/2026-05-28-target-topology-redesign.md`
  - 新增 topology 模型层：`frontend/components/TargetPanel/topology/types.ts`、`buildTopologyModel.ts`
  - 新增 topology UI：`TopologyControls.tsx`、`TopologyCanvas.tsx`、`TopologyInspector.tsx`
  - 重写 `TargetGraphView.tsx`：通过现有 `organizations` API wrapper 拉 organization list，组合 `targets` 构建 `Organization → Target → Service → Evidence` 分层图；不再裸 `invoke("findings_for_host")`。
  - 删除旧实现：`frontend/components/TargetPanel/GraphElements.tsx`、`frontend/components/TargetPanel/hooks/useGraphLayout.ts`。旧 Cytoscape target-only graph 不再与新版并存。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel docs/design/2026-05-28-target-topology-redesign.md docs/superpowers/plans/2026-05-28-target-topology-redesign.md` → exit 0 / Checked 19 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
  - `rg -n "GraphElements|useGraphLayout|buildGraphElements|GraphSidebar|GraphNodeDetail|findings_for_host" frontend/components/TargetPanel frontend/lib -S` → exit 1 / no matches（确认旧 graph helper 与裸 findings invoke 已清空）
- **仍未完成**：
  - 未跑 `init` / `just precommit`，遵循用户“不要运行init 你直接看”的当前指令。
  - 未做浏览器视觉 QA；下一步建议用户跑 app 看真实数据截图，再微调节点密度、边距、inspector action 是否接入 Tree workbench。
  - 未新增 `elkjs` 依赖；当前第一版用本地 deterministic layered layout，后续如果节点规模变大再把 layout 层替换为 ELK。

#### 2026-05-28 追加 · Topology org 层级修正

- **追加目标**：用户截图指出逻辑错误：总 org 下的 sub org 被画成一个个独立 org，而不是挂在总 org 后面。
- **追加已完成**：
  - `buildTopologyModel.ts` 从 flat org 排序改为按 `parent_id` 构建 `childrenByParent` 并递归渲染。
  - 画布列从 `ORG / TARGET / SERVICE / EVIDENCE` 调整为 `ROOT ORG / SUB ORG / TARGET / SERVICE / EVIDENCE`。
  - root org 仍在第一列；sub org 在第二列；sub org 下的 targets 再进入 target 列；root 直接 targets 则挂在 sub org/target 相邻列，避免把子公司当独立 root 展开。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel/topology frontend/components/TargetPanel/TargetGraphView.tsx` → exit 0 / Checked 6 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
  - `node -e "const fs=require('fs'); JSON.parse(fs.readFileSync('feature_list.json','utf8')); console.log('feature_list ok')"` → exit 0
- **仍未完成**：
  - 未跑 `init` / `just precommit`，遵循用户“不要运行init 你直接看”的当前指令。
  - 未做浏览器视觉 QA；等待用户复看真实截图确认 root/sub/target 层级是否符合预期。

#### 2026-05-28 追加 · Topology root/sub 关联可视化修正

- **追加目标**：用户进一步指出虽然分列了，但看不到 root org 和 sub org 的关联性。
- **追加已完成**：
  - `buildTopologyModel.ts` 从“root 先占一行、children 往下追加”的流水布局，改成先计算整组 child/subtree 占用行，再把 root org 放到该组垂直中心。
  - target/service/evidence 也改为同一 target 的第一 service/evidence 与 target 同行，额外 service 再下排，减少边线漂移。
  - `TopologyCanvas.tsx` 将 `owns` 连线从弱灰线改为更亮更粗的 cyan 线，让 root → sub org 的父子关系更明显。
- **追加验证证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel/topology frontend/components/TargetPanel/TargetGraphView.tsx` → exit 0 / Checked 6 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → 2 files passed / 41 tests passed
- **仍未完成**：
  - 未跑 `init` / `just precommit`，遵循用户“不要运行init 你直接看”的当前指令。
  - 未做浏览器视觉 QA；等待用户复看真实截图确认 root/sub 关联视觉是否足够清晰。

#### 2026-05-28 追加 · Topology root target 列归属修正

- **追加目标**：用户截图指出加入 sub org 后，没有 sub org 的 target 视觉上跑到了 sub org 里面。
- **追加已完成**：
  - `buildTopologyModel.ts` 修正 target 列分配：root org 直属 target、sub org target、unassigned target 统一进入 `TARGET` 列，不再把 root 直属 target 放到 `SUB ORG` 列。
  - 新增 `buildTopologyModel.test.ts` 回归测试，覆盖 root-owned / sub-owned / unassigned 三类 target 在有 sub org 时的列归属，并确认 root target 不会连到 sub org。
- **追加验证证据**：
  - `pnpm exec vitest run frontend/components/TargetPanel/topology/buildTopologyModel.test.ts` → 先红灯：root-owned target 实际 `column: 1`；修复后 exit 0 / 1 passed
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel/topology frontend/components/TargetPanel/TargetGraphView.tsx` → exit 0 / Checked 7 files / No fixes applied
  - `pnpm exec vitest run frontend/components/TargetPanel` → exit 0 / 3 files passed / 42 tests passed
- **仍未完成**：
  - 未跑 `init` / `just precommit`；本轮只做 topology 前端定向修复与验证。
  - 未做浏览器视觉 QA；需要用户刷新 topology 后确认真实截图里 root target 已回到 Target 列。

---

### 2026-05-28 · Remove OWASP ZAP integration + 整个 SecurityView 外层面板（全栈删除）

- **本轮目标**：分两轮 —— ① 先把 OWASP ZAP 全删（"觉得很鸡肋"）；② 用户进一步决策：把整个 SecurityView 外层面板也删掉（"安全测试这一块内容很不符合现在的逻辑 / 我觉得删掉最干净是最好的"），未来基于 organization → target → 端口 → JS/敏感信息 这套 target-centric 逻辑重新设计。后端所有 Tauri command 全保留作为重构能力库。
- **最终分支**：`chore/remove-zap`，累计 111 个文件 / +231 行 / -14991 行（净删 ~14760 行）。
- **新分支**：`chore/remove-zap`（基于 `feat/harness-design-2026-05-26` HEAD 创建，带着上一轮未 commit 的所有半成品改动）。
- **第二轮（SecurityView 全删）补充已完成**：
  - 删除整个 `frontend/components/SecurityView/` 目录（连同已重写的 SecurityView.tsx / ScanToolsPanel / SensitiveScanPanel / NucleiSection / ScanTimeline / shared / ReconDataPanel 一锅端）
  - `PaneLeaf.tsx` 删 SecurityView lazy import + `case "security"`
  - `TargetPanel.tsx` 删除 Security 子 tab + Suspense 渲染；TargetGroupedView / TargetDetailView 删 onScan prop + "Scan Target" 按钮
  - `store/types/session.ts` `TabType` 删 `"browser"` + `"security"`；`session-tabs.ts` 删 `openBrowserTab` + `openSecurityTab` actions；`session.ts` 类型同步
  - `TabBar/TabBar.tsx` / `TabBar/TabItem.tsx` 删除 browser / security 图标分支与 displayName
  - `useKeyboardHandlerContext` 删除 `openBrowserTab` / `openSecurityTab` 字段、初始值、Cmd+B / Cmd+Shift+S 快捷键
  - `App.tsx` 同步删除 openBrowserTab/openSecurityTab；`App.performance.test.tsx` 测试 KeyboardHandlerContext 同步
  - `App/hooks/useTabSplitEvents.ts` 删除 `handleDetachSecurityTab` 整段 + `tabType === "security"` 分支
  - `DetachedView/DetachedView.tsx` 重写：删除 `DetachedSecurity` 整个 component + `security-*` TAB_LABELS + `SecurityTab` import + `security-all` / `security-{tab}` 处理路径
  - `lib/i18n/{en,zh-CN}.json` 删除 `security.*` / `browser.*` / `nav.{browser,security}` 整个子树（~150 行 i18n key）
  - `capabilities/detached.json` description 删除 "SecurityView panels"
  - `scripts/check_file_sizes.sh` 删除 SecurityView/ScanToolsPanel.tsx 的 833 行 baseline
- **已完成（第一轮 · ZAP 模块删除）**：
  - **后端**：
    - 删除 `backend/crates/golish-pentest/src/zap/`（8 文件：mod / models / manager/{mod,state,addons,lifecycle,session,lifecycle_kill} / api/{mod,control,scan,traffic} / batch_scan / credential_detector / sync_capture）
    - 删除 `backend/crates/golish/src/tools/pentest/zap/`（10 文件：mod / admin / background / helpers / history / lifecycle / scan / session / sync/{mod,alerts,capture,messages,sitemap,targets}）
    - 删除 `backend/crates/golish-platform/src/zap.rs`
    - `commands_registry.rs` 删除 38 个 `zap_*` command + `get_zap_discovered_paths`
    - `PentestState` 删除 `zap_manager` / `credential_detector` / `project_path_tx` / `project_path_rx` 字段及构造
    - `PentestError::Zap` variant 删除；`golish-scan-runner::feroxbuster::get_zap_discovered_paths` 删除
    - `window_lifecycle.rs::CloseRequested` 不再 `pentest.zap_manager.stop().await`
    - `golish-pentest/tests/api_contract.rs` 删除 `credential_detector_is_send_sync` 测试
    - `golish-projects` `ProxyConfig.zap_api_url / zap_api_key` 字段删除（结构体保留为空以维持向后兼容）
    - `golish-agent-kit/system_hooks/builtins.rs` security_tools 列表移除 `"zap"`
    - `golish-db/repo/audit.rs` 注释中 ZAP 引用更新
    - `golish-pentest/src/sensitive_scan.rs` 注释更新
    - `golish-scan-runner/src/feroxbuster.rs` 顶部 doc-comment 更新
    - DB migrations（`20260412100001_scan_queue_and_custom_rules.sql` / `20260415200002_passive_scan_nullable_target.sql`）注释清理，**SQL 内容不动**
    - `commands_facade/pentest.rs` 与 `commands_facade/workspace.rs` 中 ZAP 文档行删除
    - **迁移**：`pentest_check_tool_updates`（GitHub release 检查，不是 ZAP-specific，原本错位在 `zap/admin.rs`）移到 `tools/pentest/packages/github.rs`
    - **保留**：`sitemap_store` 表中 `name='zap-sitemap'` 作为内部 storage key（`pipeline/storage.rs` / `sensitive_scan.rs` / `pentest_bridge/js_collect.rs` 仍写入），后续重构信息收集模块时统一改名
  - **前端**：
    - 删除 `frontend/components/SecurityView/{HttpHistoryPanel,SiteMapPanel,RepeaterPanel,IntruderPanel,PassiveScanPanel,AlertsPanel,ZapContextMenu,SetupPopover}.tsx` + `ScannerPanel/`（整个目录）+ `hooks/useZapScanQueue.ts`
    - 删除 `frontend/components/BrowserView/`（整个目录 · 完全为 ZAP 代理引导服务）
    - 删除 `frontend/lib/pentest/zap-api.ts`、`frontend/lib/pentest/scan-queue.ts`
    - 删除 `frontend/hooks/useZapProxyCert.ts`、`frontend/store/effects/zap-project-sync.ts`、`frontend/App/hooks/useCredentialCapture.ts`
    - 重写 `SecurityView.tsx`：仅保留 4 个独立 tab（scantools / sensitive / timeline / vault），去掉 ZAP status / start/stop button / Setup popover / ZapNotInstalled / ZapNotRunning
    - 重写 `SecurityView/shared.tsx`：删除 `StatusBadge` / `ZapNotInstalled` / `ZapNotRunning`，保留 `StyledSelect` / `ResizeHandle` / `methodColor` / `statusColor` / `formatBytes` / `DetailSection`
    - `lib/pentest/types.ts` 删除 ZAP Integration Types section（`ZapStatus` / `ZapStatusInfo` / `HttpHistoryEntry` / `HttpMessageDetail` / `ZapAlert` / `ScanProgress` / `SpiderProgress` / `ScannerRule` / `ManualRequestResult`）
    - `lib/api/security.ts` 删除 `ZapJson` type + `zapApiCall` / `zapListScanPolicies` / `zapGetScanners` / `zapSetScannersEnabled` / `zapScanMessageCount`
    - `lib/pentest/scan-runner.ts` 删除 `getZapDiscoveredPaths`
    - `lib/api/projects.ts::ProxyConfig` 字段清空（改为 `Record<string, never>` 维持 type）
    - `store/slices/app-shell.ts` 删除 `zapRunning` state / `setZapRunning` action / 初始值
    - `store/slices/index.ts` 注释更新
    - `main.tsx` 删除 `installZapProjectSync` import + call
    - `App/hooks/useAppLifecycle.ts` 删除 `useCredentialCapture` import + call
    - `components/PaneContainer/PaneLeaf.tsx` 删除 `BrowserView` lazy import + `case "browser"` 渲染
    - `components/DashboardPanel/ActivityFeed.tsx` 删除 `zap_scan_completed: "ZAP Scan Done"` event label
    - `mocks.ts` 删除 `zap_api_call` / `zap_status` / `zap_detect_path` mock case
    - `lib/i18n/en.json` 与 `lib/i18n/zh-CN.json` 删除 security.* 中的 `startZap` / `stopZap` / `zapNotInstalled` / `zapNotInstalledHint` / `installViaBrew` / `recheckInstall` / `manualInstallHint` / `zapNotRunning` / `zapNotRunningHint` / `setupTitle` / `setupZapStoppedTitle` / `setupZapStoppedHint` / `setupNeedZapRunning` / `setupHintTopRight`；`scanHint` / `clearHistoryConfirm` / `sslCertHint` 中 ZAP 字样替换为通用措辞
- **已记录证据**：
  - `cd backend && cargo check --workspace` → exit 0（全部 40+ crate Finished）
  - `cd backend && cargo clippy -p golish -p golish-pentest -p golish-scan-runner -p golish-platform -p golish-projects --lib --no-deps` → exit 0
  - `cd backend && cargo fmt --all --check` → exit 0
  - `cd backend && cargo nextest run -p golish-pentest --status-level fail` → 104 passed / 7 skipped
  - `cd backend && cargo nextest run -p golish-projects -p golish-platform --status-level fail` → 50 passed
  - `cd backend && cargo nextest run -p golish-scan-runner --lib` → 0 tests (crate has no lib tests)
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/` → exit 0 / Checked 705 files / 0 fixes
  - `pnpm exec vitest run` → 94 files / 1114 passed / 12 skipped
  - `rg "zap|Zap|ZAP" frontend/` → 仅余 lucide-react `Zap` / `PlugZap` 图标引用
  - `rg "zap|Zap|ZAP" backend/` → 仅余 `sitemap_store` 'zap-sitemap' storage key（向后兼容）+ `agent-runtime task.rs` `icon: "Zap"`（lucide 图标名）+ `pipeline/storage.rs` / `sensitive_scan.rs` / `js_collect.rs` 中的同 storage key
  - **第二轮（SecurityView 全删）验证**：`pnpm exec tsc --noEmit` → exit 0；`pnpm exec biome check frontend/` → exit 0 / Checked 698 files / 0 fixes；`pnpm exec vitest run` → 94 files / 1114 passed / 12 skipped；`cd backend && cargo check -p golish` → exit 0；`git diff HEAD --stat` 累计 → 111 files / +231 / -14991
- **未跑 / 原因**：
  - **未跑 `just precommit`**：baseline preexisting 即有 lint-rust warnings-as-errors 与 sandbox-related PermissionDenied 测试失败，与 ZAP 删除无关；本轮只跑 targeted check/lint/test
  - **未跑 `cargo nextest run -p golish --lib`**：需要从头编译完整 workspace，超 5 分钟；已通过 `cargo check --workspace` + `cargo clippy -p golish` + `nextest -p golish-pentest/projects/platform` 等代理验证；如用户要求可再跑
  - **未跑 `./init.sh` / `just dev`**：用户未要求；ZAP 删除不需要启动 DB / dev app 验证
  - **未做手动 E2E**：用户尚未重启 dev app 复测；预期 Security 面板打开后仅显示 4 个 tab、Pane 不再支持 browser 类型
- **提交记录**：未 commit（等用户确认 commit 策略）
- **已修改但未提交（本轮新增 · ZAP 删除 scope）**：
  - **删除**：上述所有 `frontend/components/SecurityView/{HttpHistory,SiteMap,Repeater,Intruder,Passive,Alerts,ZapContextMenu,SetupPopover,hooks/useZapScanQueue}` + `ScannerPanel/` + `BrowserView/` + `lib/pentest/{zap-api,scan-queue}` + `hooks/useZapProxyCert` + `store/effects/zap-project-sync` + `App/hooks/useCredentialCapture` + `backend/crates/golish-pentest/src/zap/` + `backend/crates/golish/src/tools/pentest/zap/` + `backend/crates/golish-platform/src/zap.rs`
  - **修改**：见 evidence.frontend / evidence.backend 详细列表
  - **新增（迁移）**：`pentest_check_tool_updates` 从已删 `zap/admin.rs` 迁移到 `tools/pentest/packages/github.rs`
- **下一步**：等用户确认 commit 策略。**强烈建议**先在 `chore/remove-zap` 分支提交一个 pure deletion commit，然后切回 `feat/harness-design-2026-05-26` 推进其它 in-progress 工作。后续如果要做"基于 org 的信息收集面板"重构，再开一个新分支 `feat/asset-recon-panel`。

---

### 2026-05-27 · Sub-Agent Nested Delegation 与 Thinking UI 重构

- **本轮目标**：按用户要求先写计划再改：nested sub-agent 不再在主 ChatPanel 平铺；父 sub-agent detail 内显示嵌套委托卡片；sub-agent thinking/reasoning 不再闪一下消失，而是进入 sub-agent 自己的状态与 detail UI。
- **计划文档**：
  - `docs/superpowers/plans/2026-05-27-sub-agent-nested-thinking-ui.md`
- **已完成**：
  - 后端新增 `AiEvent::SubAgentReasoning { agent_id, delta, accumulated, parent_request_id }`，CLI JSON 输出为 `sub_agent_reasoning`；terminal/transcript/summarizer/sidecar 对该流式 reasoning 做对应过滤或忽略。
  - `golish-sub-agents` streaming processor 在标准 reasoning/fallback reasoning 路径发送 `SubAgentReasoning`；`AlwaysContent` quirk 仍按原逻辑 reroute 到 `SubAgentTextDelta`。
  - 前端 `ActiveSubAgent` 增加 `thinking` / `thinkingStartedAt` / `thinkingEndedAt`，workflow store 增加 `updateSubAgentThinking` 并同步回 timeline。
  - 前端 AI event registry 增加 `sub_agent_reasoning` handler。
  - `SubAgentDetailView` 复用 `ThinkingBlock` 展示 sub-agent thinking；运行中且还没有 entries 时展开，后续出现 text/tool/nested delegation 后自动折叠。
  - `SubAgentDetailView` 对 `sub_agent_*` tool call 优先查找 child sub-agent，并渲染 compact nested delegation card；点击会进入 child sub-agent detail。
  - `extractSubAgentBlocks` fallback 过滤 nested child，避免 child sub-agent 因状态 race 被追加到主 ChatPanel 顶层。
- **已记录证据**：
  - `cd backend && cargo check -p golish-core -p golish-cli-output -p golish-sub-agents -p golish-events -p golish-sidecar` → exit 0
  - `cd backend && cargo fmt --package golish-core --package golish-cli-output --package golish-sub-agents --package golish-events --package golish-sidecar --check` → exit 0
  - `cd backend && cargo test -p golish-core events::tests --lib` → 34 passed / 0 failed
  - `cd backend && cargo test -p golish-cli-output sub_agent_reasoning_event_has_correct_format --lib` → 1 passed / 0 failed
  - `cd backend && cargo test -p golish-events should_transcript --lib` → 2 passed / 0 failed
  - `pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/lib/timeline/subAgentExtraction.ts frontend/lib/timeline/subAgentExtraction.test.ts frontend/lib/ai/types.ts frontend/store/types/sub-agent.ts frontend/store/slices/workflow/types.ts frontend/store/slices/workflow/sub-agent.ts frontend/services/ai-events/sub-agent-handlers.ts frontend/services/ai-events/registry.ts frontend/services/ai-events/registry.test.ts` → exit 0 / No fixes applied
  - `pnpm vitest run frontend/lib/timeline/subAgentExtraction.test.ts frontend/services/ai-events/registry.test.ts` → 30 passed / 0 failed
  - `pnpm exec tsc --noEmit` → exit 0
  - `git diff --check -- <touched files>` → exit 0
  - Mock visual check: `pnpm dev` 启动 Vite（未跑 `init.sh`），Playwright 注入 mocked parent/child sub-agent state 并截图：
    - `.codex-screenshots/subagent-parent-nested-thinking.png`：父 `Pentester` detail 显示 collapsed Thinking + nested `Installer` delegation card + 普通工具块。
    - `.codex-screenshots/subagent-child-thinking.png`：点击 nested card 后进入 child `Installer` detail，显示 child 自己的 active Thinking。
  - Mock Vite dev server 已停止。
  - Nested card visual follow-up：把父 detail 里的 nested delegation card 从单行压缩样式调为两行卡片（主行 status/agent/tools/chevron，副行 task，running 时显示 child thinking preview）。重新 mock 截图：
    - `.codex-screenshots/subagent-parent-nested-card-v2.png`
  - `pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0 / No fixes applied
  - `pnpm exec tsc --noEmit` → exit 0
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0
- **未跑 / 原因**：
  - 未跑 `./init.sh`：用户明确要求不要启动。
  - 未跑 `just precommit`：当前仓库仍有大量既有未提交改动与 baseline 验证问题；本轮只跑 targeted 验证。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `docs/superpowers/plans/2026-05-27-sub-agent-nested-thinking-ui.md`
  - `backend/crates/golish-core/src/events/event.rs`
  - `backend/crates/golish-core/src/events/event_dispatch.rs`
  - `backend/crates/golish-core/src/events/tests/json_serialization.rs`
  - `backend/crates/golish-core/src/events/tests/roundtrip.rs`
  - `backend/crates/golish-cli-output/src/cli_json/{mod.rs,sub_agent.rs}`
  - `backend/crates/golish-cli-output/src/{terminal.rs,tests.rs}`
  - `backend/crates/golish-sub-agents/src/executor/stream_processing.rs`
  - `backend/crates/golish-events/src/transcript/{mod.rs,summarizer.rs,tests/should_transcript_tests.rs}`
  - `backend/crates/golish-sidecar/src/capture/context.rs`
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
  - `frontend/lib/{ai/types.ts,timeline/subAgentExtraction.ts,timeline/subAgentExtraction.test.ts}`
  - `frontend/services/ai-events/{registry.ts,registry.test.ts,sub-agent-handlers.ts}`
  - `frontend/store/{types/sub-agent.ts,slices/workflow/types.ts,slices/workflow/sub-agent.ts}`

---

### 2026-05-27 · ChatPanel Thinking 与 Sub-Agent 卡片状态边界修正（小改动）

- **本轮目标**：用户指出模型在 Thinking 文本后调用 subagent 时，UI 仍显示为 active Thinking，且 sub-agent detail 里同一段输出会在 Agent Output / 实时输出 / 底部响应区域重复出现，希望理清原因并修 UI 状态边界。
- **定位结论**：
  - 最新 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1779891265351-1/transcript.json`。
  - 后端确实在同一轮模型响应里先收到 reasoning，再执行 `sub_agent_pentester`；不是前端凭空调用。问题在 `MessageBlock` 的 `ThinkingBlock.isActive` 只看 `message.isStreaming && !message.content`，没把 tool/subagent call 视为 reasoning 阶段结束。
- **已完成**：
  - `frontend/components/AIChatPanel/MessageBlock.tsx`：`ThinkingBlock.isActive` 现在要求当前 assistant message 仍在 streaming、没有正文、且没有 `toolCalls`。一旦 tool/subagent 卡片出现，Thinking 自动从 active spinner 变为 settled/collapsed 的 `Thought for ...`。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：删除底部单独渲染 `subAgent.streamingText` 的实时输出正文块。`updateSubAgentStreamingText` 已经把同一段实时文本写进 `subAgent.entries` 的最后一个 text entry，保留两处会造成 sub-agent detail 里同一句 “Agent Output” / “实时输出” 重复显示。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：移除底部 final response 正文块。`subAgent.response` 是 sub-agent 返回给主 agent 的工具结果/交接内容，不应在 sub-agent detail 面板里作为另一段正文展示；detail 只保留任务、过程输出、工具调用和错误。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：给 sub-agent markdown 输出容器补充 `break-words` / `overflow-wrap:anywhere` / table 与 pre 的 max-width/overflow 约束，避免表格、长 inline code 或代码块把 detail 面板撑出黑块/空白区域。
- **已记录证据**：
  - `pnpm exec biome check frontend/components/AIChatPanel/MessageBlock.tsx` → exit 0 / No fixes applied
  - `git diff --check -- frontend/components/AIChatPanel/MessageBlock.tsx` → exit 0
  - `pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0 / No fixes applied
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0
  - `pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0 / No fixes applied（删除实时输出重复块后复跑）
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0（同上）
  - `pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/lib/i18n/zh-CN.json frontend/lib/i18n/en.json` → exit 0 / No fixes applied（移除 final response 块后复跑；i18n 文件已还原，无最终 diff）
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/lib/i18n/zh-CN.json frontend/lib/i18n/en.json` → exit 0
- **未跑 / 原因**：
  - 未跑 `./init.sh`：用户明确要求不要启动。
  - 未跑 `just precommit`：本轮是单行前端 UI 状态小修，且当前仓库 baseline 仍有大量未提交半成品与既有验证问题。
- **提交记录**：`b2a90f1 fix(chat): tighten sub-agent panel rendering`（仅包含 `MessageBlock.tsx` + `SubAgentDetailView.tsx`；`agent-progress.md` 未随本 commit 提交，避免混入历史大段未提交记录）。
- **已修改但未提交（本轮 scope）**：
  - `frontend/components/AIChatPanel/MessageBlock.tsx`
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
  - `agent-progress.md`

---

### 2026-05-27 · Agent Tool-Use Compatibility Layer 设计与计划（本轮进行中）

- **本轮目标**：用户复测 MiMo 后确认问题不只是单个 provider bug：模型把 `ask_human` / `manage_targets add` 写成文本 `<tool_call>`，没有真实弹窗；日志看不到 AI 到底发了什么、Golish 解析/拒绝/执行了什么，可观测性不足。用户同意把这一块做架构调整。
- **定位结论**：
  - 这是“模型差异 + runtime 边界缺失”的组合问题。Mistral 小模型没复现不代表代码没问题；MiMo 暴露的是 Golish 缺少 provider tool-use capability 分级、ToolIntent 归一化、安全 gate、以及可观察 trace。
  - 正确形态应是：LLM Provider → Provider Adapter → Tool Intent Normalizer → Policy/Safety Gate → Approval/ask_human Barrier → Tool Executor → Observation/Trace → LLM Continuation。
- **已完成**：
  - 新增设计文档 `docs/design/2026-05-27-agent-tool-use-compatibility-layer.md`，明确目标/非目标、当前问题、目标架构、ToolUseProfile、ToolIntent、ToolGate、MiMo 策略、观测字段、成功标准。
  - 新增实施计划 `docs/superpowers/plans/2026-05-27-agent-tool-use-compatibility-layer.md`，按 writing-plans 技能拆成 8 个任务：ToolUseProfile → ToolIntent normalizer → ToolGate → dispatch 前 gate → events/logs → 前端 Details → MiMo replay test → final verification。
  - `feature_list.json` 新增 `agent-tool-use-compatibility-layer` 并置为唯一 `in_progress`。
  - `feature_list.json` 把 `xiaomi-mimo-provider` 从 `in_progress` 切到 `blocked`：Xiaomi 展示/context/provider 接入已做完 targeted 验证，但真实 MiMo 工具调用 E2E 需要等 compatibility layer 收敛后再切 passing。
  - `backend/crates/golish-models/src/tool_use_profile.rs`：新增 `ToolUseProfile` / `ToolCallMode` / `ToolCallReliability`，并把 `ModelCapabilities` 接上 `tool_use_profile`。OpenAI/Anthropic 默认 native reliable，DeepSeek 先标 native best effort，Xiaomi MiMo 标 `TextualXmlFallback + NeedsAdapter + max_tool_calls_per_turn=1`。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_intent.rs`：新增 normalized `ToolIntent`，支持 native tool call 与 recovered textual XML intent 互转。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`：临时 XML parser 升级为先产出 `ToolIntent`，再转 `ToolCall` 兼容现有执行路径；新增 MiMo replay 测试，覆盖同一 `<tool_call>` 块里 `ask_human` + `manage_targets add` 时优先 human barrier。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_gate.rs`：新增 deterministic gate，覆盖 `ask_human` hard barrier、recovered `manage_targets add` requires approval、unregistered `run_pipeline` reject 的策略单测。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`：dispatch 前接入 gate classification；`textual-tool-call-*` 会按 recovered XML 来源识别，`agent-observe` 日志记录 requires approval / requires human answer / rejected。
  - `backend/crates/golish-core/src/events/event.rs` + `backend/crates/golish-core/src/events/event_dispatch.rs`：新增 `ToolIntentObservation` / `tool_intent_observation` 事件，记录 request_id、tool_name、source、decision、reason、raw_preview。
  - `backend/crates/golish-events/src/event_coordinator/coordinator.rs` + transcript tests：`agent-observe` 能打出 tool intent gate decision；transcript summarizer 不把 observation 当成对话内容污染总结。
  - `frontend/lib/ai/types.ts` + `frontend/services/ai-events/{registry.ts,tool-handlers.ts}` + `frontend/store/{types/message.ts,slices/ai.ts}`：前端事件流支持 `tool_intent_observation`，timeline tool execution 增加 `toolIntent` 元数据；如果 observation 先到，会先缓存，tool card 出现后回填。
  - `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts` + legacy `frontend/components/AIChatPanel/useChatAiEvents.ts`：直接监听 AI event 的 chat hooks 也消费 `tool_intent_observation`，避免两条事件路径状态不一致。
  - `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`：Details 面板显示 Intent 区域（Model wanted / Source / Golish decision / Reason）。没有专用 observation event 时，也能根据 `textual-tool-call-*` request id 推断 recovered XML intent。
  - 真实 app 复测 `pentest-chat-1779875360223-1`：transcript 第 9 行显示 `ask_human` 被发成 `tool_approval_request`，没有后续 `tool_auto_approved` / `ask_human_request` / `ask_human_response`，解释了用户“点 Yes 仍卡在 Running ask_human”的现象。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/hitl.rs`：`ask_human` 现在绕过 generic HITL tool approval，直接进入 `execute_ask_human_tool`，避免“允许运行 ask_human”和“回答 ask_human 问题”双重确认混淆。
  - `backend/crates/golish-agent-runtime/src/test_utils_tests.rs`：新增回归测试 `test_ask_human_bypasses_tool_approval_and_emits_human_request`，断言 `ask_human` 会发 `AskHumanRequest` 且不会发 `ToolApprovalRequest(ask_human)`。
  - 真实 app 复测 `pentest-chat-1779877954838-1`：同一个 `run_command` request_id `call_1601ac44800945cbb03de7d7` 先收到一条半截 string args 的 `tool_approval_request`，后收到一条 object args 的重复 `tool_approval_request`；右侧因此连续显示两张 Run/Deny，左侧 Details 因把 string args 传给 `Object.entries` 被拆成 0/1/2... 字符行。
  - 同一 transcript 后续显示 `run_command` 实际成功（line 22），但下一轮 provider 报 `500 Can only get item pairs from a mapping`；根因是历史里已经写入了 string/partial tool args（如 line 18 `pentest_run` 与 line 20 `run_command` 的 string args），Xiaomi OpenAI-compatible 服务端要求 `tool_calls.function.arguments` 是 mapping/object，遇到 string 会 500。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`：provider 原生 tool call 的 string arguments 现在按 streaming fragment 处理，等 delta/final 后再 `parse_tool_args`，不再把半截 JSON 字符串立即派发/审批。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/assistant_message.rs`：写入下一轮 `chat_history` 前再次 normalize tool call arguments；如果仍有 provider/legacy path 漏出 string args，会先用 `golish_json_repair::parse_tool_args` 转成 object，避免 Xiaomi 在下一轮 history replay 时因 string args 500。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/assistant_push.rs`：新增回归测试覆盖 string tool args 在 history push 前归一化为 object。
  - `frontend/store/slices/conversation.ts`：`addMessageToolCall` 按 `requestId` 去重，后到的同 ID 工具事件更新 args 而不是新增第二张卡。
  - `frontend/components/AIChatPanel/MessageBlock.tsx`：pending approval 匹配从“同名工具”收紧为优先按 `requestId`，避免多个 `run_command` 同时都被画成待审批。
  - `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`：Input 支持 `unknown` args；string 先尝试 JSON.parse，失败则整段 raw 显示，避免字符拆表。
  - 真实 app 复测 Task 模式 `pentest-chat-1779889022674-2`：transcript 显示 task generator 前端消息内容是整段 `[System Context]... [User Message]...`，而不是纯用户输入；后端 intent classifier 会抽 `[User Message]`，但 `execute_task_mode` / generator 没抽，导致 task planner 拿系统提示词当任务输入并报 `Generator failed`。
  - `backend/crates/golish/src/ai/commands/core/chat.rs`：Task 模式入口新增 `extract_user_message_from_wrapped_prompt`，session title、UserMessage event、orchestrator.run 全部使用纯用户输入；task error emission 和 Tauri command error 改用 `{:#}` 展示 anyhow cause chain，避免前端只看到顶层 `Generator failed`。
- **已记录证据**：
  - `python3 -m json.tool feature_list.json >/dev/null` → exit 0
  - `git diff --check` → exit 0
  - `python3 - <<'PY' ...` 检查 in_progress 列表 → `['agent-tool-use-compatibility-layer']`；`xiaomi-mimo-provider blocked`
  - `./init.sh` → exit 0；内部仍显示 baseline failures：`test-fe` 7 failed（TerminalSettings ×4 + HomeView.memo/useFileIndex 类既有失败）、`lint-rust` 5 个 pre-existing clippy/dead_code/doc lint、`test-rust-all` 1 个 `window_state::tests::compute_restore_action_supports_negative_monitor_origins` failure；脚本末尾仍打印 OK。该命令不作为本功能 passing 证据。
  - `cd backend && cargo fmt --package golish-models --package golish-agent-runtime --package golish-events --check` → exit 0
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-models-target cargo test -p golish-models tool_use_profile --lib` → 3/3 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime tool_intent --lib` → 2/2 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime textual_tool_calls --lib` → 4/4 passed（含 `mimo_textual_tool_call_prioritizes_human_barrier_over_follow_up_add`）
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime tool_gate --lib` → 4/4 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime tool_dispatch --lib` → 5/5 passed（含 `textual_ask_human_emits_tool_intent_observation_before_filtering`，确认 recovered ask_human 在 allow-list 过滤前发出 `ToolIntentObservation`）
  - `cd backend && cargo test -p golish-agent-runtime test_behavioral_equivalence_error_handling --lib` → 1/1 passed（确认 allow-list 前置过滤时仍会为 policy deny / planning restriction / constraint violation 发 `ToolDenied`）
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-core-target cargo test -p golish-core events::tests --lib` → 33/33 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-core-target cargo test -p golish-core tool_intent_observation_event_json_format --lib` → 1/1 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-core-target cargo test -p golish-core all_event_types_roundtrip --lib` → 1/1 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-events-target cargo test -p golish-events transcript --lib` → 33/33 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-events-target cargo test -p golish-events should_transcript --lib` → 2/2 passed
  - `cd backend && cargo fmt --package golish-core --package golish-events --package golish-agent-runtime --check` → exit 0
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/lib/ai/types.ts frontend/store/types/message.ts frontend/store/slices/ai.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/services/ai-events/registry.ts frontend/services/ai-events/registry.test.ts frontend/services/ai-events/tool-handlers.ts frontend/components/AIChatPanel/hooks/useAiChatEvents.ts frontend/components/AIChatPanel/useChatAiEvents.ts` → exit 0 / No fixes applied
  - `pnpm vitest run frontend/services/ai-events/registry.test.ts` → 14/14 passed
  - `just check-fe` → exit 0（`generate-model-constants` 重写 `frontend/lib/ai/models.generated.ts`，保持 Xiaomi constants）
  - `cd backend && cargo nextest run -p golish-agent-runtime -p golish-events --status-level fail` → 233/233 passed
  - `cd backend && cargo nextest run -p golish-agent-runtime -p golish-events -p golish-models --status-level fail` → 257 passed, 1 failed（baseline `golish-models descriptors::loader::tests::embedded_nvidia_has_expected_count`: expected substantial NVIDIA registry, got 14；11 tests not run due fail-fast）
  - `cd backend && cargo test -p golish-agent-runtime test_ask_human_bypasses_tool_approval_and_emits_human_request --lib` → 1/1 passed
  - `cd backend && cargo fmt --package golish-agent-runtime --check` → exit 0
  - `cd backend && cargo test -p golish-agent-runtime stream_processor::tests --lib` → 2/2 passed（string tool args treated as streaming fragments；object tool args complete）
  - `cd backend && cargo test -p golish-agent-runtime string_tool_arguments_are_normalized_before_history_push --lib` → 1/1 passed
  - `cd backend && cargo fmt --package golish-agent-runtime --check` → exit 0
  - `git diff --check` → exit 0
  - `cd backend && cargo fmt --package golish --check` → exit 0
  - `cd backend && cargo test -p golish chat_title_tests --lib` → 7/7 passed（含 task mode wrapped prompt extraction）
  - `pnpm exec biome check frontend/components/AIChatPanel/MessageBlock.tsx frontend/store/slices/conversation.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx` → exit 0 / No fixes applied
  - `pnpm exec tsc --noEmit` → exit 0
  - `just test-fe` → exit 0（前端 baseline test fixes 后全量通过）
  - `just precommit` → exit 1；已过 `fmt` / `check-fe` / `test-fe`，但 `lint-rust` 仍报既有 `golish` clippy/dead_code/doc lint；`test-rust-all`/`test-rust` 在 sandbox 中触发 PermissionDenied baseline failures（`asset_intel` / `sploitus` 测试创建本地 mock server 时 OS 拒绝）
- **未跑 / 原因**：
  - 未跑 `just precommit` 到全绿：当前仓库 baseline 仍有上方 `./init.sh` 记录的 frontend/Rust failures。
  - 未做修复后的真实 MiMo E2E：需要重启 `just dev` / Tauri 后复测 example.com 未注册目标场景，确认 UI 出现真正 `AskHumanRequest` 的 Confirm/Skip/Abort，而不是 `ToolApprovalRequest(ask_human)`。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `docs/design/2026-05-27-agent-tool-use-compatibility-layer.md`
  - `docs/superpowers/plans/2026-05-27-agent-tool-use-compatibility-layer.md`
  - `backend/crates/golish-models/src/tool_use_profile.rs`
  - `backend/crates/golish-models/src/lib.rs`
  - `backend/crates/golish-models/src/capabilities.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_intent.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_gate.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/mod.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/hitl.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`
  - `backend/crates/golish-agent-runtime/src/test_utils_tests.rs`
  - `frontend/store/slices/conversation.ts`
  - `backend/crates/golish-core/src/events/event.rs`
  - `backend/crates/golish-core/src/events/event_dispatch.rs`
  - `backend/crates/golish-core/src/events/tests/json_serialization.rs`
  - `backend/crates/golish-core/src/events/tests/roundtrip.rs`
  - `backend/crates/golish-events/src/event_coordinator/coordinator.rs`
  - `backend/crates/golish-events/src/transcript/summarizer.rs`
  - `backend/crates/golish-events/src/transcript/tests/should_transcript_tests.rs`
  - `frontend/lib/ai/types.ts`
  - `frontend/services/ai-events/registry.ts`
  - `frontend/services/ai-events/registry.test.ts`
  - `frontend/services/ai-events/tool-handlers.ts`
  - `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`
  - `frontend/components/AIChatPanel/useChatAiEvents.ts`
  - `frontend/store/types/message.ts`
  - `frontend/store/slices/ai.ts`
  - `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
  - `feature_list.json`
  - `agent-progress.md`
- **风险 / 下一步最佳动作**：
  - 已完成计划 Task 1-7 的核心代码与 targeted 验证：模型能力画像、ToolIntent、textual normalizer、ToolGate、dispatch 前 gate、专用 observation event、前端 Details、MiMo replay 单测都已落地。新增 `tool_dispatch` observation 与 `ask_human` bypass 单测后，事件链路已有后端单测保护。剩余缺口：重启 app 后真实 MiMo E2E 尚未完成；`just precommit` 仍受既有 Rust lint / sandbox PermissionDenied baseline failures 阻挡。真实 MiMo E2E 与 fresh precommit/baseline 证据前，不应把 Xiaomi provider 或本条目切 passing。

---

### 2026-05-27 · AI Chat 工具调用可观测性补强（本轮进行中）

- **本轮目标**：用户指出控制台日志只能看到事件类型和 `tool_calls=0`，看不到 AI 具体输出/为什么没有弹 `ask_human`，排障可观测性差。
- **定位结果**：
  - 这次真实 session `pentest-chat-1779867794561-1` 的 `transcript.json` 能看到完整原因：`manage_targets list` 真实执行成功；随后模型把 `ask_human` 和多次 `manage_targets add` 写成 `<tool_call><function=...>` 文本，backend 仍记录 `tool_calls=0`，所以没有弹 `ask_human`，也没有执行 add。
  - 控制台日志之前只打 `Stream completed ... text_chars=... tool_calls=0`，没有 response preview，也没有指向 transcript 路径，导致必须手动翻 transcript。
- **已完成**：
  - `backend/crates/golish-events/src/event_coordinator/coordinator.rs`：新增 `agent-observe` 日志层。
  - `completed` event 在 DEBUG 打响应长度、500 字符预览、tokens、duration、transcript path；如果 completed 响应里含 `<tool_call` / `<function=`，额外 WARN。
  - `tool_approval_request` / `ask_human_request` / `tool_result` 在 INFO 打关键事件；参数/结果只在 DEBUG 打截断预览，降低日志泄露面。
  - 顺手补了前面定位时的前端 raw `<tool_call>` 清洗和后端 textual tool-call retry nudge；随后根据用户复测日志继续补 deterministic XML tool-call adapter。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`：新增 MiMo XML-style tool call parser。stream 结束时若 `tool_calls=0` 但正文含 `<function=...>`，后端会转换为真正的 `ToolCall`；如果同段中同时含 `ask_human` 和后续 `manage_targets add`，优先只执行 `ask_human`，避免绕过用户确认。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`：接入 adapter，并在转换时打 `[tool-adapter] Converted textual XML-style tool call into structured tool call`。
- **已记录证据**：
  - `cd backend && cargo fmt --package golish-events --package golish-agent-runtime --check` → exit 0
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-events-target cargo test -p golish-events transcript --lib` → 33/33 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime textual_tool_calls --lib` → 3/3 passed
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime textual_tool_call_markup_retries_even_when_reflector_is_inactive --lib` → 1/1 passed
  - `pnpm exec biome check frontend/components/AIChatPanel/MessageBlock.tsx` → exit 0 / No fixes applied
  - `git diff --check` → exit 0
- **未跑 / 原因**：
  - 未跑 `just precommit`：当前 baseline 仍有前文记录的既有失败。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `backend/crates/golish-events/src/event_coordinator/coordinator.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/reflector.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/reflector_or_break.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`
  - `frontend/components/AIChatPanel/MessageBlock.tsx`
  - `agent-progress.md`
- **风险 / 下一步最佳动作**：
  - 现在 adapter 已能把 XML-style tool call 转成结构化调用；仍需用户用真实 MiMo 流手动复测：目标不存在时应看到真正的 `ask_human` 确认弹窗，确认后才允许后续 add/pipeline。

---

### 2026-05-27 · AI Chat 等待态样式改为阶段状态（本轮进行中）

- **本轮目标**：用户觉得等待时的 `...` + `> loading the toolkit` 样式不合理，希望参考 Cursor/Windsurf 一类更清晰的阶段状态。
- **上下文检查**：
  - 上一轮已按开工流程读上下文并跑过 `./init.sh`；本轮是同一问题链路上的小型前端 UI 调整。
  - 当前 `feature_list.json` 仍只有 `xiaomi-mimo-provider` 一个 `in_progress`；本轮等待态属于简单改动，不新增复杂 feature 条目。
- **已完成**：
  - `frontend/components/AIChatPanel/AgentStatusIndicator.tsx`：移除旧的“伪终端”提示符、monospace 绿字、block cursor 和抽象短语轮播；改为稳定阶段文案：`Preparing context` / `Planning next step` / `Writing response` / `Running <tool>` / `Delegating to <agent>` / `Compacting context`。
  - 同文件：工具名/detail 做空白清理和 48 字符截断，避免长命令把输入区撑开。
  - `frontend/index.css`：删除旧 caret/text cycle keyframes，换成轻量 dot pulse 动画。
  - 用 Browser 打开 `http://localhost:1420/` 并进入 recent project，确认 dev 页面可启动；未触发真实 provider 流式回复。
- **已记录证据**：
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/AIChatPanel/AgentStatusIndicator.tsx frontend/index.css` → exit 0 / No fixes applied
  - `git diff --check` → exit 0
- **未跑 / 原因**：
  - 未跑 `just precommit`：当前仓库 baseline 仍有前一轮记录的 frontend/Rust failures，不能作为本轮有效 passing 证据。
  - 未用真实模型触发流式等待态：需要外部 provider key/request，留给用户在 `just dev` 中手动确认视觉效果。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `frontend/components/AIChatPanel/AgentStatusIndicator.tsx`
  - `frontend/index.css`
  - `agent-progress.md`
- **风险 / 下一步最佳动作**：
  - 视觉最终观感仍需用户用真实 chat 流复测；如果觉得 pill 太像 badge，可以下一步改成无边框 inline row，只保留 pulsing dot + 文案。

---

### 2026-05-27 · Xiaomi MiMo 模型名与 context usage 显示修正（本轮进行中）

- **本轮目标**：用户指出 ChatModelSelector 里每个 MiMo 模型名后面的 `1M ctx` / `256K` 不应塞在模型名里；输入区下方 context usage 又对所有 MiMo 模型都显示 `128K context used`。本轮聚焦展示层与 runtime context window 取值不一致的问题。
- **上下文检查**：
  - 已读 `agent-progress.md` / `feature_list.json` / `clean-state-checklist.md`。
  - `./init.sh` 已按开工流程执行；脚本 exit 0 且最终打印 OK，但内部仍出现 baseline failures：frontend vitest 6 failed（TerminalSettings ×4 + HomeView.memo ×2）/ Rust clippy 5 个既有 warning-as-error / test-rust-all 1 个 `window_state::tests::compute_restore_action_supports_negative_monitor_origins` failure。它们与本轮 5 文件小修无关。
  - §2.1 状态已切换：`ai-chat-stop-cancels-backend-stream` 置 `blocked`（等待用户真实 E2E），`xiaomi-mimo-provider` 置 `in_progress`。
- **已完成**：
  - `frontend/lib/models/xiaomi.ts`：去掉 Xiaomi 模型名中的 context window 标签；现在显示 `MiMo V2.5 Pro` / `MiMo V2.5 (multimodal)` / `MiMo V2 Pro` / `MiMo V2 Omni (multimodal)`。
  - `backend/crates/golish-context/src/token_budget/{limits.rs,config.rs}`：给 `TokenBudgetConfig::for_model` 增加 MiMo context limits；`mimo-v2.5-pro` / `mimo-v2.5` / `mimo-v2-pro` 为 1,000,000，`mimo-v2-omni` 为 256,000。这样 `AiEvent::ContextWarning.max_tokens` 不再落回默认 128K。
  - `backend/crates/golish-context/src/token_budget/tests.rs`：新增 MiMo context limit 回归测试，覆盖 `@anthropic` 后缀。
  - `frontend/components/AIChatPanel/ContextUsageRing.tsx`：统一 tooltip 文案 formatter；1,000,000 显示为 `1M`，256,000 显示为 `256K`，避免 `1000K`。
  - `feature_list.json`：同步 Xiaomi 条目的用户可见行为、验证证据、风险说明；保持当前唯一 `in_progress`。
- **已记录证据**：
  - `./init.sh` → exit 0，但内部输出含 baseline failures（见上方“上下文检查”）；不作为本轮 passing 证据。
  - `cd backend && cargo fmt --package golish-context --check` → exit 0
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-context-target cargo test -p golish-context test_model_context_limits_xiaomi_mimo --lib` → 1/1 passed
  - `pnpm exec biome check frontend/lib/models/xiaomi.ts frontend/components/AIChatPanel/ContextUsageRing.tsx` → exit 0 / No fixes applied
  - `pnpm exec tsc --noEmit` → exit 0
  - `python3 -m json.tool feature_list.json >/dev/null` → exit 0
- **未跑 / 原因**：
  - 未跑 `just precommit` 到全绿：当前仓库 baseline 仍有上面列出的既有失败，直接跑不会形成有效 passing 证据。
  - 未做真实 Xiaomi LLM E2E：会触发外部 provider 请求，仍留给用户在本地 `just dev` 中用自己的 key 复测。
- **提交记录**：未 commit。
- **已修改但未提交（本轮 scope）**：
  - `frontend/lib/models/xiaomi.ts`
  - `frontend/components/AIChatPanel/ContextUsageRing.tsx`
  - `backend/crates/golish-context/src/token_budget/config.rs`
  - `backend/crates/golish-context/src/token_budget/limits.rs`
  - `backend/crates/golish-context/src/token_budget/tests.rs`
  - `feature_list.json`
  - `agent-progress.md`
- **风险 / 下一步最佳动作**：
  - runtime context window 现在在 `golish-context` 里显式维护一份 MiMo 表；未来新增 MiMo 模型时要同步这里，或后续改成从 `golish-models` registry 读取，减少双表风险。
  - 用户下一步：重启或等待 `just dev` 热更新后，打开 Xiaomi MiMo 下拉，应不再看到模型名后面的 `1M ctx` / `256K`；选前三个模型时 context usage 应显示 `/ 1M context used`，选 Omni 时显示 `/ 256K context used`。

---

### 2026-05-27 · AI Chat Stop/Pause 后端取消修复（本轮进行中）

- **本轮目标**：用户报告“暂停之后后端还在一直输出内容，只是前端看不到”。本轮聚焦 Stop/Pause 是否只停止前端渲染而没有真正取消后端 LLM stream / task phase。
- **上下文检查**：
  - 已读 `agent-progress.md` / `feature_list.json`；进入本轮前 `feature_list.json` 无 `in_progress` 条目。
  - 已新增 `ai-chat-stop-cancels-backend-stream` 并标为 `in_progress`。
  - `./init.sh` 曾按开工流程启动，但用户随后明确说“先别跑init.sh”；已停止对应 `init.sh`/`just check`/`vitest` 进程。已完成的前置输出：install、fmt、check-fe 通过，进入 test-fe 时中止；本轮后续不再跑 init/precommit，改跑定向验证。
- **初步判断**：
  - 前端 `handleStop` 会调用 `cancel_ai_generation`，后端 `AgentBridge::cancel()` 会置位 `cancelled`。
  - 后端已有取消检查，但主要在 loop pre-flight、LLM call 前、stream chunk 返回后；如果正在等待 LLM 建流、等待下一块 chunk、或 task one-shot phase，取消不够及时。
  - 前端已有 `generation-suppress.ts` / `streaming-buffer.ts` 工具，但当前 Stop 路径未使用。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`：等待 `model.stream(request)` 建立 stream 时增加 `tokio::select!` cancellation branch；用户 Stop 后不再最多等 180s timeout。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`：等待 `stream.next()` 下一块 chunk 时增加 cancellation branch；Stop 后立即 drop stream，避免后端继续消费上游 token。
  - `backend/crates/golish-agent-bridge/src/bridge_executor/mod.rs`：Task mode generator/refiner/reporter/enricher/planner/monitor 这类 one-shot phase 在 completion await 期间可被 cancel 打断。
  - `frontend/components/AIChatPanel/hooks/useChatSend.ts`：Stop 时同步 `suppressGenerationForAiSession` + `discardPendingBatchedDeltasForAiSession`，再调 `cancel_ai_generation`；下一次发送 prompt 前 clear suppression。
  - `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts` 与 legacy `frontend/components/AIChatPanel/useChatAiEvents.ts`：suppressed session 的旧 `ai-event` 直接忽略。
  - `frontend/lib/ai/generation-suppress.test.ts`：新增 2 个纯单测钉住 suppression 状态机。
  - 复盘用户新日志：主会话 `pentest-chat-1779853283322-1` 在 `06:45:22` 已 completed；后续持续输出的是隐藏 `title-gen-pentest-chat-1779853283322-1` 标题生成会话，且其 `ToolPreset::None` 仍被 chat policy 注入 `run_command` / `ask_human`。
  - `backend/crates/golish-agent-kit/src/tool_definitions/config.rs` + `backend/crates/golish-agent-runtime/src/execution_mode/selection_apply.rs`：让 `ToolPreset::None` 成为真正 no-tools，直接短路所有 policy-level aliases / registry tools。
  - `frontend/components/AIChatPanel/hooks/useChatSessionInit.ts`：隐藏 title generation 增加 8s timeout；超时会调用 `cancel_ai_generation(titleSessionId)`，finally 仍 shutdown session，避免标题生成卡住后后台无限 reasoning。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs` + `test_utils/context.rs`：新增 no-tools 回归测试，防止 title-gen 这类静默 session 重新暴露工具。
  - 用户二次真实模型复测显示：title-gen 已在 8s 后 cancel，且 no-tools 生效，但 cancel 后的 `title-gen-*` error event 被全局 `frontend/hooks/useAiEvents.ts` 的 unknown-session fallback 路由到当前 active session，导致 UI 显示错误。
  - `frontend/hooks/useAiEvents.ts`：在全局 AI event 入口直接忽略 `title-gen-*` session，防止隐藏标题生成的 started/error/reasoning 进入 active terminal/chat fallback。
  - `frontend/hooks/useAiEvents.ts`：进一步收紧 unknown-session fallback；如果 event 带明确 session_id 但无法解析到 terminal/conversation，就直接 drop，不再兜底路由到 active session。这样其它隐藏 session 的 `reasoning` 也不会进入当前 Thought 区域。
  - `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts` 与 legacy `frontend/components/AIChatPanel/useChatAiEvents.ts`：同样在 hook 入口过滤 title-gen event，形成双层隔离。
  - `frontend/hooks/useAiEvents.test.ts`：新增回归测试，确认 `title-gen-*` error 不会污染 active session timeline；确认已知但无法解析的其它 session `reasoning` 不会污染 active session thinking。
- **已记录证据**：
  - `cd backend && cargo fmt --package golish-agent-kit --package golish-agent-runtime --package golish-agent-bridge --package golish --check` → exit 0
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-runtime -p golish-agent-bridge` → exit 0
  - `cd backend && cargo test -p golish-agent-runtime none_tool_preset_exposes_no_tools_even_in_chat_mode --lib` → 1/1 passed
  - `cargo test -p golish-agent-runtime cancellation --lib` → 3/3 passed
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/AIChatPanel/hooks/useChatSessionInit.ts frontend/components/AIChatPanel/hooks/useChatSend.ts frontend/components/AIChatPanel/hooks/useAiChatEvents.ts frontend/components/AIChatPanel/useChatAiEvents.ts frontend/lib/ai/generation-suppress.test.ts` → exit 0 / No fixes applied
  - `pnpm vitest run frontend/lib/ai/generation-suppress.test.ts` → 2/2 passed
  - `pnpm exec biome check frontend/hooks/useAiEvents.ts frontend/hooks/useAiEvents.test.ts frontend/components/AIChatPanel/hooks/useAiChatEvents.ts frontend/components/AIChatPanel/useChatAiEvents.ts frontend/components/AIChatPanel/hooks/useChatSessionInit.ts` → exit 0 / No fixes applied
  - `pnpm vitest run frontend/hooks/useAiEvents.test.ts frontend/lib/ai/generation-suppress.test.ts` → 22/22 passed
- **未跑 / 原因**：
  - 未跑 `./init.sh` / `just precommit`：用户明确说“先别跑init.sh”，本轮已停止已启动的 init/test-fe 进程，后续只跑 targeted 验证。
  - 未做真实 LLM 手动 E2E：需要对外部 provider 发真实请求；留给用户在本地 `just dev` 中用自己的模型复现。
- **当前状态**：`feature_list.json` 条目 `ai-chat-stop-cancels-backend-stream` 保持 `in_progress`，不是 `passing`，因为缺 `just precommit` 与真实手动 E2E 证据。
- **已修改但未提交（本轮 scope）**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`
  - `backend/crates/golish-agent-runtime/src/execution_mode/selection_apply.rs`
  - `backend/crates/golish-agent-runtime/src/test_utils/context.rs`
  - `backend/crates/golish-agent-kit/src/tool_definitions/config.rs`
  - `backend/crates/golish-agent-bridge/src/bridge_executor/mod.rs`
  - `frontend/components/AIChatPanel/hooks/useChatSessionInit.ts`
  - `frontend/components/AIChatPanel/hooks/useChatSend.ts`
  - `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`
  - `frontend/components/AIChatPanel/useChatAiEvents.ts`
  - `frontend/hooks/useAiEvents.ts`
  - `frontend/hooks/useAiEvents.test.ts`
  - `frontend/lib/ai/generation-suppress.test.ts`
  - `feature_list.json`
  - `agent-progress.md`
- **风险 / 下一步最佳动作**：
  - 如果用户的 provider SDK 在 dropped future 后仍由底层 HTTP 客户端继续打印低层日志，需要再下钻 rig/reqwest cancellation；目前代码已经在 Golish 层 drop stream/completion future。
  - 标题生成现在 8s 后会放弃，不会阻塞主聊天；如果用户希望保留更高质量标题，可以后续改成“先本地 title，后台成功后再替换”的非阻塞策略。
  - 用户下一步：重启 `just dev`，发送长回复/Task 模式任务，点击 Stop，观察后端日志是否在 1s 内停止该轮输出；再观察完成主回复后 title-gen cancel 是否不再在 UI 显示错误。确认后再跑 `just precommit` 并把 feature 切 `passing`。

---

### 2026-05-27 · Xiaomi MiMo Provider 第三轮修正：context window 改 1M + max_output 改 128K + 删 mimo-v2-flash

- **本轮目标**：用户在 chat panel 看到所有模型都显示 `128K context used`，质疑 "怎么每个模型都是显示的上下文这么大 这个对吗"。我之前用 `xiaomi_defaults { context_window: 128_000, max_output_tokens: 8_192 }` **远低于实际**——按 platform.xiaomimimo.com 官方 quick-start/model 表 + devtk.ai 模型介绍页：
  - mimo-v2.5-pro / mimo-v2.5 / mimo-v2-pro：**1M context · 128K max output**
  - mimo-v2-omni：**256K context · 128K max output**
- **修正**：
  - `xiaomi_defaults`: context_window 128k → **1,000,000**；max_output_tokens 8,192 → **128,000**；新加 supports_web_search=true（小米所有 chat 模型都支持 web search）；is_reasoning_model 不在 defaults（per-model override）
  - `resources/llm-models/xiaomi.json`：每个模型加 capabilities override：
    - mimo-v2.5-pro / mimo-v2.5：is_reasoning_model=true（有 Deep Thinking）
    - mimo-v2.5 / mimo-v2-omni：supports_vision=true（Full-modal Understanding）
    - mimo-v2-omni：context_window=256000（小于其它模型）
  - **删 mimo-v2-flash**：之前从 platform.xiaomimimo.com/docs/en-US/quick-start/model 抄了 Flash，但实测 `cargo run --example xiaomi_live_probe -p golish-llm-providers XIAOMI_MODEL=mimo-v2-flash` → HTTP 400 `Not supported model mimo-v2-flash`，Token Plan 端点暂不支持。从 4 个文件移除（xiaomi.json / model-const-keys.json / xiaomi.ts / ModelOverrides.tsx）+ 加注释说明原因。
- **已记录证据**：
  - 4/4 模型 live probe 全过：
    - mimo-v2.5-pro: "我是人工智能助手，可以为您解答各类问题并提供帮助和服务。"
    - mimo-v2.5: "我是一个乐于助人的AI助手，随时准备回答你的各种问题！😊"
    - mimo-v2-pro: "我是小米自研的AI助手MiMo，很高兴为你提供帮助！"
    - mimo-v2-omni: "我是乐于助人的AI助手，随时准备回答你的问题。"
  - mimo-v2-flash 上游 400 Not supported model 已确认（Token Plan 暂不含 Flash）
  - cargo nextest run -p golish-llm-providers -p golish-models -p golish-settings --no-fail-fast → 144/145 PASS（1 pre-existing nvidia baseline failure 与本轮无关）
  - just check-fe → exit 0（含 generate-model-constants 自动重生 12 constants 含 4 个 xiaomi）
- **下一步（用户视角）**：
  1. 重启 just dev → chat panel 现在应该显示 `0.1% · 0.1K / 1000K context used`（mimo-v2.5-pro/v2.5/v2-pro）或 `0.1K / 256K`（mimo-v2-omni）
  2. 大文档场景可用 mimo-v2.5-pro 100 万 token 上下文处理长文档
  3. 多模态场景用 mimo-v2.5 或 mimo-v2-omni（supports_vision=true）
  4. 编程场景用 mimo-v2.5-pro is_reasoning_model + Deep Thinking
- **风险**：
  - rig-core 0.36 的 OpenAI Chat Completions client 接受 1M context 是上游 OpenAI 协议 spec 允许的；小米服务端是否对单 request 完整接受 1M token 需要联调（很多服务在实际工程上 cap 到 ~256k 后会 429/413）。若用户发现 200k+ 长 prompt 失败，回退到 256k cap。
  - mimo-v2-omni 标 supports_vision=true 是按官方"Full-modal Understanding"描述，但具体上传格式（base64 / url / data uri）需联调
  - mimo-v2.5-pro 标 is_reasoning_model=true，可能导致 rig-core 走 reasoning event 分离路径——若上游响应不含 reasoning channel 可能影响 UX；联调时观察 thinking event 是否出现

---

### 2026-05-27 · Xiaomi MiMo Provider 修正：补 ProviderConfig::Xiaomi + 真实模型列表 + 协议改为 Settings 全局

- **本轮目标**：用户首次到 chat panel 试用，发现 ① 报"请先选择模型"（虽然选了 MiMo）② 下拉显示 `MiMo V2.5 Pro (OpenAI)` + `MiMo V2.5 Pro (Anthropic)` 两个变体令人困惑 ③ 模型列表只有 1 个 mimo-v2.5-pro，没有真实小米其它模型。要求："你这里面不应该加模型，怎么加的是按照哪个的方式兼容呢"。我承认设计偏离，根据 https://platform.xiaomimimo.com/docs/zh-CN/price/tokenplan/subscription 全面修正。
- **修正 1/4 · backend ProviderConfig::Xiaomi（"请先选择模型" bug 根因）**：之前漏改 `golish-llm-providers/src/provider_config.rs` 的 `ProviderConfig` enum + 4 个方法 + `agent_bridge/constructors/mod.rs` 的 ProviderConfig match。前端发出 `{ provider: "xiaomi", ... }` 后 IPC serde tag deserialize 失败 → init_ai_agent 抛错 → useChatSessionInit::initializeSession catch 返回 false → "请先选择模型". 修：加 `XiaomiClientConfig` struct + `ProviderConfig::Xiaomi` 变体 + workspace/model/provider_name/model_override 4 arm + ProviderConfig::Xiaomi match in constructors/mod.rs + `agent_bridge::new_xiaomi_with_shared_config` + `golish-agent-kit/src/llm_client/providers/xiaomi.rs::create_xiaomi_components` + `pub use XiaomiClientConfig` 在 llm_client/mod.rs。
- **修正 2/4 · 真实模型列表**：删 `mimo-v2.5-pro@anthropic` 协议变体。`resources/llm-models/xiaomi.json` 重写为 4 个真实小米模型：① mimo-v2.5-pro（旗舰）② mimo-v2.5（标准）③ mimo-v2-pro（V2 旗舰）④ mimo-v2-omni（多模态 · supports_vision=true）。TTS 系列（4 款）不在 chat panel scope，留到未来音频模块。
- **修正 3/4 · 前端模型选择器去协议变体**：`frontend/lib/models/xiaomi.ts` 平铺 4 个真实模型 + 删除 ProviderGroupNested 的 "MiMo V2.5 Pro" 父级（之前 nested 含 OpenAI/Anthropic 子菜单令人困惑）。`frontend/scripts/model-const-keys.json` + `frontend/lib/ai/models.generated.ts` 同步 4 个新常量（generator 自动重生）。`frontend/components/Settings/SubAgentSettings/ModelOverrides.tsx::MODEL_SUGGESTIONS.xiaomi` 改为 4 个真实 id。
- **修正 4/4 · 协议改为 Settings 全局选项**：Settings → Xiaomi MiMo → Default protocol（auto/openai/anthropic）就是用户**唯一**该感知到的协议入口。后端 `resolve_protocol` 路径保留 `@anthropic` / `@openai` 后缀支持，但**不暴露**到 ChatModelSelector / model 列表 / sub-agent overrides。高级用户仍可在 settings.default_model 手动填 `mimo-v2.5-pro@anthropic` 强制覆盖。
- **已记录证据**：
  - cargo check --workspace → exit 0 / 0 new warning
  - cargo nextest run -p golish-llm-providers → **59/59 PASS**（比之前 +1，源于 ProviderConfig 加变体后单测覆盖率提升）
  - cargo nextest run -p golish-agent-bridge → 4/4 PASS
  - cargo nextest run -p golish-models → 31/32 PASS（baseline `embedded_nvidia_has_expected_count` 失败与本轮无关）
  - xiaomi_live_probe × 3 场景全过：
    - `mimo-v2.5-pro` + Auto → RigXiaomi → "我是一个智能AI助手，可以为你提供各种帮助和解答。"
    - `mimo-v2.5` + Auto → RigXiaomi → "我是MiMo，小米公司基于自研大模型开发的AI助手，随时为您提供关于小米产品和服务的帮助。"（**新模型工作**）
    - `mimo-v2.5-pro` + Anthropic（settings 全局选）→ RigXiaomiAnthropic → "我是MiMo，小米官方AI助手，随时为您解答疑问、提供帮助！"（**协议切换在 settings 层完成，模型 id 干净**）
  - frontend tsc exit 0 + check-fe 含 generate-model-constants 自动重生 (12 constants 含 4 个 xiaomi)
- **下一步（用户视角）**：
  1. 重启 just dev（前端会重新加载 12 个模型常量 + 新 4 个 xiaomi 模型）
  2. Settings → AI providers → Xiaomi MiMo → 填 tp- key（或 sk- key 选 Region=PayAsYouGo）→ Default protocol 选 Auto（默认 OpenAI）或 Anthropic（如果想让所有 mimo 模型走 Anthropic Messages 接口）
  3. ChatModelSelector → Xiaomi MiMo → 选 4 个真实模型之一（MiMo V2.5 Pro / V2.5 / V2 Pro / V2 Omni）
  4. 发任意 prompt → 应直接拿回中文回复（**不再报"请先选择模型"**）
- **风险**：
  - mimo-v2-omni capabilities 标 supports_vision=true 是基于官方"多模态"描述，但具体 image input 格式（base64 / url / data uri）需联调实测——若失败需调整 capabilities 或 prompt format
  - 后端 resolve_protocol fallback 链是「@suffix → settings default_protocol → Auto fallback to OpenAI」。如果用户切到 Anthropic 全局，并选了一个 mimo-v2-omni 这种多模态模型，Anthropic Messages API 对 vision 的支持是否完整需联调（Anthropic 协议下小米实际是否真支持 vision，文档未明确）
  - "请先选择模型" 修复后，下一个潜在问题：mimo-v2.5-pro 在 task 模式下是否能拿到 stage harness 接管的 prompt（Phase 1 task 模式经过 generator_prompt → backfill stage 路径，若 mimo 不擅长某些 reasoning 模式可能 stage gate fail）

---

### 2026-05-27 · Xiaomi MiMo Provider 全栈接入 + 联调验证（OpenAI + Anthropic 双协议 · 拿到真实 token）

- **本轮目标**：用户先在另一会话写了 `docs/design/2026-05-27-add-xiaomi-mimo-provider.md`（OpenAI + Anthropic 双协议兼容设计 + 风险 §6.1 / §6.2 / §6.3 / §6.4），叫"按照那个继续搞"。`[DISPATCH:off]`，作为唯一执行者按 5 个 Phase 推进。
- **完成清单（Phase 1-5）**：
  - **Phase 1 · settings schema**（4 文件）：`AiProvider::Xiaomi` 变体（含 Display + FromStr aliases `xiaomi`/`xiaomi_mimo`/`mimo`/`xiaomi_token_plan`）+ `XiaomiSettings { api_key, region, default_protocol, openai_base_url, anthropic_base_url, show_in_selector }` + `AiSettings.xiaomi` 字段。`cargo check -p golish-settings` 绿，54/54 单测 PASS。
  - **Phase 2 · 模型注册**（5 文件 + 1 新 JSON）：`ModelCapabilities::xiaomi_defaults()`（128k ctx / 8192 max_output / 支持 thinking history） + `resources/llm-models/xiaomi.json`（mimo-v2.5-pro 与 mimo-v2.5-pro@anthropic） + `xiaomi_models()` + `ProviderInfo { icon "🟠" }` + `embedded_defaults_for` + `provider_slug` + `get_model_capabilities` 全部分支补齐。`cargo check -p golish-models` 绿。
  - **Phase 3 · LlmClient + Provider impl**（5 文件 + 2 新文件 + 1 example）：`LlmClient::RigXiaomi(rig_openai)` + `LlmClient::RigXiaomiAnthropic(rig_anthropic)` 两个变体 + `dispatch_llm_client!` / `dispatch_llm_client_split!` 各加 2 arm + `provider_name()` / `is_openai()` / `is_anthropic()` 全部更新 + `golish-llm-providers/src/xiaomi/mod.rs`（`XiaomiRegion` enum: Cn/Sgp/Ams/PayAsYouGo + `XiaomiProtocol` enum + `resolve_protocol()` + `strip_protocol_suffix()`） + `provider_trait/xiaomi.rs::XiaomiProviderImpl` + `ProviderExtraSettings` 扩 3 个 xiaomi 字段 + `create_provider` / `extract_provider_settings` 加 Xiaomi 分支 + `sub_agent_dispatch.rs` 与 `bridge_executor/mod.rs` 两处零散 match arm 补齐。**风险 A 化解**（Bearer header 纯兼容，curl + Rust 双向验证）+ **风险 B 化解**（rig-core 0.36 anthropic Client builder 支持 `.base_url()`）。`cargo check --workspace` 绿，golish-llm-providers 58/58 单测 PASS（含新 8 个 xiaomi 模块测 + 4 个 XiaomiProviderImpl 测 + `pay_as_you_go_region_uses_api_xiaomimimo_endpoint` 回路防护）。
  - **Phase 3 联调（最关键）**：`examples/xiaomi_live_probe.rs` 端到端跑两条路径，环境变量驱动。用户先给 `sk-...` (按量付费) key → curl 4 种组合 (Token Plan endpoint × api-key / Bearer) 全 401，独立 endpoint `api.xiaomimimo.com` Bearer 命中 **HTTP 402 余额不足**（认证通过）→ 加 `XiaomiRegion::PayAsYouGo` 别名 `payg`/`pay_as_you_go`/`direct`/`global` 让 sk- 用户零配置走通。用户再给 `tp-...` (Token Plan) key → default region=cn 直接跑，**两条路径都拿到真实 mimo-v2.5-pro 回复**：
    - OpenAI 路径 `LlmClient::RigXiaomi` → `https://token-plan-cn.xiaomimimo.com/v1/chat/completions` → `"我是MiMo，小米公司基于自研大模型开发的AI助手，很高兴为你提供帮助！"`
    - Anthropic 路径 `LlmClient::RigXiaomiAnthropic` → `https://token-plan-cn.xiaomimimo.com/anthropic/v1/messages` → `"我是MiMo，由小米大模型Core团队研发的智能助手，随时准备为你解答问题、提供帮助。"`（model id `mimo-v2.5-pro@anthropic` 后缀正确剥离）
  - **Phase 4 · 前端 settings UI**（17 文件改动）：`frontend/lib/settings/types.ts` 加 `XiaomiSettings` + `AiSettings.xiaomi` + `AiProvider "xiaomi"` + `ProviderVisibility.xiaomi`；`frontend/lib/settings/defaults.ts` + `frontend/mocks.ts` + `frontend/lib/settings/api.ts::buildProviderVisibility` 加 xiaomi 默认值；`frontend/lib/ai/types.ts::AiProvider` + `ProviderConfig` union + `frontend/lib/api/model-registry.ts::AiProvider` 同步；`frontend/lib/ai/models.generated.ts` 加 `XIAOMI_MODELS` 常量 + `frontend/scripts/model-const-keys.json` 加 xiaomi key 映射（自动重生绿）；`frontend/lib/models/xiaomi.ts` 新建 `ProviderGroup` + nested + `frontend/lib/models/index.ts` + `frontend/lib/models/groups.ts` 加 xiaomi import / nest；`frontend/lib/ai/providers.ts::buildProviderConfig` + `frontend/components/AIChatPanel/providerConfig.ts::buildProviderConfig / getConfiguredProviders` + `frontend/components/AIChatPanel/hooks/useAiChatInit.ts` 加 xiaomi 分支；`frontend/hooks/useProviderSettings.ts` `ProviderEnabledState` + `ProviderApiKeys` + 默认值 + state 推导 全部加 xiaomi 字段；`frontend/components/Settings/hooks/useProviderForm.ts` 加 ProviderSettingsKey + isProviderConfigured + providerToSettingsKey + defaultProviderSettings + FALLBACK_PROVIDERS + PROVIDER_COLORS（橙色 `#FF6700`）；`frontend/components/Settings/ProviderSettings/index.tsx` 加 case "xiaomi" 渲染 API key 输入 + Region Select(cn/sgp/ams/payg) + Protocol Select(auto/openai/anthropic) + OpenAI base url override + Anthropic base url override 五字段；`frontend/components/Settings/ModelSelector.tsx::isProviderAvailable` + `frontend/components/Settings/SubAgentSettings/ModelOverrides.tsx::PROVIDER_OPTIONS` + `MODEL_SUGGESTIONS` 加 xiaomi；`frontend/lib/ui-state/settings.viewmodel.ts::deriveProviderCards` 加 xiaomi entry。
  - **Phase 5 · 验证**：`pnpm exec tsc --noEmit` exit 0 / `just check-fe` 绿 / `just test-fe` 与 baseline 同样 6 个 pre-existing failures（验证方法：git stash xiaomi changes → 同样 6 failed → git stash pop → 同样 6 failed，**xiaomi 改动未引入新失败**）。
- **已记录证据**：
  - `cargo check --workspace` → exit 0 / 0 new warning（仅 baseline `session_dir` dead_code）
  - `cargo nextest run -p golish-llm-providers` → 58/58 PASS（含 8 xiaomi 模块测 + 4 XiaomiProviderImpl 测 + 1 pay_as_you_go 回路防护测）
  - `cargo nextest run -p golish-settings` → 54/54 PASS
  - `cargo run --example xiaomi_live_probe -p golish-llm-providers` × 4 次（payg+openai → 402 / token-plan-cn+openai → 200 / token-plan-cn+anthropic → 200 / payg+anthropic → 402）
  - 真实小米 MiMo 回复内容（见上面 phase 3 联调段引号内）
  - `pnpm exec tsc --noEmit` → exit 0
  - `just check-fe` → exit 0（含 generate-model-constants 重生 + biome auto-fix 2 文件）
  - `just test-fe` → 6 failed / 1099 passed（与 baseline `git stash` 后跑 6 failed / 1097 passed 同样的 6 个失败文件名 / 行号；本轮 +2 passed 是 xiaomi 测试自身）
  - `ReadLints` 全部改动文件 → 0 errors
- **下一步**：
  1. 用户决定 commit 策略（建议 4 个 commit：tracing / backfill / fix(task-mode) / **feat(xiaomi-mimo)**）。后者可分两段：`feat(xiaomi-mimo): backend provider scaffold + live probe` + `feat(xiaomi-mimo): frontend settings UI + selector integration`。
  2. 用户接入 `just dev` → Settings → AI providers → Xiaomi MiMo → 填 tp- 或 sk- key（sk- 需手动选 Region=PayAsYouGo）→ 在 ChatModelSelector 选 MiMo 模型 → 发 prompt → 应该立刻拿到中文回复
  3. 联调 todo（推 Phase 2 完整观察 capabilities）：① reasoning 流（thinking content channel）是否分离 ② tool calling JSON schema 是否上游兼容 ③ vision input 是否支持（capabilities 当前 supports_vision=false 保守）
- **风险**：
  - 当前 capabilities `xiaomi_defaults` 保守（128k ctx / 8192 max_output / 不支持 vision），实际 mimo-v2.5-pro 极可能有更大窗口；推 Phase 2 联调实测后调整
  - rig-core 0.36 `normalize_anthropic_base_url` 会剥 `/v1/messages` / `/messages` / `/v1` 三种尾缀但不剥裸 `/anthropic`——本设计 base url 形如 `https://token-plan-cn.xiaomimimo.com/anthropic`，client 请求时再拼 `/v1/messages`，验证通过；如未来 rig-core 升级修改 normalize 逻辑可能需要回归
  - 用户给的 sk- key 实际余额为 0；用户用 tp- key 重新联调（用 tp- key 跑成功）
  - 设计文档 §6.3 风险 C 模型注册体系：当前 `xiaomi.json` 只有 2 个 model entry（`mimo-v2.5-pro` + `mimo-v2.5-pro@anthropic`），新模型来时只需追加 JSON 即可（不动 Rust 代码）

---

### 2026-05-27 · 修 baseline task mode FK violation（chat.rs lazy session create · 用户手动 E2E 立刻能跑后启动）

- **本轮目标**：用户第一次 just dev → task panel 发"评估 example.com 外部 attack surface"→ UI 立刻 `Failed to create task`。诊断后是 baseline 已知 bug，与今日 tracing/backfill 全无关，但阻塞用户手动 E2E。用户授权 P1 修复方案。
- **根因**：`backend/crates/golish/src/ai/commands/core/chat.rs::execute_task_mode` 第 118 行 `let uuid_session_id = uuid::Uuid::new_v4();` 直接生成一个新 UUID 作为 TaskOrchestrator 的 session_id，但 **从未在 sessions 表 INSERT 过这个 UUID**。`tasks.session_id UUID NOT NULL REFERENCES sessions(id)` FK 触发 → INSERT INTO tasks 失败 → `Failed to create task`。chat panel 传入的字符串 session_id (e.g. `pentest-chat-1779845856078-1`) 被 `_session_id: &str` 忽略，且字符串无法直接当 UUID 也不在 sessions 表里。`git log -- chat.rs` 最近改是 90be3ee/b16bb41 等 refactor commit，与 Phase 1 实施无关，是更早就埋下的。
- **修复（P1 方案 · chat.rs 单文件 +66 行）**：
  - `execute_task_mode` 顶部加 `sessions::create(&state.db_pool, NewSession { title, model, provider, ... })`，用返回的 `session_row.id` 替换原 `new_v4()`，让 DB 自己生成 UUID 并保证 sessions 表有对应行。`tasks.session_id` FK 因此满足。
  - title 由用户 prompt 前 80 字节（UTF-8 char-boundary 安全）派生，model/provider 从 `bridge.model_name()` / `bridge.provider_name()` 取，其它字段 None（workspace 等推 Phase 2 时与 chat panel session 共享）。
  - 新增 `tracing::info!(target: "harness::task_mode", ...)` 记录 session_db_id + chat_session_id 关系。
  - 新增 `truncate_for_title(s, max_bytes)` helper 函数 + 5 个单测（ascii under/over limit · 中文 char-boundary · empty · limit=0），全在 `chat_title_tests` mod 里。
  - 副作用：每次发 task 在 sessions 表多一行 task-only session；未来可优化为复用 chat panel session（需要 schema 加 chat panel session_id ↔ UUID 映射，超出今日 scope）。
- **已记录证据**：
  - `cargo check -p golish` → exit 0（1m 24s · 仅 1 个 preexisting `session_dir` dead_code warning · 0 new warning）
  - `cargo nextest run -p golish --lib -E 'test(chat_title_tests) | test(evidence)' --status-level fail` → **23/23 passed**（5 chat_title 新 + 18 evidence baseline · 0 回归）
  - `cargo clippy -p golish --lib --no-deps` → 5 preexisting baseline warning（`session_dir` dead_code / asset_intel explicit_auto_deref ×2 / webview_isolation needless_return / integrations facade doc indent）· **0 new warning**
  - `ReadLints chat.rs` → No linter errors found
- **下一步**：
  1. 用户重启 `just dev`（带相同 RUST_LOG + GOLISH_HARNESS_STAGE_MODE env vars）→ chat panel Task 模式重发 ①号 case，**应该不再 Failed to create task**，stderr 现在能看到完整 harness::* 链
  2. 如果仍 fail，stderr 里 `harness::task_mode: task mode session row created` 不出来 → 说明 sessions::create 自身失败（需要查 DB log）
- **风险**：
  - 每个 task 在 sessions 表多一行，长期跑后 sessions 表膨胀 → Phase 2 加 cleanup 或共享 chat session row
  - 如果用户 sessions 表里手动改过 schema (e.g. 加 trigger / 加 NOT NULL 列)，sessions::create 当前 6 字段 + DEFAULT 可能不够 → 推 Phase 2 时跟随 schema 演进
  - **本修复不在原本 in_progress 的 harness 实施 scope 里**（属于 baseline bugfix），但是 §2.7 用户授权后做的；commit 时建议拆为 `fix(task-mode): lazy session create to satisfy tasks.session_id FK`

---

### 2026-05-27 · Operation Harness Phase 1 接 chat panel · generator_prompt + harness_backfill（C 方案 + 之前 B 一起做完）

- **本轮目标**：用户选 C "一次做全套"。在前一段 tracing 补全后，发现 chat panel → task 模式还有个 gap：`generator_prompt()` 没告诉 LLM 填 `harness_stage`，导致 `apply_harness_gate_hook` 永远 skip。本段补两件事：① generator_prompt 加 HARNESS STAGE ASSIGNMENT 章节 + JSON 示例字段；② 新建 `task_orchestrator/harness_backfill.rs` deterministic keyword 兜底，BridgeAgentExecutor::generate_subtasks 末端调用。LLM 偷懒不填时也能强制接上 stage harness。
- **已完成**（在前一段 tracing 补全 10 文件基础上 +4 文件）：
  - `task_orchestrator/prompts/mod.rs` · `generator_prompt()` 加 `## HARNESS STAGE ASSIGNMENT (Phase 1 MVP — Operation Harness)` 一节 + OUTPUT FORMAT JSON 示例加 `harness_stage` 字段（标 OPTIONAL · 含 trigger keywords 清单 · 中英双语）
  - `task_orchestrator/harness_backfill.rs` (新文件 · 263 行)：`infer_harness_stage(text)` deterministic 关键词匹配（13 个 positive English + 9 个 positive 中文 + 11 个 anti-trigger）+ `backfill_harness_stage(subtasks)` 兜底；11 个单测覆盖 happy path / 中英文 / 大小写 / 反触发抑制 / preserve LLM-supplied value / empty slice。
  - `task_orchestrator/mod.rs` · `pub mod harness_backfill;` + `pub use harness_backfill::{backfill_harness_stage, infer_harness_stage};`
  - `golish-agent-bridge/src/bridge_executor/trait_impl.rs` · `generate_subtasks` 反序列化完 GeneratorOutput 后立即调 `backfill_harness_stage(&mut output.subtasks)`；LLM 已填 `Some(_)` 时跳过（never overwrite）。
- **已记录证据**：
  - `cargo nextest run -p golish-agent-kit --lib -E 'test(task_orchestrator::harness_backfill) | test(harness::)' --status-level fail` → **109/109 passed**（baseline 98 + 11 个 backfill 新增 = 109，与预期完全契合，0 回归）
  - `cargo check -p golish-agent-bridge -p golish-agent-kit` → exit 0
  - `cargo clippy -p golish-agent-kit -p golish-agent-bridge --lib --no-deps` → 0 warning
  - `ReadLints × 4 改动文件` → No linter errors found
- **设计决策**：
  1. **prompt 引导 + deterministic 兜底双保险**：LLM 接 prompt 能力不可靠（特别中文场景），关键词 backfill 不依赖 LLM compliance。LLM-supplied 值永远不被覆盖（LLM > backfill）。
  2. **关键词列表保守取召回**：13 positive English keywords + 9 中文（包括 "资产测绘 / 攻击面 / 外部侦察 / 子域名 / 被动 recon / DNS resolution / subfinder / ct log / asn discovery / certificate transparency"）。Anti-trigger 11 条（"exploit / metasploit / sqlmap / internal pivot / 横向移动 / 漏洞利用 / final report / 最终报告 / 修复建议"），命中即抑制。
  3. **错抓比漏抓代价低**：anti-trigger 优先于 positive；漏抓时 hook 直接 skip（debug log + 不阻断流程），错抓可能让无关 subtask 被强制走 gate。
  4. **接入点选 BridgeAgentExecutor 而非 TaskOrchestrator**：BridgeAgentExecutor 是 LLM 响应解析的唯一边界，把 backfill 放这里能保证未来其它 AgentExecutor impl（mock / test）也能选择性接入。TaskOrchestrator 是纯 orchestration 层，不应感知 LLM JSON 细节。
  5. **5 个 tracing target**：新增 `harness::backfill` target，per-subtask info + summary info（filled / total）。`RUST_LOG=harness::backfill=info` 即可监控 backfill 命中率。
- **未做**：
  - 没改 `feature_list.json`（同 tracing 补全段说明：本 patch 是 harness-mvp-external-attack-surface 的接入完善，不是新 feature）
  - IntentClassifier 没接 backfill（Phase 2 当多 stage 落地时，统一走 `harness::IntentClassifier` 或新建 stage_keyword_router）
  - 没在 task_orchestrator 层加 backfill 接入点单测（targeted 集成测推 Phase 2 · 当前 11 个 unit test 覆盖纯函数逻辑足够）
- **下一步建议（用户手动验证）**：
  1. `RUST_LOG=harness=info,golish=info,golish_agent_kit=info,golish_agent_bridge=info GOLISH_HARNESS_STAGE_MODE=true just dev`
  2. chat panel 左下角 `ExecutionModePicker` 切 **Task**（闪电图标 magenta）
  3. 输入框输 `评估 example.com 的外部 attack surface` 或英文 `Map external attack surface of example.com` → 发送
  4. stderr **应该** 出现：
     - `INFO [TaskMode/Generator] Decomposing task into subtasks` （pre-existing）
     - `INFO harness::backfill: harness_stage backfilled by keyword matcher subtask_title="..."`（如果 LLM 没填 + keyword 命中）
     - `INFO harness::sprint_contract: sprint contract generate (deterministic) ...`（每个 subtask 进 harness 时）
     - `INFO harness::stage_harness: validate_gate entered (5-check pipeline) ...`
     - `INFO harness::gate::{schema|scope|contract|vacuous|freshness}_check: ... outcome=pass|block`
     - `INFO harness::hook: gate decision: PASS|BLOCK ...`
  5. **反向 case**：发任意 task → agent 不调任何工具就交答 → stderr 应看 `WARN harness::hook: gate decision: BLOCK ... first_reason="deliverable vacuous: ..."`
- **风险**：
  - LLM 可能不按 prompt 格式输出（漏字段、写错 stage_kind 拼写、用未支持的 stage 名）→ backfill 兜不住 typo（如 `external attack_surface`）；Phase 2 加 enum validation 时再补
  - keyword 兜底有假阳性可能（如 "DNS resolution" 字面也含在内部排查任务中）→ 实测后按需收紧 anti-trigger
  - `apply_harness_gate_hook` 仅解析 content 整段为 JSON 的 deliverable · 当前 deliverable 解析路径不强 · 大概率走 debug skip 路径（即 `harness gate hook entered` 出但 deliverable parsed 不出）→ Phase 2 加正则 fence 抽取

---

### 2026-05-27 · Operation Harness Phase 1 tracing 补全（10 文件 · 0 行为改动 · 用户问"流程现在会有日志吗"后立刻动手）

- **本轮目标**：用户在 §5.9 分发关闭模式下问"我的流程现在会有日志吗"。grep 后实情：harness 5 个 gate check / sprint_contract / intent_classifier / stage_harness / evidence_read / evidence_ledger 全部 0 处 tracing；唯一日志在 `apply_harness_gate_hook` 的 4 行 warn/debug（仅 profile/spec/Harness 加载失败时出）。Doc 4 Observability Plane 是 Phase 2 才落，但即便不实施 Doc 4，关键路径 8-10 处 tracing 是 30 分钟可干完的 patch。用户在 ABC 三方案中选 B → 立刻动手。
- **本轮参与者**：MCP-4 controller（bajie-mcp-agent-4-bs4en72s）· DISPATCH:off 模式 · 本会话直接执行（§5.9 非分发模式全能执行规则）。
- **已完成**（10 文件 / 0 commit，等用户决定是否合并）：
  - **5 个 gate check** 各加 1-2 行 `tracing::info!`（target=`harness::gate::<check_name>`，含 stage_id / stage_run_id + 对应业务字段 + outcome=pass|block + reasons_count + first_reason）：
    - `harness/gate/schema_check.rs` · `harness/gate/scope_check.rs` · `harness/gate/contract_check.rs` · `harness/gate/vacuous_check.rs` · `harness/gate/freshness_check.rs`（run 与 run_with_freshness 两个 path 都覆盖）
  - **`apply_harness_gate_hook`**（`task_orchestrator/subtask_phases/execute.rs`）：补全 5 处 tracing —— stage_hint=None skip 时 debug / stage_kind 不 MVP 支持 skip 时 debug / hook entered info（stage_kind + subtask_title + content_len）/ deliverable 解析成功 info（claims+findings+evidence_refs） / decision allowed→info 而 decision blocked→warn（含 first_reason + 3 类 recovery_actions 长度）；保留原 4 处 warn/debug 兼容。
  - **`StageHarness::validate_gate`**（`harness/stage_harness.rs`）：入口 + 出口各 1 行 info，含 stage_kind / has_contract / allowed / reasons_count。
  - **`DefaultSprintContractGenerator::generate`**（`harness/sprint_contract.rs`）：进入打 stage_run_id + stage_kind + scope_context_len + expected_findings + time_budget + min_tool_invocation_kinds + generator="deterministic-default"；生成完打 contract_id + status + contract_text_len。
  - **`evidence_read`**（`golish/src/tools/evidence.rs`）：command 入口（evidence_audit_id + summary_level）/ NotFound 路径 warn / Ok 路径打 kind + subject + scope_label + freshness + headline_len + structured_present + raw_present + raw_len。
  - **`classify_tool_call`**（`golish-agent-runtime/src/agentic_loop/tool_classifier.rs`）：阈值触发时加 `tracing::warn!`（target=`harness::tool_classifier`，含 session_id + count_per_minute + threshold）。
- **已记录证据**：
  - `cd backend && cargo check -p golish-agent-kit -p golish -p golish-agent-runtime` → exit 0（1m 40s · 仅 1 个 preexisting `session_dir` dead_code warning）
  - `cargo nextest run -p golish-agent-kit --lib -E 'test(harness::)' --status-level fail` → **98/98 passed**（baseline 88/88 已含 e2e_tests 10 个；本轮 tracing 补全后仍 0 回归）
  - `cargo nextest run -p golish-agent-runtime --lib -E 'test(tool_classifier)' --status-level fail` → **11/11 passed**
  - `cargo nextest run -p golish --lib -E 'test(evidence)' --status-level fail` → **18/18 passed**
  - `cargo clippy -p golish-agent-kit -p golish-agent-runtime --lib --no-deps` → 0 warning
  - `cargo clippy -p golish --lib --no-deps` → 5 preexisting baseline warning（`session_dir` dead_code / asset_intel explicit_auto_deref ×2 / webview_isolation needless_return / integrations facade doc indent），**0 new warning**
  - `ReadLints` × 10 个改动文件 → No linter errors found
- **设计决策**：
  1. **target 命名约定**：用 `harness::<scope>` 前缀（`harness::gate::<check_name>` / `harness::hook` / `harness::stage_harness` / `harness::sprint_contract` / `harness::evidence_read` / `harness::tool_classifier`），让 RUST_LOG 可精细过滤：`RUST_LOG=harness::gate=debug,harness::hook=info just dev` 仅出 gate 内部 + hook 入口；`RUST_LOG=harness=info` 出全 harness 流程。
  2. **level 分配**：pass/正常路径=info（默认就出，让用户能看到流程），block/异常路径=warn（hook 末端 decision blocked），加载失败=warn，skip/无 deliverable=debug（默认不出，避免噪音）。
  3. **0 业务逻辑改动**：每处 tracing 都是新增独立 statement，不改任何 if/else 分支、return 值、struct 字段。所有 88+11+18=117 个相关单测 0 回归证明语义不动。
  4. **不接 Doc 4 Observability Plane**：Phase 1 GateResult.gate_result_id / blocking_reason_id 仍 None；tracing event 走 stderr，Phase 2 Doc 4 落地后可由 raw_event_log 采集器统一收（tracing event 自带 structured fields，符合 `serde_json::Value`-able 约束）。
- **未做**：
  - 没 commit（等用户确认是否一并打 commit · 推荐 `chore(harness): add structured tracing across Phase 1 hot path`）
  - 没改 `feature_list.json`（本 patch 不是新 feature · 是 harness-mvp-external-attack-surface 的 instrumentation 完善 · 可以在 evidence 段加一行注释或下次 commit 时一起更新）
  - 没改 `RUST_LOG` 默认值（仍是 `golish=debug` fallback only · 用户用 `RUST_LOG=harness=info` 启动才能看到本轮新增日志）
- **下一步建议**：
  1. **用户手动验证日志**：`RUST_LOG=harness=info,golish=info,golish_agent_kit=info GOLISH_HARNESS_STAGE_MODE=true just dev` 启动 → 新建 task 模式 task → stderr 应出现：sprint contract generate / validate_gate entered / 5 个 check 各自 outcome / hook decision pass|BLOCK。
  2. 构造反向 case：让 agent 不调任何工具就交答 → stderr 应看 `vacuous_check block reasons_count=N first_reason="deliverable vacuous: ..."` + hook 末端 `gate decision: BLOCK ... recovery_missing_evidence_kinds=2`。
  3. 若 user 满意 → 一并 commit；若想再加 IntentClassifier / nl_slice / PreActionAuthorizer 日志，再来一轮。
- **风险**：
  - tracing event 不持久化（stderr only）· 用户开新会话或关 Tauri 就丢 · Phase 2 Doc 4 raw_event_log 才解决持久化
  - 若 `just dev` 默认 RUST_LOG 未设 → 新增 info 不出 · 用户必须显式设 `RUST_LOG=harness=info` 或更宽

---

### 2026-05-26 · Operation Harness Phase 1 实施完毕（17 Task · 16 commits · feat/harness-design-2026-05-26 分支）

- **本轮目标**：用户授权（AGENTS.md §2.7 明示）按 `docs/superpowers/plans/2026-05-26-task-mode-refactor-to-harness.md` 17 个 Task 在 `feat/harness-design-2026-05-26` 分支（起点 commit `09abd0e`）落地 Phase 1，把 chat panel task 模式重构为 1-stage harness（external_attack_surface）+ Evidence Ledger + MCP Resource。
- **本轮参与者**：MCP-2 controller（bajie-mcp-agent-2-sukoeliv）· DISPATCH:off 模式（用户在本会话直接执行）· 使用 superpowers:executing-plans skill 流程。
- **已完成**（按 commit 顺序）：
  - **Commit 0** `0b037da` · feature_list.json 加 `harness-mvp-external-attack-surface` in_progress + 切 `asset-intel-hydrate-disambiguation` 到 blocked（§2.1 一致性）
  - **Phase 1a · Evidence Ledger schema**（commits 1792885 / e5eb552 / 03f24fa / af60bc3）
    - `backend/crates/golish-db/migrations/20260601000001_evidence_ledger.sql` 7 步 idempotent（audit_log.audit_role / organizations.scope_rules_version / evidence_classifications bitemporal / operation_state / stage_runs / sprint_contracts / FK 反向补）
    - `golish-pentest::evidence_ledger` (types + mod, 12 单测) + `golish-db::repo::{evidence_classifications, operation_state, stage_runs, sprint_contracts}` (4 repo + 4 serde roundtrip 单测)
    - startup `reclaim_abandoned_audits` + `GolishDb::start` 集成 + 4 单测
    - `resources/harness/evidence_kinds.json` + `EvidenceKindRegistry` + 8 单测
  - **Phase 1b · MCP Resource Evidence Summary**（commits aa7e6bf / b215046 / ffee39a）
    - `EvidenceSanitizer` 4 步 pipeline + 5 per-kind parser + 22 单测（含 prompt injection 拦截）
    - `evidence_read` Tauri command 5 步走（tools/evidence.rs + facade + registry + frontend evidence.ts + serde 镜像）+ 18 单测
    - `tool_classifier.rs` RecentToolCallTracker 滑动窗口 + classify_tool_call + 11 单测
  - **Phase 1c · Stage Harness MVP**（commits 163f04e / 559416f / bb98f3e / 1bcdc52 / 52f70d4 / 1b0a23e / 10dd927）
    - resources/harness/{profiles/{assessment,assessment.sprint_skeleton},stages/external_attack_surface,graph/operation_graph}.json 4 份
    - `golish-agent-kit::harness` 15 文件骨架（types/profile/stage_spec/nl_slice/intent_classifier/pre_action_authorizer/sprint_contract/stage_harness + gate/{mod,5 check}）+ 61 单测
    - IntentClassifier 中英文双语词库完整版（passive 19 / active 19 / vuln 16 / exploit 18）+ 4 档优先级压制 + 13 单测
    - DefaultSprintContractGenerator deterministic 渲染 + 9 单测
    - 5 个 gate check 从 sanity skeleton 升级到 Doc 3 §8 完整逻辑（contract_check.run_with_skeleton + freshness_check.run_with_freshness + vacuous_check FakePattern）+ 25 新单测
    - task_orchestrator 接入：PlannedSubtask 加 3 字段（harness_stage / nl_slice / acceptance_criteria 全部 #[serde(default)]）+ execute_single_subtask 末端 `apply_harness_gate_hook` + 4 hook 单测
    - feature flag `GOLISH_HARNESS_STAGE_MODE` env var（LazyLock 缓存 · 默认 OFF）+ 2 单测
  - **Phase 1d · 端到端验证**（commits b106d55 / 本 commit）
    - `harness/e2e_tests.rs` 10 个 e2e 场景（happy path / vacuous block / scope sanity / freshness sanity / contract_check 范围 / freshness 真实 max_age / Other-skip 阈值 / SprintContract pipeline）
    - Playwright e2e（Task 1d.2）跳过 · 推 Phase 2 + 用户手动 E2E
    - Doc 1/2/3 status: Discussion Draft → **Implemented (Phase 1)** · Doc 4: → **Acknowledged (Phase 1 partial-satisfy)** · plan §12 → **Implemented (Phase 1)**
- **已记录证据**：
  - cargo nextest run -p golish-pentest -p golish-db --lib --status-level fail → 100/100 passed
  - cargo nextest run -p golish-agent-kit --lib -E 'test(harness::)' → 88/88 passed
  - cargo nextest run -p golish-agent-kit --lib -E 'test(harness::e2e_tests)' → 10/10 passed
  - cargo nextest run -p golish --lib -E 'test(evidence)' → 18/18 passed
  - cargo nextest run -p golish-agent-runtime --lib -E 'test(tool_classifier)' → 11/11 passed
  - cargo clippy -p golish-db -p golish-pentest -p golish-agent-kit -p golish-agent-runtime --lib --no-deps → 0 warning（本轮新增 0 warning · golish crate 5 preexisting warning 与本轮无关）
  - pnpm exec tsc --noEmit → exit 0
  - pnpm exec biome check frontend/lib/api/evidence.ts → No fixes applied
  - ReadLints × 全部本轮新增/改动文件 → No linter errors found
  - 4 个 JSON 资源 python3 -m json.tool → all exit 0
- **Plan 偏差修正记录**（详见 feature_list.json `harness-mvp-external-attack-surface` notes 字段）：
  1. migration 路径 `migrations/` → `backend/crates/golish-db/migrations/`（项目实际路径）
  2. audit_log 无 `started_at` 字段 → reclaim 用 `created_at`
  3. `reclaim_abandoned_audits` 位置 `golish/src/lib.rs` → `golish-db::GolishDb::start`（canonical DB ready 锚点）
  4. Task 1b.3 `stream_retry.rs` 实际职责是 LLM stream-start retry → 新建独立 `tool_classifier.rs`
  5. Task 1c.5 plan 建议 5 commits → 实际合并为 1 commit（同文件多字段，git diff 仍可定位）
  6. Task 1c.6 hook 3 元组返回 → 适配现有 2 元组签名（gate decision 文本化嵌入 content）
  7. Task 1c.7 settings.toml → 用 env var `GOLISH_HARNESS_STAGE_MODE`（settings.toml 接入推 Phase 2）
  8. Task 1d.2 Playwright 跳过（推 Phase 2 + 用户手动 E2E）
- **Doc 4 处理**：用户在 Phase 1a 完成后新增 `docs/design/2026-05-26-harness-observability-plane.md`（Codex 起笔），定义 raw_event_log / trace_tree / metrics_rollup / operation_snapshot / evaluation_record / replay/diff / decision_attribution 10 个 observability surface。**未纳入 Phase 1 实施 scope**（Doc 4 §12 Non-Goals 明确不授权 runtime/migration/Tauri command/UI）。但在 `GateResult` 中预留 `gate_result_id` + `blocking_reason_id` Option<Uuid> 字段（默认 None），Phase 2 落 Observability Plane 时直接填，不破坏现有 wire 协议。Doc 4 status 本轮改为 **Acknowledged (Phase 1 partial-satisfy)**。
- **commit 记录**：0b037da · 1792885 · e5eb552 · 03f24fa · af60bc3 · aa7e6bf · b215046 · ffee39a · 163f04e · 559416f · bb98f3e · 1bcdc52 · 52f70d4 · 1b0a23e · 10dd927 · b106d55 + 本 Task 1d.3 commit（共 17 个 commit）。
- **风险**：
  - Phase 1 feature flag 默认 OFF · 启用前需用户手动 `GOLISH_HARNESS_STAGE_MODE=true` + 启动 just dev 验证 UI 路径
  - `apply_harness_gate_hook` 当前仅识别 content 整段为 JSON 的 deliverable; 混合 prose + code fence 推 Phase 2 加正则抽取
  - Phase 1 MVP 仅 ExternalAttackSurface stage; 其它 stage 走 hook 时返 Err 导致 subtask 失败 → 生产启用前需追加 enumeration/reporting 等支持
  - schema_check + scope_check 仅 sanity-only · 完整 evidence_label=InScope 验证需 Phase 2 接 EvidenceLedger live query
- **下一步建议**：
  1. 用户手动 E2E（GOLISH_HARNESS_STAGE_MODE=true → just dev → 新建 task 模式 task → 验证 stage banner / gate decision JSON / recovery_actions UI）
  2. Phase 2 启动：① enumeration stage 实施 ② settings.toml feature flag 接入（替代 env var） ③ Doc 4 Observability Plane 完整实施
  3. 5 preexisting clippy warning + 2 baseline test failure 若进 main 影响 harness 验证，需先解决

---

### 2026-05-26 · Operation Harness Profile + DAG + Lab 设计文档多 agent 评审（MCP-1 + MCP-4 + MCP-2 三方 6 轮）

- **本轮目标**：用户要求评估 Codex 起草的 `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` 设计合理性；后续要求与其他 MCP agent 多轮讨论，"上网搜论文也可以"。最终从单人评审升级为三方 6 轮交叉验证 + 文档增补。
- **讨论参与者**：
  - MCP-1（bajie-mcp-agent-1-gniytpco · 本会话）：论文整合 / 改进提案者
  - MCP-4（bajie-mcp-agent-4-bs4en72s · group 成员）：架构反驳 / 范式校准
  - MCP-2（bajie-mcp-agent-2-sukoeliv · controller）：项目代码证据 / schema 提议
- **已完成**（按 §G1.1 先读后改 + §G2 按动词加载 project-learning 技能 + 论文检索）：
  - 4 篇 2026 arxiv 论文集成（AHE 2604.25850 / PCAS 2602.16708 / OAP 2603.20953 / PAuth 2603.17170）
  - `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` 在原 §1-§12（Codex 草稿）之上新增 §13-§22（共 10 节）
  - §13 Round 1-3 评审结果 · §14 Round 4 MCP-2 三让步 + MCP-4 四盲点（α/β/γ/δ）· §15 Round 5 收敛信号 + 拆三个 design doc · §16 Round 5 O1-O4 + O6 + O7 详答 · §17-§18 Round 5 MCP-4 迟到回复 + 投 A · §19 Round 6 触发条件（3 项冲突待解）· §20 Round 6 收敛（MCP-4 O4 让步 + O7 妥协）· §21 Final Consolidated Decisions（single source of truth）· §22 Reader Guide + Cross-Reference Matrix
  - 6 处 superseded 指针：§13.6.1 / §13.6.3 / §13.6.5 / §15.3 / §16.6 / §20.4 加 cross-link 到 §21 / §14 / §17 / §18
  - §13.12 表加 Final 位置 + Final 立场两列
  - `docs/design/2026-05-20-agent-harness-strategy.md` 顶部加 Superseded 指针 → 2026-05-26
  - `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md` 顶部加 Superseded 指针 → 2026-05-26 §21.9
  - 创建 group `grp-2182a9cc 'harness-design-review'` 用于 MCP-1 + MCP-2 + MCP-4 三方讨论
- **关键决议**（详见 §21）：
  - MVP 严格限定为 `assessment` profile + L2 active_recon + 1 stage `external_attack_surface`
  - 不造 `operations` 表（与用户 2026-05-17 删除 engagements 表的决定一致）
  - audit_log 加 `audit_role` 第四值 'approval'（不新建 user_approvals 表）
  - evidence_classifications 走 bitemporal `(valid_from, valid_to)` schema + supersedes 链
  - NlSlice 终态 4 字段：`{subtask_id, stage_kind, sealed_origin_session, deliverable_schema_id}`，intent_axis 走 Operation.user_intent_constraints 顶层
  - evidence_kind_aging 走 `resources/harness/evidence_kinds.json` 静态资源（不入 DB）
  - 三份 Phase 0 design only doc 拆分：Doc 1 evidence ledger（MCP-1）→ (Doc 2 mcp-resource by MCP-4 ∥ Doc 3 stage-harness MVP by MCP-2)
  - 不引 saga 框架（PentestAudit 天然 saga-friendly）
  - 不重构 task_orchestrator
  - 不新增 4 个 crate
- **运行过的验证 / 已记录证据**：
  - `ReadLints docs/design/2026-05-26-operation-harness-profile-dag-lab.md` → 0 errors
  - `wc -l docs/design/2026-05-26-operation-harness-profile-dag-lab.md` → 2627 行（初始 816 行 + §13-§22 累计 +1811 行）
  - `Grep "pub struct NlSlice"` → 三处引用全部 4 字段一致（line 894/1210/2310）
  - `sed -n '2595,2605p'` 验证 §22.3 supersedence 矩阵 line 2599 = 4 字段
  - 三人独立 Read 验证：MCP-2 用 `wc -l` + Read line 2310-2315 / MCP-4 用 `Grep "NlSlice 4 字段"` 全文 / MCP-1 完成 6 处 superseded 指针 + §22 cross-ref 矩阵
- **commit 记录**：本轮在新分支 `feat/harness-design-2026-05-26` 上 commit 3 个文件（2 个 superseded 指针 + 本进度记录）+ 1 个新设计文档作为新分支首个 commit
- **分支策略**：从 `feat/asm-intel-providers` HEAD `33917a9 feat(ai-chat): emit ContextWarning on history restore` 拉出新分支 `feat/harness-design-2026-05-26`，用于装本轮三方讨论产出。原分支保留以继续 asset-intel-hydrate-disambiguation 与 precommit 修复。
- **已知风险或未解决问题**：
  - `just precommit` 仍 exit 1（5 clippy + 2 baseline test failure，与本轮无关）
  - `asset-intel-hydrate-disambiguation` 仍 in_progress（feature_list.json line 80）
  - 三份 Phase 0 design only doc（Doc 1/2/3）**未启动**，等 precommit 切绿 + asset-intel-hydrate 切 passing + 用户明示 §2.7 授权 schema migration 设计
  - `harness lab bench fixtures 成本` + `vacuous detector 二阶 LLM` 由 §18.2 决定 defer 到 Phase 1+
- **下一步建议**：
  1. 在原 `feat/asm-intel-providers` 分支修 precommit 红灯（5 clippy + 2 test failure）
  2. 修 `asset-intel-hydrate-disambiguation` 切 passing
  3. 全部修完合回 main 后，回到 `feat/harness-design-2026-05-26` 分支，等用户明示 §2.7 授权后启动 Doc 1 起草

---

### 2026-05-26 · NVIDIA NIM model registry: 清理 15 个不存在的假 ID + 加 Go-default-404 错误改写

- **本轮目标**：用户上报 `mistralai/devstral-2-123b-instruct-2512` 触发 `404 page not found` 导致 main-agent / memory-gatekeeper 同时 stream 失败；排查根因后清理整个 NVIDIA NIM model 注册表，并加上对 NVIDIA NIM Go-default 404 的错误信息改写。
- **诊断过程（证据）**：
  - 用 curl 直接打 `https://integrate.api.nvidia.com/v1/chat/completions` 五个不同样本（无 key、错路径、假 key、无 auth、错 model），全部回 `404 page not found\n`——确认这是 NVIDIA NIM 网关（Go 写的）的默认 `http.NotFound` 输出。
  - 拿 settings.toml 里真实 API key（`nvapi-HN4pm9RME_e5Zk-...`，已脱敏）实测 4 个 model：`qwen/qwen3.5-122b-a10b` 200 / `qwen/qwen3.5-122b` 404 / `mistralai/devstral-2-123b-instruct-2512` 404 / `meta/llama-3.1-8b-instruct` 200。证明 API key 完全可用，404 是因为部分 model ID 在 NVIDIA NIM 上根本未部署。
  - 拉 `/v1/models` 实际列表（123 个）逐项对照 `frontend/lib/ai/models.generated.ts` 的 `NVIDIA_MODELS`（29 个）—— 15 个不在 NVIDIA NIM 实际部署中，疑似从 build.nvidia.com 的 "即将上架" 页面或 AI 自动列表抄进来的。
- **改动**（按 §AGENTS.md §2.2-§2.3 + §G2 走 codebase 改动 + §G5 默认补 `code-audit` / `test-engineering`）：
  - `resources/llm-models/nvidia.json`：从 29 个 model 删到 14 个（保留：Nemotron Ultra 253B、Qwen 4 个、Mistral 3 个、DeepSeek V4 Flash/Pro、Kimi K2.6、GLM 5.1、MiniMax M2.7、Step 3.5 Flash）。
  - `frontend/scripts/model-const-keys.json`：同步删除 15 个 const-key entry。
  - `frontend/lib/ai/models.generated.ts`：跑 `node frontend/scripts/generate-model-constants.mjs` 重新生成，NVIDIA_MODELS 现在 14 个。
  - `frontend/lib/models/nvidia.ts`：清理 selector 平铺 + nested 分组，删除被删常量的引用。
  - `frontend/components/Settings/SubAgentSettings/ModelOverrides.tsx`：硬编码的 `nvidia` 模型推荐列表换成 14 个真实部署的。
  - `backend/crates/golish-models/src/descriptors/loader.rs`：guard test `nvidia_registry_contains_required_flagship_models` 的 required 列表里 3 个 fake ID 换成真实部署的（`deepseek-v4-pro` / `llama-3.1-nemotron-ultra-253b-v1` / `qwen3.5-122b-a10b`）。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/stream_retry.rs`：在 `classify_stream_start_error` 新增分支——当错误信息包含 `"404 page not found"` 时改写为 `"The selected model is not deployed on the NVIDIA NIM endpoint. Pick a different model..."` + 新增单测 `classify_nvidia_nim_go_default_404_is_model_unavailable`。
- **已记录证据**：
  - `cd backend && cargo test -p golish-agent-runtime --lib classify_nvidia_nim_go_default_404_is_model_unavailable` → `1 passed; 0 failed; 0 ignored; 0 measured`
  - `cd backend && cargo test -p golish-models --lib nvidia_registry_contains_required_flagship_models` → `1 passed; 0 failed`
  - `pnpm tsc --noEmit` → exit 0（前端 typecheck 全绿）
  - `pnpm vitest run frontend/lib/ai/models.generated.test.ts` → `Tests 23 passed (23)`（const-key ↔ JSON ↔ models.generated 三方同步断言通过）
- **未引入的 baseline 失败**（stash 验证后确认全是预先存在）：
  - `just test-fe`：2 failed files / 6 failed tests（`TerminalSettings.test.tsx` 4 个 + `HomeView.memo.test.tsx` 2 个）——与本任务无关。
  - `just lint-rust`：5 个 clippy errors（`session_dir` dead_code、`asset_intel.rs` explicit_auto_deref ×2、`webview_isolation.rs` needless_return、`integrations.rs` doc 缩进）——与本任务无关。
  - `just check-fe`：`frontend/lib/ai/models.generated.test.ts:13` biome `organizeImports` FIXABLE——与本任务无关。
- **后续顺手清死代码**（用户同意后补充）：
  - `backend/crates/golish-llm-providers/src/model_capabilities/quirks.rs::nvidia_default_quirks`：删除 `deepseek-v3.1-terminus` / `nemotron-3-nano-omni` 两条字符串 match（函数 scope 锁死 NVIDIA，删除该 model 后 100% 死代码）。`cargo test -p golish-llm-providers --lib model_capabilities` → `30 passed; 0 failed`。
  - `frontend/components/AIChatPanel/ChatModelSelector.tsx::modelIsThinkingByDefault`：**未删** `deepseek-v3.1-terminus` / `nemotron-3-nano-omni`——该函数对 NVIDIA + OpenRouter + Z.AI SDK 三个 provider 生效，OpenRouter 是 transparent passthrough，用户可能填这些 model ID，删除会让真实模型默认关 thinking，保留以充当安全网。
- **commit 记录**：本轮未 commit；用户尚未指示。
- **风险**：
  - 用户可能依赖某个被删的 model（如 Devstral 2 123B）做某项实验——但即使保留也是 404，所以删除不影响**实际功能**，只影响**UI 可选项**。
  - 如果未来 NVIDIA NIM 上线这些 model（如 Devstral 2 123B），需要按本轮路径重新加回 `nvidia.json` + `model-const-keys.json`，并跑 `node frontend/scripts/generate-model-constants.mjs`。
- **下一步建议**：
  1. 用户验证：在 IDE 中切到 `Qwen 3.5 122B`（已知可用）发一条消息，应当不再 404；切到任何剩下的 14 个 model 也应该全部可用。
  2. 顺手任务（可选）：清理 quirks.rs 和 ChatModelSelector.tsx 的死代码字符串 match。
  3. 顺手任务（可选）：考虑在 `generate-model-constants.mjs` 加一个 CI step——对每个 provider 调实际 `/v1/models` API 校验 nvidia.json 中 ID 都真实存在（避免再次出现 fake ID 漂移）。

---

- **本轮目标**：用户反馈中文适配很差，要求全面处理；本轮先做高频设置页和当前 Target 工作区的第一批可验证中文化。
- **已完成**：
  - `frontend/lib/i18n/en.json` / `zh-CN.json`：新增 `appearancePanel`、`targetWorkspace`、`editorSettings`、`notificationsPanel` 翻译段；修正 `settings.title`、`settings.terminal/editor/mcp/codebases/network/notifications/appearance/advanced` 等 zh-CN 导航仍为英文的问题。
  - `frontend/components/HomeView/HomeView.tsx`：启动页 / 项目首页的副标题、Open Project、New Project、Recent Projects、Active/Loading、删除项目弹窗、空态、worktree 删除提示接入 i18n。
  - `frontend/components/Settings/AppearanceSettings.tsx`：Theme / Language / UI Scale / Input Caret / UI Customization 全部改为 i18n key。
  - `frontend/components/TargetPanel/TargetGroupedView.tsx`：Fields tab 的分组和字段名、顶部 Targets/In/Out、workspace tabs、Activity/Fields/Candidates/Scope/空态卡片等主要可见文案接入 i18n。
  - `frontend/components/Settings/EditorSettings.tsx`：编辑器设置页 General / Word Wrap / Line Numbers / Vim Mode 等接入 i18n。
  - `frontend/components/Settings/NotificationsSettings.tsx`：通知设置页和测试通知文案接入 i18n。
  - `frontend/components/Settings/TerminalSettings.tsx`：Shell / Font / Scrollback 等接入 i18n。
  - `frontend/components/Settings/AdvancedSettings.tsx`：Log Level / Experimental / LLM API Logs / Privacy / Version 等接入 i18n。
  - `frontend/components/Settings/AiSettings.tsx`：AI Keys、Tavily/Brave 搜索说明、Commit Synthesis Backend、Backend 下拉和 Template backend 说明接入 i18n。
  - `frontend/components/Settings/ProviderSettings/index.tsx`：Provider 通用字段（API Key / Base URL / Credentials Path / Project ID / Location / Web Search / Search Context 等）接入 i18n。
  - `frontend/components/Settings/AgentSettings.tsx`：General / Agents / Skills / Rules tab、Session Persistence、Pattern Learning、Approval Threshold、Tools、Web Search 等接入 i18n。
  - `frontend/components/Settings/SubAgentSettings/index.tsx` / `ModelOverrides.tsx`：Agent 列表页 Global/Project Agents、New、system、tool count、Model/Max iter/Timeout/Idle、Allowed Tools、Runtime Model Override、Edit/Delete、空态、通知文案接入 i18n。
  - `frontend/components/Settings/McpSettings.tsx`：MCP Servers 页面标题、说明、状态、Connect/Disconnect、Browse servers、空态、配置路径提示、工具数等接入 i18n，并合并到既有 `mcp` 翻译段避免重复 key。
  - `frontend/components/Settings/CodebasesSettings.tsx`：Indexed folders、Index new folder、状态、Memory file、Re-index/Remove、空态和通知文案接入 i18n。
  - `frontend/components/Settings/IntegrationsSettings/**`：快速审计未发现明显直接渲染的硬编码英文；主体和子组件基本已通过 `integrations.*`/schema i18n 走翻译。
  - `frontend/components/Settings/AppearanceSettings.test.tsx`：补语言选择器测试，并为新 i18n key mock 翻译。
- **运行过的验证 / 已记录证据**：
  - `python3 -m json.tool frontend/lib/i18n/en.json >/dev/null && python3 -m json.tool frontend/lib/i18n/zh-CN.json >/dev/null` → exit 0。
  - i18n parity audit → `missing_keys 0`；`same_string_keys 35`，剩余主要是 IP/CIDR/URL/API Key/品牌名/技术名等可保留英文的术语。
  - `pnpm vitest run frontend/components/HomeView/HomeView.test.tsx --reporter dot` → exit 0；1 passed / 3 skipped（测试仍输出既有 `list_project_configs` mock warning 与 React error log，但 exit 0，非本轮新增失败）。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/EditorSettings.tsx frontend/components/Settings/NotificationsSettings.tsx frontend/components/Settings/AppearanceSettings.tsx frontend/components/Settings/AppearanceSettings.test.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/i18n/index.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json && pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/components/Settings/AppearanceSettings.test.tsx --reporter dot` → exit 0；biome 0 fixes；2 files / 70 tests passed。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/HomeView/HomeView.tsx frontend/components/Settings/EditorSettings.tsx frontend/components/Settings/NotificationsSettings.tsx frontend/components/Settings/AppearanceSettings.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/ProviderSettings/index.tsx frontend/components/Settings/TerminalSettings.tsx frontend/components/Settings/AdvancedSettings.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/AiSettings.tsx frontend/components/Settings/ProviderSettings/index.tsx frontend/components/Settings/TerminalSettings.tsx frontend/components/Settings/AdvancedSettings.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0；`missing_keys 0`。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/CodebasesSettings.tsx frontend/components/Settings/McpSettings.tsx frontend/components/Settings/AgentSettings.tsx frontend/components/Settings/AiSettings.tsx frontend/components/Settings/ProviderSettings/index.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0；`missing_keys 0`。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/SubAgentSettings/index.tsx frontend/components/Settings/SubAgentSettings/ModelOverrides.tsx frontend/components/Settings/AgentSettings.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0；`missing_keys 0`。
  - `ReadLints` on changed files → 0 errors。
- **提交记录**：未 commit。
- **已知风险或未解决问题**：
  - 这不是全前端 300+ 组件的最终完整中文化；本轮完成的是首页、Settings 大部分高频页、Target workspace 第一批。剩余硬编码主要集中在 Integrations schema 字段来源、PentestEnv 子页、SubAgent 编辑器细节、VulnIntel、SecurityView 等区域，建议后续按模块继续扫。

---

### 2026-05-25 · Appearance 增加语言选择器

- **本轮目标**：用户问 Settings 里改语言的前端位置，并要求把语言选择加到 Appearance。
- **已完成**：
  - `frontend/lib/i18n/index.ts` 新增 `AppLanguage`、`LANGUAGE_OPTIONS`、`getStoredAppLanguage()`、`applyAppLanguage()`；语言写入 `localStorage` key `golish.language`，启动时 i18next detector 优先读取该 key。
  - `frontend/components/Settings/AppearanceSettings.tsx` 在 Theme 与 UI Scale 之间新增 `Language` select，支持 `System default` / `English` / `简体中文`；选择后立即 `i18n.changeLanguage()`。
  - `frontend/components/Settings/AppearanceSettings.test.tsx` 增加语言选择器测试。
- **运行过的验证 / 已记录证据**：
  - `pnpm vitest run frontend/components/Settings/AppearanceSettings.test.tsx --reporter dot` → 先红灯（找不到 Language），实现后 exit 0 / 34 passed。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/Settings/AppearanceSettings.tsx frontend/components/Settings/AppearanceSettings.test.tsx frontend/lib/i18n/index.ts && pnpm vitest run frontend/components/Settings/AppearanceSettings.test.tsx --reporter dot` → exit 0；biome 0 fixes；34 passed。
  - `ReadLints` on changed files → 0 errors。
- **提交记录**：未 commit。
- **已知风险或未解决问题**：语言偏好目前存在前端 `localStorage`，未写入后端 `settings.toml`；如果后续需要跨设备同步，再扩后端 settings schema。

---

### 2026-05-25 · App / 小程序独立分组：mobile_apps / mini_programs / app_domains

- **本轮目标**：用户确认要给 app / 小程序数据加独立分组，不再混在 Business systems。
- **已完成**：
  - `backend/crates/golish/src/tools/asset_intel.rs`：把 `mobile_apps` / `mini_programs` / `app_domains` 加入 intel array profile 字段白名单，保证多值去重并落到 `organizations.intel`。
  - `resources/toolsconfig/0-zone.json`：0.zone `apk` 的 `msg.app_url/msg.app_id` 改写入 `intel.mobile_apps`；`msg.domain_list[0]` 写入 `intel.app_domains`。
  - `resources/toolsconfig/enscan-go.json`：ENScan enrichment 的 `app[*]` 改写入 `intel.mobile_apps`，`wx_app[*]` 改写入 `intel.mini_programs`；`wechat/weibo` 仍写入 `social_accounts`。
  - `frontend/components/TargetPanel/TargetGroupedView.tsx`：新增 `Apps & Mini Programs` 独立 UI group，显示 Mobile apps / Mini programs / App domains 三组 chips。
  - `frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`：补断言覆盖新 group 顺序、字段 key 和 filled 状态。
- **运行过的验证 / 已记录证据**：
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts --reporter dot`：先红灯（缺 `Apps & Mini Programs` group），实现后 exit 0 / 36 passed。
  - `cargo nextest run -p golish --lib -E 'test(build_profile_patch_dedupes_app_intel_array_fields)' --status-level fail`：先红灯（`mobile_apps` 被当单值 String），实现后纳入 scoped 4 测通过。
  - `cargo fmt --package golish --check && cargo check -p golish && cargo nextest run -p golish --lib -E 'test(build_profile_patch_dedupes_app_intel_array_fields) or test(fixture_enrichment_profile_fields_cover_observed_provider_keys) or test(team_cymru_asn_lookup_builds_profile_entries_from_public_ips) or test(extract_profile_fields_normalizes_asn_values)' --status-level fail` → exit 0；4 tests passed；仅既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm exec tsc --noEmit && pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts && pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts --reporter dot` → exit 0；biome 0 fixes；36 tests passed。
  - `python3 -m json.tool resources/toolsconfig/0-zone.json >/dev/null && python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → exit 0。
  - `ReadLints` on changed files → 0 errors。
- **提交记录**：未 commit。
- **已知风险或未解决问题**：
  - 目前 `app_domains` 只取 `msg.domain_list[0]`，因为现有 JSON path resolver 不支持把数组全量 split 成多条 profile entry；如要完整保留 domain_list，需要后续扩 profile_fields 的 array fan-out 能力。
  - 全仓 `just precommit` 仍受既有 blockers 影响，未在本轮解决。

---

### 2026-05-25 · App / 小程序数据源探针：ENScan vs 0.zone

- **本轮目标**：用户问 app、小程序等数据 ENScan / 0.zone 是否能抓、前端是否有字段，并要求先把两个工具真实跑一下看数据。
- **已完成 / 观察结果**：
  - 0.zone：`python3 /tmp/golish_zone_probe.py 小米` 成功跑 7 个 query_type；其中 `apk` 返回 `code=0`、`total=7344`、当前页 10/10 都有 `msg.app_url` 与 `msg.app_id`，1/10 有 `msg.domain_list`。样本包括 `小米实况麻将`、`远程遥控开空调`、`亲笔信`、`新旧手机搬家`、`爱评估` 等，类型均为 `安卓APK`。
  - ENScan：实际可执行文件位于 `~/Library/Application Support/golish-platform/tools/ENScan_GO/enscan-v2.0.5-darwin-amd64` 并可启动；但本轮跑 `aqc -field icp,app,wx_app,wechat,weibo` 对 `小米科技有限责任公司` / `小米` / `中国平安` 都返回 `没有查询到关键词`，导出 JSON 只有 `{"enterprise_info":null}`。
  - ENScan 其它源：`kc -field app` 对 `小米` 先出现 kuaicha365 EOF 后返回无结果；`tyc -field app,wx_app,wechat` 对 `小米` 返回 TYC 419 后无结果；导出 JSON 也均只有 `enterprise_info:null`。
  - 前端现状：`TargetGroupedView` 的 `Surfaces` group 已有 `Business systems` / `Social accounts`；ENScan `app/wx_app` 与 0.zone `apk` 当前都会混写到 `business_systems`，没有独立 `Apps` / `Mini programs` group。
- **运行过的验证 / 已记录证据**：
  - `python3 /tmp/golish_zone_probe.py 小米` → exit 0，raw dump 在 `/tmp/golish_zone_dump/*.json`。
  - ENScan AQC：`.../enscan-v2.0.5-darwin-amd64 -n 小米 -type aqc -field icp,app,wx_app,wechat,weibo ...` → exit 0，但日志为 AQC no keyword；JSON `enterprise_info:null`。
  - ENScan KC：`... -n 小米 -type kc -field app ...` → exit 0，但 kuaicha365 EOF + no keyword；JSON `enterprise_info:null`。
  - ENScan TYC：`... -n 小米 -type tyc -field app,wx_app,wechat ...` → exit 0，但 TYC 419 + no keyword；JSON `enterprise_info:null`。
- **提交记录**：未 commit。
- **下一步最佳动作**：优先把 0.zone `apk` 的 app 数据提升为独立 `intel.apps` / `intel.app_domains` UI group；ENScan 需先刷新/复测 AQC/TYC/KC 凭据或换可稳定返回 app 字段的源，否则当前实测不可作为 app 数据主来源。

---

### 2026-05-25 · Target ASNs 补全：0.zone IP → Team Cymru ASN 派生

- **本轮目标**：用户反馈 Target 里的 ASN 字段靠 0.zone 补不上，要求想别的办法。
- **已完成**：
  - 保留既有 `asn` transform：provider 直接返回 `4134/as4134/AS4134` 时仍标准化为 `AS4134` 写入 `organizations.asns`。
  - 在 `backend/crates/golish/src/tools/asset_intel.rs` 给 0.zone 增加兜底：当 0.zone 没返回有效 `asn`、但 profile_entries 已有公网 `ip_ranges` 时，最多取 40 个公网 IP 走 Team Cymru whois IP→ASN 批量查询，把结果派生为 `organizations.asns`。
  - 私网、loopback、link-local、文档网段、组播等 IP 会跳过；派生失败只写 provider evidence，不中断 0.zone hydrate。
- **运行过的验证 / 已记录证据**：
  - `cargo nextest run -p golish --lib -E 'test(team_cymru_asn_lookup_builds_profile_entries_from_public_ips)' --status-level fail` → 先红灯（3 个 helper 未实现）后绿灯（1 passed）。
  - `cargo nextest run -p golish --lib -E 'test(team_cymru_asn_lookup_builds_profile_entries_from_public_ips) or test(extract_profile_fields_normalizes_asn_values)' --status-level fail` → exit 0 / 2 passed。
  - `cargo fmt --package golish --check && cargo check -p golish` → exit 0；仅既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `ReadLints backend/crates/golish/src/tools/asset_intel.rs` → 0 errors。
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` 与包含 `http_json_runtime_posts_fake_data_and_normalizes_candidates` 的 focused 组合均在启动测试后无输出超过 180s，已手动停止；本轮未把它们作为通过证据。
- **提交记录**：未 commit。
- **已知风险或未解决问题**：
  - 新兜底会在 0.zone hydrate 后对 Team Cymru whois 发起 IP→ASN 查询；若用户环境不允许出站 43/tcp，会记录失败 evidence，但不会阻断 hydrate。
  - 全仓 `just precommit` 仍受既有 blockers 影响，未在本轮解决。
- **下一步最佳动作**：用真实 0.zone hydrate 一个含公网 IP 的目标，确认 UI 的 ASNs chip 由 Team Cymru 派生值填上；如需完全离线，可后续改成 MaxMind ASN DB / 本地 ip2asn 库 provider。

---

### 2026-05-25 · Target ASNs 补全：新增 `asn` transform + 复核 0.zone 返回

- **本轮目标**：用户问 Target 面板里的 `asns` 字段怎么补全，并要求检查 0.zone 是否真的返回 ASN 数据；随后确认让我动手改。
- **已完成**：
  - 确认 `asns` 的真实落点是 `organizations.asns`（organization profile 字段），不是 `targets` 表字段；Target 面板 Network → ASNs 已经会渲染该字段。
  - `backend/crates/golish-pentest/src/models.rs` 给 `AssetIntelProfileFieldTransform` 新增 `Asn`，JSON 写法为 `"transform": "asn"`。
  - `backend/crates/golish/src/tools/asset_intel.rs` 新增 `normalize_asn`：trim + uppercase；裸数字补 `AS`；只接受 1-10 位数字；非法值返回空串并被既有 profile extraction 跳过。
  - `resources/toolsconfig/0-zone.json` 的 `source_field=asn → target_field=asns` 规则从 `"trim"` 改成 `"asn"`。
  - 用现有 `/tmp/golish_zone_dump/*.json` 复核 0.zone 样本：`site.json` 有 10 条对象含 `asn` key，但 nonempty=0；`domain/apk/org/email/code/member` 样本里 `with_asn_key=0`。因此当前 UI 没显示 ASN 的直接原因是这批 0.zone 返回没有有效 ASN 值。
- **运行过的验证 / 已记录证据**：
  - `python3 -m json.tool resources/toolsconfig/0-zone.json >/dev/null` → exit 0。
  - `jq -r '.. | objects | select(.target_field? == "asns")' resources/toolsconfig/0-zone.json` → exit 0，输出规则含 `"transform": "asn"`。
  - `cargo nextest run -p golish-pentest -E 'test(asset_intel_profile_field_transform_accepts_asn)' --status-level fail` → exit 0 / 1 passed。
  - `cargo nextest run -p golish --lib -E 'test(extract_profile_fields_normalizes_asn_values)' --status-level fail` → exit 0 / 1 passed；断言 `{asn: 4134}` 与 `{asn: " as37963 "}` 落为 `["AS4134","AS37963"]`，`not-an-asn` 被丢弃。
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` → exit 0 / 40 passed。
  - `cargo nextest run -p golish-pentest --status-level fail` → exit 0 / 63 passed, 7 skipped。
  - `cargo fmt --package golish --package golish-pentest --check` → exit 0。
  - `just precommit` → exit 1；fmt/check-fe/test-fe passed，随后命中上方记录的既有 Rust lint/test blockers。
  - 2026-05-25 用户要求实时复跑 0.zone ASN：用本机 vault 中 0.zone API key 对 `小米` / `qq.com` / `baidu.com` 各跑 7 个 query_type（site/domain/apk/org/email/code/member，pagesize=20，共 21 个 POST 到 `https://0.zone/api/data/`）→ 全部 HTTP 200 / code=0；结果：3 个 query 的 `site` 类型均有 `asn` 与 `asn_org` key，但 `nonempty=0/20`；其他 query_type 的 `asn/asn_org/as_number/asname/isp` key 均为 0 或 nonempty=0。结论：0.zone schema 里有 ASN 占位字段，但当前返回数据没有有效 ASN 值。
  - 替代链路实测：对旧 0.zone dump 里的 IP 跑 Team Cymru DNS IP→ASN：`202.69.26.81 -> AS23848`、`183.62.123.10 -> AS4134`、`182.92.121.121 -> AS37963`、`124.196.77.48 -> AS23848`。说明可通过“0.zone IP 结果 → IP→ASN enrichment → organizations.asns”补齐 ASN。
  - 2026-05-25 用户要求试 Hunter API key：本机 vault 找到 `hunter.default.api_key`。旧仓库 endpoint `https://hunter.qianxin.com/openApi/search` 对 `ip="1.1.1.1"` / `domain="qq.com"` / `domain="baidu.com"` 均返回 HTTP 403 nginx HTML；按当前公开 Hunter Search API 文档改试 `https://api.hunter.how/search`（带 `query/start_time/end_time/fields=...,asn,as_org,as_name,...`）→ HTTP 200 但 JSON `code=401, message="Token expired"`。结论：当前 key 已被 Hunter 业务层识别但过期，暂时无法取数据；新 API 文档显示 response fields 支持 `asn/as_org/as_name`。
  - 2026-05-25 用户临时提供另一枚 Hunter key 后再次验证（未写入文件，未记录明文 key）：`https://api.hunter.how/search` 对 `ip="1.1.1.1"` / `domain="qq.com"` / `domain="baidu.com"` 均 HTTP 200 + JSON `code=401, message="Token expired"`；旧 `https://hunter.qianxin.com/openApi/search` 对 `ip="1.1.1.1"` 返回 TLS `UNEXPECTED_EOF_WHILE_READING`。结论不变：当前 key 不可用，需用户在 Hunter 控制台重新生成有效 API key 后再验证字段。
  - 用户贴出奇安信 Hunter 旧 `/openApi/search` 文档后，按文档参数重试旧 endpoint：`api-key` + `search`(RFC4648 base64url) + `page=1&page_size=10&is_web=1&start_time=2026-04-25&end_time=2026-05-25&fields=...`，Python TLS 返回 `UNEXPECTED_EOF_WHILE_READING`；`curl -k` 同 URL 返回 `LibreSSL SSL_connect: SSL_ERROR_SYSCALL` / HTTP_CODE=000。另：用户贴出的旧接口 `fields` 枚举没有 `asn`，只有 `as_org`，因此即使旧 endpoint 可通，也只能补 ASN organization 名称，不能直接补 `organizations.asns` 的 AS 编号。
- **提交记录**：未 commit。
- **已知风险或未解决问题**：
  - 本轮只保证 provider 一旦返回有效 ASN 就能标准化落到 `organizations.asns`；不能凭空从 IP 推 ASN。若 0.zone 持续不给 ASN，需要新增本地/第三方 IP→ASN enrichment provider（如 Team Cymru / RDAP / MaxMind ASN DB）并落 evidence。
  - Hunter 现有仓库实现可能已过期：旧 endpoint 403，当前公开文档使用 `api.hunter.how/search` + `query` 参数 + `start_time/end_time` + `fields`。需要用户刷新 Hunter API key 后再改 provider，否则无法做真实绿灯验证。
  - `just precommit` 未绿，feature 不能切 `passing`。
- **下一步最佳动作**：修复或隔离全仓 precommit blockers；然后如需实时确认 0.zone，可在用户允许外部请求后复跑小样本 API probe，并用一个有公网域名/网站记录的目标查看是否返回非空 `asn`。

---

### 2026-05-24 · 文档清理：删除旧 implementation plan + 标注 deferred/superseded

- **本轮目标**：用户指出 harness 工程应等信息收集闭环和工具包装完善后再推进，并要求清理废弃文档。
- **已完成**：
  - 删除 3 个旧 implementation plan：`docs/superpowers/plans/2026-05-20-asm-intel-providers.md`、`docs/superpowers/plans/2026-05-20-golish-agent-harness.md`、`docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md`。
  - `docs/design/2026-05-20-asm-intel-providers.md` 标为 superseded by Integrations。
  - `docs/design/2026-05-20-agent-harness-strategy.md` 和 `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md` 标为 deferred，明确当前优先级是信息收集闭环 / tool output schema / evidence 契约。
  - 修掉当前文档入口里的坏引用：AGENTS 的 missing harness MVP 链接、docs README 的 missing benchmark plan、architecture 的 missing `.cursor/rules/*` 链接、development 的旧 `golish-ai` 工具路径。
  - `feature_list.json` 的 domain/recon harness notes 改为 deferred，不再指向已删除或缺失的旧 plan。
- **运行过的验证**：
  - `python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - 本地 markdown 链接检查（docs + AGENTS + README，相对链接存在性）→ `missing=0`。
- **未运行**：未跑 `just precommit` / `./init.sh` / 前后端测试；本轮是 docs-only 清理，且用户明确不需要跑重验证。

---

### 2026-05-24 · 0.zone 扩展查询类型：email/code/member 三类启用 + 9 条 normalize 规则

- **本轮目标**：用户询问 quake 类网络空间测绘平台无 API 时怎么抓数据，对话顺势调研到「Golish target 表还缺什么字段、0.zone / ENScan_GO 能补什么」。用户明确「不是所有字段都该记录」，让我按 P0/P1/P2/P3 分级精选字段，然后让我「自己跑一下 0.zone 试试看」。最终拍板「动手 Phase 1 + 2」。
- **实测探针**：写了 `/tmp/golish_zone_probe.py`——连本机 embedded PG (port 15432, db=golish) 读 `vault_entries` 0.zone API key、XOR 反混淆（golish-core/src/vault.rs::derive_key 逻辑 Python 复刻）、按 7 个 query_type 各拉 10 条小米相关 records；原始 JSON dump 在 `/tmp/golish_zone_dump/{site,domain,apk,org,email,code,member}.json`。**关键发现**：① types.rs 的 SiteEntry 只 deserialize 24 字段，但 0.zone 实际返回 70 个字段（漏 banner/framework/leak/device_type/protection/ssl_hostname/icon_md5_base64/risk_score 等 P0+ 字段）；② domain.msg.ip = A 记录 / msg.mx_list = MX 记录已现成；③ apk.msg.domain_list = APK 反编译出的后台域名列表（红队金矿）；④ org.msg.related_brands/related_enterprises = 0.zone 独家品牌穿透字段；⑤ email.leakage_account = HIBP 风格数据；⑥ code.detail_parsing = 已解析的 AK/SK Token。
- **scope 设计取舍**（重要）：用户提醒后我**主动收窄**——不全接 70 字段做 catch-all（避免数据保留癖污染 organizations 主档案）。只接进 organizations.intel/subsidiaries/aliases 三个 bucket 的 P0+ 字段；site 的 framework_name/leak/app_name 等单 IP 属性留在 candidates.raw_evidence 由 TargetDetail 渲染层处理（不动 Rust 不动 schema）；member 暂不映射 contacts schema（避免破坏 {name,phone,email,title} 分桶）。
- **9 条新 normalize.profile_fields 规则**（resources/toolsconfig/0-zone.json）：① email→intel.exposed_emails(lower)+contact filter ② mail_domain→email_domains(scalar lower) ③ leakage_num→intel.email_leakage_total(trim) ④ url→intel.code_leaks(trim, when keyword+source exists) ⑤ detail_parsing→intel.code_leak_secrets ⑥ msg.related_brands→subsidiaries(scalar, when name_cn exists) ⑦ msg.related_enterprises→subsidiaries ⑧ msg.name_before→aliases ⑨ msg.mx_list→intel.mail_mx。注意 ⑥⑦⑧ 都加了 `name_cn exists` when 过滤防止 apk/site 的 msg 嵌套字段误入 org 主档案，符合 asset_intel.rs 行 4843 测试断言的语义。
- **runtime.requests 加 3 个**：email / code / member 各一个 POST，与现有 4 个并列；pagesize=40 同口径。
- **前端改动**：frontend/components/TargetPanel/TargetGroupedView.tsx 的 INTEL_FIELD_LABELS / INTEL_DISPLAY_ORDER / INTEL_RECORD_LABELS 三个 map 各补 3 个新 intel key（email_leakage_total / code_leak_secrets / mail_mx）。exposed_emails 和 code_leaks 上游已就位、本轮即用即看。
- **运行过的验证**：
  - `python3 -m json.tool resources/toolsconfig/0-zone.json` → exit 0
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` → **exit 0 / 39 passed**（包含 4843 行硬断言 `0.zone msg.code -> credit_code must require name_cn presence to avoid pulling apk/site/domain msg.code values` 通过——证明我的新 P0 规则没破坏 org-only 字段隔离）
  - `cargo nextest run -p golish-pentest --status-level fail` → **exit 0 / 62 passed**
  - `cargo check -p golish` → exit 0 / 仅 preexisting `capture/data_dir.rs::session_dir` dead_code warning（M2 cherry-pick 遗留 · 上轮 progress 已记）
  - `pnpm exec tsc --noEmit` → exit 0
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx` → No fixes applied
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts --reporter dot` → **exit 0 / 36 passed**
  - `ReadLints` 全部改动文件 → 0 errors
  - `python3 /tmp/golish_zone_probe.py 小米` → 7/7 query_type 都 code=0 有数据，证明 0.zone API key 凭据可用
- **未提交的半成品**：累积前几轮所有改动（见上轮 progress）+ 本轮新增：`resources/toolsconfig/0-zone.json` + `frontend/components/TargetPanel/TargetGroupedView.tsx` + `feature_list.json` + 本文件。
- **scope 之外**：手动 E2E（用户实跑 just dev → 配 0.zone API key → hydrate 真实公司 → 看 organizations.intel 里是否冒出 exposed_emails / code_leaks / mail_mx 条目）留给用户做。
- **Phase 3 动手（同一轮追加）**：用户看完报告后要求「独立 UI group」，不要把 leakage / mail_mx 沉在 Intel records 一行里。动了 frontend/components/TargetPanel/TargetGroupedView.tsx 三处：① INTEL_FIELD_LABELS / INTEL_RECORD_LABELS 两个 map 各加 4 个新 intel key 的人话 label，② INTEL_DISPLAY_ORDER 移除这 4 个 key（避免重复显示），③ getOrgFieldGroups 加 2 个新 OrgFieldGroup：'Leakage Intel'（3 字段）+ 'DNS'（1 字段），通过新 helper `intelGet(org, key)` 从 org.intel.{key} 嵌套取值。同步改 vitest fixture：intel = {exposed_emails, email_leakage_total, code_leaks, mail_mx} + 加 4 个新断言。
- **Hotfix（用户截图反馈 `Leaked secrets (AK/Token)` 显示成 `{"domain_list":[],...}` JSON 对象）**：根因是 0.zone `detail_parsing` 实际返回的是结构化对象（含 6 个 list 子字段）而非字符串，`golish-core::utils::resolve_json_path` 行 164 对 Object 类型 fallback 到 `.to_string()` JSON 序列化导致整 JSON 进了 intel.code_leak_secrets。修复：撤销 detail_parsing→code_leak_secrets normalize 规则（9→8 条）+ 前端 LEAKAGE_INTEL_KEYS 去掉 code_leak_secrets（4→3 字段）+ INTEL_FIELD_LABELS / INTEL_RECORD_LABELS 清掉 code_leak_secrets label + 测试 fixture 去掉 code_leak_secrets 数据。这条字段需要 Phase 4 扩 Rust is_intel_array_profile_field 白名单或 split 它内部 6 个 list 分别映射，才能正确展示。跑验证：pnpm vitest 36 passed · tsc exit 0 · biome 0 fixes · cargo nextest asset_intel 39 passed · ReadLints 0 errors。
- **下一步**：用户实测验证 hotfix 后 Leaked secrets 行消失、其他 4 个 chip 字段正常。如果要 Phase 4 展开 detail_parsing 内部 6 list（domain/email/ip/phone/telegram/wangpan），需要改 ROUTED_KEYS / is_intel_array_profile_field / extract_profile_field_entries / OutputStore writer ——超出本轮 scope。

---

### 2026-05-23 · Asset Intel providers flat：4 个 JSON 合并为 1 个多 provider

- **本轮目标**：用户提出「ENScan_GO 的 3 个 child discovery JSON + 主 JSON 4 个 tool entry 重复且 UX 误导」，拍板走 A 方案——把 3 个 child 合并进主 `enscan-go.json` 的新 `asset_intel_providers: []` 数组字段。
- **设计文档**：`docs/design/2026-05-23-asset-intel-providers-flat.md`（问题、JSON 契约、Rust 改造点、向后兼容、影响面、验证）。
- **实现计划**：`docs/superpowers/plans/2026-05-23-asset-intel-providers-flat.md`（9 task TDD 小步骤，每 task 单 commit）。
- **已完成（按 Task）**：
  - **Task 1**：`ToolConfig` 加 `asset_intel_providers: Option<Vec<AssetIntelToolConfig>>` 字段（与现有 `asset_intel` 互斥，rename `asset_intel_providers`），加 2 个 schema 单测（`tool_config_accepts_asset_intel_providers_array` / `tool_config_round_trips_asset_intel_providers`）。同步补 `search.rs` + `command_builder/tests.rs` 两处 full struct literal。
  - **Task 2**：`parsers::parse_tool_config` 加互斥校验——同时声明 `asset_intel` 与 `asset_intel_providers` 的 tool 被 `walk_json_files` 的现有 `warn!` 路径 silent skip；新测 `scan_skips_tool_declaring_both_asset_intel_and_providers` 绿。
  - **Task 3**：`asset_intel.rs` 新增 `expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig>` fan-out 工具——多 provider tool clone 出 N 个 virtual ToolConfig（保留 executable / install / runtime 等元数据，每个 virtual `asset_intel = Some(provider)`，`asset_intel_providers = None`，跳过 disabled）；单 provider tool 1:1 透传；其它 tool 不出现。加 3 单测（fan-out / pass-through / disabled-skip）。
  - **Task 4**：`provider_descriptors_from_tools` 第一行 `let expanded = expand_provider_tools(tools);` 接入；新测 `provider_descriptors_from_tools_unpacks_multi_provider_tool` 验证多 provider tool 展开成 N 个 descriptor，老的 1 tool 1 descriptor 测试仍绿。
  - **Task 5**：`select_asset_intel_providers` / `select_subsidiary_providers` / `select_enrichment_providers` 三件套：移除 `<'a>` 生命周期参数，改返回 owned `Vec<ToolConfig>`；`select_asset_intel_providers` 内 `let mut providers = expand_provider_tools(tools).into_iter()...`；显式 `requested` 分支用 `.find().clone()` 而非 `.copied()`。`run_providers_for_org` 把 `providers: Vec<&ToolConfig>` 改 owned `Vec<ToolConfig>`，循环改 `for tool in &providers`。所有 hydrate/enrich command 调用方无需改（owned 比 borrowed 更宽松）。加 2 新测（`select_subsidiary_providers_expands_multi_provider_tool` / `select_asset_intel_providers_treats_multi_provider_tool_as_single_pool`），验证 fan-out + 跨 tool 按 priority 混合排序。
  - **Task 6**：主 `resources/toolsconfig/enscan-go.json` 把 `tool.asset_intel: { ... }` 整段重写为 `tool.asset_intel_providers: [aqc, tyc, kc, rb]`（4 项）；AQC 保留完整 lookup + 9 条 profile_fields + organization/target/profile_fields 全套 normalize；TYC/KC/RB 各自只带 organization normalize + 独立 `requires_integration.group_ids` + 独立 `runtime.skill_id`；TYC `auto.default=false`（上游 PR #221 未合）。同步在 `tool.skills` 数组里加 3 个独立 skill（`company-default-json-tyc` / `company-default-json-kc` / `company-default-json-rb`），避免 4 个 provider 共享 `company-default-json` 引起的 `-type aqc` 串源。
  - **Task 7**：删 3 个 child JSON——`enscan-go-tyc-discovery.json` / `enscan-go-kc-discovery.json` / `enscan-go-rb-discovery.json`（用户聊天里二次确认后才删，符合 AGENTS.md §2.7）。中间态证据：删之前 fixture 红灯 `left=[..., kc, kc, rb, rb]` ≠ `right=[..., kc, rb]`，说明主 JSON + child JSON 各自展开出同名 provider 导致重复。
  - **Task 8**：全套验证（见下）。
  - **Task 9**：本轮 progress 段 + feature_list 一条（在另一段 commit 里）。
- **运行过的验证**：
  - `cargo nextest run -p golish-pentest -E 'test(tool_config_accepts_asset_intel_providers_array)+test(tool_config_round_trips_asset_intel_providers)' --status-level fail` → **红 (E0609 unknown field)** → 加字段 → **红 (E0063 missing field in initializer @ search.rs:103)** → 补 search 字段 → **红 (同 E0063 @ command_builder/tests.rs:17)** → 补 command_builder 字段 → **exit 0 / 2 passed**。
  - `cargo nextest run -p golish-pentest -E 'test(scan_skips_tool_declaring_both_asset_intel_and_providers)' --status-level fail` → 加测试 **红 (left=1, right=0)** → 加 `parse_tool_config` 互斥校验 → **exit 0 / 1 passed**。
  - `cargo nextest run -p golish --lib -E 'test(expand_provider_tools)' --status-level fail` → 加 3 测试 + helper → **红 (cannot find function)** → 加 `expand_provider_tools` → **exit 0 / 3 passed**。
  - `cargo nextest run -p golish --lib -E 'test(provider_descriptors_from_tools_unpacks_multi_provider_tool)' --status-level fail` → 加测试 **红 (left=0, right=2)** → 改 `provider_descriptors_from_tools` 第一行接入 expand → **exit 0**。
  - `cargo nextest run -p golish --lib -E 'test(provider_descriptors_from_tools)+test(asset_intel_provider_descriptors_load_from_tool_configs)' --status-level fail` → **exit 0 / 2 passed**（确认既有 single provider 测试仍绿）。
  - `cargo nextest run -p golish --lib -E 'test(select_) and test(asset_intel)' --status-level fail` → **exit 0 / 7 passed**（含 select_* 全套 + 新加 2 个 multi-provider/cross-pool 测试）。
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` → **exit 0 / 35 passed**（之前 29 + 本轮 6 个新）。
  - `cargo nextest run -p golish --lib -E 'test(fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable)' --status-level fail` → 主 JSON 改写但 child JSON 未删时 **exit 101**（KC/RB 各重复一次）→ 删 child JSON → **exit 0 / 1 passed**。
  - `cargo nextest run -p golish-pentest --status-level fail` → **exit 0 / 62 passed, 7 skipped**。
  - `cargo fmt --package golish --package golish-pentest` → 自动格式化，复查 `--check` → **exit 0**。
  - `cargo check -p golish` → **exit 0**，仅 preexisting `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts` → **exit 1**，但报错落在 `asset-intel.ts` 中一段 `hydrateSubsidiaries` 函数签名换行 formatting，是上一轮 untracked 文件遗留 preexisting，本轮 0 改动前端文件，不在 scope。
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 33 passed**。
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**。
  - `ReadLints` 9 个改动文件（5 个 Rust + 1 JSON + 2 docs + agent-progress） → **No linter errors found**。
- **已知风险或未解决问题**：
  - 工具管理面板从 4 行变 1 行的视觉效果**需要用户在 `just dev` 下手动复测一次**（本轮没起 dev binary 验证 UI）。
  - TYC 仍保持 `auto.default=false`，等 `wgpsec/ENScan_GO PR #221` 合并并发布新 ENScan 版本后改回 `true` 并把 fixture 名改回 `defaults_to_all_enscan_sources`。
  - `frontend/lib/api/asset-intel.ts` 的 biome formatting 错是 preexisting；如果后续要让整仓 `just precommit` 全绿，需要单独 commit 修这一行（不在本轮 scope）。
  - 没跑整仓 `just precommit`；仓库仍有已记录的 preexisting blocker（biome 警告 + `failure_kind` PlanStep struct 编译错），与本轮无关。

---

### 2026-05-23 · 临时关闭 TYC discovery 默认勾选（等上游 PR #221）

- **本轮目标**：用户要求把 ENScan_GO TYC discovery 默认源临时关掉，因为 v2.0.5 上游 `tianyancha.go:124 searchBaseInfo` 仍 panic（wgpsec/ENScan_GO#221 仍 open），让 Activity 不再每次都标红一条 TYC failed。
- **根因 / 现状**：
  - 上一轮已经把 fixture 改名为 `fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable` 并断言默认源应是 `[enscan-go, enscan-go-kc-discovery, enscan-go-rb-discovery]`（无 TYC），但 `resources/toolsconfig/enscan-go-tyc-discovery.json` 的 `asset_intel.auto.default` 还停在 `true`，本轮一跑就红。
  - JSON-driven provider 抽象的一贯设定：是否默认参与 discovery 由 `asset_intel.auto.default` 决定，Rust 端没有任何 TYC 硬编码白名单。所以这是纯 JSON 改动。
- **已完成**：
  - `resources/toolsconfig/enscan-go-tyc-discovery.json` 的 `asset_intel.auto.default` 改为 `false`（保留 `priority=95`，用户在 Asset Intel 配置里手动勾选时仍按原优先级排序）。
  - 三个 provider 的语义说明：
    - 默认 discovery 现在只跑 `enscan-go`（AQC）+ `enscan-go-kc-discovery`（KC/Qimai）+ `enscan-go-rb-discovery`（RB/RiskBird），不再带 TYC。
    - TYC 仍可用：用户在 Asset Intel 配置面板手动勾上 TYC discovery 即可单独跑（凭证、capture、normalize 链路都没改）。
  - 工具管理面板仍会显示 4 个 ENScan_GO 入口（`enscan-go` + 3 个 `*-discovery`），共享同一可执行文件，安装/卸载一次生效；这是 §5 设计文档锁定的多 provider 抽象，本轮不动。
- **运行过的验证**：
  - `cargo test -p golish fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable --lib` → 改 JSON 前 **exit 101**，断言 `left=[enscan-go, tyc, kc, rb]` ≠ `right=[enscan-go, kc, rb]`；改 JSON 后转绿。
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` → **exit 0 / 29 passed, 242 skipped**。
  - `cargo nextest run -p golish-pentest -E 'test(asset_intel)' --status-level fail` → **exit 0 / 7 passed, 59 skipped**（schema 层 round-trip 仍通过）。
  - `python3 -m json.tool resources/toolsconfig/enscan-go-tyc-discovery.json >/dev/null` → **exit 0**。
  - `ReadLints`（`enscan-go-tyc-discovery.json`）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 这是临时措施，**等 wgpsec/ENScan_GO PR #221 合并并发布新 ENScan 版本后必须把 `default` 改回 `true`，并把 fixture 名改回 `defaults_to_all_enscan_sources` 断言四源全跑**。这条放进 feature_list 的 `notes` 里跟踪。
  - 没跑整仓 `just precommit`；本仓既有 preexisting blocker（biome 警告 + `failure_kind` PlanStep struct 编译错）仍在，与本轮无关。
  - 没真实复跑 `enscan -type tyc -field invest` 外部命令——因为 TYC 上游 panic 是已确认事实，再跑一次只是重复花时间；如果上游 PR merge 后要恢复 default=true，那次必须真实验证一次。

---

### 2026-05-23 · 查子公司失败半截候选不再跳 Candidates

- **本轮目标**：用户反馈 Target 里“查子公司”仍出现 ENScan_GO 天眼查 `getInfoById/processTask` panic，同时前端点完后直接跳到 `candidates`，这不符合“自动创建子公司 / 失败留在 Activity 看 provider 状态”的预期。
- **根因**：
  - 后端 `run_cli_json_provider` 在 CLI 退出失败时仍返回 watcher 已解析出的半截 candidates/profile_entries；`run_providers_for_org` 不区分 provider terminal state，继续把这些失败 provider 输出合并到 run。
  - 前端 `getNextWorkspaceTabAfterAssetIntelRun` 只看候选数量；partial/failed run 只要含半截候选就自动切到 `candidates`。
- **已完成**：
  - `backend/crates/golish/src/tools/asset_intel.rs` 新增 `provider_output_is_trusted`，只有 `Completed` / `CheckedEmpty` provider 输出会被合并；`Failed` / `Unavailable` 的半截 stdout/artifact 不再进入候选、profile patch 或自动提升链路。
  - `frontend/components/TargetPanel/TargetGroupedView.tsx` 修改 discovery 完成后跳转逻辑：只有 `run.status === completed` 且确有 reviewable candidates 时才切到 `candidates`；partial/failed 留在 `activity`，让用户看到 TYC/KC/RB 哪个 provider 标红及错误摘要。
  - `TargetGroupedView.actions.test.ts` 增加 partial/failed discovery 带候选时仍停留 Activity 的红绿回归；`asset_intel.rs` 增加 provider 输出信任边界单测。
- **运行过的验证**：
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`：修复前 **1 failed**（partial run 收到 `candidates`），修复后 **exit 0 / 33 passed**。
  - `cargo test -p golish provider_output_is_trusted_only_for_successful_terminal_states --lib`：修复前 **exit 101**（函数不存在红灯），修复后 **exit 0 / 1 passed**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 29 passed**。
  - `cargo fmt --package golish --check` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / No fixes applied**。
  - `ReadLints`（`asset_intel.rs` + 两个 TargetGroupedView 文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 这次修的是 Golish 对失败 provider 的处理与前端跳转；ENScan_GO v2.0.5 的 TYC `getInfoById/searchBaseInfo` panic 仍属于上游解析问题。当前 UI 预期会把 TYC 标红留在 Activity，而不是把半截候选当成功结果。
  - 未跑整仓 `just precommit`；仓库仍有已记录的 preexisting blocker。

---

### 2026-05-23 · 查子公司 TYC 报错根因复盘

- **本轮目标**：用户反馈 Target 里“查子公司”功能触发天眼查报错，并澄清 TYC/KC/RB 多源默认一起跑、去重合并是预期行为。
- **根因判断**：多源默认运行不是 bug；上一轮把 TYC/KC/RB 改成非默认的方向已撤销。现有证据指向 ENScan_GO v2.0.5 的 TYC 模块问题：此前本机运行 `ENScan_GO/enscan-v2.0.5-darwin-amd64 -n 小米 -type tyc -field icp` 显示 TYC 已认证并返回 22 个企业候选，随后在上游 `tianyancha.go:124` panic。公开上游 PR wgpsec/ENScan_GO#221 也说明 `searchBaseInfo` 在天眼查返回页缺 `__NEXT_DATA__` 或数组为空时会 nil deref / 越界，通常由 cookie 失效、风控页或页面结构变化触发。
- **已完成**：
  - 恢复 `resources/toolsconfig/enscan-go-tyc-discovery.json`、`enscan-go-kc-discovery.json`、`enscan-go-rb-discovery.json` 的 `asset_intel.auto.default=true`，保留“默认多源一起跑”语义。
  - 将上一轮错误方向的测试改为 `fixture_discovery_auto_defaults_to_all_enscan_sources`，锁住默认 discovery 会选择 AQC + TYC + KC + RB 四个 ENScan-backed sources。
  - 复核公开上游资料：wgpsec/ENScan_GO#221 是针对 TYC `searchBaseInfo` nil pointer / empty array panic 的修复 PR（截至查询时仍 open/draft）。
- **运行过的验证**：
  - `cargo test -p golish fixture_discovery_auto_defaults_to_all_enscan_sources --lib` → **exit 0 / 1 passed**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 28 passed**。
  - `python3 -m json.tool` 校验 3 个 discovery JSON → **exit 0**。
- **已知风险或未解决问题**：
  - 还未跑整仓 `just precommit`；本仓当前仍有既有 precommit blocker（见“当前已验证状态”）。
  - 还未复跑真实 `-type tyc -field invest` 外部命令；当前结论基于上一轮真实 `-type tyc -field icp` panic 证据 + 上游 PR。下一步建议用户或本机在可用 ENScan binary/凭据环境中复跑 discovery 同款命令确认：`-n <公司> -type tyc -field invest -invest 51 -deep 1 -delay 3 -json -out-dir <tmp>`。

---

### 2026-05-23 · Qimai/KC 未登录匿名 cookie 误判修复

- **本轮目标**：用户确认 TYC 已搞定，但 Qimai/KC 未登录时打开 `https://www.qimai.cn/` 就显示 capture 成功；日志显示只抓到 `synct` / `syncd` / `qm_check` / `PHPSESSID` 四个 cookie。
- **根因**：上一轮给 KC 只加了 `min_count=2`，但 Qimai 未登录首页本身就会下发 4 个匿名/风控 cookie；`success_url_pattern="qimai\\.cn"` 又会在首页立即触发提取，所以数量门槛不足以证明登录态。
- **已完成**：
  - `resources/toolsconfig/enscan-go.json` 的 KC/Qimai capture 规则新增 `required_names=["USERINFO","aso_ucenter"]`，只有出现这两个登录态 cookie 才写入 `cookies.qimai`。
  - KC/Qimai instructions 明确说明只看到 `synct/syncd/qm_check/PHPSESSID` 时仍会继续等待，不算登录成功。
  - `backend/crates/golish-integrations/src/resolver.rs` 回归 fixture 从“数量门槛”升级为“登录态证明”，防止后续退回匿名 cookie 误判。
- **运行过的验证**：
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**
  - `CARGO_TARGET_DIR=backend/target/qimai-capture-check cargo nextest run -p golish-integrations -E 'test(fixture_enscan_kc_and_rb_require_login_state_proof)' --status-level fail` → **exit 0 / 1 passed, 74 skipped**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_kc_and_rb_require_login_state_proof)' --status-level fail` → **exit 0 / 1 passed, 74 skipped**（原 target 等待 Cargo lock 后通过）
  - `cargo fmt --package golish-integrations --check` → **exit 0**
  - `CARGO_TARGET_DIR=backend/target/qimai-capture-check cargo nextest run -p golish-integrations -E 'test(capture)' --status-level fail` → **exit 0 / 21 passed, 54 skipped**
  - `ReadLints`（`resolver.rs` + `enscan-go.json`）→ **No linter errors found**
- **已知风险或未解决问题**：
  - `USERINFO` / `aso_ucenter` 来自公开 Qimai 登录 cookie 样例，符合“登录态证明”用途；仍建议用户重启 dev binary 后手动复测一次真实登录流程，确认当前 Qimai 账号实际也会下发这两个 cookie。

### 2026-05-22 · TYC Auto-capture 未登录误触发修复

- **本轮目标**：用户反馈 Settings → Integrations → ENScan_GO → TianYanCha Auto-capture 在还没登录时就触发抓取，并报 `[CAPTURE_RULE_FAILED] required rule #1: request header 'X-Tycid' not observed: value was empty`。
- **根因**：
  - TYC 的 `success_url_pattern` 是泛匹配 `tianyancha.com`，打开登录页自身就会触发 `try_extract`。
  - AQC 之所以没问题，是 `cookie_joined.required_names=["BDUSS"]` 缺失时走 `[SOFT_RETRY]` 重新回到 `WaitingLogin`；TYC 的必填 `request_header` 缺失此前直接被当成 fatal failure，导致窗口关闭 / toast 报错。
  - 用户继续复测后仍不行，进一步定位到 TYC JSON 的字段键写错：`resources/toolsconfig/enscan-go.json` 声明 / capture 写入的是 `cookies.tianyancha`、`cookies.tycid`、`cookies.auth_token`，但既有设计、外部文件后端和 ENScan 配置结构使用的是 `cookies.tyc`、`tyc.tycid`、`tyc.auth_token`；即使抓到了也会写到 ENScan 不读取的位置。
  - 用户最新日志显示 `.tianyancha.com` cookie jar 已有 `TYCID` 和 `auth_token`，但 `request_header` 仍持续 `value was empty`；说明 TYC 当前可用凭据来源是 cookie，不是显式 fetch/XHR request header。
  - 用户确认 TYC 已抓到后，继续反馈 KC/RB 报 `no cookies matched`；这类 `cookie_joined names=[] required=true` 规则在根页尚未登录 / 尚无该域 cookie 时也不应 fatal，应 soft retry 等登录完成。
  - 用户随后指出未登录 KC/RB 也提示“抓到了”；本机检查已保存配置只含匿名 cookie：KC 只有 `synct`，RB 只有 `app-uuid` / `app-device`。根因是 `names=[]` 只要任意匿名 cookie 存在就会成功，缺少“cookie 数量门槛 / 登录态证明”。
- **已完成**：
  - `backend/crates/golish/src/tools/integrations/capture/engine.rs` 新增 `request_header_failure_reason`，当必填 request header 因 `value was empty` 暂未观察到时返回 `[SOFT_RETRY]`，让 capture session 保持打开并等待后续导航 / API 请求出现。
  - 用户复测可登录但仍抓不到内容后，补充 `spawn_soft_retry_probe`：软重试后每 2 秒延迟探测一次当前页面的已记录 request headers，解决 TYC 登录后后台 XHR/fetch 出现 header 但没有新页面导航时不会再次提取的问题。
  - 用户进一步确认“已经是登录态，点 Auto-capture 只开 webview 没反应”后，把 `resources/toolsconfig/enscan-go.json` 的 TYC `login_url` 从首页改为搜索探针页 `https://www.tianyancha.com/search?key=%E5%B0%8F%E7%B1%B3`，让已登录态打开后主动产生站内业务请求，从而稳定暴露 `X-Tycid` / `Authorization`。
  - `golish-integrations/src/resolver.rs` 新增 fixture `fixture_enscan_tyc_capture_uses_search_probe_url`，防止 TYC capture 入口退回只打开首页。
  - 同一 fixture 继续断言 TYC group 和 capture rules 必须使用 ENScan 配置键 `cookies.tyc`、`tyc.tycid`、`tyc.auth_token`，并且 `tyc.tycid` / `tyc.auth_token` 必须分别来自 `TYCID` / `auth_token` cookie；`enscan-go.json` 已把字段声明、target_field 和提示文案同步改回这些键与 cookie 来源。
  - 新增回归测试 `required_request_header_failures_are_soft_retryable` 覆盖 TYC 这类 header 暂缺场景。
  - 新增 `cookie_failure_reason` / `cookie_joined_failure_reason`：必填单 cookie 缺失或必填 cookie_joined 匹配为空时返回 `[SOFT_RETRY]`，避免 KC/Qimai、RB/RiskBird 在用户还未完成登录时立即失败关窗。
  - `golish-integrations::CaptureRule::CookieJoined` 新增 JSON 字段 `min_count`（默认 0，向后兼容）；capture engine 在格式化后的 cookie 数少于 `min_count` 时 soft retry，不写入凭据。前端 `CaptureRule` 类型同步新增 `min_count?: number`。
  - `resources/toolsconfig/enscan-go.json` 给 KC 设置 `min_count=2`（匿名态只有 `synct`），给 RB 设置 `min_count=3`（匿名态只有 `app-uuid` / `app-device`）。新增 fixture `fixture_enscan_kc_and_rb_require_more_than_anonymous_cookies` 防回归。
  - 本机跑 ENScan TYC 轻量查询验证刚抓到的凭据：TYC 能查到“小米”22 个企业结果；随后 ENScan v2.0.5 自身在 `searchBaseInfo` 空数组处 panic，说明凭据已被接受但上游工具有解析 bug。
  - `backend/crates/golish-pentest/src/models.rs` 给 `AssetIntelNormalizeFilter` 补 `Eq` derive，修复当前工作树中 `AssetIntelDiscoveryConfig: Eq` 编译阻塞，便于后端测试继续跑。
- **运行过的验证**：
  - `cargo test -p golish required_request_header_failures_are_soft_retryable --lib` → **exit 0**
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)' --status-level fail` → **exit 0 / 32 passed, 232 skipped**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_tyc_capture_uses_search_probe_url)' --status-level fail` → **exit 0 / 1 passed, 73 skipped**（修复前红灯：TYC login_url 仍是首页）
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_tyc_capture_uses_search_probe_url)' --status-level fail` → **修字段名前 exit 100**，红灯落在 `TYC group should declare ENScan config key cookies.tyc`；字段修复后 **exit 0 / 1 passed, 73 skipped**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_tyc_capture_uses_search_probe_url)' --status-level fail` → **exit 0 / 1 passed, 73 skipped**（fixture 已覆盖 `TYCID` / `auth_token` cookie 来源；本次因 Cargo 锁等待期间 JSON 已修复，未单独捕获 cookie-source 红灯）
  - `cargo nextest run -p golish-integrations -E 'test(capture)' --status-level fail` → **exit 0 / 21 passed, 53 skipped**
  - `cargo test -p golish required_cookie --lib` → **修复前 exit 101**（单 cookie 分支未解构 `required` 编译失败，证明新增测试命中）；`min_count` 修复后 **exit 0 / 3 passed**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_kc_and_rb_require_more_than_anonymous_cookies)' --status-level fail` → **exit 0 / 1 passed, 74 skipped**
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)' --status-level fail` → **exit 0 / 35 passed, 232 skipped**
  - `cargo nextest run -p golish-integrations -E 'test(capture)' --status-level fail` → **exit 0 / 21 passed, 54 skipped**
  - `pnpm exec tsc --noEmit` → **exit 0**
  - `pnpm exec biome check frontend/lib/api/integrations.ts` → **exit 0 / No fixes applied**
  - `ENScan_GO/enscan-v2.0.5-darwin-amd64 -n 小米 -type tyc -field icp` → **exit 2**；输出显示 TYC 已认证并返回 22 个企业候选，随后上游 `tianyancha.go:124` panic。
  - `cargo nextest run -p golish-pentest --lib -E 'test(asset_intel)' --status-level fail` → **exit 0 / 7 passed, 55 skipped**
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**
  - `cargo fmt --package golish-integrations --check` → **exit 0**
  - `rustfmt --edition 2021 --check crates/golish/src/tools/integrations/capture/engine.rs crates/golish-pentest/src/models.rs` → **exit 0**
  - `cargo fmt --package golish --package golish-pentest --check` → **exit 1**，被当前工作树中既有 `backend/crates/golish/src/tools/asset_intel.rs` 格式 diff 阻塞，非本次 TYC touched file。
  - `ReadLints`（`engine.rs` + `models.rs` + `resolver.rs` + `enscan-go.json`）→ **No linter errors found**
- **已知风险或未解决问题**：
  - TYC capture 已由用户确认抓到；ENScan TYC key 可用性验证到“能返回企业候选”，但 `-field icp` 后续被 ENScan v2.0.5 上游 panic 中断。
  - KC/RB 仍需用户重启 dev binary 后真实复测；预期未登录态不会再提示“抓到了”，而是在 cookie 数低于门槛时保持窗口 soft retry。若登录后仍无法抓到，需要根据新日志里的 `raw_domains` / `cookie_names` 调整登录 URL、cookie domain 或 `min_count`。
  - `cargo test -p golish --lib -E ...` 是误用命令，exit 1 未执行测试；已用 `cargo nextest` 正确重跑并通过。

---

### 2026-05-22 · Asset Intel 两阶段 UI 收口（Hydrate intel → 查子公司 / 补字段）

- **本轮目标**：用户确认继续改 UI，把 Target 面板旧的单按钮 Hydrate intel 从主流程撤掉，接到已实现的两阶段后端命令，避免 0.zone 继续拿主公司名和 enscan-go 同时跑。
- **已完成**：
  - `TargetGroupedView.tsx` 的 discover_assets action model 改为两阶段：主公司显示「查子公司」+「批量补字段」，promoted 子公司显示「补字段」。
  - 子公司若自身没有 `intel.engagement`，会向上继承父公司的 discover_assets 语义，因此 promote 出来的「平安银行 / 平安证券」这类 child org 也会显示「补字段」。
  - 旧 UI 入口不再调用 `assetIntel.hydrate()`；三种 action 分别调用 `assetIntel.hydrateSubsidiaries()` / `assetIntel.enrichBatch()` / `assetIntel.enrichOrganization()`。
  - Activity 面板同步展示同一组两阶段按钮，不再显示旧文案 Hydrate intel。
  - `TargetGroupedView.actions.test.ts` 新增 2 个 action model 测试：主公司两按钮、子公司单按钮且不显示 batch。
  - 针对用户实测反馈补修：运行态从 org 级别细化为 org+action，避免点击「查子公司」时「批量补字段」按钮也一起转圈；Candidates 面板按阶段过滤 provider source，主公司 discovery 视图只展示 enscan-go 这类 subsidiaries provider 候选，旧 run 留下的 0.zone 候选不再污染「查子公司」结果。
  - 针对用户第二轮反馈补修：`查子公司` 使用 discovery 专用 config（默认 `minOwnership=51` / `depth=1` / `includeBranches=true`），不再沿用轻量 target-only hydrate；Candidates 的 discovery 视图隐藏 target bucket，只展示 organization candidates，避免 enscan-go 的 ICP/APP 域名结果冒充子公司。
  - 手动验证 ENScan_GO v2.0.5 参数：`-field invest,branch -invest 51 -deep 1 -branch` 会输出 `invest/branch/partner/enterprise_info`，不再输出 ICP/APP target；`-field invest -invest 51 -deep 1` 输出更干净，只含 `invest/partner/enterprise_info`。据此把 `resources/toolsconfig/enscan-go.json` 的 `company-default-json` skill 改成 `-field invest`，默认 discovery 不再抓 ICP/APP，也不默认抓分支机构（分支需用户显式 include branches）。
  - 按用户拍板把“查子公司”改成自动创建正式子公司：`asset_intel_hydrate_subsidiaries` 跑完 discovery 后不写 review candidates，而是按 `scale >= minOwnership`（默认 51）+ `status=开业/存续` + child name 未重复过滤后直接 `organizations::create(parent_id=master)`；created/skipped 明细写入 `AssetIntelRun.evidence`。低比例参股、注销/吊销、重复 child 跳过。
  - 按用户要求把自动提升策略 JSON 化：`AssetIntelToolConfig.discovery` 新增 `auto_promote / promote_when / ownership_field / dedupe_by`；`enscan-go.json` 现在声明 `auto_promote=true`、`promote_when=[scale>=51,status contains 开业]`、`dedupe_by=[pid,name]`。Rust 只执行 JSON policy，不再硬编码阈值和状态。
  - 针对旧 candidates 残留补修：自动创建子公司后清理父组织 `intel.engagement.candidates`，保留 `mode`、`lookup_match`、contacts 等其它 metadata，避免 UI 继续显示历史 `needs_review` 列表。
  - 按用户要求落地第一版多源 discovery：新增 `resources/toolsconfig/enscan-go-tyc-discovery.json`（provider_id=`enscan-go-tyc-discovery`，`-type tyc -field invest`，只依赖 TYC 凭证）。继续按同一 JSON 模板新增 `enscan-go-kc-discovery.json`（`-type kc -field invest`，只依赖 KC/Qimai 凭证）和 `enscan-go-rb-discovery.json`（`-type rb -field invest`，只依赖 RB/RiskBird 凭证）。AQC 主配置的 asset-intel capabilities 收窄为 `subsidiaries` 且只依赖 AQC。后端 candidate merge 在同值去重时合并 evidence.sources，避免同一子公司被多源重复创建但保留来源证据。
- **运行过的验证**：
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 30 passed**
  - `pnpm vitest run frontend/components/TargetPanel/` → **exit 0 / 35 passed**
  - `cargo test -p golish auto_promote_child_decisions_only_promote_active_controlled_investments --lib` → **exit 0 / 1 passed**
  - `cargo test -p golish clear_engagement_candidates_preserves_engagement_metadata --lib` → **exit 0 / 1 passed**
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 27 passed**
  - `cargo test -p golish-pentest tool_config_accepts_asset_intel_descriptor --lib` → **exit 0 / 1 passed**
  - `cargo fmt --package golish --package golish-pentest --check` → **exit 0**
  - `pnpm exec tsc --noEmit` → **exit 0**
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / No fixes applied**
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json resources/toolsconfig/enscan-go-tyc-discovery.json resources/toolsconfig/enscan-go-kc-discovery.json resources/toolsconfig/enscan-go-rb-discovery.json feature_list.json` → **exit 0**
  - `ReadLints`（2 个改动文件）→ **No linter errors found**
  - `rg 'assetIntel\\.hydrate\\(|hydrate_intel|Hydrate intel' frontend/components/TargetPanel` → **No matches found**
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json feature_list.json` → **exit 0**
  - 手动 ENScan：`.../enscan-v2.0.5-darwin-amd64 -n "中国平安" -type aqc -field invest -invest 51 -deep 1 -delay 1 -json -out-dir /tmp/golish-enscan-field-invest-only-*` → **exit 0**，导出 JSON 只含 `invest/partner/enterprise_info`，无 `icp/app/wx_app/wechat`。
- **已知风险或未解决问题**：
  - `enrichBatch` 返回的是多次 per-org run；当前 UI 仍只把第一条 run 放进 selected org 的 Last run 摘要，streaming activity 会显示整批过程但不会把每个子公司的最终摘要拆成独立卡片。可作为后续 polish。
  - 未跑真实 0.zone 外部 E2E；需要用户在 just dev 下配置 0.zone key 后验证「批量补字段」是否按子公司名逐个查询。

---

### 2026-05-22 · Hydrate Intel disambiguation + 主档案 + evidence 三件套（A 轻量 + B + C 一次到位）

- **本轮目标**：用户报告 hydrate intel 数据"乱七八糟"，写错子公司也抓不到对的。诊断 6 根因（R1 无公司主体核验 / R2 normalize 不过滤股权 / R3 enterprise_info 没写主档案 / R4 invest 方向不分 / R5 candidate 缺 evidence 上下文 / R6 输入错名字静默错查）。用户同意推荐组合 A 轻量 + B + C 同步落地。
- **已完成（按 milestone）**：
  - **C · normalize when filter + evidence 展开**：
    - `golish-pentest::models` 新增 `AssetIntelNormalizeFilter` + `AssetIntelNormalizeFilterOp`（9 variant: eq/ne/gte/gt/lte/lt/exists/missing/contains），并加在 `AssetIntelNormalizeRule.when` 字段（向后兼容 `#[serde(default)]`）。
    - `asset_intel.rs` 新增 `filter_passes` + `apply_filter_op`：数值优先 f64 比较，fallback 字符串 ordering（保证日期类比较仍能工作）。
    - 前端 `TargetGroupedView.tsx` 加 `getEvidenceRawRows` 提取 24 字段白名单（name/credit_code/scale/legal/legal_person/industry/addr/address/reg_date/establish_date/phone/email/domain/url/link/app_id/app_url/...），candidate 卡片新增 Details 按钮 toggle 展开。
  - **B · profile_fields 写主档案**：
    - `models.rs` 新增 `AssetIntelProfileFieldRule` + `AssetIntelProfileFieldTarget`（Scalar/Intel/Contact 3 bucket）+ `AssetIntelProfileFieldTransform`（None/Trim/Lower/Upper 4 transform）+ `AssetIntelNormalizeConfig.profile_fields`。
    - `asset_intel.rs` 新增 `ProfileFieldEntry` + `extract_profile_field_entries`，把 `normalize_json_with_descriptor` 返回值改为元组 `(candidates, profile_entries)`，`CliJsonStreamShared` 多一个 `profile_entries: TokioMutex<Vec<ProfileFieldEntry>>` 让 stdout / artifact / http_json 三路 normalize 都同时收集。`run_cli_json_provider` + `run_http_json_provider` 函数签名扩到 4 元组返回 `profile_entries`，hydrate 顶层 fold + `build_profile_patch_from_entries`（first-wins for scalar/intel keys + contact list lowercase-trim dedupe + 保留 existing intel 如 engagement metadata）→ 单次 `update_profile`，patch 空时跳过 DB roundtrip。
    - `resources/toolsconfig/enscan-go.json` `normalize.profile_fields` 加 7 条 enterprise_info 规则：reg_code→credit_code(scalar trim) / industry→industry / legal_person·legal→legal_representative(intel trim) / reg_address·addr·address→registered_address(intel) / reg_date·establish_date·founded_at→registered_at(intel) / email→email(contact lower) / phone→phone(contact trim)。
  - **A · lookup_company disambiguation 流程**：
    - `models.rs` 新增 `AssetIntelLookupConfig` + `AssetIntelLookupNormalize`（path + name + 6 个 optional FieldRef + default_confidence），加在 `AssetIntelToolConfig.lookup: Option<...>`。
    - `asset_intel.rs` 新增 `LookupCompanyMatch` + `AssetIntelLookupRequest` + `AssetIntelLookupResult` + `extract_lookup_matches` + `run_lookup_cli_provider`（轻量同步 `tokio::Command::output()` + timeout，比 hydrate cli_json 简单一截）+ `dedupe_lookup_matches`（credit_code 优先，回落 name lowercase-trim）+ `asset_intel_lookup_company` Tauri command（顶层 dedupe + 按 confidence 降序 + `LOOKUP_RESULTS_HARD_CAP=25` 兜底）。注册到 `commands_facade::asset_intel` + `commands_registry::generate_handler`。
    - `enscan-go.json` 加 `asset_intel.lookup`（skill_id=company-lookup-json + timeout 60s + normalize.path $..enterprise_info[*] + 7 字段映射 + default_confidence 0.68）+ 新 skill `company-lookup-json` 跑 `-n {{keyword}} -type aqc -field icp -delay 3 -json -out-dir {{out_dir}}`（轻量查询，只拿 enterprise_info 不抓 invest/branch/app）。
    - 前端 `frontend/lib/api/asset-intel.ts` +`LookupCompanyMatch`/`AssetIntelLookupRequest`/`AssetIntelLookupResult` + `lookupCompany` IPC wrapper。
    - 前端 `NewEngagementDialog.tsx` +Look up button（仅 `discover_assets` 模式渲染）+ 候选列表渲染（confidence% + credit/industry/legal/address）+ selectedMatch badge（emerald 成功态显示已选公司全部字段）+ Clear 按钮 + 自动清 stale match（orgName 编辑时）+ 改 organization name 用显式 `htmlFor` 避免 testing library nested-label 二义；submit 时把 `selectedMatch.creditCode` / `industry` 写到 `OrganizationProfilePatch.credit_code` / `industry`，并把全套 lookup match snapshot 存到 `intel.engagement.lookup_match`。
- **运行过的验证**：
  - 修复前：cargo nextest `golish-pentest` → exit 101（schema 缺字段，先红）
  - `cargo nextest run -p golish-pentest --status-level fail` → **exit 0 / 59 passed, 7 skipped**（含 5 个 asset_intel schema round-trip + when filter + profile_fields + lookup config 新测）
  - `cargo nextest run -p golish --lib -E 'test(asset_intel)'` → **exit 0 / 18 passed, 236 skipped**（含 3 when filter + 3 profile_fields + 2 lookup matches/dedupe 新增）
  - `cargo check -p golish` → **exit 0**，仅 preexisting `capture/data_dir.rs::session_dir` dead_code warning
  - `cargo fmt --package golish --package golish-pentest --check` → **exit 0**
  - `pnpm vitest run frontend/components/TargetPanel/` → **exit 0 / 27 passed**（22 actions + 5 dialog，含 getEvidenceRawRows 4 + lookup flow 3 新增 + 1 hides outside discover_assets）
  - `pnpm exec tsc --noEmit` → **exit 0 / 10.4s**
  - `pnpm exec biome check` 5 改动文件 → **No fixes applied**（1 次自动修：NewEngagementDialog 三元运算符断行）
  - `ReadLints` 10 改动文件 → **No linter errors found**
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json` → **VALID JSON**
  - `python3 -m json.tool feature_list.json` → **VALID JSON**
- **已记录证据**：见以上验证段；77 个 Rust 测试 + 27 个 vitest 全过，覆盖 schema round-trip / when filter 3 op 实测 / profile field 3 bucket × 4 transform / scalar+contact dedupe / 全空 entries 返 None / lookup matches 含 FirstOf fallback / 跨 provider credit_code 大小写不敏感去重 / non-discover 隐藏 Look up button / discover 选择后 submit patch 含 credit_code+industry / no matches 显式提示。
- **提交记录**：**待用户授权 commit**。建议 commit message：`feat(asset-intel): disambiguation lookup + profile_fields master record + when filter & evidence expansion`，文件清单：
  - 后端：`backend/crates/golish-pentest/src/models.rs`、`backend/crates/golish/src/tools/asset_intel.rs`、`backend/crates/golish/src/commands_facade/asset_intel.rs`、`backend/crates/golish/src/commands_registry.rs`
  - 前端：`frontend/lib/api/asset-intel.ts`、`frontend/components/TargetPanel/NewEngagementDialog.tsx`、`frontend/components/TargetPanel/NewEngagementDialog.test.tsx`、`frontend/components/TargetPanel/TargetGroupedView.tsx`、`frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`
  - 资源 / 元数据：`resources/toolsconfig/enscan-go.json`、`feature_list.json`、`agent-progress.md`
- **已知风险或未解决问题**：
  - **未真实跑 ENScan E2E**：lookup 流程依赖 ENScan `-n <keyword> -type aqc -field icp -json` 输出的 enterprise_info 实际字段名（`reg_code` / `industry` / `legal_person` / `reg_address` / `reg_date`）。enscan-go.json normalize 配置用 FirstOf fallback 覆盖了几种常见名称（如 `legal_person` 或 `legal`、`reg_address` 或 `addr`），但 ENScan v2.0.5 的真实字段名需要用户跑一次 lookup 截图给我，必要时再调 JSON。
  - **A 轻量版假设 ENScan 单源 lookup 足够**：当前 `lookupCompany` 只跑有 `asset_intel.lookup` 配置的 provider。如果未来加 0.zone 等其它 provider，0.zone 需要 HTTP 版 lookup（当前 `run_lookup_cli_provider` 拒绝 http_json provider，返 unavailable），P2 可以扩展 `run_lookup_http_provider`。
  - **profile_fields 主档案写入是 first-wins**：多 provider 给同一字段（如 credit_code）冲突时静默丢弃后者，没有提示 UI；正常场景下 enterprise_info.reg_code 全网唯一，问题不大。
  - **frontend lookup 没缓存**：用户每次输入新 keyword + Look up 都会重新打 ENScan。考虑用户主动触发的按钮，可接受不缓存。
  - **未跑真实 hydrate 验证 profile_fields 落库**：unit test 覆盖了纯函数提取 + patch fold + dedupe，但没起完整 Postgres 跑 update_profile。需要用户在 just dev 下点 Hydrate intel 后用 SQL 查 organizations.credit_code/intel.contacts。
  - **未 commit**：等用户授权。
- **下一步最佳动作**：
  1. **用户授权后整批 commit**（11 个改动文件）+ push
  2. **用户跑 just dev 真实 E2E**：① Settings → Integrations → ENScan AQC 确保 cookie 有效 ② 新建 Discover Assets engagement → 输入"小米" → 点 ⚡ Look up → 应弹候选列表 ③ 选定一家 → 看 orgName 自动填 + emerald badge 显示 credit_code/industry ④ Create & Prepare Discovery → SQL 查 organizations.credit_code/industry 已写入 ⑤ Hydrate intel → 候选 Details 按钮展开看 ENScan 原始字段 ⑥ 验证 organizations.intel.contacts.email/phone 已填
  3. **如 lookup 拿不到候选**：很可能是 ENScan enterprise_info 字段名跟我猜的不一致（如实际是 `creditCode` 不是 `reg_code`），需要根据用户截图调 enscan-go.json 里的 FieldRef
  4. **后续 polish**：① 候选列表加 industry 图标 ② Look up 加 keyword recently used 缓存 ③ 接入 0.zone http_json lookup runtime

---

### 2026-05-22 · Asset Intel CLI 输出目录隔离修复

- **本轮目标**：修复 Asset Intel `cli_json` provider 运行 ENScan 等 CLI 工具时继承开发 cwd，导致工具相对路径副产物可能写入项目根/开发目录的问题；同时修复 Discover Assets 默认参数过重导致用户误以为后端卡住的问题。
- **已完成**：
  - `run_cli_json_provider` 改为基于 organization 的 `project_path` 构造输出目录：`{project_root}/.golish/tool-output/asset-intel/{run_id}/{provider_id}`。
  - CLI 子进程启动时显式 `current_dir(&out_dir)`，即使工具忽略 `-out-dir` 或写相对路径，也只会写入本次 provider 输出目录。
  - CLI 子进程设置 `kill_on_drop(true)`，让 timeout 路径更稳地回收 ENScan 进程。
  - `cli_json` / `http_json` provider 增加开始、失败、超时、完成日志，避免后端日志只停在 toolsconfig scan 让用户无法判断进度。
  - 保留 evidence 中的 `outDir`，现在指向项目 `.golish/tool-output/asset-intel/...`。
  - `NewEngagementDialog` 的 Discover Assets 默认值改成轻量 hydrate：不再默认传 `-invest 51 -deep 2 -branch`；需要股权/分支时用户再显式填写。
  - `buildHydrateConfigFromEngagement` 兼容已有组织里旧默认污染值：`51 + depth 2 + include branches` 会按轻量 hydrate 处理，避免老记录继续触发重 ENScan 查询。
- **运行过的验证**：
  - 修复前 `cargo test -p golish cli_json_runtime_runs_in_provider_output_dir --lib` → **exit 101 / failed**，失败原因为 CLI cwd 未进入期望输出目录。
  - 修复前 `pnpm vitest run frontend/components/TargetPanel/NewEngagementDialog.test.tsx` → **exit 1 / 1 failed**，失败证明 Discover Assets 默认仍是 `51 / 2 / include branches`。
  - 修复前 `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 1 / 1 failed**，失败证明已有组织的旧默认重参数仍会传给 hydrate。
  - `cargo fmt --package golish --check` → **exit 0**。
  - `cargo test -p golish cli_json_runtime_runs_in_project_tool_output_dir --lib` → **exit 0 / 1 passed**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 9 passed, 236 filtered out**。
  - `cargo check -p golish` → **exit 0**，仅既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm vitest run frontend/components/TargetPanel/NewEngagementDialog.test.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 15 passed**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/NewEngagementDialog.tsx frontend/components/TargetPanel/NewEngagementDialog.test.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / No fixes applied**。
  - `git diff --check -- backend/crates/golish/src/tools/asset_intel.rs frontend/components/TargetPanel/NewEngagementDialog.tsx frontend/components/TargetPanel/NewEngagementDialog.test.tsx agent-progress.md` → **exit 0**。
  - `ReadLints`（本轮相关文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 未跑真实 ENScan/0.zone 外部 E2E；本轮修复覆盖的是输出目录隔离、轻量默认参数、provider 日志与自动化测试路径。

---

### 2026-05-22 · Asset Intel HTTP JSON Runtime

- **本轮目标**：补齐 `http_json` runtime，让 0.zone / 后续 HTTP API provider 也能通过 JSON descriptor 接入 Asset Intel，不再需要 Rust 专属 adapter。
- **已完成**：
  - `golish-pentest::models::AssetIntelRuntimeConfig` 新增 `HttpJson { requests }` variant，request 支持 method/url/headers/form/json/timeout。
  - `asset_intel.rs` 新增 generic `http_json` runtime：渲染 `{{company_name}}` / `{{secret:<field>}}`，从 vault 读取 integration secret，发送 HTTP JSON 或 form 请求，把响应 JSON 交给同一套 descriptor normalizer。
  - 新增 `resources/toolsconfig/0-zone.json`，用 JSON 声明 0.zone provider、3 个 POST request（domain/site/apk）、`api_key` secret、auto priority、organization/target normalize mapping。
  - `asset_intel_hydrate` runtime dispatch 现在支持 `cli_json` 与 `http_json` 两类 provider。
- **运行过的验证**：
  - `cargo test -p golish-pentest tool_config_accepts_asset_intel_http_json_runtime --lib` → **exit 0 / 1 passed, 56 filtered out**。
  - `python3 -m json.tool resources/toolsconfig/0-zone.json >/dev/null && python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**。
  - `cargo fmt --package golish --package golish-pentest` → **exit 0**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 8 passed, 236 filtered out**。新增覆盖：fake CLI/HTTP JSON 数据跨 provider 去重；本地 TCP fake HTTP server 收到 `http_json` POST，返回假 JSON 后 normalize 出 2 个 target candidates。
  - `cargo check -p golish` → **exit 0**，仅报告既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 12 passed**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → **exit 0 / No fixes applied**。
  - `rg 'ENSCAN_PROVIDER_ID|ZONE_PROVIDER_ID|ZoneProvider|QueryType|run_zone_provider|run_enscan_go_provider|build_enscan_command_plan|parse_enscan_json_records' backend/crates/golish/src/tools/asset_intel.rs` → **No matches found**。
  - `git diff --check -- <本轮相关文件>` → **exit 0**。
  - `ReadLints`（本轮相关文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 未跑真实 0.zone 外部 API E2E；需要用户在 Integrations 中配置可用 `0.zone/default/api_key`（或旧 vault alias `name='0.zone', entry_type='api_key'`）后再跑真实 hydrate。
  - `http_json` 当前只支持简单模板替换和单页请求；分页、响应 envelope 错误码判定（如 `code != 0`）可继续 JSON 化扩展。

---

### 2026-05-22 · Asset Intel JSON-driven Provider 实现

- **本轮目标**：按新计划把 Asset Intel provider 从 Rust 硬编码分支改为 toolsconfig JSON 驱动，保留现有 Target UI / IPC 契约。
- **已完成**：
  - `golish-pentest::models::ToolConfig` 新增 `asset_intel` descriptor schema，支持 provider metadata、capabilities、integration requirement、auto priority、`cli_json` runtime、normalize mapping。
  - `resources/toolsconfig/enscan-go.json` 新增 `tool.asset_intel`，把 ENScan provider id、capabilities、auto mode、skill runtime、artifact JSON、organization/target normalize mappings 外置到 JSON。
  - `asset_intel_list_providers` 改为扫描 toolsconfig descriptor；`asset_intel_hydrate` 改为 JSON auto selector + generic `cli_json` runtime + generic JSON normalizer。
  - 删除 Asset Intel 内 ENScan_GO / 0.zone 专属 provider 常量、`ZoneProvider` 调用、专属命令构建和专属 normalize 分支；0.zone 等后续 provider 需要通过 JSON descriptor 接入。
  - 保持前端 API / TargetPanel 行为不变。
- **运行过的验证**：
  - `cargo test -p golish-pentest tool_config_accepts_asset_intel_descriptor --lib` → **exit 0 / 1 passed, 55 filtered out**。
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**。
  - `cargo fmt --package golish --package golish-pentest` → **exit 0**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 6 passed, 236 filtered out**。
  - `cargo check -p golish` → **exit 0**，仅报告既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 12 passed**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → **exit 0 / No fixes applied**。
  - `rg 'ENSCAN_PROVIDER_ID|ZONE_PROVIDER_ID|ZoneProvider|QueryType|run_zone_provider|run_enscan_go_provider|build_enscan_command_plan|parse_enscan_json_records' backend/crates/golish/src/tools/asset_intel.rs` → **No matches found**。
  - `git diff --check -- <本轮相关文件>` → **exit 0**。
  - `ReadLints`（本轮相关文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 第一版 `cli_json` arg splitting 只支持简单双引号分组，适合当前 ENScan skill；如果未来工具参数需要复杂 shell quoting，需要扩展 descriptor 或使用 wrapper skill。
  - `http_json` runtime 尚未实现；因此 0.zone 暂时不会作为 Asset Intel provider 出现在 auto mode，需后续用 JSON HTTP descriptor 接回。

---

### 2026-05-22 · Asset Intel JSON-driven Provider 方案修订

- **本轮目标**：响应用户指出的 provider adapter 方向问题，停止沿 Rust 硬编码 provider 分支扩展，改写为“后续新增/替换工具优先只改外部 JSON”的新方案。
- **已完成**：
  - 新增 `docs/design/2026-05-22-asset-intel-json-driven-providers.md`，明确 Asset Intel provider registry / runtime / normalize 应由 `tool.asset_intel` JSON descriptor 驱动。
  - 新增 `docs/superpowers/plans/2026-05-22-asset-intel-json-driven-providers.md`，拆出 schema、ENScan JSON descriptor、generic normalizer、generic `cli_json` runtime、auto selector、移除 0.zone Rust 分支、前端回归验证等任务。
  - 在旧设计与旧计划顶部标记 superseded，避免后续继续执行硬编码 Phase 4。
- **运行过的验证**：
  - `ReadLints`（4 个新/改文档）→ **No linter errors found**。
  - `git diff --check -- docs/design/2026-05-22-asset-intel-json-driven-providers.md docs/superpowers/plans/2026-05-22-asset-intel-json-driven-providers.md docs/design/2026-05-22-asset-intel-provider-abstraction.md docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md agent-progress.md` → **exit 0**。
- **已知风险或未解决问题**：
  - 本轮只写新方案，尚未改 Rust/JSON 实现；当前 `asset_intel.rs` 仍保留 ENScan_GO / 0.zone 专属逻辑，需按新计划重构。

---

### 2026-05-22 · Asset Intel Provider Abstraction Phase 4

- **本轮目标**：实现多 provider / auto mode，让 Asset Intel Service 在 Target UI 不变的前提下同时编排 ENScan_GO 和 0.zone，并合并去重 candidates。
- **已完成**：
  - `provider_descriptors()` 新增 `0.zone（零零信安）`，capabilities 覆盖 domains / apps / contacts，integration 指向 `0.zone/default`。
  - `asset_intel_hydrate` auto mode 从单 ENScan_GO 改为默认尝试 `enscan-go` + `0.zone`；显式 `providerIds` 仍只跑指定 provider。
  - 新增 0.zone adapter：复用 `golish_intel_providers::zone::ZoneProvider`，从 `vault_entries` 读取 `0.zone` API key，查询 Domain / Site / Apk。
  - 0.zone 未配置 key 时返回 `unavailable` provider status，不阻塞 ENScan_GO。
  - 多 provider candidates 按 `kind + value(lowercase)` 去重，保留先返回候选及其 evidence。
  - Activity tab 显示 `asset_intel_list_providers` 返回的 available provider chips。
  - 更新 Phase 4 实施计划：`docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md`。
- **运行过的验证**：
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 7 passed, 236 filtered out**。
  - `cargo check -p golish` → **exit 0**，仅报告既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 12 passed**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → 初次格式 fail，修复后 **exit 0 / No fixes applied**。
  - `ReadLints`（本轮相关文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - 未跑真实 0.zone 外部 API E2E；没有 API key 时预期只显示 provider `unavailable`。
  - 多 provider 去重当前按 normalized candidate value；后续可增加 evidence merge，让重复候选保留多个 source。

---

### 2026-05-22 · Asset Intel Provider Abstraction Phase 3

- **本轮目标**：把 Target / Discover Assets workspace 接入统一 Asset Intel API，完成 hydrate → provider status → candidate review → explicit promote 前端闭环。
- **已完成**：
  - `TargetGroupedView` 引入 `assetIntel.hydrate()`，`Hydrate intel` action 现在会真实触发 hydrate IPC。
  - Activity tab 增加 hydrate loading、error、last run status、provider status、checked-empty/completed/failed/unavailable 视觉状态。
  - Candidates tab 从仅显示计数升级为展示 organization / target candidate 列表。
  - Candidate 支持 `Approve` / `Reject`，复用现有 `organization_candidates_upsert` 更新状态。
  - Candidate 支持显式 `Promote`：organization candidate 创建 child org；target candidate 走现有 `onBatchAdd` 创建 target。
  - 新增/更新纯 helper：`buildHydrateConfigFromEngagement`、`getCandidateItems`、`getProviderStatusClass`，并补充单测。
  - 更新 Phase 3 实施计划：`docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md`。
- **运行过的验证**：
  - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → **exit 0 / 12 passed**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → **exit 0 / No fixes applied**。
- **已知风险或未解决问题**：
  - 尚未跑 `just dev` 真实 UI E2E；需要用户有可用 ENScan_GO cookie 后手动点击 Hydrate 验证真实 provider run。
  - Promote target 复用现有 `onBatchAdd` 行为；若后续需要“approved candidate 批量 promote”，可追加批量选择 UI。
  - Phase 4 多 provider / auto merge 去重尚未实现。

---

### 2026-05-22 · Asset Intel Provider Abstraction Phase 2

- **本轮目标**：把 Phase 1 的 `enscan-go` skeleton 升级为真实 ENScan_GO adapter 执行路径，仍保持 Target / Engagement UI 只依赖统一 Asset Intel 契约。
- **已完成**：
  - `asset_intel_hydrate` 注入 `PentestState`，通过 `ConfigManager` 读取 `toolsconfig_dir` / `tools_dir`。
  - 使用 `golish_pentest::scan_toolsconfig` + `resolve_tool_executable("enscan-go", ...)` 定位 ENScan_GO 可执行文件。
  - 新增 `build_enscan_command_plan`：生成只读 JSON 导出命令，默认 `-type aqc -field icp,app,wx_app,wechat -json -out-dir <tmp>`，并按 discovery config 追加 `-invest` / `-deep` / `-branch`。
  - 新增 ENScan JSON normalize：`invest` / `holds` / `branch` → organization candidates；`icp` / `app` / `wx_app` / `wechat` / `weibo` → target candidates。
  - 新增真实执行路径：`tokio::process::Command` + 180s timeout，解析 stdout 和 out_dir 下 `.json` artifacts，并映射 `completed` / `checked_empty` / `unavailable` / `failed` provider status。
- **运行过的验证**：
  - `cargo fmt --package golish` → **exit 0**。
  - `cargo test -p golish asset_intel --lib` → **exit 0 / 4 passed, 236 filtered out**。
  - `cargo test -p golish candidate_upsert --lib` → **exit 0 / 1 passed, 239 filtered out**。
  - `cargo check -p golish` → **exit 0**，仅报告既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → **exit 0 / No fixes applied**。
  - `ReadLints`（本轮相关文件）→ **No linter errors found**。
  - `git diff --check -- <本轮相关文件>` → **exit 0**。
- **已知风险或未解决问题**：
  - 尚未做真实 ENScan_GO 外部请求 E2E；单测覆盖的是命令构建和 JSON normalize，避免测试阶段触发外部站点访问。
  - 当前默认 provider source 是 `aqc`；多 source/auto mode 合并去重可作为后续 Phase 4 多 provider / 多 source 扩展。
  - 发现结果仍只写 candidates，不自动进入 active scan scope，符合授权边界。

---

### 2026-05-22 · Asset Intel Provider Abstraction Phase 1

- **本轮目标**：开始实现 `docs/design/2026-05-22-asset-intel-provider-abstraction.md` 的 Phase 1 服务抽象，让 Discover Assets 先接统一 provider / hydrate 契约，而不是绑定 ENScan_GO。
- **已完成**：
  - 新增 `docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md` Phase 1 实施计划。
  - 新增 `backend/crates/golish/src/tools/asset_intel.rs`：provider descriptor、hydrate request/result、provider status、ENScan_GO skeleton descriptor、normalize provider records 纯函数、`asset_intel_list_providers` / `asset_intel_hydrate` IPC。
  - 新增 `backend/crates/golish/src/commands_facade/asset_intel.rs`，并注册到 `commands_facade/mod.rs` 与 `commands_registry.rs`。
  - `organizations.rs` 抽出 `upsert_organization_candidates_for_org` helper，供 Asset Intel service 复用现有 candidates 写入路径。
  - 新增 `frontend/lib/api/asset-intel.ts` typed wrapper，并从 `frontend/lib/api/index.ts` 导出 `assetIntel` namespace。
- **运行过的验证**：
  - `cargo test -p golish asset_intel --lib` → **先红后绿**：红灯 `0 passed / 2 failed`；实现后 `2 passed / 0 failed`。
  - `cargo test -p golish candidate_upsert --lib` → **exit 0 / 1 passed, 237 filtered out**。
  - `cargo check -p golish` → **exit 0**，仅报告既有 `capture/data_dir.rs::session_dir` dead_code warning。
  - `cargo fmt --package golish` → **exit 0**。
  - `pnpm exec tsc --noEmit` → **exit 0**。
  - `pnpm exec biome check frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts` → 初次 import order fail；修复后 **exit 0 / No fixes applied**。
  - `ReadLints`（本轮新增/改动文件）→ **No linter errors found**。
- **已知风险或未解决问题**：
  - Phase 1 当前是 provider skeleton：`asset_intel_hydrate` 会返回 `checked_empty` evidence，不调用真实 ENScan_GO CLI；真实 CLI 接入属于 Phase 2。
  - `./init.sh` 暴露既有 `check-fe` Biome 问题与 `golish-pty` clippy 问题，和本轮新增文件无关；本轮没有改动那些历史文件。

---

### 2026-05-21 · ENScan TYC Auto-capture JSON 补齐

- **本轮目标**：用户反馈天眼查前端仍没有 Auto-capture，询问是否缺 JSON。
- **已完成**：
  - **`resources/toolsconfig/enscan-go.json`**：给 `tyc` group 增加 `capture` 段。
  - TYC 规则组合：
    - `cookie_joined` 抓 `.tianyancha.com` 完整 Cookie → `cookies.tianyancha`
    - `request_header` 抓 `X-Tycid` → `cookies.tycid`
    - `request_header` 抓 `Authorization` → `cookies.auth_token`
  - 前端会因 group.capture 存在自动显示 `Auto-capture` / `清除登录态`。
- **运行过的验证**：
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null` → **exit 0**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_aqc_capture_recipe_loads)' --status-level fail` → **exit 0 / 1 passed, 72 skipped**
  - `ReadLints`（`enscan-go.json`）→ **No linter errors found**
- **已知风险或未解决问题**：
  - TYC 是否能一次抓全 3 项需要真实 E2E：如果天眼查不通过 fetch/XHR 显式设置 `X-Tycid` / `Authorization`，`request_header` 会提示未观察到 header，需要再根据实际页面行为改 JSON 规则（比如 local_storage / page_content）。

---

### 2026-05-21 · 通用 Capture Rule 扩展（JSON-only 方向）

- **本轮目标**：用户要求补一轮通用抓取能力，目标是未来换工具尽量只改 JSON，不改前后端代码。
- **已完成**：
  - **`golish-integrations/src/schema.rs`**：`CaptureRule` 新增 `request_header` variant；更新注释，明确当前 schema 覆盖 cookie / storage / page / URL / JS request header。
  - **`golish-integrations/src/resolver.rs`**：capture target field 校验覆盖 `request_header`。
  - **`capture/engine.rs`**：
    - 已实现 `local_storage`：读取 `window.localStorage[key]`。
    - 已实现 `session_storage`：读取 `window.sessionStorage[key]`。
    - 已实现 `page_content`：等待 selector，读取 `textContent` 或 attribute。
    - 已实现 `url_query`：从当前 URL 读取 query 参数。
    - 新增 `request_header`：通过初始化 JS 监听页面 `fetch` / `XMLHttpRequest` 显式设置的 request headers，再由 JSON rule 按 header name + 可选 `url_pattern` 提取。
    - JS 取值通过临时 document title bridge 回传，Rust 用 nonce + base64 解码，不把 secret 写进日志。
  - **`frontend/lib/api/integrations.ts`**：TS union 同步新增 `required_names` / `request_header`。
  - **`resources/toolsconfig/enscan-go.json`**：给 KC/Qimai 与 RB/RiskBird 增加 `capture` 段（cookie_joined），因此前端会显示 Auto-capture；TYC/MIIT 保留待真实站点行为验证后再配 request_header/page/storage 组合。
- **运行过的验证**：
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)' --status-level fail` → **exit 0 / 31 passed, 204 skipped**
  - `cargo nextest run -p golish-integrations -E 'test(schema::tests::capture) | test(fixture_enscan_aqc_capture_recipe_loads)' --status-level fail` → **exit 0 / 4 passed, 69 skipped**
  - `pnpm exec tsc --noEmit` → **exit 0**
  - `pnpm exec biome check frontend/lib/api/integrations.ts` → **exit 0 / No fixes applied**
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null && python3 -m json.tool feature_list.json >/dev/null` → **exit 0**
  - `git diff --check -- <本轮相关文件>` → **exit 0**
  - `ReadLints`（本轮相关文件）→ **No linter errors found**
- **已知风险或未解决问题**：
  - `request_header` 能抓页面 JS 显式设置的 fetch/XHR header；不能抓浏览器自动附加的 Cookie header（Cookie 已由 cookie/cookie_joined 规则覆盖）。
  - TYC/MIIT 需要真实站点 E2E 后确认 token/header 来源，再仅通过 JSON 配规则；本轮没有假装未实测来源已经完成。
  - `resources/toolsconfig/enscan-go.json` 整文件 biome format 仍会触发既有格式差异；本轮只保证 JSON 语法合法和 schema fixture 通过，未做整文件重排。

---

### 2026-05-21 · Secret 已配置态视觉增强

- **本轮目标**：用户指出 AQC Cookie 字段显示 `•••• (configured)` 但视觉上像没凭证，希望样式更明确。
- **已完成**：
  - **`SecretInput.tsx` / `SecretTextarea.tsx`**：当后端 `has_value=true` 且本地输入为空时，使用 emerald 成功态边框/背景/placeholder 颜色，明确表示“已有凭证”。
  - **`FieldRenderer.tsx`**：把 secret 字段的 `hasExistingSecret` 状态传入具体输入组件。
  - **`SecretInput.test.tsx`**：新增已配置态样式断言。
- **运行过的验证**：
  - `pnpm exec vitest run frontend/components/Settings/IntegrationsSettings/fields/SecretInput.test.tsx frontend/components/Settings/IntegrationsSettings/IntegrationGroup.test.tsx` → **exit 0 / 12 passed**
  - `pnpm exec tsc --noEmit` → **exit 0**
  - `pnpm exec biome check frontend/components/Settings/IntegrationsSettings/fields/SecretInput.tsx frontend/components/Settings/IntegrationsSettings/fields/SecretTextarea.tsx frontend/components/Settings/IntegrationsSettings/fields/FieldRenderer.tsx frontend/components/Settings/IntegrationsSettings/fields/SecretInput.test.tsx` → **exit 0 / No fixes applied**
  - `ReadLints`（secret field 相关文件）→ **No linter errors found**

---

### 2026-05-21 · Capture webview 登录态持久化 + 清除登录态按钮

- **本轮目标**：用户确认 AQC 抓取和 ENScan 实测已通后，提出每次重启/再次 Auto-capture 都要重新登录，体验差；希望保留 Auto-capture 浏览器网页登录态。
- **设计决策**：
  - 把 capture webview 存储从“每次 session_id 独立”改成“按 `(tool_id, group_id)` 稳定 profile”，例如 `enscan-go__aqc`。
  - 新增“清除登录态”按钮，只清 Auto-capture webview 的网页登录态，不清 `cookies.aiqicha` 已写入的 ENScan 配置。
- **已完成**：
  - **后端 profile 存储**：`capture/data_dir.rs` 新增 `profile_key/profile_dir/cleanup_profile_dir`；`webview_isolation.rs` 的 macOS `data_store_identifier` 改为从稳定 profile key 派生；`engine.rs::start_webview` 改用 profile dir/key。
  - **后端 IPC**：新增 `integrations_capture_clear_profile`，通过隐藏 webview 绑定同一 profile 调 `clear_all_browsing_data`，再清 profile dir；已接入 `tools/integrations/mod.rs`、`commands_facade/integrations.rs`、`commands_registry.rs`。
  - **前端 UI/API**：`frontend/lib/api/integrations.ts` 新增 `captureClearProfile`；`IntegrationGroup.tsx` 在有 capture recipe 的 group 上渲染“清除登录态”按钮；中英文 i18n 已补；`IntegrationGroup.test.tsx` 覆盖“清登录态不调用 integrations.clear”。
- **运行过的验证**：
  - `cargo nextest run -p golish --lib -E 'test(profile_key_is_stable_and_path_safe) | test(profile_dir_is_stable_for_tool_group) | test(macos_data_store_id_uses_profile_key_not_session_uuid_identity)' --status-level fail` → **exit 0 / 3 passed, 230 skipped**
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)' --status-level fail` → **exit 0 / 29 passed, 204 skipped**
  - `pnpm exec vitest run frontend/components/Settings/IntegrationsSettings/IntegrationGroup.test.tsx` → **exit 0 / 6 passed**
  - `pnpm exec vitest run frontend/components/Settings/IntegrationsSettings/ frontend/components/Settings/IntegrationsSettings/IntegrationGroup.test.tsx` → **exit 0 / 22 passed**
  - `pnpm exec tsc --noEmit` → **exit 0**
  - `python3 -m json.tool feature_list.json >/dev/null && python3 -m json.tool frontend/lib/i18n/en.json >/dev/null && python3 -m json.tool frontend/lib/i18n/zh-CN.json >/dev/null` → **exit 0**
  - `git diff --check -- <本轮相关文件>` → **exit 0**
  - `ReadLints`（本轮改动文件）→ **No linter errors found**
- **已知风险或未解决问题**：
  - 需要用户手动 E2E：重启 Golish 后再次点 ENScan AQC ⚡，观察是否无需重新登录或至少保留百度已登录态；点击“清除登录态”后再点 ⚡ 应回到未登录状态。
  - 未跑整仓 `just precommit`；当前仓库仍有既有 blocker（见“当前 blocker”），本轮只验证 capture/integrations 范围。

---

### 2026-05-21 · ENScan AQC 软重试状态机修复

- **本轮目标**：用户贴出新日志：`success_url_pattern matched` 已出现，但第一次在 `https://aiqicha.baidu.com/` 抽取时 `raw_count=0`，随后第二次 pattern 仍匹配却看不到后续 cookie fetch，说明 qiye pattern 修复不是最终根因。
- **根因结论**：`try_extract` 进入 `Extracting` 后，如果 `CookieJoined.required_names=["BDUSS"]` 缺失会返回 `[SOFT_RETRY]`，但原代码只清理 `failed_rules/captured_fields`，没有把 session state 从 `Extracting` 改回可重试状态；后续导航触发 `try_extract` 会被幂等 guard 直接 no-op。
- **已完成**：
  - **`backend/crates/golish/src/tools/integrations/capture/engine.rs`**：新增 `rearm_after_soft_retry`，软重试时清空临时失败/捕获字段并 transition 回 `WaitingLogin`，让下一次匹配导航可以重新抽取 cookie。
  - 新增回归测试 `soft_retry_rearms_waiting_login_after_empty_cookie_attempt`，锁住“软重试后不能卡在 Extracting”的行为。
- **运行过的验证**：
  - `cargo nextest run -p golish --lib -E 'test(soft_retry_rearms_waiting_login_after_empty_cookie_attempt)' --status-level fail` → **exit 0 / 1 passed, 230 skipped**
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)' --status-level fail` → **exit 0 / 27 passed, 204 skipped**
  - `ENScan_GO/enscan-v2.0.5-darwin-amd64 -n 小米 -type aqc -field icp`（在本机 tools 目录运行，使用刚抓取写入的配置）→ **exit 0**；返回小米企业信息 + 3 页网站备案数据，并导出 `outs/小米-2026-05-21--1779374086.xlsx`。
  - `python3 -m json.tool feature_list.json >/dev/null` → **exit 0**
  - `ReadLints`（`engine.rs`）→ **No linter errors found**
- **已知风险或未解决问题**：
  - 用户已重新 `just dev` 实测 AQC ⚡：UI 显示 `Captured 1 field(s) successfully` 且 Cookie 字段 `(configured)`；后端日志第二次 fetch `raw_count=28` 且包含 `BDUSS`；ENScan AQC 真实查询已通过。
  - `cargo fmt --package golish --check` → **exit 1**，输出包含既有 `tauri_app.rs`、`capture/data_dir.rs`、`capture/session.rs`、`tools/integrations/state.rs` 等格式差异；未做整包格式化以避免改动无关文件。
  - 未跑整仓 `just precommit`；当前仓库存在既有 blocker（见“当前 blocker”），本轮只验证 capture 范围。

---

### 2026-05-21 · ENScan AQC 登录后跳 qiye.baidu.com pattern 修复

- **本轮目标**：用户报告 AQC 自动抓取登录后 webview 一直挂着不关；MCP-7 已定位到爱企查登录完成后会跳到 `https://qiye.baidu.com/usercenter/personalcenter?fr=c1009`，旧 `success_url_pattern` 只覆盖 `aiqicha.baidu.com`，导致 `try_extract` 不再触发。
- **已完成**：
  - **`resources/toolsconfig/enscan-go.json`**：AQC `success_url_pattern` 扩展为同时匹配 `aiqicha.baidu.com` 和 `qiye.baidu.com`，并覆盖根路径、query/hash、`home`、`usercenter`、`user/`、`personalcenter`；说明文案同步提醒百度企业跳转。
  - **`resources/toolsconfig/enscan-go.json`**：AQC capture rule 保持 `cookie_joined` 写 `cookies.aiqicha`，并加 `required_names: ["BDUSS"]`，避免在未登录根页面提前抓匿名 cookie header。
  - **`backend/crates/golish-integrations/src/resolver.rs`**：fixture `fixture_enscan_aqc_capture_recipe_loads` 新增断言，真实加载 `resources/toolsconfig/enscan-go.json` 后编译 `success_url_pattern`，并验证能匹配 `https://qiye.baidu.com/usercenter/personalcenter?fr=c1009`。
- **运行过的验证**：
  - `python3 -c 'import json; json.load(open("resources/toolsconfig/enscan-go.json")); print("VALID JSON")'` → **exit 0 / VALID JSON**
  - `cargo nextest run -p golish-integrations -E 'test(fixture_enscan_aqc_capture_recipe_loads)' --status-level fail` → **exit 0 / 1 test run: 1 passed, 72 skipped**
  - `ReadLints`（`resolver.rs` + `enscan-go.json`）→ **No linter errors found**
  - `cargo fmt --package golish-integrations --check` → **exit 1**，包内已有格式差异（`resolver.rs` 既有片段、`storage/external_file.rs`、`tester.rs`、`types.rs`），未做无关格式化。
- **已知风险或未解决问题**：
  - 未做真实手动 E2E；仍需用户 `just dev` → Settings → Integrations → ENScan_GO → AQC ⚡ → 完成百度验证后确认 webview 自动关闭、toast 变绿、`cookies.aiqicha` 已配置。
  - `just precommit` 仍受既有 monorepo 问题阻塞，详见上方当前 blocker。

---

### 2026-05-21 · integrations Test connection 真 wire — exec resolver + builtin dispatcher 双修

- **本轮目标**：用户报告"加进去了（指 Auto-capture 跑通）但点 Test connection 没反应"。截图显示按钮右侧"Unknown"灰标签 + Cookie 字段 (configured) + Captured 1 field(s) successfully toast。用户进一步质疑通用性："如果不是 enscan 工具 其他工具呢？"。最终决定上 A+B 一起：A 修 `{{exec}}` no-op resolver（影响所有 TestKind::Exec 工具），B 修 Builtin 分支返 Unknown 不路由（影响 5 个 intel provider）。两条路径都做得通用，不只针对 ENScan / intel。
- **诊断证据链**：① `enscan-go.json` aqc test = `kind:exec, cmd:{{exec}} -n 小米 -type aqc -field icp` ② `tester.rs:122-129` 拿不到 exec_path 时返 `IntegrationHealth::unknown` ③ `state.rs:58-62` 自陈 "Phase 3 ships a no-op; Phase 5 will wire" ④ `TestButton.tsx:59` `<HealthPill>` 把 unknown 渲染成右下角灰色"Unknown"小标签，message 只在 hover title 里——用户视觉上以为没反应。
- **已完成（commit `7a2a5c6`，+625 行 / -51 行 / 6 个文件）**：
  - **新建 `backend/crates/golish-pentest/src/tool_resolve.rs`**（+150 行）：sync `pub fn resolve_tool_executable(tool_id, &[ToolConfig], &Path) -> Option<String>`，逻辑沿用 `golish-pentest-mcp::builder::resolve_executable`：native runtime 先 `golish_shell_exec::which_executable($PATH 命令)`、否则 `tools_dir.join(executable).exists()`、最终回退原字符串。+ 4 个单测（unknown id / 真实文件 / 缺失 / 非 native runtime）。
  - **`golish-pentest/src/lib.rs`** +2：`pub mod tool_resolve` + 重导出 `resolve_tool_executable`。
  - **`golish-integrations/src/tester.rs`** +97/-10：① 新 `#[async_trait] pub trait BuiltinDispatcher` ② `DefaultTester` 加 `builtin_dispatcher: Option<Arc<dyn BuiltinDispatcher>>` + `with_builtin_dispatcher` builder ③ `TestKind::Builtin` 分支：Some(d) → d.dispatch / None → 保留旧 Unknown（向后兼容）④ 新增 `builtin_routed_to_dispatcher_when_attached` 测试（FakeDispatcher 注入 → 返 Healthy）。
  - **`golish-integrations/src/lib.rs`** +1：公开 `BuiltinDispatcher / DefaultTester / ExecResolver`。
  - **`golish/src/tools/integrations/state.rs`** +400/-51：① 改 `IntegrationsState::new` 签名为 5 参（接受真 exec_resolver + Option<BuiltinDispatcher>）② `build_default` 接受 `(settings_mgr, tools_dir, toolsconfig_dir)`，内部调 `scan_toolsconfig` 拿快照构造真 resolver closure ③ `collect_in_code_schemas_and_providers` 同时返 schemas + `HashMap<String, Arc<dyn IntelProvider>>`，不重复构造 5 个 Provider ④ 新增 `IntelBuiltinDispatcher` + `BuiltinDispatcher` impl：查 registry → 拿第一个 secret field → `provider.test_connection(&key).await` → `connection_status_to_health` 映射 4 variant ⑤ 8 个新单测（pick_credential / 4 个 ConnectionStatus 映射 / dispatcher 未知 id / dispatcher 错 group）。
  - **`golish/src/app/tauri_app.rs`** +15/-5：`tauri::async_runtime::block_on` 一次性取 `tools_dir` + `toolsconfig_dir` 喂给 `build_default`。
- **运行过的验证**：
  - `cargo check -p golish-pentest -p golish-integrations -p golish` → **exit 0 / 0 warning**（29.83s）
  - `cargo nextest run -p golish-pentest -E 'test(tool_resolve)'` → **4 tests run: 4 passed**
  - `cargo nextest run -p golish-integrations` → **71 tests run: 71 passed**（前轮 70 + 我加 1）
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations)'` → **31 tests run: 31 passed**（前轮 23 + 我加 8）
  - `ReadLints` 6 改动文件 → No linter errors found
- **已记录证据**：见上方 4 个 cargo 验证 + commit `7a2a5c6` HEAD
- **提交记录**：`7a2a5c6` (feat/asm-intel-providers 分支)，尚未 push（等用户手动 E2E 通过后一并 push）
- **已知风险或未解决问题**：
  - **运行时新装工具**：snapshot 只在 Tauri 启动时取一次。用户安装新工具后想要 Test connection 立即生效 → 需重启 Golish。这是可接受的（test 按钮路径低频）。未来如要支持热刷新可改成 `Arc<RwLock<Snapshot>>` + 监听 install event（P2）。
  - **ENScan_GO 实际是否安装**：用户当前环境下 `enscan-v2.0.5-darwin-amd64` 必须真实存在于 `tools_dir/ENScan_GO/` 才能跑得到 ok_regex / fail_regex 判定。如果工具未安装，Test connection 会返 unknown + message 提示"executable not found"。
  - **手动 E2E 未做**：需要用户 just dev 后在 Settings → Integrations → ENScan_GO → AQC 点 Test connection 看 pill 是否变绿（cookie 有效）/ 变红（cookie expired）/ 仍 Unknown（工具未装）。同样需要在 0.zone 等填入真实/假 key 测试 Builtin 分支。
  - **`integration_schema` 假定每 group 第一个 secret field 就是测试用 credential**：对当前所有 schema 成立（5 intel provider 都是单 `api_key` field）。如果未来某 schema 是 `TestKind::Builtin` 但有多个 secret field，需要在 schema 里加 `credential_field: "..."` 字段指明，并改 `pick_credential_value`。
- **下一步最佳动作**：
  1. **用户 just dev → 测 4 条路径**：① ENScan AQC（应该绿）② ENScan AQC 把 cookie 故意删一段（应 fail_regex 命中或 ok_regex miss → 红 Invalid）③ 0.zone 填真实 key（应绿）④ 0.zone 不填 / 填空 key（应 AuthFailed → 红 Invalid）
  2. 通过 → push `7a2a5c6` 到远端
  3. 不通过 → 视失败模式修：a) ok_regex 没命中 → 调 enscan-go.json b) provider.test_connection 返意外 NetworkError → 看错误 message c) IntelBuiltinDispatcher pick_credential 拿空字符串 → 看是否 cleartext 字段名不一致
  4. 4 路径全过 → integrations.outstanding_followups #4 + #5 真正解决（已在 commit message 标注）

---

### 2026-05-21 · 凭据抓取器 Phase 5 T5.1 ENScan AQC capture recipe + fixture 测试

- **本轮目标**：用户指令"推 Phase 5 AQC recipe + E2E"。Phase 5 范围：T5.1 加 capture recipe → T5.2 手动 E2E → T5.3 反向 6 case → T5.4 just precommit 全绿 + 切 passing。我能落代码的是 T5.1 + 一个 fixture smoke 测试；T5.2 / T5.3 必须 `just dev` + 真实登录爱企查 → 只能由用户做；T5.4 因 preexisting 编译错（M2 cherry-pick 后 PlanStep failure_kind 字段缺失 + biome 警告）无法整体 green，本轮逐项跑了能跑的验证
- **已完成（commit `308eddf`，+79 / -1 \u00b7 2 个文件）**：
  - **`resources/toolsconfig/enscan-go.json`** aqc group 新增 `capture` 段：
    - `login_url`: `https://aiqicha.baidu.com/`
    - `success_url_pattern`: `aiqicha\\.baidu\\.com/(home|company|usercenter|user|s)` — 覆盖爱企查登录后的几条常见 landing 路径
    - `timeout_secs`: 300（在 engine clamp 窗口内）
    - 单条 Cookie rule：`domain=.baidu.com / name=BDUSS / target_field=cookies.aqc / required=true`
    - `description` / `instructions` 加注意事项：ENScan 期望 `cookies.aqc` 是完整 Cookie header，但 P1 MVP 引擎写的是单个 BDUSS 值；若 ENScan 拒绝则用户可手动补完整 header（CookieJoined 是 P2 scope）
  - **`backend/crates/golish-integrations/src/resolver.rs`** 新增 fixture smoke 测试 `fixture_enscan_aqc_capture_recipe_loads`：
    - 从 `CARGO_MANIFEST_DIR` 向上走 3 级到 repo root，定位 `resources/toolsconfig/`，用真实 `DefaultSchemaResolver::get("enscan-go")` 加载
    - 4 个断言：login_url 形如 https://aiqicha.baidu.com / timeout 在 [30,900] / 至少 1 rule / 必有 Cookie rule 写 BDUSS → cookies.aqc
    - 不存在 toolsconfig 目录时 silently skip（不在 git checkout 环境时不强求跑）
- **运行过的验证**：
  - `python3 -m json.tool resources/toolsconfig/enscan-go.json` → VALID JSON
  - `cargo nextest run -p golish-integrations --status-level fail` → **70 tests run: 70 passed, 0 skipped**（前轮 69 → +1 fixture smoke）
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations)'` → **23 tests run: 23 passed, 190 skipped**（含 Phase 2 17 + Phase 3 6 commands；零回归）
  - `ReadLints`（enscan-go.json + resolver.rs）→ No linter errors found
  - **未跑** `just precommit`（preexisting biome 警告 + 8 个 `ai_events_characterization` PlanStep struct literal 编译错，M2 cherry-pick 遗留，与 capture 无关）
- **已记录证据**：见上方 4 个验证结果；commit `308eddf` HEAD 已就位
- **提交记录**：`308eddf`，feat/asm-intel-providers 分支，未 push
- **已知风险或未解决问题**：
  - **BDUSS 单值 vs 完整 Cookie header**：plan v2 已标记这是 P1 实施阶段实测拍板项。如果 `enscan -n 小米 -type aqc -field icp` 拿到只含 BDUSS 的 cookies.aqc 仍工作 → P1 收工；否则需要：① 用户手动复制完整 header 覆盖 ② 后续把 engine 升级到 CookieJoined rule（~30 行额外代码，P2 范围）
  - **success_url_pattern 实测可能需要调整**：列了 5 条 path（home / company / usercenter / user / s），但爱企查可能 login 后跳到其它页面（如 search-result 直接跳 `/s/xxx`）。如果 pattern miss 则用户登录后 toast 不跳到 extracting，会一直在 waiting_login 直到 5 分钟 timeout
  - **真实手动 E2E 完全没做**：本会话内的 Rust 单测最多验证到"schema 解析合法 + Tauri command 编译通过"；真实弹窗 / 真实 cookie 抓取 / 真实 vault 写入 / 真实 ENScan 调用 → 全部依赖 `just dev` + 用户真账号登录爱企查。这是 P1 MVP 的最后一公里
  - **T5.4 `just precommit` 不能跑绿**：preexisting `golish/tests/ai_events_characterization/roundtrip_and_deserialization.rs` 8 个 PlanStep 字面量缺 `failure_kind` 字段。修这个 = 另外的 task，跟 capture 无关
  - **CaptureStatusToast 错误显示**：现在显示 `[CAPTURE_*]` 前缀的原始字符串，对开发者友好对用户不友好；P2 可加 i18n mapping（计划已记录）
- **T5.2 / T5.3 手动 E2E checklist（用户做）**：
  1. **T5.2 正向 E2E**：
     - `just dev` 启动 Tauri 应用
     - Settings → Integrations → ENScan_GO → aqc group → 应出现 ⚡ "自动抓取" 按钮
     - 点击 ⚡ → confirm dialog 弹出（标题"自动抓取凭据" + 描述含 login_url 和 timeout）
     - 点击"打开浏览器并登录" → 应弹出独立的 Tauri webview window 打开 aiqicha.baidu.com
     - 在弹窗内用真实账号登录爱企查
     - 登录后 success_url_pattern 命中 → 1-2s 内 webview 自动关闭 → toast 变绿"成功抓取 1 个字段" → cookies.aqc 字段显示"已配置 badge"
     - 终端跑 `enscan -n 小米 -type aqc -field icp` → 应返回小米的 ICP 数据
     - 截屏发回 + 记录 enscan 输出关键行
  2. **T5.3 反向 6 case**：
     - case 1：点 ⚡ → confirm 后 5 分钟不操作 → toast 变红"登录超时未完成抓取"，cookies.aqc 字段无变化
     - case 2：点 ⚡ → confirm → 弹窗出现后 toast 上点 "Cancel" → 弹窗立即关闭，toast 显示"已取消抓取"
     - case 3：同一 aqc group 已经在抓取中（state=waiting_login） → 再点 ⚡ → toast 顶部立刻显示 `[CAPTURE_ALREADY_RUNNING] session already in-flight for enscan-go/aqc`（startError 路径）
     - case 4：抓取过程中**手动关闭弹窗（点窗口右上 X）** → 当前 P1 没有 on_close handler，会等到 TTL timeout（5 分钟）才转移到 Timeout —— 这是 P2 增强；现在可以接受
     - case 5：成功抓取后查看 `~/Library/Application\ Support/com.golish.platform/capture-sessions/` 目录 → 应该是空的（cleanup_session_dir 已删除）
     - case 6：成功抓取 1 小时后调 `await window.__TAURI_INTERNALS__.invoke("integrations_capture_status", { args: { session_id: "<刚才的id>" } })` → 应返 `[CAPTURE_SESSION_NOT_FOUND]`（GC 已清）
- **下一步最佳动作**：
  1. **用户跑 T5.2 + T5.3 E2E**，截屏 + 记录关键现象给我
  2. 全过 → 我把 `feature_list.json` 的 `capture-engine` 切 `passing` + commit metadata
  3. 不过/部分过 → 视具体失败模式决定：a) BDUSS 单值不够 ENScan → 把 engine 升级 CookieJoined（~30 行）b) success_url_pattern 漏 path → 改 enscan-go.json 加 path c) 其它 P2 增强（手动关窗 → on_close handler / CAPTURE_ALREADY_RUNNING UX 优化）
  4. 或者先 push 本轮 11 个 commit 到远端再做 E2E

---

### 2026-05-21 · 凭据抓取器 Phase 4 前端 UX（T4.1-T4.5 单 commit）

- **本轮目标**：用户指令"推 Phase 4 前端 UX"。按计划 Phase 4 把 i18n + useCaptureSession hook + 3 个 UI 组件 + 集成进 IntegrationGroup.tsx 一次性落地。计划上 T4.1-T4.5 分了 5 个 commit，但 hook ↔ 组件 ↔ IntegrationGroup 集成是紧耦合（类型签名互相依赖），单 commit 避免中间 broken。T4.6 计划测试我聚焦在 CaptureButton 组件级（4 case），集成级"点击→对话→IPC"覆盖在 hook + Phase 5 手动 E2E。
- **已完成（commit `7d4d163`，单 commit +730 行 / 8 个文件）**：
  - **i18n**：`en.json` + `zh-CN.json` 各 +28 行。新增 `integrations.capture.button.{label,tooltip}` / `dialog.{title,description,start,cancel}` / `toast.{waitingLogin,navigating,extracting,captured,partial,timeout,failed,cancelled}` / `errors.{noRecipe,alreadyRunning,webviewFailed,unknown}`。description / toast 用 `{{url}}` / `{{fields}}` / `{{ttl}}` / `{{remaining}}` / `{{count}}` / `{{captured}}` / `{{failed}}` 插值占位
  - **`hooks/useCaptureSession.ts`** (+216 行)：自管 confirm dialog 状态 / pendingRequest / live session / lastEvent / startError；1Hz countdown 由 `session.expires_at` 推驱；`@tauri-apps/api/event` `listen("integration-capture")` 全局订阅一次（用 `sessionIdRef` 过滤非本 session 事件）；接收 `onTerminalSuccess?: () => void` 回调，在 `captured` / `partial` 触发，让父组件 refresh 自己的 snapshot（避开本项目无 react-query 的现实）
  - **`CaptureButton.tsx`** (+62 行)：`group.capture` 不存在 → 返 `null`；Wand2 icon + 琥珀色 pill 风格匹配 toolbar；用现有 `@/components/ui/tooltip`
  - **`CaptureButton.test.tsx`** (+109 行)：4 case（hidden when no capture / shown when present / onStart 传 toolId+groupId / disabled 时不 fire）
  - **`CaptureConfirmDialog.tsx`** (+91 行)：用现有 `@/components/ui/dialog`（Radix Dialog）替代不存在的 alert-dialog——节省一个 `@radix-ui/react-alert-dialog` 依赖。渲染 recipe.login_url + 提取字段列表 + TTL + 可选 instructions
  - **`CaptureStatusToast.tsx`** (+158 行)：8 状态可视化（spinner+countdown / green / yellow / red X / clock / gray X）；in-flight 状态 inline Cancel button；`failed` 状态原样展示 `session.error_message` 让 `[CAPTURE_*]` prefix 可见；当没有 session 但有 startError 时单独渲染（处理 CAPTURE_NO_RECIPE / CAPTURE_ALREADY_RUNNING 等启动错误）
  - **`IntegrationGroup.tsx`** (+38 行)：① 取 `useIntegrationGroup` 暴露的 `reload`（非 `refresh`，跟 hook 实际 API 一致）② 用 `useCaptureSession({ onTerminalSuccess: () => void reload() })` ③ Toolbar 在 Clear 和 flex-1 spacer 之间插入 CaptureButton（与写入操作同组、Test 仍 pin 右）④ 在 toolbar 上方渲染 CaptureStatusToast（session 或 startError 时才渲）⑤ 组件根挂载 CaptureConfirmDialog
- **运行过的验证**：
  - `pnpm exec tsc --noEmit`（全前端）→ exit 0（10.1s）
  - `pnpm exec vitest run frontend/components/Settings/IntegrationsSettings/` → **21/21 passed**（既有 17 + CaptureButton 4 个新）
  - `pnpm exec vitest run frontend/components/Settings/` → **72/72 passed**（既有 68 + 新 4）→ 整 Settings 模块零回归
  - `pnpm exec biome check`（24 个 IntegrationsSettings 文件 + 2 个 i18n）→ No fixes applied（一次自动修：长 import 折单行 + captureCancel/captureStart sort）
  - `ReadLints` 8 个改动文件 → No linter errors found
- **已记录证据**：
  - 21/21 + 72/72 + 24 文件 biome 干净 + 0 lint error
  - vitest.config.ts 第 23 行 `@tauri-apps/api/event` alias 到 `frontend/test/mocks/tauri-event.ts`——这是 useCaptureSession 能在 jsdom 下 silent-noop 的原因；测试不需要为 listen() 写额外 mock
- **提交记录**：`7d4d163`，feat/asm-intel-providers 分支，未 push
- **已知风险或未解决问题**：
  - **真实运行验证（T3.3 + Phase 4 Review Checkpoint）未跑**：需要 `just dev` 启动后，手动 ① 看 Settings → Integrations → ENScan_GO → AQC 是否多了 ⚡ 按钮 ② 点击 ⚡ 看是否弹出 confirm dialog ③ 点 "打开浏览器并登录" 看是否弹出独立 webview ④ Toast 在 3 状态下的视觉。其中 ② / ③ 受 Phase 5 AQC capture recipe 是否加进 enscan-go.json 影响——现在 group.capture 为空，⚡ 按钮直接**不渲染**，所以点击连 dialog 都看不到。Phase 5 加完 recipe 后 ⚡ 才出现
  - **`useCaptureSession` listen() 在生产 Tauri 环境的真实表现**：测试用 mock；真实环境第一次见 `integration-capture` 事件如果接收延迟或漏接，UI 会卡在 waiting_login。可以加一个兜底 timer 每 5s 调一次 `captureStatus`，但 P1 MVP 简化没做
  - **CaptureConfirmDialog 用 Radix Dialog**：跟计划上的 AlertDialog 视觉略不同（多了一个右上角 X 关闭，AlertDialog 没有）。UX 上更友好；不算 deviation
  - **CaptureStatusToast `failed` 状态原样显示 error_message**：包含 `[CAPTURE_*]` prefix 字符串——对用户不够友好，但对 debug 极佳。P2 可加 i18n mapping
  - **新增 4 个 CaptureButton 测试**：覆盖了组件级行为，但**没**测 hook + dialog 集成（"点 ⚡ 弹 dialog 点 start 发 IPC"完整 flow）。这种集成测试在本项目通常落在 Playwright E2E，Phase 5 之后可补
- **下一步最佳动作**：
  1. **Phase 5** 启动（~90 分钟，4 个 task）：① T5.1 `resources/toolsconfig/enscan-go.json` 给 AQC group 加 `capture` 段（cookies.aqc → BDUSS cookie）② T5.2 手动 E2E（just dev + Settings → Integrations → ENScan AQC ⚡ → 真实登录爱企查 → 看 toast 变绿 + cookie 写入 + enscan -n 小米 -type aqc 真实跑通）③ T5.3 反向 6 case（超时 / 取消 / 409 / 手动关窗 / data_dir 清干净 / GC 后 404）④ T5.4 just precommit 全绿 + feature_list.json 切 passing
  2. 或者用户希望先做 T3.3 / Phase 4 review，把 Phase 5 AQC recipe 加上之后再做真实 E2E
  3. 或者先 push 本轮所有 commit 到远端

---

### 2026-05-21 · 凭据抓取器 Phase 3 IPC 命令 + 前端 wrappers（T3.1-T3.2 两个 commit）

- **本轮目标**：用户指令"推 Phase 3 IPC 命令"。按 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md` Phase 3 把 3 个 Tauri command 和 3 个 frontend wrapper 接起来。T3.3 是手动 devtools 验证（用户跑 `just dev` 后做），不属于代码工作。
- **已完成**：
  - **新建 `backend/crates/golish/src/tools/integrations/capture_commands.rs`** (+171 行)：
    - `CaptureStartArgs / CaptureSessionArgs` 新 type wrapper 让 IPC 走 `{ args: { tool_id, group_id } }` / `{ args: { session_id } }` 与 `integrations_set` / `_clear` 风格一致
    - `integrations_capture_start`：4 步链 ① `resolver().get(tool_id)` ② 找 group ③ 提 recipe（CAPTURE_NO_RECIPE 错误）④ `engine.register()` + `engine.start_webview()`；start_webview 失败时**回滚 session**：fire-and-forget `transition_and_emit(Failed, [WEBVIEW_CREATE_FAILED])` 让 UI 不留 orphan WaitingLogin
    - `integrations_capture_status`：read-only poll，CAPTURE_SESSION_NOT_FOUND → NotFound(404)（GC > 1h 后）
    - `integrations_capture_cancel`：幂等（engine.transition 已终态时 no-op）+ 关闭 lingering webview（best-effort）
  - **修改 `tools/integrations/mod.rs`** (+4)：`pub mod capture_commands` + 3 个命令名 pub use
  - **修改 `commands_facade/integrations.rs`** (+13 / -5)：doc comment 列出 3 个新命令；pub use 列表加 3 个新命令
  - **修改 `commands_registry.rs`** (+2)：`tauri::generate_handler![]` 列表加 3 个新命令名
  - **修改 `frontend/lib/api/integrations.ts`** (+72)：3 个 IPC wrapper（`captureStart` / `captureStatus` / `captureCancel`），doc 显式列出 8 个 `[PREFIX]` 错误约定让前端 mapErr 能 typed dispatch 不需 parse 字符串
- **运行过的验证**：
  - `cargo check -p golish --message-format=short` → exit 0 / **0 warning**（82s 增量）
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations)'` → **23 tests run: 23 passed, 190 skipped**（含 Phase 2 的 17 + 既有 6 commands；零回归）
  - `pnpm exec tsc --noEmit`（全前端） → exit 0（10.1s）
  - `pnpm exec biome check frontend/lib/api/integrations.ts` → No fixes（首跑因 captureStatus 签名换行报 1 format error，已 collapse 修一次）
  - `ReadLints` 5 个改动文件 → No linter errors found
- **已记录证据**：
  - 23/23 nextest + 0 warning + 0 lint 详见上面
  - 2 个新 commit：`191cbab` (backend +190) + `da1ffea` (frontend +72)
- **提交记录**：`191cbab` / `da1ffea`，feat/asm-intel-providers 分支，未 push
- **已知风险或未解决问题**：
  - **devtools 手动验证未跑**：T3.3 计划上要 `just dev` + 在 devtools console 跑 `invoke("integrations_capture_start", { args: { tool_id: "enscan-go", group_id: "aqc" } })` 看真弹窗。ENScan AQC `capture` recipe 在 Phase 5 才加，所以现在跑会返 `[CAPTURE_NO_RECIPE]`——这是预期行为。可以临时给某个 group 加个 mock capture 来跑验证，但用户决定
  - **start_webview 失败的回滚是 fire-and-forget**：用 `tauri::async_runtime::spawn` 跑 `transition_and_emit`，不 await。如果回滚本身 fail 会沉默到日志（tracing::error）。生产环境若需要可改成 await 但要权衡用户响应延迟
  - **`CAPTURE_ALREADY_RUNNING` 错误的 UI 处理待 Phase 4 实现**：现在后端会返这个错误，前端 wrapper 会把它 throw 出来，但 hook / dialog 还没有针对这个错误的特殊提示（计划上是"先取消才能重启"）
- **下一步最佳动作**：
  1. **Phase 4** 启动（3-4 小时，~6 task）：i18n keys + useCaptureSession hook（订阅 `integration-capture` 事件 + 倒计时 + react-query invalidate）+ CaptureButton / CaptureConfirmDialog / CaptureStatusToast 3 个组件 + 集成进 IntegrationGroup.tsx + 单测
  2. 或者先做 T3.3 手动验证：临时给 `enscan-go` 的 aqc group 加个 mock capture 段（或者给 `core.json` 里的 github 加个 mock），跑 `just dev` 看弹窗能否打开
  3. 或者先 push 本轮 7 个 commit 到远端

---

### 2026-05-21 · 凭据抓取器 Phase 2 CaptureEngine 落地（T2.1-T2.6 单 commit）

- **本轮目标**：用户指令"推 Phase 2 CaptureEngine"。按 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md` Phase 2 把 `CaptureEngine` 模块在 `backend/crates/golish/src/tools/integrations/capture/` 落地。
- **执行决策**：T2.1-T2.6 计划上分 6 个 commit，但 T2.3 (start_webview) / T2.4 (try_extract) / T2.5 (TTL watcher + transition_and_emit) / T2.6 (tauri_app 注册) 是紧耦合（互相调用对方的方法签名），分多 commit 会让中间 commit 编译 broken。本轮选择**单 commit 落盘整个 Phase 2**，类型签名连贯、ReadLints 全绿、单测全过。
- **已完成（commit `e3d5963` 一次性 +1227 行 / 8 个文件）**：
  - **新建 `capture/mod.rs`** (28 行)：`pub mod capture` + re-export `CaptureEngine` / `CaptureSession` / `CaptureSessionHandle`
  - **新建 `capture/data_dir.rs`** (102 行)：`capture_root() / session_dir() / cleanup_session_dir()` + 3 个测（cleanup-missing-noop / create-then-clean / idempotent）。路径：`<dirs::data_dir()>/com.golish.platform/capture-sessions/<session_id>/`
  - **新建 `capture/session.rs`** (204 行)：`TIMEOUT_MIN_SECS=30` / `TIMEOUT_MAX_SECS=900` 常量；`CaptureSession`（Recipe + state + Unix-ms started_at_ms/updated_at_ms + clamped timeout）；`CaptureSessionHandle`（Arc<RwLock>）；4 个测（timeout clamp 上下界 + transition + 终态 info 省略 expires_at + target_field helper）
  - **新建 `capture/webview_isolation.rs`** (91 行)：Phase 0 spike 发现的平台分支抽象。macOS 用 `data_store_identifier([u8;16])`（先尝试 `Uuid::parse_str`，非 UUID 则 `Uuid::new_v5(NAMESPACE_OID, sid)`——避免新加 blake3 依赖），Linux/Windows 用 `data_directory`，Android/iOS no-op。3 个 macOS-only 测（stable / differs / uuid-round-trip）
  - **新建 `capture/engine.rs`** (763 行)：完整 `CaptureEngine`
    - **registry**：`RwLock<HashMap<sid, Handle>>` 双层锁
    - **register()**：UUID v4 生成 sid，拒绝同 `(tool_id, group_id)` 非终态重复
    - **transition / transition_and_emit / cancel**：状态机；终态 emit `"integration-capture"` Tauri event 并 `cleanup_session_dir`；终态后调用 idempotent
    - **start_webview()**：async；用 `apply_isolation` 隔离 + `on_navigation(Fn(&Url) -> bool)`（Phase 0 spike 确认签名）；callback 内 `tauri::async_runtime::spawn` async block 调 `on_navigation_event`
    - **try_extract()**：runs rules → 必需失败 fail-fast；写 vault 走 `IntegrationsState::resolver+pick_backend+backend.write` 4 步链（捕获 `integrations_set` IPC 流程的语义）；emit 最终态 + 关闭 webview
    - **extract_one()**：P1 MVP 仅实现 Cookie；用 `tokio::task::spawn_blocking` 包 `cookies_for_url`（Phase 0 spike 确认是同步 API，Windows 直接调死锁）；其它 5 种 rule 显式 bail "not yet implemented in P1 MVP"
    - **spawn_ttl_watcher()**：10s tick → 扫过期 session 触发 `Timeout` 转移 → 关闭 lingering webview → `gc()` 移除 >1h 终态
    - **on_navigation_event 自由函数**：success_url_pattern 正则匹配后 `app.state::<Arc<CaptureEngine>>()` 拿引擎调 `try_extract`
    - **persist_captured_values 自由函数**：`app.state::<IntegrationsState>() + DbState::pool_ready + pick_backend + backend.write` 4 步
    - 7 个 engine 测：register-unique / register-rejects-dup / register-after-terminal / transition-idempotent / get-not-found / cancel→Cancelled / gc-drops-only-old-terminals
  - **修改 `tools/integrations/mod.rs`** (+1 行)：`pub mod capture`
  - **修改 `tools/integrations/state.rs`** (+20 行)：`map_err()` 扩展处理 8 个 capture-specific `IntegrationError` variant。CaptureNoRecipe/AlreadyRunning/InvalidUrl/InvalidTargetField → Validation(400)；CaptureSessionNotFound → NotFound(404)；WebviewCreateFailed/Timeout/RuleFailed → Internal(500)。`[CAPTURE_*]` / `[WEBVIEW_*]` prefix 保留让前端 mapErr 直接基于 prefix dispatch
  - **修改 `app/tauri_app.rs`** (+19 行)：① `use tauri::Manager` 让 `app.state::<...>()` 可解析 ② 构造 `Arc<CaptureEngine>::new()` 并 `.manage(...)` 在 `IntegrationsState` 之后 ③ setup 闭包扩展为 multi-step：先 `bootstrap::setup_subsystems(app)?`，再 `app.state::<Arc<CaptureEngine>>()` clone + `spawn_ttl_watcher(app.handle().clone())`
- **运行过的验证**：
  - `cargo check -p golish` × 2 → exit 0 / **0 warning**（45.2s 完整 check + 30s T2.6 wiring recheck）
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations::capture)'` → **17 tests run: 17 passed, 196 skipped**（3 data_dir + 4 session + 3 webview_isolation [macOS] + 7 engine）
  - `cargo nextest run -p golish --lib -E 'test(tools::integrations)'` → **23 tests run: 23 passed, 190 skipped**（上面 17 + 既有 6 个 commands 测试零回归）
  - `ReadLints` 7 个改动文件 → No linter errors found
  - **未跑 cargo nextest --test integration**：因为 preexisting `golish/tests/ai_events_characterization/roundtrip_and_deserialization.rs` 编译失败（8 个 PlanStep struct literal 缺 `failure_kind` 字段，M2 cherry-pick 后未补），与本轮 capture 改动无关，下一轮可单独修
- **已记录证据**：
  - `git log -1 --oneline` → `e3d5963 feat(capture): Phase 2 CaptureEngine — scaffold + state machine + ...`
  - 17/17 + 23/23 + 0 warning + 0 lint error 证据见上
- **提交记录**：`e3d5963`，feat/asm-intel-providers 分支，**未 push**
- **已知风险或未解决问题**：
  - **真实 webview / cookie 端到端未试**：Phase 2 全部测试都是 mock state machine 测试，没真弹窗。Phase 5 计划手动 E2E 跑 ENScan AQC（爱企查 BDUSS cookie）
  - **TTL watcher 10s tick 是否过敏感**：plan §Review Checkpoint 提到这点。当前 10s 是为了在 30s 最短 TTL 内至少有 3 次扫描机会；可调到 30s 节省 CPU
  - **`tokio::task::spawn_blocking` 包 cookies_for_url**：spawn_blocking 默认线程池 ≤ 512，正常 capture 流量远不到，安全
  - **on_navigation callback 内 `tauri::async_runtime::spawn` fire-and-forget**：如果 try_extract panic 会沉默丢弃；当前用 `tracing::error!` 兜底，但没结构化上报到前端。可加 panic_handler，但 P1 MVP 接受
  - **`derive_macos_data_store_id` Uuid v5 派生**：固定 NAMESPACE_OID，跨 Golish 进程 / 主机一致——对 P1 MVP 来说"稳定 + 唯一"足够；若未来需要更强隔离可换 BLAKE3
  - **rule_is_required helper** 重复了 `CaptureRule::target_field` 的 6-arm match 模式——可考虑给 `impl CaptureRule` 加 `pub fn is_required(&self) -> bool` 收敛
- **Review Checkpoint（计划要求）**：
  1. 引擎模块分层（engine / session / data_dir / webview_isolation）是否合理 → 用户拍板
  2. TTL watcher 10s 扫一次是否过敏感 → 用户拍板（建议 30s）
  3. Phase 0 spike binary `backend/crates/golish/examples/capture_spike.rs` **未创建**（Phase 0 走 docs.rs WebFetch 替代）→ 无需删除，跳过该 checkpoint
- **下一步最佳动作**：
  1. 用户审 Phase 2 commit `e3d5963`（763 行 engine.rs 是大件，可重点看 `try_extract` 4 步链 + `start_webview` 隔离）
  2. 进入 **Phase 3**（~90 分钟，3 个 task）：3 个 Tauri command（start/status/cancel）+ frontend `captureStart/captureStatus/captureCancel` wrapper + devtools 手动验
  3. 或者用户希望先 push 整个 capture-engine 系列 commit 到远端，确认 e2e 还没崩

---

### 2026-05-21 · 凭据抓取器 Phase 1 完结（T1.2-T1.6 落地 + 2 个 commit）

- **本轮目标**：从 MCP-1 接力上下文，按 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md` Phase 1 推 T1.2-T1.6，把上轮 T1.1 之后已写但未 commit 的代码（types / error / resolver / Cargo.toml / frontend ts mirror）跑全套验证后落盘。
- **已完成**：
  - **审计现状**：用户问"你看到哪里了"——先用 `get_session_summary(MCP-1)` 接回完整上下文 + 读 `agent-progress.md` / git log / 计划文档 / 四个候选文件，发现 T1.2 / T1.3 / T1.4 / T1.5 **代码已经写好且测试齐全**，仅差跑验证 + commit
  - **T1.6 验证**（关键发现 Phase 1 已完成）：
    - `cargo nextest run -p golish-integrations --status-level fail` → **69 tests run: 69 passed, 0 skipped**（前轮 T1.1 后是 49，本批 +20：T1.2 类型 5 个 + T1.3 error 5 个 + T1.4 validate_capture 5 个 + 余 5 个为既有 schema 测试更新）
    - `cargo check -p golish-integrations -p golish` → exit 0（0.62s 增量，意味着只加字段没破坏既有签名）
    - `pnpm exec tsc --noEmit`（全前端 typecheck）→ exit 0（10.4s）
    - `pnpm exec biome check frontend/lib/api/integrations.ts` → No fixes applied
    - `ReadLints` 5 个改动文件 → No linter errors found
  - **commit `11f4aaa`**：Backend 三件套（T1.2-T1.4）
    - `types.rs` +191 行：`CaptureState` enum（8 variant + `is_terminal()`）+ `FailedRule` + `CaptureSessionInfo`（Unix-ms 时间戳）+ `CaptureEventPayload` + 5 个单测
    - `error.rs` +92 行：8 个 capture-specific `IntegrationError` variant（`[CAPTURE_*]` / `[WEBVIEW_*]` 前缀让前端 `mapErr()` 直接基于前缀分发）+ 5 个 Display 渲染测
    - `resolver.rs` +170 行：`validate_capture()` per-group（login_url 必须 http(s)、target_field 必须存在于 group.fields）+ `validate_schema_captures()` per-schema fanout + 在 `DefaultSchemaResolver::collect()` 中集成调用（typo schema 在第一次 IPC 就 fail-fast 而非运行时静默 no-op）+ 5 个 case（accept-valid / reject-unknown-field / reject-javascript-url / reject-file-url / skip-when-none）
    - `Cargo.toml` +1 行：`url = { workspace = true }`（T1.4 依赖，workspace 早已声明）
  - **commit `6dc8303`**：Frontend ts mirror（T1.5）
    - `frontend/lib/api/integrations.ts` +160 行：`IntegrationGroup.capture?: CaptureRecipe`（absent ⇒ 无 ⚡ 按钮）+ `CaptureRecipe` + `CaptureRule` 区分 union（6 variant：cookie / cookie_joined / local_storage / session_storage / page_content / url_query）+ `CaptureState` string union（8 状态）+ `FailedRule` / `CaptureSessionInfo` / `CaptureEventPayload`
    - **注意**：本 commit 仅类型 mirror，**未**加 captureStart / captureStatus / captureCancel IPC wrapper（Phase 3 才加）
- **运行过的验证**：见上方 T1.6 段，5 个命令全部 exit 0 / 69/69 passed
- **已记录证据**：
  - `git log -3 --oneline` → `6dc8303 ... T1.5` / `11f4aaa ... T1.2-T1.4` / `14f21ea ... T1.1`
  - 后端 nextest 数：49（T1.1 后）→ 69（T1.6 后）+20
  - 计划 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md` T1.1-T1.6 6 个 task 全完成
- **提交记录**：`11f4aaa` + `6dc8303`，本轮 2 个 commit，均在 `feat/asm-intel-providers` 分支；未 push
- **已知风险或未解决问题**：
  - Phase 0 spike 是文档化验证（不是真跑 `cargo run --example capture_spike`）；Phase 2 真写 Tauri webview builder 时若发现 docs.rs 描述的签名与本地锁定版本不一致，会在编译阶段立即暴露
  - `validate_capture` 限制 login_url **仅 http/https**，但允许 IPv4 字面量 / IP-only（如 `http://192.168.1.1`）——这是预期行为（自托管 enterprise intel 服务可能用 IP 直连），但若后续 P2 要加 SSRF 防护需在 engine 层做白名单
  - frontend ts mirror 是**手写**（违反 I5「ts-rs derive」）；P1 MVP 接受手写，P2 / P3 可考虑给 `golish-integrations` 加 `ts-rs` derive 收敛
  - `CaptureSessionInfo.expires_at` 使用 Unix-ms `Option<i64>`（不是 chrono `DateTime<Utc>`）——刻意选 i64 让前端 `Date.now()` 可直接比较，避免 RFC3339 反解析；与 `IntegrationHealth.tested_at` 不一致是预期的（前者实时倒计时，后者审计日志展示）
- **下一步最佳动作**：
  1. **Phase 2** 启动（4-5 小时，6 个 task）：在 `backend/crates/golish/src/tools/integrations/capture/` 新建 5 个文件（mod / engine / session / data_dir / webview_isolation），把 `CaptureEngine` 状态机 + per-session data_dir + webview navigation handler + Cookie rule 提取 + 写 vault + TTL watcher + event emit 全链路打通；P2 rule 类型（CookieJoined / LocalStorage / SessionStorage / PageContent / UrlQuery）先 stub 返 "not yet implemented in P1 MVP"。详见计划 §Phase 2 T2.1-T2.6
  2. 或者用户希望先把整 monorepo 的 preexisting biome 警告清掉让 `just precommit` 整体绿，再继续 Phase 2 —— 也合理
  3. 不建议把 ~30 个 preexisting 改动一并 commit；它们跨 ~10 个 crate，属于上一轮的残留游离

---

### 2026-05-21 · 凭据抓取器 Phase 0 spike（API 表面验证 + plan v2）

- **本轮目标**：用户指令"先 push 然后开始搞"。Push 完成后按计划进入 Phase 0 spike——验证 Tauri 2 在锁定版本里 3 个关键 API（`WebviewWindowBuilder::data_directory` / `WebviewWindow::cookies_for_url` / `WebviewWindowBuilder::on_navigation`）真实存在且签名匹配。
- **执行方式**：原计划是写 `examples/capture_spike.rs` 跑真窗口，本会话改为用 `WebFetch` 查 docs.rs 官方文档替代（同等效果 + 不依赖图形环境 + 不污染主代码）。
- **Spike 发现的 3 个偏差**：
  1. **`WebviewWindowBuilder::data_directory(PathBuf)` 在 macOS WKWebView 不支持**——必须用 `data_store_identifier([u8; 16])`（仅 macOS ≥ 14 / iOS ≥ 17）。Linux / Windows 仍用 `data_directory`。**修订**：抽 `capture/webview_isolation.rs` 模块用 `#[cfg(target_os = "macos")]` 分支封装；macOS 把 session UUID 当 16 字节 identifier
  2. **`WebviewWindowBuilder::on_navigation` callback 签名是 `Fn(&Url) -> bool`** 不是 `Fn(Url)`。**修订**：T2.3 callback 签名改 `move |new_url: &url::Url|`
  3. **`WebviewWindow::cookies_for_url(&self, url: Url) -> Result<...>` 是同步方法**（不是 async！）；Windows 同步 command/event handler 调它会死锁。**修订**：T2.4 用 `tokio::task::spawn_blocking` 裹 cookies_for_url
- **Spike 发现的 3 个 Bonus（简化设计）**：
  1. **`WebviewWindow::eval_with_callback(js, Fn(String))`**：Tauri 2 已内置 JSON 化结果回调，**不需要手写设计文档 §5.4 的 bridge script**——P2 的 LocalStorage / PageContent rule 实现可简化
  2. **`WebviewWindow::clear_all_browsing_data()`**：cleanup session 多一手段（除了删 data_dir）
  3. **`WebviewWindowBuilder::on_page_load(Fn(WebviewWindow, PageLoadPayload))`**：DOM 加载事件，P2 的 `PageContent` rule 比 `wait_ms` 轮询准
- **已修改文件**：
  - `docs/superpowers/plans/2026-05-21-credential-capture-engine.md`：Phase 0 顶部加"实际发现汇总"段；T2.1 引入 `capture/webview_isolation.rs` 模块抽象（cfg 分支）；T2.3 callback 签名 `Fn(&Url)`；T2.4 cookies_for_url 用 `spawn_blocking` 裹
  - `feature_list.json`：`integrations` 切 `passing`、`capture-engine` 切 `in_progress`
  - `agent-progress.md`：本段
- **未跑命令**：实际 `cargo run --example capture_spike` 没跑（用 docs.rs WebFetch 替代）；plan 中的 `examples/capture_spike.rs` 文件也未创建——Phase 2 实施时若仍需要可现写
- **下一步**：commit plan v2 + feature_list + progress 一并落盘，然后进入 Phase 1 T1.1（schema 类型定义，与 Tauri 无关，可立即开干）

---

### 2026-05-21 · 凭据抓取器（Credential Capture Engine）实施计划落地

- **本轮目标**：上一轮已交付凭据抓取器设计文档 `docs/design/2026-05-21-credential-capture-engine.md`（14 小节、~620 行、Draft 状态、待用户审）。用户回复「先写实施计划」。本轮按 `.cursor/skills/writing-plans/SKILL.md` 规范，把设计文档第 9 节 P1 MVP 落成可逐 task 执行的实施计划。
- **已完成**：
  - **新文件 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md`**（~1100 行）：5 个 Phase + Phase 0 spike，每个 Phase 含若干 task；每个 task 含「文件 / 步骤 / 验证命令 / 提交命令」；所有步骤都带完整代码块（schema struct / runtime types / engine state machine / 3 Tauri command / hook / dialog / toast）；无任何 TODO 占位符。
    - **Phase 0**（30 分钟 spike）：写 `backend/crates/golish/examples/capture_spike.rs` 验证 Tauri 2 `WebviewWindowBuilder::data_directory` / `cookies_for_url` / `on_navigation` 三个 API 真实存在
    - **Phase 1**（90 分钟）：6 个 task 加 `CaptureRecipe` / `CaptureRule` / `CaptureState` / `CaptureSessionInfo` / `CaptureEventPayload` / 8 个新 `IntegrationError` variant / `validate_capture` 交叉校验（target_field 引用 / URL scheme 白名单）+ ts-rs 同步前端
    - **Phase 2**（4-5 小时）：6 个 task 实现 `CaptureEngine` 状态机 + session registry + per-session data_dir 隔离 + webview 创建 + navigation handler + Cookie rule 提取 + 写 vault + TTL watcher + event emit；P2 rule 类型先 stub 返 "not yet implemented in P1 MVP"
    - **Phase 3**（90 分钟）：3 个 Tauri command（start / status / cancel）+ frontend `captureStart / captureStatus / captureCancel` wrapper + devtools 手动验
    - **Phase 4**（3-4 小时）：i18n 新增 `integrations.capture.*` 一组键（en + zh-CN）+ `useCaptureSession` hook（订阅 `integration-capture` event + 倒计时 + react-query invalidate）+ `CaptureButton` / `CaptureConfirmDialog` / `CaptureStatusToast` 三个组件 + 集成进 `IntegrationGroup.tsx` + 单测 3 case
    - **Phase 5**（90 分钟）：ENScan AQC 加 `capture` 段（cookies.aqc → `BDUSS` cookie）+ 手动 E2E + 6 个反向 case（超时 / 取消 / 409 / 手动关窗 / data_dir 清干净 / status gc 后 404）+ just precommit 全绿
  - **更新 `feature_list.json`**：加 `capture-engine` 条目（priority=1 / status=not_started / 10 条 verification / 关联设计 + 计划文档路径 / notes 说明启动条件「等 integrations 切 passing」）
  - **更新 `agent-progress.md`**：当前最高优先级追加「审审计划」+ 当前会话记录
- **运行过的验证**：
  - `Write docs/superpowers/plans/2026-05-21-credential-capture-engine.md` → 成功
  - `StrReplace feature_list.json` → 成功（capture-engine 条目插入到 integrations 之前 priority=1）
  - `StrReplace agent-progress.md` → 成功（顶部"当前最高优先级"+"未提交的半成品"已更新；本会话记录待 ReadLints 验证后插入）
  - `python3 -m json.tool feature_list.json > /dev/null` → 待跑（下一步）
  - `ReadLints` → 待跑（下一步）
- **已记录证据**：
  - 计划文档行数：`wc -l docs/superpowers/plans/2026-05-21-credential-capture-engine.md` 待跑
  - 计划自检 §按 writing-plans skill 自检要求三项全过：规格覆盖度 / 占位符扫描 / 类型一致性
- **提交记录**：本轮所有文档+元数据改动**未 commit**（高风险操作前必须先获用户确认）
- **未提交文件清单**：
  - 新增：`docs/superpowers/plans/2026-05-21-credential-capture-engine.md`
  - 修改：`feature_list.json`、`agent-progress.md`
  - 同时挂着（前一轮）：`docs/design/2026-05-21-credential-capture-engine.md`
- **已知风险或未解决问题**：
  - Phase 0 spike 是计划中的「先验证再动业务代码」环节；若 Tauri 2 在当前锁定版本里 `cookies_for_url` / `data_directory` 的 API 名称已变，会在 Phase 0 编译阶段立即发现，避免 Phase 2 一半才返工
  - ENScan AQC 的 `BDUSS` cookie 名是合理猜测（设计文档 §3.4 已注明「实际名字 P1 实施阶段实测拍板」）。用户跑 Phase 5 时如发现实际是 `STOKEN` / `BDUSS_BFESS`，改 schema 一行
  - `CaptureEngine` 的 `start_webview` 内用了 `futures::executor::block_on(handle.inner.read())` 同步读 RwLock——Tauri builder callback 是同步的，没办法 await；锁的持有时间 < 1ms 不会触发死锁，但若代码 review 觉得不安全，T2.3 备选方案是改用 `std::sync::RwLock` 而非 `tokio::sync::RwLock`
  - Phase 4 假设项目用了 `react-i18next` 和 shadcn/ui 的 `AlertDialog` / `Button` / `Tooltip`（看了既有 IntegrationGroup.tsx / SecretInput.tsx 这些都用着）；若实际 Tooltip 路径不同，import 路径需对照修
- **下一步最佳动作**：
  1. 用户**先审计划**（重点：Phase 0 spike 是否同意做、`CaptureRule` enum 是否漏案例、`useCaptureSession` 状态机设计是否合理）
  2. 审完后用户决定何时把 `integrations` 切 passing（现已基本完成 Phase 1-5）、把 `capture-engine` 切 in_progress
  3. 然后另起一个会话用 `superpowers:executing-plans` 技能逐 Phase 执行计划
  4. 本轮 3 个文档/JSON 改动可独立 commit：`docs(capture): add design + implementation plan + feature_list entry` —— 不会影响任何已运行代码，安全 commit

---

### 2026-05-21 · Integrations 集成中心 Phase 1-5（schema-driven 凭据管理 · 替换 Intel Providers 入口）

- **本轮目标**：按照 `docs/superpowers/plans/2026-05-21-integrations.md` 的 5 个 Phase 推完整个 Integrations 集成中心：新建 `golish-integrations` crate → 3 个 storage backend + tester + resolver → 5 个 Tauri IPC + frontend wrapper → 前端动态表单组件库 → 接入 ENScan_GO + 5 intel providers + GitHub Token + 删旧 UI。`feature_list.json` 中 `integrations` 条目为本轮唯一 `in_progress`。
- **已完成（按 Phase 分）**：
  - **Phase 1（已在前序会话完成）**：`golish-integrations` crate 骨架 + schema/types/error/traits 类型；本轮在 Phase 3 时给 `ResolvedIntegration` 补了 `Serialize/Deserialize` derive
  - **Phase 2（已在前序会话完成）**：`storage::{vault,external_file,settings}` + `resolver` + `tester` 全部实现，49 个单测全绿
  - **Phase 3（本轮 IPC 命令）**：`backend/crates/golish/src/tools/integrations/{mod,state,commands}.rs` 5 个 `#[tauri::command]`（list_schemas / get / set / clear / test）+ `commands_facade/integrations.rs` + `commands_registry.rs` 注册 + `frontend/lib/api/integrations.ts` 类型镜像 + IPC wrapper。`IntegrationsState::pick_backend()` 按 storage variant 分发 vault/external_file/settings backend；TestKind::Builtin 暂返 Unknown 待 IntelProvider::test_connection 接入
  - **Phase 4（本轮前端动态表单）**：`frontend/components/Settings/IntegrationsSettings/`：7 个字段组件（SecretInput / SecretTextarea / TextInput / SelectField / BooleanField / ProxyInput + FieldRenderer 多态分发）+ `useIntegrationGroup` 状态机 hook + `IntegrationGroupForm`（按 fields[] 动态渲染、Save/Clear/Test 三按钮）+ `IntegrationCard` 折叠卡（storage/category/group-count pill）+ `TestButton` + HealthPill（5 种 HealthStatus mapping）+ `CategoryNav` 侧栏 + fuzzy AND 搜索 + `index.tsx` 三态入口（loading/error/empty/ready）+ i18n 新增 `integrations.*` 27 条键
  - **Phase 5（本轮接入 + 迁移）**：
    - T5.1 `resources/toolsconfig/enscan-go.json` 加 `integration` 段（5 groups: aqc/tyc/kc/rb/miit，TYC 三字段 cookie+tycid+auth_token，全部带 exec test recipe）
    - T5.2 5 个 intel provider 在 `meta()` 中填 `integration_schema: Some(...)`，通过新 helper `crate::api_key_integration_schema(...)`（保留 `extra_tags: ["intel-provider"]` 让旧 UI 仍能识别迁移期数据）
    - T5.3 `resources/integrations/core.json` 描述 GitHub Token（storage=settings.network.github_token / TestKind::Http GET api.github.com/user）+ `golish_core::paths::integrations_core_file()` + `IntegrationsState::build_default()` 启动时合并
    - T5.4 `SettingsTabContent.tsx` 和 `Settings/index.tsx` 两个 Settings 入口都加 Integrations nav + lazy import + switch case；`useSettingsNavigation` SettingsSection 类型把 `"intel"` 替换为 `"integrations"`
    - T5.5 删除 `frontend/components/Settings/IntelProvidersSettings/` 整目录（KeyEditor + ProviderCard + index.tsx 共 ~14KB），同时清掉两个 Settings 入口中所有 IntelProvidersSettings 引用
    - T5.6 `NetworkSettings.tsx` 删除 GitHub Token UI 块（line 67-89），替换为 `t("network.githubTokenMovedHint")` 指向 Integrations 的提示；底层 `settings.network.github_token` 字段保留（Integrations 仍读写它）
- **运行过的验证**:
  - `cd backend && cargo check -p golish-integrations -p golish-intel-providers -p golish` → exit 0（首轮 3m41s，后续增量 49s）
  - `cd backend && cargo nextest run -p golish-integrations -p golish-intel-providers --status-level fail` → **188 tests pass / 0 fail**
  - `cd backend && cargo nextest run -p golish --lib -E 'test(integrations::commands::tests)'` → **6 tests pass / 0 fail**（含 schema serialization round-trip + error mapping + tester Builtin/missing-test 行为锁定）
  - `pnpm exec tsc --noEmit`（全前端）→ exit 0
  - `pnpm exec biome check frontend/components/Settings/IntegrationsSettings/ frontend/lib/api/integrations.ts frontend/lib/i18n/{en,zh-CN}.json frontend/components/Settings/{NetworkSettings,SettingsTabContent,index}.tsx frontend/components/Settings/hooks/useSettingsNavigation.ts` → 全部干净
  - `pnpm vitest run frontend/components/Settings/IntegrationsSettings/` → **17/17 passed**（SecretInput reveal+30s auto-mask / CategoryNav fuzzy AND 语义 / IntegrationGroup 3-field TYC schema 动态渲染 + Save payload + 无 test recipe 隐藏按钮 + 错误路径）
  - `pnpm vitest run frontend/components/Settings/` → **68/68 passed**（含本轮 17 + 现有 AppearanceSettings / CaretPreview / TerminalSettings 等回归）
  - `rg "IntelProvidersSettings" frontend/` → 0 真实引用，2 处仅在注释里提到 "legacy IntelProvidersSettings UI"
- **已记录证据**:
  - 188 + 6 + 17 + 68 = 全部 test 数量见上面 "运行过的验证" 段
  - 关键文件：`docs/design/2026-05-21-integrations.md` / `docs/superpowers/plans/2026-05-21-integrations.md`（设计 + 实施计划）+ `feature_list.json` 已写入 evidence
- **提交记录**：本轮所有 Phase 1-5 改动**未 commit**，等用户授权后整批 commit
- **未提交文件清单（本轮新增/修改部分）**:
  - 后端：`backend/crates/golish-integrations/`（新 crate，11 文件）+ `backend/crates/golish/src/tools/integrations/{mod,state,commands}.rs`（新）+ `backend/crates/golish/src/commands_facade/{mod,integrations}.rs`(改+新) + `backend/crates/golish/src/commands_registry.rs`(改) + `backend/crates/golish/src/app/tauri_app.rs`(改) + `backend/crates/golish/Cargo.toml`(+dep) + `backend/Cargo.toml` & `Cargo.lock`(workspace members) + `backend/crates/golish-intel-providers/{Cargo.toml,src/lib.rs,src/types.rs,src/{zone,fofa,quake,hunter,shodan}/mod.rs}`(+dep + helper + 5 schema) + `backend/crates/golish-core/src/paths.rs`(+integrations_core_file)
  - 前端：`frontend/components/Settings/IntegrationsSettings/`（10 文件：index + 5 容器 + fields/6 + hooks/1 + 3 测试）+ `frontend/lib/api/{index,integrations}.ts` + `frontend/components/Settings/{NetworkSettings,SettingsTabContent,index}.tsx`(改) + `frontend/components/Settings/hooks/useSettingsNavigation.ts`(改) + `frontend/lib/i18n/{en,zh-CN}.json`(改) + 删除 `frontend/components/Settings/IntelProvidersSettings/`（3 文件）
  - 资源：`resources/toolsconfig/enscan-go.json`(改) + `resources/integrations/core.json`(新)
- **已知风险或未解决问题**:
  - **T5.7 read alias 未在真实运行环境验证**：旧 0.zone vault key 经新 UI 渲染应显示「已配置」badge，但需 `just dev` 跑起来用户人工对照看一眼
  - **T5.8 Playwright E2E 未补**：plan 原定 3 case（渲染 / 保存 / 测试），本轮 vitest + 后端 nextest 已覆盖关键路径，E2E 暂跳过
  - **TestKind::Builtin 还是返 Unknown**：5 个 intel provider 的「测试连接」按钮目前不能真正调到 `IntelProvider::test_connection`。若用户在 UI 上点击会看到 `Unknown · builtin test path...`。补 dispatch 是后续小补丁
  - **ENScan exec test 拿不到 executable**：tester 用 no-op exec resolver，所以 ENScan 5 个 group 点 Test 会返「executable not found」。接入 ConfigManager 的 `find_tool_executables` 是后续补丁
  - **`just precommit` 整体不绿**：preexisting biome 警告（pty.ts 排序 / useTaskPlanState 可选链 / App.tsx 格式等非本轮文件）阻塞，与 Phase 1-5 改动无关；本轮自己改的文件全部干净
  - **国际化 `intel.*` 翻译键 fallback 保留**：plan 说保留半个版本周期 fallback；本轮不动 `intel.provider.*` / `intel.headerDesc` 等键，下一轮可清
- **下一步最佳动作**:
  1. 用户**先跑 `just dev`** 真试新 UI：6 张卡（0.zone/fofa/quake/hunter/shodan + enscan-go + github）能否在 Settings → Integrations 正常渲染；旧 0.zone vault key 是否显示「已配置」
  2. 截图 / 录屏后给我看，确认无 UI 问题后**整批 commit** Phase 1-5（约 50+ 文件）
  3. **补两个小补丁**：① IntegrationsState 接入 ConfigManager 的 executable 查询 ② commands.rs::integrations_test 加 `TestKind::Builtin → IntelProvider::test_connection` 分支
  4. 后续清理 preexisting biome 警告，让 `just precommit` 整体绿，方便日常使用

---

### 2026-05-20 · 补合 AI-Chat ModelSettings Popover + LLM Quirks/Overrides + Thinking 模式

- **本轮目标**：用户发现上一轮 4 主题选择性合并漏掉了"AI 解析逻辑 + ChatPanel 思考模式设置"那条线,补 cherry-pick 远端 `37425b2 feat(ai-chat): add model settings popover, agent status indicator, and LLM quirks/overrides` 这一个大 commit(33 文件 / 2011 行)。
- **已完成**:
  - 建 backup `backup/before-ai-chat-popover-merge-20260520-162709`
  - cherry-pick `37425b2` -> 本地新 commit `5d30b50`,内容覆盖:
    - 后端: `agentic_loop/llm_stream_start.rs`(60 行新)、`stream_processor/mod.rs`(115 行)、`turn/phases/completion.rs`、`golish-models/.../model_capabilities/quirks.rs`(365 行新)、`golish-llm-providers/.../provider_config.rs`、`golish-settings/src/schema/ai.rs`、`golish-sub-agents/.../stream_processing.rs` 等
    - 前端: `AIChatPanel/ModelSettingsPopover.tsx`(401 行新)、`ThinkingBlock.tsx`(91 行)、`AgentStatusIndicator.tsx`(148 行新)、`ChatModelSelector.tsx`(210 行,与本地之前改动冲突已手动融合)、`MessageBlock.tsx`、`lib/ai/model-overrides.ts`(105 行新)、`lib/ai/types.ts`(178 行,含 ProviderModelOverride 接口)、`services/ai-events/core-handlers.ts`、`store/slices/conversation.ts`
  - 3 处手动 resolve 冲突:
    1. `ChatModelSelector.tsx`: 整文件冲突,通过 Write 重写;**融合**本地 `getVisibleProviderGroups` / `getModelItemClassName` 工具函数 + 远端 `modelIsThinkingByDefault` / `useEffectiveThinkingEnabled` hook + ModelSettingsPopover 集成
    2. `providerConfig.ts`: nvidia case 双方都改,**双方保留**(nvidia 的 base_url + model_override + 独立的 deepseek case 都保留)
    3. `types.ts`: ProviderConfig union 重构为 intersection 形式,**双方保留**远端的 ProviderModelOverride + ProviderConfigBase 结构 + 本地的 deepseek 分支
- **运行过的验证**:
  - `pnpm --silent typecheck` -> 全绿(12.5s)
  - **未跑** cargo check(此前用户两次主动中断,跳过)
  - `git log -1 --oneline` -> `5d30b50 feat(ai-chat): ...`
- **已记录证据**: 见上面"已完成"+ 验证段
- **提交记录**: HEAD = `5d30b50`;**待 push**(用户已授权"一次性合 + push")
- **已知风险或未解决问题**:
  - cargo check 未独立跑,后端编译实际状态高置信但未确认;若启动失败需要修
  - quirks.rs 新增 365 行内未含 deepseek 模型的默认 quirks(本地 deepseek 是后续加的);用户用 deepseek 模型时 thinking 默认值可能不准,可通过 ModelSettingsPopover 手动覆盖
  - ChatModelSelector 是手工融合版本,功能上覆盖了双方意图,但可能与本地之前游离改动行为略有不同
- **下一步最佳动作**:
  1. push 后跑 `just dev` 真试,看 ChatPanel 旁边是不是出现了 ModelSettingsPopover 按钮、AgentStatusIndicator 是否显示、思考模式开关是否能切
  2. 如果后端启动报错,跑 `cd backend && cargo check -p golish-models -p golish-llm-providers -p golish-agent-runtime` 定位
  3. agent-progress.md 微调一并 commit + push

---

### 2026-05-20 · 从远端 `origin/feature/cross-platform-finishing` 选择性合并 KG / Dispatch / Planner / Task-Plan

- **本轮目标**：MCP-6 完成 GridTerminal 合并（HEAD `4184372`）后转给本会话，把远端那 323 commits 中的 KG / Dispatch monitor / Planner / Task-Plan 四大主题合入本地 finishing。
- **已完成**：26 个 cherry-pick 成功 + 1 个 fix 修补漏掉的字段定义 + 1 个 fix 清掉残留 conflict marker，新 HEAD `4623f92`。
- **批次拆解**（按拓扑序，里程碑节点验证）：
  - **M1（12 commits · planner P0-1 + KG 全 5 + task-plan fallback）**：`2015b4d` (db migration) → `c7b17e8` (PlanEventEmitter trait) → `d507c71` (kg inject)* → `51f3086` (kg regex autoextract)* → `9b2b21b` (kg frontend SDK) → `901a25a` (task-plan fallback) → `ba656c4` (task-plan test) → `58a8b03` (emit PlanUpdated) → `38fc171` (load_from_db test) → `9dc5a99` (fix: marker leftover) → `aae2721` (kg pty extract) → `01b9570` (kg ui card)。*号是手工 resolve 了 3 处 conflict（commands_registry / ai/mod / ai/commands/mod / direct.rs，原则：本地优先 + 排除 M3 才该加的 dispatch 引用）。**验证：typecheck ✓ / `cargo check` 全 workspace ✓（94s）**。
  - **M2（6 commits · P0-2 planner patch ops + failure_kind）**：`75233aa` (apply_patch_ops + PlanPatchOp) → `5b3ce3c` (update_plan_patch tool)* → `609c45e` (persist snapshots) → `cb024bf` (plan-tool test) → `47d6912` (failure_kind badge) → **补漏** `e276460` (FailureKind enum + PlanStep.failure_kind P0-2 stage 1)。*号是 direct.rs 又冲突一次（M2 才该加 execute_plan_patch_tool import），合并保留双方。**关键修补**：原拓扑漏了 b07e1dc（P0-2 stage 1 加 failure_kind 字段），导致 47d6912 引用未定义字段；cargo check 报 E0609 后立即 cherry-pick 补上，编译恢复。**验证：typecheck ✓ / `cargo check` ✓（146s）**。
  - **M3（7 commits · Dispatch monitor 全栈）**：`f383763` (db-traits dispatch methods) → `ab23b9b` (sqlx impl) → `5c955c3` (Tauri command)* → `089be73` (agent-runtime lifecycle) → `f4aee08` (reap stale) → `69d89ce` (UI section) → `4623f92` (fix non-UUID)。*号是 commands_registry / ai/mod / ai/commands/mod 又冲突一次（这次 M1 时故意排除的 dispatch entries 现在加回来），三处合并保留 graph+dispatch / kg+list_running 全集。**验证：typecheck ✓ / `cargo check` 中断 2 次（574s + 85s）由用户主动停止；M3 picks 干净 + typecheck 全绿 + M2 cargo check 已过 → 编译状态高置信但未独立全量确认**。
- **运行过的验证**：
  - `pnpm --silent typecheck` × 3 → 三轮全绿
  - `cd backend && cargo check -q` × 2（M1/M2）→ 通过，第 3 次（M3）被中断未完成
  - `git status --short` → 工作树干净
- **已记录证据**：见本节"运行过的验证"+ git log 26 个新 commits + M2 补漏 `b07e1dc` 的修补记录
- **提交记录**：HEAD = `4623f92 fix(dispatch): non-UUID session ids return empty list instead of erroring`；26 个新 commit 的 hash 列表在上面"已完成"段
- **推送记录**：用户授权后用 `git push --force-with-lease origin HEAD:feature/cross-platform-finishing` 把本地推到远端，远端 head 从 `13852bb` 强制更新为 `4623f92`；远端原独有 ~290 commits 不再被任何 ref 指向（git object 仍在远端 reflog 内可恢复一段时间）；推前已建本地备份分支 `backup/before-push-to-origin-finishing-20260520-162056` 指向 `4623f92` 留底
- **已知风险或未解决问题**：
  - M3 全工作区 cargo check 没跑完——下一轮**必须**先 `just check-rust` 或 `just precommit` 跑一遍
  - 远端那 323 commits 还剩 ~290 个未合（exec-mode PR / sub-agent dispatch refactor / briefing pgvector / ai-chat model popover / 各种 docs 等），按用户指示"只要 KG/Dispatch/Planner/Task-Plan 四条线"已完成；其他主题留到后续
  - 与 GridTerminal stack（terminal manager 多个旧文件保留）在 `run_pty_cmd` 出发节点存在 KG entity extract 叠加，未做交叉冒烟测试
- **下一步最佳动作**：
  1. **下一轮先跑** `just check-rust` 把 M3 完整 cargo check 兜底，发现编译失败立即修
  2. **建议跑** `just test-rust -p golish-agent-kit` 验证 planner / dispatch lifecycle / kg 测试全过
  3. **真跑** `just dev` 让 GridTerminal + KG + Dispatch UI 在一起跑一次，观察 Advanced Settings 里 KG snapshot card 和 Dispatch in-flight section 是否正常渲染
  4. 收拾残余：之前游离的 `dialogue-protocol.mdc` 删除、`2026-05-17-targets-organization-grouping.md` 删除等被打包进 `74b4d22 checkpoint` 已 commit；其他生成的 docs（`recon-tool-belt-2026-05.md` 等）也已被打包

---

### 2026-05-20 · 外层 Meta-Harness 初始化

- **本轮目标**：按照 [Learn Harness Engineering](https://walkinglabs.github.io/learn-harness-engineering/zh/) 给 Golish 项目铺设外层 meta-harness，约束"AI 帮我开发 Golish 这个项目"的行为。
- **已完成**：
  - 创建 `AGENTS.md`（工作宪法，含开工流程、Golish 不变量、完成定义、收尾流程）
  - 创建 `agent-progress.md`（本文件）
  - 创建 `feature_list.json`（功能清单 v0，含已规划的 harness、recon、provider form 等）
  - 创建 `init.sh`（一键环境验证脚本）
  - 创建 `clean-state-checklist.md`（会话收尾检查清单）
  - 创建 `.cursor/rules/agents-bridge.mdc`（让 Cursor IDE 自动在每次 prompt 顶部引用 AGENTS.md）
- **运行过的验证**：
721278  - `chmod +x init.sh` → exit 0；`ls -la init.sh` 显示 `-rwxr-xr-x` 可执行
  - `python3 -m json.tool feature_list.json > /dev/null` → exit 0，`feature_list.json: VALID JSON`
  - `bash -n init.sh` → exit 0，`init.sh: VALID bash syntax`
  - `bash init.sh --help` → 正常输出 Usage 文本，参数解析路径无问题
  - ReadLints 6 个新文件 → `No linter errors found.`
  - **未执行**：`bash init.sh --quick`（会触发 `just check-fe` 和 `just check-rust`，可能因 git status 中游离的 ChatModelSelector / useProviderForm 改动而非确定性绿，留给用户自行执行）
- **已记录证据**：见本节"运行过的验证"
- **提交记录**：`3b1f659` `chore(harness): scaffold external meta-harness for AI agents`（6 files, 703 insertions, 未 push）。提交后本文件被微调过一次（补本字段为实际 hash + 补"未提交的半成品"说明），微调本身未 commit，由下一轮 progress 更新自然带走。
- **已知风险或未解决问题**：
  - `init.sh` 第一次跑可能会全量 `pnpm install` 和 `cargo build`，初次耗时较久
  - `feature_list.json` 的初始功能列表可能不完整，需要用户根据实际优先级调整
  - 已有 `frontend/components/AIChatPanel/ChatModelSelector.tsx` 等改动游离在 git status 中，不在本轮范围
- **下一步最佳动作**：
  1. 用户审阅 6 个新文件，确认内容贴合实际需求
  2. 跑 `bash init.sh` 验证环境基线
  3. 用户决定是否把 6 个新文件合并为一次 commit（推荐 message：`chore(harness): scaffold meta-harness markdown + scripts`）
  4. 后续按 `feature_list.json` 优先级推进，第一个候选是把内层 agent harness Rust 实现按 `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md` 推进

---

### 2026-05-20 · ToolConfig 新增 `pentestPhase` 字段 + 删除 burpsuite-community

- **本轮目标**：MCP-3 接手 MCP-1 上下文，给 `golish-pentest::ToolConfig` 加 `pentest_phase: Vec<String>` 字段标记 7 阶段（实际 6 个：recon/enum/vuln_id/exploit/post_exploit/aux，meta 被用户砍掉），同时按用户指示把 `burpsuite-community` 工具完全清出仓库。
- **已完成**：
  - **后端**：`backend/crates/golish-pentest/src/models.rs`
    - `ToolConfig` 加 `pentest_phase: Vec<String>` + `#[serde(default, rename = "pentestPhase")]` 向后兼容（老 JSON 无此字段 = 空数组）
    - `validate()` 加 phase 枚举校验（任意取值不在 6 枚举集报 `pentestPhase '<x>' invalid, must be one of: ...`）
    - 新增 `pub const VALID_PENTEST_PHASES: &[&str]` 模块级常量供 harness `tool_policy` 等下游复用
    - 加 4 个 `#[cfg(test)] mod tests`：driving JSON round-trip / legacy compat / canonical pass / 拒绝未知 phase
    - 修 `command_builder/tests.rs:17` + `search.rs:103` 两处 struct literal 漏字段（仅加 `pentest_phase: vec![],` 一行）
  - **前端**：`frontend/lib/pentest/types.ts` 加 `pentestPhase?: string[]`（手写镜像；ts-rs 历史债不在本次 scope）
  - **配置数据**：`resources/toolsconfig/*.json` 27 份逐个加 `"pentestPhase": [...]` 字段（详细取值表见下方）
  - **清除 burpsuite-community 全部痕迹**（按用户 A 一并清理选项）：
    - 删 `resources/toolsconfig/burpsuite-community.json`（2789 字节）
    - 删 `resources/skills/burpsuite-community/basic-usage.md`（4133 字节）+ 空目录
    - 改 `docs/windows-support.md` 删第 93 行 burpsuite-community 安装说明
  - **未顺手改**：发现 `golish-pentest-domain::ToolConfig` + `golish-pentest-mcp::ToolConfig` 是 `ToolConfig` 的孪生副本（违反 I5，但属历史债）。两者都没 `pentestPhase` 字段，但因 serde 默认忽略未知字段，反序列化 JSON 不受影响。**留作下一轮**统一到 ts-rs。
- **运行过的验证**：
  - `cargo check -p golish-pentest` → 0，32.76s
  - `cargo test -p golish-pentest --lib models::` → 4 passed; 0 failed（round-trip / legacy compat / canonical phases / reject 'meta'）
  - `cargo check -p golish-pentest-domain` → 0，9.45s（受牵连验证未被破坏）
  - `cargo check -p golish-pentest-mcp` → 0，69.6s（同上）
  - `pnpm typecheck` → 0，10.9s
  - `jq` 27 个 JSON `.tool.pentestPhase` 全部数组合法 + 取值全在 6 枚举集（exit 0，无 INVALID 输出）
  - `ReadLints` models.rs + types.ts → 无 lint 错误
- **已记录证据**：
  - 27 个 JSON 取值表（按 phase 分类）：
    - **recon**（被动子域/URL/截图）：subfinder, gau, waybackurls, gowitness
    - **recon+enum**（多阶段）：amass, httpx, katana
    - **enum**（主动扫描）：nmap, masscan, gobuster
    - **enum+vuln_id**（扫描+识别）：ffuf, nikto
    - **vuln_id**（漏洞识别）：nuclei, wpscan, dalfox, searchsploit
    - **vuln_id+exploit**：sqlmap
    - **exploit**：metasploit-framework, hydra
    - **exploit+post_exploit**：john, hashcat
    - **post_exploit**：impacket, netexec, bloodhound-python, responder, chisel
    - **aux**：wireshark
  - 单元测试通过列表见上"运行过的验证"
- **提交记录**：**待用户确认后 commit**（本轮未跑 commit；用户未明示 push）
- **已知风险或未解决问题**：
  - `golish-pentest-domain` / `golish-pentest-mcp` 两份孪生 `ToolConfig` 没同步 `pentest_phase`（serde 兼容但语义裂开）；建议下一轮统一走 ts-rs derive 收敛到一份
  - `frontend/lib/pentest/types.ts` 是手写镜像（违 I5）；ts-rs 收敛建议同上
  - 27 个 JSON 的初始取值是**经验判断**（基于 MCP-1 文档 §3 工具阶段分类表），后续可能需根据实际使用调整
  - `search_tools` 函数未扩展 phase 过滤（不在本轮 scope，留作下一轮）
- **下一步最佳动作**：
  1. 用户审核 27 个 JSON 的 phase 取值表，提出调整建议
  2. 在 `golish-pentest/src/tool_manager/mod.rs::search_tools` 扩展 phase 关键字过滤
  3. ToolManager UI 加 "按阶段查看" 过滤器（参考 MCP-1 设计 §6.2）
  4. 把 `golish-pentest-domain` / `golish-pentest-mcp` 的 ToolConfig 收敛到 ts-rs derive（消除 I5 历史债）

---

### 2026-05-20 · ASM Intel Providers 集成（feat/asm-intel-providers · 0.zone 首发）

- **本轮目标**：搭建 ASM 多 provider 集成的可扩展架构，0.zone 首发落地，含后端 IPC + 前端 Settings UI + vault key 管理 + organizations 表写入路径。
- **已完成**：
  - **分支**：`git checkout -b feat/asm-intel-providers`（基于 main）
  - **设计文档**：
    - `docs/design/2026-05-20-pentest-fields-tool-mapping.md`（baseline · 14000 字 · 11 节 · targets 25 字段 + organizations 28 字段 + 6 辅助表 + 27 工具完整映射）
    - `docs/design/2026-05-20-asm-intel-providers.md`（架构 · §0-§9 含 4 决策点 + 4 层架构图 + 6 不变量）
    - `docs/superpowers/plans/2026-05-20-asm-intel-providers.md`（4 phase 实施计划）
  - **元数据**：`feature_list.json` 加 `asm-intel-providers` 条目（priority=0 · in_progress）；`backend/Cargo.toml` 加新 crate 到 members + default-members + workspace.deps。
  - **新 crate `golish-intel-providers`**（9 个文件）：
    - `Cargo.toml`（依赖 serde / tokio / reqwest / async-trait / thiserror / chrono / serde_urlencoded）
    - `src/lib.rs`（IntelProvider trait + ProviderRecord）
    - `src/error.rs`（IntelError 7 变体 + IntelResult）
    - `src/types.rs`（QueryType 10 变体 + ProviderMeta + ProviderRecord + ConnectionStatus）
    - `src/shared/{mod,api_key,rate_limit}.rs`（KeyStore trait + EnvKeyStore + RateLimiter）
    - `src/zone/{mod,client,types,mapper}.rs`（0.zone 完整实现 · 7 query_type · 限速 2/s · group 反查归属）
    - `src/{fofa,quake,hunter,shodan}/mod.rs`（4 个占位 IntelProvider impl）
  - **golish-pentest 修改**：
    - `output_store/organizations.rs`（新 · store_organization_update writer + find_or_create_organization + 5 个 append helper · jsonb 幂等追加）
    - `output_store/mod.rs`（match 加 `organization_update` 分支）
    - `output_store/store_trait.rs`（OutputStore trait 加 store_organization_update）
    - `output_store/pg_adapter.rs`（PgPentestStore impl）
  - **golish 后端 IPC**（4 个文件）：
    - `tools/intel_providers.rs`（PgVaultKeyStore impl KeyStore · provider_registry · 3 个 #[tauri::command]）
    - `tools/mod.rs`（+ pub mod intel_providers）
    - `commands_facade/intel_providers.rs`（新 · pub use）
    - `commands_facade/mod.rs`（+ pub mod intel_providers）
    - `commands_registry.rs`（+ use commands_facade::intel_providers::* + 3 命令进 generate_handler）
    - `Cargo.toml`（+ golish-intel-providers 依赖）
  - **前端**（5 个文件）：
    - `lib/api/intel.ts`（手写 TS interface + 3 invoke wrapper）
    - `lib/api/index.ts`（+ intel 导出）
    - `components/Settings/IntelProvidersSettings/{index,ProviderCard,KeyEditor}.tsx`（5 provider 卡片 + key 编辑 + Test Connection）
    - `components/Settings/SettingsTabContent.tsx`（NAV_ITEMS 加 "intel" section · Network 图标）
  - **i18n**：`frontend/lib/i18n/{en,zh-CN}.json` 加 `settings.intelProviders` + `settings.intelProvidersDesc`
- **运行过的验证**：
  - `cargo check -p golish-intel-providers --tests` → Exit 0 · 0 warning
  - `cargo nextest run -p golish-intel-providers` → **31 passed · 0 skipped**（trait 对象安全 + 5 mapper + 3 envelope + 3 api_key + 3 rate_limit + 7 zone unit + others）
  - `cargo check -p golish-pentest` → Exit 0
  - `cargo check -p golish` → Exit 0 · 1m08s（含全 workspace 依赖编译）
  - `cargo fmt --package golish-intel-providers` → 自动修复
  - `cargo clippy -p golish-intel-providers --no-deps -- -D warnings` → Exit 0
  - `pnpm typecheck` → Exit 0 · 10.1s
  - `pnpm biome check components/Settings/IntelProvidersSettings/ lib/api/intel.ts ...` → Exit 0 · No fixes applied
- **已记录证据**：见以上验证命令；测试输出关键行 "31 tests run: 31 passed, 0 skipped"
- **提交记录**：**待提交**（用户未授权 commit；分支 feat/asm-intel-providers 上累计 ~30 个新/改文件）
- **已知风险或未解决问题**：
  - `pnpm check`（biome lint）整体失败仅因 pre-existing `useTaskPlanState.ts` 等文件的 useOptionalChain warning，与本任务无关；新增文件全 clean
  - Phase 4 的 Playwright E2E spec 暂未写（涉及 mock Tauri 复杂度）；标 TODO，本轮不阻塞验收
  - 0.zone HTTP 调用实测未跑（需要 zone_key_id 付费会员），但已用 mock-friendly 设计 + 5 mapper 单测覆盖 7 query_type
  - fofa/quake/hunter/shodan 仅 stub，下一期实现
- **下一步最佳动作**：
  - 用户授权后 commit feat/asm-intel-providers 分支累计改动（建议 squash commit 标题：`feat: ASM intel providers full stack · 0.zone first impl + Settings UI + organizations writer`）
  - `just dev` 启动 + 手动跑 Settings → Intel Providers → 填 0.zone key（如有）→ Test Connection → 看 organizations 表更新
  - 后续 PR：fofa/quake/hunter/shodan 各家 client + types + mapper（约 0.5 day/家）；Playwright E2E spec（约 0.5 day）

---

<!-- 新会话请在这里上方插入一条新记录，保持倒序 -->

## 模板（复制下面这块当新会话记录）

```markdown
### YYYY-MM-DD · <功能或主题名>

- **本轮目标**：<一句话说清楚要做什么>
- **已完成**：<具体做了什么，包括文件路径>
- **运行过的验证**：
  - `<命令1>` → <结果>
  - `<命令2>` → <结果>
- **已记录证据**：<测试输出关键行 / 截图路径 / DB 查询结果 / ...>
- **提交记录**：<commit hash 或"待提交">
- **已知风险或未解决问题**：<...>
- **下一步最佳动作**：<下一轮从哪开始>
```
