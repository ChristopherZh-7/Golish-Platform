# Gate 真实性闭合（min_invocations 接 ledger + fail-open 旁路收敛）

> **Status**: Approved（2026-06-10 用户在 BaJie 会话授权「如果重要就修复」；架构评审定级 P0）
> **Author**: BaJie MCP-agent-2
> **来源**: 2026-06-10 架构师评审发现的 H1/H2/H3 三个 gate 完整性缺口（H4/H5/H6 挂账，见 §6）
> **关联**: `2026-05-26-operation-harness-profile-dag-lab.md` §13.6.2（detector 铁律）、`2026-06-01-harness-rebuild.md`（validation-first）

---

## 0. 决策（TL;DR）

| # | 缺口 | 修复 |
|---|---|---|
| H3 | `target_intel.json` 声明了 `min_invocations` 但 `gate_rules` 没接 `named_check:min_invocations` | gate_rules 补 1 条（对齐 EAS/enumeration 写法） |
| H1 | `min_invocations_check` 只看 agent 自报 `required_checks_done`（违反 §13.6.2「detector 不读 agent 可伪造字段」） | orchestrator 外层加第四道 ledger 回查 `enforce_min_invocations_ledger`：按 chat session 查 `audit_role='evidence'` 行的真实 (kind, subject)，能力键 → 工具集合映射后计数，不足翻 BLOCK |
| H2 | gate 多条 fail-open 旁路 | ① 嵌入资源（profile/StageSpec）加载失败从「跳 gate」改 **fail-closed BLOCK**（资源是 include_str! 编译进的，失败=代码 bug，不存在合法失败）② 新增 `GOLISH_HARNESS_STRICT`（默认关）：开启时 ledger 回查的 infra 错误也翻 BLOCK ③ 「无 harness_stage → 跳 gate」**不改**——graph-flow stage 模式下 `synthesize_stage_subtask` 已无条件打标（结构上不可达），chat 自由模式跳过是预期行为 |

## 1. H1 核心：能力键 → ledger 工具映射

`min_invocations` 的 key 是**能力别名**（全部 12 个 stage spec 实测只有 3 个 key）：`dns_resolve` / `subdomain_enum_passive` / `http_probe`。
ledger 的 `detail->>'kind'` 是**真实工具名**（pentest_run → 内层工具；run_pty_cmd/background_command → wrapper 名，真实工具在 subject 命令串里）。

映射（新纯模块 `harness/capability_match.rs`，复用 `tool_taxonomy`）：

- 行解析：kind ∈ {run_pty_cmd, run_command, background_command} → 从 subject 首 token 解析底层工具（`underlying_tool_name`）；否则 kind 即工具。
- `dns_resolve` ⇐ 工具类目 `recon/dns`（dig/dnsx/nslookup/host/dnsrecon）或同名工具
- `subdomain_enum_passive` ⇐ 类目 `recon/subdomain`（subfinder/amass/…）**或** `recon_enrich_assets` / `recon_discover_subsidiaries`（被动情报 provider 路径——2026-06-07 活体 run 用的就是它，必须收编否则回归）
- `http_probe` ⇐ 类目 `recon/http`（httpx/whatweb/curl/…）或同名工具
- 未知能力键 ⇐ 仅同名工具精确匹配（fail-closed）；加守卫测试「全部嵌入 spec 的 min_invocations key 必须被 capability_match 认识」，新 spec key 不配映射就红。

## 2. H1 数据通路

- `DbRepoProvider::evidence_tool_rows_for_session(session_id, limit) -> Result<Option<Vec<(kind, subject)>>>`；默认 `Ok(None)`=「provider 不支持」（test doubles 跳检，**区别于** Err=infra 错误）。
- golish-db `repo::audit::evidence_tool_rows_for_session`：`SELECT detail->>'kind', detail->>'subject' WHERE audit_role='evidence' AND session_id=$1 ORDER BY id DESC LIMIT $2`。
- db_bridge 实现返回 `Some(rows)`（空 Vec 是真实信号「没跑过工具」→ 该 BLOCK 就 BLOCK）。
- orchestrator：`HarnessGateOutcome` 加 `min_invocations` 字段（spec 透传），`enforce_min_invocations_ledger` 排在 evidence existence/kinds/freshness 之后；`chat_session_id` 缺失时跳过（GUI/CLI 都设；test 环境不设）。
- 自报版 `min_invocations_check` 保留（快速反馈 + 离线路径），ledger 版是权威。

## 3. H2 范围

- `apply_harness_gate_hook` 两处 `return (content, None)`（profile 加载失败 / StageHarness 构造失败）→ 返回 `internal_error_gate_outcome`（BLOCK、无 repair_correction——重试治不了坏资源，直接 Hold 给人看）。
- `feature_flags::harness_strict_enabled()`（`GOLISH_HARNESS_STRICT`，缺省关）；开启时 5 处 ledger 回查 infra Err → BLOCK + 纠正文案。in-scope 资产查询失败（gate 前置上下文）不在 strict 范围（保持回退自报，已有 trace）。

## 4. 测试

- 守卫：所有嵌入 spec 的 min_invocations key 被认识（红线测试）。
- capability_match 单测：dig via run_pty_cmd ✓、httpx via pentest_run ✓、recon_enrich_assets 算 subdomain_enum_passive ✓（防 06-07 活体回归）、空 rows → missing。
- enforce 层：rows=None 不动 outcome；rows=Some([]) 且 spec 要求 → BLOCK；rows 满足 → PASS 保持。
- H2：profile_id 不存在 → outcome 为 BLOCK（非 None）；strict flag 纯函数测试。
- H3：stage_spec 测试断言 target_intel gate_rules 含 named_check:min_invocations（先红后绿）。

## 5. 风险

| 风险 | 缓解 |
|---|---|
| 活体 run 的合法工具路径没被映射收编 → 误 BLOCK | 映射按 06-07 活体 run 证据反推（recon_enrich_assets 收编）；守卫测试钉 spec key；漏网路径的纠正文案会指名「哪个能力缺几次」便于排查 |
| 子 agent 工具调用是否落 ledger | 06-07 活体证据 #1115/#1133 证明 pentester 子 agent 路径落账（同 session_id 字符串） |
| strict 模式误伤 | 默认关；只建议 CI/活体验证开 |

## 6. 挂账（本设计不做）

- H4 ScopeService 真实化（InMemory 全 InScope → PG-backed）——真跑多 org red_team 前必须补
- H5 Branch 多后继声明式选边
- H6 GUI org_id 透传（并入 coverage-asset-scope-isolation P1）
