# golish-integrations

> **一句话职责**：schema 驱动的外部服务**凭据管理**——用 JSON/Rust schema 描述每个外部服务（FOFA/Quake/Hunter/Shodan/0.zone/ENScan/GitHub）要哪些字段、存哪、怎么测，前端按 schema 渲染通用表单。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-integrations/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改外部服务凭据（API key / cookie / token）、凭据存储后端、连通性测试时
- 区分「集成凭据」与 pentest「采集到的凭据」时

## 职责

统一管理 Golish **访问外部服务**所需的凭据。每个集成由 `IntegrationSchema` 描述：① 用户要填的字段；② 存储位置（`Vault` / `ExternalFile` / `Settings`）；③ 如何测试（`TestKind`）。后端通过统一 `StorageBackend` trait 写入。新集成是纯 config 改动。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `IntegrationSchema` / `Storage` / `TestKind` | schema/存储位置/测试种类 |
| `StorageBackend`(trait)（`traits`/`storage`） | 统一存储后端 |
| `resolver` / `tester` | 解析 / 连通测试 |
| `IntegrationError` / `IntegrationResult` | 错误 |

## 依赖

- **内部**：`golish-core`、`golish-db`、`golish-settings`

## 被谁依赖 / 改动影响面

`golish`、`golish-intel-providers`、`golish-recon-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `schema/` | 集成 schema 定义 | [→](golish-integrations/schema.md) |
| `storage/` | Vault/ExternalFile/Settings 存储后端 | [→](golish-integrations/storage.md) |

## 关键文件

`resolver.rs`、`tester.rs`、`traits.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- **命名陷阱**：这里叫 "Integrations" 不是 "Credentials"——`golish-pentest` 的 `target.credentials` 命名空间专指 pentest **采集到**的凭据（账号/泄露密码），方向相反，别混。
- 凭据存储涉及安全（Vault），改存储后端注意 I3（后端独立安全校验）。
- 相关：`docs/superpowers/plans/2026-05-21-integrations.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-integrations
```
