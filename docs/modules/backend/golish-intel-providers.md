# golish-intel-providers

> **一句话职责**：ASM/威胁情报 provider 抽象层——统一的 `IntelProvider` trait 接入 0.zone / FOFA / 360 Quake / Hunter / Shodan 等攻击面/情报平台。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-intel-providers/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 ASM 情报 provider（FOFA/Quake/Hunter/Shodan/0.zone）、查询类型、结果映射时
- recon 阶段被动情报源相关时

## 职责

为各 ASM/威胁情报平台提供统一异步接口，结果归一成 provider-agnostic 字段表。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `IntelProvider`(async trait) | 每个平台实现 |
| `ProviderRecord` | 归一结果格式 |
| `ProviderMeta` | 静态元数据（id/名称/注册 url） |
| `QueryType` | 查询类别（site/domain/email…） |
| `api_key_integration_schema` / `ConnectionStatus` | 集成 schema/状态 |
| `IntelError` / `IntelResult` | 错误 |

## 依赖

- **内部**：`golish-integrations`（凭据）

## 被谁依赖 / 改动影响面

`golish`、`golish-recon-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `zone/` | 0.zone 完整实现（7 QueryType，参考实现） | [→](golish-intel-providers/zone.md) |
| `fofa/` | FOFA（鹰图，`email\|key`） | [→](golish-intel-providers/fofa.md) |
| `hunter/` | 奇安信 Hunter（URL-safe base64） | [→](golish-intel-providers/hunter.md) |
| `quake/` | 360 Quake（`X-QuakeToken`） | [→](golish-intel-providers/quake.md) |
| `shodan/` | Shodan（key query + DSL 重写） | [→](golish-intel-providers/shodan.md) |
| `shared/` | provider 间共享（KeyStore/RateLimiter/http） | [→](golish-intel-providers/shared.md) |

## 关键文件

`types.rs`、`error.rs`。

## 注意事项 / 坑

- 加 provider 四步：建 `src/<name>/{mod,client,types,mapper}.rs` → 实现 `IntelProvider` → `pub mod <name>` → 在消费 crate（recon-app facade）注册。
- 完整设计：`docs/design/2026-05-20-asm-intel-providers.md`、`docs/superpowers/plans/2026-05-23-asset-intel-providers-flat.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers
```
