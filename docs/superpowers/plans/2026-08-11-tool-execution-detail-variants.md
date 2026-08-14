# 工具执行详情变体实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 Codex 式 Tool Activity 对进程内 HTTP 工具显示真实请求详情，并让 Nuclei CLI wrapper 展示 active runner 返回的真实命令。
**架构：** 前端 presentation adapter 保留现有 terminal 字段并增加严格的 HTTP variant；组件按 variant 渲染 HTTP observation 卡或 terminal 卡。后端 Nuclei facade 只在 active runner 真正返回后生成白名单 `runner_execution`，前端读取该 exact path，不从 wrapper args 或任意 nested object 重建命令。
**技术栈：** React 19、TypeScript 6、Vitest/Testing Library、Rust 2021、serde_json、cargo nextest、Biome、Clippy。

## 文件结构

- 修改 `frontend/components/Engagement/toolActivityPresentation.ts`：HTTP execution 类型、严格 result/observation 解析、匿名访问 copy。
- 修改 `frontend/components/Engagement/toolActivityPresentation.test.ts`：HTTP object/JSON-string/empty/malformed contract 测试。
- 修改 `frontend/components/Engagement/ToolActivityDisclosure.tsx`：HTTP request summary 与单 observation disclosure。
- 修改 `frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`：真实展开交互、timeout/empty/no-fake-curl 回归。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`：active Nuclei runner execution annotation 与纯单测。
- 修改 `docs/modules/frontend/components.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`：同步模块事实源。
- 修改 `feature_list.json`、`agent-progress.md`：唯一 active 状态和新鲜验证证据。

## 任务 1：写 HTTP presentation RED 测试

**文件：**
- 修改：`frontend/components/Engagement/toolActivityPresentation.test.ts`

**步骤 1：** 构造 `vuln_probe_anonymous_access` object result，包含一个 `GET /admin` 200 suspicious observation 与一个 `HEAD /profile` request_timeout observation，断言：

```ts
expect(presentation.execution).toMatchObject({
  kind: "http",
  origin: "https://api.example.test:443",
  selectedCount: 2,
  networkAttempted: true,
  requests: [
    { method: "GET", path: "/admin", statusCode: 200, verdict: "suspicious" },
    { method: "HEAD", path: "/profile", errorClass: "request_timeout" },
  ],
});
expect(presentation.command).toBeNull();
```

**步骤 2：** 增加一层 JSON-string result、`selected_count=0/network_attempted=false/observations=[]`、非 object observation/错误 scalar 被忽略的测试；nested JSON string 不递归解析。

**步骤 3：** 运行：

```bash
pnpm exec vitest run frontend/components/Engagement/toolActivityPresentation.test.ts
```

预期：因 `execution` 尚不存在而失败，exit 非零；记录 RED 证据。

## 任务 2：实现 HTTP adapter 并跑 GREEN

**文件：**
- 修改：`frontend/components/Engagement/toolActivityPresentation.ts`

**步骤 1：** 新增 `HttpExecutionPresentation`、`HttpRequestPresentation`、response/query-binding 类型，并给 `ToolActivityPresentation` 增加 `execution`。

**步骤 2：** 新增严格 helper：number 只接受 finite number、boolean/string 不互转；observation 必须是 plain object且 `method/path` 为非空 string；query binding 只接受 exact string pair；response 只读 exact fingerprint 字段。

**步骤 3：** 只在工具名等于 `vuln_probe_anonymous_access` 时创建 `{kind:"http"}`；给该工具 copy 设置 `Probing/Probed anonymous access` 与 runner `Golish HTTP client`。命令字段继续仅由既有 exact result.command/requested-shell 逻辑决定。

**步骤 4：** 重跑任务 1 的 focused Vitest，预期全部通过、exit 0。

## 任务 3：写 HTTP disclosure RED 测试

**文件：**
- 修改：`frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`

**步骤 1：** 构造与实体 transcript 同 shape 的匿名访问 tool，展开 activity 与 tool，断言 region `Probed anonymous access HTTP requests` 存在，显示 Origin、`GET /admin`、`200`、`Suspicious`、timeout/error，且不存在 `$ curl`。

**步骤 2：** 点击单 request disclosure，断言 `Query overrides`、captured bytes、SHA-256/truncated 等 exact fingerprint 可见。

**步骤 3：** 构造 selected_count 0 的完成结果，断言 `No HTTP requests were sent` 与 `0 endpoints selected`，Raw Data 默认折叠。

**步骤 4：** 运行：

```bash
pnpm exec vitest run frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
```

预期：因 HTTP section 尚不存在而失败，exit 非零；记录 RED 证据。

## 任务 4：实现 HTTP request disclosure

**文件：**
- 修改：`frontend/components/Engagement/ToolActivityDisclosure.tsx`

**步骤 1：** 新增 HTTP section，header 明确 `HTTP requests` 与 `In process`，origin 独立显示。

**步骤 2：** 每个 observation 用 button disclosure 显示 `METHOD PATH`、status/error、verdict；展开后显示 Query overrides、response fingerprint 与 network-attempted truth。

**步骤 3：** requests 为空时显示 `No HTTP requests were sent`；selected count 单独显示，不把 Completed 等价成已发网络请求。

**步骤 4：** 将 HTTP variant 纳入 execution details；terminal 仍只在 command/output/job/hint 存在时渲染。重跑任务 1+3 focused tests，预期全部通过、exit 0。

## 任务 5：写 Nuclei command propagation RED 测试

**文件：**
- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`

