# 枚举阶段批次化 + 每资产终态收口 实现计划

> Raw-IP / DNS-only `not_applicable` 注入已被
> `docs/design/2026-07-10-enumeration-origin-terminal-closeout.md` 取代；当前实现只对
> 可物化的 exact Web Origin 建立四轴分母，其余 host 直接排除。

> 面向 AI 工作者：按 `.cursor/skills/executing-plans` 逐任务实现。设计源：
> `docs/design/2026-07-03-enumeration-batch-and-terminal-coverage.md`。
> 用户指示：**一口气实现全部，中途不跑 precommit / 大测试，最后统一修 build**。

**目标**：修掉 enumeration 反复 block 的根治缺口（JS 轴子代理不落 checked_empty），并把三个单 target 内容采集工具改成**批次多 target**（分步骤，非大一统工具），katana 作补充语料合并去重。

**技术栈**：Rust（`golish-pentest-app` / `golish-agent-runtime` / `golish-db`）+ resource JSON/MD + prompt。

**关键既有事实（2026-07-03 核对）**：
- `browser_collect_js_api.rs`：`execute()` 单 target；结尾 `record_jsapi_outcome`（JSAPI），**无 JS outcome**；`upsert_jsapi_outcome` 已在文件内。
- `js_extract_apis.rs`：`execute()` 单 target；`upsert_param_outcome`（PARAM）+ `upsert_jsapi_outcome`（JSAPI）。
- `route_probe_paths.rs`：`execute()` 单 target；`RouteProbePathsTool::upsert_dir_outcome`（DIR）。
- `evidence.rs`：`append_bridge_evidence(pool, BridgeEvidenceInput)`。
- runtime `direct/mod.rs`：`record_enumeration_bridge_evidence` + `enumeration_evidence_projections`（**只投影 browser→GOLISH-ENUM-JS**）在 `execute_tool_direct_generic` 里调用（约 1045 行）。
- `coverage_truth.rs`：`TECH_ENUM_JS/DIR/PARAM/JSAPI`（52-57）。
- enumerator prompt：`execution_planning.rs::build_enumerator_prompt`（约 182 行）。

**DRY / YAGNI / TDD**：每任务先加/改测试，再实现。批次循环复用单 target 逻辑，不复制落库代码。

---

## PR-0 · P0 根治：JS 终态工具自负

### Task 0.1 · browser_collect_js_api 新增 record_js_outcome（TDD）
**文件**：`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`
**步骤**：
1. 加纯函数 `js_outcome_from_browser(persisted_js_rows, status, js_persist_errors) -> &'static str`：
   - `persisted_js_rows > 0` → `"found"`；`status` 含 timeout / == error / js_persist_errors>0 → `"error"`；else `"empty"`。
2. 加 `record_js_outcome(...)` + `upsert_js_outcome(...)`（镜像 `record_jsapi_outcome`/`upsert_jsapi_outcome`，`technique=TECH_ENUM_JS`, `kind="js_collection"`, `source="browser_collect_js_api"`）。
3. `execute`（后续变 execute_single）结尾在 `record_jsapi_outcome` 后调用 `record_js_outcome`，把 `js_outcome` / `js_outcome_persisted` 塞进结果。
4. 测试：`js_outcome_from_browser` 三态；schema 测不受影响。

