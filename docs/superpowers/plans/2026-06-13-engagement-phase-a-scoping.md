# Engagement Phase A — Scoping 独立化 + 企查查纠名 + 范围锁定信号 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 设计来源：`docs/design/2026-06-13-engagement-scoping-fanout-redesign.md` §6.2/§6.4/§12 Phase A。
> 验证节奏（用户 2026-06-13 指定）：**整期实现 → 末尾统一门禁 → 按报错修**（不走 TDD 红绿循环）。

**目标：** scoping 全流程（粘名单 → 企查查纠名 → 建母 org → 议子公司 → org 树落库）在 chat 里可被 AI 工具链完成；范围锁定信号（org 树 + 覆盖状态快照）可被前端读取。
**架构：** ① lookup 纠名核心从 GUI 命令抽成自由函数，包成新 agent 工具 `recon_lookup_company`；② `manage_organizations` 加 `create_batch`（批量 get-or-create 根 org）；③ stash@{0} 的 fleet 纯逻辑（scheduler/weakness/contract/query）搬回为 `golish/src/engagement/`，weakness 裸 SQL 下沉 `golish-db::repo::engagement_truth`（SHARED）；④ `engagement_get_snapshot` Tauri 命令 + 4 个 ts-rs 类型 + 前端 API。
**技术栈：** Rust（golish-db / golish-recon-app / golish-pentest-app / golish-agent-runtime / golish bin）+ ts-rs + React/TS 前端 API 层。

---

## 实施期勘验结论（写计划前实读）

1. **纠名底座已存在**：`golish-recon-app/src/asset_intel/commands.rs::asset_intel_lookup_company`（瘦壳化对象）+ `runtime/lookup.rs::run_lookup_cli_provider`（保持不动）+ `types.rs::LookupCompanyMatch{name, credit_code, legal_representative, address, registered_at, confidence}`。ENScan `enscan-go.json` 已配 `lookup.enabled=true`（skill `company-lookup-json`，normalize `$..enterprise_info[*]`）。
2. **stash@{0}^3** 持有 fleet 全套源码；本计划只搬纯逻辑 4 文件（scheduler/weakness/contract/query），headless 编排（fleet/mod.rs 244 行、boot.rs、main.rs/args.rs CLI 入口）**不搬**（已被新设计取代）。
3. **守卫约束**（`scripts/check_repo_ownership.py`）：golish/src 下新文件裸 `sqlx::query*` 触发 RAW_SQL ratchet；`golish_db::repo::organizations`（owner=recon）被无 DOMAIN_RULES 的 golish/src 路径引用触发 ownership 违规。→ weakness 计数 SQL 下沉 golish-db 新 SHARED repo `engagement_truth`（同 coverage_truth 性质：跨服务只读真值投影）；query.rs 的 `organizations::list` 调用改走 engagement_truth 提供的 org 列表读（同文件一并下沉），engagement/ 不直接 import 任何 owned repo。
4. **依赖**：golish Cargo.toml 已有 futures / async-trait / ts-rs（workspace）。
5. **工具暴露链**：BridgeToolSelection（policy.rs）→ enabled_tool_names → selection_apply allow-list → chat `all_enabled()` 自动得新工具；task.rs 显式开。prompt_render.rs BRIDGE_ROWS 渲染工具表进 system prompt。
6. **find_root_id_by_name / create** 已在主干 `golish_db::repo::organizations`（create_batch 直接用）。

## 文件清单

