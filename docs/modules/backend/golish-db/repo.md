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
| pentest/recon 表：`findings` / `methodology` / `execution_plans` / `evidence_classifications` / `targets` / `target_assets` / `organizations` / `directory_entries` / `sensitive_scan` / `custom_rules` / `passive_scans` / `scan_queue` / `fingerprints` / `vuln_scan` / `screenshots` / `sitemap_store` / `api_endpoints` / `endpoint_tests` / `js_analysis` / `crawl_observations` | 各表 scoped CRUD；`crawl_observations` 是 crawler URL 的来源 origin 归属表，不参与 target promotion 或 ENUM gate truth |
| agent/会话表：`sessions` / `tasks` / `subtasks` / `tool_calls` / `message_chains` / `msg_logs` / `agent_logs` / `search_logs` / `terminal_logs` / `conversation_store` / `operation_state` / `stage_runs` / `sprint_contracts` / `sub_agent_dispatches` | agent 运行/会话数据 |
| harness 物化事实：`technique_outcomes` / `source_query_log` / `expansion_queue` / `stage_asset_waves` / `attack_candidates` | stage gate/reviewer 的 coverage/source/扩展线索投影；`stage_asset_waves` 冻结 wave-aware stage 的当前资产批次；`attack_candidates` 持久化 attack_candidate 阶段的结构化假设（设计 2026-07-02） |
| 其它：`memories`（向量记忆） / `vault`（凭据） / `notes` / `kb_research` / `wiki_kb` / `prompt_templates` / `vuln_intel` / `vector_store_logs` | 记忆/凭据/笔记/知识库等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 仅 `pub mod` 声明（40+ 表模块 + `audit`） |
| `scoped.rs` | scope/所有权校验基座（IDOR）；`TargetWriteGuard` 保存 target 的 id/org/project/scope/name/value/ports 原始授权快照，`lock_target_write_guard` 在 caller-owned 短事务内 `SELECT ... FOR UPDATE` 并逐字段 fail-closed 校验 |
| `audit/` | 审计相关 repo（嵌套子目录） |
| `coverage_truth.rs` | stage gate 的只读 DB 真值投影；所有 target-bound child truth（target_assets/dns_records/fingerprints/API/JS/directory）必须同时匹配 current in-scope target 的 project，旧 workspace 行不得跟随 moved target 进入 Intel/EAS/Enumeration gate；EAS 只从当前 target 本身的 fresh guarded 事实投影，绝不沿 `real_ip`/DNS 关系代投；IP 的 SERVICE 要求每个 confirmed-open port 有端口级服务面或同 target/port nmap fingerprint，CIDR 行只投 LIVENESS/PORT，WEB 只认 exact-origin WhatWeb outcome；并暴露 `confirmed_open_service_ports_for_assets` / JSON helper 给 EAS wrapper、coverage read-model 复用；Enumeration 投影 JS/DIR/PARAM/JSAPI |
| `operation_state.rs` | operation cursor/resume state；`get` 返回含 `state_blob` 的完整行，`get_epoch` 只返回 stage-attempt guard 所需的 operation/stage/start/supersede/engagement 五字段，供 producer 热路径避免搬运大型 resume JSON；EAS exact-origin transport breaker 使用本模块的 epoch-guarded namespaced JSONB 原子更新，generic checkpoint write 必须保留该 reserved namespace；producer seal 与独立 probe handoff 都用正 evidence id，并在 Enumeration read 时回查 current owner/project/outcome |
| `tasks.rs` | task CRUD + session-scoped `graph_flow` 通用恢复查询；startup reaper 用同一 fail-closed predicate 区分可恢复 checkpoint：合法 `graph_flow`（允许 checkpointer 已持久化下一节点而 `current_stage` 尚未推进），或仅供显式 CLI repair 的完整首阶段 flat checkpoint（identity/run 一致、`completed_count=0`，且每个 worker 的 org/chain/specialist/唯一 chain 与 `message_chains` session/task/agent/body ownership 全部匹配）会从旧 `running` 降为 `waiting`，其余 abandoned task 才标 `failed`；`ABANDONED_TASK_RESULT` 是 exact-resume CAS 的稳定公开 marker |
| `surface_content_queries.rs` | Target Surface backend hierarchy 的只读 legacy web content 聚合：从 `api_endpoints` / `js_analysis_results` / `directory_entries` / `passive_scan_logs` 按候选 `target_id` 归一到已有 WebOrigin origin key，返回 counts + `refs_by_origin` / `unassigned_refs`（Phase 2.5C 轻量 refs，capped `MAX_REFS_PER_BUCKET=200`），不创建 identity |
| `crawl_observations.rs` | crawler output 归属 repo：`upsert` 按 `(origin_target_id, observed_url, source_tool, kind)` 幂等写入；active harness 走 `upsert_guarded`，在同一短事务锁定/复核 origin target raw witness，且 conflict 只允许相同 org/project 更新（legacy 未绑定行由 migration 先 backfill）；`list_for_current_target_owners` 仅在 observation org/project 仍匹配 current in-scope origin target 时给 Target Surface 返回 crawled URL；不写 `targets` / `api_endpoints` |
| `targets.rs` | recon target CRUD + harness in-scope asset axis；`list_in_scope_values_created_before` 用 `created_at <= cutoff` 支持 no-schema stage wave denominator freeze |
| `stage_asset_waves.rs` | durable per-operation/org/stage asset batch repo：创建/读取 running batch、完成 batch；所有尚未进过该 operation/org/stage 任一 wave 的 in-scope targets（含首波 limit 截断 backlog 与运行中新资产）都可进入 supplemental wave，不使用 parent `started_at` 时间地板；最后一次候选读取与 org completion 水位由短事务锁屏障原子裁决 |
| `surface_identity_queries.rs` | EAS exact-origin/current-owner 查询：从 fresh web observation 投影 canonical `scheme://host:port`，再复用 `golish-pentest-domain::confirmed_target_web_origins`，只以 target 当前 exact URL、confirmed-open `ports[].url` 或明确 HTTP(S) service hint 验证 origin 仍受当前 target 授权；display name、foreign ports URL、CIDR child IP origin 均不能授权 |
| `stage_purge.rs` | dev stage reset 的破坏性事实清理 executor；所有函数接受同一个 `PgConnection` 以便 command 在单 transaction 内执行，`technique_outcomes` 必须按 org subtree + affected embedded stage specs 的 technique union 双重过滤 |
| `audit/mod.rs` / `audit/pentest.rs` / `audit/queries.rs` / `evidence_classifications.rs` | evidence append 的事务内写接口；`PentestAudit::{started,completed,failed}_guarded` 让 target-bound scan timeline 的 witness lock、lineage lookup 与 audit insert 共用短事务；默认 target timeline `list_by_target` 只返回 audit project 仍匹配 current in-scope target project 的行，跨 workspace 历史需未来显式授权入口 |
| `attack_candidates.rs` | attack_candidate 阶段假设持久化（设计 2026-07-02）：`upsert_by_hash`（去重键 operation_id+target+`hypothesis_hash`=sha256(normalize)）/`list_by_operation`/`list_by_wave`/`update_disposition`，全 IDOR org 隔离（`organization_id IS NOT DISTINCT FROM`）；供 chain-wave 去重与 disposition 状态机 |
| `<table>.rs` | 各表 scoped CRUD（一个表一个文件） |