### Task 0.2 · runtime 删除 JS 投影 hook
**文件**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
**步骤**：
1. 删除 `record_enumeration_bridge_evidence` 调用（约 1045）与函数本体、`enumeration_evidence_projections`、`EnumerationEvidenceProjection`、`resolve_enumeration_target_asset`、`enumeration_subject`、`browser_js_outcome`、`compact_enumeration_raw_output`、`host_asset_from_subject`（如仅此处用）及相关测试。
2. 保留 `record_recon_passive_evidence` 及所有 pentest_run/run_pty_cmd 证据逻辑不动。
3. 更新 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`：说明 enumeration 四轴 evidence 现全部由 `golish-pentest-app` bridge 工具自负。
4. 验证：`cargo check -p golish-agent-runtime`。

---

## PR-1 · P1 批次多 target（分步骤）

### Task 1.1 · browser_collect_js_api 批次
**文件**：同 0.1。
**步骤**：
1. 把现 `execute` body 抽成 `async fn execute_single(&self, args, workspace) -> Result<Value>`（单 target 语义完全不变）。
2. `execute` 头部：若 `args.target_urls` 是非空数组 → 逐个构造 per-target args（继承公共参数），循环 `execute_single`，聚合 `{ "batch": true, "count": N, "results": [...], "js_found": .., "jsapi_found": .., "errors": [...] }`；批次上限 `MAX_BATCH_TARGETS=50`（超限截断标注）。per-target 失败进 errors 不中断。
3. schema `parameters()` 增 `target_urls`（数组），`target_url` 与 `target_urls` 二选一（描述写清「批次优先」）。
4. 测试：批次参数解析（数组→N 次；单值→1 次回退）。

### Task 1.2 · js_extract_apis 批次
**文件**：`js_extract_apis.rs`
**步骤**：同 1.1（`target_urls`），抽 `execute_single`，聚合 PARAM/JSAPI 计数。schema 增 `target_urls`。测试批次解析。

2026-07-11 可观测性补项：batch 只在 `results[].result` 放
`bounded_batch_summary_v1`，每 target ≤8 KiB；白名单保留 status/completion、
JSAPI/PARAM outcome/persisted flags、全部计数、partial/retry 诊断、最多 3 个 endpoint
sample 与小候选 sample、capture manifest/DB 引用。single-target 完整结果不变。
`response_parsing.rs` 为 batch 生成逐 root `root_diagnostics`，endpoint 数必须读
`endpoints_total`，不能因顶层无 `endpoints` 误报 0。先加红测，再验证单 root 上限、
50-root 聚合 ≤512 KiB、model-visible counts/outcomes。

### Task 1.3 · route_probe_paths 批次
**文件**：`route_probe_paths.rs`
**步骤**：
1. 抽 `execute_single`。
2. `execute` 头部：若 `args.targets`（`[{target_id, base_url}]`）非空 → 逐个 execute_single，聚合 DIR 计数。批次上限。
3. schema 增 `targets` 数组；`target_id`+`base_url` 与 `targets` 二选一。
4. 测试批次解析。

### Task 1.4 · repair 模式批次 target 校验
**文件**：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`（`coverage_gap_direct_tool_target_block_reason` 在 `executor_types.rs`）
**步骤**：
1. `coverage_gap_direct_tool_target_block_reason` 扩展：识别批次入参（`target_urls` / `targets[].base_url`），对**每个** target 逐一比对 `coverage_gap_actions`；任一不在清单 → block（返回精确原因）。
2. 测试：批次里混入未点名 target 被 block；全部点名放行。

---

## PR-2 · P2 katana 补充 + prompt/methodology

### Task 2.1 · methodology 批次顺序 + katana 合并
**文件**：`resources/harness/stages/enumeration/methodology.md`
**步骤**：改「Recommended sequence」为批次口径：① 批次 `browser_collect_js_api(target_urls=[...])` 对整份 live web root 集一次跑；② 一次 `pentest_run(katana -list)` 补充 URL 语料（合并进 api_endpoints，去重自动）；③ 批次 `js_extract_apis(target_urls=[...])`；④ 批次 `route_probe_paths(targets=[...])`；⑤ worklist/coverage 预检后 submit。强调 katana 补充、不替代 browser 闭包。

### Task 2.2 · enumerator prompt 批次口径
**文件**：`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`（`build_enumerator_prompt`）
**步骤**：更新工具用法段：优先批次数组入参一次覆盖整资产集；katana 经 `pentest_run(tool_name="katana", args="-list ...")` 作补充；保留「worklist 是权威计划」「不 hand-write found」等既有约束。

### Task 2.3 · spec 注释同步（如需）
**文件**：`resources/harness/stages/enumeration/spec.json`
**步骤**：更新 `$comment_*` 说明批次流程；`gate_rules` / `expected_techniques` 不变。

---

## PR-3 · P3 enumeration not_applicable（可选、最低优先）

### Task 3.1 · DB 判定 + gate 注入
**文件**：`coverage_truth.rs`、`org_gate.rs`、`harness_submit_tool.rs`、`stage_coverage.rs`
**步骤**：新增 `enum_content_not_applicable_assets`（web-capable 但仅 DNS/无 web 面 IP）；经 `not_applicable_coverage` 注入 enumeration gate + UI。仅在 P0/P1 验证仍有噪声时做。

---

## 最终收口
1. `cargo check`（workspace 或逐 crate）修编译。
2. `cargo fmt -p <改动 crate>`。
3. 相关单测 `cargo nextest run -p golish-pentest-app` / `-p golish-agent-runtime`。
4. 更新 `docs/modules/**` 受影响模块卡 + `agent-progress.md` + `feature_list.json`。
5. 用户要求：中途不跑 precommit / 大测试，build 问题最后统一修。
