# golish-memory-app

> **一句话职责**：Memory Fabric 应用服务——通过 ports 编排 canonical transaction/outbox、确定性 Assertion/Document/Embedding 投影与 scoped ContextPack retrieval；授权能力保持 opaque，不拥有数据库/provider/runtime 生命周期实现。

- **类型**：crate（Layer 3 · 应用服务）
- **路径**：`backend/crates/golish-memory-app/`
- **状态**：✅ C1/C2/C3 已实现（atomic adapters + process-global live supervisor + deterministic projectors）

---

## 何时该读这张卡（给 AI 的触发提示）

- 修改 promotion / invalidation / Document Projector 时
- 新增 projector、delivery dependency 或 supervisor port 时
- 接入 canonical producer 的 atomic terminal + outbox seam 时

## 职责

- 通过 `KnowledgeUnitOfWork` 把 canonical terminal write 与 immutable outbox/delivery 写入表达为一个 transaction port；production adapter 已接入，P1 四信息阶段 final-seal、Candidate terminal、Post-Exploit fact/action 与 Cleanup terminal 均在各自 compound transaction 内写 deterministic catalog event，禁止事后补写。
- 实现 `assertion-promoter@1 → document-projector@1 → embedding-projector@1` 的服务器端 delivery dependency；`succeeded_suppressed` 与 `succeeded` 都满足后继 dependency。
- `assertion-promoter@1` 对 `StageEpisodeClosed.v1` 与 `PostExploitActionPrepared.v1` 从已持久化 canonical row/evidence 幂等派生 Assertion；对 `CandidateAttemptTerminal.v1`、`PostExploitFactTerminal.v1`（foothold/objective）与 `CleanupObligationTerminal.v1` 从 immutable canonical event envelope/source + strict structured payload 派生 scoped Assertion。operation/project/org/source authority 一律取 envelope 与 exact sealed frozen scope，payload 同名字段不能覆盖。Candidate 的唯一 intentional suppression 是 canonical `blocked + blocker_reason_code + 0 audit evidence + 0 FactDelta`，且 suppression 前也必须通过与真实 projection 完全相同的 sealed authority 校验；reason 被保留但绝不冒充 evidence，其他缺 evidence 状态 fail closed。Candidate terminal event 不推断 FactDelta evidence 归属；FactDelta 只在 consolidation 接受后由独立 typed event 提升。
- `FactDeltaAccepted.v1` 由 Wave consolidation producer 在同一事务内写入 evidence-backed Assertion、catalog event 与四条 delivery；projector 只接受该 producer 已持久化的 Assertion，裸 event 或缺失 lineage 时 fail closed，不能 `succeeded_suppressed` 冒充支持。`SourceScopeInvalidated.v1` 则只消费 producer 已关闭的 Assertion；`ReportRevisionFinalized.v1` intentional empty-route。
- Document Projector 只读取 promoted structured assertions/episodes，使用稳定 key 与内容排序生成 deterministic document；不读取模型 prose、tool stdout 或 transcript。
- 失效操作同步关闭 Assertion/Document/Embedding 的有效期，不物理删除历史。
- C3 Graph projector 只把有效 typed Assertion 映射到 local temporal graph port；identity 与 lineage 分离，source invalidation 只关闭对应 Assertion lineage。
- C3 rebuild 从 Assertion 历史写新 generation，验证 hash/count 后才切 active；绝不从 legacy/V2 graph 倒推 canonical truth。
- C7 先从 DB ownership snapshot + server-owned principal/data policy 构造字段私有、无公开 constructor/Deserialize 的 `TrustedAuthorizationContext`，再按 stage/request/classification 取交集；scope/classification 必须先于 canonical/runtime/handoff/assertion/document/graph/vector 查询。
- C7 retrieval 固定 `canonical → runtime → handoff → episode → assertion → document → temporal graph → vector` 顺序；mandatory canonical/runtime 超 token cap 时 fail closed，optional layer 才可稳定截断。Graph/vector 可显式 degrade，但禁止回退 legacy global memories/wiki。
- C7 embedding projector 只在 exact predecessor document delivery=`succeeded` 后调用 provider，并强制 provider/result 均为 1536 维；未批准外部 embedding 或 restricted classification 均 terminal suppress。`QueryEmbeddingProvider` 明确声明是否需要数据外发：loopback-only provider在默认 customer-local policy下可用于 VectorPrior，外部 provider仍必须有显式策略授权；维度/数值/调用失败只降级 optional vector layer，不影响 mandatory canonical/runtime。

