# Enumeration Agent Team 与 JS/API/参数证据解析 v2 实现计划

> **For AI workers:** 必须遵守 `AGENTS.md`，使用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans` 逐 Task 执行；每个 Task 都先写 focused RED，再做最小 GREEN。共享 working tree 很脏，只能修改本计划列出的精确文件/区块，不得回滚、格式化或提交其他会话的改动。

**Goal:** 把 Enumeration 中一次性 JS/API 黑盒工具升级为 server-owned、有限并发、证据驱动的 Agent Team，并让 endpoint 的每个浏览器/JS/AI 发现、URL 解析链和参数来源都可持久化、可 Gate、可在统一 Stage Workspace 与 Target Surface 中追踪。

**Architecture:** 顶层继续一次 `stage_run(enumeration)`；每家公司一个 Controller，服务端从 exact-origin 四轴 worklist 生成 typed producer shards，按 Content/Browser/JSAPI/Parameter/Resolution/Review 波次滚动执行。确定性采集与 AST/data-flow 是主路径，AI 只处理 bounded unresolved cluster。`api_endpoints` 保持 canonical identity，新增 operation-scoped immutable terminal occurrences、occurrence parameters/evidence links；只有 runtime/static-confirmed occurrence 可投影 canonical manifest，AI/ambiguous/unresolved/scope-excluded 只保留证据与终态。

**Tech Stack:** Rust 2021、Tauri 2、sqlx/PostgreSQL、serde/ts-rs、ast-grep、Node.js `node:test`、Playwright、React 19、TypeScript 6、Vitest、Biome、cargo-nextest。

**Design:** `docs/design/2026-08-02-enumeration-agent-team-js-api-analysis-v2.md`

---

## 0. 执行硬门与依赖

在任何产品代码修改前必须同时满足：

1. `unified-visible-stage-workspace-mock-2026-08-02` 已结束并退出 `in_progress`；本功能才能成为唯一 `in_progress`。
2. 依赖功能 `tool-truth-coverage-contract-2026-07-29` 必须先达到 `passing`，且其 migration/repo 已提供 execution authority、sealed denominators、receipt inputs、normalized evidence/business-ref authorities。本功能不得在它仍 `blocked` 时复制一套弱化 authority。
3. 用户重新明确批准：
   - 新增唯一 additive migration `20260802000003_enumeration_endpoint_provenance_v2.sql`；
   - 修改 `operation_state` additive frozen contract、`golish-db` repo/trigger/stage-reset guard；
   - 新增 additive ts-rs IPC 类型并运行 `just gen-types`。
4. production contract cutover 不包含在 schema 批准中。先完成 `agent_team_v2_shadow` 报告，用户再次批准后才能把新 production operation 默认值改为 `agent_team_v2`。
5. 真实 target/browser/provider 验收是第三个独立批准点；focused fixture 可以用本地 HTTP server，不能访问互联网或 Test1 live DB。
6. 确认依赖、migration 与 feature slot：

```bash
test ! -e backend/crates/golish-db/migrations/20260802000003_enumeration_endpoint_provenance_v2.sql
jq -e '.features[] | select(.id == "tool-truth-coverage-contract-2026-07-29") | .status == "passing"' feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) == 0' feature_list.json
git status --short
```

7. 用 `apply_patch` 把本 feature 精确改为唯一 `in_progress`，并在 `agent-progress.md` 顶部新建开工记录，写明 scope、授权、依赖版本和定向验证方案；随后验证：

```bash
jq -e '([.features[] | select(.status == "in_progress" and .id == "enumeration-agent-team-js-api-analysis-v2-2026-08-02")] | length) == 1 and ([.features[] | select(.status == "in_progress")] | length) == 1' feature_list.json
```

预期：Tool Truth 依赖 passing、00003 不存在；切换后本功能是唯一 `in_progress`；dirty tree 被完整记录但不清理。

若任一批准缺失，保持 feature=`not_started`，只允许继续审阅计划，不开始 Task 1。

---

## 文件结构

### 新建

- `backend/crates/golish-db/migrations/20260802000003_enumeration_endpoint_provenance_v2.sql`
- `backend/crates/golish-db/src/repo/enumeration_endpoint_occurrences.rs`
- `backend/crates/golish-db/tests/enumeration_endpoint_provenance_v2.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_analysis_contract.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_module_graph.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/js_resolution_clusters.rs`
- `frontend/components/Engagement/EnumerationArtifactView.tsx`
- `frontend/components/Engagement/EnumerationArtifactView.test.tsx`
- `frontend/lib/api/stage-team.test.ts`
- `frontend/components/TargetPanel/surface/EndpointProvenancePanel.tsx`
- `frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx`
- `frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.tsx`
- `frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx`
- `docs/validation/2026-08-02-enumeration-agent-team-v2-shadow.md`

### 重点修改

- 浏览器：`scripts/browser_collect_js_api.mjs`、`scripts/browser_collect_js_api.test.mjs`
- Analyzer：`backend/crates/golish-js-analyzer/src/{lib.rs,patterns.rs,ast_filter.rs,lib_tests.rs}`
- Producers：`backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,browser_collect_js_api.rs,js_extract_apis.rs,evidence.rs}`
- Port/repo：`backend/crates/golish-app-core/src/ports/recon/scans.rs`、`backend/crates/golish-db/src/repo/{mod.rs,capability_execution_receipts.rs,enumeration_surface_manifest.rs,operation_state.rs,stage_purge.rs}`
- Scheduler/runtime：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_team_scheduler.rs,stage_run_call.rs,sub_agent_call.rs}`
- Roles/tools：`backend/crates/golish-sub-agents/src/defaults/{builder/mod.rs,builder/registry.rs,prompts/execution_planning.rs,tests.rs}`、`backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- Gate/spec：`resources/harness/stages/enumeration/{spec.json,methodology.md}`、`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,resources.rs,org_gate.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`
- Typed API：`backend/crates/golish-agent-app/src/ai/commands/stage_team.rs`、`backend/crates/golish-pentest-app/src/{security_analysis.rs,target_surface_hierarchy.rs}`、`backend/crates/golish/src/commands_facade/{ai.rs,workspace.rs}`、`backend/crates/golish/src/commands_registry.rs`、`frontend/lib/api/{stage-team.ts,stage-team.test.ts,security-analysis.ts,security-analysis.test.ts}`、`frontend/lib/generated/`
- UI：`frontend/components/Engagement/{StageTeamRunView.tsx,StageTeamRunView.test.tsx,StageTeamWorkspaceView.tsx,StageTeamWorkspaceView.test.tsx}`、Task 11 的 Enumeration artifact view、Target Surface hook/model/tab files列于 Task 12。
- System of record：相关模块卡、`docs/modules/INDEX.md`、两个文档索引、`feature_list.json`、`agent-progress.md`。

---

## Task 1：冻结 v2 分析合同与纯解析 reducer

**Files:**

- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_analysis_contract.rs`
- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`

### 1.1 RED

先在新模块写纯单元测试，测试名固定为：

- `resolution_v2_axios_client_base_combines_segments`
- `resolution_v2_root_fetch_uses_origin_root`
- `resolution_v2_relative_fetch_uses_document_base`
- `resolution_v2_conflicting_bases_remain_ambiguous`
- `resolution_v2_runtime_observation_outranks_static_without_erasing_sources`
- `resolution_v2_ai_without_anchor_is_rejected`
- `parameter_fact_never_serializes_value`
- `parameter_requirement_unknown_is_not_optional_or_required`
- `resolution_base_applicability_prevents_router_and_asset_leakage`

测试使用下列闭集：

```rust
enum ObservationKind { RuntimeRequest, HtmlForm, StaticAst, AiAnalysis }
enum InferenceLevel { Observed, Deterministic, AiInferred }
enum ResolutionStatus { Resolved, Ambiguous, Unresolved, NotApplicable }
enum ScopeDecision { InScope, ScopeExcluded }
enum CandidateClassification { Endpoint, Noise }
enum ParameterRequirement { Required, Optional, Unknown }

