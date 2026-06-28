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
| **未提交的半成品** | **2026-06-15（MCP-agent-1）：0.zone apk 微信公众号映射补全——已 commit `3ea3466d`（8 文件：0.zone http partial-success/retry [与 MCP-3 entangled，含 http.rs/models] + wechat 映射 0-zone.json/profile_patch.rs/org-fields.ts/测试；未 push）。i18n `en/zh-CN.json` 的 wechat 标签因与他会话「assets」hunk 共文件**未纳入**（feature 经英文回退仍显示，中文标签待协调提交）。TDD 全绿（nextest asset_intel 66/66 + clippy -D 零告警 + 前端全量 1322 + check-fe exit 0），未跑全量 precommit。详见会话记录最新一条。** **2026-06-14（MCP-agent-2）：engagement 总览/扇出全栈移除 + stage-run 闭环——47 项改动未 commit（branch `feat/stage-run-fanout`）；完整 `just check` 全量全绿（`cargo clippy --workspace -- -D warnings` 0 告警 + `cargo nextest --workspace` 3269 passed/7 skipped + check-fe/test-fe 绿）。详见会话记录最新一条。** **2026-05-30：架构优化批已拆 9 commit 落 `feat/recon-service`（`98beea9`→`6aaa0fb`，HEAD `d060ce4`）。** 其上叠了 **P0-3b 残余作用域 SQL 下沉**（T1-T6 全部完成，**未 commit**）：26 个 tracked 文件改动 + 6 个新 repo 模块（untracked：`repo/{scan_queue,sensitive_scan,conversation_store,directory_entries,sitemap_store,custom_rules}.rs`）。验证：rg 命令层裸作用域 SQL 清零、`golish-db` nextest 46/46、`golish --lib` nextest 318/318、`clippy golish-db+golish` 全绿，并跑通**全栈 `just precommit` → `✓ All checks passed!`（exit 0）**（含用户授权后修的 1 个 pre-existing `integrations/commands.rs:179` baseline）。**已按拆分提交 4 个 commit**（`65e0292`/`06af27a`/`d023386`/`c2f5ad2`，落 `feat/recon-service`，未 push）。**2026-05-30 续（MCP-2）：P2 拆分①完成——`golish-pentest-domain/src/models.rs`(1310) 模块化为 module-root + `models/{tool_config,asset_intel,runtime,tests}.rs`（全 < 500 行），全验证通过（crate check/nextest 17✓/clippy `-D warnings`/`cargo check --workspace` 全绿），**未 commit**（`M models.rs` + `?? models/`）。P2 拆分②完成——`golish/src/tools/pentest_bridge/js_collect.rs`(1357) 模块化为 module-root + `js_collect/{extract,judge,quality,sitemap,tool_impl,tests}.rs`（全 < 500 行，max 470），全验证通过（`cargo check -p golish`/`nextest js_collect` 26✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M js_collect.rs` + `?? js_collect/`）。P2 拆分③完成——`golish/src/tools/integrations/capture/engine.rs`(1483) 模块化为 module-root + `engine/{extract,helpers,tests}.rs`（全 < 500 行，engine.rs 496）；生命周期/webview 方法留 root 避免 super:: 改写，全验证通过（`cargo check -p golish`/`nextest capture::engine` 23✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M engine.rs` + `?? engine/`）。P2 拆分④（进行中）——`frontend/mocks.ts`(4135→2353) 抽出事件系统/AI 模拟/showcase 三层到 `mocks/{event-bus,events,simulations,showcase}.ts`（公共面零变更；`showcase.ts` 1146 仍 >500 待再分），`check-fe`+`test-fe` 全绿；剩余 demos/有状态 ipc 待续。**✅ 已按块 commit**：经 `just precommit` 全绿（`✓ All checks passed!`，~21.7min）后落 5 个 commit 到 `feat/recon-service`（`a71319b` pentest-domain models / `03871db` js_collect / `63c196e` capture engine / `83a105c` frontend mocks / `dd3c367` docs progress，**未 push**）。**2026-05-30 收尾（MCP-agent-2）：本会话架构体检全批（拆/合并/优化/dedup）已 `cargo fmt --all` 后按主题拆 20 个 commit（`a85f7d4`(scripts)→…→ docs(progress)，**未 push**）；提交后工作树 clean。完整 `just precommit` 本轮未重跑（树稍早已全绿，fmt 仅排版）。** **2026-05-30 续（MCP-5 · 接 MCP-3 转交）：S1-1 repo 数据所有权守卫 + check_dag 修复**——已修既有 `golish-graphiti(L1)→golish-db(L2)` DAG 违规（graphiti 归 L2，非删依赖）；`just arch` → **exit 0**（双守卫全绿）。已落 4 commit 到 `feat/recon-service`（`b0811ea`/`dc9ad0f`/`821c101` + 1 docs commit，**未 push**），提交后工作树 clean。feature_list `arch-s1-1-repo-ownership-guard` → **passing**；`just precommit` 未重跑（改动集零 Rust/TS/Cargo diff）。 **2026-05-30 续（MCP-agent-4 数据工程）：S1-2a `VaultReadPort` 走路骨架** —— 另一会话写 Tasks 1-4（端口/迁移/注入），本会话接手 Task 5（守卫拔 ratchet）+ Task 6（文档/feature_list/progress）。改动：`?? golish/src/ports/`(3 文件)、`M golish/src/lib.rs`、`M tools/pentest_bridge/{vault_ops,auth_probe,mod}.rs`、`M scripts/check_repo_ownership.py`、`M docs/architecture.md`、`M feature_list.json`、`M agent-progress.md`、`?? docs/{design,plans}/2026-05-30-s1-2-*`。验证：`cargo check -p golish` exit 0、`just arch` exit 0（ALLOWLIST **30→28**）、guard OK clean、`rg golish_db::repo::vault` 于 pentest_bridge 空。**2026-05-30 续（MCP-agent-3 后端工程，用户授权 C: A+B 一气呵成）**：跑 `cargo nextest -p golish ports::platform::vault` → **1 passed/373 skipped exit 0**（4m53s 冷编译）+ `just precommit` → **✓ All checks passed! exit 0**（29.6 min · fmt+check-fe+test-fe+lint-rust+test-rust-all 全绿）；按 plan 拆 **6 commit 落 feat/recon-service**：`6abaec8`(feat 端口骨架,4f+118)/`1e162de`(refactor VaultTool,1f)/`1a7018b`(refactor AuthProbeTool,1f)/`1149ddb`(refactor 构造点注入,1f)/`389d3fd`(chore 拔 ratchet,1f) + `23e47a6`(docs S1-2 design+plan+architecture+feature_list+progress,5f +947-3)；**未 push**，本地 ahead 10。**2026-05-30 续 2（MCP-agent-3 · 用户授权"你想怎么搞合适"）**：S1-2 父条目 `arch-s1-2-port-horizontal-coupling` → **passing**（走路骨架确立）；**新增** `arch-s1-2b-recon-port` 条目 `not_started`（等用户审 §10 5 决策再转 in_progress）；**新写** `docs/design/2026-05-30-s1-2b-recon-read-port.md` S1-2b 高层设计（22 条 allowlist 精确清单+grep 实证、6 子片划分 b1-b6、ReconPort trait 25 method 含读+写、守卫配合、5 待拍板决策）；命名差异关键：a 是 ReadPort（read-only），b 是 Port（含写，因 agent-bridge 适配器内有 insert/upsert/update）。新增/修改 3 文件：`?? docs/design/2026-05-30-s1-2b-recon-read-port.md`、`M feature_list.json`、`M agent-progress.md`。**待 commit + 不 push**（push 需用户单独点头，按 AGENTS.md §2.7 红线保守处理）。 **2026-05-30 续（MCP-agent-2）：M1 crate 抽取全套未 commit** —— `?? backend/crates/golish-app-core/`(M0)、`?? backend/crates/golish-vuln-app/{Cargo.toml,src/lib.rs}` + `RM` 19 文件（vuln_intel 8 + wiki 11，git mv 进 golish-vuln-app/src/）、`M backend/Cargo.toml`、`M golish/{Cargo.toml, src/commands_facade/{vuln_intel,wiki}.rs, src/tools/mod.rs, src/error.rs, src/state/db.rs, src/event_emitter.rs}`、`M scripts/check_{dag,repo_ownership}.py`、`M feature_list.json`、`M agent-progress.md`。验证：`cargo check` 两 crate + 双守卫全 exit 0；**未跑 just precommit 全量、未 commit、未 push**。 |

---

## 会话记录

> 倒序排列,最新一轮在最上面。每轮一条。

> 历史会话已归档：`docs/archive/agent-progress-archive-2026-06-28.md`。主文件只保留最近 20 条会话，避免旧日志干扰新判断；需要追溯旧验证证据时去 archive grep。

---

### 2026-06-28 · SubAgent detail DSML 工具调用泄漏清理

- **本轮目标**：回应用户截图问题：“detail 里这一大段是谁的，为什么样式很奇怪”；定位归属并修复 `DSML` 文本工具调用标记泄漏到子 agent 正文的问题。
- **根因/判断**：
  - 这段属于 `stage_run` 下的 EAS `Prober` 子 agent 普通 narrative，不是工具 stdout，也不是主 agent 最终报告。
  - 样式怪有两层：一是多轮 `sub_agent_text_delta.accumulated` 被作为正文连续渲染，读起来像一整块日志；二是 provider 退化出的 `DSML` 文本工具调用（`submit_stage_deliverable` 参数/coverage JSON）没有被 detail 清洗函数识别，直接混进了 Markdown 正文。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：`stripAgentXmlTags` 增加 DSML 伪标签兜底，剥离完整/未闭合的 `tool_calls` / `invoke` / `parameter` 文本工具调用块。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增完整 DSML submit block 和未闭合 DSML streaming block 回归；顺手把该测试里的 session mock `mode` 从旧的 `"chat"` 对齐为当前 `SessionMode` 的 `"agent"`。
  - `docs/modules/frontend/components.md`：同步记录 provider 文本工具调用标记不属于 agent prose，detail 渲染前必须剥掉。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（既有本机 pnpm install/approval gate）。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0，fixed 1 file。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 1 file / 47 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新同一 detail 面板确认 DSML submit 参数不再出现在正文里；大段 narrative 仍会按子 agent 输出展示，只是内部工具调用标记会被剥掉。full `just precommit` 未跑，仍受当前本机 pnpm install/approval gate 与 dirty tree 影响。

---

### 2026-06-28 · ChatPanel Thought / 正文间距收紧

- **本轮目标**：回应用户截图反馈：ChatPanel 里 Thought 和正文之间距离过大；另一个 `stage_run` 工具卡 + 底部 `Running stage run` 重复状态先讨论，不直接改。
- **已完成**：
  - `frontend/components/AIChatPanel/ThinkingBlock.tsx`：默认 message variant 不再自带 `mb-2`，避免 Thought 自身 margin 与 MessageBlock segment gap 叠加。
  - `frontend/components/AIChatPanel/MessageBlock.tsx`：正文紧跟 Thought 时加 compact top spacing（`-mt-1`），只收紧 Thought→正文这条相邻关系，不改变工具卡与其它 segment 的常规间距。
  - `docs/modules/frontend/components.md`：同步记录 ChatPanel Thought / 正文连续出现时的 spacing 约束。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/AIChatPanel/MessageBlock.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/messageSegments.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/AIChatPanel/ThinkingBlock.tsx`、`frontend/components/AIChatPanel/MessageBlock.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新 ChatPanel 视觉确认 Thought 与正文距离是否合适；`stage_run` 重复状态样式需用户确认方案后再改。

---

### 2026-06-28 · EAS PORT 查空时 SVC 覆盖派生终态

- **本轮目标**：回应用户截图里平安信托 `36/43 done`、7 个 IP 行显示 `查空 LIVE/PORT` 但仍 `未查 SVC` 的问题；判断这些 IP 没有开放端口时，SERVICE-FINGERPRINT 不应继续作为 pending 缺口展示。
- **根因/判断**：
  - `ai_get_stage_asset_coverage` 的 read-model 只认 found truth 和 terminal outcomes；IP/domain 结构上适用 PORT/SERVICE，所以当 PORT 已 `checked_empty` 且没有显式 SERVICE outcome 时，SVC 仍落到 `pending`。
  - gate 可以接受模型/交付物里的 `not_applicable` 终态，但 UI snapshot 没有把“无开放端口 => 无服务指纹面”做确定性派生，导致 pass 后仍显示 `未查 SVC`。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：EAS coverage cells 生成后，若 PORT cell 已 terminal `checked_empty/not_applicable`，且 SERVICE-FINGERPRINT 仍是 `pending`，则把 SERVICE-FINGERPRINT 派生为 `not_applicable`，并清空 suggested tools；显式 SERVICE outcome（found/empty/error/blocked/not_applicable）不会被覆盖。
  - 同文件新增回归测试：PORT 查空派生 SVC not_applicable；PORT found 时 SVC 仍 pending；显式 SERVICE outcome 优先。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 EAS SVC read-model 派生规则。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 24 tests passed / 88 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：full `just precommit` 未跑；本机该工作流此前仍受 pnpm ignored-build approval gate 影响。需要重启/热重载后刷新平安信托详情页，预期这些 `查空 PORT` 的 IP 不再显示 `未查 SVC`，summary 从 `36/43` 收敛为当前 batch 全 done（除非还有别的真实 pending/error）。

---

### 2026-06-28 · EAS per-org wave 改为 global delta expansion backlog

- **本轮目标**：按用户澄清修正 EAS wave 口径：不要在单个子公司 gate PASS 后立即 promote/continue 下一 wave；所有 org 先完成当前 seed batch，新发现 HTTP(S) 入口 / 新 host 作为 expansion backlog，后续由全局 delta pass 统一处理。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：移除 per-org gate PASS 后的自动续跑；当前 durable wave 只作为 current batch denominator freeze，PASS 后 mark completed 并写 `org_stage_completions`。
  - 同文件：所有 org seed batch 都 PASS 后，统一为有新增 target 的 org queue durable delta batch；只要 queue 出 delta batch，本轮不发 close `pass_token`，要求主 agent 再跑一次 `stage_run` 处理全局 delta。
  - 同文件：worker objective / current wave instruction 改为 `next_wave_pending` 是 global delta expansion backlog，不是“马上下一 wave”。
  - `docs/design/2026-06-28-stage-expansion-wave-barrier.md` 与对应 plan 标记 superseded；新增 `docs/design/2026-06-28-eas-global-delta-expansion.md`、`docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md` 记录新方向。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-db/repo.md`；`feature_list.json` 的功能条目改成 global delta expansion 口径。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime --check` → exit 0。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-app/ai.md docs/design/2026-06-28-stage-expansion-wave-barrier.md docs/design/2026-06-28-eas-global-delta-expansion.md docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md feature_list.json` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch --status-level fail` → 1 test passed / 286 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/design/2026-06-28-eas-global-delta-expansion.md`、`docs/design/2026-06-28-stage-expansion-wave-barrier.md`、`docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md`、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、上述 4 张模块卡、`feature_list.json`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：这次完成了“seed 全部 org 收口后统一 queue delta batch，并暂停 close token”的调度修正；HTTP(S) endpoint promotion classifier / `expansion_queue` processed/skipped 状态仍待实现。full `just precommit` 未跑，仍受本机 pnpm ignored-build approval gate 影响。

---

### 2026-06-28 · EAS 资产覆盖 summary / wave 口径修复

- **本轮目标**：回应用户质疑“主资产 288/294 done 怎么也过 gate”：核对 Ping An EAS DB/run_tree 真相，并修复前端资产覆盖 summary 与 wave cutoff 口径不一致造成的误导。
- **根因/判断**：
  - DB/run_tree 显示 root org `0e9753e6-3cbb-40bf-9510-8d1bda7193f1` 的 EAS 不是在 `288/294` 时最终 PASS；当时确有 6 个 `GOLISH-EAS-PORT` pending，后续补成 `empty/naabu` 后继续跑 wave #2/#3，最终 `org_stage_completions` 于 `2026-06-28 21:30:32 +08:00` 写入。
  - UI 问题是 `StageAssetCoverageBlock` 没传 `stageStartedAt`，并且 compact/header summary 从 rows 自行重算，容易把 `new_in_stage`/下批资产混进当前分母或展示旧 attempt 快照，造成“288/294 也过了”的错觉。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：coverage API 请求透传 `stageStartedAt`；summary/compact strip/full panel 统一使用后端 `snapshot.summary`，删除前端 rows 重算分母逻辑。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：从父 `stage_run` 工具块读取 `startedAt`，传给资产覆盖块，供后端按 stage/wave cutoff 标记 `next_wave_pending`。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：回归锁定 `stageStartedAt` 传参，并把 summary 期待改为以后端 summary 为准。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 coverage summary/wave 口径约束。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782643983045-1 --db` → exit 0；确认 Ping An root EAS 中间有 `6 cells never reached a terminal state`，后续 wave #1/#2/#3 完成后 root org PASS。
  - Python 只读 DB 查询 embedded Postgres → exit 0；root org EAS waves = 200 / 108 / 1 all completed；6 个域名的 `GOLISH-EAS-PORT` 均已有 `empty` outcome（source `naabu`，evidence `[12113]`）。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 65 tests passed。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：full `just precommit` 未跑；本机此前仍有 pnpm ignored-build approval gate。需要刷新同一 EAS 历史/详情 UI，确认最终 PASS 上下文不再显示旧的 `288/294` 口径，并能把下批资产以 `下批`/`next_wave_pending` 表达。

