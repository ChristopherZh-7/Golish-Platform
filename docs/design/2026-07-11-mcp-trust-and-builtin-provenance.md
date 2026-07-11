# MCP 项目信任与内置来源收口设计

**日期：** 2026-07-11
**状态：** Approved for implementation（P0/P2 收尾审计）

## 1. 问题

当前 `golish_mcp::load_mcp_config(workspace)` 会无条件把
`<workspace>/.golish/mcp.json` 合入可执行配置。GUI bootstrap 和 CLI agent
bootstrap 随后把该配置交给 `McpManager::connect_all`；stdio transport 最终会执行
配置中的 command。虽然仓库已经有 `is_project_config_trusted` 与
`trust_project_config`，加载链没有使用该信任结果。

这形成一个 P0 边界缺口：仅打开未信任 workspace 就可能执行项目提供的命令。
把 server 改成 `enabled=false` 也不充分，因为手工 `mcp_connect` 路径不会重新验证
项目信任。

同一审计还发现三处来源问题：

1. `mcp_list_servers` 按 `user > builtin > project` 猜来源，和真实 merge 优先级
   `project > user > builtin` 不一致；受信项目覆盖同名 builtin 时会被误标。
2. `mcp_setup_builtin` 从 merged config 取 entry path。受信 user/project 同名覆盖
   builtin 后，setup 可能在非 canonical 目录执行 `npm install` / `npm run build`。
3. builtin resolver 把 `QBIT_WORKSPACE` 与 runtime cwd 当候选根；而
   `QBIT_WORKSPACE` 正是用户打开的项目路径。恶意项目可伪造
   `tools/js-reverse-mcp`，绕过 project MCP config 的信任门禁。

另有一个 P2 真实性问题：仓库里的 `js-reverse-mcp/src/index.js` 存在，但依赖的
生成文件 `chrome-devtools-frontend/mcp/mcp.js` 不存在，启动时却仍被当成可用
builtin，产生 `ERR_MODULE_NOT_FOUND`。

## 2. 安全与行为目标

- 未信任项目配置不解析、不合并，也绝不进入 `McpManager`。
- builtin 与 user-global 配置仍可正常加载。
- 已信任项目配置保持现有最高优先级，并可覆盖 user/builtin。
- malformed 的未信任项目配置不能阻塞应用启动；malformed 的已信任配置继续报错。
- server 来源按真实优先级标记：trusted project > user > ready builtin。
- builtin setup 只接受 registry 中的 canonical 名称和 canonical 工具目录，完全不读
  merged override 的 path。
- builtin discovery 不读取 `QBIT_WORKSPACE`/runtime cwd，只使用 executable/resource
  相对位置和 compile-time repository root。
- 缺少必要生成运行时的 `js-reverse` 不进入可执行配置，不连接、不宣称可用。
- GUI 与 CLI 不各自实现门禁；二者继续共享 `load_mcp_config` 这一安全边界。

## 3. 方案

### 3.1 可执行配置门禁

`load_mcp_config_inner` 增加 `project_config_trusted: bool`。公开
`load_mcp_config` 使用 `is_project_config_trusted(project_dir)` 计算该值。inner 始终
先合并 ready builtins 与 user-global config，只有 trusted=true 才读取项目文件。

未信任内容不是“disabled config”，而是根本不进入返回值。项目配置的发现继续由
`mcp_has_project_config` 完成；信任状态继续由 `mcp_is_project_trusted` 完成。审批预览
若以后实现，应使用独立的只读、脱敏接口，而不能复用 executable merged config。

### 3.2 来源判定

`mcp_list_servers` 只为已信任项目读取 key set，然后按
`project > user > builtin` 判定 source。这样 source 与 loader 的覆盖结果一致。未信任
项目的 key 不参与分类。

### 3.3 canonical builtin setup

`golish-mcp` 暴露 `builtin_setup_directory(server_name)`。该函数使用固定 registry
把 `js-reverse` 映射到仓库/安装包内 canonical `tools/js-reverse-mcp/package.json`
所在目录；未知名字返回 None。`mcp_setup_builtin` 只使用这个目录，不再从 merged
config 或 server args 推导目录。

共享 resolver 同时移除 `QBIT_WORKSPACE` 与 runtime cwd candidate。开发模式用
`CARGO_MANIFEST_DIR` 锚定编译仓库，发布模式用 executable/Resources 相对路径；项目
workspace 不再有机会冒充 builtin 根。

### 3.4 builtin readiness

active builtin config 优先寻找 `build/src/index.js`，兼容 prepared source
`src/index.js`。无论哪种布局，都必须同时存在该布局对应的生成 DevTools runtime
entry。检查是启动必要条件，不宣称覆盖 Node/浏览器等所有运行条件。

## 4. 明确不做

- 本轮不新增项目 MCP 审批 UI；当前没有消费既有 trust wrappers 的组件。
- 本轮不自动信任 workspace，也不把 trust 绑定到配置内容 hash。
- 本轮不现场补齐不完整的 `js-reverse` vendored build/source 工具链。
- 本轮不改 MCP schema、数据库或 IPC 类型。

## 5. 验证

- loader TDD 覆盖 trusted/untrusted、malformed、user 保留、project override。
- builtin readiness 覆盖 source/build 两种目录布局。
- source precedence 使用纯函数单测。
- setup registry 单测证明未知/同名 override 不能控制目录。
- resolver 单测证明 `QBIT_WORKSPACE/tools/**` 不能被解析为 builtin。
- `cargo nextest run -p golish-mcp`、相关 `golish` MCP 测试、clippy 全绿。
- 最新二进制打开 Test1：不出现 `ERR_MODULE_NOT_FOUND`，不连接不可运行的
  `js-reverse`，主应用和数据库正常启动。

## 6. 风险与后续

信任 command 当前只写入 trust store，不热重建已初始化 manager；用户首次批准后需
重启应用才能生效。这是 fail-closed 的可用性缺口，不削弱本轮安全边界。后续可单独
设计“审批 + 原子 reload manager”，但 reload 前仍不得让未信任配置进入 manager。
