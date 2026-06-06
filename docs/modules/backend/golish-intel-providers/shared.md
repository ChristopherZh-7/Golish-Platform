# golish-intel-providers / shared

> **一句话职责**：所有 provider 实现共享的基础设施——`KeyStore` trait + `EnvKeyStore`、per-provider 请求限速 `RateLimiter`、共享 reqwest client builder + JSON 解码（`http_common`）。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/shared/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 provider 的 key 获取（`KeyStore`）、限速（`RateLimiter`）、共享 HTTP/JSON 解码时
- 加新 provider 时复用这些共享件

## 职责

为所有 `IntelProvider` 实现提供共享件：`api_key`（`KeyStore` trait + `EnvKeyStore`，从环境取 key）、`rate_limit`（per-provider 请求 pacing）、`http_common`（统一 reqwest client + 简单 JSON decoder）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `KeyStore`（trait）/ `EnvKeyStore` | key 获取抽象 + 环境变量实现 |
| `RateLimiter` | per-provider 请求限速 |
| `http_common`（client builder + JSON decoder） | 共享 HTTP |

## 关键文件

| 文件 | 作用 |
|---|---|
| `api_key.rs` | `KeyStore` + `EnvKeyStore` |
| `rate_limit.rs` | `RateLimiter` |
| `http_common.rs` | reqwest client + JSON 解码 |

## 依赖

- `reqwest`、`async-trait`；上层（golish-recon-app）可注入自定义 `KeyStore`（如 `PgVaultKeyStore`）

## 注意事项 / 坑

- `KeyStore` 是 trait：生产用 vault-backed impl（在 app 层），`EnvKeyStore` 是默认/测试用。
- 各 provider 各持一个 `RateLimiter` 实例执行自家限速上限（如 zone 2 req/s）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers shared
```