---

### 2026-06-28 · SubAgent detail Thought / Agent Output 视觉统一

- **本轮目标**：回应用户截图反馈：detail 里的 `Thought` 和 `Agent Output` 看起来像两个不同层级，希望视觉更统一。
- **已完成**：
  - `frontend/components/AIChatPanel/ThinkingBlock.tsx`：新增 `variant="detail"`，只在 detail 场景调整 Thought 的标题权重、间距、展开内容字号/行高；默认聊天消息里的 Thought 样式保持不变。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：抽出统一 narrative block class，让 Thought 与 Agent Output 共用左侧 rail、背景、内边距；Agent Output 标题从强分区标题降为与 Thought 同级的紧凑标题。
  - 同文件：修复 `parentStageRunTool` Zustand selector 每次返回新对象导致 detail 挂载时可能触发 React `Maximum update depth exceeded`；拆成 status / startedAt 两个 primitive selector。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增 stage-run backed detail 挂载回归，锁住 selector 不再触发无限更新。
  - 根据用户复看截图再次收紧视觉：去掉 Thought/Agent Output 外层连续左侧 rail，压缩 narrative block 上下间距，并让 Agent Output 正文缩进到标题文字列下方，避免正文从图标列起头造成错位。
  - 根据用户继续反馈：进一步淡化 detail Thought（使用 muted foreground、normal weight），并移除普通正文前的 `Agent Output` 标题；保留时间顺序，不把 output 倒排到 thought 上方。
  - 根据用户最新截图：修正 Thought 后紧跟正文时的“双 padding”问题；正文块在前一条是 Thought 时使用 compact top spacing，让 Thought 和正文更贴近同一段叙述。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 Thought / Agent Output 共用紧凑 narrative chrome。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（无代码编译阶段）。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 55 tests passed。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0，fixed 1 file。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 2；失败在既有 dirty 文件 `frontend/components/Engagement/StageAssetCoveragePanel.tsx(186,10): 'coverageRowsSummary' is declared but its value is never read`，非本轮修改文件。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/AIChatPanel/ThinkingBlock.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新实际 detail 面板看视觉效果；全量 typecheck / precommit 仍受当前 dirty tree 的资产覆盖面板未使用变量与 pnpm install gate 影响。

---

### 2026-06-28 · stage_run 提交前 coverage 自检提示收口

- **本轮目标**：回应用户“能不能提交之前告诉 AI 先看少了什么，而不是交了看报错”：把已有 `check_stage_asset_coverage` 从可选提醒强化为 coverage-gated worker 的提交前 mandatory self-check。
- **根因/判断**：
  - `check_stage_asset_coverage` 已能返回 `ready_to_submit` / `gap_examples` / `cell_summary` / `next_action`，但此前主要靠 methodology 文案和 submit 后的 `needs_fix`，worker objective 本身没有强制“提交前先查缺口”。
  - 这会让弱模型继续先调 `submit_stage_deliverable`，再从 gate 报错里学习缺什么；用户看到的体验就是“每次过 gate 很麻烦”。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：coverage-gated per-org objective 新增 `PRE-SUBMIT SELF-CHECK (mandatory)`，要求 `submit_stage_deliverable` 前先调用 `check_stage_asset_coverage(stage, organization_id)`；`ready_to_submit=false` 时按 `gap_examples` / `cell_summary` / `next_action` 补洞或终态收口，不允许试提交。
  - `resources/harness/stages/{target_intel,external_attack_surface,enumeration}/methodology.md`：同步说明 preflight 是 required self-check，不是 trial submit。
  - 模块卡同步：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime build_org_objective --status-level fail` → 2 tests passed / 285 skipped。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本轮 runtime/stage methodology/doc/progress 文件>` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`resources/harness/stages/external_attack_surface/methodology.md`、`resources/harness/stages/target_intel/methodology.md`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：这是 prompt/objective 层收口，能显著减少“先提交再看 gate 报错”的行为；如果后续还想做到硬约束，需要在 submit tool 里记录最近一次 `check_stage_asset_coverage ready_to_submit=true` 的 preflight stamp，再拒绝未预检的提交，这会是更大一层 runtime 状态改造。

---

### 2026-06-28 · EAS Prober 后台等待与 batch SERVICE 落库收口

- **本轮目标**：回应用户对最新 Test1 EAS run 的诊断结论：主公司 Prober 因 broad `nmap -sV -iL` 后台任务和 SERVICE coverage 未终态而看起来卡死；先修低风险 runtime/landing 问题，暂不做 stage_run 并发大改。
- **根因/判断**：
  - `whatweb --input-file='/abs/path'` 这类 equals+quoted 绝对路径会被 batch input parser 当成带引号的相对路径，拼成 `workspace/'/abs/path'`，导致后台 batch SERVICE outcome 读不到 input file，工具跑完也不补 `GOLISH-EAS-SERVICE-FINGERPRINT` terminal rows。
  - EAS Prober / StageRefiner 文案仍容易把 SERVICE 缺口引向对 raw in-scope 大列表跑 broad `nmap -sV -iL`，而不是基于确认开放端口的 host:port 分组。
  - `wait_for_background_jobs` 需要按 Cursor/Codex 式 wait/check loop 表达：总等待可长，但 idle 无新输出时应返回可操作状态，让 agent `check_job` 一次，有进展继续等，无进展再 kill/缩窄/终态收口；不能静默卡住整个 org，也不能误杀有进展的长任务。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：batch input file parser 先剥 `--input-file='/abs/path'` 参数值两侧引号，再判断绝对路径；新增回归测试。
  - `backend/crates/golish-app-core/src/pty_interactive.rs`：`wait_for_background_jobs` 新增 idle-progress 跟踪，默认总等待仍 300s；如果 stdout/stderr 在 idle 窗口内无新进展，返回 `still_running` + `wait_reason=idle_timeout` + 推荐 `check_job`/按需 `kill_job`，有进展则继续等到总窗口。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：EAS Prober objective 明确禁止 raw 大列表 broad `nmap -sV -iL`；SERVICE 只能基于确认开放端口的 host:port 分组；后台等待按 visible wait/check loop。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs` 与 `resources/harness/stages/external_attack_surface/methodology.md`：SERVICE repair 提示改为 confirmed-open-ports 分组；不可解析/无开放端口/批次过宽用具体 terminal note 收口。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 仍失败（本机 pnpm ignored-builds approval gate）。
  - `cd backend && cargo fmt -p golish-app-core -p golish-agent-runtime -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-app-core pty_interactive --status-level fail` → 10 tests passed / 35 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 17 tests passed / 92 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime build_org_objective --status-level fail` → 2 tests passed / 285 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_refiner --status-level fail` → 3 tests passed / 763 skipped。
  - `cd backend && cargo check -p golish-agent-app -p golish-app-core -p golish-agent-runtime -p golish-agent-kit` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-app-core -p golish-agent-runtime -p golish-agent-kit --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本轮后端/runtime/doc 文件>` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-app-core/src/pty_interactive.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`resources/harness/stages/external_attack_surface/methodology.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：本轮没有实现 stage_run per-org 并发，因为当前 `stage_run_call.rs` 明确使用共享 side-channel 串行执行；若要并发，需要先隔离 deliverable sink / worker state。需要重启 dev app 后重新跑 Test1 EAS，确认主公司 Prober 不再 broad service sweep 卡住，且 WhatWeb/nmap batch SERVICE terminal rows 能落入 coverage。

---

### 2026-06-28 · target_intel 组织情报 source target 汇总修复

- **本轮目标**：回应用户截图反馈：后端 target_intel 阶段按理已经通过，但前端组织情报行里 DNS / ASN / CT / 子域 / OSINT 仍显示未查，只有 WHOIS 显示查空。
- **根因/判断**：
  - 用户判断成立：这里不是单纯前端样式问题，而是 read-model key 对不上。
  - `ai_get_stage_asset_coverage` 的 organization row 用公司名当 asset key；但 `source_query_log` 里的 terminal rows 常常记录在实际查询目标上（例如 `pingan.com`），当前 org 又可能还没有登记真实 asset row，于是 `merge_source_query_row` 只匹配空 target 或完全相同 asset value，导致 DNS/ASN/CT/子域/OSINT 这类 source/provider terminal rows 被漏投影。
  - gate/后端可通过不等于 UI 的组织情报行都应是 `found`；UI 应解释 source/provider 的 terminal 状态，不能因 target key 不同画成未查。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：对 target_intel source/provider terminal rows 增加 organization-row rollup；当 source row target 无法匹配任何已登记资产，但当前 snapshot 有 organization row 时，按 technique 汇总到组织情报行。
  - 同文件：新增回归测试覆盖 `target="pingan.com"` 这类 unmatched source row 会汇总到组织行；同时锁定没有 organization row 时不能误映射到普通资产。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 target_intel source/provider terminal rows 的 organization row 汇总语义。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 21 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 dev app / 后端进程后刷新同一组织情报行；DNS/ASN/CT/子域/OSINT 不应再因为 source target 是域名而显示未查。full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · target_intel 组织情报维度标签可读性修复

- **本轮目标**：回应用户截图反馈：Intel 阶段资产覆盖里“组织情报”只显示一排 `? / ✓` 小格，用户不知道每一类分别代表什么。
- **根因/判断**：
  - 后端 `target_intel` organization row 实际有 6 个被动情报维度：DNS、WHOIS、ASN、CT、Subdomain、OSINT。
  - 前端之前复用真实资产矩阵的紧凑小状态格，只把维度名放在 `title` hover 里；默认视觉上看不到维度名。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：新增 organization 专用 coverage chip，把组织情报维度直接显示为 `DNS` / `WHOIS` / `ASN` / `CT证书` / `子域` / `OSINT`，并保留每格状态符号与 hover 详情。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增 target_intel organization-only snapshot 回归，锁定 6 个维度标签必须可见。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 target_intel 组织情报不允许只画无标签小状态格。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 20 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新资产覆盖面板确认截图里的组织情报行不再是一排无标签小格；full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · 资产覆盖 evidence_refs fallback SQL 修复

- **本轮目标**：回应用户截图反馈：Target Intel 的资产覆盖面板显示“加载失败”，API 报 `[API trace=...] ai_get_stage_asset_coverage: no column found for name: evidence_refs`。
- **根因/判断**：
  - 这不是资产为空，而是 `ai_get_stage_asset_coverage` 后端读模型失败。UI fallback 查询 `technique_outcomes` / `source_query_log` 最新 terminal rows 时，SQL 选出列名 `evidence_ids`，但 `TechniqueOutcomeProjectionRow` / `SourceQueryProjectionRow` 的 `sqlx::FromRow` 字段名是 `evidence_refs`。
  - `sqlx::FromRow` 按列名取值；一进 latest fallback 就因为找不到 `evidence_refs` 列而直接让整个覆盖面板失败。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：把 latest fallback 两条 SQL 抽成常量，并将 `evidence_ids AS evidence_refs` 显式 alias 给投影结构。
  - 同文件：新增单测锁住 `evidence_ids AS evidence_refs` alias，避免后续改 SQL 又把 UI fallback 打断。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 19 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 dev app / 后端进程后刷新资产覆盖面板；该面板不应再因 `evidence_refs` 列名错误加载失败。full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · stage_run 历史详情持久化修复

- **本轮目标**：回应用户截图反馈：关掉/重开后，`stage_run` 之前的调用记录、子 agent 对话/工具详情看起来丢失，只剩聊天里的 `Running specialist agents` 工具卡。
- **根因/判断**：
  - `Running specialist agents` 不是后端重新跑出来的阶段名，而是 `frontend/lib/tools.ts` 给 `stage_run` 的人类化工具标题。
  - DB autosave 的 conversation fingerprint 之前只看 timeline block 数量和最后一块 id/type；`sub_agent_activity` 旧 block 内部的 `entries`、`toolCalls`、`result`、`thinking` 变化，以及 `ai_tool_execution.streamingOutput/result` 变化不一定触发保存。关窗后恢复端只能拿到轻量 `stageRunJson`，完整子 agent 运行流可能没写进 `timeline_blocks`。
  - `terminal_state.stage_run_json` 之前只保存 session 当前 `stageRun`，没有保存 `stageRuns[requestId]` 历史 map；连续 `stage_run` / continue 后，旧工具行可能找不到自己 requestId 对应的 rows。
- **已完成**：
  - `frontend/lib/conversation-db-sync.ts`：新增 timeline 内容指纹，覆盖 `sub_agent_activity.entries/toolCalls/result/thinking`、`ai_tool_execution.streamingOutput/result`、command output 等关键内容；旧 block 内容变化也会触发 autosave。
  - 同文件：`stage_run_json` 改为兼容 v2 包 `{ current, byRequestId }`，保存当前 run 和 request-scoped 历史 map；`stageRuns[requestId]` 变化也进入 autosave fingerprint。
  - `frontend/lib/terminal-restore.ts`：恢复端兼容旧的单个 `SessionStageRun` JSON，并能把 v2 `byRequestId` map 放回 session。
  - `frontend/lib/conversation-db-sync.test.ts`：新增回归覆盖 sub-agent 旧 block 更新、非最后一条工具 streaming output 更新、stage_run v2/legacy 持久化形状。
  - `docs/modules/frontend/lib.md`：同步记录 conversation DB autosave / stage_run restore 约束。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（沿用本机 `ERR_PNPM_IGNORED_BUILDS` approval gate）。
  - `./node_modules/.bin/biome check --write frontend/lib/conversation-db-sync.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/conversation-db-sync.test.ts frontend/store/stage-run.test.ts` → 2 files / 21 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/conversation-db-sync.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.ts docs/modules/frontend/lib.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（stage_run 历史详情持久化 scope）**：`frontend/lib/conversation-db-sync.ts`、`frontend/lib/conversation-db-sync.test.ts`、`frontend/lib/terminal-restore.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：该修复保证后续保存/恢复不再只剩 `stage_run` 轻量卡；已经被关窗前未写进 DB 的旧子 agent 详情，仍需要从 `{workspace}/.golish/transcripts/<session>/run.log` / `transcript.json` 或 `scripts/run_tree.py` 追，不会凭空从 DB 里恢复。