**步骤 1：** 添加纯 helper 测试：runner result 有 `command/exit_code/duration/truncation/original_bytes` 时，outer facade result 获得 exact nested `runner_execution`，并保留 facade `exit_code`。

**步骤 2：** 添加 malformed/empty runner fields 测试，断言不插入 execution、不从 runner args 或 report 重建；添加 whitelist 测试，断言 stdout/stderr/input_file/error/未知字段不会进入 facade result。

**步骤 3：** Cargo 前运行空间守卫，再跑：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(nuclei_runner_execution_copies_exact_runner_command_and_process_status) | test(nuclei_runner_execution_never_reconstructs_missing_command) | test(nuclei_runner_execution_excludes_raw_streams_and_unlisted_fields)' --status-level fail
```

预期：helper 尚不存在或断言失败，exit 非零；记录 RED 证据。

## 任务 6：实现 Nuclei active command annotation

**文件：**
- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`

**步骤 1：** 新增纯 `runner_execution_summary(runner)` 与 `attach_runner_execution(outer, runner)`：command 必须非空；只复制 exact integer process metadata 与 exact boolean truncation flags，不复制 raw streams。

**步骤 2：** 只在 active runner 成功返回且 `land_result` 完成之后调用 helper；template proof、preflight、guard failure路径不调用。

**步骤 3：** 重跑任务 5 selector，随后运行与 Nuclei parser/runner shape 相关的 focused tests：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(nuclei_runner_execution_) | test(runner_error_and_truncation_parse_as_nonterminal) | test(network_attempted_requires_a_successful_guarded_launch_value)' --status-level fail
```

预期：全部通过、exit 0。

## 任务 7：定向静态验证与文档收尾

**文件：**
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤 1：** 运行 frontend focused tests、affected Biome 与 typecheck：

```bash
pnpm exec vitest run frontend/components/Engagement/toolActivityPresentation.test.ts frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
pnpm exec biome check frontend/components/Engagement/toolActivityPresentation.ts frontend/components/Engagement/toolActivityPresentation.test.ts frontend/components/Engagement/ToolActivityDisclosure.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
pnpm typecheck
```

预期：全部 exit 0。

**步骤 2：** Cargo 前运行 `just space-guard`，随后运行 affected Rust test、Clippy 与 rustfmt：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(nuclei_runner_execution_) | test(runner_error_and_truncation_parse_as_nonterminal) | test(network_attempted_requires_a_successful_guarded_launch_value)' --status-level fail
just space-guard
cd backend && cargo clippy -p golish-pentest-app --lib -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs
```

预期：全部 exit 0、Clippy 零 warning。

**步骤 3：** 更新两张模块卡与 INDEX 状态，记录 truth 边界和测试入口；在 progress 记录命令、退出码、测试数与未运行的大型门禁。

**步骤 4：** 运行：

```bash
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) == 1' feature_list.json
git diff --check -- frontend/components/Engagement/toolActivityPresentation.ts frontend/components/Engagement/toolActivityPresentation.test.ts frontend/components/Engagement/ToolActivityDisclosure.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs docs/design/2026-08-11-codex-style-tool-activity-disclosure.md docs/design/2026-08-11-tool-execution-detail-variants.md docs/superpowers/plans/2026-08-11-tool-execution-detail-variants.md docs/modules/frontend/components.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/INDEX.md feature_list.json agent-progress.md
```

预期：全部 exit 0。只有 fresh focused evidence 全绿后才把本 feature 标为 `passing`；不自动 stage、commit 或 push共享 dirty tree。
