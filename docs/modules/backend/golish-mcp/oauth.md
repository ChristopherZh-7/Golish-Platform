# golish-mcp / oauth

> **一句话职责**：MCP server 认证的 OAuth 2.1 实现——完整流程含 PKCE、动态客户端注册（DCR）、metadata 发现、回调、token 持久化。

- **类型**：目录模块（属于 crate [`golish-mcp`](../golish-mcp.md)）
- **路径**：`backend/crates/golish-mcp/src/oauth/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 MCP server 的 OAuth 流程（授权码 + PKCE、DCR、discovery、回调、token 刷新/存储）时
- MCP server 认证失败、token 过期/持久化问题时

## 职责

为需要认证的 MCP server 实现 OAuth 2.1 完整流程：metadata 发现（discovery）、动态客户端注册（registration）、PKCE 授权码流（pkce + flow）、本地回调接收（callback）、token 存取（token_store）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `flow`（授权码 + PKCE 流程编排） | OAuth 主流程 |
| `discovery` / `registration` | metadata 发现 / 动态客户端注册 |
| `pkce` / `callback` | PKCE 生成校验 / 本地回调 |
| `token_store` / `types` | token 持久化 / OAuth 类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `flow.rs` | OAuth 流程编排 |
| `discovery.rs` / `registration.rs` | 发现 / DCR |
| `pkce.rs` / `callback.rs` | PKCE / 回调 |
| `token_store.rs` / `types.rs` | token 存储 / 类型 |

## 依赖

- HTTP（reqwest）、crate 内 client/transport；OAuth 2.1 规范

## 注意事项 / 坑

- **PKCE 必用**（OAuth 2.1 强制）：别退化成无 PKCE 的隐式流。
- token 持久化涉及敏感凭据，存储安全要慎；刷新逻辑别丢 refresh token。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-mcp oauth
```