---

### 2026-06-28 · Chat / 调用树调试编号与工具次数隐藏

- **本轮目标**：回应用户要求：ChatPanel 和左侧调用/详情区域里可见的 `Txx` 调试编号，以及“调用步骤/工具调用了多少次”的次数徽标不要再展示。
- **已完成**：
  - `frontend/components/ui/AnchorChip.tsx`：保留组件和调用点兼容性，但不再渲染可见 anchor chip；requestId 仍留在 store / detail navigation 内部使用。
  - `frontend/components/AIChatPanel/SubAgentInlineCard.tsx`、`frontend/components/SubAgentCard/*`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentTreeView/SubAgentTreeView.tsx`：移除 inline card、sub-agent card、modal、detail header、左侧调用树 header/agent row 中的工具次数汇总；具体工具调用行仍可展开查看。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 request-id 锚点和工具次数汇总只作为内部导航/调试数据保留，不作为产品 UI 展示。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts frontend/components/AIChatPanel/InlinePlanCard.test.tsx` → 3 files / 57 tests passed, exit 0。
  - `git diff --check -- frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/ui/AnchorChip.tsx`、`frontend/components/AIChatPanel/SubAgentInlineCard.tsx`、`frontend/components/SubAgentCard/SubAgentCard.tsx`、`frontend/components/SubAgentCard/SubAgentDetailsModal.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentTreeView/SubAgentTreeView.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新 ChatPanel / sub-agent detail / 左侧调用树确认不再显示调试编号和工具次数；full precommit 仍需先处理本机 pnpm `approve-builds` gate。

---

### 2026-06-28 · EAS 覆盖 UI session fallback 收口

- **本轮目标**：接续崩溃 session 的截图/对话，排查「深圳平安人寿保险公司」资产覆盖 UI 仍显示未查，但 DB/gate 已有终态的问题。
- **现场结论**：
  - 用户质疑成立：当前 embedded PG 中 `深圳平安人寿保险公司` 的 `124.196.57.222`、`202.69.19.167` 等 IP 已有 `technique_outcomes` 终态；例如 `LIVENESS=empty/httpx`、`PORT=empty/naabu`，不是未查。
  - 崩溃 session 留下了半改状态：`stage_asset_coverage_snapshot` 已把 `session_id: Option<&str>` 传给 `stage_outcomes`，但 `stage_outcomes` 签名仍是 `&str`，导致当前 `golish-agent-app` 会编译失败。
  - UI 解释层和 agent 预检层需要分开：UI 可以在 session id 缺失/对不上时用同 org 最新 terminal outcome 做显示兜底，避免把已查空画成 pending；`check_stage_asset_coverage` 仍必须 strict session，不允许旧 run 结果帮 agent 通过提交前预检。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：补完 `stage_outcomes(... session_id: Option<&str>, allow_latest_fallback)`；Tauri UI 命令开启 latest terminal fallback，DB trait/agent preflight 关闭 fallback。
  - 同文件：`technique_outcomes` merge 时统一走 `coverage_lookup_asset`，修正 EAS LIVENESS URL endpoint key（`http://x:90` ↔ `x:90`）读模型匹配；latest fallback SQL 按 org + technique 取每个 `(asset, technique)` 最新 terminal row。
  - `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`：共享 snapshot helper 调用接入 strict 模式。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 UI fallback 与 agent preflight strict session 的边界。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - embedded PG 查询 `technique_outcomes` / `org_stage_completions`：确认 `深圳平安人寿保险公司` 的 `124.196.57.222`、`202.69.19.167` 等已有 `empty` terminal rows；latest fallback SQL 对 `124.196.57.222` / `202.69.19.167` 返回 4 行 `empty` outcome。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 18 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：重启 dev app 让后端 read-model 生效；刷新这家公司的资产覆盖面板后，已有 `empty` terminal rows 的 IP 不应再显示为 `? / 未查`。full precommit 需先处理 pnpm `approve-builds` gate。

### 2026-06-28 · EAS 覆盖 gate 与 UI read-model 口径排查

- **本轮目标**：回应用户追问“UI 里还有未查，为什么 gate 能过；政策资产覆盖是否应该 48/48 才算过”。
- **现场结论**：
  - `external_attack_surface` gate 的完整性不是“全绿色命中”，而是当前 wave 的每个适用 `(asset × technique)` 都有 terminal 状态；terminal 包括 `found`、`checked_empty`、`blocked`、`not_applicable`。本阶段新发现并排入下一批 wave 的资产不计入当前 wave 分母。
  - 该 org (`41f0a556-1176-43ec-b854-5cef2005494b`) 的 `org_stage_completions` 显示 EAS 在 `2026-06-28 16:21:25 +08` 通过；`stage_asset_waves` 有 wave 0/1/2 三批 completed。
  - transcript 显示早期 submit 确实被 `coverage_gap_actions` 拦过，不是“未查也放行”；后续 submit 在 `2026-06-28T08:16:45Z` accepted，其中 20 个 Pingan internal-only 域名提交了 60 个 `not_applicable` coverage cells（LIVENESS/PORT/SERVICE-FINGERPRINT），后续 wave #2/#3 也 accepted。
  - UI 截图里仍像 pending 的主要问题是 read-model 表达不完整：accepted deliverable 的 `not_applicable` terminal cells 目前没有稳定物化到 `technique_outcomes` 读模型；另有一个确定 bug 是 UI 适用性曾只看 `targets.type`，而 gate 用 `target_type + value` 的 value-aware 分类，URL 形态值可能被 UI 多显示假 PORT/SERVICE pending。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：覆盖矩阵适用性改用 `AssetClass::classify(Some(target_type), value)`，与 gate 口径一致；URL 形态 hostname 即使存成 `domain` 也不会冒出假的 PORT/SERVICE pending。
  - 同文件：`outcome_state` 识别 `not_applicable`，避免未来物化进 `technique_outcomes` 后又被 UI 读成 pending。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 `ai_get_stage_asset_coverage` 必须 value-aware classification，并说明 checked_empty/error/blocked/not_applicable 的读模型来源。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` → 查到该 run 的 DB 自诊断、gate block/pass 线索。
  - DB 查询 `org_stage_completions` / `stage_asset_waves` / `targets` / `technique_outcomes` → EAS completion `2026-06-28 16:21:25 +08`；wave 0/1/2 均 completed；targets 为 65 seed domain + 59 seed IP + 2 active domain + 3 active IP。
  - transcript grep `prober-call_00_iTmmAVW57YDu8fCqG1yq1095::org::41f0a556.../transcript.json` → `2026-06-28T08:16:45Z` submit accepted，内部域名 `not_applicable` cells 存在；`2026-06-28T08:19:08Z` / `08:21:18Z` 后续 wave submit accepted。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 15 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（coverage gate/read-model scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：后续需要把 accepted deliverable 里的 `blocked/not_applicable` terminal coverage 物化到稳定读模型（或单独的 coverage projection），否则旧 run/自报 terminal cell 仍可能在 UI 里看起来像 pending。

---

### 2026-06-28 · StageAssetCoverage 状态语义可读性修复

- **本轮目标**：回应用户截图反馈：资产覆盖矩阵里的小点容易被误读成“查空”，不确定哪些是未查、哪些是查空、哪些是新增/下批数据。
- **根因/判断**：
  - 后端读模型语义是正确的：`empty` → `checked_empty / 查空`；没有 terminal outcome 的格子 → `pending / 未查`；本阶段新增且排到下一 wave 的资产 → `next_wave_pending / 下批`。
  - 前端之前用弱化小点 `·` 表示 `pending`，顶部 chip 仍写英文 `pending`，图例没有列出 `next_wave_pending`；解析 IP 聚合行写“未登记 IP direct 行”，容易让用户误以为该 IP 行也是未查/查空。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：`pending` 状态格从弱点号改为 `?`，顶部 chip 改成 `N 未查`；图例补上 `下批`。
  - 行副标题追加状态摘要，例如 `未查 LIVE/PORT/SVC`、`下批待查 LIVE`、`查空 PORT`，让每一行不用 hover 就能区分未查和查空。
  - 解析 IP synthetic group 行文案改为 `仅分组，不计覆盖`，明确这类行不是 direct 覆盖行，也不代表未查/查空。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增/更新回归，锁定 pending 行摘要、next-wave 行摘要、synthetic IP group 不计覆盖。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 pending 必须有行级状态摘要、next_wave_pending 必须可见、synthetic IP group 只能作为分组行。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 19 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：待提交。
- **本轮修改但未提交（资产覆盖状态语义 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新资产覆盖面板后，截图中小点位置应变成 `?` 并在行副标题看到 `未查 ...`；真正查空才显示 `∅ / 查空`。若仍觉得矩阵太密，可以再把未查列加淡色背景或按状态筛选。

---

### 2026-06-28 · AIChatPanel 恢复期 loading 空态修复

- **本轮目标**：回应用户截图反馈：载入/恢复时右侧 AI 面板显示“今天要做点什么呢 / 工具可用”，看起来不像正在加载。
- **根因/判断**：
  - `AIChatPanel` 之前在 `messages.length === 0` 时无条件显示真实空会话提示；没有区分 `workspaceDataReady=false`、`pendingTerminalRestoreData`、`terminalRestoreInProgress`，以及 conversation 已恢复但 `activeSessionId` 尚未绑定的中间状态。
  - 截图中左侧仍是 `No active session`，右侧却可见空会话提示，正是 conversation/terminal restore 的绑定空窗。
- **已完成**：
  - `frontend/components/AIChatPanel/restoreLoadingState.ts`：新增 `shouldShowChatRestoreLoading`，把 workspace 未就绪、pending restore、restore in progress、active session 未绑定统一判为恢复 loading。
  - `frontend/components/AIChatPanel/AIChatPanel.tsx`：空消息区先判断恢复 loading，显示 spinner + “正在载入工作区 / 正在恢复会话和终端...”；恢复完成且确实空会话时才显示“今天要做点什么呢”。
  - `frontend/lib/i18n/{en,zh-CN}.json`：新增 loading 文案。
  - `frontend/components/AIChatPanel/restoreLoadingState.test.ts`：新增 4 条回归测试覆盖 workspace 未 ready、pending/running restore、conversation-terminal binding gap、正常空态。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 AIChatPanel 空态必须区分真实空会话与恢复期 loading。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/restoreLoadingState.test.ts` → 1 file / 4 tests passed, exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `jq empty frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（UI loading scope）**：`frontend/components/AIChatPanel/AIChatPanel.tsx`、`frontend/components/AIChatPanel/restoreLoadingState.ts`、`frontend/components/AIChatPanel/restoreLoadingState.test.ts`、`frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：未启动 dev app 做截图 QA；真实恢复窗口需要刷新/打开项目时观察。full `just precommit` 仍被本机 pnpm approval gate 阻塞。

---

### 2026-06-28 · EAS LIVENESS endpoint key gate loop 修复

- **本轮目标**：用户要求在确认最后一次跑的原因后直接修改；修复 `pentest-chat-1782574914157-1` 中 `http://linquankuaipin.com:90` / `http://ytzp.top:90` 已跑探活但 submit gate 仍报 `GOLISH-EAS-LIVENESS never attempted` 的问题。
- **现场结论**：
  - `run.log` 显示 `httpx` 已对 `http://linquankuaipin.com:90` 执行，completion 也记录了 `background batch liveness outcomes stored stored=1`；随后 gate 仍按 `http://linquankuaipin.com:90 × GOLISH-EAS-LIVENESS never attempted` 拦截。
  - DB 里 `technique_outcomes` / evidence fact 写成 `linquankuaipin.com`，而 gate 对 in-scope URL endpoint 的 join key 是去 scheme 后保留 port 的 `linquankuaipin.com:90`。因此事实存在，但落在 host-only key 上，关不掉 URL:port cell。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/evidence_facts.rs`：新增 `eas_liveness_asset_key`，专门给 EAS LIVENESS 使用；去 scheme/大小写，但保留 URL endpoint 的 port/path。`httpx -u http://x:90` 现在派生 `GOLISH-EAS-LIVENESS` fact `x:90`。
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：后台批量 liveness completion 写 `technique_outcomes` 时改用 endpoint key，避免 `http://x:90` 被 `canonical_asset_key` 折叠成 `x`。
  - `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`：`upsert_technique_outcome_impl` 对 `GOLISH-EAS-LIVENESS` 走 endpoint key；PORT / SERVICE-FINGERPRINT 继续 host-level canonicalization。
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：coverage snapshot / `check_stage_asset_coverage` 对 LIVENESS 使用同一 endpoint key，前端矩阵和 submit preview 与 gate 口径一致。
  - `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`：加回归证明裸 host liveness fact 不能关闭 `http://host:90`，endpoint fact `host:90` 可以关闭。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit --status-level fail -E 'test(eas_liveness_asset_key_preserves_url_endpoint_port) | test(coverage_maps_eas_liveness_tools) | test(coverage_complete_liveness_fact_must_preserve_url_port_endpoint)'` → 3 tests passed, 763 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app --status-level fail -E 'test(batch_liveness_input_file_is_recovered_from_httpx_l_flag) | test(batch_liveness_input_is_recovered_from_httpx_quoted_heredoc) | test(batch_liveness_and_service_commands_are_classified_by_intent) | test(eas_url_asset_only_requires_liveness) | test(outcome_merge_keeps_stronger_terminal_state_and_evidence)'` → 5 tests passed, 95 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机仍有前序已记录的 pnpm ignored-build approval gate（`@swc/core` / `electron` / `esbuild`），且本轮是后端 targeted 修复。
- **提交记录**：未提交。
- **本轮修改但未提交（本 bugfix scope）**：`backend/crates/golish-agent-kit/src/harness/evidence_facts.rs`、`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`、`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`agent-progress.md`。当前工作树仍叠有本日此前 wave/UI 等未提交改动，本轮未回滚。
- **下一步建议**：重启 dev app 让 Rust 代码生效；对同一目标再跑 EAS liveness 后，`http://linquankuaipin.com:90` 应写成 `linquankuaipin.com:90` 的 outcome，submit gate 不应再因 host/endpoint key mismatch 报 never attempted。

---

### 2026-06-28 · 资产覆盖快速滚动黑底与卡顿残留修复