| # | 文件 | 动作 | 职责 |
|---|---|---|---|
| 1 | `backend/crates/golish-db/src/repo/engagement_truth.rs` | 新建 | WeaknessCounts + fetch_weakness_counts（5 个 org 隔离 COUNT）+ list_project_orgs 透传 |
| 2 | `backend/crates/golish-db/src/repo/mod.rs` | 改 | `pub mod engagement_truth;` |
| 3 | `scripts/check_repo_ownership.py` | 改 | SHARED_REPOS + `"engagement_truth"` |
| 4 | `backend/crates/golish/src/engagement/{mod,scheduler,weakness,contract,query}.rs` | 新建（搬回） | 调度内核（Phase B 备）/ 评分+续跑 oracle / ts-rs 契约 / snapshot 查询命令 |
| 5 | `backend/crates/golish/src/lib.rs` | 改 | `pub mod engagement;` |
| 6 | `backend/crates/golish/src/commands_facade/{engagement.rs,mod.rs}` + `commands_registry.rs` | 新建/改 | facade + 注册 `engagement_get_snapshot` |
| 7 | `backend/crates/golish-recon-app/src/asset_intel/{mod.rs,commands.rs,service/lookup_core.rs}` | 改/新建 | lookup 核心抽 `lookup_company_matches`（free async fn，输入 `&golish_pentest::PentestConfigSnapshot 等价物 + keyword + provider_ids + limit`） |
| 8 | `backend/crates/golish-recon-app/src/agent_tools/mod.rs` | 改 | + `ReconLookupCompanyTool`（name=`recon_lookup_company`） |
| 9 | `backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs` | 改 | + `create_batch` action（names[] → get-or-create 根 org → created/existing/failed 三态） |
| 10 | `backend/crates/golish/src/pentest_tool_factory.rs` | 改 | push ReconLookupCompanyTool |
| 11 | `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs` | 改 | BridgeToolSelection + `recon_lookup_company` 字段（all_enabled/none/enabled_tool_names + 顺序测试） |
| 12 | `backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs` | 改 | BRIDGE_ROWS + 一行 |
| 13 | `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs` | 改 | task primary 开 `recon_lookup_company: true` |
| 14 | `resources/harness/stages/scoping.json` | 改 | coverage_complete on_fail hints 首条加纠名引导 |
| 15 | `frontend/lib/generated/{EngagementSnapshot,OrgRunStatusDto,OrgTreeNode,OrgWeaknessScore}.ts` | 新建（搬回） | ts-rs 产物（与 contract.rs 同步） |
| 16 | `frontend/lib/api/engagement.ts` + `frontend/lib/api/index.ts` | 新建/改 | getEngagementSnapshot + flattenTree/sortForest 纯函数 |
| 17 | `feature_list.json` + `agent-progress.md` | 改 | 收口 |

## 任务

### Task 1 · golish-db `repo/engagement_truth.rs`
- 内容：`WeaknessCounts{cve_hits,login_surfaces,open_ports,certs,subdomains}`；`fetch_weakness_counts(pool, org_id)`（subdomains=target_assets JOIN targets scope='in'；open_ports=SUM(jsonb_array_length(t.ports))；login=api_endpoints+directory_entries url ILIKE login/admin/manage；certs=organizations.certificates 数组长度，NULL→0）；`list_orgs(pool, project_path) -> Vec<Organization>`（透传 organizations::list，让 engagement/ 不直接碰 owned repo）。SQL 文本与 stash weakness.rs 等价（build_*_sql() 纯函数 + 单测断言 SQL 文本，沿用 coverage_truth 风格）。
- mod.rs 登记 + 守卫 SHARED_REPOS 加条目（注释说明：engagement 总览的跨服务只读真值投影，同 coverage_truth）。

### Task 2 · `golish/src/engagement/` 搬回
- `scheduler.rs`：stash 原样（doc 注释 fleet→engagement 措辞微调；`crate::fleet::`→`crate::engagement::`）。
- `weakness.rs`：删本地 SQL（scalar_count/fetch_weakness_counts 移除），改 `pub use golish_db::repo::engagement_truth::{fetch_weakness_counts, WeaknessCounts};`；保留 WeaknessWeights/weakness_score/org_stage_has_truth/DbWeaknessScorer + 全部纯函数测试。
- `contract.rs` / `query.rs`：stash 原样 + 路径替换；query.rs 的 `organizations::list`→`engagement_truth::list_orgs`。
- `mod.rs` 新写：模块声明 + 文档（指向新设计文档；说明 scheduler 供 Phase B 前端池注入用）。
- lib.rs `pub mod engagement;`。

### Task 3 · facade + registry
- `commands_facade/engagement.rs`：`pub use crate::engagement::query::engagement_get_snapshot;`（doc 措辞改 chat-native）。
- facade mod.rs 按字母序插 `pub mod engagement;`；registry `use commands_facade::engagement::*;` + handler 列表加 `engagement_get_snapshot`（放 organization_recon 段后）。

### Task 4 · lookup 核心抽取（recon-app）
- 新 `asset_intel/service/lookup_core.rs`（或挂 runtime 同级）：`pub(crate) async fn lookup_company_matches(pentest_config: &golish_pentest::PentestConfig, keyword: &str, provider_ids: &[String], limit: Option<usize>) -> Result<AssetIntelLookupResult, GolishError>`——把 commands.rs 第 43-130 行主体平移（scan → select providers → loop run_lookup_cli_provider → dedupe/sort/truncate），行为零变更。
- `asset_intel_lookup_company` 命令瘦身为：pool_ready 校验 + keyword 非空校验 + `pentest.0.get().await` + 调核心函数。
- mod.rs re-export 给 agent_tools 用。

