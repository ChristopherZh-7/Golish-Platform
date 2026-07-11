# MCP 项目信任与内置来源收口实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 GUI/CLI 只执行已信任的项目 MCP 配置，并确保 builtin 展示、来源和 setup 都来自可验证的 canonical registry。
**架构：** 把信任判断收口到共享的 `load_mcp_config` 边界；把 UI 来源判定与 builtin setup 分别改为真实 merge precedence 和 canonical registry。缺少生成运行时的 builtin 在 loader 层 fail closed。
**技术栈：** Rust 2021、Tauri 2、rmcp、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish-mcp/src/loader/mod.rs`：可执行配置的信任门禁、builtin readiness、canonical setup directory。
- 修改 `backend/crates/golish-mcp/src/loader/tests.rs`：loader/builtin 的红绿回归。
- 修改 `backend/crates/golish-mcp/src/lib.rs`：导出 canonical setup directory API。
- 修改 `backend/crates/golish/src/mcp/commands.rs`：来源 precedence 与 setup provenance。
- 修改 `docs/modules/backend/golish-mcp.md`、`docs/modules/backend/golish-mcp/loader.md`、`docs/modules/backend/golish/mcp.md`：同步模块契约。
- 修改 `docs/modules/INDEX.md`：保持相关模块卡状态为当前。

## Task 1：锁住未信任项目配置不能执行

**文件：**
- 测试：`backend/crates/golish-mcp/src/loader/tests.rs`
- 修改：`backend/crates/golish-mcp/src/loader/mod.rs`

**步骤 1：写失败测试**

新增测试，创建包含 `malicious-project` stdio server 的临时
`.golish/mcp.json`，调用公开 `load_mcp_config(temp.path())` 并断言结果不含该
server。再用 inner seam 覆盖：false 跳过 malformed project JSON、false 仍加载
user config、true 允许 project 覆盖 user。

**步骤 2：验证红灯**

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-mcp test_load_mcp_config_does_not_activate_untrusted_project_servers --status-level fail
```

预期：旧实现返回 `malicious-project`，断言失败。

**步骤 3：最小实现**

把 inner 签名改成：

```rust
fn load_mcp_config_inner(
    user_config: Option<PathBuf>,
    project_dir: &Path,
    project_config_trusted: bool,
) -> Result<McpConfigFile>
```

公开入口传 `is_project_config_trusted(project_dir)`，并仅在
`project_config_trusted` 时读取项目文件。现有需要验证项目 merge 的测试显式传
`true`，空/未信任测试传 `false`。

**步骤 4：验证绿灯**

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-mcp loader --status-level fail
```

预期：全部 loader 测试通过。

**提交：** 不提交；本工作区按用户要求保留未提交状态。

## Task 2：让 builtin readiness fail closed

**文件：**
- 测试：`backend/crates/golish-mcp/src/loader/tests.rs`
- 修改：`backend/crates/golish-mcp/src/loader/mod.rs`

**步骤 1：写失败测试并验证**

对 source 与 build 两种布局分别创建 entry；生成 DevTools runtime 不存在时断言
false，创建对应 `node_modules/chrome-devtools-frontend/mcp/mcp.js` 后断言 true。

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-mcp generated_devtools --status-level fail
```

预期：旧 loader 没有必要条件 helper，测试编译失败或行为断言失败。

**步骤 2：最小实现**

active config 按 `build/src/index.js`、`src/index.js` 顺序解析，并用
`js_reverse_entry_point_has_generated_devtools_runtime` 检查入口与对应 runtime
entry 都是文件。

**步骤 3：验证绿灯**

重复上面的 nextest 命令，预期 source/build 测试全部通过。

**提交：** 不提交；本工作区按用户要求保留未提交状态。

## Task 3：固定 source precedence 与 setup provenance

**文件：**
- 测试/修改：`backend/crates/golish/src/mcp/commands.rs`
- 修改：`backend/crates/golish-mcp/src/loader/mod.rs`
- 修改：`backend/crates/golish-mcp/src/lib.rs`

**步骤 1：写失败测试**

提取纯函数 `classify_mcp_server_source`，测试同名 key 同时存在时返回 project，只有
user+builtin 时返回 user，只有 builtin 时返回 builtin。为
`builtin_setup_directory` 测试未知 server 返回 None，保证 caller 不能传 override
path。另创建仅存在于临时 `QBIT_WORKSPACE/tools/` 下的伪 builtin，先证明旧 resolver
会命中，再要求修复后返回 None。

**步骤 2：验证红灯**

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish classify_mcp_server_source --status-level fail
```

预期：helper 尚不存在，测试不能通过。

**步骤 3：最小实现**

`mcp_list_servers` 只在项目受信时读取 project key set，并按 project、user、builtin
顺序分类。`mcp_setup_builtin` 删除 `load_mcp_config` 与 entry args 推导，改为：

```rust
let tool_dir = golish_mcp::builtin_setup_directory(&server_name)
    .ok_or_else(|| format!("Unknown built-in MCP server '{}'", server_name))?;
```

registry 只把固定 `js-reverse` 映射到 canonical package manifest 所在目录。
resolver 删除 `QBIT_WORKSPACE`/cwd candidate，增加 compile-time repository root 与
executable/resource 相对 candidate。

**步骤 4：验证绿灯**

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-mcp --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish classify_mcp_server_source --status-level fail
```

预期：两组测试全绿。

**提交：** 不提交；本工作区按用户要求保留未提交状态。

## Task 4：同步文档并做完整验证

**文件：**
- 修改：`docs/modules/backend/golish-mcp.md`
- 修改：`docs/modules/backend/golish-mcp/loader.md`
- 修改：`docs/modules/backend/golish/mcp.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤 1：同步契约**

文档明确：项目 config 只有受信后才进入 executable merge；不可运行 builtin 不注册；
setup 只用 canonical registry；source precedence 是 trusted project > user > builtin。

**步骤 2：crate 验证**

```bash
cd backend
cargo fmt -p golish-mcp -p golish -- --check
CARGO_INCREMENTAL=0 cargo nextest run -p golish-mcp --status-level fail
CARGO_INCREMENTAL=0 cargo clippy -p golish-mcp -p golish --all-targets -- -D warnings
```

预期：exit 0，零 warning。

**步骤 3：启动验证**

```bash
just dev /Users/christopherzheng/golish-platform/Test1
```

预期：最新二进制、Vite、嵌入式 Postgres 正常；本次启动日志没有
`ERR_MODULE_NOT_FOUND` 或 js-reverse connect failure。验证后 Ctrl-C 停止并确认
1420 无监听。

**步骤 4：仓库门禁**

```bash
CARGO_INCREMENTAL=0 ./init.sh
CARGO_INCREMENTAL=0 just precommit
git diff --check
```

预期：全部 exit 0。把命令、退出码和关键证据写进 progress/feature evidence。

**提交：** 不提交；明确记录未 stage、未 commit、未 push。
