# golish-mcp / loader

> **一句话职责**：MCP 可执行配置加载 + 信任处理——只合并 ready builtin、user-global 与已信任 project server，并管理 `TrustedMcpConfigs`（已信任路径集，存 `trusted-mcp-configs.json`）。

- **类型**：目录模块（属于 crate [`golish-mcp`](../golish-mcp.md)）
- **路径**：`backend/crates/golish-mcp/src/loader/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 MCP 配置加载（user/project/内置 server 合并）、信任路径管理时
- 加内置 MCP server 或改信任文件（`trusted-mcp-configs.json`）时

## 职责

加载并合并 MCP server 的**可执行配置**：运行产物完整的内置 server + user-global 配置 + 已信任 project 配置；管理 `TrustedMcpConfigs`（已信任项目路径集）。未信任 project 文件在解析前就被排除，不能进入 `McpManager`。项目覆盖 user、user 覆盖 builtin。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TrustedMcpConfigs`（`trusted_paths`） | 已信任配置路径集 |
| `builtin_server_names` / `builtin_configs` | 当前运行产物完整、可执行的内置 MCP server |
| `builtin_setup_directory` | 按固定 registry 解析 canonical builtin 工具目录；不接受 merged override path |
| （配置加载/合并函数） | user/project/内置合并 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 配置加载 + 信任 + 内置 server |
| `tests.rs` | 单测 |

## 依赖

- crate 内 `config`（`McpConfigFile`/`McpServerConfig`）、`serde`、`anyhow`；常量 `trusted-mcp-configs.json`

## 注意事项 / 坑

- 内置 server **不可被用户删**，但同名 user/project 配置可覆盖——真实优先级是 trusted project > user > builtin。
- 信任是安全边界：未信任 project 配置在读取/解析前就跳过，不能只设 `enabled=false`（手动 connect 不以 enabled 作为安全门禁）。
- builtin 路径不能从 `QBIT_WORKSPACE` 或运行时 cwd 解析；这两个位置属于用户打开的项目输入。开发构建只用 compile-time repository root，发布构建只用 executable/resource 相对位置。
- `js-reverse` 只有 entry 与对应的 generated DevTools runtime entry 同时存在时才注册；源码目录存在不代表服务可运行。
- `builtin_setup_directory` 使用独立 canonical registry，绝不能从 user/project override 的 args 推导并执行 npm。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-mcp loader
```