- **本轮目标**：回应用户截图反馈：资产覆盖完整矩阵滑动太快时，列表下方会露出黑底/空白；用户复测后确认黑底可用但滚动仍有点卡，继续优化滚动路径；随后用户反馈每次 polling/live 更新会突然刷新列表、打断正在看的资产，继续加阅读稳定窗口；最后用户反馈快滑仍偶发黑底，继续把当前 332 资产规模退出虚拟化路径。
- **根因**：
  - 上一轮已做 group 虚拟化，但虚拟窗口的 scroll 读数仍走 rAF；快速甩动滚动条时，浏览器可能先把 scrollTop 移到新位置，而 React 仍渲染旧窗口，出现一帧黑底。
  - `CoverageGroupsList` 通过 `RefObject.current` 读外层 scroll 容器；ref 赋值本身不触发 effect，存在监听绑定时序空窗。
  - active/all 或内容缩短时，旧 `scrollTop` 的夹取发生在 effect + 下一帧，也可能短暂落在空白区。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：scroll 容器改用 callback ref + state 传给虚拟列表，确保节点出现后重新绑定监听；scroll 事件同步读取 metrics，不再等 rAF；resize 仍保留 rAF 合并。
  - 同组件：内容缩短时在 `useLayoutEffect` 内立即夹住 `scrollTop` 并同步刷新 metrics；虚拟 spacer/scroll body 增加稳定背景；overscan 从 8 组提高到 12 组，降低快速滚动边缘露空概率。
  - 同组件：复测后将虚拟化阈值提高到 160 组，截图里的 89 组 running slice 直接渲染，不再在滚轮事件里频繁触发 React 虚拟窗口更新；每个 group 加 `content-visibility: auto`，让 Chromium 跳过屏幕外绘制；大矩阵 overscan 提到 24 组。
  - 同组件：新增 `ASSET_COVERAGE_READING_FREEZE_MS=8000` 阅读冻结窗口；用户滚动/滚轮/拖动矩阵后，`snapshot` 与 live work 更新先排队，当前可见矩阵保持稳定，停下后再应用最新数据。
  - 同组件：虚拟化阈值再提高到 512 组；当前 332 资产完整矩阵也走直接渲染 + `content-visibility`，只把 600+ 超大矩阵留给虚拟化，彻底避开当前页面快滑时虚拟窗口追不上的黑底。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增快速滚动回归，模拟超大矩阵从顶部直接甩到底部，锁定虚拟窗口会立即切到尾部资产；新增 89 组中等列表和 332 组当前完整矩阵直接渲染回归，锁住平滑滚动路径；新增滚动后 polling 新快照不立即替换矩阵的阅读稳定回归。
  - `docs/modules/frontend/components.md`：同步记录资产覆盖虚拟列表的 scroll 同步刷新 / layout clamp / spacer 背景 / 500 组以下直接渲染 / 阅读冻结约束。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 被 `ERR_PNPM_IGNORED_BUILDS` 阻断。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 18 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需 `pnpm approve-builds`。
- **提交记录**：待提交。
- **本轮修改但未提交（资产覆盖快速滚动 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新完整资产覆盖矩阵后快速上下甩动；89 组左右的 running slice 应走直接渲染并明显更顺。滚动后 8 秒内 polling/live 更新不应替换正在看的矩阵。若完整 300+ 资产全量视图仍卡，再上浏览器 performance 采样看是 row 绘制、spinner 动画还是外层详情页布局造成。

---

### 2026-06-28 · EAS stdin batch liveness outcome 落库修复

- **本轮目标**：排查用户反馈的最新 Task run `pentest-chat-1782574914157-1` 仍在 EAS coverage gate 里反复被拦。
- **现场结论**：
  - `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 显示最后不是新资产扩分母，而是当前 wave 里 14 个资产缺 `GOLISH-EAS-LIVENESS` 终态。
  - `~/.golish/backend.log` 里 2026-06-28T04:32:29Z 的 `httpx <<'GOLISH_STDIN'` stdin 列表正好是这 14 个资产；04:32:51Z completion 后只看到 `background job structured output not detected`，没有 `background batch liveness outcomes stored`。
  - 根因：`commands/bridge_config.rs` 之前只从 `httpx -l <file>` / `nmap -sn -iL <file>` 读取批量探活输入，未识别 `httpx` 直接 stdin/heredoc 批量输入；当 httpx 零输出时，没有逐资产写 `empty`，gate 就一直按 `never attempted` 拦。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：新增 heredoc/stdin batch input 解析，`httpx <<'GOLISH_STDIN'` 也归类为 batch liveness；completion 写入每个 stdin 目标的 `GOLISH-EAS-LIVENESS` `found/empty` outcome。复用同一个 input-text helper 给 port/service batch 路径，保留原 input-file 行为。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 `httpx` stdin/heredoc 批量探活也必须落 `technique_outcomes`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app batch_liveness --status-level fail` → 4 tests passed, 96 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机仍有前序记录的 pnpm ignored-build approval gate，且本轮是后端 targeted 修复。
- **提交记录**：未提交。
- **下一步建议**：重启 dev app 让该修复生效；当前正在跑的旧进程不会自动加载这段 Rust 代码。若继续同一 run，需要让 prober 再跑一次这批 stdin httpx，completion 后 coverage 应从 `never attempted` 收敛为 `found/empty`。

### 2026-06-28 · Stage expansion durable wave 自动续批

- **本轮目标**：继续用户确认的“新资产发现按批次汇总，当前批全部做完后再检查下一批，再集体跑一次 stage_run”的方案；在 Phase 1/2 的 no-schema cutoff + UI/read model 之上，落 Phase 3/4 durable wave 表和 runtime 自动续批。
- **已完成**：
  - `backend/crates/golish-db/migrations/20260625000001_stage_asset_waves.sql`：新增 `stage_asset_waves` / `stage_asset_wave_items`，纯 additive；一条 wave 固定一个 operation×org×stage 的 target 集合。
  - `backend/crates/golish-db/src/repo/stage_asset_waves.rs`：新增 repo helper，支持读取 running wave、创建 initial wave、promote 未分配 in-scope targets 到下一 wave、完成 wave；asset hash 用稳定摘要，仅作批次指纹。
  - `backend/crates/golish-agent-kit/src/db_traits/{types.rs,repo.rs}` + `backend/crates/golish-agent-app/src/ai/db_bridge/{orchestration.rs,mod.rs}`：新增 `StageAssetWaveView` 和 `DbRepoProvider` wave seam，app bridge 接到 golish-db repo。
  - `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：per-org gate 支持 durable wave asset override；有 wave 时用 wave asset list 冻结 `GateContext.in_scope_assets`，并同步过滤 typed asset map；无 durable wave 时回退 Phase 1 的 `stage_started_at` cutoff。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：wave-aware stage/org 先准备 current wave；specialist objective 显示当前批资产；gate PASS 后先 mark wave completed，再 promote 下一批并继续同 org；只有没有下一批时才写 `org_stage_completions`。达到自动 wave cap 时会创建下一批并 blocked，让后续 `stage_run` 从 running wave 接上。
  - 同步模块卡：`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-kit/db_traits.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`；更新计划和 feature evidence。
- **运行过的验证（实跑）**：
  - `cargo fmt`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-db stage_asset_wave --status-level fail`（cwd `backend`）→ 3 passed / 113 skipped。
  - `cargo nextest run -p golish-agent-kit external_attack_surface_enables_asset_wave_barrier_only coverage_preflight_does_not_block_on_next_wave_pending_cells --status-level fail`（cwd `backend`）→ 2 passed / 762 skipped。
  - `cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch --status-level fail`（cwd `backend`）→ 1 passed / 284 skipped。
  - `jq empty feature_list.json` → exit 0；`git diff --check` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；底层 `pnpm install` 被 `ERR_PNPM_IGNORED_BUILDS` 阻塞：`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`。
- **未跑/未通过**：full `just precommit` 未绿（原因如上）；live app rerun 尚未做，migration 需要 app 重启/apply 后才能验证真实 DB rows。
- **提交记录**：未 commit。
- **本轮修改但未提交（durable wave scope）**：`backend/crates/golish-db/migrations/20260625000001_stage_asset_waves.sql`、`backend/crates/golish-db/src/repo/{mod.rs,stage_asset_waves.rs}`、`backend/crates/golish-agent-kit/src/db_traits/{repo.rs,types.rs}`、`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,orchestration.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、相关模块卡、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、`feature_list.json`、`agent-progress.md`。
- **已知风险 / 下一步**：pass-token closeout 仍用既有 org completion token，尚未把 `wave_id/asset_hash` 折进 token；`run_tree.py` wave summary 也还没补。下一步应重启 app 让 migration apply，重新跑 EAS，确认 `stage_asset_waves` 生成、当前批 PASS 后 next wave 自动继续，且 UI next_wave_pending 不再挡当前提交。

---

### 2026-06-28 · stage expansion wave barrier Phase 1

- **本轮目标**：回应用户确认的方向：新发现资产不要实时撑大当前 EAS 覆盖分母；当前批次先全部完成，再检查新资产总和并集体触发下一批 `stage_run`。
- **已完成**：
  - 新增设计文档 `docs/design/2026-06-28-stage-expansion-wave-barrier.md`：定义 wave / seed asset / new asset / expansion barrier；明确当前问题是 UI 已有 `seed_assets/new_assets`，但 gate 仍用 live `targets.scope='in'` 分母。
  - 新增实现计划 `docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`：拆 Phase 1 无 schema 当前 wave freeze、Phase 2 barrier read model、Phase 3 durable wave tables、Phase 4 自动下一批 dispatch。
  - `feature_list.json` 新增 `stage-expansion-wave-barrier-2026-06-28`，状态 `in_progress`；notes 标明 Phase 3 涉及 migration，必须在动 DB schema 前再次确认。
  - 完成 Phase 1 no-schema current-wave freeze：
    - `StageSpec.asset_wave_barrier` + `external_attack_surface/spec.json` 开关。
    - `golish-db::repo::targets::list_in_scope_values_created_before` + `ReconTargetsPort::in_scope_values_created_before` + `DbRepoProvider::in_scope_assets_created_before`。
    - `submit_stage_deliverable` 预检、`stage_run` per-org gate、Task-mode stage close gate 三条路径都用 active `operation_state.stage_started_at` 冻结 EAS 当前 wave 资产轴；DB truth freshness 同步按该 cutoff 收敛。
    - `AgentBridge` 暴露 `harness_active_operation_id_handle` 给 submit tool 注册层读取 active operation id。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-bridge/agent_bridge.md`、`docs/modules/backend/golish-app-core/ports.md`、`docs/modules/backend/golish-db/repo.md`。
  - 完成 Phase 2 no-schema read model：
    - `ai_get_stage_asset_coverage` 仍展示本阶段新发现资产，但将 wave cutoff 后的新资产 cell 标为 `next_wave_pending`，并从当前 wave `total_assets` / pending / done 分母中排除。
    - `check_stage_asset_coverage` 压缩预检不再把 `next_wave_pending` 当作当前 gap，`ready_to_submit` 可在当前 wave 已完成时返回 true，并在 `next_action` 提醒下一批资产。
    - `StageAssetCoveragePanel` 将 `new_in_stage` 行显示为“下批”，summary `done/total` 只计算当前 wave；下批资产仍留在完整矩阵里可见。
  - 同步模块卡 Phase 2 行为：`docs/modules/backend/golish-agent-kit/tool_executors.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**:
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败，未进入 `just check` / `just test`。该阻塞与近期记录一致：本机 pnpm ignored-build approval gate。
  - `cd backend && cargo fmt` → exit 0。
  - `cd backend && cargo check -p golish-db -p golish-app-core -p golish-agent-kit -p golish-agent-app -p golish-agent-bridge -p golish-agent-runtime` → exit 0（8.09s；前一轮冷 check 36.56s 也 exit 0）。
  - `cd backend && cargo nextest run -p golish-agent-kit external_attack_surface_enables_asset_wave_barrier_only --status-level fail` → 1 test passed, 762 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-db list_in_scope_values_before_sql_adds_wave_cutoff --status-level fail` → 1 test passed, 112 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app next_wave --status-level fail` → 1 test passed, 97 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit coverage_preflight_does_not_block_on_next_wave_pending_cells --status-level fail` → 1 test passed, 763 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-app -p golish-agent-kit -p golish-agent-runtime` → exit 0。
  - `pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx && pnpm exec tsc --noEmit --pretty false` → exit 1；仍被 pnpm ignored-build approval gate 拦截（`@swc/core` / `electron` / `esbuild`）。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx && ./node_modules/.bin/tsc --noEmit --pretty false` → 15 tests passed + typecheck exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
- **未跑**：`just precommit`；`./init.sh` 仍在 `pnpm install --silent` 的 ignored-build approval gate 处失败，未进入全量 check/test。
- **提交记录**：未提交。
- **本轮修改但未提交（本需求 scope）**：`docs/design/2026-06-28-stage-expansion-wave-barrier.md`、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、`feature_list.json`、`agent-progress.md`、`resources/harness/stages/external_attack_surface/spec.json`、`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`、`backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs`、`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,recon.rs,evidence.rs}`、`backend/crates/golish-agent-app/src/ai/commands/{bridge_config.rs,stage_coverage.rs}`、`backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`、`backend/crates/golish-app-core/src/ports/recon/targets.rs`、`backend/crates/golish-db/src/repo/targets.rs`、`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、上述模块卡。当前工作树仍有此前任务留下的其它未提交文件，本轮未回滚。
- **下一步建议**：实现 Phase 3/4 前需要用户确认 migration：durable wave tables、current/next wave promotion ledger、当前 wave PASS 后自动 dispatch 下一批 `stage_run`。

---

### 2026-06-28 · Target 管理本级/子树计数口径优化

- **本轮目标**：回应用户截图反馈：Target 目标管理页左侧主公司显示 813，但右侧本公司只有 338，页面口径混乱且视觉别扭；先优化数量语义和右侧默认资产视图。
- **已完成**：
  - `frontend/lib/target-panel/org-tree.ts`：新增 `TargetCountSummary` / `summarizeTargetCounts` / `findOrgTreeNode`，把本组织 own 计数和含子公司 subtree 汇总拆开；保留 `countAllTargets` 作为递归汇总兼容入口。
  - `frontend/components/TargetPanel/OrgTreeSidebar.tsx`：左树主数字改为本组织自己的目标数，in-scope chip 也只看本组织；含子公司汇总只用弱化 `Σ` chip 展示，避免 813 被误读成本公司资产数。
  - `frontend/components/TargetPanel/{TargetGroupedView,OrgWorkspacePanel}.tsx`：右侧 workspace 同时接收本公司资产和子树资产；默认展示本公司资产，父公司有子公司资产时提供“本公司 / 含子公司”切换；顶部指标显示本公司、范围内、含子公司、子公司数。
  - `frontend/lib/i18n/{en,zh-CN}.json`：Target workspace tab 文案从“总览/字段”收紧为“资产/组织资料”，新增本公司/含子公司指标文案。
  - 同步模块卡：`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `./node_modules/.bin/biome check frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md docs/modules/frontend/lib.md` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.test.ts frontend/lib/target-panel/asset-groups.test.ts frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → 4 files / 51 tests passed。
  - `git diff --check -- frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md docs/modules/frontend/lib.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需 `pnpm approve-builds`。
- **已记录证据**：`org-tree.test.ts` 新增回归锁定 `P` 本级 1 个目标、子树 4 个目标（含 1 个 out-of-scope）、2 个 descendant org；scoped typecheck / biome / vitest 全绿；full precommit 未绿的原因是本机 pnpm approval gate。
- **提交记录**：待提交。
- **本轮修改但未提交（TargetPanel UI scope）**：`frontend/lib/target-panel/org-tree.ts`、`frontend/lib/target-panel/org-tree.test.ts`、`frontend/components/TargetPanel/OrgTreeSidebar.tsx`、`frontend/components/TargetPanel/TargetGroupedView.tsx`、`frontend/components/TargetPanel/OrgWorkspacePanel.tsx`、`frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 未解决问题**：未做动作区收纳（hover 多 icon 仍偏挤），这是下一刀视觉优化；未启动 dev server 做截图 QA；`just precommit` 仍被本机 pnpm ignored-builds 阻塞。
- **下一步建议**：刷新 Target 页面，主公司左树应显示本公司 own 数（例：338）+ 弱化 `Σ 813` 汇总；右侧默认“本公司”，需要集团口径时点“含子公司”。若继续优化视觉，下一步收纳左树 hover actions 到更多菜单。