enum ParameterLocation {
    Path,
    Query,
    Body,
    Form,
    Header,
    GraphqlVariable,
    Unknown,
}
```

先运行：

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -E 'test(resolution_v2_) | test(parameter_fact_never_serializes_value) | test(parameter_requirement_)' --status-level fail
```

预期 RED：新模块/类型/reducer 尚不存在；失败只来自本 Task。

### 1.2 GREEN

实现：

1. `EndpointOccurrenceDraft`、`ResolutionContextV2`、`ResolutionStep`、`ResolutionCandidate`、`ParameterFactV2` 与上述正交 enum；`promotion_eligibility` 只由 reducer 计算。
2. 所有 enum 使用稳定 snake_case serde；未知字符串 fail closed，不用 `#[serde(other)]` 吞掉新状态。
3. `ResolutionContextV2` 同时保存 sanitized raw expression、document/html/app/router/client/bundler base、每条 base 的 `applies_to`、candidate URLs 与 selected URL。
4. reducer 的优先级固定为 runtime > exact client > 与调用类型匹配的 document/framework/bundler fact > cross-file static > anchored AI；root fetch 不受 router/asset base 影响。
5. `ParameterFactV2` 没有 value 字段；serde round-trip 测试扫描 JSON，证明 secret 示例值不出现。
6. `ScopeDecision` 独立于 inference/resolution；高置信 foreign URL 仍必须 `scope_excluded`。
7. `ParameterRequirement` 保留 required/optional/unknown；legacy bool 只有 required=true，unknown 不得被 UI 文案写成 optional。

再次运行同一 focused command，预期 9/9 GREEN。

### 1.3 定向静态检查

```bash
just space-guard && cd backend && cargo clippy -p golish-pentest-app --lib --tests -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-pentest-app/src/pentest_bridge/js_api_analysis_contract.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs
```

