# Enumeration Web Origin 与非终态收口实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 Enumeration 以 exact Web Origin×四技术为分母，并保证 `error` / `partial` 在 worklist、gate、submit preview、pass-token 上都保持未完成。

**架构：** 在 `golish-pentest-domain` 建立共享 Web Origin identity；app/agent 层把现有 target metadata 展开为 origin rows；Enumeration completion 只认 origin-keyed `technique_outcomes`。三个 bridge 保留 partial discoveries 但写 `partial` marker，完整复跑再覆盖终态。数据库 schema 与 `golish-db` crate保持不动。

**技术栈：** Rust 2021、serde/serde_json、url、Tauri command read model、React 19/Vitest、cargo nextest。

## 文件结构

- `backend/crates/golish-pentest-domain/src/web_origin.rs`：唯一的 HTTP(S) origin normalization。
- `backend/crates/golish-agent-kit/src/harness/{stage_spec,gate/rule_engine,org_gate}.rs`：Enumeration error 与 origin gate合同。
- `backend/crates/golish-agent-kit/src/{db_traits,tool_executors/security.rs}`：origin root DTO、worklist 的 partial 状态与 root 输出。
- `backend/crates/golish-agent-app/src/ai/{commands/stage_coverage.rs,db_bridge/recon.rs,harness_submit_tool.rs}`：origin snapshot、submit preview同口径。
- `backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api,js_extract_apis,route_probe_paths,enumeration_capabilities}.rs`：origin-keyed outcomes、partial marker、Katana exact-origin scope。
- `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：origin row key 与 partial 展示。
- `resources/harness/stages/enumeration/{spec.json,methodology.md}`：声明 Enumeration 专属合同。
- 对应模块卡与 `agent-progress.md`：记录公开合同和验证证据。

## Task 1：共享 Web Origin identity（TDD）

**文件：**
- 创建 `backend/crates/golish-pentest-domain/src/web_origin.rs`
- 修改 `backend/crates/golish-pentest-domain/src/lib.rs`
- 修改 `backend/crates/golish-pentest-domain/Cargo.toml`

**步骤 1：写失败测试。** 测试 API 固定为：

```rust
assert_eq!(
    canonical_web_origin("HTTPS://A.Example/login?q=1").unwrap().key,
    "https://a.example:443"
);
assert_ne!(
    canonical_web_origin("http://a.example:8080/").unwrap().key,
    canonical_web_origin("https://a.example:8080/").unwrap().key
);
assert!(canonical_web_origin("https://u:p@a.example/").is_none());
```

**步骤 2：验证 RED。**

```bash
cd backend && cargo test -p golish-pentest-domain canonical_web_origin --lib
```

预期：因 `canonical_web_origin` 尚不存在而编译失败。

**步骤 3：最小实现。** 使用 `url::Url`，只接受 http/https，显式化 80/443，
输出 `key/root_url/scheme/host/port`，并从 crate root re-export。

**步骤 4：验证 GREEN。** 重跑上条命令，预期全部通过。

## Task 2：error 不得让 Enumeration gate PASS（TDD）

**文件：**
- 修改 `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
- 修改 `resources/harness/stages/enumeration/spec.json`

**步骤 1：写失败测试。** 把既有 error characterization 保留给默认 stage，新增：

```rust
#[test]
fn enumeration_error_fact_keeps_coverage_incomplete() {
    let spec = enumeration_spec_with_expected("GOLISH-ENUM-DIR");
    let ctx = context_with_fact("https://a.example:443", "GOLISH-ENUM-DIR", EvidenceOutcome::Error);
    assert!(matches!(evaluate_rules(&spec, &deliverable(), &ctx), GateDecision::Block { .. }));
}
```

**步骤 2：验证 RED。**

```bash
cd backend && cargo test -p golish-agent-kit enumeration_error_fact_keeps_coverage_incomplete --lib
```

预期：当前 `error_ok` 令 gate PASS，断言失败。

