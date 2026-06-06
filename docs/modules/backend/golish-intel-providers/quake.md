# golish-intel-providers / quake

> **一句话职责**：360 Quake provider——`IntelProvider`，`X-QuakeToken` header 认证（raw token），支持 Site/Domain/Cert（Quake DSL），月度积分配额 + 防御性 2 req/s。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/quake/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Quake 查询/映射、`X-QuakeToken` 认证、Quake DSL 时

## 职责

`IntelProvider` for 360 Quake（API base `https://quake.360.net/api/v3/`）。HTTP header `X-QuakeToken: <key>` 认证（vault 存原始 token，无需编码）。支持 Site（`service:`/`port:`…）、Domain（`domain:"..."`）、Cert（`cert:"..."`）。月度积分配额（免费默认 3000），额外 pace 2 req/s。

## 公开接口

| 符号 | 说明 |
|---|---|
| `QuakeProvider`（impl `IntelProvider`） | Quake provider |
| 支持 QueryType | Site / Domain / Cert（Quake DSL） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntelProvider` impl |
| `client.rs` | HTTP + `X-QuakeToken` header |
| `mapper.rs` / `types.rs` | 响应映射 / wire 类型 |

## 依赖

- crate 内 `shared`、`error`、`types`、`IntelProvider`、`async-trait`

## 注意事项 / 坑

- token 走 `X-QuakeToken` header，原始存（无编码）；别误用 query string。
- 月度积分配额（非 per-second），但仍 pace 2 req/s 防批量过猛。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers quake
```
