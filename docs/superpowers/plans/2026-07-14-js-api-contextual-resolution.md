# JS/API 候选上下文解析 v1 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在不启用 AI、不改 DB schema 的前提下，让同一 JS 文件中的命名 axios client 按各自 baseURL 解析 raw endpoint，并在 resolved URL 后才去重和落库。
**架构：** `golish-js-analyzer` 以并行 additive API 暴露 source span/receiver call-site evidence；`js_extract_apis` 建同文件 client/base binding index，生成 versioned resolution evidence，再复用现有 exact-origin classifier 投影 `api_endpoints`。旧 `Endpoint` 与旧 extractor API 保持兼容。
**技术栈：** Rust 2021、regex、ast-grep 既有 call-site filter、serde JSON、sqlx guarded persistence、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish-js-analyzer/src/lib.rs`：公开 candidate/context/span 类型和兼容 extractor API。
- 修改 `backend/crates/golish-js-analyzer/src/patterns.rs`：让 candidate 构建端保留 callee/receiver 所需捕获事实，不改变 endpoint 识别规则。
- 修改 `backend/crates/golish-js-analyzer/src/lib_tests.rs`：验证 span、receiver、minified 同行和旧 API 兼容。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`：client/base index、resolution evidence、resolved projection/dedupe。
- 修改 `docs/modules/backend/golish-js-analyzer.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`：同步模块事实源。
- 修改 `agent-progress.md`、`feature_list.json`：记录状态和可重放证据。

## Task 1：Analyzer call-site candidate API

**文件：**

- `backend/crates/golish-js-analyzer/src/lib.rs`
- `backend/crates/golish-js-analyzer/src/lib_tests.rs`

**步骤：**

1. 先写失败测试，要求：

```rust
let src = "const admin=axios.create({baseURL:'/admin'});admin.get('/users');";
let candidates = extract_candidates_from_source("app.js", src);
assert_eq!(candidates[0].call.receiver.as_deref(), Some("admin"));
assert_eq!(&src[candidates[0].call.span.start_byte..candidates[0].call.span.end_byte],
           "admin.get('/users')");
```

2. 增加 `SourceSpan`、`CallSiteContext`、`EndpointCandidate`、
   `CandidateExtractReport`，把当前 `(offset, Endpoint)` hit 流升级为 candidate 流。
3. 为 fetch/axios/custom client/config/jQuery/Request/concat/template 设置确定的
   `callee/receiver`；AST range 仍是最终真假门禁。
4. 让旧 `extract_from_source/files` 从新 candidate API 投影，验证旧 endpoint 数量、顺序、
   serde 结构不变。

**验证：**

```bash
cd backend && cargo nextest run -p golish-js-analyzer --lib --status-level fail
```

预期：新增测试先因 API 不存在 RED；实现后 analyzer 全部测试 GREEN。

**提交：** 本轮不自动 commit；仅在用户另行要求后提交。

## Task 2：同文件 axios client/base index

**文件：**

- `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`

**步骤：**

1. 先写纯函数失败测试覆盖：

```rust
const ADMIN: &str = "const BASE='/admin-api'; const admin=axios.create({baseURL:BASE});";
const PUBLIC: &str = "const open=axios.create({baseURL:'/public-api'});";
```

断言 `admin -> /admin-api`、`open -> /public-api`；alias 可传播；同名不同文件隔离；
同一 client 两个不同 base 返回 `ambiguous`。
2. 实现 `ApiBaseContextIndex`：literal symbol、axios factory、defaults assignment、alias
   edge，固定点迭代上限为 identifier 数量，禁止跨文件传播。
3. base 接受 root-relative、absolute 与 protocol-relative 静态值；后两者原样保留为 scope
   candidate，最终仍交给 exact-origin classifier，不把 foreign origin 截成同源 path。
4. legacy global prefix 修正 segment boundary：`raw == base` 或 `raw` 以 `base + '/'` 开头
   才算已带 prefix；命名 Axios client 单独遵循 Axios combine，不能套用 legacy 防双拼。
