# golish-db / repo

> **一句话职责**：各表的 **scoped CRUD helper**——每个表一个模块（sessions/tool_calls/findings/targets/memories/vault/…40+ 个），统一经 `scoped.rs` 做资源所有权（IDOR）校验，是全仓库 DB 读写的权威入口。

- **类型**：目录模块（属于 crate [`golish-db`](../golish-db.md)）
- **路径**：`backend/crates/golish-db/src/repo/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何表的 CRUD（新 repo 方法、新表模块）时
- 任何跨服务读写——app crate 不直接写 SQL，走这里的 `repo::*`（横向耦合经 ports，纵向走 repo）
- 改 IDOR scoping 基座（`scoped.rs`）或批量操作的所有权校验时

## 职责

owns 全部表的结构化访问。每个表一个子模块（如 `findings.rs` / `targets.rs` / `memories.rs`），方法是 **scoped CRUD**：写/读都带 owner/project scope 校验（I2 IDOR）。`scoped.rs` 是 scope 校验基座，`audit/` 是审计相关 repo 子区。

## 公开接口

| 区域 | 说明 |
|---|---|
| `scoped`（`scoped.rs`） | scope/所有权校验基座（IDOR 守卫核心） |
| pentest/recon 表：`findings` / `methodology` / `execution_plans` / `evidence_classifications` / `targets` / `target_assets` / `organizations` / `directory_entries` / `sensitive_scan` / `custom_rules` / `passive_scans` / `scan_queue` / `fingerprints` / `vuln_scan` / `screenshots` / `sitemap_store` / `api_endpoints` / `endpoint_tests` / `js_analysis` | 各表 scoped CRUD |
| agent/会话表：`sessions` / `tasks` / `subtasks` / `tool_calls` / `message_chains` / `msg_logs` / `agent_logs` / `search_logs` / `terminal_logs` / `conversation_store` / `operation_state` / `stage_runs` / `sprint_contracts` / `sub_agent_dispatches` | agent 运行/会话数据 |
| harness 物化事实：`technique_outcomes` / `source_query_log` / `expansion_queue` / `stage_asset_waves` / `attack_candidates` | stage gate/reviewer 的 coverage/source/扩展线索投影；`stage_asset_waves` 冻结 wave-aware stage 的当前资产批次；`attack_candidates` 持久化 attack_candidate 阶段的结构化假设（设计 2026-07-02） |
| 其它：`memories`（向量记忆） / `vault`（凭据） / `notes` / `kb_research` / `wiki_kb` / `prompt_templates` / `vuln_intel` / `vector_store_logs` | 记忆/凭据/笔记/知识库等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 仅 `pub mod` 声明（40+ 表模块 + `audit`） |
| `scoped.rs` | scope/所有权校验基座（IDOR） |
| `audit/` | 审计相关 repo（嵌套子目录） |
| `coverage_truth.rs` | stage gate 的只读 DB 真值投影；EAS 会从 `targets.ports` / `fingerprints` / `real_ip` 关联 IP target 推导 found coverage；Enumeration 投影 JS/DIR/PARAM/JSAPI，并暴露 `web_capable_ip_assets` 给 IP-web 分母 |
| `surface_content_queries.rs` | Target Surface backend hierarchy 的只读 legacy web content 聚合：从 `api_endpoints` / `js_analysis_results` / `directory_entries` / `passive_scan_logs` 按候选 `target_id` 归一到已有 WebOrigin origin key，返回 counts + `refs_by_origin` / `unassigned_refs`（Phase 2.5C 轻量 refs，capped `MAX_REFS_PER_BUCKET=200`），不创建 identity |
| `targets.rs` | recon target CRUD + harness in-scope asset axis；`list_in_scope_values_created_before` 用 `created_at <= cutoff` 支持 no-schema stage wave denominator freeze |
| `stage_asset_waves.rs` | durable per-operation/org/stage asset batch repo：创建/读取 running batch、完成 batch；未分配 in-scope targets 可被后续 expansion pass 读取为待扩展 backlog |
| `attack_candidates.rs` | attack_candidate 阶段假设持久化（设计 2026-07-02）：`upsert_by_hash`（去重键 operation_id+target+`hypothesis_hash`=sha256(normalize)）/`list_by_operation`/`list_by_wave`/`update_disposition`，全 IDOR org 隔离（`organization_id IS NOT DISTINCT FROM`）；供 chain-wave 去重与 disposition 状态机 |
| `<table>.rs` | 各表 scoped CRUD（一个表一个文件） |

## 依赖

- `sqlx`（`PgPool`）、`golish-core`（scope 类型）；被全部 `*-app` + domain crate 消费

## 注意事项 / 坑

- **不变量 I2**：所有 CRUD 必须验资源所有权（含批量）——新增表模块时复用 `scoped.rs`，别写裸 scope-less SQL。
- **raw SQL allowlist**：极少数裸 sqlx 调用在 `check_repo_ownership.py` ALLOWLIST 登记；新增裸 SQL 要么走 repo，要么显式登记。
- **不变量 I9**：repo 方法别在事务里调外部 HTTP/MQ/长耗。
- app crate **不直接写 SQL**：纵向走 `repo::*`，横向跨服务走 `golish-app-core/ports`。
- `audit_log.project_path` 是 NOT NULL；`repo::audit` 对 `project_path=None` 统一写空字符串，工具/ledger 调用方不要绕过 repo 直接插 NULL。
- `source_query_log` 的幂等键必须包含 `organization_id`：`(organization_id, run_id, source, query, target)`。多 org `stage_run` 扇出时，root/子公司同源 provider 查询不能互相覆盖；`list_for_run` 供 gate/reviewer 只读 `(org, run)` 的 source/provider terminal rows，证明 source 尝试，不可当作 found truth。
- `coverage_truth.rs` 是 Found-only 投影：只能把业务表里确实存在的事实注入 gate，不从缺失数据推断 checked_empty；EAS 的 PORT/SERVICE/LIVENESS 必须保留 freshness window 约束。Enumeration 的 `GOLISH-ENUM-JS` 只从 `js_analysis_results` 行投影，`web_capable_ip_assets` 只返回 in-scope 且 `targets.http_status IS NOT NULL` 的 IP/CIDR 资产，用于“只有 IP 但确认为 Web 服务”时进入 JS/DIR/PARAM/JSAPI 分母。
- `surface_content_queries.rs` 是 Phase 2.5A/2.5C 的只读聚合层：candidate target ids 只包含 root IP target、同 scope 且 `real_ip == root_ip` 的 domain/url/wildcard target、以及 host 是 root IP 的 IP-literal URL target；legacy URL 归属必须走 `normalize_web_origin`，相对/坏 URL 只进 unassigned counts，解析到未出现在 backend `web_origins` 的 origin 只进 unmatched counts，不能新建 WebOrigin。Phase 2.5C 在 counts 之外附带**轻量 refs**（`SurfaceContentRef { kind,id,url,method?,status_code?,capture_path?,source? }`）：每个 matched origin 一份 `refs_by_origin`、unmatched/unassigned 一份 `unassigned_refs`，各自 capped 到 `MAX_REFS_PER_BUCKET`；refs 只是指针，绝不是完整 legacy row，counts 仍是总数的事实源。
- `targets.real_ip` 只属于可解析主体（domain/url/host），不能写到 `target_type in ('ip','ipv4','ip_address','cidr')` 的行上；`set_real_ip_by_id`、DNS backfill、`update_recon_extended_by_id` 都必须保留这个 SQL guard，避免 IP target 被错误挂到另一个 IP 聚合下。
- `surface_identity_backfill.rs`（identity backfill）里 `network_endpoints` 必须以 IP:port 为主键，所以端口来源分两类：**IP target 的 `ports`** 与**显式 IP 的 target_asset** → confirmed 端点（`backfill:targets.ports` / `backfill:target_assets`）；**域名/URL target 的 `ports`** 与**被动 service target_asset（`value="<port>/<proto>"` 无 IP）** → 用该 target 的 `real_ip` 补 IP 落成 **inferred** 端点（`backfill:targets.ports.real_ip` / `backfill:target_assets.real_ip`，confidence 0.6、`last_confirmed=false`）；`real_ip` 为空则跳过（不凭空造 IP）。这样 intel 被动发现的 host:port/service 也能进 identity 层，而不是只停在 legacy target_assets。
- `js_analysis::insert` 按 `(target_id, filename)` 幂等更新最新行：`browser_collect_js_api` 的 placeholder 可先落库，`js_extract_apis` 后续原地升级为完整分析；已存在完整静态分析时，新的 collector placeholder 不能把它降级覆盖。`js_analysis::list_by_target` 返回每个 filename 的最新行，避免历史重复行把前端 JS 数量和 ENUM-JS 口径放大。
- `stage_asset_waves` 是 additive schema：wave items 固定 `target_id/value/type/source` 成员关系，gate 仍从业务表/ledger 读事实；新发现 target 不会进入当前 batch denominator，也不会在单个 org PASS 后被自动重跑，后续由 global delta expansion pass 统一消费。兼容旧 org-level pass ledger 时，没有历史 wave 的 org 只把 `org_stage_completions.passed_at` 之后新增的 target 作为 delta；若已存在 running wave 但全部 item 早于该 pass，runtime 会补 complete 并跳过 worker。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db repo
```