本 crate 不直接依赖 sqlx 或 embedding provider；local temporal graph 通过 port/typed client 注入，外部 Graphiti 不在默认实现。`KnowledgeProjectorSupervisor` 是唯一通用 worker owner：并发 `start` 幂等、panic 后保留 DB lease 等待重领、shutdown 等当前 batch。projector 是可重建 read model，**不是 Gate authority**，也不随 AI session 启停。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `KnowledgeUnitOfWork` | canonical terminal + outbox 的单事务 port |
| `DocumentProjectionPort` | load promoted assertions + upsert deterministic document |
| `DocumentProjector` | 结构化 assertion → deterministic document |
| `PromotionService` / `InvalidationService` | typed policy orchestration |
| `KnowledgeProjectorSupervisorPort` | C2 process-global supervisor 的预留边界 |
| `KnowledgeProjectorSupervisor` / `KnowledgeProjectorWorker` | 唯一 process owner、取消/join、bounded retry loop 与 per-projector worker port |
| `project_assertion` / `GraphProjection` | assertion→closed entity/relation projection，属性显式 allowlist |
| `GraphProjector::run_once` / `rebuild_graph_scope_from_assertions` | graph-only delivery replay 与 latest-per-stream generation rebuild 编排 |
| `TrustedAuthorizationContext` / `EffectiveContextQuery` | DB ownership 与 server policy 交集后的 opaque authorization capability；loader 保持 crate-private |
| `KnowledgeRetriever` / `ContextPackProvider` | exact-scope 分层检索、稳定排序、预算与 degrade 合同 |
| `EmbeddingProjector` / `EmbeddingProvider` | predecessor-gated 1536 维 projection port |

## 依赖

- **内部**：`golish-memory-domain`、`golish-graphiti`（只消费 temporal DTO/client；不调用外部服务）
- **外部**：`async-trait`、`serde_json`、`uuid`、`chrono`、`sha2`、`thiserror`

## 被谁依赖 / 改动影响面

C2 的 `golish-agent-app` DB adapter/runtime lifecycle；后续 KG/RAG projector 与 Post-Exploit/Cleanup/Reporting canonical producers。

## 关键文件

| 文件 | 作用 |
|---|---|
| `ports.rs` | transaction、Document 与 supervisor ports |
| `promotion.rs` | deterministic promotion policy |
| `invalidation.rs` | projection chain temporal invalidation |
| `outbox.rs` | delivery state/dependency 应用契约 |
| `projectors/assertion.rs` | assertion projector seam |
| `projectors/document.rs` | deterministic Document Projector |
| `graph_projection.rs` | Assertion→entity/relation identity+lineage 纯映射 |
| `projectors/graph.rs` | graph delivery worker ports、幂等 replay 与 rebuild orchestration |
| `supervisor.rs` | process-global exactly-once lifecycle、panic isolation、取消与 graceful join |
| `context_pack.rs` / `retrieval.rs` | opaque auth context、effective query 与分层 ContextPack 组装 |
| `ranking.rs` / `redaction.rs` | 稳定排序/token 估算与 prompt-safe value rendering |
| `embedding_projector.rs` | document-delivery-gated 1536 维 embedding projection |

## 注意事项 / 坑

- producer 不能传 routes；route 由 domain event catalog 与 DB registry 共同决定。
- 所有非空 catalog route 必须在 assertion promoter 有显式 authority policy：canonical-event deriver、producer-prewritten Assertion 或 producer-preclosed invalidation三类；唯一第四种是 closed reason-only blocked Candidate suppression。新增事件不能落入通用“空 assertion → suppress”分支。
- 外部 embedding/Graph I/O 不得放进数据库 transaction；C1 不发外部 embedding 请求。
- 同一 projector 的 ack 不能吞掉其他 delivery。
- classification/policy 禁止 embedding 时必须 terminalize 为 `succeeded_suppressed` 并记录原因，不能永久 pending。
- global-sanitized graph 只允许安全 `technique_experience → technique`；unknown predicate、VaultRef、credential/token/raw payload fail closed。
- properties 只允许有界 scalar/小 scalar array；nested object、控制字符、超长 canonical/display/property payload 一律拒绝，不能借 allowlisted key 夹带材料。
- rebuild 先验全量 assertion scope/integrity，按 stream 仅保留最高 source version（同版本多 Assertion 全保留），再写单一 building generation。
- local graph delivery 与任何未来 external Graphiti delivery 必须独立 ack/retry/DLQ；C3 不发真实 Graphiti 请求。
- supervisor constructor 不启动任务；desktop/CLI composition root 只能在 DB-ready 后调用一次 `start`。per-session bridge 只 clone `KnowledgeUnitOfWork`，绝不 spawn worker。
- `TrustedAuthorizationContext` 不能暴露 public fields/constructor/Deserialize；runtime/model 构造的 subject 永远不等于授权，仍必须由 DB exact identity 重验。
- VaultRef 只可渲染 opaque UUID reference；任何 plaintext token/password/private key 或 prompt markup 都必须拒绝/转义，不能进入 ContextPack prose。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-memory-app --status-level fail
```