---

### 2026-06-28 · Task 模式 AI transcript 归档与 progress 瘦身

- **本轮目标**：回应用户澄清“task 模式的 AI 日志，不是删”；把旧 Task transcript 移出默认判断路径，同时精简 `agent-progress.md`，避免后续修改判断被旧日志噪声带偏。
- **已完成**：
  - `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts`：保留最新 8 个 `pentest-chat-*` + 2 个 `stage-run-*`；74 个旧 session / `title-gen-*` session 移到 `_archive/2026-06-28-task-transcripts/`，并写入 `ARCHIVE_MANIFEST.json`。
  - `scripts/run_tree.py`：默认 latest-session 候选跳过 `_*/.*` 归档目录和 `title-gen-*` 噪声；显式传 session 名或路径仍可追溯旧 transcript。
  - `agent-progress.md`：从 8047 行瘦到 635 行；288 条旧会话归档到 `docs/archive/agent-progress-archive-2026-06-28.md`；主文件保留最近 20 条和归档链接。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 | sed -n '1,18p'` → exit 0；默认命中最新真实 Task run `pentest-chat-1782574914157-1`。
  - `python3 -m py_compile scripts/run_tree.py` → exit 0。
  - `git diff --check -- scripts/run_tree.py agent-progress.md docs/archive/agent-progress-archive-2026-06-28.md` → exit 0。
  - `wc -l agent-progress.md docs/archive/agent-progress-archive-2026-06-28.md` → `agent-progress.md` 635 行，archive 7465 行。
- **未跑**：`just precommit`；本轮是 transcript 归档 + progress 文档瘦身 + 脚本默认候选过滤，且本机前序仍有 `pnpm` ignored-build approval gate。
- **提交记录**：待提交。
- **本轮修改但未提交（归档/降噪 scope）**：`scripts/run_tree.py`、`agent-progress.md`、`docs/archive/agent-progress-archive-2026-06-28.md`；另有本机 Test1 transcript 目录移动到 `_archive/2026-06-28-task-transcripts/`。
- **下一步建议**：后续排查 Task run 优先用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db`；需要旧日志时显式传 archive 下的 session 路径，不让默认 latest 被旧 `title-gen` / 历史 run 抢走。

---

### 2026-06-28 · fast resume thinking-mode tool_choice 兼容修复

- **本轮目标**：回应用户截图：输入裸 `继续` 后连续报 `ProviderError: Invalid status code 400 Bad Request ... "Thinking mode does not support this tool_choice"`。
- **根因**：
  - 上一轮为裸 resume 加了 native `tool_choice` lock，把第一轮强制到 `stage_run`。
  - 当前 provider/model 开了 thinking/reasoning mode，而该模式拒绝 API 层 `tool_choice`；于是请求在到达模型执行前被 provider 400 拒绝，UI 连续显示红错。
  - 二次截图后查 `~/.golish/backend.log`：运行代码已经带 `native_tool_choice_allowed` 日志，但 `provider=deepseek model=deepseek-v4-flash` 的 thinking 是 provider/model 默认行为，不一定体现在 `enable_thinking=true` / `reasoning` request 参数里；上一版只看 request 参数，因此漏判成 `native_tool_choice_allowed=true`。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`：当请求已经启用 explicit thinking/reasoning（OpenAI reasoning effort、`enable_thinking=true`、`chat_template_kwargs.enable_thinking=true`、非 excluded `reasoning`）时，不再发送 native/API `tool_choice`。
  - 同文件：新增 provider/model 默认 thinking 兼容判断；`deepseek-v4-flash` 这类 DeepSeek thinking-capable model 即使 request 没显式 thinking 参数，也不发送 native `tool_choice`。
  - 同文件：如果 provider 仍返回 `tool_choice + thinking/reasoning` 不兼容错误，立即剥掉 native `tool_choice` 并重试同一轮请求；prompt 级 forced-tool directive 和 dispatch 层拦截仍保留，所以 fast resume 语义不撤回。
  - 新增回归测试覆盖 thinking-mode suppress、错误识别和 `additional_params.tool_choice` 剥离。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only thinking_mode --status-level fail` → 12 passed / 271 skipped。
  - `cd backend && cargo fmt -p golish-agent-runtime && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only thinking_mode deepseek --status-level fail` → 13 passed / 271 skipped。
  - `cd backend && cargo check -p golish-agent-runtime -p golish-agent-bridge -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime -p golish-agent-bridge -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序 `pnpm install` / `./init.sh` 仍被 `ERR_PNPM_IGNORED_BUILDS` approval gate 阻断，本轮做 scoped backend hotfix 验证。
- **提交记录**：待提交。
- **本轮修改但未提交（hotfix scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`、`agent-progress.md`。
- **下一步建议**：重启/热重载后再输入裸 `继续`；不应再出现 thinking-mode `tool_choice` 400。首轮仍会收到 forced-tool prompt，dispatch 层会拒绝非 `stage_run` 工具。

---

### 2026-06-28 · 裸继续直进 stage_run fast resume

- **本轮目标**：回应用户“既然 resume 没问题，为什么点/说继续还要先 Thought、读 organizations/targets，能不能直接继续跑”的 UI/执行语义问题。
- **根因**：
  - `commands/core/chat.rs` 已经能把短“继续/continue”路由到同 chat session 的 checkpointed `TaskOrchestrator::resume`，所以断电/重启后的真续跑没问题。
  - 但 resume 后回到 active specialist stage 时，depth-0 primary 仍然先进完整 agentic loop；模型可能先 `manage_organizations` / `list_in_scope_targets` / 思考，再调用 `stage_run`，于是 UI 看起来像“继续前又重新思考/读库”。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`：新增“裸继续”窄口径识别；`继续/接着跑/continue the previous stage` 会启用 fast path，带“先解释/看日志/不要扫”等 steering 的继续仍走普通 resume。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/`：新增一次性 `stage_run` resume hint；仅在当前 stage 有 specialist 且已绑定 engagement root 时生效，非 specialist/rootless resume 不强制。
  - `backend/crates/golish-agent-bridge/src/`：把 `harness_forced_tool` 从 orchestrator side-channel 透传到 runtime，并在 isolated loop 返回后清空，避免 stale tool lock 泄漏到后续普通聊天。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/`：completion 阶段把 tool_choice 锁到 forced tool 并注入高优先级指令；dispatch 阶段拒绝同一批里的其它 allow-listed 工具。`stage_run` fast path 指令使用 `{"orgs":[]}`，由 runtime 按 bound engagement root 自动展开 authoritative subtree。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-agent-bridge/{agent_bridge,bridge_executor}.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bare_resume --status-level fail` → 1 passed / 95 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only --status-level fail` → 9 passed / 271 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit specialist_stages non_specialist --status-level fail` → 2 passed / 760 skipped。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`；本机前序 `./init.sh` / `pnpm install` 仍被 `ERR_PNPM_IGNORED_BUILDS` approval gate 阻断，本轮只做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交（fast resume scope）**：`backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,subtask_phases/execute.rs,types.rs}`、`backend/crates/golish-agent-bridge/src/{agent_bridge/{mod.rs,constructors/mod.rs,prepare.rs},bridge_executor/trait_impl.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,llm_stream_start.rs,turn/{executor.rs,state.rs,phases/{completion.rs,tool_dispatch.rs}}}`、`backend/crates/golish-agent-runtime/src/eval_support/{multi_turn.rs,single_turn.rs}`、`backend/crates/golish-agent-runtime/src/test_utils/context.rs`、相关模块卡、`agent-progress.md`。
- **下一步建议**：重启/热重载后，在已有 checkpoint 的 specialist stage 里输入裸 `继续`，首个可见动作应直接是 `stage_run` dispatch；如果输入“继续，但先看日志/解释原因”，仍应保持普通 resume 语义。

---

### 2026-06-28 · 资产覆盖大矩阵滚动卡顿修复

- **本轮目标**：回应用户截图反馈：资产覆盖完整矩阵快速滚动很卡，滚快后列表下方出现大片黑色空白。
- **根因**：
  - `StageAssetCoveragePanel` 在完整矩阵里直接渲染全部资产 group/row；EAS 批量扫描时常见 80+ 组、上百资产，每行还有 grid/border 与 live spinner。
  - 快速滚动时 Tauri/Chromium 需要持续重绘整张覆盖表，容易 checkerboarding；同时 live slice 变短时旧 `scrollTop` 可能落在新内容之外，看起来像底部黑掉。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：资产 group 数超过阈值时改为窗口化渲染，只挂可视窗口 + overscan 内的 group；小列表仍走原直接渲染路径。
  - 同组件：滚动/resize 读数用 rAF 合并；active/all 或 live slice 变化后夹住旧 `scrollTop`，避免内容缩短后留在越界滚动位置。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增 80 资产大矩阵回归，锁定虚拟列表路径只渲染可视 group。
  - 同步模块卡：`docs/modules/frontend/components.md`，记录完整覆盖矩阵大列表必须窗口化渲染。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败，底层仍是本机 `ERR_PNPM_IGNORED_BUILDS` approval gate。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 14 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 58 tests passed。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 1；底层 `pnpm install` 被 `ERR_PNPM_IGNORED_BUILDS` 阻断（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`）。
- **未跑/未通过**：全量 `just precommit` 未绿；阻塞原因是本机 pnpm approve-builds gate，本轮用直接二进制完成 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（资产覆盖滚动性能 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入资产覆盖完整矩阵，80+ 资产时滚动应只重绘窗口内 group；如果仍看到黑块，再用同一截图定位是否是 active slice 内容本身过短，而不是渲染卡顿。

---

### 2026-06-28 · stage_run resume-skip 行误显示 queued 修复

- **本轮目标**：回应用户截图反馈：上一轮只有 3 个 org blocked，第二次补洞时后面已经 passed 的 org 也显示成 `Queued`，解释是否因为 gate/pass token/hash，并修掉 UI 误导来源。
- **根因**：
  - 最新 run.log 明确显示这轮模型实际只传了 3 个 blocked org：`stage_run filled missing requested org(s) ... requested_orgs=3 total_orgs=12 auto_added=[...]`。
  - runtime 会把 `stage_run` 入参补回当前 engagement root 的完整 organization subtree，这是为了保持 fan-out 分母与最终 pass-token/closeout gate 一致，避免模型漏传子公司导致阶段假通过。
  - 但旧实现为了让 UI 立刻看到完整分母，会先把所有 org seed 成 `queued`，然后 serial loop 轮到某个 org 时才查 `org_stage_completions` 并 resume-skip 为 `passed`。因此已经通过但排在 blocked org 后面的 rows，会短暂显示成 `Queued`，看起来像要重跑。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：在初始 seed 前预查每个 org 的 `resume_skip_passed_at`；fresh PASS 的 org 直接 emit `passed` + skip 活动文案，只有真正待跑/待补的 org 才 emit `queued`。
  - 后续 serial loop 复用同一份 `resume_skips`，不再重复查询，也不会把已通过 worker 临时降级成 queued。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail` → 28 passed / 248 skipped（首次运行出现 unused warning，随后已修复）。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序多次记录 `pnpm` ignored-build approval gate（`ERR_PNPM_IGNORED_BUILDS`）会阻断全量前端安装/检查，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`agent-progress.md`。
- **结论给用户**：正常语义确实是补 blocked org；runtime 补回全树是为了完整阶段 closeout，不是要求重跑 passed org。hash/pass token 只在最终 close 阶段从 DB ledger 重算，不要求把后面的 passed worker 再排队跑一遍。

---

### 2026-06-28 · EAS 批量 liveness coverage 落库修复

- **本轮目标**：回应用户“那就修一下”并解释关机后续跑是否是真 resume；修补最新 Test1 run 中批量 `httpx -l` / `nmap -sn -iL` 探活空结果不落 `GOLISH-EAS-LIVENESS` 的问题。
- **根因**：
  - 最新 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782574914157-1/run.log` 显示 sub-agent 多次跑 `nmap -sn -iL ...` 与 `httpx -l ...` 后，gate 仍报同一批 `(asset × GOLISH-EAS-LIVENESS) never attempted`。
  - 旧后台 completion 只对 `naabu`/`masscan` 批量 PORT 和 `whatweb`/`nmap -iL` 批量 SERVICE 写 `technique_outcomes`；批量探活零输出只有 evidence，没有按 input file 每个目标写 terminal outcome。
  - `nmap -sn -iL` 是探活命令，不应被泛化的 `nmap -iL` service 分支误记为 SERVICE-FINGERPRINT。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：新增 `maybe_store_background_batch_liveness_outcomes`；后台成功完成 `httpx -l` / `nmap -sn -iL` 后读取 input file，对每个非 CIDR host/IP 写 `technique_outcomes(GOLISH-EAS-LIVENESS)`，输出命中为 `found`，无命中为 `empty`。
  - `bridge_config.rs`：新增批量命令意图分类；`nmap -sn/-sP -iL` 只走 LIVENESS，`nmap -sV/-A -iL` 才走 SERVICE-FINGERPRINT，避免探活扫描误落服务识别。
  - `bridge_config.rs`：补 `httpx -l` input-file 解析与分类回归测试。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 14 passed / 80 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`；本机前序多次记录 `pnpm` ignored-build approval gate（`ERR_PNPM_IGNORED_BUILDS`）会阻断全量前端安装/检查，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。
- **续跑解释**：当前代码通过 `latest_resumable_by_session` 找同 chat session 的 non-terminal operation，并从 `operation_state.state_blob` 恢复；`stage_run_workers[stage][org_id]` 持久化每个 org 的 sub-agent chain id，所以断电/关机重启后能继续未完成 worker，而不是只靠日志重放。
- **下一步建议**：重启 app 后继续当前 EAS；新的后台 completion 日志应出现 `background batch liveness outcomes stored`，再用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 看 LIVENESS pending cells 是否下降。

---

### 2026-06-28 · EAS submit retry 批量 service coverage 修复

- **本轮目标**：回应用户“刚刚跑了一次逻辑，为什么一直报错提交过不去”，诊断最新 Test1 run，并修掉 EAS repair/submit 在批量服务指纹阶段继续卡住的问题。
- **根因**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db` 显示本轮 submit 并不是 JSON schema 主因；最终 deterministic gate 卡在 `external_attack_surface coverage_complete`，最后仍有 130 个 cell 没有 terminal state，主要是 `GOLISH-EAS-SERVICE-FINGERPRINT`，另有部分 `LIVENESS`。
  - DB 自诊断显示本 session 已有 `GOLISH-EAS-LIVENESS found:165 / empty:33`、`GOLISH-EAS-PORT found:60 / empty:5`、`GOLISH-EAS-SERVICE-FINGERPRINT found:40`，说明工具确实跑了一部分，但 service fingerprint 分母还远未闭合。
  - run_tree 中 repair 阶段多次出现 `coverage-gap repair blocks list-file probes`；`StageRefiner` 文案要求 batch-first（`input_lines + {{input_file}}`），但 `SubmitRepairMode` 又拦 list-file/multi-target，导致模型只能单目标 nmap/whatweb，面对数百资产必然循环。
  - 批量 `whatweb --input-file` / `nmap -iL` 的后台 evidence 会存在，但旧 coverage fact 只能从命令行解析单个 target；命令行里只有 input file 路径，不能给每个 input target 写 `GOLISH-EAS-SERVICE-FINGERPRINT` terminal outcome。
