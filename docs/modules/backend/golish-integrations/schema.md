# golish-integrations / schema

> **一句话职责**：一个外部服务集成的**自描述** schema——`IntegrationSchema` 描述「秘密存哪（Storage）+ 用户填哪些字段组（IntegrationGroup）+ 怎么测连通（TestKind）+ 浏览器凭据捕获（capture）」，本身不含秘密值。

- **类型**：目录模块（属于 crate [`golish-integrations`](../golish-integrations.md)）
- **路径**：`backend/crates/golish-integrations/src/schema/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改某外部服务集成的字段描述、存储变体、连通测试方式、浏览器捕获 recipe 时
- 改前端按 schema 渲染的通用表单契约时

## 职责

`IntegrationSchema` 是一个外部服务（ENScan_GO / FOFA / Quake / …）的自描述：①秘密存哪（`Storage`：external_file / vault / settings）；②用户填哪些字段组（`IntegrationGroup`）；③怎么验证凭据（`TestKind`）；④浏览器凭据捕获（`capture`）。它**不携带秘密值**——值由 `StorageBackend` 加载/持久化。可来自 tool 的 `toolsconfig` JSON 或代码构造。

## 公开接口

| 符号 | 说明 |
|---|---|
| `IntegrationSchema` | 单服务自描述 |
| `Storage`（来自 `storage`） | 持久化变体（external_file/vault/settings） |
| `IntegrationGroup` | 字段组 |
| `TestKind`（来自 `test_kind`） | 连通测试 recipe |
| capture 类型（来自 `capture`） | 浏览器凭据捕获 recipe |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntegrationSchema` + re-export |
| `storage.rs`（子模块） | `Storage` 持久化变体描述 |
| `test_kind.rs` / `capture.rs` | 连通测试 / 浏览器捕获 recipe |

## 依赖

- `serde`（JSON schema）；与 `crate::traits::StorageBackend` 配对

## 注意事项 / 坑

- schema **不存秘密值**，只描述结构——值走 `storage/` 的 `StorageBackend`。别把凭据塞进 schema。
- 这是前端通用表单的渲染契约（wire JSON）；改字段要同步前端 + tool 的 `toolsconfig` JSON。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-integrations schema
```
