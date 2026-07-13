# golish-memory-domain

> **一句话职责**：Memory Fabric 的纯领域契约——定义 `OperationScope`、长期 Assertion 可见性、typed canonical source、Episode、事件目录、四级分类、scoped ContextPack 数据类型和固定 1536 维 embedding schema；不执行 I/O。

- **类型**：crate（Layer 1 · 纯领域基础）
- **路径**：`backend/crates/golish-memory-domain/`
- **状态**：✅ C1 core 已实现

---

## 何时该读这张卡（给 AI 的触发提示）

- 修改 Memory Fabric 的 source identity、Episode、Assertion 或事件目录时
- 新增可检索 canonical 事件或 projector route 时
- 修改 embedding schema version / dimension 时

## 职责

- `OperationScope` 描述一次 operation 的 exact project scope、organization-at-time 与 scope snapshot；它不等于跨 operation 的长期可见性。
- `AssertionVisibility` 显式区分组织长期知识与 `global_sanitized`；后者不得包含客户 canonical ref、Vault 值或可反查客户的内容。
- `CanonicalRowId::{Uuid, Int64, Text}` 覆盖 UUID、BIGSERIAL 和文本型 canonical row identity，并提供严格、可逆的规范化存储表示。
- 事件目录是 projector routing 的服务器端唯一事实源；producer 不得自选 route。
- `PostExploitFactTerminal.v1` 使用同一 mandatory projection DAG；Foothold 与 ObjectiveOutcome producer 必须在 canonical transaction 内追加该 event。
- `PostExploitActionPrepared.v1` 以 `PostExploitAction` canonical source 进入同一 mandatory projection DAG；event payload 只能携带持久化 action/obligation 的安全哈希与 evidence ids，不能携带 raw plan/secret。
- ContextPack 只有 `Public / Internal / CustomerConfidential / Restricted` 四个 classification；`VaultCredentialRef` 是 `KnowledgeValue` 的 value kind，不是第五种 classification。
- V1 embedding 契约固定为 `EMBEDDING_DIMENSION_V1 = 1536`。

本 crate 不依赖 sqlx、Graphiti、embedding provider 或 Tauri，也不执行网络/文件/数据库 I/O。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `OperationScope` / `ProjectScopeId` | 单次运行的 exact scope identity |
| `CanonicalRowId` / `SourceRef` / `StoredCanonicalRowId` | canonical row typed identity 与严格 round-trip |
| `KnowledgeAssertion` / `KnowledgeAssertionDraft` | evidence-backed 长期事实及校验 |
| `StageEpisode` | 阶段/attempt 的结构化、历史安全运行摘要 |
| `KnowledgeEventEnvelopeV1` / `event_catalog` | versioned typed event 与 mandatory delivery DAG |
| `ContextSubject` / `ContextRequest` / `ContextItem` / `KnowledgeClass` | operation/org/stage scoped retrieval 的纯数据合同；不携带授权能力 |
| `KnowledgeValue::{Text, Json, VaultRef}` | prompt-safe value kind；secret material 必须留在 VaultRef |
| `EMBEDDING_DIMENSION_V1` | V1 唯一允许的向量维度（1536） |

## 依赖

- **内部**：无
- **外部**：`serde`、`serde_json`、`uuid`、`chrono`、`sha2`、`thiserror`

## 被谁依赖 / 改动影响面

`golish-memory-app` 与 `golish-db` memory repos。后续 KG/RAG/Post-Exploit/Cleanup/Reporting 只通过这里的 typed source/event 契约接入。

## 关键文件

| 文件 | 作用 |
|---|---|
| `scope.rs` | operation scope / project scope identity |
| `source_ref.rs` | typed canonical row id 与 source reference |
| `episode.rs` | Stage Episode 模型 |
| `assertion.rs` | Assertion identity、对象、状态与校验 |
| `classification.rs` | classification / visibility 规则 |
| `context.rs` | ContextPack subject/request/item/layer/value 合同 |
| `event_catalog.rs` | V1 event catalog 与三段 delivery DAG |
| `embedding.rs` | embedding schema 常量与维度校验 |

## 注意事项 / 坑

- projector 只是可重建投影，**不是 Gate authority**；Gate truth 仍来自 canonical DB/evidence ledger。
- `CanonicalRowId` kind/value 不匹配是 corruption，禁止降级成 `Text`。
- Assertion 幂等身份包含 `subject_key + predicate + object_hash`；同一 source version 可以产生同 predicate 的不同 object。
- raw model prose、tool stdout、transcript 自由文本不是合法 projector 输入。
- classification 不是由请求端自报；运行时请求只能被 server policy、冻结 operation ownership 与 stage allowlist 进一步收窄。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-memory-domain --status-level fail
```