- **已完成**：
  - `backend/crates/golish-sub-agents/src/executor_types.rs`：coverage-gap repair 允许 `pentest_run` 用 `input_lines` / list-file 批量处理 sibling gap targets；仍阻止 CIDR/range sweep、隐藏 list file（没给可校验 `input_lines`/`stdin`）以及 coverage_gap_actions 外的目标。
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：后台 completion 新增批量 service outcome 写点；`whatweb --input-file` / `nmap -iL` 成功完成后读取 input file，为每个 host/URL 写 `technique_outcomes(GOLISH-EAS-SERVICE-FINGERPRINT)`，输出命中为 `found`，无命中为 `empty`。
  - `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`bridge_config.rs`：补回归测试，锁住批量 input_lines 允许、隐藏 list file 阻止、`--input-file=...` 解析、service output target 匹配，以及 IP 前缀不误命中。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db > /tmp/golish-run-tree-1782574914157-full.txt` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 12 passed / 80 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair --status-level fail` → 6 passed / 104 skipped。
  - `cd backend && cargo check -p golish-agent-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序 `./init.sh` / `just install` 已稳定被 pnpm `ERR_PNPM_IGNORED_BUILDS`（`@swc/core` / `electron` / `esbuild`）阻塞，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`、`agent-progress.md`。
- **下一步最佳动作**：重启 app 后继续/重跑 EAS；repair 阶段应能用 `input_lines + {{input_file}}` 批量扫 gate 点名资产，后台 whatweb/nmap 完成后 service fingerprint terminal outcomes 应进入 `technique_outcomes`，再用 `check_stage_asset_coverage` 看 pending cells 是否下降。

---

### 2026-06-28 · SubAgent detail refiner 指令卡片化

- **本轮目标**：回应用户截图里 `Resuming submit repair: STAGE REFINER DIRECTIVE...` 被当成普通 `Agent Output` 展示，导致 detail 里系统纠错指令看起来很怪的问题。
- **根因**：
  - `golish-sub-agents` 在恢复 submit repair mode 时会发一条 `SubAgentTextDelta`，内容是 `Resuming submit repair: ... STAGE REFINER DIRECTIVE...`。
  - 前端 `SubAgentDetailView` 之前把所有 text delta 都渲染为普通 `Agent Output`，没有区分 StageRefiner / harness repair 指令和 agent prose。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：新增 `parseStageRefinerDirectiveSummary`，识别 `STAGE REFINER DIRECTIVE` 并解析 stage、repair kind、gap/action 数、allowed/forbidden tools、batch-first 标记。
  - `SubAgentDetailView`：StageRefiner directive 现在渲染成紧凑 `Stage Refiner` 修复卡，默认折叠原始长指令，只显示 `Coverage Gap` / `289 gaps` / `Batch-first` / allowed tools 等摘要。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增 3 个回归，锁定普通输出不受影响、CoverageGap 指令摘要、EvidenceRefs 指令摘要。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 submit-repair / StageRefiner directive 不应再作为普通 Agent Output 展示。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 1 file / 44 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `just check-fe` → exit 1；包装层只输出 recipe failure。
  - `pnpm check` → exit 1；底层为 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未跑；`./init.sh` / `just check-fe` 均被本机 pnpm ignored-build approval gate 阻断，本轮做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（refiner detail UI scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，截图里那类 `Resuming submit repair` 长段落应显示为一条 `Stage Refiner` 卡；点 `Details` 才展开完整 directive。

---

### 2026-06-28 · pentest_run 工具卡摘要降噪修复

- **本轮目标**：回应用户截图里卡片显示 `Running Nmap nmap -sV -iL [input file] ...`，标题和后缀重复、后缀命令过长的问题。
- **根因**：
  - 上一轮把标题从 raw tool id 改成动作文案后，`pentest_run` 的参数摘要仍然返回完整 `<tool> <args>` 命令串；因此标题 `Running Nmap` 后面又出现 `nmap ...`。
  - `SubAgentDetailView` 的 coverage 维度推断之前间接依赖摘要里出现 `-sV`；如果直接把命令串从摘要里删掉，需要让推断显式读取 raw args/action label，避免 SERVICE 维度丢失或误判。
- **已完成**：
  - `frontend/lib/tools.ts`：`getToolActionLabel("pentest_run")` 改成意图文案：`nmap -sV` → `Probing services`，`naabu/masscan` → `Scanning ports`，`httpx` → `Checking web services`，`whatweb` → `Fingerprinting web services` 等。
  - `frontend/lib/tools.ts`：`getToolPrimaryArg("pentest_run")` 不再返回完整原始命令；现在返回短上下文，例如 `Nmap · batch 3 targets (...) · ports 80,443,10180`、`Naabu · batch ... · top 1000 ports`。
  - `frontend/components/ToolExecutionCard/ToolExecutionCard.tsx`：标题也接入 `getToolActionLabel`，和聊天卡 / sub-agent detail 保持一致。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：coverage 维度推断显式看 action + raw args；隐藏 `-sV` 后仍能推断 SERVICE，同时 `Checking web services` 不会误判 SERVICE。
  - `frontend/lib/tools.test.ts`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增/更新回归，锁定 nmap service probe、naabu batch 摘要、coverage 推断。
  - `docs/modules/frontend/{components,lib}.md`：同步模块卡，记录工具卡标题用动作文案、`pentest_run` 摘要避免 `Running Nmap nmap ...`。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 3 files / 66 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx docs/modules/frontend/lib.md docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx` → exit 1；底层在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡摘要降噪 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`frontend/components/ToolExecutionCard/ToolExecutionCard.tsx`、`frontend/components/SubAgentDetailView/{SubAgentDetailView.tsx,stripAgentXmlTags.test.ts}`、`docs/modules/frontend/{components,lib}.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，截图里的行应类似 `Probing services  Nmap · batch ... · ports 80,443,10180`，而不是 `Running Nmap nmap -sV -iL ...`。

---

### 2026-06-28 · 工具卡片人类动作文案修复

- **本轮目标**：回应用户反馈工具卡片直接显示 `wait_for_background_jobs` 这类下划线内部名很难受，希望像 Cursor 一样显示“正在做什么”。
- **根因**：
  - 聊天流工具卡和 pending approval 卡片头部直接或间接展示内部 tool id；`SubAgentDetailView` 折叠工具行更是直接渲染 `tool.name`。
  - `getToolPrimaryArg` 只负责参数摘要（如 timeout / command），没有单独的人类动作文案层，导致“工具是什么”和“正在做什么”混在一起。
- **已完成**：
  - `frontend/lib/tools.ts`：新增 `getToolActionLabel`，把内部 tool id 转成动作句子；例如 `wait_for_background_jobs` → `Waiting for background jobs`，`pentest_run` + `tool_name=whatweb` → `Running WhatWeb`，未知工具也 fallback 为 `Using Custom Internal Tool` 而不是露下划线。
  - `frontend/components/AIChatPanel/ToolCallSummary.tsx`：聊天工具卡和 pending approval 卡头部改用 `getToolActionLabel`；raw tool id 只放在 `title` 里用于 hover/debug。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：sub-agent detail 折叠工具行头部改用动作文案，参数摘要继续显示在后面（如 `wait up to 180s`）。
  - `docs/modules/frontend/{components,lib}.md`：同步模块卡，记录折叠工具卡不直接展示 `snake_case` tool id。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0（fixed 1 file）。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 3 files / 63 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/lib.md docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 1；底层在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡动作文案 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`frontend/components/AIChatPanel/ToolCallSummary.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`docs/modules/frontend/{components,lib}.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，工具卡标题应显示类似 `Waiting for background jobs` / `Running WhatWeb`，下一行或旁边再显示 `wait up to 180s` / batch targets 等参数摘要；不应再把 `wait_for_background_jobs` 作为卡片主标题。

---

### 2026-06-28 · wait_for_background_jobs 折叠摘要修复

- **本轮目标**：回应用户截图里 `wait_for_background_jobs` 不展开时看不到等待秒数，并确认 `timeout_secs` 的来源。
- **结论 / 根因**：
  - 后端 `WaitForBackgroundJobsTool` 的 `timeout_secs` 是可选参数；不传时默认 `DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS=300_000`（300s），最大 900s。
  - 截图里的 `timeout_secs: 180` 是模型本次实际传入的参数，不是前端默认值；前端之前只在展开 Input 后显示完整 args，折叠摘要没有 `wait_for_background_jobs` 分支。
- **已完成**：
  - `frontend/lib/tools.ts`：`getToolPrimaryArg("wait_for_background_jobs", args)` 现在返回折叠态摘要：传参时显示 `wait up to 180s`，未传时显示 `default wait up to 300s`，自定义 `poll_interval_ms` 时追加 `poll ...ms`。
  - `frontend/lib/tools.test.ts`：新增 3 个回归，锁定显式 timeout、默认 timeout、poll interval 的折叠摘要。
  - `docs/modules/frontend/lib.md`：同步模块卡，记录 `wait_for_background_jobs` 折叠态必须显示实际 timeout / 默认 300s。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts` → 1 file / 11 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts docs/modules/frontend/lib.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `just fmt-fe` / `just check-fe` / `just test-fe` → exit 1；just 包装层只输出 recipe failure。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` / `pnpm test:run frontend/lib/tools.test.ts` → exit 1；底层均在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（wait 折叠摘要 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，`wait_for_background_jobs` 折叠行应显示 `wait up to 180s`；若模型没传 timeout，则显示 `default wait up to 300s`，方便直接判断是模型自填还是默认。

---

### 2026-06-28 · EAS 批量扫描 coverage 落库与 live 匹配修复

- **本轮目标**：回应用户截图里 EAS 资产覆盖显示 `naabu -list [input file] ... | batch 226 targets` 正在跑，但覆盖面板显示 `0 组 / 0 资产`，并且“扫描完了提交过不了”的问题。
- **根因**：
  - 最新 Test1 run 的 submit gate 明确因为 EAS coverage incomplete BLOCK：仍有大量 `(asset × LIVENESS/PORT/SERVICE-FINGERPRINT)` 格子是 never attempted；这不是单纯前端显示问题。
  - 前端 `SubAgentDetailView` 的 live work 匹配只看命令文本/单目标参数，没读 `pentest_run.input_lines` / `stdin`；批量命令里目标都在输入文件/批量参数中，所以 UI 只能显示“运行中但尚未匹配到资产行”。
  - 后台 job completion 之前只拿 8KB `stdout_tail` 做 structured output 解析；批量 `naabu` / `whatweb` 这类长输出可能已经 append evidence，但完整结果没有写回 targets/ports/fingerprints，coverage truth 仍缺。
  - `naabu -silent` 对无开放端口资产是零输出；旧逻辑没把 input file 中“扫过但无结果”的 host/IP 写入 `technique_outcomes`，gate 会把它们当 never attempted，而不是 checked-empty。
- **已完成**：
  - `frontend/lib/tools.ts` 导出 `getPentestRunInputLines`；`SubAgentDetailView` live coverage 资产提取复用它，能从 `pentest_run.input_lines` / `stdin` 展开批量资产。
  - `backend/crates/golish-core/src/agent_session.rs` 的 `AgentToolContext` 增加 `organization_id`；主 agent 用 `harness_org_id`，sub-agent 用 `active_org_id_override`，后台 job completion 继承该 org。
  - `background_jobs::JobCompletion` 增加 `organization_id`；`bridge_config.rs` 的后台 structured landing 优先读取 `background_jobs::manager().snapshot(job_id).stdout`，fallback completion tail，并调用 `maybe_detect_and_store_via_context` 带 org context。
  - `bridge_config.rs` 对成功完成的 `naabu -list` / `masscan -iL` 批量端口扫描读取 input file，把每个非 CIDR host/IP 的 `GOLISH-EAS-PORT` outcome 写入 `technique_outcomes`：有开放端口 `found`，无开放端口 `empty`。
  - 同步模块卡：`docs/modules/frontend/{components,lib}.md`、`docs/modules/backend/{golish-core.md,golish-app-core.md,golish-agent-app/ai.md,golish-agent-runtime/agentic_loop.md,golish-sub-agents/executor.md}`。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check frontend/lib/tools.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 49 tests passed。
  - `cd backend && cargo fmt -p golish-core -p golish-app-core -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-app-core background_jobs --status-level fail` → 13 passed / 31 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 9 passed / 80 skipped。
  - `cd backend && cargo check -p golish-core -p golish-app-core -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit`（本工作区已有大量未提交跨模块改动，且前序记录显示 `./init.sh`/pnpm install 被 ignored-build approval gate 阻断；本轮做 scoped 前后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（EAS batch coverage landing scope）**：`frontend/lib/tools.ts`、`frontend/components/SubAgentDetailView/{SubAgentDetailView.tsx,stripAgentXmlTags.test.ts}`、`backend/crates/golish-core/src/agent_session.rs`、`backend/crates/golish-app-core/src/background_jobs.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、上述模块卡、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑/继续 EAS；正在运行的 `naabu -list [input file]` 应能在资产覆盖里匹配到批量资产，后台完成后 PORT 的 found/empty outcome 会落入 `technique_outcomes`。SERVICE-FINGERPRINT 对“无开放端口”的 not_applicable 仍依赖 gate/submit 语义后续扩展或模型自报 note，不在本轮 DB 投影里伪造。

---

### 2026-06-28 · pentest_run 批量输入摘要修复

- **本轮目标**：回应用户截图里 `naabu -list {{input_file}} -top-ports 1000 -s c -silent` 连续显示 4 次，解释是否重复执行，并修掉 UI 摘要误导。
- **根因**：
  - 最新 Test1 transcript 里 4 次 `naabu` 并不是同一批目标：`input_lines` 分别为 96 / 76 / 55 / 34 条；实际执行时后端也替换成了不同的 `.golish/tool-inputs/pentest-input-*.txt` 临时文件。
  - 前端工具卡共用 `getToolPrimaryArg`，之前只显示 `tool_name + args`，没有显示 `input_lines` / `stdin`，所以所有 list-file 批量命令都看起来像同一条模板命令重复跑。
- **已完成**：
  - `frontend/lib/tools.ts`：`pentest_run` 摘要现在会统计 `input_lines` / `stdin`，显示 `batch N targets (first ... last)`；带批量输入时把 `{{input_file}}` / `{{targets_file}}` / `{{hosts_file}}` / `{{urls_file}}` / `{input_file}` / `$GOLISH_INPUT_FILE` 展示为 `[input file]`。
  - `frontend/lib/tools.test.ts`：补 `naabu -list {{input_file}}` 和 `httpx stdin` 批量摘要回归。
  - `docs/modules/frontend/lib.md`：同步模块卡，记录工具卡共享摘要入口的 batch 展示规则。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts` → 1 file / 8 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts docs/modules/frontend/lib.md agent-progress.md` → exit 0。
- **未跑**：`just precommit`（本机 `./init.sh` / `pnpm install` 仍受 `ERR_PNPM_IGNORED_BUILDS` 策略阻断；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡批量摘要 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，同样的 `naabu` 批量卡应显示类似 `naabu -list [input file] ... | batch 96 targets (113.105.78.99 ... 120.233.149.95)`，不再误以为同一命令重复执行。

---

### 2026-06-28 · stage_run 续跑 org 子树补齐修复

