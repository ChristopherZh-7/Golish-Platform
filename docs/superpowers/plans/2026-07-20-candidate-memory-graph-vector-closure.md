# Candidate、记忆、知识图谱与向量闭环实现计划

1. [x] 为 Candidate no-purge replacement exact lineage 写 focused RED，修复 manifest authority 查询并保留冲突/缺失 fail-closed 回归。
2. [x] 为 bound Worker ContextPack 写 focused RED，将 trusted execution/Unit/Worker identity显式传入检索 helper，验证无跨 Unit 泄漏。
3. [x] 为 Target Intel episode lineage写 focused 回归：正常事实沿 sealed handoff传播；合法空结果由 final-seal transaction 生成唯一 server attestation，Assertion → Document → Temporal Graph 继续坚持正 evidence。
4. [x] 增加显式 loopback-only Ollama embedding 配置；客户端禁代理/重定向、请求 1536 维，并将同一 provider接到 projector 与 VectorPrior。
5. [x] 增加 provider-unconfigured delivery 的严格 CAS backfill；验证只重开允许的存量文档。
6. [x] 每次 Cargo 前运行 `just space-guard`；运行受影响 crate/test 的 focused nextest、scoped clippy 和 rustfmt。
7. [x] 将本地设置指向 `http://127.0.0.1:11434/v1` 和 `qwen3-embedding:4b`，完成本地模型维度、DB projector 和 VectorPrior 健康检查。
8. [x] 重建 `golish` CLI，在 `golish_gatefix_20260720_d` 新建 Candidate fork并 Gate PASS；使用单 run log + clone DB复核 Runtime Memory、ContextPack、Assertion、Document、Temporal Graph、Vector。
9. [x] 大 Candidate manifest 使用 server-expanded 分组输入；服务端只在 exact frozen manifest 内展开 canonical prefix并补回 frozen evidence，未知/重复/混用继续 fail closed。
10. [x] 更新模块卡、INDEX、feature_list 和 agent-progress，记录完整可重放证据与剩余风险。