**步骤 3：最小实现。** `coverage_complete` 新增 `error_is_terminal: bool`（serde
default `true`）；当 false 且 exact cell 有 Error 时，found/empty/error 均不得闭格。
Enumeration spec 设 false。

**步骤 4：验证 GREEN。** 重跑单测，并运行：

```bash
cd backend && cargo nextest run -p golish-agent-kit rule_engine stage_spec --status-level fail
```

## Task 3：三个 bridge 写 exact-origin completion / partial marker（逐工具 TDD）

**文件：**
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`
- 修改 `backend/crates/golish-pentest-app/Cargo.toml`

**步骤 1：route RED。** 新增断言：队列未完成时，无论已有 match 与否，
`outcome="partial"`；完整无命中才 `empty`；完整全 transport failure 才 `error`。

```bash
cd backend && cargo test -p golish-pentest-app route_probe_outcome --lib
```

预期：当前 partial/no-match 返回 empty，测试失败。

**步骤 2：route GREEN。** outcome helper 接收 `queue_completed`，partial row 写
origin key；响应增加 `completion_state`，原始 directory rows照常落库。重跑测试。

**步骤 3：browser RED/GREEN。** 新增
`closure_partial_keeps_js_jsapi_and_param_incomplete_even_with_rows`，先确认当前
found 断言失败，再让 JS/JSAPI/PARAM 写 partial；hard timeout保持 error。

```bash
cd backend && cargo test -p golish-pentest-app browser_collect_js_api --lib
```

**步骤 4：js_extract RED/GREEN。** 新增
`partial_extract_keeps_jsapi_and_param_incomplete`，partial 时两轴写 partial；完整
运行才按结果写 found/empty。

```bash
cd backend && cargo test -p golish-pentest-app js_extract_apis --lib
```

**步骤 5：origin 一致性。** 三个 outcome writer 都通过
`canonical_web_origin(effective_url).key` 写 evidence asset 与 technique outcome。

## Task 4：Web Origin 分母、read model 与 worklist（TDD）

**文件：**
- 修改 `backend/crates/golish-agent-kit/src/db_traits/{types.rs,repo.rs}`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`
- 修改 `backend/crates/golish-agent-kit/src/tool_executors/security.rs`

**步骤 1：写失败测试。** 一个 target 带 confirmed URL
`http://a:80`、`https://a:443`、`https://a:8443` 时，snapshot 必须有 3 assets、
12 cells；path 变体去重，closed/non-http排除。

```bash
cd backend && cargo test -p golish-agent-app enumeration_web_origins --lib
```

**步骤 2：实现 origin root DTO 与展开。** 保留 owner `target_id` / host value，
coverage asset/value改为 origin key；root_url直接来自共享 helper，不再
`max_by_key`。

**步骤 3：写 worklist RED/GREEN。** 同 target 两 origin 的 `work_item_id` 必须不同，
每项 root_url/scheme/port精确；`partial` 计入 unfinished并进入默认 next page。

```bash
cd backend && cargo test -p golish-agent-kit enumeration_worklist --lib
```

**步骤 4：read model completion优先级。** exact origin outcome 的
`partial/error` 优先于任何兼容 found；Enumeration 四轴不再消费 host-level
business-table found投影。完整 origin outcome 才显示 found/empty。

## Task 5：org gate 与 submit preview 使用同一 origin axis（TDD）

**文件：**
- 修改 `backend/crates/golish-agent-kit/src/harness/{org_gate.rs,gate/rule_engine.rs}`
- 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`

**步骤 1：写 gate RED。** 两个 origin、只有 HTTPS fact 时必须仍缺 HTTP；
`http://h:8080` 与 `https://h:8080` 不得被 `canon_asset` 合并。

```bash
cd backend && cargo test -p golish-agent-kit enumeration_origin --lib
```

**步骤 2：实现 technique-aware join。** ENUM 四轴使用
`canonical_web_origin` key；其它 technique保留 `canonical_asset_key`。