**Commit boundary（仅实施会话获用户提交授权后）：**

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/js_api_analysis_contract.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs
git commit -m "feat(enumeration): define JS API analysis v2 contract"
```

---

## Task 2：Browser capture v3、只读阻断与字段脱敏

**Files:**

- Modify: `scripts/browser_collect_js_api.mjs`
- Modify: `scripts/browser_collect_js_api.test.mjs`

### 2.1 RED

新增 `node:test`：

1. `capture v3 preserves duplicate script provenance`：相同 hash 从两个 script URL/页面加载，manifest 保留两个 source context。
2. `capture v3 keeps occurrences with distinct body shapes`：同 method+URL 的两个 JSON shape 不合并。
3. `capture v3 extracts value-free JSON and form fields`：得到 body/form 字段名与类型，序列化结果不含示例 password/token。
4. `capture v3 marks blocked unsafe request unsent`：POST 在网络 dispatch 前 abort，记录 `sent=false`。
5. `capture v3 correlates CDP initiator by request id timestamp and ordinal`：保留 page URL、document base、CDP request ID、script URL/line/column；两个同 URL 并发请求不会串 initiator。
6. `capture v3 reads legacy v2 without inventing provenance`：缺失字段为 unknown/empty，不猜测。
7. `capture v3 records form action and fields without submission`：读取 action/method/input names，测试服务器没有收到 submit。
8. `capture v3 removes query values userinfo fragments and secret sentinels`：扫描整个 v3 JSON，URL/body/header/response 均不存在 sentinel。
9. `capture v3 retry reuses logical key while event ids differ`：相同 server shard/navigation input 的两次 attempt 得到相同 logical key，原 event ID 可不同。
10. `capture v3 records unsupported initiator without guessing`：CDP attach 不可用或无法唯一关联时 initiator 为 null、reason 明确，不能拿 document/script tag 猜行号。

```bash
node --test --test-name-pattern='capture v3' scripts/browser_collect_js_api.test.mjs
```

预期 RED：v3 schema/extractors 尚不存在。

### 2.2 GREEN

实现并导出纯 helper：

- `extractRequestParameterFacts(request)`：JSON、URL encoded、`URLSearchParams`、`FormData`、multipart 字段名；超过深度/字段数时写 truncation receipt。
- `sanitizeCaptureUrl(url)`：移除 userinfo/fragment；query values 全部替换为 `{value}`，query names 另产 parameter facts；page/document/script/request/redirect URL 全部走同一函数。
- `redactCaptureValue(name, value)`：header/body/query/response 敏感 key 在 helper 内替换；普通 occurrence 不保留 cookie/auth/password/API key/Set-Cookie 值。
- `runtimeOccurrenceKey`：由 server-issued stable collection key + sanitized page identity + initiator fingerprint + method/path + value-free schema shape + duplicate ordinal 组成；原 capture event ID 只作 provenance，禁止参与 retry idempotency，也禁止对原 secret value 做 hash。
- `attachCdpNetworkLedger(page)`：Chromium 为每个 page/context 建 CDP session并启用 `Network`；以 `Network.requestWillBeSent.requestId` 为原始键保存 monotonic timestamp 与 `initiator` stack，再按 page/context、method、sanitized URL、timestamp window 与每 fingerprint ordinal 唯一关联 Playwright capture。line/column 只取 CDP payload，绝不由 script tag 或相邻文本推断。
- 非 Chromium、CDP attach 失败或关联不唯一时写 `initiator=null` 与 `initiator_status=unsupported_cdp|unmatched`；CDP event 的出现不等价于请求已发出，`sent` 仍只由 route safety path 决定。
- `writeScriptManifestV3`：保存 bounded `source_urls`、`discovered_from`、document bases、content hash、manifest/chunk/source-map metadata。
- `observeDocumentForms`：只读提取 form action/method 与 value-free field names；不 click、不 submit，危险/跨 scope action 仍只记录候选。
- capture v1/v2 reader 兼容；新 writer 固定 `browser_js_api_capture_v3`。
- JS/source-map 原 body 只留 scoped capture/raw-witness artifact；manifest/DB/普通 IPC 只保存 sanitized URL、content hash、source path 与 bounded redacted span/hash。

写请求的路由必须在 `route.continue()` 前判断并 abort；测试服务器只绑定 `127.0.0.1`，测试结束关闭端口。

### 2.3 验证

```bash
node --test --test-name-pattern='capture v3|dangerous|exact origin' scripts/browser_collect_js_api.test.mjs
node --check scripts/browser_collect_js_api.mjs
```

预期：新增 v3 与既有 read-only safety 测试 GREEN；无外部请求。

**Commit boundary（有授权才执行）：**

```bash
git add scripts/browser_collect_js_api.mjs scripts/browser_collect_js_api.test.mjs
git commit -m "feat(enumeration): add redacted browser capture v3"
```

---

## Task 3：Additive occurrence schema 与 guarded compound repo

> 本 Task 是 schema 高风险批准点。没有用户明确批准时立即停止，不创建 migration、不修改 `golish-db`。

**Files:**

- Create: `backend/crates/golish-db/migrations/20260802000003_enumeration_endpoint_provenance_v2.sql`
- Create: `backend/crates/golish-db/src/repo/enumeration_endpoint_occurrences.rs`
- Create: `backend/crates/golish-db/tests/enumeration_endpoint_provenance_v2.rs`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Modify: `backend/crates/golish-db/src/repo/capability_execution_receipts.rs`
- Modify: `backend/crates/golish-db/src/repo/enumeration_surface_manifest.rs`
- Modify: `backend/crates/golish-db/src/repo/operation_state.rs`
- Modify: `backend/crates/golish-db/src/repo/stage_purge.rs`
- Modify: `backend/crates/golish-app-core/src/ports/recon/scans.rs`

### 3.1 RED migration/repo tests

新 integration test 先要求以下行为：

- `same_endpoint_keeps_browser_and_two_js_occurrences`
- `canonicalizer_links_runtime_and_static_only_on_unique_template_match`
- `canonicalizer_keeps_ambiguous_template_matches_separate`
- `same_request_keeps_distinct_body_shape_parameters`
- `sealed_script_checked_empty_is_distinct_from_missing_receipt_input`
- `sealed_candidate_without_terminal_occurrence_blocks_closure`
- `parameter_checked_empty_is_distinct_from_missing_receipt_input`
- `unresolved_occurrence_does_not_create_canonical_endpoint`
- `scope_excluded_occurrence_cannot_target_foreign_origin`
- `cross_origin_source_a_resolved_b_keeps_both_authorities`
- `occurrence_rejects_wrong_operation_org_origin_and_worker`
- `occurrence_evidence_requires_normalized_tool_truth_authority`
- `occurrence_evidence_rejects_cross_execution_authority`
- `retry_reuses_denominator_input_key_while_capture_event_ids_differ`
- `derived_occurrence_rejects_cross_scope_parent`
- `shadow_occurrence_never_mutates_canonical_or_manifest`
- `legacy_contract_rejects_v2_writer`
- `production_v2_requires_receipt_v1_tool_truth_contract`
- `operation_insert_freezes_server_rollout_contract`
- `stage_reset_preserves_contract_occurrences_and_authorities`
- `occurrence_and_assessment_updates_are_rejected`
- `js_analysis_item_allows_one_terminal_cas_and_rejects_update_delete`
- `graphql_operations_form_distinct_groups`
- `websocket_group_never_projects_http_api_endpoint`
- `legacy_manifest_parameter_locations_remain_readable`

```bash
just space-guard && cd backend && cargo nextest run -p golish-db --test enumeration_endpoint_provenance_v2 --status-level fail
```

预期 RED：migration/table/repo API 不存在。

### 3.2 DDL

唯一 migration 创建/扩展：

1. `enumeration_analysis_rollout` + 受控 promotion receipt/function；默认 `legacy_v1`。`operation_state.enumeration_analysis_contract` 在 INSERT trigger 中从 rollout 写入并永久 immutable，历史行 backfill `legacy_v1`；stage reset 不删除/重冻。
2. 扩展 Tool Truth business-ref closed kind/validator，加入 `enumeration_endpoint_occurrence` 与 `enumeration_endpoint_group`；不复制 evidence/denominator authority。
3. `enumeration_js_analysis_items`：domain descriptor，组合 FK 到 sealed denominator item、execution authority 和 terminal receipt input；保存 sanitized manifest/page/document/chunk/map metadata，不维护第二套 status。
4. `enumeration_endpoint_candidate_inputs`：组合 FK 到 sealed candidate denominator item、execution authority；保存 server logical input key、sanitized source/callsite/event fingerprint、duplicate ordinal 与 resolution input。原 capture event ID 不是 idempotency key。
5. `enumeration_endpoint_occurrences`：组合 FK 到 candidate + terminal receipt input + execution authority；显式 source/resolved target+origin、parent lineage、sanitized source/initiator、protocol/method/GraphQL operation、正交 outcome、canonical request URL、route kind/template、sent/schema/redaction metadata。所有 terminal row UPDATE/DELETE 均由 trigger 拒绝。
6. `enumeration_endpoint_parameter_assessments`：组合 FK 到 parameter denominator/receipt input/occurrence；terminal outcome + reason，UPDATE/DELETE 拒绝。
7. `enumeration_endpoint_occurrence_parameters`：assessment、name/location/type、`required/optional/unknown`、confidence/source anchor；无 value。扩展 legacy location check，保留 `body_or_form` 并增加 `body/form/graphql_variable`。
8. `enumeration_endpoint_groups`：operation-scoped `(resolved origin, protocol, method, route kind/template, GraphQL operation)` identity；runtime samples不决定 key。
9. `enumeration_endpoint_occurrence_group_links` 与 `enumeration_endpoint_group_api_links`：前者保留 v2 lineage，后者只为兼容 HTTP(S) group 投影 `api_endpoints`/manifest；WebSocket、无 runtime sample template、Shadow、scope/noise/unresolved 不进入 legacy link。
10. `enumeration_endpoint_occurrence_evidence`：组合 FK 到 `(tool_truth_evidence_authority_id, execution_authority_id, authority_hash)`，不接收裸 audit ID；role=`discovery/resolution/parameter`。
11. 所有 composite FK/trigger 复核 frozen operation contract、Tool Truth contract、scope snapshot、organization/project、source/resolved EAS origin、worker fence 与 normalized authority。新表 additive，不修改历史 migration，不回填历史 operation。

### 3.3 Guarded repo

新增：

```rust
pub async fn persist_endpoint_occurrence(
    tx: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    candidate: &CapabilityReceiptInputRef,
    draft: &EndpointOccurrenceWrite,
    evidence_authorities: &[EvidenceAuthorityRef],
) -> Result<PersistedEndpointOccurrence>
```

规则：

- DB trigger 在 operation INSERT 从 server rollout 冻结 enumeration contract；caller 不能选择或改写。`legacy_v1` 拒绝 v2 writer；Shadow 需要 compatible Tool Truth authority；production v2 额外要求 Tool Truth=`receipt_v1`。
- 先按稳定 UUID 顺序获取 source/resolved target guard；A页面发现B API时 discovery authority留在A，只有B已在同 frozen EAS scope才填 resolved IDs并允许group projection。
- 所有 script/candidate/parameter input 先写并 seal generic denominator，再写 receipt input；domain row必须引用它们，Gate可见任何缺失 terminal。
- occurrence insert 永不创建/更新 canonical endpoint；normalized evidence/business refs只接受 Tool Truth authority ID/hash。
- frozen contract=`agent_team_v2_shadow` 时只写 descriptors/occurrence/assessment/参数/authority refs，绝不运行 group/API projector。
- 新增 `project_endpoint_groups`：只在 production v2、origin输入终态、正交 eligibility通过且 route match唯一时建group；兼容 projector只使用无query exact URL或有runtime sample的HTTP(S) template group，模板本身永不成为replay URL。
- canonical parameter 只从 linked promotable occurrence 的 terminal assessments 做 set union，provenance 留在 child table；legacy required bool 仅三态中的 required=true。
- v2 manifest 的单值 `source` 固定为 `occurrence_v2_aggregate`，不得再被 browser/static producer 的 last-writer 覆盖；
- DB transaction 中不做 HTTP、文件读取、模型调用或 evidence 文本生成；
- `stage_purge`/reset 不删除 contract、sealed denominator、normalized authority、occurrence 或 assessment；新 generation/attempt 用新 authority 前向记录，测试锁定 immutable truth。

`ReconScansPort` 增加同形状的 guarded compound method；in-proc adapter 不把 `PgPool`/transaction 细节泄漏给上层。

### 3.4 GREEN/Clippy

```bash
just space-guard && cd backend && cargo nextest run -p golish-db --test enumeration_endpoint_provenance_v2 --status-level fail
just space-guard && cd backend && cargo nextest run -p golish-db --test enumeration_surface_manifest -E 'test(enumeration_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-db -p golish-app-core --lib --tests -- -D warnings
```

另提供 `persist_js_analysis_descriptor`、`persist_candidate_descriptor`、`persist_parameter_assessment` 与 `project_endpoint_groups`；复用 Tool Truth CAS/seal API和 immutable links，确保 retry 不把 missing/pending 伪装成 checked-empty。

预期：上述 authority、denominator、immutability、cross-origin、group 与 legacy compatibility tests 全 GREEN；两个 crate 零 warning。

**Commit boundary（有授权才执行）：**

```bash
git add backend/crates/golish-db/migrations/20260802000003_enumeration_endpoint_provenance_v2.sql backend/crates/golish-db/src/repo/enumeration_endpoint_occurrences.rs backend/crates/golish-db/tests/enumeration_endpoint_provenance_v2.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/capability_execution_receipts.rs backend/crates/golish-db/src/repo/enumeration_surface_manifest.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/stage_purge.rs backend/crates/golish-app-core/src/ports/recon/scans.rs
git commit -m "feat(enumeration): persist endpoint occurrence provenance"
```

---

## Task 4：Analyzer 从 regex-first 提升为 callsite 参数与模块事实

**Files:**

- Modify: `backend/crates/golish-js-analyzer/src/lib.rs`
- Modify: `backend/crates/golish-js-analyzer/src/patterns.rs`
- Modify: `backend/crates/golish-js-analyzer/src/ast_filter.rs`
- Modify: `backend/crates/golish-js-analyzer/src/lib_tests.rs`

### 4.1 RED

新增固定测试：

- `candidate_params_are_bound_to_exact_minified_callsite`
- `fetch_config_extracts_query_body_and_header_names_without_values`
- `axios_config_extracts_body_and_params_by_location`
- `request_and_jquery_extract_form_fields`
- `graphql_extracts_operation_and_variable_names`
- `dynamic_values_keep_field_names_and_unknown_types`
- `two_equal_paths_keep_distinct_candidate_ids_and_spans`
- `legacy_endpoint_projection_remains_serde_compatible`

```bash
just space-guard && cd backend && cargo nextest run -p golish-js-analyzer --lib -E 'test(candidate_params_) | test(fetch_config_) | test(axios_config_) | test(request_and_jquery_) | test(graphql_) | test(dynamic_values_) | test(two_equal_paths_) | test(legacy_endpoint_)' --status-level fail
```

预期 RED：`ParameterFact`/argument AST facts 不存在。

### 4.2 GREEN

1. Additive 扩展 `EndpointCandidate`：candidate ID、source span、callee/receiver、argument/config facts、`ParameterFact`。
2. 只在 AST-confirmed call node 内提取参数；删除/绕开邻近 400-byte 文本把参数贴到相邻调用的路径。
3. fetch/Request、axios、XHR、jQuery、GraphQL、WebSocket、EventSource 使用闭集 adapter；不认识的 wrapper 只产 raw candidate。
4. query/body/form/header/GraphQL variables 分位置；动态值只保留字段名与 unknown type。
5. 旧 `extract_from_source/files` 继续从 candidate 流投影，旧数量、顺序和 JSON 结构不变。

### 4.3 验证

```bash
just space-guard && cd backend && cargo nextest run -p golish-js-analyzer --lib --status-level fail
just space-guard && cd backend && cargo clippy -p golish-js-analyzer --lib --tests -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-js-analyzer/src/lib.rs backend/crates/golish-js-analyzer/src/patterns.rs backend/crates/golish-js-analyzer/src/ast_filter.rs backend/crates/golish-js-analyzer/src/lib_tests.rs
```

**Commit boundary（有授权才执行）：** `feat(js-analyzer): bind parameter facts to AST callsites`。

---

## Task 5：Rust Browser producer 消费 v3 并写 occurrence

**Files:**

- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/evidence.rs`
- Modify: `backend/crates/golish-app-core/src/ports/recon/scans.rs`

