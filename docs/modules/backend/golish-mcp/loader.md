# golish-mcp / loader

> **一句话职责**：MCP 配置加载 + 信任处理——读 user/project MCP server 配置、内置 server（js-reverse-mcp 等）、`TrustedMcpConfigs`（已信任路径集，存 `trusted-mcp-configs.json`）。

- **类型**：目录模块（属于 crate [`golish-mcp`](../golish-mcp.md)）
- **路径**：`backend/crates/golish-mcp/src/loader/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 MCP 配置加载（user/project/内置 server 合并）、信任路径管理时
- 加内置 MCP server 或改信任文件（`trusted-mcp-configs.json`）时

## 职责

加载并合并 MCP server 配置：内置 server（随 Golish 发布、不可删，用户/项目同名可覆盖）+ user/project 配置；管理 `TrustedMcpConfigs`（已信任配置路径集），决定哪些外部 MCP 配置可加载。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TrustedMcpConfigs`（`trusted_paths`） | 已信任配置路径集 |
| `builtin_server_names` / `builtin_configs` | 内置 MCP server（如 js-reverse-mcp） |
| （配置加载/合并函数） | user/project/内置合并 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 配置加载 + 信任 + 内置 server |
| `tests.rs` | 单测 |

## 依赖

- crate 内 `config`（`McpConfigFile`/`McpServerConfig`）、`serde`、`anyhow`；常量 `trusted-mcp-configs.json`

## 注意事项 / 坑

- 内置 server **不可被用户删**，但同名 user/project 配置可覆盖——合并顺序别搞反。
- 信任是安全边界：未信任路径的 MCP 配置不应被加载/执行（防恶意 MCP 配置）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-mcp loader
```