5. binding 按 lexical scope 与 source-order fail closed；mutable/reassigned symbol、晚于 call 的
   base fact、后置 object spread 和 opaque member-chain 都保留 unresolved evidence。

**验证：**

```bash
cd backend && cargo nextest run -p golish-pentest-app js_extract_apis --status-level fail
```

预期：index 测试先因类型/行为不存在 RED，最小实现后 GREEN。

**提交：** 本轮不自动 commit；仅在用户另行要求后提交。

## Task 3：Contextual projection 与 durable raw evidence

**文件：**

- `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`

**步骤：**

1. 写失败测试：两个 candidate 都是 `GET /users`，receiver 分别为 `admin/open`，projection
   必须得到 `/admin-api/users` 和 `/public-api/users`；raw endpoint path 仍为 `/users`。
2. DB projection 从 `EndpointCandidate` 流开始，先解析 context，再按
   `(normalized method, resolved URL)` 去重；HAE/AI-only endpoint 保留 legacy fallback，
   但相同 raw call-site 不得覆盖更强的 deterministic candidate。
3. known client 的 dynamic/conflicting base 产出 `unresolved`，不回退到第一个 global base；
   fetch/未知 wrapper 不继承唯一 axios instance base。
4. 在每文件 `raw_analysis` 写 `contextual_resolution_v1`，包含 bounded binding evidence、
   candidate id/span/raw path/receiver/base/resolved path/disposition/reason；tool result 增加计数与
   bounded sample，batch compaction 保留计数。
5. 复用现有 guarded upsert、authorization revalidation、evidence/outcome 发布，不增加事务内 I/O。
6. receiver-less fetch/Request/jQuery 固定走 origin-root；无前导 `/` 的 custom-client path 只有
   在唯一命名 binding 下才投影。HAE/AI supplemental 也必须先解析再做 resolved URL 去重。

**验证：**

```bash
cd backend && cargo nextest run -p golish-pentest-app js_extract_apis --status-level fail
```

预期：多 client、ambiguous、fetch isolation、segment boundary 全部 GREEN。

**提交：** 本轮不自动 commit；仅在用户另行要求后提交。

## Task 4：文档、静态检查与收尾

**文件：**

- `docs/modules/backend/golish-js-analyzer.md`
- `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- `docs/modules/INDEX.md`
- `agent-progress.md`
- `feature_list.json`

**步骤：**

1. 模块卡记录 candidate API、同文件 resolution v1、AI/跨 chunk 非目标，并修正
   `EndpointSource` 已含 `Hae` 的旧描述。
2. 运行 formatter、聚焦测试和 clippy；随后尝试全量 `just precommit`。若共享树中既有
   未完成改动导致失败，记录精确命令/退出码/首个非本功能 blocker，feature 保持
   `blocked`（若另一个 feature 正持有唯一 active slot），不得抢占或宣称 passing。
3. 将实际命令、退出码、关键输出写入 progress 和 feature evidence；核对只存在一个
   `in_progress`。

**验证：**

```bash
just space-guard
cd backend && cargo fmt -p golish-js-analyzer -p golish-pentest-app --check
cd backend && cargo nextest run -p golish-js-analyzer --lib --status-level fail
cd backend && cargo nextest run -p golish-pentest-app js_extract_apis --status-level fail
cd backend && cargo clippy -p golish-js-analyzer -p golish-pentest-app --all-targets -- -D warnings
just precommit
git diff --check
jq empty feature_list.json
```

预期：聚焦命令全绿；只有 full precommit 也全绿后才能把 feature 改为 `passing`。

**提交：** 本轮不自动 commit；仅在用户另行要求后提交。

## Execution status（2026-07-14）

- Analyzer candidate API、context index、resolved projection、bounded raw evidence 与兼容投影已实现。
- 聚焦测试与 targeted Clippy 已通过；完整命令/run id 记录在 `agent-progress.md` 和
  `feature_list.json`。
- 未运行 `init.sh`，未做外部请求/AI 调用/Test1 live rerun，未 commit/push。
- `just precommit` 与 clean-state 结论以本轮最终 progress 记录为准。