### 5.1 RED

在 `browser_collect_js_api.rs` 相邻测试区新增：

- `capture_v3_preserves_script_source_urls_and_document_bases`
- `capture_v3_repeated_runtime_requests_create_distinct_occurrences`
- `capture_v3_exposes_body_form_header_facts_to_parameter_reducer`
- `capture_v3_blocked_write_is_unsent_and_never_replayed`
- `capture_v3_form_becomes_occurrence_without_submission`
- `capture_v3_redacted_shape_has_no_sensitive_values`
- `capture_v2_compatibility_does_not_invent_initiator`
- `capture_v3_cdp_initiator_correlation_keeps_concurrent_requests_separate`
- `capture_v3_cdp_unavailable_persists_null_with_reason`
- `browser_occurrence_uses_trusted_stage_worker_authority`
- `browser_occurrence_cross_origin_discovery_binds_source_a_and_resolved_b`

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -E 'test(capture_v3_) | test(browser_occurrence_)' --status-level fail
```

### 5.2 GREEN

1. `CollectedScriptRow` 与 manifest loader 保留 URL/hash/discovery/page/document-base 多来源，不再只投影 `script_paths`。
2. `ApiEndpointRecord` 改为 occurrence draft，保存 capture v3 与 CDP request/initiator 读取引用；只有由 request ID/timestamp/ordinal 唯一关联的 CDP initiator 才写 script/line/column，unsupported/unmatched 保持 null + reason。value-free parameter facts 留在受保护的 capture artifact，等待独立 Parameter reducer，Browser producer 不发布 PARAM outcome。
3. 每个 runtime capture 独立写 occurrence；相同 canonical URL 由 repo 去重 endpoint，不丢 occurrence。
4. `AgentToolContext` 中 operation/execution/unit/worker lineage 传入 compound repo；`evidence.rs` 不再把可信 stage identity 丢成 `None`。
5. unsafe request 只有 `sent=false` occurrence，绝不调用 replay/verification 路径。
6. capture body/headers 只传 shape/redaction metadata；既有 raw capture Inspector 的兼容读取与新 provenance DTO 分离。
7. A 页面运行时发现 B origin 时，occurrence 的 discovery evidence/worker authority 仍绑定 source A；只有 B 属于同 frozen EAS scope 才设置 resolved target/origin，并由后续 projector 在 B 上分组。不得把 B 回写成发现来源，也不得访问 scope 外 B。

### 5.3 验证

```bash
node --test --test-name-pattern='capture v3|dangerous' scripts/browser_collect_js_api.test.mjs
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -E 'test(capture_v3_) | test(browser_occurrence_) | test(browser_collect_js_api_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-pentest-app -p golish-app-core --lib --tests -- -D warnings
```

**Commit boundary（有授权才执行）：** `feat(enumeration): persist runtime API occurrences`。

---

## Task 6：跨文件 module/client data-flow、URL resolution v2 与 Parameter assessment reducer

**Files:**

- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_module_graph.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`

### 6.1 RED

新增测试：

- `resolution_v2_imported_axios_client_keeps_export_source_chain`
- `resolution_v2_wrapper_propagates_method_path_and_body_shape`
- `resolution_v2_relative_client_base_uses_module_or_document_context`
- `resolution_v2_new_url_import_meta_is_module_relative`
- `resolution_v2_html_base_and_router_base_do_not_pollute_other_pages`
- `resolution_v2_same_hash_two_execution_bases_remains_contextual`
- `resolution_v2_dynamic_template_is_unresolved_not_joined`
- `resolution_v2_static_template_and_runtime_sample_form_stable_group`
- `resolution_v2_arbitrary_concat_never_groups`
- `resolution_v2_group_identity_is_input_order_independent`
- `resolution_v2_duplicate_endpoint_keeps_all_callsites`
- `resolution_v2_param_hints_do_not_cross_callsites`
- `parameter_reducer_combines_runtime_and_static_facts_without_values`
- `parameter_assessment_checked_empty_is_terminal_not_missing`
- `resolution_v2_foreign_origin_is_scope_excluded_without_visit`
- `resolution_v2_source_map_metadata_is_anchored_or_explicitly_unsupported`
- `resolution_v2_source_map_descriptor_never_persists_sources_content`

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -E 'test(resolution_v2_) | test(parameter_reducer_) | test(parameter_assessment_)' --status-level fail
```

### 6.2 GREEN

1. 用 `ScriptCaptureSource` 取代 `script_paths`，保留 source URL/hash/page/document base/chunk/manifest/source-map metadata。
2. `ModuleGraph` 只解析 captured manifest 内的静态 import/export、常量、client factory、alias 和 wrapper；fixed-point 上限为节点数 + 明确 fuel，循环产 `unresolved` receipt。
3. URL resolver 分开实现：Axios combine、URL standard document resolution、`new URL(..., import.meta.url)`；禁止统一字符串拼接函数。
4. config 覆盖 `axios.create/defaults`、HTML `<base>`、常见 runtime config、Vite base、Next basePath/assetPrefix、Webpack public path；每个 fact 绑定 source/module/page。
5. source map 普通 descriptor 只保存 sanitized map URL/hash、`sources` path、generated span、可验证 original span 与 bounded redacted source window/hash；`sourcesContent`/完整 map body 只能留在受保护 capture/raw-witness，禁止写 DB/manifest/IPC。若无法确定 line mapping，保留 bundle span并写 `unsupported_mapping`，不虚构源码行。
6. Parameter reducer 读取 runtime capture facts 与 candidate-bound AST facts，写 immutable assessment；`found` 写 child parameters，`checked_empty` 写零 child 的显式终态，`unresolved` 写原因。随后才从 promotable occurrence 的 terminal assessments 归并 legacy canonical params；停用按 `(path,method)` 的 free-form hint 主路径。
7. HAE/legacy AI supplemental 只能生成 occurrence candidate，必须经过同一 resolver/scope classifier；production v2 路径不再调用叶子工具内部 `ai_assist=true`。
8. 每个 candidate 调 guarded repo；`ai_inferred/ambiguous/unresolved/scope_excluded` endpoint FK 必须为空。
9. group reducer 区分 `resolved_exact`、AST 证明的 `resolved_route_template` 与 arbitrary dynamic unresolved；static template 本身不是 replay URL。exact runtime sample 只在唯一命中 template 时 link，key 对输入顺序稳定，GraphQL operation/WS protocol 继续遵守 Task 3 的分组边界。

### 6.3 验证

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -E 'test(resolution_v2_) | test(parameter_reducer_) | test(parameter_assessment_) | test(js_extract_apis_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-pentest-app --lib --tests -- -D warnings
```