- **本轮目标**：回应用户指出的“继续逻辑有问题，有时 `stage_run` 总是少几个资产”，定位续跑/repair 阶段为什么漏部分 org/资产，并做 runtime 侧兜底。
- **根因**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db` 显示同一 run 中 `target_intel` 的 `stage_run` 入参有 12 个 org；后续 EAS continuation 的 `stage_run` 入参只有 10 个 org；repair 轮进一步只剩 6 个 org。
  - 旧 `stage_run_call.rs` 在已绑定 `harness_org_id` 时只会把 subtree 外的 org 丢掉，但不会把模型少传的 subtree 内 org 补回来；续跑/修复轮一旦靠模型重建 `orgs` 数组，就会让部分子公司及其资产完全不进入 fan-out 分母。
- **已完成**：
  - `golish-agent-kit::db_traits` 新增 `OrgScopeUnit` 与 `org_subtree_units` trait，保留默认 fallback 给测试 double。
  - `golish-db::repo::organizations::subtree` 新增 read-only recursive CTE，返回 root + descendants 完整 organization 行；无 schema / migration。
  - `golish-agent-app` 的 DB bridge 通过 `organizations::subtree` 实现 `org_subtree_units`。
  - `stage_run_call.rs` 在 `harness_org_id` 已绑定时以 DB organization subtree 作为权威 fan-out 集合：模型传入 `orgs` 只保留 ownership hint；缺失的 subtree org 会自动补回，subtree 外 org 会记录并拒绝；工具返回新增 `scope_source` / `requested_orgs` / `auto_added_orgs` / `rejected_orgs`。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-db.md`；`docs/modules/INDEX.md` 状态仍为 ✅，无需状态列变更。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db > /tmp/golish-run-tree-1782574914157.txt` → exit 0；证据显示同 run 内 `stage_run` org 入参从 12 → 10 → 6。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime authoritative_subtree_fills_missing_requested_orgs --status-level fail` → 1 passed / 275 skipped。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail` → 28 passed / 248 skipped。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db --all-targets -- -D warnings` → exit 0。
  - `python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 已在 pnpm install/build approval gate 阶段失败；本轮做 scoped Rust/JSON/doc 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（stage_run 续跑 org 子树补齐 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,recon.rs}`、`backend/crates/golish-db/src/repo/organizations.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-db.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app 后用同一个 Test1 engagement 继续跑 EAS；观察 `stage_run` tool result 的 `scope_source` 应为 `engagement_org_subtree`，`total_orgs` 应回到 DB root subtree 数量，即使模型只传 blocked org 或少传子公司，也会通过 `auto_added_orgs` 补齐。

---

### 2026-06-28 · EAS 批量探测路径修复

- **本轮目标**：回应用户发现 EAS 阶段 `httpx` 等工具被模型一个个调用、没有利用工具批量能力的问题。
- **根因**：
  - EAS `methodology.md` 已写 `httpx` 应批量跑，但 prober fallback prompt、primary stage 描述和 StageRefiner coverage-gap repair hint 仍会把 gap 拆成 `httpx -u <asset>` / `naabu -host <asset>` / `nmap ... <asset>` 这类单资产提示。
  - `resources/toolsconfig/httpx.json` 的 skills 推荐 `-json`，但 output config 仍是 `format=text`；批量 JSONL 输出可能出现“工具跑了但 parser 不解析、不落 targets/fingerprints”，导致 DB truth 缺口继续触发补洞。
  - `naabu` / `nmap` toolsconfig 没显式暴露 `-list` / `-iL` 批量参数和 bulk skills，模型看不到一等批量入口。
  - 更深一层：`pentest_run.args` 本来不是固定参数，但 `pentest_list_tools` 只把 `skills[].args` 暴露给模型，没暴露完整 `params`；同时 `pentest_run` 没有结构化 `stdin/input_lines`，导致模型即使想 batch 也很容易退化成一资产一调用。
  - `naabu` / `masscan` / `nmap` / `whatweb` / `gowitness` 这类工具的批量入口多是 list-file 参数，不是纯 stdin；之前没有 `{{input_file}}` 这类运行期占位，AI 没法可靠创建 hosts.txt，仍会抄单目标 recipe。
- **已完成**：
  - `resources/toolsconfig/httpx.json`：改为 `output.format=json_lines`，补 JSONL fields 映射（`ip` 取 `a[0]`，避免把 IP 数组字符串写进 `real_ip`），并保留旧文本 pattern fallback；批量 skills 改为带 `-json -sc -title -td -server`。
  - `resources/toolsconfig/naabu.json` / `nmap.json`：显式加入 `-list` / `-iL` 参数与 batch skills，batch skill 统一使用 `{{input_file}}`。
  - `resources/toolsconfig/masscan.json` / `whatweb.json` / `gowitness.json`：补 list-file batch 参数与 bulk skills，覆盖 EAS 端口发现、Web 指纹、截图工具，不只修 `httpx`。
  - `backend/crates/golish-pentest-app/src/pentest_ai/list_tools.rs`：`pentest_list_tools` 现在返回 `params`、`batching`、`usage_hint`，明确 skills 是示例 recipe，不是固定调用签名；bulk skills 会排在前面并带 `batch: true`。
  - `backend/crates/golish-pentest-app/src/pentest_ai/run.rs`：`pentest_run` schema 增加 `stdin` / `input_lines`；无 `{{input_file}}` 时用 quoted heredoc 喂 stdin，有 `{{input_file}}` 时自动写 workspace `.golish/tool-inputs/` 临时目标文件并替换占位符，支撑 `naabu -list` / `masscan -iL` / `nmap -iL` / `whatweb --input-file` / `gowitness file -f`。
  - `resources/harness/stages/external_attack_surface/methodology.md`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/{prompts/mod.rs,subtask_phases/execute.rs}`：统一 EAS/prober 为 batch-first 口径，primary 通过 `stage_run` 扇出 prober，并明确 list-file 工具使用 `{{input_file}} + input_lines`。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`：EAS coverage-gap directive 现在按 technique 聚合同类 gap，明确要求少量批量 `pentest_run`，并把 command hints 从单资产命令改成 batch hints。
  - `backend/crates/golish-pentest/src/output_parser.rs`：新增回归测试，锁定真实 `resources/toolsconfig/httpx.json` 同时解析 JSONL 和旧文本 fallback。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（延续本机 pnpm install gate）。
  - `python3 -m json.tool resources/toolsconfig/httpx.json` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/naabu.json` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/nmap.json` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-pentest` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-pentest -- --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_toolsconfig_parses_jsonl_and_text_fallback --status-level fail` → 1 passed / 153 skipped。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_json_parse --status-level fail` → 1 passed / 153 skipped。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_toolsconfig_parses_jsonl_and_text_fallback test_httpx_json_parse --status-level fail` → 2 passed / 152 skipped（补充锁定 `ip=a[0]` 解析为单 IP）。
  - `cd backend && cargo nextest run -p golish-agent-kit eas_coverage_gap_instruction_is_batch_first --status-level fail` → 1 passed / 759 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit external_attack_surface_charter_surfaces_liveness_technique --status-level fail` → 1 passed / 759 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents test_prober_prompt_is_active_surface --status-level fail` → 1 passed / 108 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills input_lines_become_stdin_payload stdin_payload_wraps_command_in_quoted_heredoc heredoc_delimiter_avoids_payload_collision --status-level fail` → 4 passed / 87 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills input_lines_become_stdin_payload stdin_payload_wraps_command_in_quoted_heredoc heredoc_delimiter_avoids_payload_collision input_file_placeholder_writes_target_file input_without_file_placeholder_uses_stdin shell_quote_handles_single_quotes --status-level fail` → 7 passed / 87 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills --status-level fail` → 1 passed / 93 skipped（锁定 bulk skills 排序 + `batch` 标记）。
  - `cd backend && cargo nextest run -p golish-agent-kit eas_coverage_gap_instruction_is_batch_first external_attack_surface_charter_surfaces_liveness_technique --status-level fail && cargo nextest run -p golish-sub-agents test_prober_prompt_is_active_surface --status-level fail` → 3 tests passed（锁定 `naabu` / `nmap` / `whatweb` / `gowitness` 的 `{{input_file}}` 批量提示）。
  - `cd backend && cargo check -p golish-pentest -p golish-agent-kit -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app -p golish-pentest -p golish-agent-kit -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-pentest -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-agent-kit -p golish-sub-agents -p golish-pentest -- --check` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/httpx.json >/dev/null && python3 -m json.tool resources/toolsconfig/naabu.json >/dev/null && python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null && python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/httpx.json >/dev/null && python3 -m json.tool resources/toolsconfig/naabu.json >/dev/null && python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null && python3 -m json.tool resources/toolsconfig/masscan.json >/dev/null && python3 -m json.tool resources/toolsconfig/whatweb.json >/dev/null && python3 -m json.tool resources/toolsconfig/gowitness.json >/dev/null && python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `rg -n "{{hosts}}|{{urls}}|-host {{target}}|{{input_file}}" resources/toolsconfig/{httpx,naabu,nmap,masscan,whatweb,gowitness}.json backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs resources/harness/stages/external_attack_surface/methodology.md` → 只剩单目标 skills 仍含 `-host {{target}}`，所有 batch skills/prompt 均使用 `{{input_file}}`。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 仍在 pnpm install gate 失败；本轮做 scoped Rust/JSON 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（EAS 批量探测 scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/{prompts/mod.rs,stage_refiner.rs,subtask_phases/execute.rs}`、`backend/crates/golish-pentest-app/src/pentest_ai/{list_tools.rs,run.rs}`、`backend/crates/golish-pentest/src/output_parser.rs`、`backend/crates/golish-sub-agents/src/defaults/{prompts/execution_planning.rs,tests.rs}`、`resources/toolsconfig/{httpx,naabu,nmap,masscan,whatweb,gowitness}.json`、`resources/harness/stages/external_attack_surface/methodology.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`agent-progress.md`、`feature_list.json`。
- **下一步建议**：重启 app 后重新跑 EAS；观察 prober 是否先调用 `pentest_list_tools` 读取 `params/batching`，再用少量 `pentest_run(args=..., input_lines=[...])`：`httpx` 可 stdin/`-l {{input_file}}`，`naabu` 用 `-list {{input_file}}`，`masscan`/`nmap` 用 `-iL {{input_file}}`，`whatweb` 用 `--input-file={{input_file}}`，`gowitness` 用 `file -f {{input_file}}`。

---

### 2026-06-28 · 资产覆盖运行态页面跳动修复

- **本轮目标**：回应用户截图里完整资产覆盖页运行时顶部/当前资产区域一直跳动、刷新感很强的问题。
- **根因**：
  - `StageAssetCoveragePanel` 之前只在 live work 全部清空时短暂保留上一帧；如果运行中的 work item 切换、事件批次短暂漏掉某个 item，active slice 会立即缩小/扩大，导致「正在做的资产」区域频繁重排。
  - 资产覆盖 summary chips、live count、顶部运行状态条和外层 panel header 的计数都依赖内容宽高自适应；running badge / 数字出现消失时会推动同一行元素位置，看起来像整块在刷新。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：新增 `mergeDisplayLiveWorkItems`，运行中 work 切换时按 id 合并 incoming + 上一帧 display，并用 `LIVE_WORK_RETENTION_MS=3500` 延迟裁剪消失的 item；短暂轮询空隙不再让 active rows 立刻闪空或换位。
  - 同文件把完整矩阵 header、`LiveFocusBar`、panel/collapsible header 的 summary/live chips 改成固定高度 / 最小宽度 / `tabular-nums` 槽位；live count 为 0 时保留 invisible 槽，避免右侧 `Live` / summary 位置左右跳。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增回归，锁定 live work 从资产 A 切到资产 B 时短窗口内保留 A+B，窗口后再裁剪旧资产；更新旧保留窗口测试使用导出的常量。
  - `docs/modules/frontend/components.md`：同步模块卡，记录完整资产覆盖页运行态必须保留上一帧 active slice 并使用固定槽位。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate 延续既有环境问题）。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 13 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
- **未跑**：`just precommit` / `just check-fe` / `just test-fe`（本机 pnpm wrapper 当前被 `ERR_PNPM_IGNORED_BUILDS` 阻断：`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（资产覆盖跳动修复 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入 EAS 资产覆盖完整矩阵，运行中顶部 `15/332 done` / live count / `Live` 和下方「正在做的资产」区域应只更新内容，不再反复挤动整块布局。

---

### 2026-06-28 · ask_human Confirm 后卡片残留修复

- **本轮目标**：回应用户截图里进入 `external_attack_surface` 后，阶段边界 `AI Needs Your Input` 点 Confirm 仍残留的问题。
- **根因**：
  - `AIChatPanel` 同时渲染 hook 本地 `askHumanRequest` 和全局 `pendingAskHuman` store 兜底；同一个 `ask_human_request` 可能同时被 hook 和 app-level AI event pipeline 记录。
  - 点 Confirm 走本地 hook 分支时只清了本地态，没有同步清 store；下一帧 `visibleAskHumanRequest` 又从 store 兜底拿到同一 `requestId`，所以卡片看起来“确认了还有”。
- **已完成**：
  - 新增 `frontend/components/AIChatPanel/askHumanStore.ts`，按 `requestId` 清理同一 ask_human 请求在 AI session / terminal session / conversation key 下的 store 副本；不会误清同 session 上更晚的新 prompt。
  - `frontend/components/AIChatPanel/AIChatPanel.tsx` 的 Confirm / Skip 两条路径都在 finally 里清理匹配的 store 副本；store-only 兜底路径仍直接响应对应 session。
  - 补 `frontend/components/AIChatPanel/askHumanStore.test.ts` 回归；同步 `docs/modules/frontend/components.md` 模块卡。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate 延续既有环境问题）。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/askHumanStore.test.ts frontend/components/AIChatPanel/AskHumanInline.test.tsx frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx` → 3 files / 42 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just check-fe` / `just test-fe` → exit 1；底层 `pnpm check` / `pnpm typecheck` / `pnpm test:run ...` 均在执行脚本前被 `ERR_PNPM_IGNORED_BUILDS` 阻断（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑**：`just precommit`（`./init.sh` / `just check-fe` / `just test-fe` 已在 pnpm install/build-approval gate 阶段失败；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（ask_human 残留修复 scope）**：`frontend/components/AIChatPanel/AIChatPanel.tsx`、`frontend/components/AIChatPanel/askHumanStore.ts`、`frontend/components/AIChatPanel/askHumanStore.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新/重启前端后复测阶段边界 prompt；点 Confirm 后卡片应立即消失，EAS 阶段继续跑。

---

### 2026-06-27 · Scoping REUSE 扩树导致资产爆炸诊断与门禁修复

