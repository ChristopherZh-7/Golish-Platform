# golish-vuln-intel-domain

> **一句话职责**：漏洞情报领域层——纯类型与 I/O 边界 trait，**无任何 I/O 依赖**（reqwest/sqlx 都没有），feed 抓取与 DB 存储抽象成 trait。

- **类型**：crate（Layer 1/2 领域核心）
- **路径**：`backend/crates/golish-vuln-intel-domain/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改漏洞 feed 类型（`VulnFeed`/`VulnEntry`）、默认 feed 列表、feed 抓取/存储的 port trait 时

## 职责

承载漏洞情报的纯领域：feed/entry 类型、默认 feed 与 NVD URL helper、feed 抓取与 DB 存储的 trait（由 `golish-vuln-intel` adapter 实现）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `VulnFeed` / `VulnEntry` | feed/条目类型 |
| `default_feeds()` / `nvd_recent_url()` | 默认 feed / NVD URL |
| `traits` | feed 抓取 / 存储的 port trait |

## 依赖

- **内部**：无（零 I/O）

## 被谁依赖 / 改动影响面

`golish-vuln-intel`。

## 关键文件（无目录子模块）

`types.rs`、`traits.rs`。

## 注意事项 / 坑

- 与 `golish-pentest-domain` 同理：领域层不引 I/O，新增外部交互走 trait 由 adapter 实现。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-vuln-intel-domain
```