**Commit boundary（有授权才执行）：** `feat(enumeration): resolve JS API contexts across captured modules`。

---

## Task 7：Bounded unresolved cluster 与真正的 AI Resolution Worker

**Files:**

- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/js_resolution_clusters.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_extract.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_recipe.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/tests.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`

### 7.1 RED

测试固定要求：

- `resolution_worker_reads_only_assigned_cluster_sources`
- `resolution_worker_rejects_unanchored_endpoint`
- `resolution_worker_rejects_source_span_outside_cluster`
- `resolution_worker_cannot_promote_ai_inference_to_canonical`
- `resolution_worker_rejects_ai_noise_terminal`
- `resolution_worker_budget_exhaustion_writes_terminal_receipt`
- `resolution_worker_has_no_browser_route_or_final_submit_tools`
- `legacy_one_shot_ai_is_not_used_by_v2_producer`

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -p golish-sub-agents -E 'test(resolution_worker_) | test(legacy_one_shot_ai_)' --status-level fail
```

### 7.2 GREEN

1. 新增叶子工具 `enum_js_get_resolution_cluster`：输入 typed cluster ID，只返回 bounded source windows、capture/config anchors 与 candidate IDs。
2. 新增 `enum_js_submit_resolution`：每项必须引用 cluster candidate、source artifact/hash/span 或 capture ID；URL/method/params 仍过 deterministic validation。
3. AI 写入只能是 anchored `ai_inferred|ambiguous|unresolved` suggestion；若认为是噪声，只能写非终态 `ai_noise_suspected` reason，交 closed deterministic validator 判定。AI 不能写 `candidate_classification=noise`/`noise_excluded`，不能直接设置 `runtime_observed|static_confirmed` 或 canonical endpoint FK；所有 child 以 `parent_occurrence_id` 引用被分析的 immutable candidate。
4. 每 cluster 固定 token/source/attempt budget；耗尽写 terminal receipt，不能丢回自由聊天。
5. Resolution Analyst 使用独立 host-enforced tool mask；无网络、browser、route probe、任意文件读取或 final submit。
6. legacy one-shot 工具继续为历史 contract 可读，但 v2 spec/refiner/prompt 不再设置 `ai_assist=true`。

### 7.3 验证

```bash
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -p golish-sub-agents -E 'test(resolution_worker_) | test(legacy_one_shot_ai_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents --lib --tests -- -D warnings
```

**Commit boundary（有授权才执行）：** `feat(enumeration): add evidence-bound JS resolution worker`。

---

## Task 8：Server-owned Enumeration shards、角色与滚动波次

**Files:**