- **本轮目标**：回应用户“怎么越搞资产越多、很多乱七八糟的”，复盘前一次 run 为什么从平安 scope 膨胀到大量资产，并修复 scoping REUSE mode 被硬门禁逼着重复 create 的问题。
- **日志证据 / 根因**：
  - 最新相关 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782571959315-1/`。
  - `run_tree.py --full` 显示 scoping 明明识别为 REUSE mode，但仍执行 `manage_organizations(action="create_batch")`，一次新增/复用 18 个子公司；后续 org tree 变成 27 个 org。
  - 根因是 prompt/gate 冲突：`resources/harness/stages/scoping/methodology.md` 写着 “REUSE mode: do NOT re-create”，但 `prompts/mod.rs` / `execute.rs` 的红队硬门禁仍写死 “必须 propose_candidates → unit_review → manage_organizations(create)”。
  - 另一个放大器：`golish-db::repo::tool_calls::scoping_actions_for_session` 只统计 `action='create'`，不统计推荐的 `create_batch`。模型用 `create_batch` 批量扩树后，gate 审计还可能认为没有 create，诱发更多纠错/重跑。
  - target_intel 阶段对 27 个 org 扇出；部分 org 的 provider survey 极大，例如 root org 记录 `subdomains=143 / subdomain_hosts=676`，root summary 自述 “499+ in-scope assets”；平安证券自述注册 168 targets；后续又出现 `blocked-org-1` 占位补跑并注册 158 assets。资产多不是单纯“扫出来了”，而是 scoping 扩树 + 大 org passive provider 泛匹配 + retry 占位补跑共同放大。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`：scoping charter 改为 REUSE mode 下不要为了 gate 调 `create`/`create_batch`；只有 root 缺失或用户显式新增/确认单位时才创建。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：red-team scoping anti-shortcut gate 改为只强制真实 `unit_review`；已有 org tree 经人审确认即可通过，不再因缺 create BLOCK。
  - `backend/crates/golish-db/src/repo/tool_calls.rs`：`create_batch` 的 `created` / `existing` id 也会被 scoping action audit 识别为真实组织记录，避免未来真正批量新增后被误判。
  - `backend/crates/golish-agent-kit/src/db_traits/repo.rs`：同步 trait 注释，明确 `organization_created` 对 REUSE mode 只是 audit 信息，不是必须条件。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-db.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit -p golish-db`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit red_team_scoping_flow --status-level fail`（cwd `backend`）→ 1 passed / 758 skipped。
  - `cargo nextest run -p golish-db create_result create_batch_result --status-level fail`（cwd `backend`）→ 4 passed / 108 skipped。
  - `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-db -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`（本机 pnpm ignored-builds/install gate 仍是全量前置阻塞；本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（scoping reuse gate scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-db/src/repo/tool_calls.rs`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-db.md`、`agent-progress.md`。
- **下一步建议**：数据库清空后重启 app 再跑“搞一下平安”。第一轮如果 root 不存在，可以按 unit_review 新建确认的 org；之后再次跑同一 root 时应该只复用/确认现有树，不应再自动 `create_batch` 扩到几十个 org。若还出现单 org 落几百资产，下一刀应收紧 `recon_map_assets` provider result 的 ownership/domain relevance threshold。

---

### 2026-06-27 · StageRun active-stage completion floor + pass-token submit preview

- **本轮目标**：回应用户“最后一次日志还是过不去”，诊断最新 run 的 target_intel submit loop，并修复新一轮 active stage 被旧 `org_stage_completions` 短路的问题。
- **日志证据 / 根因**：
  - 最新真实 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782570596001-2/`。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782570596001-2 --full --db` 显示 scoping 已 PASS，`operation_state.engagement_org_id=e51a6ae1-c6f4-4dc7-9c57-d8263d9fc107`，`current_stage=target_intel`，`stage_started_at=2026-06-27 22:31:32 +0800`。
  - 但 target_intel `stage_run` 8/8 org 都从旧 completion 跳过：completed at `2026-06-27 13:25 UTC` 等，早于本次 `stage_started_at=2026-06-27 14:31 UTC`；所以本轮没有新的 worker/evidence/source rows。
  - 随后的 `check_stage_asset_coverage` 仍有 `pending_cells=2119`，`source_query_log: none for this run`；`submit_stage_deliverable` 一直被 `coverage_complete` / `source_coverage` 打回，不是 askman 没走，也不是 scoping root 没绑。
  - 另一个下游症状：主 agent 提交 `stage_run_pass_token` claim 时，submit preview 先按普通 claim 要 evidence，返回 `every claim must cite evidence`，导致它继续乱补 invalid skipped_check；该 pass token 应由 final fan-out closeout 从 DB ledger 重算验证。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：新增 `completion_is_fresh_for_stage`，在 TTL 之外支持 active-stage `not_before` floor；补单测锁定旧 completion 不能跨 stage start 复用。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：`stage_run` resume-skip、pass-token generation 都使用当前 `operation_state.current_stage == stage` 时的 `stage_started_at` floor；旧 completion 不再短路当前 active stage worker。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：fan-out closeout 验 pass_token 时同样用 current active-stage floor 过滤 `org_stage_completions`，避免旧 ledger 生成当前 token。
  - `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`：specialist stage 的 `stage_run_pass_token` claim 在 submit preview 阶段只做结构/伪造 evidence-id 检查并收进 side-channel；最终由 orchestrator closeout 重算 DB token 判定。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`。
  - `feature_list.json`：更新 operation-continuity evidence / verification，状态仍 `in_progress`（全量 precommit 仍受 pnpm install gate 阻塞）。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit completion_fresh_for_stage fanout_completion_scope --status-level fail`（cwd `backend`）→ 4 passed / 755 skipped。
  - `cargo nextest run -p golish-agent-runtime resume_skip_floor active_stage_skip_floor --status-level fail`（cwd `backend`）→ 2 passed / 273 skipped。
  - `cargo nextest run -p golish-agent-app stage_run_pass_token --status-level fail`（cwd `backend`）→ 1 passed / 86 skipped。
  - `cargo check -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
  - `python3 -m json.tool feature_list.json` → exit 0。
- **未跑**：`just precommit`（前面 `./init.sh` 已在 `pnpm install --silent` / ignored-builds gate 卡住：`@swc/core`、`electron`、`esbuild`；本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（active-stage completion floor scope）**：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑这条 operation；target_intel 的 `stage_run` 不应再显示“已完成于 13:25 UTC 跳过重跑”，而应实际 dispatch worker 或只跳过本次 `stage_started_at` 之后新写的 completion。拿到新的 pass_token 后，`submit_stage_deliverable` 应先 accepted，再由 closeout 重算 DB ledger 判定。

---

### 2026-06-27 · Continuity rootless adoption 全库污染修复

- **本轮目标**：回应用户“最后一次日志一直跑不通”，诊断最新复用流程为什么又卡住，并修复没有绑定 engagement root 时跳过 scoping 导致的全库污染。
- **日志证据 / 根因**：
  - 最新真实 session 是 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782566555331-1/`；默认 `run_tree.py` 会先捞到 title-gen，因此本轮指定了 `pentest-chat-1782566555331-1`。
  - 这次 AskHuman 正常：transcript 里有 `ask_human_request`，用户选择了“复用已有数据继续”。
  - `stage_run` 也没有再全 org skip：run.log 里出现 continuity entry stage 的 resume-skip floor，worker 实际跑起来了。
  - 当前 blocker 是新的：`operation_state.engagement_org_id = NULL`，复用 scoping 后没有把“中国平安”的 root org 绑定进 operation。于是 `list_in_scope_targets` / pass-token closeout / coverage preflight 都落到 legacy 全库口径。
  - 结果 first `target_intel` stage_run 对 13 个平安 org passed，但 `pass_token=null`；后续 main agent 从全库历史资产里挑了 `AngularDocs` / `JsRuleFilter8080` / `example.org` / `8.138.179.62:8080` 等目标继续补洞，gate 一直报这些测试资产缺 `GOLISH-INTEL-*` 终态，不是平安本身没采完。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/continuity.rs`：没有 `engagement_root` 时，`scoping` summary 改为 `Missing`，明确要求先跑 scoping 绑定当前任务；不再把 legacy `in_scope_org_ids(None)` 当作可安全 adopt 的 scope。
  - 同文件新增 `non_empty_adoption_cursor`，如果没有任何前缀 stage 真正能被 adopt，就不弹 continuity 选择框，避免“问复用但实际从 scoping 开始”的误导。
  - 补单测：无 root 的 scoping 不能 adopt；有 root 才能复用 scoping；即便后续 `target_intel` completion fresh，只要 scoping/root 缺失，也不会生成空 adoption plan。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/{task_orchestrator,harness}.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit continuity --status-level fail`（cwd `backend`）→ 10 passed / 748 skipped。
  - `cargo check -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`（此前同日 `./init.sh` / `just install` 已被本机 pnpm `ERR_PNPM_IGNORED_BUILDS` 卡住：`@swc/core` / `electron` / `esbuild` build scripts 未 approve；本轮做 scoped 后端验证）。
- **提交记录**：待提交。
- **已知风险或未解决问题**：
  - 运行中的 app 需要重启后才会加载这次 Rust 改动。
  - 这刀是 fail-safe：没有 root 时不跳 scoping。更好的后续增强是从用户目标文本/旧 scope 精确解析出唯一 root org 后，再允许带 root 的 continuity adoption。
  - 当前工作树已有大量非本轮脏改动，本轮未回退或清理。
- **下一步最佳动作**：重启 app 后重新发“搞一搞平安”。如果没有可靠 root，系统应直接进入 scoping 重新绑定 root；如果未来传入 root，才允许安全跳过 scoping。用 `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 <session> --full --db` 确认 `operation_state.engagement_org_id` 不再为 NULL。

---

### 2026-06-27 · continuity 复用确认走 ask_human 卡片

- **本轮目标**：回应用户截图里 DB progress 复用确认显示成普通 Golish AI 文本、没有走 ask_human/AskHumanInline 的问题。
- **根因**：
  - `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs` 的 continuity preflight 在 `AskBeforeReuse` 且发现 `ContinuityAdoptionPlan` 时，直接 `render_continuity_offer()` + `emit_immediate_task_response(... Completed ...)` 后 `return Ok(message)`。
  - 这条路径没有 `CoordinatorHandle::register_approval`，也没有发 `AiEvent::AskHumanRequest`；前端只有收到 `ask_human_request` 事件才会渲染 `AskHumanInline`，所以截图里只出现普通 assistant 文本。
- **已完成**：
  - `commands/core/chat.rs`：continuity ask-before-reuse 改为有 coordinator 时注册 approval、发 `AiEvent::AskHumanRequest(input_type="choice")`，等待用户选择；选择“复用已有数据继续”才把 `ContinuityAdoptionPlan` 交给 orchestrator，选择“重新开始”/Skip/timeout 走 `start_fresh`。
  - 同路径保留无 coordinator 的文本 fallback（单测/降级环境）。
  - 因前端现有 `choice` 会自动提交第一个选项，选项顺序用“重新开始”在前，避免静默复用旧 DB facts。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步模块卡，明确 continuity preflight 必须走共享 ask_human/approval coordinator。
  - `feature_list.json`：给 `operation-continuity-adoption-2026-06-27` 追加本轮 scoped evidence，状态仍 `in_progress`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm ignored-builds/install gate，延续此前环境限制）。
  - `cargo fmt -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-app chat_title_tests --status-level fail`（cwd `backend`）→ 22 passed / 64 skipped。
  - `cargo nextest run -p golish-agent-app start_operation continuity --status-level fail`（cwd `backend`）→ 8 passed / 78 skipped。
  - `cargo check -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/core/chat.rs` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 仍被 pnpm install gate 卡住；当前工作树已有大量前序未提交改动，本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（continuity ask_human scope）**：`backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启/刷新 app 后重新触发 fresh Task/Profile operation；发现旧 DB progress 时应出现 `AI Needs Your Input` 的 choice 卡片，而不是普通 Golish AI 文本。点“复用已有数据继续”后才采用旧 DB facts 并从第一个未满足 stage 接着跑。

---

### 2026-06-27 · 信息收集阶段 progress 路由修复

- **本轮目标**：回应用户发现 EAS 完成后跳到 `reporting` 而不是 `enumeration`；确认信息收集阶段不应靠 `findings` 判断进展。
- **日志证据 / 根因**：
  - 最新 session `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782488490399-1`：EAS gate `2026-06-27T11:20:36Z` PASS 后，下一 turn `2026-06-27T11:20:55Z` 进入 `reporting` 的 `Final Report Compilation`。
  - `check_stage_asset_coverage` 曾明确显示 EAS 有 `615/825` done、`210` pending；后续补 blocked cells 后 PASS，说明不是 UI 误显，而是 graph-flow 路由走了 `external_attack_surface -> reporting` 短路。
  - 代码根因：`consume_gate_outcome` 把 `made_progress` 写死为 `outcome.findings_count > 0`；而 `external_attack_surface` / `target_intel` / `enumeration` 都是 `findings_allowed=false` 的信息收集/覆盖矩阵阶段，正常交付就是 `findings=[]`。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：新增 `gate_outcome_made_progress`，blocked outcome 永不算进展；vuln 阶段继续用 `findings_count`；`findings_allowed=false` 的 recon/info 阶段改按 evidence refs、handoff summary、engagement org binding 判断有无阶段产出，避免 EAS 因无 findings 跳 `reporting`。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`：新增回归，锁定 EAS 无 findings 但有 evidence refs 算进展；`vuln_triage` 无 findings 不算进展。
  - `docs/modules/backend/golish-agent-kit/task_orchestrator.md`：同步模块卡，记录 graph-flow progress 不能再 findings-only。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm install gate，延续此前环境限制）。
  - `cargo fmt -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit info_stage_evidence_counts_as_progress_without_findings vulnerability_stage_without_findings_is_not_progress --status-level fail`（cwd `backend`）→ 2 passed / 747 skipped。
  - `cargo nextest run -p golish-agent-kit pass_emits_stage_passed_progress block_emits_no_stage_passed --status-level fail`（cwd `backend`）→ 2 passed / 747 skipped。
  - `cargo check -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs docs/modules/backend/golish-agent-kit/task_orchestrator.md agent-progress.md` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 已被 pnpm install gate 卡住；当前工作树已有大量前序未提交改动，本轮做 scoped backend 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（progress routing scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`agent-progress.md`。
- **下一步建议**：重启/刷新 app 后，EAS PASS 且有 evidence/handoff 时 graph-flow 应走主路 `enumeration`；当前已停在 `reporting` 的旧 operation 仍是旧 checkpoint 状态，需重新跑或修复 operation cursor 才能从 `enumeration` 接上。

---

### 2026-06-27 · SubAgent detail 资产覆盖二级视图

- **本轮目标**：回应用户看完整左右布局后的确认：右侧已经是 ChatPanel，资产覆盖不做右侧 drawer；在左侧 `SubAgentDetailView` 内改成 summary 进入的轻量二级视图，默认保持 Codex 风格的干净运行流。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：默认运行流只显示任务块、轻量资产覆盖 summary strip、Thought/Agent Output/tool call 时间线；去掉「运行流 / 资产覆盖」两个大 tab，点 summary 进入完整矩阵，避免矩阵挤占 agent 叙事流。
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：`StageAssetCoverageBlock` 增加 `summary` / `panel` 呈现模式；summary 模式只加载并显示 done/live/current-tool 摘要，不渲染矩阵，右侧只留小箭头；panel 模式渲染完整 coverage matrix，并在卡 header 右侧提供小号「运行流」返回按钮，避开页面左上角返回上级 Agent。独立 coverage view 改为占满 detail 内容区、列表自身滚动，不再显示底部拖拽高度 handle。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增回归，锁定 summary 模式不渲染矩阵且点击进入覆盖视图、panel 模式渲染完整矩阵、小号返回动作，并确认独立 panel 不显示高度调节控件。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 coverage matrix 不能再 inline 展开在运行流里，也不要铺两个大 tab；完整矩阵由 summary 进入、卡内小按钮返回运行流，独立覆盖视图不显示高度拖拽控件。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm install gate，延续此前环境限制）。
  - `./node_modules/.bin/biome format --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0（fixed import order）。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0（2 files / 51 tests passed）。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit` / `just check-fe` 全量（`./init.sh` 仍被 pnpm install gate 卡住；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（coverage detail view scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入正在运行的 EAS/target_intel specialist detail；默认应只看到一条资产覆盖摘要和下方 Thought/Output/tool stream；点击摘要进入完整矩阵，点矩阵 header 右侧小号「运行流」返回时间线。