**步骤 3：submit preview parity。** 同一 fixture 在 preview 与 org gate 都 BLOCK；
补齐第二 origin 后都 PASS。

```bash
cd backend && cargo test -p golish-agent-app submit_preview_feeds_enumeration --lib
cd backend && cargo nextest run -p golish-agent-kit org_gate rule_engine --status-level fail
```

## Task 6：Katana exact-origin 调度边界（TDD）

**文件：**
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/enumeration_capabilities.rs`

**步骤 1：写失败测试。** scope matcher/regex接受授权 exact origin及其路径，拒绝
scheme flip、wrong port、sibling、`example.com.evil`；wrapper args必须含 anchored
scope约束。

**步骤 2：验证 RED。**

```bash
cd backend && cargo test -p golish-pentest-app exact_origin_scope --lib
```

**步骤 3：最小实现并 GREEN。** 从 normalized roots构造 anchored union scope，
传入 Katana command；保留输出过滤作纵深防御。重跑测试。

## Task 7：前端 origin key / partial 呈现（TDD）

**文件：**
- 修改 `frontend/components/Engagement/StageAssetCoveragePanel.tsx`
- 修改 `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`

**步骤 1：写失败测试。** 同 target 两 origin 同时渲染且无 duplicate-key；partial
cell显示“部分完成/需续跑”，不显示完成图标。

```bash
pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx
```

**步骤 2：实现并 GREEN。** React/group key 使用 `target_id + value(origin)`；
partial加入未完成样式与计数。重跑 Vitest 和 Biome。

## Task 8：文档、完整验证与 fresh run

在进入最终验证前，先完成 browser collector 的 durable checkpoint 活性回归：

- `max_pages` 只切分执行，不截断安全 same-origin link discovery；
- manifest v2 持久化 page queue、recursive JS queue、已验证 scripts/API 与
  per-item recovery signatures；
- 同 run/session/trusted operation/stage attempt 可继续 cursor，任何 provenance、
  exact-origin、危险路由或旧脚本完整性不匹配均 fail-closed；
- navigation/body/recursive/pending-wait 同 signature 第二次失败后输出
  nonterminal `recovery_exhausted`，第三次同 provenance 不再发请求，AI recipe 停止；
- 不得把 exhausted 升成 checked-empty/blocked。

回归命令：

```bash
node --check scripts/browser_collect_js_api.mjs
node --test scripts/browser_collect_js_api.test.mjs
cd backend && cargo nextest run -p golish-pentest-app js_ai_recipe --status-level fail
```

预期：301-link/page-slice、recursive-chain、retry-breaker 与 Rust AI breaker 全绿。

**文件：**
- 修改相关 `docs/modules/` 卡、`docs/modules/INDEX.md`
- 修改 `resources/harness/stages/enumeration/methodology.md`
- 修改 `feature_list.json`、`agent-progress.md`

**步骤 1：scoped 验证。**

```bash
cd backend && cargo fmt --check
cd backend && cargo nextest run -p golish-pentest-domain -p golish-agent-kit -p golish-agent-app -p golish-pentest-app --status-level fail
cd backend && cargo clippy -p golish-pentest-domain -p golish-agent-kit -p golish-agent-app -p golish-pentest-app --all-targets -- -D warnings
pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx
just check-fe
```

预期：exit 0、0 failed、0 warnings。

**步骤 2：headless fixture。** seed 同 host 多 origin + 一个 partial，确认
preflight/gate一致 BLOCK；完整复跑后 PASS。

**步骤 3：最终门禁。**

```bash
just precommit
```

预期：全部检查通过。把命令、exit code、关键统计写入 `agent-progress.md` 和
feature evidence；缺任一项则保持 `in_progress`。

**步骤 4：fresh Test1。** 在现有授权 workspace 重启新后端后启动 fresh
Enumeration，使用 `scripts/run_tree.py --workspace ... --full --db` 核对：origin
数量、partial/error格、preflight verdict、gate verdict、completion/pass-token。