- Modify: `resources/harness/stages/enumeration/spec.json`
- Modify: `resources/harness/stages/enumeration/methodology.md`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/tests.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`

### 8.1 RED

新增/扩展测试：

- `enumeration_plan_is_server_worklist_owned`
- `enumeration_shards_partition_exact_origin_and_producer_without_overlap`
- `enumeration_browser_and_dir_wave_can_run_concurrently`
- `enumeration_jsapi_waits_for_browser_manifest_receipt`
- `enumeration_parameter_waits_for_runtime_and_static_occurrences`
- `enumeration_resolution_worker_only_spawns_for_unresolved_cluster`
- `enumeration_rolling_window_never_exceeds_company_or_global_caps`
- `enumeration_browser_jobs_never_exceed_global_two`
- `enumeration_retry_preserves_shard_identity_and_increments_attempt`
- `enumeration_controller_is_only_final_submitter`
- `enumeration_role_tool_masks_are_host_enforced`
- `enumeration_deterministic_lanes_never_dispatch_provider`
- `free_text_objective_cannot_expand_typed_shard_scope`

```bash
just space-guard && cd backend && cargo nextest run -p golish-agent-runtime -p golish-sub-agents -E 'test(enumeration_plan_is_server_) | test(enumeration_shards_) | test(enumeration_browser_and_dir_) | test(enumeration_jsapi_) | test(enumeration_parameter_) | test(enumeration_resolution_) | test(enumeration_rolling_) | test(enumeration_retry_) | test(enumeration_controller_) | test(enumeration_role_) | test(enumeration_deterministic_) | test(free_text_objective_)' --status-level fail
```

### 8.2 GREEN

1. 在 scheduler 定义 `EnumerationWorklistShard`，authority 字段与 design 一致；由 DB exact-origin worklist 生成，不由模型生成。
2. `sub_agent_call` 接受 typed shard ref；自由文本 objective 仅用于 UI，不参与 subject/producer authority 或 dedupe。
3. `stage_team_executor_specialist` 只映射 Company Controller 与 Resolution Analyst；Content/Browser/JSAPI/Parameter/Coverage 使用 server-owned deterministic executor，固定 `model=None/provider=None`，并由 host tool mask 限制，不再全部映射 `enumerator`。
4. company loop 实现 Wave 0–5 依赖；DIR/Browser 可并行，JSAPI 等 manifest，PARAM 等 occurrence，Resolution 条件启动，Review 最后以确定性只读 reducer 执行。
5. 冻结 `max_company_units_active=2`、每公司 `max_workers=3`、全局 deterministic jobs `=6`、全局 browser jobs `=2`、`global_provider_cap=4`、每公司 dynamic requests `=8`；Company Controller 与 Resolution Analyst 计入 provider cap，确定性 lane 计入 host job cap。
6. Controller-only final submit 与 close epoch 后 DB Gate 保持不变。
7. Worker crash/retry 从 durable shard 恢复；同 generation terminal cell 不重复领取。

### 8.3 验证

```bash
python3 -m json.tool resources/harness/stages/enumeration/spec.json >/dev/null
just space-guard && cd backend && cargo nextest run -p golish-agent-runtime -p golish-sub-agents -E 'test(enumeration_) | test(rolling_stage_team_child_drain_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-agent-runtime -p golish-sub-agents --lib --tests -- -D warnings
```

**Commit boundary（有授权才执行）：** `feat(enumeration): schedule server-owned analysis shards`。

---

## Task 9：Gate candidate closure、ReceiptV1 一致性与 shadow rollout

**Files:**

- Modify: `resources/harness/stages/enumeration/spec.json`
- Modify: `resources/harness/stages/enumeration/methodology.md`
- Modify: `backend/crates/golish-agent-kit/src/harness/stage_capability.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/resources.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- Modify: `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Create: `docs/validation/2026-08-02-enumeration-agent-team-v2-shadow.md`

### 9.1 RED

新增测试：

- `enumeration_gate_distinguishes_checked_empty_unresolved_and_unchecked`
- `enumeration_gate_requires_every_manifest_script_terminal`
- `enumeration_gate_requires_parameter_assessment_for_promotable_endpoint`
- `enumeration_gate_accepts_bounded_unresolved_receipt_without_counting_confirmed`
- `enumeration_gate_rejects_ai_claim_without_occurrence_evidence`
- `enumeration_terminal_producer_ownership_is_disjoint`
- `enumeration_receipt_v1_preflight_gap_keeps_four_axes_partial`
- `enumeration_refiner_uses_preflight_browser_extractor_route_owners`
- `enumeration_refiner_never_enables_one_shot_ai`
- `enumeration_shadow_reuses_capture_without_second_network_dispatch`
- `enumeration_contract_is_frozen_per_operation`
- `legacy_operation_remains_legacy_after_v2_available`

```bash
just space-guard && cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -E 'test(enumeration_gate_) | test(enumeration_terminal_) | test(enumeration_receipt_) | test(enumeration_refiner_) | test(enumeration_shadow_) | test(enumeration_contract_) | test(legacy_operation_)' --status-level fail
```

### 9.2 GREEN

1. Capability ownership固定：JS=browser、JSAPI=extractor/resolution reducer、PARAM=parameter reducer、DIR=route/content producer。
2. Gate 查询 occurrence lifecycle：每个 script、candidate 和 promotable endpoint 都必须有确定终态/参数评估。
3. `unresolved|ambiguous` 只有在 attempted + reason + bounded exhausted/unsupported receipt 完整时才可形成阶段 residual handoff；不计入 confirmed 或 checked-empty。
4. ReceiptV1 preflight failure 只表示 prerequisite gap，清理 methodology/prompt 中“block all four axes”的 legacy 漂移；LegacyV1 路径保持冻结兼容。
5. StageRefiner 补 `enum_preflight_web_origins`，JSAPI fallback 指向 extractor，browser args 不再带 `ai_assist=true`。
6. operation INSERT 的 DB trigger 从 server-owned `enumeration_analysis_rollout` 把值冻结到 `operation_state.enumeration_analysis_contract`；caller/profile 不直接选值。历史行在 migration 中 backfill `legacy_v1`，reset/purge 保留；fixture 环境只有在受控 rollout selector 指向 `agent_team_v2_shadow` 后，新 operation 才进入 shadow。
7. Shadow 只 tee 同一次 legacy/browser capture 给 v2 analyzer，禁止重复导航/抓取；写 occurrence/coverage comparison，但不改变 legacy Gate outcome/canonical projection。报告记录 recall、false positive、unresolved、token/worker cost、safety violations 和 provenance completeness。

### 9.3 Shadow promotion gate

`docs/validation/...shadow.md` 必须填入真实 focused fixture 命令/退出码和比较表，至少包含：

- simple fetch、two axios clients、relative document base、cross-file wrapper；
- duplicate runtime shapes、dynamic template、foreign origin、blocked write；
- legacy vs v2 confirmed/unresolved/noise 数；
- occurrence/parameter/evidence completeness；
- 模型 worker 次数与 budget。

报告未评审前，不把 production profile 默认值改成 `agent_team_v2`。

### 9.4 验证

```bash
python3 -m json.tool resources/harness/stages/enumeration/spec.json >/dev/null
just space-guard && cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -E 'test(enumeration_) | test(preflight_blocked_is_prerequisite_gap_not_content_coverage) | test(legacy_preflight_rows_keep_frozen_compatibility)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-agent-kit -p golish-agent-runtime --lib --tests -- -D warnings
```

**Commit boundary（有授权才执行）：** `feat(enumeration): gate endpoint candidate lifecycle in shadow`。

---

## Task 10：Typed artifact/progress IPC 与 IDOR 边界

**Files:**

- Modify: `backend/crates/golish-agent-app/src/ai/commands/stage_team.rs`
- Modify: `backend/crates/golish-pentest-app/src/security_analysis.rs`
- Modify: `backend/crates/golish-pentest-app/src/target_surface_hierarchy.rs`
- Modify: `backend/crates/golish/src/commands_facade/ai.rs`
- Modify: `backend/crates/golish/src/commands_facade/workspace.rs`
- Modify: `backend/crates/golish/src/commands_registry.rs`
- Modify: `frontend/lib/api/stage-team.ts`
- Create: `frontend/lib/api/stage-team.test.ts`
- Modify: `frontend/lib/api/security-analysis.ts`
- Modify: `frontend/lib/api/security-analysis.test.ts`
- Generated: `frontend/lib/generated/`

### 10.1 RED

Backend 新增测试：

- `enumeration_artifacts_require_operation_execution_scope`
- `enumeration_artifacts_filter_unit_and_worker`
- `enumeration_artifacts_are_bounded_and_cursor_stable`
- `enumeration_artifacts_return_allowlisted_occurrence_fields_only`
- `enumeration_artifacts_do_not_leak_canonical_output_or_secret_values`
- `enumeration_artifacts_expose_typed_execution_kind`
- `target_surface_endpoint_provenance_checks_target_ownership`
- `target_surface_endpoint_provenance_requires_operation_scope`
- `target_surface_endpoint_provenance_includes_all_occurrences`
- `target_surface_endpoint_provenance_rejects_sibling_project_org_and_wrong_operation`
- `target_surface_endpoint_candidates_include_unpromoted_residuals`
- `target_surface_endpoint_candidates_reject_sibling_project_org_and_wrong_operation`
- `target_surface_endpoint_reads_are_cursor_stable_and_clamped`

Frontend wrapper RED：

- command 名和 camelCase request 正确；
- snake/camel response 被 generated type约束；
- pagination cursor 和 loading/error 透传；
- wrapper 不直接裸 `invoke` 到组件。
- provenance/candidate 两条 request 都强制 operationId，不能默认“最近 operation”。

```bash
just space-guard && cd backend && cargo nextest run -p golish-agent-app -p golish-pentest-app -E 'test(enumeration_artifacts_) | test(target_surface_endpoint_provenance_) | test(target_surface_endpoint_candidates_) | test(target_surface_endpoint_reads_)' --status-level fail
pnpm exec vitest run frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.test.ts
```

### 10.2 GREEN

1. 新 Tauri command `ai_get_enumeration_artifacts`：operation + stage execution + 可选 unit/worker + cursor/limit，返回 progress summary、bounded occurrence summaries 和 typed refs；Stage Team worker row additive 增加 `executionKind=host_deterministic|llm_subagent`，provider/model 可空但 UI 不从空值推断 kind。
2. 新 Tauri command `target_surface_get_endpoint_provenance`：`targetId + endpointId + operationId + cursor + limit`，operation 必填；服务端验证 operation→project/organization、target ownership、frozen scope 与 endpoint linkage 后返回 occurrence/resolution/parameter/evidence detail。
3. 新 Tauri command `target_surface_list_endpoint_candidates`：`targetId + operationId + optional webOrigin/outcome filters + cursor + limit`，返回没有 canonical endpoint 的 ambiguous/unresolved/scope-excluded/AI-inferred residual occurrence；不得通过 endpoint inner join 把它们过滤掉。
4. `ai_get_enumeration_artifacts` 由 `commands_facade/ai.rs` 的既有 `crate::ai::commands::*` 导出；两条 Target Surface command 由 `commands_facade/workspace.rs` 的既有 pentest-app globs 导出。facade 代码只更新顶部公开命令组注释，不新增第二层 glob/手写 re-export；三条 command 仍按函数 → facade 可达性 → registry → frontend wrapper → ts-rs generated types 五步核验，禁止手改 generated 文件。
5. Error DTO 均含稳定 `code`；invalid UUID、scope mismatch、not found、cursor invalid 分开；错误 operation/sibling project/org 一律 fail closed，不泄漏 row 是否存在。
6. artifact response 不返回任意 worker `canonical_output`、raw request body/header value 或 response body。
7. 三条分页 API 默认 limit=50、server-side clamp 1–100；cursor 由 `(observed_at,id)` 稳定排序。全局/跨 operation Target Surface 不在本 Task，禁止 silent latest-operation fallback。

### 10.3 类型生成与验证

```bash
# 在 agent-progress.md 先记录当前 `git status --short frontend/lib/generated` 的既有共享改动；后续只接受下列 v2 DTO 对应的新 generated 路径，出现额外/破坏性 diff 时停止并审阅。
just space-guard && just gen-types
just space-guard && cd backend && cargo nextest run -p golish-agent-app -p golish-pentest-app -E 'test(enumeration_artifacts_) | test(target_surface_endpoint_provenance_) | test(target_surface_endpoint_candidates_) | test(target_surface_endpoint_reads_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-agent-app -p golish-pentest-app -p golish --lib --bins --tests -- -D warnings
pnpm exec vitest run frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.test.ts
pnpm exec biome check frontend/lib/api/stage-team.ts frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts
pnpm typecheck
```

预期：generated diff 只包含 `EnumerationArtifact*`、`StageTeamWorkerExecutionKind`、`TargetSurfaceEndpointProvenance*`、`TargetSurfaceEndpointCandidate*` 的 additive DTO；与开工 baseline 比较，既有 generated 文件不发生破坏性改名/删字段。实际生成文件名若超出该前缀 allowlist，先停止并更新计划/证据，不得直接纳入提交。

**Commit boundary（有授权才执行）：** `feat(enumeration): expose scoped analysis artifacts`。

---

## Task 11：现有统一 Stage Team Workspace 的 Enumeration artifact 与实时进度

> `unified-visible-stage-workspace-mock-2026-08-02` 已在本轮审计期间以现有 `StageTeamRunView` / `StageTeamWorkspaceView` 生产 shell 收口；本 Task 复用该实现，禁止另建平行 `StageWorkspace` 目录。

**Files:**

- Create: `frontend/components/Engagement/EnumerationArtifactView.tsx`
- Create: `frontend/components/Engagement/EnumerationArtifactView.test.tsx`
- Modify: `frontend/components/Engagement/StageTeamRunView.tsx`
- Modify: `frontend/components/Engagement/StageTeamRunView.test.tsx`
- Modify: `frontend/components/Engagement/StageTeamWorkspaceView.tsx`
- Modify: `frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`
- Modify: `frontend/lib/api/stage-team.ts`

### 11.1 RED

测试：

- Enumeration 使用统一 Workspace shell，不回退 Company Controller 汇总卡。
- 非终态每 1.5 秒重新读取 artifact summary，终态停止 polling。
- Stage Team read model 与 artifact page 同时 polling，切换 operation/execution 时两条旧请求都被 abort/sequence guard 丢弃，terminal 后两条都停止。
- origin/script/confirmed/inferred/unresolved/parameter 指标正确映射。
- `StageTeamWorkspaceView` controlled selection 在 Controller、host worker、LLM worker、无选择之间回传稳定 worker run ID；父级据此过滤 occurrence，选择 occurrence 更新右侧 evidence inspector。
- UI 只按 typed `executionKind` 显示 host deterministic/no-LLM 或 LLM subagent；不通过 role/provider/model 猜测。
- loading/error/empty 三态都可见且不会沿用上次 operation 数据。
- Recon/AU/Vuln/Verification 仍走原 adapter，不被 Enumeration 专用数据污染。

```bash
pnpm exec vitest run frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
```

### 11.2 GREEN

1. `StageTeamWorkspaceView` 增加 controlled `selectedWorkerRunId` 与 `onSelectionChange`（或等价 render-prop）合同；内部不再把 selection 封死。`StageTeamRunView` 在 `stageKind === "enumeration"` 时读取 `EnumerationArtifactPage`，用同一 selected worker 过滤后，把 `EnumerationArtifactView` 作为现有 Workspace 的 evidence supplementary child；不复制第二套 UI。
2. Stage Team 与 artifact polling 共享 operation/execution key、visibility/terminal 判断、abort controller 与单调 sequence；stage 非 terminal 且页面可见时两条流每 1.5 秒刷新，terminal 后同时停止，旧 response 不能覆盖新 operation。
3. role label 固定为 Content Mapper、Browser Runtime、JS/API Analyzer、Parameter Analyzer、Resolution Analyst、Coverage Reviewer。
4. worker 执行方式只读取 typed `executionKind`；occurrence badge 由正交 outcome 派生 display label，不把 `ai_inferred` 计入 confirmed。
5. evidence ID 变成可选择 typed artifact ref，不再只是 inert badge。

### 11.3 验证

```bash
pnpm exec vitest run frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
pnpm exec biome check frontend/components/Engagement/EnumerationArtifactView.tsx frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/lib/api/stage-team.ts
pnpm typecheck
```

**Commit boundary（有授权才执行）：** `feat(frontend): connect Enumeration to unified Stage Workspace`。

---

## Task 12：Target Surface endpoint provenance 与 URL 解析链

**Files:**

- Create: `frontend/components/TargetPanel/surface/EndpointProvenancePanel.tsx`
- Create: `frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx`
- Create: `frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.tsx`
- Create: `frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx`
- Modify: `frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`
- Modify: `frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts`
- Modify: `frontend/components/TargetPanel/surface/types.ts`
- Modify: `frontend/components/TargetPanel/surface/surfaceModel.ts`
- Modify: `frontend/components/TargetPanel/surface/surfaceModel.test.ts`
- Modify: `frontend/components/TargetPanel/surface/surfaceHierarchy.ts`
- Modify: `frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts`
- Modify: `frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`
- Modify: `frontend/components/TargetPanel/surface/tabs/EvidenceTab.tsx`
- Modify: `frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx`
- Modify: `frontend/lib/api/security-analysis.ts`

### 12.1 RED

测试：

- canonical endpoint 下按时间/强度列出 browser + 两个 JS callsite 三条 occurrence。
- 展示 page、JS URL/hash、line/column、capture/initiator。
- 展示 `raw → client/document base → candidates → selected URL` 完整链。
- ambiguous/unresolved/scope-excluded 单独分组，不混入 executable endpoint。
- parameter 显示 path/query/body/form/header/graphql_variable、类型、confidence、source；无 value。
- evidence 数取自 typed links，不再由 passive log 或 path/method heuristic 推断。
- 加载、错误、空 provenance 三态；legacy endpoint 显示“历史数据无 occurrence”，不伪造来源。
- candidate residual panel 在没有 canonical endpoint 时仍列出 ambiguous/unresolved/scope-excluded/AI-inferred occurrence，并能按 web origin/outcome 分页筛选。
- operation context 必填；切换 operation/target/endpoint 时取消旧请求，sibling project/org 或错误 target 的 occurrence 不显示，也不 fallback 到 latest operation。

```bash
pnpm exec vitest run frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx
```

### 12.2 GREEN

1. Endpoint row 保持轻量；用户展开时用当前明确 operation context 按需调用 provenance command，不查询或猜测“最近 operation”。
2. `EndpointProvenancePanel` 使用 typed occurrence，无 path/method/source heuristic。
3. 参数来源跳转 occurrence；evidence link 跳转 allowlisted detail/capture Inspector。
4. `EvidenceTab` 合并 timeline 与 ledger links，不因已有 timeline 隐藏 occurrence evidence。
5. legacy rows明确 degraded state；不把“无 provenance”显示成“已检查为空”。
6. `EndpointCandidateResidualsPanel` 独立调用 candidate list command；无 canonical link 的 occurrence 仍可从 origin/candidate 视图进入解析链和 evidence detail。

### 12.3 验证

```bash
pnpm exec vitest run frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx
pnpm exec biome check frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/types.ts frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.tsx frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx frontend/lib/api/security-analysis.ts
pnpm typecheck
```

**Commit boundary（有授权才执行）：** `feat(frontend): show endpoint occurrence provenance`。

---

## Task 13：模块卡、证据记录与 focused 收尾

**Files:**

- Modify: `docs/modules/backend/golish-js-analyzer.md`
- Modify: `docs/modules/backend/golish-pentest-app.md`
- Modify: `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- Modify: `docs/modules/backend/golish-db/repo.md`
- Modify: `docs/modules/backend/golish-app-core/ports.md`
- Modify: `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- Modify: `docs/modules/backend/golish-agent-kit/harness.md`
- Modify: `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- Modify: `docs/modules/backend/golish-sub-agents/defaults.md`
- Modify: `docs/modules/backend/golish-sub-agents/executor.md`
- Modify: `docs/modules/backend/golish-agent-app/ai.md`
- Modify: `docs/modules/backend/golish/commands_facade.md`
- Modify: `docs/modules/frontend/components.md`
- Modify: `docs/modules/frontend/lib.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `docs/validation/2026-08-02-enumeration-agent-team-v2-shadow.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

