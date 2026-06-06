# golish-intel-providers / zone

> **一句话职责**：0.zone（零零信安）provider 完整实现——`IntelProvider` impl 覆盖 7 个 QueryType（Site/Domain/Email/Apk/Code/Member/Org），限速 2 req/s，是其它 provider 的参考实现。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/zone/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 0.zone 的查询/映射/限速，或加 QueryType 时
- 写新 provider 时参考其 client/mapper/types 三段结构

## 职责

`IntelProvider` for 0.zone（API `https://0.zone/api/data/`）。支持 7 个 QueryType；免费层 250 q/day · 2 req/s，由单个 `RateLimiter` 强制 2 req/s 上限。结构：`client`（HTTP）/ `mapper`（响应→`ProviderRecord`）/ `types`（wire 类型）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ZoneProvider`（impl `IntelProvider`） | 0.zone provider |
| 支持 QueryType | Site / Domain / Email / Apk / Code / Member / Org |
| `types::*`（test）：`SiteEntry`/`DomainEntry`/`EmailEntry`/`ApkEntry`/`CodeEntry`/`MemberEntry` | wire 条目类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntelProvider` impl |
| `client.rs` | HTTP 调用 |
| `mapper.rs` | 响应 → `ProviderRecord`（含 group 反查归属） |
| `types.rs` | wire 类型 |

## 依赖

- crate 内 `shared::RateLimiter`、`error`、`types`（`QueryType`/`ProviderRecord`/…）、`IntelProvider` trait、`async-trait`

## 注意事项 / 坑

- **限速 2 req/s 必须保**：免费层封顶；`mapper` 的 group 反查归属是 0.zone 特性。
- 是 provider **参考实现**（最完整）；写其它家照其三段结构。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers zone
```
