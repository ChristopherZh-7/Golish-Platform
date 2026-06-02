# Engine v2 · P3 知识+持续（RAG 先验 + 知识图）实现计划

> 必需子技能：superpowers:executing-plans。commit/push 按 AGENTS.md §2.7。

**目标（设计 §5 P3）：** 测漏洞前自动检索历史 writeup/PoC 并入交付（RAG 先验）；知识图深化（借 PentAGI Graphiti）。验收：「测漏洞前自动检索 writeup 并入交付」。

**架构（留-搓-借）：** **留** 现有 RAG/KG 底座——`DbRepoProvider.{wiki_search_fts, wiki_search_by_tag, wiki_list_cves_with_pocs, vuln_intel_search}`（wiki KB + vuln intel 检索）+ `GraphKnowledgeBase` trait（`tool_executors/graph_trait.rs`）+ `golish-graphiti` crate。**借** PentAGI（`vxcontrol/pentagi`，已 clone：Graphiti 知识图）。**搓** vuln_triage/verification 前的 RAG 先验检索 + 注入。

**现状（本会话亲核）：** RAG 检索方法已在 DbRepoProvider；KG 有 GraphKnowledgeBase trait + golish-graphiti；但**没有**「stage 前置自动检索 prior writeup 注入 prompt」这条线。

---

## 增量（建议 P3-a 先做）

### P3-a · RAG 先验检索 + 注入（concrete，可测）
**新增** `harness/rag_prior.rs`：
- `PriorWriteup { source, title, snippet }` + `PriorKnowledge { writeups: Vec<PriorWriteup> }`。
- `async fn retrieve_prior_knowledge(repo: &dyn DbRepoProvider, query: &str, limit: i64) -> PriorKnowledge`：调 `wiki_search_fts(query)`（+ query 像 CVE 时调 `vuln_intel_search`），把返回 JSON 解析成 writeups。
- `fn render_prior_knowledge(&PriorKnowledge) -> String`：markdown 段（仿 `render_inherited_handoff`），供 vuln_triage/verification stage prompt 注入「## PRIOR KNOWLEDGE（已检索 writeup）」。
**接线（follow-up 或本增量末）**：stage prompt 生成（prompts/mod.rs stage_charter）在 vuln_triage/verification 前拼接 render_prior_knowledge（需 repo + target query 穿到 prompt 生成点；先做模块 + 纯 render 可测，注入接线随后）。
**测试**：mock DbRepoProvider 返回固定 wiki_search Value → retrieve 解析出 writeups；render 含标题/来源。
**验证**：`cargo nextest -p golish-agent-kit -E 'test(rag_prior)'`。

### P3-b · 知识图深化（借 PentAGI Graphiti，larger）
扩 `GraphKnowledgeBase` / golish-graphiti：把 finding/evidence/target 关系入图（entity + relation），供跨 operation 检索先验。**确认点**：读 golish-graphiti 现有 API + PentAGI Graphiti 用法（已 clone /tmp/refs/pentagi）。**较大、独立**，建议单独做。

### P3-c · 持续运行（continuous）
operation 完成后把交付物摘要/finding 回灌 KB（wiki upsert）供下次先验。小增量，依赖 P3-a/b。

---

## 类型/依赖一致
- P3-a 复用现有 DbRepoProvider RAG 方法 + 纯 render（仿 P2 eval/guardrail 的可测 SDK 模式）；零新依赖。
- P3-b 动 golish-graphiti（已存在）+ GraphKnowledgeBase；借 PentAGI 设计不引依赖。

## 自检（writing-plans）
- 覆盖设计 §5 P3（RAG 先验 + 知识图 + 持续）。P3-a 是独立可测最小增量（检索+render），建议先做；b/c 叠加。
- 活体注入（prompt 真拼）与 KG 深化标为接线/larger follow-up，与 P2 eval/guardrail 的「先 SDK 后 live」一致。