## 依赖

- `sqlx`（`PgPool`）、`golish-core`（scope 类型）；被全部 `*-app` + domain crate 消费

## 注意事项 / 坑

- `tool_calls::scoping_actions_for_session` 必须返回当前 operation Scoping 窗口内**全部** completed `scope_review`（稳定升序），不能只取 latest：第一次人工编辑/拒绝后再弹一次确认旧快照，不得把前一次决定洗掉。上层对非空 trusted snapshot 只接受 exactly one persisted review。Red-team subsidiary choice/proposal/unit review 还必须绑定调用方传入的 trusted root org：明确 parent-only choice 可直接完成该分支；纳入分支只接受同 org 成功 proposal 后的 non-skipped、可解析 unit review，不能用 finished error/skip 或乱序生命周期冒充人审。新 choice 严格验结构化 context UUID；legacy fallback 必须在 prompt 中精确出现当前 root 全名。
- **不变量 I2**：所有 CRUD 必须验资源所有权（含批量）——新增表模块时复用 `scoped.rs`，别写裸 scope-less SQL。
- Active producer 的 target-bound 业务写不能只在 Rust 写前独立 revalidate：`api_endpoints::*_guarded` / `js_analysis::*_guarded` / `crawl_observations::upsert_guarded` 必须先在同一短事务调用 `lock_target_write_guard`，再用 guard 派生的 target/project 写 child row。API conflict update 和 crawler idempotent conflict 都要匹配 existing/excluded project；crawl observation 还要拒绝其它 org/project 的 conflict；JS existing-row/file-path update 要同时匹配 target/project。零返回或任一 raw witness 漂移都视为失败并 rollback。
- Network-capable legacy callers 可用 `load_target_write_guard` 捕获 current in-scope/project-bound raw witness，并在准备前后调用 `validate_target_write_guard`；需要补写指纹时走 `fingerprints::upsert_batch_guarded`，scan timeline 走 `PentestAudit::{started,completed,failed}_guarded`，整批/lineage 写在同一短事务锁 target，且旧 project 同键 conflict 必须 fail-closed。
- Target-bound child 的展示/汇总读取走 `*_by_current_target_owner`，或在聚合 SQL 中显式要求 child `project_path` 与 current in-scope target 一致；Target Surface legacy union/crawl、security overview、engagement truth 和 coverage truth 都不能只凭稳定 `target_id` 让旧 workspace 行随 target 迁移。`surface_identity_backfill` 可接受 child 或 target 一侧缺失 project 的 legacy 行，但两侧 project 都已知且冲突时必须 skip。
- `directory_entries` 的 target-owned 唯一键是 `(target_id,url,tool) WHERE target_id IS NOT NULL`，绝不能退回全局 `(url,tool)`；active route producer 走 `insert_entry_guarded`。target-bound timeline、ledger evidence 与 Enumeration outcome batch 同样必须在各自短事务先锁 `TargetWriteGuard`，不能把独立 revalidate 当作写授权。
- `directory_entries::list_by_current_target_owner` 是 target-specific UI/port 的默认读入口；它要求 current target 仍 in-scope 且 child project 精确匹配。该查询 JOIN `targets` 后的 projection 必须全部限定为 `de.<column>`，因为两表同时拥有 `id/content_type/created_at`，未限定会让 PostgreSQL 以 ambiguous column 拒绝整个 Target Surface 请求。只有调用方已持显式 project authorization 时才用 `list_by_target_project` / `list_by_project`，不要回退裸 `list_by_target`。
- **raw SQL allowlist**：极少数裸 sqlx 调用在 `check_repo_ownership.py` ALLOWLIST 登记；新增裸 SQL 要么走 repo，要么显式登记。
- active producer 只校验 operation stage attempt 时必须优先用 `operation_state::get_epoch`；只有确实消费 resume/cursor JSON 的路径才可用完整 `get`，避免批次内按 root 重复读取和反序列化大型 `state_blob`。
- EAS WEB transport breaker 的 slot 绑定 operation EAS epoch/org/target/canonical origin/technique，并只对连续同一 `failure_class` 递增；成功 observation 清 slot。attempt 3 的 producer marker、可选 independent handoff 及其 evidence ids 必须继续被 epoch/current-stage/supersede guard 保护。进入一个新的 EAS epoch 才清 namespace，EAS→Enumeration 必须保留 handoff；Enumeration reader还要联结 producer/independent audit、producer blocked outcome 和 current in-scope target，不能信任裸 JSON flag。
- startup abandoned-task reaper 的 pause/fail 分支必须继续复用 `tasks.rs` 的同一 recoverable-checkpoint predicate；flat checkpoint carve-out 只覆盖 `graph_flow` 缺失且仍在首节点（`completed_count=0`）的完整 exact-resume repair witness，不能把任意 partial JSON 当作通用 resumable。`latest_resumable_by_session` 仍只认 `graph_flow`，不要在通用聊天恢复入口放宽 flat repair。
- **不变量 I9**：repo 方法别在事务里调外部 HTTP/MQ/长耗。
- app crate **不直接写 SQL**：纵向走 `repo::*`，横向跨服务走 `golish-app-core/ports`。
- `audit_log.project_path` 是 NOT NULL；`repo::audit` 对 `project_path=None` 统一写空字符串，工具/ledger 调用方不要绕过 repo 直接插 NULL。
- `source_query_log` 的幂等键必须包含 `organization_id`：`(organization_id, run_id, source, query, target)`。多 org `stage_run` 扇出时，root/子公司同源 provider 查询不能互相覆盖；`list_for_run` 供 gate/reviewer 只读 `(org, run)` 的 source/provider terminal rows，证明 source 尝试，不可当作 found truth。
- `technique_outcomes` 的幂等键同样必须包含 `organization_id`：`(organization_id, run_id, asset, technique)`。一个 `stage_run` session 会扇出多个 org；缺 org 的旧唯一键会让相同 exact origin/technique 的 sibling org 相互覆盖。`seq` 按 org/run 生成但只是排序提示；并发同 seq 时读取必须用 `asset, technique` 业务键稳定 tie-break，读写继续双重按 org + run 隔离。Enumeration producer 用 `query` 保存 opaque attempt generation：attempt-start partial 与 terminal publish 分别走短事务 conditional batch，事务同时锁 target、锁/校验 operation epoch 与 engagement org，并在 terminal publish 前锁 marker 做 generation CAS；禁止跨网络持事务，旧 attempt/restart 后的写必须返回 `Superseded`。`directory_entries::insert_entry_guarded_if_attempt_current` 复用同一锁序，防 route 旧 HTTP 响应在新 generation 后补写业务行；`technique_outcomes::upsert_batch_guarded_if_attempt_current_and_clear_state_slot` 只接受 `found|empty|blocked`，并在同一事务发布 terminal outcome + 删除对应 operation-state checkpoint slot，保证 crash 前后不会留下 terminal row 与可重放 cursor 的矛盾组合。
- `technique_outcomes::upsert_terminal_if_unfinished` 专供最终 gate PASS 后物化 `blocked|not_applicable`。其 `ON CONFLICT ... WHERE existing.outcome IN ('partial','error')` 是竞态护栏：可补缺行/未完成行，但绝不覆盖 found、empty 或已有 blocked/not_applicable。它不接受 checked_empty/found，后两者继续由直接 producer/evidence 路径负责。
- `stage_purge` 不能按整棵 org 无差别删除 `technique_outcomes`：`restart_from_stage_purge` 只删除 affected stages 的 embedded `expected_techniques` 并集，祖先 stage outcomes 必须保留。domain facts、completion ledger、asset waves、technique outcomes 与 target status rollback 必须共用一个 transaction；任一 SQL 失败全部 rollback。
- Evidence hash-chain append 必须通过 caller-owned transaction 调 `log_evidence_in_transaction` + `evidence_classifications::insert_in_transaction`：target-bound append 先锁 raw target witness；同 operation 的 advisory lock、prev-hash 读取、显式 hash timestamp 的 audit insert 和初始 classification 共用一个 transaction，避免授权漂移、并发分叉或孤儿 evidence/classification。`technique_outcomes::upsert_batch_guarded` 还必须校验每条 row 的 organization 与 guard owner 一致。
- `audit::evidence_facts_for_session_org_fresh` 的 target-bound projection 必须携带 `tool_name` 与 `detail->>'kind' AS evidence_kind`，不能只返回事实四元组。Enumeration blocked read path 用这两列验证可信 producer（`enum_preflight_web_origins` / `enumeration_transport_blocked`）；found/empty 仍保留各自多 producer 合同。
- EAS/Enumeration fresh target-bound evidence 读回同时要求 session、`detail.organization_id` producer 快照、target 当前 org/in-scope、非空且精确相等的 project、`created_at >= stage_started_at`；并返回 target type/name/value/ports raw witness，由 app 层再次确认 evidence asset/technique 仍受当前 target 授权。guard org=None、legacy 无 producer org、target 跨 org/project 移动或同 org 内换值/接管 origin 均 fail closed。
- `coverage_truth.rs` 是 Found-only 投影：只能把业务表里确实存在的事实注入 gate，不从缺失数据推断 checked_empty；EAS 的 LIVENESS/PORT/SERVICE/WEB-FINGERPRINT 必须保留 freshness window 约束。SERVICE-FINGERPRINT truth 是 per-port 严格口径：`targets.ports[]` 里每个 SERVICE-applicable confirmed-open entry 都要有 service/version/product/banner/webserver/technologies 这类端口级服务面，或 `fingerprints` 中存在同 `target_id`、同 `evidence.port`、`source='nmap'` 的 service fingerprint；任意泛化 `fingerprints` 行（尤其 WhatWeb web-origin 技术栈）不能替代 IP:port 服务指纹。WEB-FINGERPRINT truth 只从 `source='whatweb'` 且 category 为 web server/technology 的 `fingerprints` 投影，并通过 `eas_web_capable_assets` 只让已有 HTTP surface 的资产进入 WEB 分母。`tcpwrapped` / `unknown` / `open` / `filtered` / `closed` 不算强服务面，但 port-scoped nmap terminal 行可关闭该端口，避免重复重扫；多端口主机上的 bare DNS/53 不阻塞 SERVICE-FINGERPRINT。`confirmed_open_service_ports_for_assets` 只解析当前 in-scope、且在调用方给出 workspace 时 `targets.project_path` 与其精确相等的 `targets.ports[]` open non-53 端口；NULL/空 project legacy 行不能给 exact-workspace EAS recipe 注入端口。它供工具自动补扫和避免 empty outcome 覆盖仍 open 的 DB truth，不是 gate PASS shortcut。Enumeration 的 `GOLISH-ENUM-JS` 只从 `js_analysis_results` 行投影，`web_capable_ip_assets` 只返回 in-scope 且 `targets.http_status IS NOT NULL` 的 IP/CIDR 资产，用于“只有 IP 但确认为 Web 服务”时进入 JS/DIR/PARAM/JSAPI 分母。
- 上述 helper 名中的 `ip_assets` 是历史名称；当前 stage-aware resolver 不允许
  CIDR 行直接成为 SERVICE/WEB/Enumeration 执行身份。CIDR 只闭 range LIVENESS/PORT，
  guarded child IP 在 supplemental wave 形成 exact origin 后才进 Web/Enumeration 分母。