### 13.1 模块事实源

每张卡同步：

- 职责与公开接口；
- typed shard/role/tool mask；
- capture v3 与 redaction；
- occurrence schema/guard/purge；
- resolution/parameter/provenance contract；
- Gate ownership与测试入口；
- legacy/shadow/v2 compatibility。

`docs/modules/INDEX.md` 只更新真正改动模块的状态/摘要，不创建孤儿卡。

### 13.2 最终 focused verification

每个 Cargo 命令都单独先跑 `just space-guard`：

```bash
node --test scripts/browser_collect_js_api.test.mjs

just space-guard && cd backend && cargo nextest run -p golish-js-analyzer --lib --status-level fail
just space-guard && cd backend && cargo nextest run -p golish-db --test enumeration_endpoint_provenance_v2 --test enumeration_surface_manifest --status-level fail
just space-guard && cd backend && cargo nextest run -p golish-pentest-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-kit -p golish-agent-app -E 'test(capture_v3_) | test(browser_occurrence_) | test(resolution_v2_) | test(parameter_reducer_) | test(parameter_assessment_) | test(resolution_worker_) | test(enumeration_) | test(target_surface_endpoint_provenance_) | test(target_surface_endpoint_candidates_) | test(target_surface_endpoint_reads_)' --status-level fail
just space-guard && cd backend && cargo clippy -p golish-js-analyzer -p golish-db -p golish-app-core -p golish-pentest-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-kit -p golish-agent-app -p golish --lib --bins --tests -- -D warnings

pnpm exec vitest run frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.test.ts frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx
pnpm exec biome check frontend/lib/api/stage-team.ts frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/Engagement/EnumerationArtifactView.tsx frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.tsx frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/types.ts frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx
pnpm typecheck

python3 -m json.tool resources/harness/stages/enumeration/spec.json >/dev/null
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json

git diff HEAD --check -- scripts/browser_collect_js_api.mjs scripts/browser_collect_js_api.test.mjs backend/crates/golish-js-analyzer/src/lib.rs backend/crates/golish-js-analyzer/src/patterns.rs backend/crates/golish-js-analyzer/src/ast_filter.rs backend/crates/golish-js-analyzer/src/lib_tests.rs
git diff HEAD --check -- backend/crates/golish-db/migrations/20260802000003_enumeration_endpoint_provenance_v2.sql backend/crates/golish-db/src/repo/enumeration_endpoint_occurrences.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/capability_execution_receipts.rs backend/crates/golish-db/src/repo/enumeration_surface_manifest.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/stage_purge.rs backend/crates/golish-db/tests/enumeration_endpoint_provenance_v2.rs backend/crates/golish-app-core/src/ports/recon/scans.rs
git diff HEAD --check -- backend/crates/golish-pentest-app/src/pentest_bridge/js_api_analysis_contract.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_api_module_graph.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_api_resolution.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_resolution_clusters.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs backend/crates/golish-pentest-app/src/pentest_bridge/evidence.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_extract.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_recipe.rs
git diff HEAD --check -- backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-sub-agents/src/defaults/tests.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-agent-kit/src/harness/stage_capability.rs backend/crates/golish-agent-kit/src/harness/resources.rs backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs
git diff HEAD --check -- backend/crates/golish-agent-app/src/ai/commands/stage_team.rs backend/crates/golish-pentest-app/src/security_analysis.rs backend/crates/golish-pentest-app/src/target_surface_hierarchy.rs backend/crates/golish/src/commands_facade/ai.rs backend/crates/golish/src/commands_facade/workspace.rs backend/crates/golish/src/commands_registry.rs resources/harness/stages/enumeration/spec.json resources/harness/stages/enumeration/methodology.md
git diff HEAD --check -- frontend/lib/api/stage-team.ts frontend/lib/api/stage-team.test.ts frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/Engagement/EnumerationArtifactView.tsx frontend/components/Engagement/EnumerationArtifactView.test.tsx frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
git diff HEAD --check -- frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts frontend/components/TargetPanel/hooks/useTargetSurfaceData.test.ts frontend/components/TargetPanel/surface/types.ts frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/EndpointProvenancePanel.tsx frontend/components/TargetPanel/surface/EndpointProvenancePanel.test.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.tsx frontend/components/TargetPanel/surface/EndpointCandidateResidualsPanel.test.tsx frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.tsx frontend/components/TargetPanel/surface/tabs/EvidenceTab.test.tsx
git diff HEAD --check -- docs/modules/backend/golish-js-analyzer.md docs/modules/backend/golish-pentest-app.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-app-core/ports.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish/commands_facade.md docs/modules/frontend/components.md docs/modules/frontend/lib.md docs/modules/INDEX.md docs/design/2026-08-02-enumeration-agent-team-js-api-analysis-v2.md docs/superpowers/plans/2026-08-02-enumeration-agent-team-js-api-analysis-v2.md docs/validation/2026-08-02-enumeration-agent-team-v2-shadow.md docs/design/INDEX.md docs/superpowers/plans/INDEX.md feature_list.json agent-progress.md
```

