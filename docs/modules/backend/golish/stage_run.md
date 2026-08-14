# golish / stage_run

> **一句话职责**：headless 单/区间阶段实跑、shared-DB stage fork 与 exact recovery（fresh `--stage-run` / immutable-source `--stage-run-fork` / persisted `--stage-run-resume`）——三者共享 GUI 的 TaskOperation/TaskOrchestrator/Stage/Gate 内核。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/stage_run/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 headless 阶段实跑（boot/seed/run/report）、`--stage-run`/`--stage-run-fork`/`--from`/`--to`/`--only`/`--org`/`--target` 行为时
- 调试逐阶段测试（替代 `just dev` 起 GUI 手动驱阶段）时
- 改真实 smoke runner（`scripts/stage_smoke.py` / `just stage-smoke`）或临时 DB 测试模式时

## 职责

无 GUI bootstrap（lazy pool + spawn_embedded_pg + 就绪门 → `AppState::new` → `extract_agent_state`）→ `cli::initialize_agent(CliRuntime)` → `configure_bridge(None)`。fresh stage slice 与 exact resume 都先进入 `golish-agent-app::ai::task_operation` 共享 kernel；前者再调用 `TaskOrchestrator.run_stage`，后者先做 selector/identity/chain scope 校验、operation advisory claim 与必要的首-stage `graph_flow` repair，再调用同 task 的 `TaskOrchestrator.resume`（绝不 `run_stage`）。`main.rs` 把两条路径都放到专用 32MiB 大栈线程。测试入口可显式传 `--ephemeral-db`，但 resume 与 ephemeral/fresh slice/seed 参数互斥。**V2-writing 的 post-Scoping direct entry 会在创建 operation 前一次性解析 CLI root/descendants/ownership threshold，并把 `CliFlags` decision + sealed snapshot 与唯一 task/operation/stage execution 原子提交；从 Scoping 起跑是窄例外，只先绑定 confirmed root identity，待 typed choice + trusted deliverable 过 gate 后再用 `finalize_scoping_scope` 原子冻结 snapshot/root Unit/submission。两条路径都只调用一次 `run_stage`。历史 per-org child-operation scheduler 仅保留给 `LegacyV1`，resume 也始终只重驱选中的同一 operation。** 设计见 `docs/design/2026-06-06-headless-single-stage-runner.md`、`docs/design/2026-07-11-stage-run-cli-exact-resume.md`、`docs/design/2026-07-15-cli-scoping-explicit-org-fast-path.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| stage_run 入口（boot → orchestrate → report） | headless 跑 + 报告 |
| `--stage-run-resume <selector>` | 恢复旧 stage-run chat key / DB session UUID / operation UUID；非终态复用旧 DB session UUID、task/operation、org、profile、stage freshness、transcript 和 worker chain。若 task 已 `finished`，只有 caller 同时给出完整且精确匹配的 session/task/operation/org/stage identity 才直接返回 durable result；该终态重放不要求 active execution、不取得 runtime claim、不初始化模型，也不重建 report revision |
| `--allow-orphan-running` + exact expected ids | 仅作 orphan 身份诊断；durable resume claim 仍严格要求 `waiting`，残留 `running` 必须先由 startup reaper 与显式 repair 转回 `waiting` |
| `--repair-missing-graph-flow` | 在 exact identity + flat checkpoint 校验后，用 guarded `jsonb_set` 只补 `graph_flow`，保留 `stage_run_workers`/producer checkpoints/未知 sibling |
| `--repair-reaped-task` | 仅将带固定 startup-reaper abandoned marker 的 exact failed orphan 以 session/profile/stage/org/state/update-time CAS 恢复为 `waiting`；普通失败不可恢复 |
| `--org`/`--target` intake（`should_seed_upstream`/`maybe_seed`/`seed_upstream`/`build_objective`） | 显式 CLI `--org`（包括 Scoping）先 get-or-create root 并进入 shared typed launch 的 `ConfirmedOrganizationIntake`，这是 headless 身份快通但 target authority 仍为空；只有本次明确且通过 URL/IP/CIDR/DNS/wildcard exact-shape 校验的 `--target` 才进入 `ConfirmedTargetIntake` |
| `GOLISH_STAGE_RUN_SEED_OPEN_PORTS` | smoke 专用：给临时 DB 的 seeded targets 写入 confirmed-open `targets.ports[]`，格式 `host=80,443;host2=9001` |
| `GOLISH_SEED_VAULT_KEY_FILE` | headless provider 验收专用：从权限受控文件读取单行 `provider=key`，同一事务更新 canonical `<provider>.default.api_key` 与 legacy provider 名称，避免 stale canonical key 覆盖 CLI seed；日志永不输出 key |
| `--ephemeral-db` / `--keep-ephemeral-db` | stage-run 测试专用临时嵌入式 PG；默认清理，可显式保留 pgdata |
| `--db-smoke-summary` | 停 PG 前在同一 repeatable-read 快照打印 operation identity、scoped counts 与完整实体链 exact sets：Stage/Deliverable/Evidence、Target Intel Goal、Enumeration lane + endpoint occurrence + parameter assessment/fact + hashed provenance + Resolution closeout、AU revision、Investigation Main/Analysis/Primary/delegation、Hypothesis/Verification/FactDelta/JIT authority、closure/publication、Reporting revision；每组都含 member count、canonical set hash 与安全字段 members |
| `scripts/stage_smoke.py` / `just stage-smoke` | 包装真实 `golish --stage-run`，默认临时 DB，可选本地 HTTP fixture；`--controlled-fixture` 是验收 preset，会同时启用本地 fixture、unified topology 与 JSON exact-set 校验，`--fixture-web` 仍只表示 fixture；脚本可显式传 `--provider` / `--model`，枚举 smoke 可用 `--route-probe-max-runtime-ms` / `--route-probe-max-requests` 控制 route_probe 前台预算 |
| `scripts/run_tree.py --db` | transcript 时间线按最新 stage 展开 Company Controller→`update_plan` 具体步骤→动态 SubAgent→Worker Output→prepare/final submit→Gate，并显示同 chain resume、Controller anomaly、主 Agent 完成 turn/token 与 SubAgent model-start 汇总；旧 Producer/Aggregator run 明确标为 `legacy-fixed (not Company Controller)`。DB 部分继续输出 runtime/attack rollout、operation-frozen 双 contract、scope/execution/unit/worker、Team Plan→WorkItem/dependency→Worker→Output/Request→Barrier、Candidate Wave/Attempt/FactDelta/residual risk 与 recovery；运行中的 task 要求唯一 active execution，finished task 若 active=0 则确定性选择 current-stage 最新 completed execution 并标 `terminal_selected`，其他形状仍 fail closed；只显示 lease/checkpoint presence/size、hash 与安全计数，不泄露 token/body/raw output/request budget |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | boot + seed + 事件消费 + orchestrate + format_report |
| `runtime_v2.rs` | trusted CLI scope 一次冻结、relational fleet report 与 Scoping/specialist/root-only resume classifier |
| `fleet.rs` / `scheduler.rs` | `LegacyV1` per-org child-operation compatibility adapter；V2-writing contract 在 production seam 与 regression test 双重禁止调用 |
| `scripts/stage_smoke.py` | 真 stage smoke runner（临时 workspace/DB，本地 fixture，可接 `run_tree.py`） |
| `justfile` (`stage-smoke`) | 脚本化入口：`just stage-smoke <profile> <to-stage> "<objective>"` |

## 依赖

- crate 内 app（bootstrap）、cli（initialize_agent）、agent 栈、`OrgFleetExecutor`/`run_fleet_scheduler`（仅 LegacyV1 child-operation fallback）；`golish-agent-kit::harness`、嵌入式 PG

## 注意事项 / 坑

- 真 LLM + 真工具 + 真 evidence（无 GUI）；活体跑需 LLM key + 网络。
- 普通公司阶段的 CLI/GUI 调度合同现统一为 `company_controller_v1`：Target Intel / EAS / Enumeration / Vuln 都应在 `run_tree.py --full --db` 中显示每 org 一个 `leader:primary` Company Controller，冻结 specialist 分别为 Recon / Prober / Enumerator / Vuln Scanner。Candidate/Verification 仍显示 Wave/CandidateAttempt，Post-Exploit/Reporting/Cleanup 仍显示 typed scheduler；不要为了 UI 统一将这些不同业务语义强行塞进普通 Team。
- Vuln continuation 不要求用户反复重发来碰撞调度窗口：已完成的narrowed anonymous HTTP batch若只剩timeout residual，会在同operation封存为blocked并恢复原Controller做final submission；历史 evidence和child输出保留，网络请求不重放。
- exact resume 不能再假设“一个 Unit 只有一个 Worker”：Stage Team Unit 必须按 exact Plan/WorkItem/Worker identity 选择唯一 server-seeded `leader:primary` Controller，同时逐一验证动态 child 的 Unit/operation/org/role/kind/key、message chain 与 active tool fence。多个 leader、foreign WorkItem/child 或仍持 live lease 的 child 均 fail closed；合法 terminal child 不影响 Controller owner 选择。
- Asset Primary 的chain agent分类读取current source schedule及其全部applied execution rearm lineage，并以exact `(work_item_id,worker_run_id)`成员判断Primary；current、immediate predecessor与更深base ancestor共享同一Primary chain，单纯复用该chain ID的foreign Worker仍按普通specialist校验并拒绝。
- `--ephemeral-db` 只隔离数据库；LLM provider / 外部情报源 / 主机工具仍是真调用，跑活体 smoke 前要确认授权与 API 成本。
- unified Scoping→Reporting smoke 不以“八个 stage_runs 存在”代替业务闭环：wrapper 会要求 Target Intel epoch/review/frontier、五条 Enumeration lane、AU revision、单一 Investigation run、每 org Main read session、Analysis binding、PentAGI task plan/唯一 Primary delegation census、Hypothesis/VerificationTask/Campaign/FactDelta、run closure/publication 与 Reporting revision exact sets 全部非空。历史 Prepared Action、Operator authorization、execution 与 residual-risk exact sets继续作为只读审计输出，但是否非空由旧run的hypothesis/risk tier决定；它们不是resume authority，不能为通过smoke伪造高风险动作。
- Investigation 在模型 Analysis 前检查冻结的 Tool Truth 前置：stage fork 若继承了过期/invalid root，返回 typed `investigation_analysis_host_stale_prerequisite`，并给出 earliest `required_from_stage` 与 exact `golish --stage-run-fork … --from … --to investigation`；same-operation stale root 则返回 Tool Truth revalidation authority，不能误建议另起 fork。人类可读 report 对失败 `stage_run` ToolResult 只投影定长单行 `code/error`，其余 payload 明确 redacted；完整结构继续留在 transcript/run.log。
- Investigation closeout 的 organization denominator 必须直接取 immutable closure publication members；不能用 engagement 的整棵 org subtree 重算，否则 root-only operation 在目录中存在子公司时会被错误拒绝。2026-08-10 fresh moresec.cn 验收只冻结默安科技根公司与两个 root target alias，没有纳入子公司。
- exact resume 不再接受 Campaign/PreparedAction 人工 authority packet；旧 flag、packet parser与授权写入分支均已物理删除。历史 Campaign/PreparedAction 行只供 DB smoke/reporting审计，新 Investigation 只能沿 asset-bound dynamic Verification session继续。
- Reporting 的 headless完成条件是 validated、evidence-linked concise summary 与 closure publication exact authority；模板替换、渲染 artifact 或自动 publish 都不是本轮必要条件。
- finished task 的 exact resume 是只读终态重放，不是再执行：它仍校验 workspace、operation topology/runtime contract、完整 expected identity、未 supersede 与非空 durable result；任一字段缺失或漂移都 fail closed。2026-08-11 moresec.cn GUI-origin operation 的同一 resume 返回既有 validated summary，DB 仍只有一个 report revision。
- `--stage-run-test-database golish_gatefix_*` 是已有 shared-DB operation 的测试专用选择器，可用于 exact resume 或 immutable-source fork：调用方必须先显式创建克隆，stage_run 只在数据库名通过前缀/字符/长度护栏后覆盖连接目标。它不接受默认库名、不负责清理，也不改变 LLM/工具仍可能真实调用的事实。
- `GOLISH_STAGE_RUN_SEED_OPEN_PORTS` 只用于 isolated smoke；它会在 seed target 后写 `targets.ports[]` 和 EAS collected timestamps，方便复现“DB 已确认 open port，但 SERVICE-FINGERPRINT 没收口”的 retry 类问题。
- `scripts/stage_smoke.py` 对**任何实际经过 Enumeration 的 slice**（包括 `scoping -> attack_candidate`，不只终点恰好是 Enumeration）默认给 `route_probe_paths` 设置小前台预算（env：`GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS=30000`、`GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS=800`）并写进 objective，避免本地 fixture / DeepSeek smoke 等三分钟才收束；从 Vuln/Candidate 之后起跑或 `--only attack_candidate` 不伪装成经过 Enumeration，需要完整字典闭环时传 `--full-route-probe`。
- 临时 DB 跑完默认删除；需要事后连库人工排查时加 `--keep-ephemeral-db`。自动验证优先看 `--db-smoke-summary`，它是在 PG 仍存活时查询出来的。
- 普通 `--stage-run` 启动前会探测配置端口：若 PostgreSQL 已在监听，说明本次复用了用户现有 DB，收尾只关闭本进程的 pool 并保留现有 PG；只有本次真正启动的 embedded PG 才调用 `stop()`。不能把“端口已占用、复用现有 PG”误当成本次拥有其生命周期。
- 被 Ctrl-C 或 panic 打断的 smoke 可能来不及停临时 embedded PG；收尾时只清理 `golish-stage-run-db-*` 临时 PG，勿杀默认 app DB（`~/Library/Application Support/golish-platform/pgdata`）。
- gate 走确定性 evidence 门（I7/I8）。`--auto-approve` 的 ask_human 路径是 typed policy：trusted `--target` 才能批准 exact `scope_review`；fresh CLI 的 subsidiary flag 只能在 request context 的 `organization_id` 精确匹配本次 seeded `--org` 时选择对应 option（默认 root-only）。当前内置 flow 的常规人工确认只在 Scoping，post-Scoping Gate PASS 后不会再发 generic phase confirmation；`--approve-phase-boundaries` 继续被 parser/auto-resolver 接受以兼容旧脚本与 Scoping-origin 兼容事件，但不是 target/Candidate/tool 授权。ordinary choice、unit_review/credentials/freetext/unknown 与无 trusted target 的 scope review仍全部 decline，禁止 generic `auto-approved` 放松 gate。subsidiary control threshold 默认与 GUI/contract 一致为 51%。
- fresh CLI 的 `--target` 是 trusted pre-stage intake：必须在 Scoping 前以
  `source='stage-run-seed'` 落精确 domain/IP/CIDR/URL/wildcard target。Headless
  `scope_review` auto-response 只从这些 `--target` 构造 exact table payload，不从 objective/
  LLM context 推断新 target；type/scope/value 必须与 DB trusted snapshot 一致。
- 若同一 exact Target 已由 `asset_intel` 等 discovery 路径落库，trusted `--target` 通过共享 `ReconTargetsPort::target_add` 原地升级来源与同 org 绑定，不插第二条重复 Target；不同 org/project/type 或 `scope=out` 仍 fail closed。
- Scoping 未落 trusted seed 时必须阻塞，不得依靠 Target Intel `manage_targets`
  补种。`organizations.domains/app_domains/ip_ranges` 及 provider 数据都不能替代 CLI seed。
- 从 Scoping 起跑时，显式 company-only `--org` 是 CLI 独有的 confirmed-identity fast path：`should_seed_upstream` get-or-create exact root，typed launch 使用 `ConfirmedOrganizationIntake`；这不要求与 GUI prompt 的 `UnconfirmedSubject` 交互对齐，也不从公司名推导任何 domain/IP/CIDR/URL。Scoping create 不预冻结 runtime scope；必须等 persisted typed subsidiary choice + trusted deliverable 通过后，由 `finalize_scoping_scope` 同事务生成 decision/sealed snapshot/passed root Unit 并绑定 submission，失败收紧为 BLOCK。typed launch 同时投影 current-invocation target authority=`Some(false)`，所以即使复用的同名 org 已有历史 in-scope targets，Target Intel → EAS 也会在读取这些历史 rows 前 HOLD。headless exact resume 仅在存在合法 fresh marker 时恢复其值；缺 marker 一律收紧为 `Some(false)`，malformed 则拒绝，不能回退 `None` 借旧 target。只有本次 invocation 明确给出且通过 shared exact-shape 校验的 `--target` 才投影 `Some(true)`，并且仍须通过 DB snapshot 校验。fresh CLI 若绕过 Scoping且 slice 会进入 EAS/Enumeration/Vuln/Candidate 或后续主动阶段，必须在**本次 invocation** 重传至少一个 exact `--target`；完整 Scoping 起跑和 passive-only Target Intel 可无 target；`maybe_seed` 写失败必须终止 run，不能丢掉显式 CLI identity/scope 后继续。
- V2-writing fresh CLI 只取得一次 universal top-level request token、只调用一次 `orchestrate`；全部 descendant Unit/Worker 共用同一个 operation 与 sealed snapshot。只有 `LegacyV1` parent/child fleet 才逐个取得 fresh request-scoped retry budget。
- V2 CLI report 的 execution selector 与 runtime terminal truth 对齐：task 运行中必须恰有一个 current-stage active execution；task 已 `finished` 且 active=0 时，选择该 operation/current-stage 最新的 completed execution 生成最终报告。terminal success 不得因为“已完成所以没有 active row”反而退出 1；非 finished、重复 active 或无匹配 completed row 仍 fail closed。
- Fresh V2 `orchestrate` 以 CLI 已解析的真实 workspace canonical path 注册稳定 `project_scope_id`，再把 registration + trusted `CliRuntimeScope` 交给 `TaskOrchestrator::run_stage` 原子创建唯一 operation；exact resume 同样注册 current workspace 并与 frozen operation scope 对比，错 workspace直接拒绝。path 只作 provenance，不能用 basename/字符串猜测 operation ownership。
- runtime/attack deployment singleton 的 forward-only cutover migration 都只推进到 rank 1 的 dual-write/legacy-read sampling；后续相邻 rank 必须由 retained whole-record Candidate cohort gate 晋级，当前不会凭 migration 直接到 `v2_only`。fresh headless operation 冻结创建时 default，exact resume 继续使用旧 operation 自身的双 contract，不因当前 singleton 改变而重绑。Candidate generation/org/runnable set 由 DB Wave authority 决定，CLI objective/`--org` 不能覆盖；`run_tree.py --db` 是核对 rollout、Wave terminal/no-input、Attempt lease 与 FactDelta/residual lineage 的首选诊断入口。
- **resume authority 必须整源选择**：`LegacyV1` / `DualWriteLegacyRead` 只读完整 legacy checkpoint；`DualWriteV2Preferred` 只有在 execution + sealed scope + 全部 Unit/Worker/chain/tool fence 构成完整 relational source 时才整源选 V2，且只在 typed structural-incomplete / chain decode failure 时整源回退 legacy；live lease、DB error、cross-identity/tool fence drift 一律 fail closed。选中的 whole-record source 会显式传入 `TaskOrchestrator`/`DbFlowCheckpointer` 与 request-local `AgentBridge`；后续 graph、worker checkpoint、bound-chain load 全部使用同一 source，preferred-relational 绝不读写 `graph_flow` 或按 worker 重新选择，fallback 才走 legacy。`V2Only` 只接受 relational source，`state_blob` 只有 server-owned namespace 也可恢复。relational/legacy chain 都按 durable specialist → DB `agent_type` 映射（如 `enumerator` / `attack_analyst` / `candidate_verifier` → `pentester`），并真实 serde-decode chain body。
- V2 Scoping 在 Gate BLOCK 后可能处于合法的 pre-freeze shape：active Scoping execution存在，但 `engagement_org_id`、snapshot、Unit、Worker尚未写入。exact resume 只有在 caller 显式给出 `--expect-org`，且 DB 能从同 operation/execution 的 needs-human receipt、唯一 Human candidate response与其后成功 root create重算出同一 org时，才临时把该 root作为 relational resume authority；finalizer成功后仍必须把它写入 operation并封存 scope。缺 expected org、Human/Create witness不完整或 identity drift一律拒绝，不能把任意 CLI UUID当授权。
- **LegacyV1 子公司扇出兼容（2026-06-14 · 方案 C）**：旧 step 6.5 手写 Rust per-child 循环 → `run_fleet_scheduler`；`run_legacy_child_operation_fleet` 与 `OrgFleetExecutor::run_org` 都检查 frozen contract，任一 V2-writing contract 都不会创建 child task/operation。
- **逐子进度 eprintln（2026-06-14 收敛后补回中途可见性）**：调度器（IO-free 内核）新增第 4 个注入 trait `FleetProgress`，CLI 传 `engagement::fleet_run::CliFleetProgress{label:"subsidiary"}` → 每个子公司进 executor 前后打 `[stage-run] ── subsidiary i/N: 名 → running/PASS/BLOCK/FAIL ──`（恢复 T1 把手写循环换成 `run_fleet_scheduler` 后丢的那条逐子可见性）。GUI 单卡路径传 `NoopProgress`（进度走 `StageRunOrgProgress` 事件）。续跑跳过的 org 只 `on_org_done`（SKIP(done)）、不 `on_org_start`。i/N 由调度器静态 org 序提供（checklist 串行下即真实顺序）。
- **session 四身份必须同值**：`initialize_agent(.., &session_id)`（event/evidence 写入）、`set_session_id`（终端）、`set_chat_session_id`（gate/refiner 查账本）、transcript 目录都用同一个 `stage-run-{uuid}`。2026-06-12 前 event 侧残留 `"cli"`，导致 evidence 落账后 gate/refiner 查不到（账本 facts=0、submit-only 锁不可达）。
- **exact resume 不得重跑原命令**：fresh `--stage-run` 每次创建新 chat key/DB
  session/task/operation，旧 `technique_outcomes.run_id` 与 chain scope 都不可见。
  Resume 必须把旧 chat key 同时用于 event/evidence/run_id，把旧 `sessions.id`
  设回 tracker persistence session，并调用旧 task 的 `resume()`；同 stage 入口不会刷新
  `stage_started_at`。
- **fail-closed orphan/claim**：durable claim 只收 `waiting`。残留 `running` 即使带显式
  flag + expected identities 也不能证明跨进程 owner 已死，必须先由 startup reaper 与
  `--repair-reaped-task` 转回 `waiting`。CLI 保留 operation advisory lock 防双 CLI；在
  bridge/provider/project-scope 全部初始化后、真正 orchestrator resume 前，再以单条 SQL
  将 exact task `waiting -> running`，同时 CAS task timestamp、operation contract/profile/
  stage/org/superseded、relational execution id 或完整 legacy blob，并拒绝任一 live Worker
  lease。TaskOrchestrator 消费 one-shot preclaim，不再发第二次无 fence status update。
- **首 stage 无 `graph_flow`**：graph executor 只在 node 返回后写嵌套 checkpoint；
  Ctrl-C 落在首 worker 内时，flat HarnessResumeState + `stage_run_workers` 仍有效但
  普通 `resume()` 不可加载。显式 repair 在 advisory claim 下要求 flat
  blob 可完整反序列化、profile/current_stage/current_stage_run_id 与
  operation/expected ids 全匹配且 `completed_count=0`，CAS `jsonb_set` 新增
  `{state: default, next_node: current_stage}`，并验证所有 sibling 原样保留后再
  resume；已有 `graph_flow.state` 也必须先完整反序列化，不能只凭 JSON object 外形。
- **startup reaper 与 flat checkpoint**：`LegacyV1`/`DualWriteLegacyRead` 的 recoverable predicate 同时接受
  完整 `graph_flow` 或严格 flat first-stage checkpoint（profile/stage/run UUID、
  `completed_count=0`、非空 stage worker map）；后者会被 pause 为 `waiting` 而非
  fail。历史版本若已经写入固定 abandoned failed marker，exact resume 还必须显式
  `--repair-reaped-task`，在 advisory lock 下先 CAS 回 `waiting` 再补 graph；任何
  其它 failed result 都 fail-closed。`DualWriteV2Preferred` 优先选择完整 relational
  shape（只允许整条 legacy fallback），`V2Only` 只读 relational truth：Scoping
  pre-freeze 无 snapshot/unit/worker；post-scope specialist 每 frozen org 一 Unit/Worker；
  non-specialist 仅 root Unit、无 Worker。startup 同一事务先把 expired/no-tool worker
  requeue、expired/active-tool worker 标 `recovery_required`，live lease 保持不动；
  duplicate execution、stale active-tool 或任何 identity/shape 漂移都 fail closed。
- **legacy chain task_id**：exact chain 必须匹配 chain id + DB session + specialist 且
  body 非空；`task_id=Some` 时还必须等于 operation。旧 stage-run chain 可能是
  `task_id=NULL`，由 guarded `operation_state.stage_run_workers` map 绑定 operation，
  可兼容但绝不手工回填；非空错 task 一律拒绝。

## Shared-DB stage fork（2026-07-18）

- `--stage-run-fork <operation UUID>` 最无歧义；chat/session selector 必须只绑定一个 operation。
- Scoping 固定采用源 sealed scope；可只跑 `target_intel` 到 `attack_candidate` 任一阶段，或连续 `--from/--to` 切片。
- target operation 拥有新的 Task/execution/Worker/tool/evidence；前缀只通过 immutable fork input读取，不复制或重跑 source Worker。
- Candidate-only 仍走共享 Candidate Wave/Gate/Review；generation-zero entry 是 exact `ForkedVulnHandoff`，不会放宽普通 Candidate 的同-operation连续性。
- EAS 及以后入口在任务非终态期间冻结创建时 Target identity/scope/source/owner，enrichment列仍可写；缺 in-scope Target 会在模型/扫描器前拒绝。

## 测试入口

```bash
cd backend && cargo nextest run -p golish stage_run
# exact resume 纯测试：cargo test -p golish stage_run::tests::resume_candidate --lib
# 活体：just stage <profile> <to> "<objective>"
# 隔离 DB 活体 smoke：just stage-smoke <profile> <to> "<objective>"
# 更细控制：python3 scripts/stage_smoke.py --fixture-web --provider deepseek --model deepseek-v4-flash --profile assessment --to target_intel --objective "smoke target_intel"
```