### Task 5 · `ReconLookupCompanyTool`（agent_tools/mod.rs）
- name=`recon_lookup_company`；参数 schema：`keyword`（必填，原始公司名）、`limit`（可选 int，默认 5）。
- description 写明 scoping 用法：「Scoping STEP 1: resolve a raw company name to its canonical registered name (以企查查为准) BEFORE creating organizations. Returns canonical matches with credit_code + confidence. Pick the best match (usually first) and use its `name` for manage_organizations create/create_batch. No provider configured / no match → record the name as 纠名失败/待人工, never guess.」
- execute：keyword 校验 → `ToolsConfigState.0.get().await` → `lookup_company_matches` → 返回 `{action:"lookup_company", keyword, matches, provider_status}`；error 走 `json!({"error": ...})` 模式（与兄弟工具一致）。无 IDOR 面（不触 org 行）。

### Task 6 · `manage_organizations` `create_batch`
- action enum + `names: string[]`（1..=200，trim+去重保序）。
- 每个名字：`find_root_id_by_name` → 命中记 existing；未命中 `create(pool, project, name, None, "", "")` → created；单条 Err 记 failed{name, error}，**不中断批**（I8 三态）。
- 返回 `{action:"create_batch", created:[{id,name}], existing:[{id,name}], failed:[{name,error}], total}`；审计一条 `organization_batch_created`（计数入 detail）。
- schema 描述提醒：names 应是 recon_lookup_company 纠名后的规范名。

### Task 7 · 注册 + 模式暴露
- pentest_tool_factory.rs：`ReconLookupCompanyTool::new(tools_state.clone())`（无 pool）push 进 bridge tools（注释补 Phase A 设计引用）。
- policy.rs：字段 + all_enabled/none/enabled_tool_names（插在 recon_list_providers 后保持顺序稳定）+ `bridge_all_enabled_lists_tools_in_stable_order` 测试更新。
- prompt_render.rs BRIDGE_ROWS 插行：purpose=“Resolve a raw company name to canonical registered names via enterprise-intel lookup (scoping step 1, 以企查查为准).”
- task.rs：`recon_lookup_company: true` + 既有断言测试补一行。
- prompt_render_tests.rs 两处工具名清单断言补 `recon_lookup_company`。

### Task 8 · scoping.json hints
- coverage_complete.on_fail.hints 头部插：「normalize each input company name FIRST via recon_lookup_company (canonical 企查查 name), create the root org with the canonical name, THEN run recon_discover_subsidiaries」。gate 语义零改动。

### Task 9 · 前端
- 4 个 generated 文件从 stash 落盘（之后 `just check-types` 复核无漂移）。
- `lib/api/engagement.ts`：stash 版去 fleet 措辞（注释改 chat-native scoping 总览读模型）；`lib/api/index.ts` 加 `export * from "./engagement";`（按既有导出风格）。

### Task 10 · 统一验证（末尾一次性，修到全绿）
```bash
cd backend && cargo check -p golish-db -p golish-recon-app -p golish-pentest-app -p golish-agent-runtime -p golish
cargo nextest run -p golish-db -p golish-recon-app -p golish-pentest-app -p golish-agent-runtime -p golish --status-level fail
cargo clippy -p golish-db -p golish-recon-app -p golish-pentest-app -p golish-agent-runtime -p golish --all-targets -- -D warnings
cargo fmt -p golish-db -p golish-recon-app -p golish-pentest-app -p golish-agent-runtime -p golish -- --check
just check-fe && just test-fe
python3 scripts/check_repo_ownership.py && python3 scripts/check_dag.py
python3 -m json.tool resources/harness/stages/scoping.json
```
预期全 exit 0；nextest 含新增单测（engagement_truth SQL 文本、scheduler 搬回 9 测、create_batch schema/三态、lookup 工具 schema、policy 顺序）。

### Task 11 · 收口
- feature_list.json 加 `engagement-phase-a-scoping`（in_progress→视证据 passing；evidence 填 Task 10 输出）。
- agent-progress.md 新会话记录（目标/完成/验证/证据/风险/下一步=Phase B）。

## 风险与对策
- **ts-rs 重导出漂移**：generated 文件手搬后跑 `just check-types`（若有该命令）或 `cargo test -p golish export_bindings` 复核。
- **scheduler 暂无运行时消费方**：golish 是 lib+bin，`pub mod engagement` 公开 API 不触发 dead_code；query.rs 经 From<OrgRunStatus> 用到 contract，weakness 被 query 用，scheduler 被 weakness(DbWeaknessScorer: WeaknessScorer) + contract(From) 引用 → 全部可达。
- **活体验证（Phase A DoD 之活体项）**：需 ENScan AQC cookie；本期代码门禁先行，活体跑留给用户环境（progress 记欠账）。