- `surface_content_queries.rs` 是 Phase 2.5A/2.5C 的只读聚合层：candidate target ids 只包含 root IP target、同 scope 且 `real_ip == root_ip` 的 domain/url/wildcard target、以及 host 是 root IP 的 IP-literal URL target；legacy URL 归属必须走 `normalize_web_origin`，相对/坏 URL 只进 unassigned counts，解析到未出现在 backend `web_origins` 的 origin 只进 unmatched counts，不能新建 WebOrigin。Phase 2.5C 在 counts 之外附带**轻量 refs**（`SurfaceContentRef { kind,id,url,method?,status_code?,capture_path?,source? }`）：每个 matched origin 一份 `refs_by_origin`、unmatched/unassigned 一份 `unassigned_refs`，各自 capped 到 `MAX_REFS_PER_BUCKET`；refs 只是指针，绝不是完整 legacy row，counts 仍是总数的事实源。
- `crawl_observations` 是 crawler URL 的来源归属表：`origin_target_id` 指向被爬的目标，`origin_key` 对齐 `web_origins.origin`（`scheme://host:port`），`observed_url` 可是同源或三方外链。它只服务 Target Surface 的 Crawl tab / 审计追踪；不要把这些行投影成 `api_endpoints`、不要让 coverage_truth 从它推导 `GOLISH-ENUM-*` found，也不要因为 observed external host 自动创建 scoped target。
- `targets.real_ip` 只属于可解析主体（domain/url/host），不能写到 `target_type in ('ip','ipv4','ip_address','cidr')` 的行上；`set_real_ip_by_id`、DNS backfill、`update_recon_extended_by_id` 都必须保留这个 SQL guard，避免 IP target 被错误挂到另一个 IP 聚合下。它只是从完整 A/AAAA 关系确定性选出的 primary cache，不代表唯一关系、alive、同一资产或主动授权；被动 DNS 写入不得碰 `is_alive` / `last_alive_at`。
- `surface_identity_backfill.rs`（identity backfill）里 `network_endpoints` 必须以 IP:port 为主键，所以端口来源分两类：**IP target 的 `ports`** 与**显式 IP 的 target_asset** → confirmed 端点（`backfill:targets.ports` / `backfill:target_assets`）；**域名/URL target 的 `ports`** 与**被动 service target_asset（`value="<port>/<proto>"` 无 IP）** → 用该 target 的 `real_ip` 补 IP 落成 **inferred** 端点（`backfill:targets.ports.real_ip` / `backfill:target_assets.real_ip`，confidence 0.6、`last_confirmed=false`）；`real_ip` 为空则跳过（不凭空造 IP）。这样 intel 被动发现的 host:port/service 也能进 identity 层，而不是只停在 legacy target_assets。
- `js_analysis::insert` 按 `(target_id, filename)` 幂等更新最新行：`browser_collect_js_api` 的 placeholder 可先落库，`js_extract_apis` 后续原地升级为完整分析；已存在完整静态分析时，新的 collector placeholder 不能把它降级覆盖。`js_analysis::list_by_target` 返回每个 filename 的最新行，避免历史重复行把前端 JS 数量和 ENUM-JS 口径放大。
- `stage_asset_waves` 是 additive schema：wave items 固定 `target_id/value/type/source` 成员关系，gate 仍从业务表/ledger 读事实；新发现 target 不会进入当前 batch denominator，也不会在单个 org PASS 后被自动递归重跑。所有当前 org wave PASS 后，runtime 调 `create_next_or_seal_completion`：同一短事务先串行化 wave closer，并以 `targets SHARE` 锁等待已在途 writer，再读取所有未分配 in-scope backlog；有候选就创建 supplemental wave，无候选才写 `org_stage_completions` 水位。这样空 SELECT 与 pass token 之间没有可丢资产窗口；屏障提交后的 target 属于后续 operation/stage 生命周期。若 runtime 因 fresh legacy completion 没带 request-local parent，但该 operation 已有历史 wave，repo 必须在锁内取 latest wave 作为 parent；不能把 backlog 建成 parentless “首波”后被 legacy resume 规则跳过。下一次 `stage_run` 只消费 supplemental items；因此首波 limit 截断的旧资产不会永久遗漏，运行中新资产也会在后续波次出现。legacy running wave 只有在没有 parent 且全部 item 早于 fresh pass 时可补 complete；有 parent 的 supplemental wave 必须跑，不能被第一轮 pass ledger 短路。
- wave item 的 DB 主身份是 immutable membership `target_id`，`asset_value/type/source` 是创建 wave 时的 target snapshot；bridge 的 `StageAssetWaveView` 同时透传对齐的 ids/values。`current_running` 读 items 后必须用同一稳定算法重算 `asset_hash`：FK cascade 删除全部或部分 items 都会产生 empty/hash mismatch 并直接 Err，不能把损坏 wave 当 NoWave。下游 canonical-origin read model 按 target id 判成员并让 current-wave owner 优先共享-origin dedupe；submit preview 的窄 bridge 仍按 `(operation_id, organization_id, stage_kind)` scoped repo，不能信模型传入 membership。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db repo
```