`frontend/lib/generated` 不允许以上一整个目录作为隐式 scope。Task 10 运行 `just gen-types` 前后各读取一次 `git status --short frontend/lib/generated`，把“本 feature 新增的 name-only 集合”逐项写入 `agent-progress.md`；它只能包含已审阅的 `EnumerationArtifact*`、`StageTeamWorkerExecutionKind`、`StageTeamWorkerView.ts`、`TargetSurfaceEndpointProvenance*`、`TargetSurfaceEndpointCandidate*`。随后把这些实际文件名逐项传给单独的 `git diff HEAD --check -- ...`；额外文件、删除、改名或超出 baseline 的既有 generated diff 都立即停止。对所有本 feature 新建且尚未 tracked 的文件，再用同一个精确 name-only 集合执行 `rg -n '[ \t]+$|^(<<<<<<<|=======|>>>>>>>)'`，预期无匹配；禁止对 `frontend/lib/generated`、`frontend/components/TargetPanel`、`docs/modules` 或任何 crate 目录做目录级 `git add`/diff allowlist。

### 13.3 完成判定

只有同时满足以下条件才能将 feature 从 `in_progress` 改为 `passing`：

1. 上述 focused commands 实际运行并全绿，命令、退出码、关键输出写入 `agent-progress.md` 和 feature evidence。
2. `feature_list.json.verification` 逐条核对；未授权的 full gate 如实记录“按项目策略未运行”。
3. Shadow report 已填写真实 fixture 对比且无 safety violation；shadow implementation/report 只能作为 cutover 前置证据，不能单独把本 production feature 标为 `passing`。
4. 用户已在 shadow 报告之后单独批准 production cutover，受控 selector 只影响新 operation；至少一个 `agent_team_v2` production-contract focused fixture 证明新 Gate/projection/UI 行为。未获 cutover 批准时 feature 保持 `in_progress` 或按实际依赖转 `blocked`，不得以“shadow 已完成”冒充完成。
5. `clean-state-checklist.md` 逐项检查；本 scope 未提交文件全部列入 progress。

按 `AGENTS.md §0.1`，本计划不默认运行 `./init.sh`、`just precommit`、`just check`、全 workspace `just test-rust`、全量前端或 E2E。用户若另行明确授权，才把这些作为额外门禁，不能用它们替代 focused evidence。

**Final commit boundary（仅用户要求提交时）：** 逐一使用 Task 1–12 已列出的精确 `git add` 路径并复核 staged diff，再执行 `git commit -m "feat(enumeration): add evidence-driven JS API agent team"`。禁止使用 `git add -A`，禁止把共享 dirty tree 其他功能纳入提交，禁止未获确认 push。
