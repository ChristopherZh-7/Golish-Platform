# golish-intel-providers / hunter

> **一句话职责**：奇安信 Hunter provider——`IntelProvider`，单 `api-key` 认证，`search` 参数需 **URL-safe base64**（非标准 base64），主打 Site 维度（含 organization_name 字段）。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/hunter/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Hunter 查询/映射、URL-safe base64 编码（`client::encode_query`）时
- Hunter 查询解码失败/编码格式问题时

## 职责

`IntelProvider` for 奇安信 Hunter（API `https://hunter.qianxin.com/openApi/search`）。单 `api-key` token 认证（无 email）。`search` 查询参数必须是 **URL-safe base64**（不是标准 base64），由 `client::encode_query` 处理。主打 Site 端点（host/ip/port + web metadata + company）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `HunterProvider`（impl `IntelProvider`） | Hunter provider |
| 支持 QueryType | Site（Domain/Cert 经查询 DSL 在 Site 端点表达，未单列） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntelProvider` impl |
| `client.rs` | HTTP + `encode_query`（URL-safe base64） |
| `mapper.rs` / `types.rs` | 响应映射 / wire（`HunterEnvelope`/`HunterData`/`HunterRow`/…） |

## 依赖

- crate 内 `shared`、`error`、`types`、`IntelProvider`、`async-trait`、base64（URL-safe）

## 注意事项 / 坑

- **URL-safe base64**（非标准）：用错 alphabet 会被 Hunter 拒；改编码走 `encode_query`。
- 目前只显式 surface Site；Domain/Cert 用 Hunter 查询 DSL 在 Site 内表达。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers hunter
```
