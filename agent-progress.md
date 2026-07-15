# agent-progress.md

> **进度日志**。每轮会话结束前必须更新；每轮新会话开始前必须先读。
>
> 配套文件：`AGENTS.md`（工作宪法）、`feature_list.json`（功能清单）、`clean-state-checklist.md`（收尾检查）。

---

#### 2026-07-16 · 组织级联删除 Memory invalidation dead-letter 修复

- **本轮目标**：修复 Target 页面删除组织时 two-phase job 永久停在
  `waiting_for_invalidation_delivery`，并恢复用户已确认删除的
  `广州有创网络科技有限公司`（org `4faf048b-3625-40cb-a2a1-829c470b151c`，18 targets）。
- **根因**：`knowledge_assertions::invalidate_projection_chain_with_event_with_connection`
  过去用裸 SQL 把 assertion `status` 改为 `expired`、写入 `valid_to`，但没有同步重算覆盖这两个字段的
  canonical `content_hash`。`assertion-promoter` 读回时得到 `Corrupt("content_hash")`，线上 delivery
  重试 5 次进入 `dead_letter(memory_port_failure)`，document/embedding/graph dependency 全部阻塞，删除
  worker因此永远不能 claim artifact cleanup。
- **已完成**：
  - assertion invalidation 现在先按 exact source、稳定 assertion id 顺序 `FOR UPDATE`，把每行还原为
    hash-valid domain object，再由 `KnowledgeAssertionDraft` 计算 lifecycle 变化后的 canonical hash；
    `status`、`valid_to`、`content_hash` 以旧 hash CAS 同步写入，同一事务随后关闭 document/embedding 并
    追加 typed invalidation event。exact no-op replay 不改写行，缺 source / 旧行损坏继续 fail closed。
  - 新增 integration regression，直接证明失效行仍能通过 repo integrity read，并可被 assertion projector
    的 exact event-source selector 读取。RED 证据为 `Corrupt("content_hash")`，修复后转绿。
  - 对线上旧 job `51eeaf9a-3253-4bfa-915c-946f16f85cf3` 做 exact CAS 恢复：先重算并用失效前
    active hash 完全匹配证明 canonical 序列化无漂移，再只更新目标 assertion hash并重排 exact
    `memory_port_failure` dead-letter。四个 projector随后全部终态成功，job 到
    `hard_delete_committed`；DB复核组织不存在且 target count=`0`。
  - 同步更新 `golish-db/repo` 模块卡与 INDEX，明确 lifecycle mutation 必须同步重算 hash。
- **验证证据**（Cargo前均执行 `just space-guard` → exit 0）：
  - RED：`cargo nextest run -p golish-agent-app -E
    'test(source_invalidation_preserves_assertion_hash_integrity_for_the_projector)' --status-level fail`
    → exit 100，1/1 failed，`Corrupt("content_hash")`。
  - GREEN：同命令 → exit 0，1/1 passed，nextest run
    `da8fa997-8588-4ddb-b446-50b56cf6c346`（含 exact invalidation replay）。
  - `cargo nextest run -p golish-agent-app --test knowledge_memory_runtime --status-level fail`
    → exit 0，6/6 passed，run `5402ef70-5313-4722-a102-16903876edb4`。
  - golish-db invalidation + deletion 聚焦 3 tests → exit 0，3/3 passed，run
    `bb283c72-3755-45a4-a463-6d07d20578c8`。
  - `cargo check -p golish-db -p golish-agent-app --tests` → exit 0；
    `cargo clippy -p golish-db -p golish-agent-app --tests -- -D warnings` → exit 0。
- **边界 / 状态**：按用户明确要求未运行 `init.sh`；沿用共享 active feature 边界未跑
  `just precommit`，未改 schema/migration/generated IPC，未 commit/stage/push。该修复作为窄 production
  bugfix，不切换 `feature_list.json` 当前唯一 `in_progress` feature。
- **提交记录**：未 commit、未 stage、未 push。
- **风险 / 下一步最佳动作**：本次线上删除已完成，当前没有需要人工补删的 org/target；后续重新加载
  backend时会使用 hash-consistent invalidation实现。若再出现组织删除超时，先查 exact deletion job 与
  projector delivery truth，不能把前端 10 秒文案当成仍会自动完成的证明。
- **以下文件已修改但未提交**：`backend/crates/golish-db/src/repo/knowledge_assertions.rs`、
  `backend/crates/golish-agent-app/tests/knowledge_memory_runtime.rs`、
  `docs/modules/backend/golish-db/repo.md`、`docs/modules/INDEX.md`、本记录。共享 dirty tree其它既有修改未
  回滚、未接管，也未替它们声明完成。

#### 2026-07-15 · Stage Run 每公司一个 Codex 式 Company Controller（进行中）

- **本轮目标**：按用户最新确认，把 `target_intel` V2 Team从服务器固定六个 Producer、结束后再启动
  Aggregator，改成每家公司一个一开始即可进入的真实 Controller；由 Controller决定调用 0..N 个
  durable SubAgent、等待结果后继续补派，并由 Controller自己提交 deterministic Gate。
- **当前 feature / 边界**：继续唯一 `in_progress`
  `stage-team-scheduler-verification-recovery-2026-07-14`。用户明确允许使用 subagent协作；本轮并行完成
  backend、DB可行性、产品协议、前端 identity/read-model与模块卡收口。沿用既有边界：不执行外部目标、
  commit/push、`init.sh`、`just precommit` 或 broad suites。用户已授权删除旧 fixed Team兼容，但数据库
  schema/migration仍按 AGENTS.md等待单独明确授权。
- **设计决策**：新增
  `docs/design/2026-07-15-stage-run-company-controller-agent.md` 与同名实现计划；2026-07-14 fixed
  sibling + later Aggregator编排形态标为 superseded，但保留 WorkItem/WorkerRun/lease/checkpoint/Gate底座。
  Controller在 DB兼容层同时占用 leader/aggregator/final-submitter角色，产品上不再出现第二个
  Aggregator。Coverage axis只是 Gate obligation，不再是固定 Worker清单。
- **用户补充的队列语义**：若 scope含一个主公司和十个子公司，`stage_run` 是十一个 durable Unit的
  可并发公司队列；每个已领取 Unit再拥有一个 Controller及其动态 child队列。实现必须同时限制公司级
  并发、每公司 child并发和 operation总 live-agent数；一家 waiting/blocked不能卡住其他公司。
- **已完成的运行时替换**：Stage Team policy不再有 fixed-shards分支；`target_intel` 每个 Unit只 seed
  `leader:primary` Company Controller。`stage_run`按冻结 C并发公司 Unit，所有 Controller/child provider
  turn再受 operation级 G semaphore限制，每公司 K（含 Controller）限制 live Worker。Controller通过 exact
  trusted binding独占 `stage_team_dispatch_workers` / `stage_team_prepare_final_submission`；child拥有独立
  WorkItem/WorkerRun/message chain/lease/checkpoint且不能再组队或提交 Unit。派发后同一 Controller进入内部
  `waiting_for_subagents`，scheduler持续 drain/监控 children，满足 barrier后 claim回同一
  WorkerRun/message chain继续决定补派或 final。partial batch中已有 child落库、后续项失败时仍进入 barrier
  并执行已接受 child，不遗留无人处理的依赖。
- **已完成的 Gate/UI替换**：同一 Controller关闭 request epoch、绑定自己为唯一 final submitter并执行
  deterministic Gate；产品中没有晚启动的第二 Aggregator。UI外层为公司队列，详情只提供 exact
  `::lead:<worker>` Controller与 `::worker:<worker>` child树；`waiting_dependency`显示“Controller正在监控
  SubAgent”，事件静默/短暂无工具不解释为暂停。旧 fixed Team没有执行或运行流兼容，只提示重新运行。
  `aggregator_*` / `is_aggregator`只保留为当前 DB/wire字段，不产生产品 Agent。
- **已完成的 Codex Plan对齐**：按用户追加要求和当前 Codex工具合同，exact Controller额外看到同名
  `update_plan`；plan严格为1..12个必填 `{step,status}`，status仅
  `pending|in_progress|completed`且最多一个`in_progress`。普通 child看不到，bound child伪造调用也
  fail closed；非StageTeam unbound orchestrator原有通用plan路径不受影响。Controller复杂首轮、dispatch
  前、child outputs回流后与Gate gap恢复后更新计划；prepare前全部completed。该工具不是scheduler
  barrier，不写主Agent全局PlanManager/`execution_plans`或`PlanUpdated`事件，tool-call/result随exact
  Controller message chain checkpoint持久化。Controller与child仍都使用`recon` executor保留阶段业务工具。
  `SubAgentDetailView`把合法调用渲染为Codex式历史计划快照；任一非法step、超12步或多个in-progress整卡
  回退普通工具展示，计划进度不改变Unit/Gate truth。
- **当前唯一实现阻塞**：Gate BLOCK compound transaction已能保存 gap/checkpoint并恢复同一
  WorkItem/WorkerRun/message chain，但现 trigger禁止 same-epoch reopen；推进 epoch又使原 Controller
  WorkItem不能作为新 child parent，且 gap来源 WorkerRun有唯一约束。因此无 migration版本只能让同一
  Controller恢复后自己补，不能安全追加新 SubAgent。完整 Codex式 repair需要用户明确授权一条向前
  migration：受限 same-epoch Controller reopen、移除 gap来源 WorkerRun唯一约束并保留普通索引、以 gap
  数量约束 fuel。
- **记忆核对**：现有 harness sub-agent在 provider调用前已经按 operation/stage/unit/org/worker读取
  scoped ContextPack，并注入 canonical/runtime/handoff/episode/assertion/document/temporal-graph；失败时
  fail closed，不退回 global/sibling customer memory。每个 Worker已有独立 durable message chain。
  当前 query embedding provider为 `None`，所以 vector similarity层未实际启用；新 Controller必须复用
  同一 scoped入口，不能把“有记忆底座”夸成完全等同 Codex内部实现。
- **运行过的验证 / 已记录证据**（Cargo前 `just space-guard` → exit 0）：
  - `cargo fmt -p golish-agent-kit -p golish-agent-runtime -p golish-db -p golish-agent-app -p golish-sub-agents -- --check` → exit 0。
  - StageSpec聚焦 → 40/40；Company Controller scheduler → 14/14；Lead/dispatch router（含 partial persist）
    → 18/18；Controller host barrier → 1/1。
  - `cargo check -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents --tests`
    → exit 0；只有共享树既有 `stage_coverage.rs::merge_source_query_row` dead-code warning。
  - DB `company_controller_parks_for_dynamic_child_and_resumes_same_worker_chain` → 1/1；
    `company_controller_gate_block_reopens_same_worker_chain_until_repair_fuel_is_exhausted` → 1/1。
  - `StageTeamRunView.test.tsx` 9/9 + `ToolCallDetailView.test.ts` 14/14；四个前端文件 scoped Biome
    → exit 0。
  - Codex Plan聚焦合并验证：`golish-sub-agents` Controller tool visibility/schema 1/1、plan非barrier
    1/1；runtime bound-child拒绝/strict plan/chain-local normalize 3/3、unbound generic route 1/1、
    Company Controller prompt/turn/scheduler 6/6；`cargo check -p golish-sub-agents -p golish-agent-runtime --tests`
    → exit 0。
  - `SubAgentDetailView`计划卡/原有detail suite 58/58；连同StageTeamRunView 9/9、ToolCallDetailView 14/14
    合并为3 files / 81 tests passed；六个前端文件 scoped Biome → exit 0。
- **当前状态 / 下一步**：Controller主循环、Codex Plan、旧执行路径删除、UI与文档已完成聚焦验证。未跑 fresh app/live
  rerun、`init.sh`、`just precommit`或 broad suites，且 repair-child migration等待用户明确授权，因此 feature
  继续 `in_progress`，不能把整个共享 dirty tree宣称为完成。
- **提交记录**：未 commit、未 stage、未 push。
- **以下文件已修改但未提交**：本切片涉及 StageSpec/Target Intel spec、Stage Team scheduler与
  `stage_run`/sub-agent host-tool runtime、runtime-memory Controller compound transaction及其 trait/app bridge与
  DB聚焦测试、Company Controller/SubAgentDetail/ToolCallDetail前端组件与测试、设计/计划、backend/frontend模块卡、INDEX、
  `feature_list.json`和本记录。共享 dirty tree中其它既有修改未回滚、未接管，也未替它们声明完成。


#### 2026-07-15 · Stage Team 运行流 / Gate truth / Aggregator repair 收敛（进行中）

- **本轮目标**：修复 Team 外层 Producer 已显示完成、Agent 运行流却到 Aggregator 才出现，以及
  Producer 自报 checked_empty 冒充 Gate truth、Aggregator 相同 needs_fix 连续报错的问题。
- **当前 feature / 边界**：继续唯一 `in_progress`
  `stage-team-scheduler-verification-recovery-2026-07-14`；不改 DB schema/migration/generated IPC，
  不执行真实目标、外部请求、commit/push。沿用已记录用户边界，不跑 `init.sh`、`just precommit`
  或全量测试，采用聚焦 TDD。
- **设计 / 计划**：新增
  `docs/design/2026-07-15-stage-team-flow-gate-repair-convergence.md` 与同名实现计划；旧设计保留。
- **已完成**：
  - Team UI 将 Producer returned、Aggregator 和最终 Stage Gate 分层；Producer 在 final seal 前只显示
    “已返回，待 Gate”，Aggregator 未创建时不再出现空运行流入口。
  - Producer/Aggregator 使用 exact `::worker:<worker_run_id>` / `::aggregator:<worker_run_id>` Agent
    identity；前端从 active SubAgent 构建 WorkerRun 映射，`::team::` retry仍正确绑定原 stage_run。
  - Target Intel Producer 的 `found/checked_empty` 在 immutable output 前按 exact technique 对齐
    authoritative coverage snapshot；ASN 等不能再用任意 evidence id 自报“查空”。
  - Team Aggregator 的首次 durable accepted/needs_fix submission在完整 checkpoint 后交回 scheduler；
    外层执行既有 Gate/repair generation，不再让同一 Aggregator 连续重复提交到 stall。
  - 架构复核确认当前是 server-seeded fixed six-axis pilot，不是 Codex 式 AI Team Lead；动态 Lead
    可以复用 WorkItem/WorkerRun/lease/checkpoint/Gate 底座，一次规划 MVP无需 DB migration，但未混入
    本 bugfix。
- **运行过的验证 / 已记录证据**（Cargo 前 `just space-guard` → exit 0）：
  - RED：前端聚焦测试最初 6 failed；runtime缺 authoritative snapshot validator；sub-agent缺 durable
    needs_fix barrier。实现后均转绿。
  - `cargo test -p golish-agent-runtime stage_team --lib -- --nocapture` → 17/17 passed。
  - `cargo test -p golish-sub-agents --lib -- --nocapture` → 194/194 passed。
  - 前端聚焦 Vitest 4 files / 36 tests passed；`pnpm exec tsc --noEmit --pretty false`、8-file scoped
    Biome、targeted rustfmt与 scoped `git diff --check` 均 exit 0。
- **提交记录**：未 commit、未 stage、未 push。
- **当前状态 / 风险**：代码级聚焦验证通过；未跑 `init.sh`、`just precommit`、全量测试或 fresh
  application rerun，因此 feature 继续 `in_progress`，不能把整个共享 dirty tree声明为完成。当前仍是
  fixed six-axis Team pilot；若用户确认切换为 Codex 式动态 Lead，应作为下一架构切片推进。
- **下一步最佳动作**：重新加载含本 backend/frontend 的应用后跑一次 fresh Target Intel，用新 session
  的 `run.log`、`transcript.json`、`scripts/run_tree.py --full --db` 验证 Producer flow即时可见、外层只
  显示 returned待 Gate、Aggregator needs_fix进入 fresh repair generation。随后再决定是否启动一次规划型
  Team Lead MVP。
- **以下文件已修改但未提交**：本轮 design/plan；`stage_team_scheduler.rs`、`stage_run_call.rs`、
  `executor_types.rs`、`executor/response_parsing.rs` 及因新增 bound-context 字段同步的构造点；
  `StageTeamRunView`、`StageRunOrgRows`、`ToolCallDetailView`、`harness-handlers` 及聚焦测试；相关
  backend/frontend 模块卡、`docs/modules/INDEX.md`、`feature_list.json` 与本记录。共享 dirty tree 的
  其它既有修改未回滚、未接管，也未替它们声明完成。


#### 2026-07-15 · Stage Team DNS tool-fence 与 SubAgent 运行流修复

- **本轮目标**：修复最新 `target_intel` session 中 DNS 已写入业务事实，却因
  `recon_list_providers` tool-fence finish 遇到 PostgreSQL deadlock 而长期停在
  `recovery_required`；同时修复 sibling producer 并发运行流被合并后出现大量伪
  `Thought for 0.001s` 的问题。
- **当前 feature / 执行边界**：继续处理唯一 `in_progress`
  `stage-team-scheduler-verification-recovery-2026-07-14`。未删除旧组件或历史 runtime rows，未执行
  真实扫描、外部请求、commit/push；按已记录用户边界未运行 `init.sh`、`just precommit` 或全量测试。
- **已完成**：
  - Stage Team 每个 WorkerRun 派生独立 SubAgent UI `parent_request_id`；组织级 pointer 仍只用于 Team
    progress，sibling reasoning/tool timeline 不再互相拼接。
  - sub-agent streaming buffer 保留 reasoning batch 首末到达时间，并在 tool request/result、completed、
    error 前同步 flush；零宽 batch 仅显示 `Thought`，真实小于 100ms 显示 `<0.1s`。
  - `finish_worker_tool` 仅对 SQLSTATE `40P01` / `40001` 做最多三次整事务重试，其他 DB/fence 错误仍
    fail closed。
  - recovery 只自动收敛 exact terminal-failed 本地只读 `recon_list_providers` split state：旧 Worker
    supersede、稳定 WorkItem 重新排队并创建新 attempt；网络/副作用工具仍保持 manual recovery，历史行不删。
- **运行过的验证 / 已记录证据**（Cargo 前 `just space-guard` → exit 0）：
  - 前端 RED 最初 7 failed，分别锁定 `0.001s`、生命周期边界未 flush、batch timing 未进入 store；实现后
    聚焦 3 files / 32 tests passed，扩展 AIChatPanel/event/store suite → 5 files / 64 tests passed。
  - backend RED 先证明 Worker UI identity 与 SQLSTATE 分类 helper 缺失；DB integration 首次运行进一步
    暴露 `recovery_required -> retry_pending` 非法 transition，改为合法 `recovery_required -> queued` 后转绿。
  - `golish-agent-runtime -E 'test(stage_team)'` → 15/15 passed，nextest run
    `de3f9dc4-4866-4812-a6a9-5b30a2baf1a7`；`golish-db -E 'test(stage_team)'` → 13/13 passed，run
    `b61193c1-8d49-430e-b2cc-eb04254e061a`，其中既有 external-tool manual recovery 测试继续通过。
  - scoped Biome、`pnpm typecheck`、`cargo fmt --manifest-path backend/Cargo.toml --all -- --check`、scoped
    `git diff --check` 均 exit 0。full `git diff --check` 只被共享树中既有 generated files
    `GeneratedAiEvent.ts:355,359` / `GeneratedHarnessTraceKind.ts:77,81` 尾随空格阻断；未手改生成文件。
- **提交记录**：未 commit、未 stage、未 push。
- **已知风险 / 未解决问题**：代码级根因已修，但没有用真实目标 fresh rerun，也没有跑全量
  `just precommit`；feature 继续保持 `in_progress`。旧 DNS row 不被原地篡改，只有下一次 producer claim
  才会按窄 allowlist 自动 supersede/requeue。
- **下一步最佳动作**：应用加载新 backend 后，在当前 task 输入一次 `继续跑` 即可；无需新建 task。
  随后用该 session 的 `run.log`、`transcript.json`、`scripts/run_tree.py --full --db` 确认 DNS 新 attempt
  被领取、Barrier 从 5/6 收敛。得到用户允许后再跑 `just precommit`。
- **以下文件已修改但未提交**：`stage_run_call.rs`、`runtime_memory_tx.rs`、`stage_teams.rs`、
  `runtime_memory_worker_transactions.rs`；`ThinkingBlock.tsx` 及新测试、`streaming-buffer.ts`、
  `sub-agent-handlers.ts` 及新测试、workflow store/type/tests；本轮 design/plan、相关 backend/frontend
  模块卡、`docs/modules/INDEX.md`、`feature_list.json` 与本记录。共享工作树其它改动未回滚或接管。

#### 2026-07-15 · Target 删除后页面即时收敛修复

- **本轮目标**：修复 Target 页面单条/批量/组织级删除后仍显示旧目标，且旧的并发
  `target_list` 响应可能覆盖较新删除结果的问题。
- **范围与状态**：这是未列入 `feature_list.json` 的小型前端行为修复；未切换当前唯一
  `in_progress` feature。按用户明确要求未运行 `init.sh`、`just precommit` 或大测试，也未修改
  backend、DB schema、migration、generated types。
- **已完成**：
  - `useTargetData` 为所有列表重载增加单调请求序号，只允许最新请求写 React state；旧轮询、
    旧事件与失败重试不能再把删除行写回来。
  - 单条删除和批量删除在后端确认成功后立即从本地 state 移除成功 ID，再等待 DB 重读并发送
    `targets-changed`；批量部分失败时只移除实际成功的 ID。
  - 组织删除不再只依赖易丢失的 refresh event，也不在 two-phase deletion job 刚接受时立即读取
    旧行；应用内弹窗保持 submitting，轮询 organization read model 直到 root row 确实消失后，才
    重载 targets、关闭弹窗并发送 umbrella refresh hint。
  - 移除 TargetPanel 全部 active 删除入口对 WebView/Tauri 全局 `confirm()` 的依赖；单条、分组和
    组织删除统一使用受控的应用内 Dialog。现场日志已证明旧调用会 rejection，但 Promise truthy
    仍让删除继续，存在确认失败后误删风险。
  - 新增 hook 与交互回归测试，覆盖“删除后的 DB 重读仍 pending 时立即消失”、“删除前旧查询晚
    返回不能恢复已删目标”以及“native confirm 抛错时应用内取消不删除、显式确认才调用后端”；
    同步更新 `frontend/components` 模块卡与索引。
- **验证证据**：RED 时新增测试按预期失败（目标仍保留）；实现后
  `pnpm exec vitest run frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx
  frontend/components/TargetPanel/hooks/useTargetData.test.ts
  frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → exit 0，3 files / 31 tests passed；
  scoped `pnpm exec biome check`、`pnpm exec tsc --noEmit` 与 scoped `git diff --check` 均通过。
- **提交与风险**：未 commit、未 stage、未 push；没有启动应用做手工 UI 点击验收。共享工作树中
  其它大量既有改动未回滚、未接管，也未替它们声明完成。
- **下一步最佳动作**：重新打开当前应用中的 Target 页面，分别点一次单条 Target 删除、分组批量
  删除和组织级联删除，确认行/计数/右侧选中详情立即收敛；代码级回归证据已经覆盖旧响应回写。
- **以下文件已修改但未提交**：`frontend/components/TargetPanel/hooks/useTargetData.ts`、新增
  `useTargetData.test.ts` / `TargetGroupedView.delete.test.tsx`、`TargetPanel.tsx`、
  `TargetGroupedView.tsx`、`OrgTreeSidebar.tsx`、`TargetTreeRow.tsx`、
  `docs/modules/frontend/components.md`、`docs/modules/INDEX.md`、本记录。

#### 2026-07-15 · Stage Team producer BLOCK 修复与前端收口

- **本轮目标**：修复 live session `pentest-chat-1784102030587-1` 暴露的 producer 输出无效即
  永久 BLOCK、同 Unit 重试 `stage_team_unit_not_runnable`，以及 Team/legacy 前端重复渲染和
  内部调试字段默认全展开问题。
- **当前 feature**：按用户最新优先级把
  `stage-team-scheduler-verification-recovery-2026-07-14` 恢复为唯一 `in_progress`；CLI/GUI parity
  保留全部实现和证据并转回 `blocked`，没有回滚其代码。
- **执行边界**：不修改或删除既有 immutable runtime rows，不重跑真实目标，不调用外部服务，
  不 commit/push；沿用本 feature 已记录的用户边界，不跑 `init.sh`、`just precommit` 或大测试，
  只做聚焦 TDD 与静态检查。
- **设计**：新增
  `docs/design/2026-07-15-stage-team-producer-retry-ui-convergence.md` 与配套实现计划。协议/authority
  错误属于可重试 attempt failure；只有合法 business blocker 才成为 immutable output。旧 BLOCK
  rows 保持审计历史，不原地更新或删除。
- **已完成**：
  - producer parser 接受“外层简短说明 + 唯一 fenced JSON object”，多 fence/歧义对象仍 fail
    closed；runtime chain marker 继续只在 UUID 与结构化 chain id 精确匹配时剥离。
  - `producer_completion_from_result` 改为返回 typed violation。执行失败、invalid JSON、无 authority
    的 `found/checked_empty` 和已登记的 `no_registrable_domain` dependency blocker 均复用
    `retry_stage_worker` compound API，在 frozen attempt budget 内重新排队；未知但合法 business
    blocker仍正常落 immutable blocked output。
  - `StageRunOrgRows` 在 exact Team pointer 存在时只渲染 DB-backed Team view；无 pointer 才保留
    legacy org card，未删除旧协议兼容组件。
  - `StageTeamRunView` 默认只显示 producer 的有效/运行/阻塞计数和 DNS/WHOIS/ASN 等业务状态；
    output disposition 优先于 Worker lifecycle，故 `Worker passed + invalid output` 显示“输出无效”。
    Plan/hash/epoch/lease/schema/chain/request 收进“调度详情”，manual recovery 仍保持可见和永不重放。
- **运行过的验证 / 已记录证据**（Cargo 前执行 `just space-guard`，exit 0）：
  - RED：`cargo test -p golish-agent-runtime producer_completion --lib -- --nocapture` 因新增
    `producer_completion_from_result` 尚不存在而按预期编译失败；实现后聚焦模块
    `cargo test -p golish-agent-runtime stage_team_scheduler::tests --lib -- --nocapture` → exit 0，
    11/11 passed，覆盖 fenced object、invalid checked-empty retry、dependency retry 与 terminal blocker。
  - RED：两份前端测试最初 3 failed，分别证明 legacy/Team 重复、缺业务计数、invalid output仍画
    completed；实现后
    `pnpm exec vitest run frontend/components/Engagement/StageRunOrgRows.test.tsx frontend/components/Engagement/StageTeamRunView.test.tsx`
    → exit 0，9/9 passed（另覆盖 mixed/incomplete Team pointer 必须回退 legacy）。
  - `pnpm exec biome check`（上述 4 个组件/测试文件）→ exit 0；`pnpm exec tsc --noEmit` → exit 0；
    `cargo check -p golish-agent-runtime` → exit 0；targeted
    `rustfmt --edition 2021 --check` 最初仅报告一处换行，修正后复跑 exit 0；`jq empty
    feature_list.json` exit 0且唯一 `in_progress` id 正确。
- **提交记录**：未 commit、未 stage、未 push；未删组件/文件，未修改 migration/DB schema，未执行
  真实目标或外部服务。
- **已知风险 / 未解决问题**：旧 session 已写入的 immutable invalid outputs 和 `gate_blocked` Unit
  不会被本修复篡改；必须新建 Stage execution 才能验证新 retry 路径。未跑 `init.sh`、
  `just precommit`、全量 Rust/前端测试或 live provider/Tauri→PG 验收，因此 feature 保持
  `in_progress`，不能宣称整个共享 dirty tree 已完成。
- **下一步最佳动作**：用户允许后，重新打开应用并对离线/授权 fixture 发起 fresh Target Intel
  Stage execution；用 `run.log`、`transcript.json`、`scripts/run_tree.py --full --db` 证明 invalid
  first attempt → queued retry → valid immutable output → Aggregator/Gate 收敛。随后再跑 `just precommit`。
- **以下文件已修改但未提交**：本轮两份 design/plan、runtime 的
  `stage_team_scheduler.rs` / `stage_run_call.rs`、`StageRunOrgRows` / `StageTeamRunView` 及测试、相关
  backend/frontend 模块卡、`docs/modules/INDEX.md`、`feature_list.json` 与本记录。共享工作树其他既有
  修改未回滚、未接管，也未替它们声明完成。

#### 2026-07-14 · Stage Team Scheduler + Candidate→Verification 一次性实现

- **本轮目标**：在已经评审通过的两份 2026-07-14 设计上直接实现：把 `stage_run` 从串行
  单 Worker 执行升级为服务端持久化的 sibling multi-agent Team Scheduler；把 Candidate
  review 后的 Verification 改为带 TerminalIntent、checkpoint barrier、确定性恢复、审批
  start-before 与版本化 executor contract 的 exact CandidateAttempt 闭环。
- **当前 feature**：新增并切换唯一 `in_progress` 为
  `stage-team-scheduler-verification-recovery-2026-07-14`；原 CLI/GUI company closure 因缺少
  exact authorized target 且用户切换优先级，转为 `blocked`，没有丢弃既有实现和 evidence。
- **执行边界**：用户已明确要求“两份全部实现”，本轮据此允许 additive migration/DB schema；
  不做 destructive migration，不执行外部扫描或真实目标动作，不 commit、不 push。用户随后
  明确要求先写代码，禁止继续 `init.sh` 和大测试；因此只跑与当前切片直接相关的小型 TDD
  测试。此前一次 `init.sh` 已在 fmt/check-fe/test-fe/lint-rust 通过、进入 Rust 全量测试后被
  按用户指令立即中止，exit 130；它不是本功能完成证据。
- **已完成 · Stage Team Scheduler**：
  - additive `20260714000003_stage_team_scheduler.sql` 已落 TeamPlan、WorkItem/dependency、
    WorkerOutput、WorkerRequest、barrier、unit gap、repair generation 与 operator recovery decision；
    owner tuple、immutable row、epoch/manifest/lease/checkpoint CAS 均由 DB 约束。
  - V2-only `stage_run` 已接 durable sibling queue：Main Agent 只调度；每个 producer/helper 是独立
    WorkerRun + chain + lease；全局/每 Team 有界并发和 org round-robin；nested sub-agent fence保留；
    dynamic request 只入队，不在父 Worker 栈中直接运行。
  - `target_intel` 是首个启用 pilot。唯一 Aggregator 才能提交/finalize；每次只提交一次、评估
    一次 Gate。BLOCK 会终结旧 Aggregator 为 `gate_blocked`，原子创建 bounded repair producer +
    新 Aggregator，同次 `stage_run` 继续收敛；repair fuel耗尽显式返回
    `STAGE_TEAM_REPAIR_FUEL_EXHAUSTED`。worker lifetime budget 已预留两代 repair attempts。
  - startup reaper 与 LocalDesktop operator recovery 使用 exact row/checkpoint/active-tool CAS；未知
    外部工具只可关闭为 blocked outcome-unknown，永不重放。Tauri read/recovery command、ts-rs API、
    `StageTeamRunView` 与 loading/error/empty/mutation 状态已接通；IPC不暴露 lease secret、raw
    checkpoint、工具参数/结果或 dynamic request body。
- **已完成 · Candidate→Verification**：
  - additive `20260714000002_candidate_verification_recovery.sql` 已落 approval `start_before`、
    authorization receipt、TerminalIntent/barrier/receipt、recovery case/decision；plan hash冻结
    recipe/executor contract version。action 未开始而审批过期会回 review；action 已开始/结束后允许
    finish/submit/checkpoint/terminalize，不会因 expiry 形成永久 blocker。
  - `submit_candidate_attempt` 只写 immutable TerminalIntent；active tool清空后 checkpoint exact
    chain/barrier，再由 server terminalizer 单事务写 Attempt/Candidate、Finding/lineage、FactDelta、
    outbox并释放 Worker/lane。action 已 terminal 但尚未提交时，只在同一 Attempt/Worker/chain进入
    submit-only continuation，runtime tool boundary拒绝再次执行 action。
  - additive `20260714000004_candidate_fact_delta_follow_on.sql` 与 consolidation route严格分离
    `delta_kind / observation_kind / allowed_techniques / enrichment_required`：exact Nuclei/anonymous
    typed evidence可 direct follow-on；`refuted` 只形成 no-attack；recognized unsupported adapter
    整个事务回滚；信息不足只落 immutable pending enrichment，source Wave保持 open、delta不消费、
    不创建 target Wave/Seed/WorkItem。orchestrator显式 BLOCK
    `ATTACK_FACT_DELTA_ENRICHMENT_REQUIRED`，Verification queue/UI只读展示安全 subject/reason/
    allowed techniques。当前版本有意没有自动 enrichment executor，未伪装已完成。
- **聚焦验证 / 已记录证据**（每次 Cargo 前均执行 `just space-guard`）：
  - Stage Team foundation：RED 3条 run `a7cfb7b4-6b96-4e20-bc62-973fbff5ab06`；expiry/reaper
    GREEN 3/3 run `f3bf7459-5243-4fda-92d3-b54aec47b879`；migration fixture run
    `4c7f012b-a360-4199-a254-163fb9519c5e`；WorkerRequest semantic replay 1/1 run
    `8181b634-c982-45ee-b43a-13d571d1f7da`。
  - Gate repair pure/runtime 2/2 run `bb422e47-e984-4c31-a4da-95eb1fe91742`；新增 DB exact
    repair epoch/current-Aggregator测试先因 fixture failure payload不符合 closed schema RED
    (`16d95063-8c26-4e3e-96c9-2bc6ca20ce90`)，修正后 1/1 GREEN run
    `40955d8b-61cd-4c1f-b120-552815b5bdc8`。Target Intel plan/lifetime budget 1/1。
  - operator recovery DB exact CAS/cross-scope FK/immutable replay 1/1 run
    `d468653c-88b8-4fd1-a90c-37b998eb8877`；app stage-team tests 15/15；StageTeam UI 3/3；
    relevant Rust library checks通过，仅有既存 `stage_coverage.rs` dead-code warning。
  - Candidate TerminalIntent：P0 crash window 1/1 run
    `149d2cff-8420-4ed7-8be0-d95fdc98c89e`；checkpoint exact replay/drift 1/1 run
    `01f38f64-fde9-41c7-8406-1424c2765ee9`；submit-only host guard 1/1 run
    `e079b678-172c-4e3c-b559-6085dd204e1b`；expiry submit-only 1/1 run
    `76872e1c-ea69-47cf-a465-cf9eceabf792`；其余三个 crash boundaries 3/3 run
    `6a7dd812-a108-4ffe-873f-3d6d78a239ad`。
  - FactDelta 精确命令各 1/1：`sibling_or_stale_canonical_ref_delta_is_rejected`（direct +
    refuted）、`pending_fact_delta_enrichment_is_stable_and_does_not_advance_wave`、
    `recognized_unsupported_fact_delta_route_rolls_back_atomically`、
    `fact_delta_seed_rejects_cross_owner_binding`；orchestrator pending BLOCK 1/1；两条 ts-rs export
    各 1/1；CandidateVerificationProtocol 3/3、CandidateAttemptRows 5/5、scoped Biome通过。
  - `jq empty feature_list.json`、本功能 scoped `git diff --check` 通过。没有把 Codex PTY session id
    冒充 Golish runtime Run ID；本轮没有启动真实 Golish application run。
- **当前状态 / completion gate**：代码切片和聚焦验证已收口，但 feature 继续保持唯一
  `in_progress`。原因是用户明确要求本轮不跑 `init.sh`、`just precommit` 或大测试，且没有做
  live provider/restart/Tauri→PG acceptance；Phase 3–5 的 EAS/Enumeration/Vuln/Candidate Team
  rollout仍按设计 gate，Verification继续使用专用 CandidateAttempt scheduler。全工作树
  `git diff --check` 仅报告 ts-rs 自动生成的 `GeneratedAiEvent.ts` /
  `GeneratedHarnessTraceKind.ts` 四处行尾空格；Biome配置明确忽略 generated 目录，未手改生成物；
  排除这两份生成器输出后全工作树 diff check exit 0。
- **提交记录**：未 commit、未 stage、未 push；未发起真实扫描、外部 API、付费服务或目标动作。
- **已知风险 / 未解决问题**：没有 fresh `init.sh` / `just precommit` / 全量 Clippy 与真实
  provider+DB restart acceptance，故不能证明整个共享脏树无回归；EAS/Enumeration/Vuln/
  Candidate Team 尚未解除 rollout gate；pending FactDelta enrichment 目前需要未来的 typed
  executor设计，不能由操作员按钮或 generic AI 临时绕过。
- **下一步最佳动作**：用户允许较慢验证后，先跑 `just space-guard` + `just precommit`，再用
  离线/授权测试 workspace 做一次 Target Intel V2 Team 的 process-restart acceptance，以
  `run.log`、`transcript.json`、`scripts/run_tree.py --full --db` 和 Team/Worker/Gap DB rows证明
  queue→Aggregator→Gate repair/recovery；通过后才按 Phase 3依次为 EAS/Enumeration增加 exact
  target/origin batch policy并保持 K=1。FactDelta enrichment executor应作为独立 additive
  feature，以 typed result table消费当前 pending authority，不能在本 feature 里假装完成。
- **以下本功能文件已修改但未提交**：三份 additive migrations（`20260714000002`–`00004`）、
  Stage Team/Candidate recovery/FactDelta 的 kit/db/runtime/app/sub-agent repos与测试、
  `resources/harness/stages/target_intel/spec.json`、Stage Team/Verification 前端 API/generated types/
  Engagement components及测试、两份 design、两份 implementation plan、相关模块卡、
  `feature_list.json` 与本记录。共享工作树仍有其他 feature 的大量既有改动/删除，本轮未回滚、
  接管或替它们声明完成。

#### 2026-07-14 · Stage Run Team Scheduler 与 Candidate→Verification 设计审计

- **本轮目标**：按用户要求只做设计，基于当前 checkout 仔细审计两条实际链路：
  1) `stage_run` 与现有 sub-agent/Worker/Unit/lease/chain 的边界，设计可排队、可恢复的多 Agent
  协同；2) `attack_candidate` 进入 durable review 后，Verification 如何逐 CandidateAttempt
  确定性验证、恢复和收口。
- **已完成**：新增
  `docs/design/2026-07-14-stage-run-multi-agent-team-scheduler.md`，明确 harness 外层保留一个
  Main Agent，`stage_run` 升级为 durable Team Scheduler；每个协作 Agent 必须是独立 sibling
  WorkerRun，不能共用 prebound Worker lease/chain。设计补齐 TeamPlan、durable WorkItem、
  WorkerOutput、worker-only completion、manifest closure fence、唯一 Aggregator/finalizer、
  sibling barrier、限流、公平、恢复、取消、UI、迁移和验收合同。
- **文档历史**：在 `docs/design/2026-06-13-stage-run-fanout-design.md` 头部补充部分取代说明；
  旧文件继续保留 chat/UI/fan-out 决策历史，但不再作为共享 lease 嵌套 sub-agent 的实现依据。
- **已完成**：新增
  `docs/design/2026-07-14-candidate-to-verification-execution.md`，明确 Candidate 是对前序
  canonical facts/typed observations/evidence 与 scoped provenance context 的综合推理，不是
  再扫描；Verification 保持一 CandidateAttempt 一 verifier。设计识别并收口当前
  submitted crash window、outcome_unknown 无 operator 闭环、approval expiry 永久 blocker、
  generic legacy recipe 证据不足、FactDelta technique 错配和 V2 stage dependency drift。
  交叉复审后采用 `TerminalIntent → finish active tool → checkpoint barrier → recoverable server
  terminalizer`，避免在 submit 工具内提前释放 Worker/lane；同时把
  `recipe_version/executor_contract_version` 纳入 plan/approval，禁止同 plan hash 静默换执行语义。
- **运行过的验证**：本轮是文档设计，没有改 runtime/schema/IPC，未运行 `init.sh`、Cargo、
  前端测试或 `just precommit`。执行了两份新文档的 trailing-whitespace 搜索、Markdown code
  fence 计数与 heading 结构检查；code fence 分别为 30/60，均为偶数；`jq` 确认
  `feature_list.json` 仍只有
  `cli-gui-operation-parity-company-closure-2026-07-14` 一个 `in_progress`。trailing-whitespace
  `rg` 无匹配（预期 exit 1）；scoped `git diff --check` exit 0；scoped status 显示本轮两份新
  design 文档为 untracked，旧 fan-out 设计头部与 `agent-progress.md` 为 modified。
- **已记录证据**：两位独立只读审计分别复核 stage-run ownership 与
  Candidate→Verification 当前代码；第三次交叉复审发现并促成四项修正：Aggregator 必须在
  finalization 事务中与 Unit/handoff 一起关闭；dynamic WorkerRequest 聚合前必须关闭 manifest
  epoch；submit/terminalize 必须尊重 active-tool/checkpoint 生命周期；legacy adapter cutover
  必须先版本化执行合同。文档内已列当前代码路径、失败矩阵和 RED-first 验收测试。
- **提交记录**：未 commit、未 stage、未 push；未修改任何 migration/schema、运行时代码、
  feature 状态或模块卡；未发起外部 API、扫描或真实目标请求。
- **已知风险 / 未解决问题**：两份文件是 Proposed 设计，不代表实现或 live acceptance；
  TeamPlan/WorkItem/TerminalIntent 的物理 schema、cutover migration 和 IPC 需要后续实现计划，
  且任何 DB schema/migration 变更必须先取得用户确认。共享工作树已有大量其他未提交改动，
  本轮未回滚、接管或替它们做完成声明。
- **下一步最佳动作**：先由用户评审两份设计边界。若认可，分别创建实现计划；执行优先级
  建议先做 Candidate/Verification Phase 1 的 TerminalIntent/recovery/approval-expiry/
  follow-on-Wave fail-closed 正确性闭环，再做 Stage Team Scheduler Phase 1 的 team-of-one
  兼容模型与 Worker/Unit lifecycle 解耦，之后才开启多 Worker 或 Verification 并发。
- **以下文件已修改但未提交**：
  `docs/design/2026-07-14-stage-run-multi-agent-team-scheduler.md`、
  `docs/design/2026-07-14-candidate-to-verification-execution.md`、
  `docs/design/2026-06-13-stage-run-fanout-design.md`、`agent-progress.md`。

#### 2026-07-14 · CLI Scoping → Attack Candidate live acceptance

- **本轮目标**：按用户要求，从 headless CLI 的 `scoping` 开始，连续运行到
  `attack_candidate`，以 fresh `run.log`、transcript、`run_tree.py --full --db` 与持久化
  DB truth 检查各阶段是否真实闭合，并整理需要修改的问题。本轮先诊断，不预设代码改动。
- **执行边界**：保持现有唯一 `in_progress`
  `target-surface-fingerprint-network-failure-closure-2026-07-12` 不变；不扩展既有授权目标、
  不运行 `verification`/exploit，不修改 schema/migration，不 push。启动前先按 AGENTS.md
  完成上下文、模块卡、`just space-guard` 与 `./init.sh` 检查。
- **当前状态**：正在执行；CLI session、阶段结果、验证证据、发现、风险与下一步将在本轮收尾补齐。

#### 2026-07-14 · JS/API contextual resolution v1 实现与验证完成

- **本轮目标**：保留 browser collector、deterministic broad capture 与 AI opt-in 边界，
  在 raw JS/API candidate 和 `api_endpoints` 之间实现确定性上下文解析，解决多 axios
  client、局部 baseURL、相对 path、歧义证据和 resolved URL 后去重。按用户要求未运行
  `./init.sh`。
- **已完成实现**：
  - `golish-js-analyzer` 新增 additive `EndpointCandidate`/`CallSiteContext`/`SourceSpan`
    API，保留 occurrence、callee、完整 receiver 与 byte span；context-only relative custom
    client 及 member/optional chain 只进入 candidate API，legacy `Endpoint` 行为保持兼容。
  - `js_extract_apis` 建同文件 client/base index，支持 `axios.create({baseURL})`、
    `defaults.baseURL`、静态 literal 与 alias；lexical scope、source-order、mutable/reassigned、
    duplicate/spread 对象语义均 fail closed。命名 Axios 使用真实 combine 语义；fetch/
    Request/jQuery 固定 origin-root；无 exact binding 的 member chain 保留 unresolved evidence。
  - deterministic/HAE/AI supplemental candidate 全部先 contextual resolve，再按
    `(method,resolved URL)` 去重并经过 HTTP(S)+exact-origin classifier。raw
    `contextual_resolution_v1` 保存带 fingerprint 的 candidate/binding/disposition，数组和
    字符串都有上限及 total/omitted；dry-run 有 resolved projection 时不再误报 empty。
- **运行过的验证 / 已记录证据**（Cargo 前均执行 `just space-guard`）：
  - TDD RED：新增 analyzer 测试先因 `extract_candidates_from_source/files` 不存在报 E0425；
    实现后 nextest run `8ebf1c3f-af80-46d7-9a6f-c3e4cfe4ab62`，59/59 passed。
  - 最终 bridge 全聚焦 nextest run `40f858ce-7009-4a08-bb00-cb653017e00e`，77/77
    passed，355 skipped；覆盖多 client、relative、Axios combine、scope/time-order、
    duplicate/spread、fetch/global 隔离、member/optional chain、exact-origin、raw bounds、
    supplemental post-resolution dedupe 与 dry-run outcome。
  - `cargo fmt -p golish-js-analyzer -p golish-pentest-app --check` exit 0；targeted
    all-targets Clippy `-D warnings` exit 0；`git diff --check` exit 0。
  - `just precommit` exit 0，最终输出 `✓ All checks passed!`；fmt、check-fe、test-fe、
    lint-rust、test-rust-all、check-types 及最终 `test` 依赖均通过。一次 space guard 在
    79GB free 时按仓库策略回收 78.35GiB 旧 Cargo 产物，回到 160GB；未用 `cargo clean`。
- **提交记录**：未 commit、未 push；未执行外部 HTTP/扫描、AI/LLM、付费 API 或 Test1
  live rerun。
- **以下本功能文件已修改但未提交**：
  `backend/crates/golish-js-analyzer/src/{lib.rs,lib_tests.rs,patterns.rs}`、
  `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`、
  `docs/design/2026-07-14-js-api-contextual-resolution.md`、
  `docs/superpowers/plans/2026-07-14-js-api-contextual-resolution.md`、
  `docs/modules/backend/golish-js-analyzer.md`、
  `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`、
  `feature_list.json`、`agent-progress.md`。共享 working tree 另有其他 feature 的大量既有
  未提交改动，本轮未回滚或接管。
- **已知风险 / 未解决问题**：v1 有意不做跨 chunk import/export、递归 wrapper/data-flow、
  登录/点击/Service Worker 闭包，也未解析 JSON/FormData body params；本轮证明代码与仓库
  门禁通过，不代表 2026-07-12 Test1 真实站点闭环已 fresh 重跑。未做 `just dev` 启动烟测，
  因本轮未改 Tauri command/IPC 且用户明确禁止 init；全量 precommit 已通过。
- **feature / clean-state**：`js-api-contextual-resolution-v1-2026-07-14` 已转 `passing` 并
  填入 evidence；当前唯一 `in_progress` 仍是共享树的
  `target-surface-fingerprint-network-failure-closure-2026-07-12`，未抢占其 active slot。
- **下一步最佳动作**：若要把“真实站点闭环”也证明掉，在明确授权的 Test1 scope 下启动
  fresh producer operation，依次跑 browser collect + `js_extract_apis`，再用
  `scripts/run_tree.py --full --db`、run.log/transcript 和 `api_endpoints`/raw analysis DB truth
  核对多 prefix、unresolved 与 outcome；若只需交付代码，则先从共享脏树中审阅并提交本功能
  文件，不把其他 active feature 混入同一 commit。

#### 2026-07-14 · Vuln → Candidate exact Verification 全闭环续做

- **本轮目标**：按用户明确要求完成 evidence operation 身份修复、Candidate 精确
  Nuclei/匿名重放、旧 `auth_probe` 删除、仓库门禁，以及“广州有创网络科技有限公司”
  的 Scoping→Vuln→Candidate→Verification CLI acceptance。按用户要求不运行 `init.sh`。
- **feature 状态**：本功能代码已实现并完成仓库门禁；共享工作树当前由
  `target-surface-fingerprint-network-failure-closure-2026-07-12` 保持唯一 `in_progress`，因此本功能
  继续标 `blocked`，避免与并行 JS/API 工作争抢唯一 active slot。剩余 blocker 仅是 CLI acceptance
  缺少明确授权的 domain/IP/CIDR/URL，不是代码回滚。
- **已完成实现**：
  - `BridgeEvidenceInput` 接受 trusted operation id；Vuln Nuclei、匿名观察和 Candidate
    Verification 都写真实 harness operation，而不是 chat session 派生 UUID。
  - classifier 对 `nuclei_match_v1` 生成 exact template-id + matched-URL replay，对
    `anonymous_access_v1` 生成 exact endpoint row/request plan/method/path/query replay；未知
    schema、observation hash、target/technique 漂移 fail closed。
  - verifier 保持 ordinal-only，Nuclei 不允许 tag fallback；匿名 replay 重载 endpoint 并在
    network 前逐项比较 row hash、plan hash、method/path/query/no-auth。两者都生成 typed
    `proof|refutation|blocker` evidence；begin 后所有路径都会 terminalize action journal。
  - CandidateAttempt submission 对 action journal、`command.evidence`、result evidence arrays 做
    evidence id/role 三方全等校验。删除旧 `golish-auth-probe` crate、bridge、依赖、注册、策略、
    prompt/taxonomy/ownership 引用和 legacy-only Vault helper。
  - 新增用户已明确授权的 additive migration
    `20260714000001_candidate_observation_shadow_hash.sql`：不新增表/列，只替换 DB shadow rebuild
    function，让数据库和 Rust canonical manifest 同时覆盖 `observation + observation_hash`；历史
    migration 未改写。
- **TDD / 已记录证据**（所有 Cargo build/test 前均运行 `just space-guard`）：
  - `bridge_evidence_uses_the_real_harness_operation_id`：先以派生 UUID 失败（exit 101），
    修复后 1/1 passed。
  - `nuclei_replay_uses_the_exact_frozen_template_and_url`：旧 verifier 缺 canonical target 的
    RED 后，exact plan 1/1 passed；`frozen_anonymous_replay_rejects_identity_endpoint_or_query_drift`
    1/1 passed。
  - pentest exact/anonymous focused：nextest run `c88debce-7e77-41e5-80e1-4e62148afdbf`，
    13/13 passed。
  - DB exact evidence + classifier/attack/final-seal regression：run
    `e03056ef-bbf2-44cd-a6d1-d63c7ce08cb0`，45/45 passed。
  - 六核心包完整定向：run `7e14cbe1-ddd3-487f-9f62-d6480658f077`，
    2407 passed / 4 skipped / 0 failed。
  - 六核心包 `cargo clippy ... --all-targets -- -D warnings` 首轮暴露 JS 路径拼接的两处
    `obfuscated_if_else`；改为明确 `if/else` 后重跑 exit 0。`git diff --check` exit 0；
    运行代码/前端/resources/scripts 对 `auth_probe` 的 `rg` 结果为空。
  - 首轮 workspace gate 暴露 `attack shadow source manifest hash drift`：Rust manifest 已冻结
    observation，但旧 DB rebuild function 仍使用旧投影。新增 additive migration 后，
    `attack_execution_v2_migrations` run `900b9e83-30cd-47d9-b7d6-dd98b1a5373d` 56/56 passed；
    第二组共享 fixture 修复后，`attack_rollout_cohort_migrations` run
    `b68954f5-347d-4b2e-818f-95d45177061e` 13/13 passed。
  - 并行 JS/API 改动的 full gate 编译暴露 SHA-256 display 与 base-resolution 回归；修复后
    contextual run `a56c5981-8266-46fb-a7be-9f51434a220e` 13/13 passed，
    `cargo clippy -p golish-pentest-app --all-targets -- -D warnings` exit 0。
  - 最终 `just precommit` exit 0，依次通过 fmt、check-fe、test-fe、workspace
    `lint-rust`、`test-rust-all`、`check-types`，最终输出 `✓ All checks passed!`；全量 Rust
    测试阶段无失败。收尾 `git diff --check`、`jq empty feature_list.json`、57-crate DAG、
    `auth_probe` 零残留检查均 exit 0。
  - 最终二进制 `./backend/target/debug/golish --version` → `golish 0.2.43`、exit 0；端口
    1420 无监听残留。该二进制 mtime 晚于新 migration，但尚未在默认持久 DB 启动，因此
    `_sqlx_migrations` 中 20260714000001 仍为空，等待真实 CLI 启动时正常应用。
- **init 说明**：本轮早期启动过一次 `./init.sh`，在用户说“不要跑init”后立即 SIGINT；
  中止前 fmt/check-fe/test-fe 已通过，exit 130 是用户中止而非基线失败。之后不再运行 init。
- **当前 blocker / 风险**：默认持久 DB 没有“广州有创网络科技有限公司”的 org/target。公开资料只能
  确认主体存在；`huanyou7.com` 属于关联运营方，不能擅自升级为该公司的授权 target。必须由用户
  提供明确授权的 domain/IP/CIDR/URL 后，才能安全执行 Scoping→Verification 并审计
  `run.log`、`transcript.json`、`run_tree.py --full --db` 与 DB rows。未获得该输入前不伪造 live
  acceptance，feature 保持 `blocked`。
- **下一步最佳动作**：收到 target 后，先用最终源码构建的 `golish` 在 Test1 持久 DB 运行
  `--profile pentest --from scoping --to verification --org ... --target ...`；锁定打印的
  `stage-run-*` session，逐项核对 operation/scope/evidence/CandidateAttempt/Verification truth。
- **提交状态 / 未提交范围**：未 commit、未 push。本轮 evidence/exact replay、Candidate DB
  constraints、additive migration、legacy `auth_probe` 删除、测试、设计/计划、模块卡、
  `agent-progress.md` 与 `feature_list.json` 均在共享工作树中未提交；并行功能既有改动未回退。

#### 2026-07-14 · JS/API contextual resolution v1 实现启动

- **本轮目标**：在 raw JS call-site 与 `api_endpoints` 之间增加确定性上下文解析层，
  先解决同文件多 axios client、各自 `baseURL`、一步常量/alias 与 resolved URL 后去重；
  AI 保持默认关闭，歧义继续作为 raw evidence。
- **根因结论**：当前不只是 base detector 规则少；`js_extract_apis` 还会在解析前按
  `(method, raw_path)` 去重，所以两个 client 的同名 leaf 会先丢一条。analyzer 已捕获
  generic client receiver，但构造 `Endpoint` 时丢弃，现有 file+line 也无法区分 minified
  同行调用。
- **设计 / 计划**：新增
  `docs/design/2026-07-14-js-api-contextual-resolution.md` 与
  `docs/superpowers/plans/2026-07-14-js-api-contextual-resolution.md`。
- **feature 状态**：`js-api-contextual-resolution-v1-2026-07-14` 是唯一
  `in_progress`；原 `vuln-observation-candidate-closure-2026-07-14` 保留全部未提交实现并
  转 `blocked`，仅表示用户优先级切换，不是回滚。
- **边界**：不改 DB schema/migration、generated IPC、`route_probe_paths` 或
  `browser_collect_js_api`；不发外部请求、不启用 AI。按用户要求不运行 `./init.sh`；
  Rust 命令前仍执行 `just space-guard`。
- **验证状态**：尚未写生产代码；下一步按 TDD 先补 analyzer candidate API 红测。

#### 2026-07-14 · Vuln observation → Candidate → Verification 闭环实现启动

- **本轮目标**：实现三个可扩展 AI capability：通用 Nuclei、指纹选模板的定向
  Nuclei、基于 Enumeration 已持久化 JS/API endpoint 的匿名 GET/HEAD 访问检查；三者
  只写 typed observation/evidence/outcome。然后让 Candidate AI 读到冻结 observation、上游
  target data 和 scoped ContextPack，Verification 按精确 template/request 重放。
- **启动前审计结论**：
  - 旧 `vuln_run_formulaic_sweep` 使用未绑 target guard 的 background runner，Nuclei
    malformed JSON 会被静默跳过，存在 unknown 被误记 empty 的 I8 风险。
  - fingerprint→PoC→targeted Nuclei 仅存在 GUI/legacy 路径，并会直写 Finding，
    未连到 AI stage。
  - 旧 `auth_probe` 会猜路径 id、允许 OPTIONS、未绑 exact-origin redirect、无界读 body
    并直写 Finding，不作为新 stage capability 复用。
  - Candidate V2 已有 observation JSONB/final Gate/approval/Attempt/terminalizer，但当前
    manifest 没投影 observation，analyst objective 也没有 exact manifest。
  - Memory/RAG/KG/ContextPack 底座已实现，但 bound Worker 的 exact unit identity 没透传给
    retrieval subject，Candidate worker 实际可能拿不到 pack。
- **已写文档**：
  - `docs/design/2026-07-14-vuln-observation-candidate-closure.md`
  - `docs/superpowers/plans/2026-07-14-vuln-observation-candidate-closure.md`
- **feature 状态**：新增 `vuln-observation-candidate-closure-2026-07-14` 并设为唯一
  `in_progress`。`runtime-memory-candidate-pipeline-v2-2026-07-12` 实现不回退，因仍等待
  明确授权的 live acceptance 而转 `blocked`。
- **验证状态**：`just space-guard` exit 0。`./init.sh` 开始后按用户指令立即中止，
  中止前 fmt/check-fe 通过，不将 SIGINT/exit 130 写成基线失败，后续不再重复 init。
- **安全边界**：本轮不改 migration/schema，不发起真实外部扫描/LLM/embedding/
  Graphiti/API 请求；只用 pure/fake/loopback 测试。

#### 2026-07-14 · Runtime Memory / Candidate Pipeline V2 full implementation closure

- **本轮目标**：在 checkpoint `13b29628` 上完成已授权的 Runtime Memory whole-record resume、
  Candidate Task 10–12、typed Verification / FactDelta follow-on Wave、shadow cohort promotion、fuel、
  Post-Exploit hash bridge 与全链路收口；跑完仓库级门禁、启动烟测并形成一个本地 commit，不 push，
  不发起真实扫描、exploit、LLM、embedding、Graphiti 或其他外部请求。
- **已完成实现**：
  - Runtime Memory resume 先由 trusted preflight 选择一个完整 `Legacy|V2|LegacyFallback` source，
    再以 exact source token 原子 CAS `waiting -> running`；source 固定在 top-level request，
    Bridge→runtime→stage worker/chain/checkpoint 全程原样传播，worker 不得逐次重选或 field-merge。
  - additive `20260712000012`–`00018` 完成 FactDelta→follow-on Wave authority、Candidate shadow
    attestation、typed Verification seal、operation-wide fuel reservation、attack retained cohort、runtime
    retained cohort/attestation，以及 Candidate tagged SHA-256→Post-Exploit bare digest 的 exact bridge。
    corrected `00002/00005` fresh default 只到 rank 1；rank 2/3 必须由真实 retained cohort 与 DB-owned
    receipt 晋级，不能靠 migration/空分母直跳。
  - Candidate approval/Attempt/Worker/lane、terminal Finding/lineage、typed Verification handoff、
    accepted/rejected FactDelta consolidation、residual risk、next Wave 与 Candidate→Memory/KG/RAG→
    Post-Exploit→Cleanup→Reporting replay/retention 路径均接入 exact DB truth；前端新增两种 HarnessTrace
    事件处理与 generated bindings。
  - 全量回归暴露的三个真实边界已按 fail-closed 语义修复：Reporting target FK-null 不再被 fuel trigger
    误判为 business mutation；Candidate `sha256:<hex>` 与既有 Foothold bare digest 在 trusted seam 等价；
    rollout-default fixtures 按当前 retained-cohort 合同定位。最后的 startup reaper 测试改用真实
    `recon` claim+bound chain，而不是裸 SQL 伪造 unknown specialist/running worker；生产 whole-record
    谓词没有放宽。
- **运行过的验证**（所有 Cargo build/test 前均运行 `just space-guard`）：
  - focused：golish-db 42/42（run `b245f728`）、golish-agent-kit 70/70（`8874d287`）、
    golish-agent-runtime 81/81（`77468236-f333-4a2b-b9b2-c14c66d91ec3`）、sub-agents 26/26
    （`e68b74a4-e98c-4c55-95e8-7b05bb3906fd`）、agent-app 38/38
    （`0b0d7a6e-71c4-41b1-abd3-4de17be45e1f`）、golish CLI/resume 56/56
    （`c0b6b6fb-9485-4e35-8b3b-8a4f9a70d444`）、`v2_closeout_replay` 3/3
    （`61188b0e-2ccd-4c02-a3d2-0be5c7f41c83`）、startup reaper exact-chain 1/1
    （`e7553696-63c2-4b50-b9f0-569bb9d7c9cc`）。
  - frontend：5 files / 30 tests passed；`pnpm typecheck` exit 0。
  - final：`just lint-rust`、`just test-rust`、第二次完整 `./init.sh`、`just precommit` 全部 exit 0；
    precommit 输出 `✓ All checks passed!`。第一次 final `init.sh` 的代码/测试全绿但按设计在未暂存的
    intended generated diff 上被 `check-types` 拒绝；暂存两份 generator output 后，`just check-types`
    重新生成无新增 drift，第二次 `init.sh` 整条 exit 0。
  - static：`cargo fmt --all -- --check`、`git diff --check`、JSON parse、58-crate DAG、Python 7/7、
    py_compile、Finding-write authority 均 exit 0；ownership no-new ratchet 相对 `13b29628` 为
    `current=106, baseline=107, removed=1`，只证明零新增，不宣称历史 full checker clean。
  - smoke：`just dev` 到 Vite ready、Rust binary、embedded PostgreSQL ready、migrations complete、
    frontend ready；主动 Ctrl-C 后 `just kill`，1420 无监听。未触发 scan/LLM/external API。
- **已记录证据**：上述可重放命令、run ID、exit code 与关键输出已同步到本记录和
  `feature_list.json`；唯一 `in_progress` count=1。feature 不改 `passing`，因为尚无授权 live
  workspace/scope 的多组织 restart、Candidate/FactDelta/follow-on Wave/report lineage transcript+DB 证据。
- **commit 记录**：本轮计划以 `feat(harness): finish runtime memory candidate v2 implementation`
  单一 commit 落在 `codex/runtime-memory-candidate-v2`；具体 hash 以 `git log -1` 为准，未 push。
- **风险 / blocker**：fresh rollout singleton 停在 rank 1 是 retained-cohort 安全设计，不是代码缺口；
  rank 2/3 必须等真实 exact samples。仓库完整 ownership checker 仍有历史基线（253 ownership +
  18 raw-SQL 的独立审计结果），本轮只满足 no-new ratchet与 Finding writer authority。唯一产品级
  blocker 是缺少用户指定并授权的真实 acceptance workspace/scope。
- **下一步建议**：用户给出明确 workspace 与 scope 后，执行一次受控多组织 run/restart；先读该
  session 的 `run.log`，再跑 `python3 scripts/run_tree.py --workspace <ws> <session> --full --db`，
  核对 frozen org snapshot、exact source、CandidateAttempt、accepted FactDelta、next Wave、Finding/report
  lineage 与 DB receipt。只有证据完整后才把 feature 改为 `passing` 并允许 rollout 继续晋级。
- **以下文件已修改但未提交**：本记录所述 Runtime Memory/Candidate V2 实现、00012–00018、测试、
  generated bindings、stage resources、脚本、设计/计划/模块卡、`feature_list.json` 与本 progress；
  将在本轮同一 commit 中提交。

---

#### 2026-07-14 · Attack rollout Candidate admission/cohort promotion safety closure

- **本轮目标**：收口 attack rollout 审计项 #2/#7；不提交。把 fresh deployment 停在
  rank 1，以 DB-owned Candidate admission/cutoff 为 promotion 分母，封住 raw adjacent UPDATE、
  late sample、空 operation 与 final-seal/promotion 同事务死锁旁路。
- **TDD RED 证据**（每条 Cargo 命令前均运行 `just space-guard`）：
  - `cargo nextest run -p golish-db --test attack_rollout_cohort_migrations --status-level fail`
    → run `261e223e-f05b-450e-968f-8a9caa52aec3`，dual attack + runtime `legacy_v1` 被旧矩阵错误接受。
  - 同一独立 test 的 schema case → run `adc88055-846d-4adb-ac08-e925d62e3f8a`，
    `attack_execution_candidate_admissions` 尚不存在（SQLSTATE `42P01`）。
- **已完成实现**：
  - `00005` 只做 attack rank0→1；新增 additive `00016_attack_rollout_candidate_cohort.sql`。
    gen0 首个 WaveUnit 在 rollout `FOR SHARE` 下写 operation/scope/Wave/Unit/contract/rank/version +
    BIGSERIAL admission；已 admission operation 可完成 follow-on，旧 contract 的首次 late admission/
    Candidate Unit 被拒绝。
  - promotion 在 rollout `FOR UPDATE` 下冻结 `MAX(admission_seq)`，left-join admission→exact
    terminal Wave/WaveUnit→唯一 final-passed Candidate Unit→shadow。DB 从 canonical Candidate/
    work-item/evidence rows递归重建 serde-compatible semantic JSON/hash，任一缺失、未终态、mismatch
    均 typed not-ready；raw UPDATE 走同一 gate。trigger 内生成 immutable receipt，receipt trigger
    再从 DB 重算 cutoff/counts，caller 不能自报。
  - Rust reconciler 对同 cohort 再 rehydrate whole record，返回 `Promoted|NotReady|AlreadyCurrent`，
    每次最多相邻一级；00017 的 `lock_execution_rollout_pair()` 在任何 rollout row lock 前取得。
    Candidate final seal 先业务 commit，再独立 best-effort reconcile；新 operation 先独立 reconcile，
    再冻结两份 contract。promotion 失败不反向否定已提交 seal。
  - operation 合同矩阵收紧：dual attack 拒绝 runtime `legacy_v1`；attack `v2_only` 仍只允许
    runtime `v2_only`。closed shadow 使用的 Candidate/work-item/evidence semantic source 同步冻结。
  - 同步 attack rollout design/plan、`golish-db`/repo、`golish-agent-app/ai` 模块卡、INDEX；
    ownership map 补登记既有新 repo `attack_execution_shadow=agent`。admission/receipt 是
    `attack_execution_rollout` 模块内部表，无新增 repo key。
- **GREEN / 已记录证据**：
  - 在 `00017` advisory pair lock 落地后重跑独立 migration/behavior 整文件最终 **5/5**：run
    `51b976ce-7a11-4e17-be10-99abf4d6c9b0`。覆盖 fresh rank1、合同矩阵、首次 admission、open-Wave
    block、late Unit cutoff、SQL canonical rebuild、closed-source tamper rejection、raw UPDATE、
    DB-generated receipt/direct-forgery rejection与 typed repo reconcile。
  - `cargo check -p golish-db -p golish-agent-app` exit 0；
    `cargo clippy -p golish-db -p golish-agent-app --all-targets -- -D warnings` 于最终审计重跑 exit 0；
    本 slice 四个 Rust 文件 `rustfmt --check` 与 scoped `git diff --check` exit 0。
  - `python3 -m unittest scripts.tests.test_check_repo_ownership -v` 1/1 exit 0；全树 ownership checker
    不再报告 `attack_execution_shadow` 未登记。本轮按 ownership 约束未修改 Verification-owned giant。
- **当前状态 / 风险**：父 feature 继续 `in_progress`。共享树 full `cargo fmt --check` 仍报告
  其它并行文件格式 drift；`check_repo_ownership.py` 仍有既有全树 ownership/raw-SQL 基线噪声，
  与 00016 新表无关；尚未跑最终 `just precommit`，不得宣称整功能 passing。
- **以下文件已修改但未提交**：`00016_attack_rollout_candidate_cohort.sql`、独立
  `attack_rollout_cohort_migrations.rs`、`attack_execution_{rollout,shadow}.rs`、
  `operation_state.rs`、runtime final-seal/create reconcile 接线、上述设计/计划/模块卡/ownership map
  与本 progress；共享树其它并行改动均未回滚或覆盖。

---

## 当前已验证状态

> 这是项目当前状态的**唯一真相来源**。任何与此处冲突的"agent 记忆"或"以前的回复"都不算数。

| 字段 | 值 |
|---|---|
| **仓库根** | `/Users/christopherzheng/WebstormProjects/Golish-Platform`（macOS）/ 同名相对路径 |
| **栈** | Tauri 2 + Rust workspace (50+ crates) + React 19 + TypeScript 6 + Vite 8 + Tailwind 4 |
| **包管理** | `pnpm`（前端）+ `cargo` nextest（后端） |
| **标准启动** | `just dev`（全栈热重载,端口 1420）/ `just dev-fe`（仅前端 mock） |
| **标准验证** | `just precommit` = `just check && just test` |
| **当前状态（2026-07-14）** | `runtime-memory-candidate-pipeline-v2-2026-07-12` 是唯一 `in_progress` feature。Runtime Memory whole-record resume/atomic claim、Candidate multi-wave V2、typed Verification/FactDelta、shadow cohorts/fuel、Memory→Post-Exploit→Cleanup→Reporting 与 00012–00018 均已实现；fresh `./init.sh`、`just precommit` 与 dev smoke 全绿。代码实现完成，但未把确定性本地门禁冒充真实 live acceptance。 |
| **当前 blocker（2026-07-14）** | 只缺用户指定并授权的真实 workspace/scope acceptance：多组织 restart/resume，以及 CandidateAttempt→FactDelta→follow-on Wave→Finding/report lineage 必须由 `run.log`、`transcript.json`、`run_tree.py --full --db` 和 DB rows 共同证明。fresh rollout 留在 rank 1 是 retained cohort 设计，不是未实现。 |
| **提交状态（2026-07-14）** | 分支 `codex/runtime-memory-candidate-v2`，checkpoint `13b29628`；本轮最终 commit message 为 `feat(harness): finish runtime memory candidate v2 implementation`，具体 hash 以 `git log -1` 为准，未 push。 |
| **当前最高优先级（2026-07-14）** | 先完成用户授权 live acceptance 与 DB/transcript truth 审计；证据完整后才把 feature 改 `passing`，并只让 DB-owned retained cohort/receipt 推进 rollout rank。禁止手工改 singleton 或用空 cohort 直跳。 |
| **历史快照说明** | 本表中未带 `2026-07-14` 的同名长行均为旧版考古快照，只保留兼容追溯，绝不代表当前 feature、blocker、commit 或未提交状态。 |
| **当前最高优先级** | **用户已澄清北极星 = crate-per-service（每个功能独立 crate、类微服务）**。新写 `docs/superpowers/plans/2026-05-30-crate-per-service-split.md`（servitization 阶段 3 S3-2 可执行化），feature_list `arch-crate-per-service-split` 已转 **`in_progress`**（M0 阶段）。**2026-05-30 进展**：§6 的 4 决策全按推荐拍板 + Tauri 跨 crate 注册机制 web 核实（Discussion #5378：invoke_handler 只调一次 → 单聚合 generate_handler! 路径引用）。**M0 State 下沉半边 = 完成+验证**：新建 `golish-app-core`(L5) 收 GolishError+DbState（AppState 故意留 golish），`golish/src/{error,state/db}.rs` 改 re-export，`check_dag.py` 加 L5；`cargo check -p golish-app-core` ✅ + `check_dag.py` ✅(46 crates) + golish 编译用户确认 OK。**M1（vuln 叶子）整体完成+验证**（MCP-agent-2 接 dead session yj5fxhjr 半成品）：vuln_intel(M1a)+wiki(M1b) 均 git mv 进 golish-vuln-app；`cargo check` 两 crate + `check_dag`(47 crates) + `check_repo_ownership` 全 exit 0；M0 欠的多 crate 命令注册由此 compile-level 实证。**M2（recon 服务）整体完成+验证（2026-05-31 · MCP-agent-4 · 层次 A 编译期依赖链）**：`golish-recon-app` 抽入 11 模块组（targets/organizations/scan_queue/sensitive_scan/custom_rules/scan_runner/intel_providers/wordlists + asset_intel + integrations），`scoping` 下沉 golish-app-core，asset_intel 解 PentestState（`ToolsConfigState` 共享同一 `Arc<ConfigManager>`），integrations（含 tauri webview 捕获引擎）搬迁 + tauri_app 启动接线。验证：`cargo check` 两 crate + `nextest -p golish-recon-app`(106✓) + `clippy -p golish-recon-app -D warnings` + `check_dag`(48 crates) + `check_repo_ownership` 全 exit 0。**M3（pentest 服务）整体完成+验证（2026-05-31 · MCP-agent-3 · 层次 A 编译期依赖链）**：`golish-pentest-app`(L5.6) 抽入 9 模块组（pentest/pentest_ai/pentest_bridge/findings/methodology/pipeline/execution_plans/evidence/security_analysis + 连带 output_parser）；**两个共享件下沉 golish-app-core**：`pty_interactive`（golish state/runtime/ai + pentest_ai 双用）+ `ports`(VaultReadPort/PgVaultAdapter，S1-2a)。pentest-app 编译期依赖 recon-app(targets)/pipeline(L3)/app-core；ai/ 入向桥 `pub(crate) use golish_pentest_app::{pentest,pentest_ai,pentest_bridge}`。验证：`cargo check` 两 crate + `nextest -p golish-pentest-app`(**47✓**) + `clippy -p golish-pentest-app -D` + `clippy -p golish --lib -D` + `check_dag`(**49 crates**) + `check_repo_ownership` 全 exit 0。**M4 调查 + M4-A（AppState 解耦）完成（2026-05-31 · MCP-agent-3）**：M4（agent）实证发现**真实 blocker**——`ai/commands/*`(19 文件) 几乎全 take 单体 `AppState`，而 `AppState` 聚合 `AiState`(定义在 ai/commands/mod.rs)，三者互锁 → 直接抽 ai/ 会造成 golish↔agent-app 循环（见 `docs/superpowers/plans/2026-05-31-m4-agent-app-feasibility.md`）。用户选「开 A：AppState 解耦」。**M4-A 完成**：新建 `golish-agent-app`(L5.6)，`AiState` 搬入 + 新 `AgentState`(13 字段 ≈ AppState 减 command_index/telemetry/langfuse)；`AppState::extract_agent_state()` + 启动 `.manage()`；**19 个 ai/commands 全部 `State<AppState>`→`State<AgentState>`**；bridge_config/mcp 接线改走 AgentState。验证：cargo check 两 crate + `clippy -p golish --lib -D` + `clippy -p golish-agent-app -D` + `check_dag`(**50 crates**) + `check_repo_ownership` 全 exit 0；ReadLints 无错（顺带 #[allow(dead_code)] 3 处 pre-existing 死字段 pty/sidecar/db_pool_ready，recompile surfaced）。**M4-proper 完成+验证（2026-05-31 · MCP-agent-3 接另一 MCP 半成品收尾）**：另一 MCP 已 `git mv` ai/ 全子树 + conversation_store 入 golish-agent-app、runtime/ 下沉 golish-app-core（TauriRuntime 解耦 AppState 改 take pty_output_tap 参数）、golish 侧 ai.rs/runtime.rs/conversation_store shim + facade + 守卫，但只跑 cargo check（带 4 unused warning）、从未跑 clippy、且在删死 re-export 前被掐断。本会话补完：① agent-app lib.rs 加 crate 级 `#![allow(clippy::too_many_arguments)]`（agents.rs:43 16 参命令）；② 删 4 个死 re-export（state AgentState / tools pentest_ai+pentest_bridge / db PgPentestStore，db/mod.rs 现空占位）。验证全绿：cargo check 两 crate + clippy 两 crate `-D warnings` + nextest -p golish-agent-app(**15✓**) + check_dag(**50 crates**) + check_repo_ownership 全 exit 0。**M5 platform 完成+验证（2026-05-31 · MCP-agent-3 · 用户「开 M5 platform」）**：抽 `golish-platform-app`(L5.5 纯叶子，零兄弟依赖)——`tools/{vault,audit,notes,recordings}.rs`(4 文件全 `State<DbState>`)`git mv` 入 crate，跨服务读经 golish_db::repo(L2) 不经兄弟 crate；导入重映射 `crate::{error,state::DbState,tools::scoping}`→`golish_app_core::*`；crate 级 too_many_arguments allow；facade vault/workspace 转发；golish tools/mod 删 4 pub mod + 删死 scoping re-export；守卫 check_dag(platform-app=5.5)+check_repo_ownership(SOURCE_ROOTS + DOMAIN_RULES 清 4 + ALLOWLIST/RAW_SQL 迁前缀)。验证全绿：cargo check 两 crate + clippy 两 crate -D + nextest -p golish-platform-app(**1✓**) + check_dag(**51 crates**) + check_repo_ownership 全 exit 0。**🎯 crate-per-service 北极星：5 个服务域(vuln/recon/pentest/agent/platform)全部层次 A 抽完。** epic 维持 in_progress 待层次 B（端口切兄弟硬依赖升真微服务）/ precommit / commit 收口。**层次 B 启动 · S1-2b1 完成（2026-05-31 · MCP-agent-3 · 用户「开层次 B 端口化」→「按推荐开干 b1」）**：发现 S1-2b 设计过期（写于层次 A 前，假设端口放 golish/src/ports），修正端口家为 **golish-app-core/src/ports/recon/**（6 消费方已分散到 4 app crate，不能依赖 golish）。建 ReconScansPort(10 method)+ReconAssetsPort(1 method)（镜像 repo 签名去 pool、返回同 Row 类型 remote-ready、纯透传适配器）；GolishDbRepoProvider 加 2 端口字段 new(pool) 内构造（外部签名不变）；agent-app recon.rs 11 调用点迁端口；守卫加 ('ports/recon','recon') + 删 5 条 ALLOWLIST。验证全绿：check app-core/agent-app + nextest ports::recon(2✓) + clippy 三处 -D 零告警 + check_dag(51) + repo_ownership(OK,ALLOWLIST 净减 5)。**b2 完成（2026-05-31，用户「接着开 b2」）**：security_analysis.rs（pentest，10 自由 Tauri 命令）5 recon 表迁端口；因须保留 pool_ready 就绪门，用『就绪门后内联构造适配器』（非 struct 注入）；扩 ReconScansPort +4 + ReconAssetsPort +1 method；删 5 条 ALLOWLIST（累计 28→18）；验证全绿（check pentest-app + nextest ports::recon 2✓ + clippy app-core/pentest-app/golish --lib -D 零告警 + 双守卫）。**b3-b6 完成（2026-05-31，用户「连续干 b3-b6」）→ S1-2b ReconPort 全 6 子片完成,22 条 recon 跨服务耦合全切断（ALLOWLIST 28→6）**：新建 ReconTargetsPort/ReconSitemapPort/ReconDirectoryPort + 扩 ReconScansPort（js_analysis_update_file_path_by_url、passive_scans_list_global_by_project，含端口 DTO ReconPassiveScanGlobal 解泛型 object-safety + app-core 加 chrono）；迁 8 文件（pentest_bridge 5 + pipeline/storage + platform/audit + vuln/matching），`&PgPool` 消费方用 Arc::new(pool.clone()) 注入。验证全绿：check 3 消费方 + nextest ports::recon(5✓) + nextest pentest/platform/vuln(48✓ 无回归) + clippy 五处 -D 零告警 + 双守卫。剩余 ALLOWLIST 6 = pentest_plan/vuln/agent_log/scan_queue（S1-2c/d/e/f，非 recon）。 **commit + S1-2c/d/e/f 完成（2026-05-31，用户「你帮我commit吧...后面全部做完」）**：① M0–S1-2b 已 commit `45f4bb2`（229 文件，未 push，本地 ahead 12）；② S1-2c（VulnIntelPort+WikiKbPort）/ S1-2d（PentestPlanPort）/ S1-2e（AgentLogReadPort，含 DTO 解泛型）/ S1-2f（scan_queue REPO_OWNER vuln→recon 伪阳性修正）全部完成 → **S1-2 横向耦合端口化整体完成，ALLOWLIST 28→0（cross-service ratchet 清空，每条横向 repo 耦合都走 golish-app-core/ports/ 服务端口）**。验证全绿：clippy app-core/agent/platform/golish --lib -D 零告警 + nextest app-core ports(10 object-safe) + nextest agent/platform/vuln(16✓ 无回归) + check_dag(51) + check_repo_ownership OK clean(ALLOWLIST 空)。**下一步 = commit S1-2c-f；（用户回来后）跑 just precommit 决定 push**。**§2.1：2 个 in_progress = arch-crate-per-service-split（父 epic）+ arch-s1-2b-recon-port（子里程碑），同一工作流父/子两粒度。** M0–S1-2b 已 commit(45f4bb2,未 push)；S1-2c-f 待 commit；全套未 push、未跑 just precommit 全量。前置端口：`arch-s1-2b-recon-port`（`not_started`，设计已写，ReconPort 是 M2 recon 抽取的前置）。父条目 `arch-s1-2-port-horizontal-coupling` 已 **passing**（S1-2a 走路骨架确立）。`target-surface-workbench` 继续 `blocked`。**§2.1 当前 in_progress 数 = 1（arch-crate-per-service-split）**。 |
| **当前 blocker** | `xiaomi-mimo-provider` 已从 `in_progress` 切 `blocked`，等待 tool-use compatibility layer 与真实 MiMo E2E 后再决定 passing。2026-05-27 复测发现 `ask_human` 被误包成普通 ToolApprovalRequest；已修为直接发 `AskHumanRequest`，但需重启 dev app 后真实复测。**2026-05-30 更新**：本机 `just check` **全绿**（fmt + check-fe + test-fe + lint-rust（clippy `-D warnings` 0 告警 + `cargo fmt --check`）+ test-rust-all（nextest **2592 passed / 7 skipped / 0 failed**）+ check-types（ts-rs 绑定无漂移）均 ✅）。此前记录的 clippy warnings 与 sandbox PermissionDenied baseline failures 在本机最新工作树**未复现**。 |
| **未提交的半成品** | **2026-06-15（MCP-agent-1）：0.zone apk 微信公众号映射补全——已 commit `3ea3466d`（8 文件：0.zone http partial-success/retry [与 MCP-3 entangled，含 http.rs/models] + wechat 映射 0-zone.json/profile_patch.rs/org-fields.ts/测试；未 push）。i18n `en/zh-CN.json` 的 wechat 标签因与他会话「assets」hunk 共文件**未纳入**（feature 经英文回退仍显示，中文标签待协调提交）。TDD 全绿（nextest asset_intel 66/66 + clippy -D 零告警 + 前端全量 1322 + check-fe exit 0），未跑全量 precommit。详见会话记录最新一条。** **2026-06-14（MCP-agent-2）：engagement 总览/扇出全栈移除 + stage-run 闭环——47 项改动未 commit（branch `feat/stage-run-fanout`）；完整 `just check` 全量全绿（`cargo clippy --workspace -- -D warnings` 0 告警 + `cargo nextest --workspace` 3269 passed/7 skipped + check-fe/test-fe 绿）。详见会话记录最新一条。** **2026-05-30：架构优化批已拆 9 commit 落 `feat/recon-service`（`98beea9`→`6aaa0fb`，HEAD `d060ce4`）。** 其上叠了 **P0-3b 残余作用域 SQL 下沉**（T1-T6 全部完成，**未 commit**）：26 个 tracked 文件改动 + 6 个新 repo 模块（untracked：`repo/{scan_queue,sensitive_scan,conversation_store,directory_entries,sitemap_store,custom_rules}.rs`）。验证：rg 命令层裸作用域 SQL 清零、`golish-db` nextest 46/46、`golish --lib` nextest 318/318、`clippy golish-db+golish` 全绿，并跑通**全栈 `just precommit` → `✓ All checks passed!`（exit 0）**（含用户授权后修的 1 个 pre-existing `integrations/commands.rs:179` baseline）。**已按拆分提交 4 个 commit**（`65e0292`/`06af27a`/`d023386`/`c2f5ad2`，落 `feat/recon-service`，未 push）。**2026-05-30 续（MCP-2）：P2 拆分①完成——`golish-pentest-domain/src/models.rs`(1310) 模块化为 module-root + `models/{tool_config,asset_intel,runtime,tests}.rs`（全 < 500 行），全验证通过（crate check/nextest 17✓/clippy `-D warnings`/`cargo check --workspace` 全绿），**未 commit**（`M models.rs` + `?? models/`）。P2 拆分②完成——`golish/src/tools/pentest_bridge/js_collect.rs`(1357) 模块化为 module-root + `js_collect/{extract,judge,quality,sitemap,tool_impl,tests}.rs`（全 < 500 行，max 470），全验证通过（`cargo check -p golish`/`nextest js_collect` 26✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M js_collect.rs` + `?? js_collect/`）。P2 拆分③完成——`golish/src/tools/integrations/capture/engine.rs`(1483) 模块化为 module-root + `engine/{extract,helpers,tests}.rs`（全 < 500 行，engine.rs 496）；生命周期/webview 方法留 root 避免 super:: 改写，全验证通过（`cargo check -p golish`/`nextest capture::engine` 23✓/`clippy -p golish --all-targets -D warnings` 全绿），**未 commit**（`M engine.rs` + `?? engine/`）。P2 拆分④（进行中）——`frontend/mocks.ts`(4135→2353) 抽出事件系统/AI 模拟/showcase 三层到 `mocks/{event-bus,events,simulations,showcase}.ts`（公共面零变更；`showcase.ts` 1146 仍 >500 待再分），`check-fe`+`test-fe` 全绿；剩余 demos/有状态 ipc 待续。**✅ 已按块 commit**：经 `just precommit` 全绿（`✓ All checks passed!`，~21.7min）后落 5 个 commit 到 `feat/recon-service`（`a71319b` pentest-domain models / `03871db` js_collect / `63c196e` capture engine / `83a105c` frontend mocks / `dd3c367` docs progress，**未 push**）。**2026-05-30 收尾（MCP-agent-2）：本会话架构体检全批（拆/合并/优化/dedup）已 `cargo fmt --all` 后按主题拆 20 个 commit（`a85f7d4`(scripts)→…→ docs(progress)，**未 push**）；提交后工作树 clean。完整 `just precommit` 本轮未重跑（树稍早已全绿，fmt 仅排版）。** **2026-05-30 续（MCP-5 · 接 MCP-3 转交）：S1-1 repo 数据所有权守卫 + check_dag 修复**——已修既有 `golish-graphiti(L1)→golish-db(L2)` DAG 违规（graphiti 归 L2，非删依赖）；`just arch` → **exit 0**（双守卫全绿）。已落 4 commit 到 `feat/recon-service`（`b0811ea`/`dc9ad0f`/`821c101` + 1 docs commit，**未 push**），提交后工作树 clean。feature_list `arch-s1-1-repo-ownership-guard` → **passing**；`just precommit` 未重跑（改动集零 Rust/TS/Cargo diff）。 **2026-05-30 续（MCP-agent-4 数据工程）：S1-2a `VaultReadPort` 走路骨架** —— 另一会话写 Tasks 1-4（端口/迁移/注入），本会话接手 Task 5（守卫拔 ratchet）+ Task 6（文档/feature_list/progress）。改动：`?? golish/src/ports/`(3 文件)、`M golish/src/lib.rs`、`M tools/pentest_bridge/{vault_ops,auth_probe,mod}.rs`、`M scripts/check_repo_ownership.py`、`M docs/architecture.md`、`M feature_list.json`、`M agent-progress.md`、`?? docs/{design,plans}/2026-05-30-s1-2-*`。验证：`cargo check -p golish` exit 0、`just arch` exit 0（ALLOWLIST **30→28**）、guard OK clean、`rg golish_db::repo::vault` 于 pentest_bridge 空。**2026-05-30 续（MCP-agent-3 后端工程，用户授权 C: A+B 一气呵成）**：跑 `cargo nextest -p golish ports::platform::vault` → **1 passed/373 skipped exit 0**（4m53s 冷编译）+ `just precommit` → **✓ All checks passed! exit 0**（29.6 min · fmt+check-fe+test-fe+lint-rust+test-rust-all 全绿）；按 plan 拆 **6 commit 落 feat/recon-service**：`6abaec8`(feat 端口骨架,4f+118)/`1e162de`(refactor VaultTool,1f)/`1a7018b`(refactor AuthProbeTool,1f)/`1149ddb`(refactor 构造点注入,1f)/`389d3fd`(chore 拔 ratchet,1f) + `23e47a6`(docs S1-2 design+plan+architecture+feature_list+progress,5f +947-3)；**未 push**，本地 ahead 10。**2026-05-30 续 2（MCP-agent-3 · 用户授权"你想怎么搞合适"）**：S1-2 父条目 `arch-s1-2-port-horizontal-coupling` → **passing**（走路骨架确立）；**新增** `arch-s1-2b-recon-port` 条目 `not_started`（等用户审 §10 5 决策再转 in_progress）；**新写** `docs/design/2026-05-30-s1-2b-recon-read-port.md` S1-2b 高层设计（22 条 allowlist 精确清单+grep 实证、6 子片划分 b1-b6、ReconPort trait 25 method 含读+写、守卫配合、5 待拍板决策）；命名差异关键：a 是 ReadPort（read-only），b 是 Port（含写，因 agent-bridge 适配器内有 insert/upsert/update）。新增/修改 3 文件：`?? docs/design/2026-05-30-s1-2b-recon-read-port.md`、`M feature_list.json`、`M agent-progress.md`。**待 commit + 不 push**（push 需用户单独点头，按 AGENTS.md §2.7 红线保守处理）。 **2026-05-30 续（MCP-agent-2）：M1 crate 抽取全套未 commit** —— `?? backend/crates/golish-app-core/`(M0)、`?? backend/crates/golish-vuln-app/{Cargo.toml,src/lib.rs}` + `RM` 19 文件（vuln_intel 8 + wiki 11，git mv 进 golish-vuln-app/src/）、`M backend/Cargo.toml`、`M golish/{Cargo.toml, src/commands_facade/{vuln_intel,wiki}.rs, src/tools/mod.rs, src/error.rs, src/state/db.rs, src/event_emitter.rs}`、`M scripts/check_{dag,repo_ownership}.py`、`M feature_list.json`、`M agent-progress.md`。验证：`cargo check` 两 crate + 双守卫全 exit 0；**未跑 just precommit 全量、未 commit、未 push**。 |

---

## 会话记录

> 倒序排列,最新一轮在最上面。每轮一条。

> 历史会话已归档：`docs/archive/agent-progress-archive-2026-06-28.md`。主文件只保留最近 20 条会话，避免旧日志干扰新判断；需要追溯旧验证证据时去 archive grep。

#### 2026-07-13 · Candidate Task 10–12 authorized continuation

- **本轮目标**：执行用户已明确授权的 `20260712000012_attack_fact_delta_wave_entry.sql` 与相关 `golish-db` / Candidate 多波实现；先以 TDD 完成 typed FactDelta-set → follow-on Wave provenance、原子 consolidation/fuel/residual，再完成 Task 11 integration，最后仅在 shadow gate 全绿时处理 `00005` cutover。
- **开工证据**：分支 `codex/runtime-memory-candidate-v2`，起点 HEAD `13b29628`，工作树 clean；`just space-guard` exit 0；`./init.sh` 完整 exit 0，`fmt`、`check-fe`、`test-fe`、`lint-rust`、`test-rust-all`、`check-types` 全部通过。用户随后在聊天中明确表示已给权限并要求开始。
- **执行约束**：不改写冻结 `20260712000001` / 已存在 `00004`；不复用首波 `vuln_triage` handoff伪造后续 Wave；不发起真实扫描、exploit、外部 API、embedding 或 Graphiti 请求；不 push。`00002/00005` 只有各自 shadow/cutover gate 满足后才允许处理。
- **当前状态**：本轮正在执行，尚未宣称 Task 10–12 完成；以下 RED/GREEN、模块卡、feature evidence、commit 与剩余风险将在收尾前补齐。

#### 2026-07-13 · V2 safe-slice final gate / startup smoke / checkpoint closeout

- **本轮目标**：继续收口 Golish 运行期记忆与 Candidate 攻击流水线 V2 的所有未阻塞实现，修完共享树全量门禁暴露的真实回归，补齐 CandidateAttempt 可达 UI 与 Reporting live-pointer canonical hash 合同，完成 clean-state 验证并形成未 push checkpoint；未获明确授权的 follow-on-wave migration 不创建。
- **已完成**：
  - 修正 Reporting blocked Cleanup fixture 的合法写入顺序：blocked decision/evidence 必须在 obligation terminal 前写入，生产 terminal evidence-union guard 不放宽。
  - 收紧 terminal `candidate_attempts` 的 retained audit 合同：删除 live target 时只允许 FK 驱动的 `target_live_id non-NULL→NULL`；旧 target 必须已不存在，除该 pointer 外整行（含 `row_version`）必须 exact 不变。目标仍存在时直接清空、夹带 canonical 字段、no-op/DELETE 仍以 `TERMINAL_CANONICAL_SOURCE_IMMUTABLE` 拒绝。Reporting CandidateAttempt source hash 同步排除该非 canonical pointer，保留 frozen target/plan/result/status/evidence/version。
  - 新增 `CandidateAttemptRows` DB-authoritative read model 与真实 `ToolCallDetailView` 入口，覆盖 loading/error/empty、exact operation/wave refresh、ordinal/status、active/queued、proof/refutation/blocker roles、verified-only Finding lineage 与 blocked residual；不伪造 IPC 未返回的 Finding id。
  - 旧 P1/P2 计划补 `Superseded by ...-corrected.md` 头注释；Reporting domain/app 模块卡与 INDEX 同步为当前已实现状态，并明确 Windows runtime 仍需 Windows runner。
- **运行过的验证**：
  - 全量门禁 RED 1：Reporting fixture 在 terminal obligation 后追加 blocked-decision evidence，被 DB guard 正确拒绝；重排 fixture 后 focused `reporting_authority` 1/1 GREEN，run `b6b295ce-34be-4be9-aed7-d0ad4814adbc`。
  - 全量门禁 RED 2：live target 删除触发 `ON DELETE SET NULL`，被 terminal Attempt guard 拒绝。首版窄例外的 focused RED run `7024ed66-c4b1-4e34-acbe-d86a3a2c0847` 又发现共享 row-version trigger 在非 Attempt 表解析 `OLD.status`；改为 table-nested 分支、direct-tamper rejection 与 canonical-row/version assertions 后 GREEN 1/1，run `9b88d1bb-8185-4e97-942a-c2ee0834a8c6`。
  - DB retention + Reporting authority focused 13/13 GREEN，run `a8cce86a-0a6a-449f-838b-4d359e517480`；Reporting CandidateAttempt 删除前后 version/content/source-set hash 集成回归整文件 13/13 GREEN，run `47bfc6af-5ea0-41ed-8515-fc20bddd9a95`。
  - CandidateAttemptRows TDD：缺组件 RED 后 5/5 GREEN；四个 feature 前端文件 19/19；真实 Tool detail mount 先 1 RED 后 1/1 GREEN，ToolCallDetail + Candidate scoped 4 files 21/21；`pnpm typecheck`、`just check-fe`、`just test-fe` 均 exit 0。
  - 最终 `just space-guard && just precommit` 完整进程 exit 0，输出 `✓ All checks passed!`；fmt、check-fe、test-fe、workspace Clippy `-D warnings`、`test-rust-all`、check-types 与 `test` 尾段全部通过。`cargo nextest list --workspace --message-format json` 当前列出 5285 tests。
  - 最终静态守卫全部 exit 0：tracked/cached/untracked whitespace diff check；`jq` feature/phases/all stage specs；`check_dag.py`（58 crates）；Finding-write authority ratchet；generated bindings 无 unstaged drift；唯一 `in_progress`。P1 `00001` SHA-384 仍为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`，`00002/00005/00012` 均不存在。
- **已记录证据**：标准 `just dev` smoke 显示 Vite `http://127.0.0.1:1420/` ready、Rust binary running、`Database migrations complete`、`Embedded PostgreSQL is fully ready` 与 frontend ready；随后主动 SIGINT（预期 exit 130）并执行 `just kill`，1420/Vite/`target/debug/golish` 残留检查 exit 0。启动对本机 embedded DB 应用了已授权 additive migrations，并按既有启动恢复逻辑回收 8 条 abandoned audit rows、暂停 1 个 abandoned runtime operation；未运行扫描、exploit、embedding、Graphiti 或其他外部请求。磁盘仍有约 121 GiB 可用，space guard 未触发清理。
- **提交记录**：本轮实现与本记录同一 checkpoint commit，message 为 `feat(harness): checkpoint runtime memory and candidate pipeline v2`；分支 `codex/runtime-memory-candidate-v2`，未 push。
- **已知风险或未解决问题**：整个 V2 **尚未全部完成**。Candidate Task 10 的 accepted FactDelta-set follow-on wave 缺少可表达 exact typed provenance 的 additive schema；用户只说“继续”，不等于授权新增 `20260712000012_attack_fact_delta_wave_entry.sql`。因此 Task 10、依赖它的 Task 11 余下多波 integration、Task 12 `00005` cutover 与 live multi-wave acceptance 均未执行，feature 必须保持 `in_progress`。Windows artifact backend 已交叉编译、Clippy 与 test binary link 验证，但 macOS 无法执行 Windows runtime tests。
- **下一步最佳动作**：请用户明确回复是否允许新增 `20260712000012_attack_fact_delta_wave_entry.sql`（additive，仅表达 typed FactDelta set → next-wave provenance）。若确认，先写 consolidation/fuel/wave RED，再实现 Task 10→11→12；若不确认，维持当前 V2 default/cutover 关闭并只审查本 checkpoint。

#### 2026-07-13 · Cleanup terminal evidence-union / Attempt immutability P2

- **本轮目标**：按已确认的窄 P2 收紧未 cutover `20260712000010_cleanup_closeout.sql`：Cleanup terminal event 的 evidence membership 在 terminal 后不得增长；terminal CleanupAttempt 不得原地变更。未碰其他 P2、reserved `00005` / `00012` 或已冻结 migration，未 commit/stage/push。
- **已完成实现**：
  - 新 `guard_cleanup_terminal_evidence_insert` 覆盖 event evidence union 的五张 retained child：`cleanup_obligation_evidence`、`cleanup_attempt_evidence`、`cleanup_absence_check_evidence`、`cleanup_waiver_evidence`、`cleanup_blocked_decision_evidence`。INSERT 先对 parent obligation 取 `FOR SHARE`，与 status UPDATE 串行：open/in_progress 时同一 compound transaction 可正常先写 child 再 terminalize；`verified_absent|waived_by_user|blocked` commit 后新增 membership 稳定拒绝为 `23514/CLEANUP_TERMINAL_EVIDENCE_IMMUTABLE`。既有 child UPDATE/DELETE immutable triggers 保持不变。
  - 新 `reject_terminal_cleanup_attempt_change` 允许 live Attempt 一次进入 `verified_absent|verification_failed|execution_failed`；当 OLD 已属于三种 terminal status 时，任何 UPDATE（含 no-op）/DELETE 均以 `23514/CLEANUP_TERMINAL_ATTEMPT_IMMUTABLE` 拒绝。失败重试继续创建下一 ordinal 的新 Attempt，不复活旧 row。
  - PG 回归覆盖 verified-absence 正常同事务 terminal event、waiver 正常同事务 terminal event、blocked decision+evidence+parent terminal 的 exact deferred transaction；三条路径 commit 后对应 late evidence INSERT 全拒绝。另覆盖三种 terminal Attempt 的 no-op UPDATE/DELETE，以及 verification/execution failure 后新 ordinal retry。
  - 同步 `golish-db`、`golish-db/repo`、`golish-cleanup-domain` 模块卡；INDEX 对应条目继续为 ✅，无状态漂移。
- **TDD RED 证据**（每条 Cargo 命令前均运行 `just space-guard`）：
  - `cargo test -p golish-db --test cleanup_obligation_kernel verified_absence_emits_one_exact_replayable_cleanup_terminal_event -- --nocapture` → exit 101；terminal event 发布后 late `cleanup_obligation_evidence` INSERT 实际 `rows_affected=1`。
  - 加入 terminal Attempt no-op 回归后同一 focused 命令再次 exit 101；OLD=`verified_absent` 的 `UPDATE cleanup_attempts SET status=status` 实际 `rows_affected=1`。
- **GREEN / 已记录证据**：
  - verified-absence focused 1/1、waiver focused 1/1、blocked exact same-transaction closeout focused 1/1、execution-failed immutable+retry focused 1/1、verification-failed immutable+retry focused 1/1 均通过。
  - 最终 `cargo test -p golish-db --test cleanup_obligation_kernel -- --nocapture` → 22/22 passed；覆盖正常 transition/rollback/replay/closeout/deletion 既有路径无回归。
  - `cargo clippy -p golish-db --lib --test cleanup_obligation_kernel --no-deps -- -D warnings`、目标 Rust 文件 `rustfmt --edition 2021 --check`、全树 `git diff --check` 与 `feature_list.json` parse 均 exit 0。
- **当前状态 / 风险**：本 P2 已有 fresh RED→GREEN 与 scoped Clippy 证据；未执行共享树 full `just precommit`，父 feature 继续 `in_progress`。00010 尚未 cutover，因此本轮按授权直接收紧原 migration；已部署数据库若未来需要同等 guard，仍须单独 additive migration/cutover 决策。
- **以下文件已修改但未提交**：`backend/crates/golish-db/migrations/20260712000010_cleanup_closeout.sql`、`backend/crates/golish-db/tests/cleanup_obligation_kernel.rs`、上述三张模块卡与本 progress；共享树其他并行 Candidate/Cleanup/Reporting/artifact 改动未回滚或覆盖。

#### 2026-07-13 · Candidate terminal Memory authority / post-write ACK-loss replay hardening

- **本轮目标**：处理独立复核的三个缺口：reason-only blocked Candidate 不得绕过 sealed scope authority；crash replay 必须证明真实 projector write 已提交而 ACK 丢失；Candidate `fact_delta_count > 0` 在 typed evidence roles 落地前不得猜测 evidence 归属。未创建/修改 migration，尤其未碰 reserved `00005` / `00012`；未 commit/stage/push。
- **已完成实现**：
  - Candidate assertion promoter 抽出并复用 exact scoped-event authority：event/source 校验后，按 envelope 的 operation/project/org 查询包含该 org 的 sealed scope snapshot。真实 projection 与 reason-only intentional suppression 走同一 authority；authority 缺失时 delivery 为 `retryable_failed`、`memory_policy_rejected`、attempt=1、0 Assertion，只有合法 sealed authority 才可 `succeeded_suppressed`，且仍不制造 fake evidence。
  - `CandidateTerminalPayload` 对任何 `fact_delta_count > 0` 稳定返回 `memory_candidate_terminal_fact_delta_evidence_untyped`。Task 10/00012 提供 typed `fact_delta_evidence_ids` 前不接受 evidence union 推断；无 FactDelta 时 terminal-role evidence 必须与完整 evidence set exact 相等。
  - closeout replay fixture 改为真实 post-write/pre-ACK failure：production assertion-promoter 先提交 deterministic Assertion，PG trigger 只拒绝目标 event 第一次 `succeeded` ACK；测试观察到 Assertion=1、delivery=`leased`/attempt=1 后撤故障并过期 lease，runtime 重领 attempt=2。Candidate 与 Cleanup 各自 assertion delivery 都为 2 次，最终 Assertion/Document/Embedding/graph lineage 每层严格 1 条；未受故障的 Foothold/Objective attempt 保持 1。
  - 同步 `golish-memory-app`、`golish-agent-app`、`golish-agent-app/ai`、`golish-db/repo` 模块卡；`docs/modules/INDEX.md` 对应条目继续为 ✅，无职责状态漂移。
- **TDD RED 证据**（每条 Cargo 命令前均运行 `just space-guard`）：
  - `cargo test -p golish-agent-app --lib candidate_fact_delta_evidence_fails_closed_until_typed_roles_exist -- --nocapture` → exit 101；旧逻辑实际返回 `Project { kind: VerifiedOutcome, evidence_ids: [71, 72] }`。
  - `cargo test -p golish-agent-app --test knowledge_memory_runtime reason_only_blocked_candidate_without_sealed_authority_fails_closed -- --nocapture` → exit 101；旧逻辑实际 `succeeded_suppressed`，预期 `retryable_failed`。
  - `cargo test -p golish-agent-app --test v2_closeout_replay candidate_to_report_closeout_is_replay_safe -- --nocapture` → exit 101；旧 crash helper 在 runtime 启动前手工 claim/expire，真实 write count 为 0，预期 1。
- **GREEN / 已记录证据**：
  - `cargo test -p golish-agent-app --lib static_composition_tests -- --nocapture` → 8/8 passed；覆盖 Candidate untyped FactDelta fail-closed、唯一 suppression policy 与 catalog route authority policy。
  - `cargo test -p golish-agent-app --test knowledge_memory_runtime -- --nocapture` → 5/5 passed；同时锁住 missing authority failure、valid sealed authority suppression、bare FactDelta failure、UoW rollback 与 supervisor replay。
  - `cargo test -p golish-agent-app --test v2_closeout_replay -- --nocapture` → 3/3 passed；正式 Candidate→PostExploit→Cleanup→Report closeout 含两次真实 write-before-ACK replay 全绿。
  - 格式化后最终合并复跑 `cargo test -p golish-agent-app --test knowledge_memory_runtime --test v2_closeout_replay -- --nocapture` → 8/8 passed；`cargo clippy -p golish-agent-app --lib --test knowledge_memory_runtime --test v2_closeout_replay --no-deps -- -D warnings` → exit 0；三份相关 Rust 文件 `rustfmt --edition 2021 --check` → exit 0。
- **当前状态 / 风险**：上述复核项已有 fresh RED→GREEN 与 scoped Clippy 证据；共享树仍未运行最终 `just precommit`，父 feature 继续 `in_progress`，不虚报 passing。Task 10/00012 typed FactDelta role schema 仍是显式后续工作，本轮只 fail closed。
- **以下文件已修改但未提交**：`backend/crates/golish-agent-app/src/ai/db_bridge/knowledge_memory.rs`、`tests/{knowledge_memory_runtime,v2_closeout_replay}.rs`、上述四张模块卡与本 progress；共享工作树的其他并行 Candidate/Cleanup/Reporting/artifact 改动均未回滚或覆盖。

#### 2026-07-13 · Candidate V2 Task 8/9 terminalizer, Finding authority and exact DB Gate closeout

- **本轮目标**：只收口 corrected Candidate plan Task 8/9；实现 exact
  `submit_candidate_attempt`、compound Attempt terminalizer、Finding writer authority 与
  Verification DB-truth Gate。明确不做 Task 10 schema/FactDelta consolidation，不做 Task 11，
  不创建 `20260712000005_attack_execution_v2_cutover.sql`，不 commit/push。
- **已完成实现**：
  - verifier 只提交 terminal business fields；server 从 opaque context/DB 重载
    Candidate/approval/plan/WorkerRun/lane，terminalizer 在同一短事务写 Attempt/Candidate、
    role-specific evidence、可选 Finding+immutable lineage、FactDelta，并释放 WorkerRun/lane；
    verified/refuted/blocked 与 response-loss replay 均 fail closed。
  - `findings.rs` 按 persisted `AttackExecutionContract` 守门；`v2_only` harness 只接受
    terminalizer authority。`scripts/check_repo_ownership.py --finding-writes-only` 是独立
    ratchet，只允许 `repo/findings.rs` / `repo/finding_lineage.rs`，不会被大 ownership checker
    的历史基线噪声掩盖；fixture test 锁定 allow/deny/ignore 规则。
  - Verification Gate 只接受同 operation/scope/wave 的完整 snapshot set；foreign/mixed/
    duplicate/missing/DB error 均 BLOCK。Legacy/dual 仍按 persisted contract 保留旧 bounded
    deliverable chain-wave；只有 `v2_only` 强制 exact DB truth，绝不 fallback。
  - stage contract 明确 formulaic observation → complete Candidate decisions → one foreground
    Attempt → `submit_candidate_attempt` disposition → terminalizer。模型只见 ordinal wrapper，
    classifier recipes/raw tools/Finding writer 不暴露。Static findings policy 保留 legacy/dual，
    `v2_only` 三个 attack stage 的 effective policy 禁止 deliverable-authored Finding。
  - 安全复核 follow-up 收紧七处边界：Candidate specialist/Verification flow 只有 runtime-memory
    与 attack-execution 两份 frozen contract 都是 `v2_only` 才启用，legacy/dual 保持旧路由；
    fresh stage prompt、裸 resume `stage_run` force 与 depth-0 tool-list 统一复用这个 effective
    specialist 判定，V2 Verification 动态取得 `candidate_verifier`，legacy Verification 不强制；
    contract authority 读取失败时 primary 留在 specialist-only 路由并由 `stage_run` 重读拒绝，
    `attack_analyst` 同时映射到可 claim 的 durable worker 类型；
    Attempt submission/terminalizer 都要求 approved plan 的每个 action journal 精确存在且已
    `completed|failed`；VerificationTruth 改为 DB-owned current-wave expected-unit authority envelope
    并在 repeatable-read snapshot 内拒绝 partial sibling；lineage-bound Finding 与 submitted/terminal
    Attempt evidence membership 由 trigger 冻结；reason-only blocked 不再强造 evidence；CVSS 持久化
    且 affected target 必须等于 frozen Candidate target；V2 vuln scanner 动态隐藏并 guard 拒绝
    `record_finding`，legacy/dual 工具面不变。
- **验证证据**：
  - 第 19 次完整 `./init.sh` → exit 0；其中 Rust nextest **5185/5185 passed**，fmt、前端、
    lint、generated type gate 全绿（由 Reporting baseline owner 记录）。
  - `cargo nextest run -p golish-db -E 'test(terminalizer_replay) |
    test(candidate_attempt_requires_exact) | test(finding_lineage) | test(verification_truth)'
    --no-tests=fail --status-level fail` → **3/3 passed**，run
    `f64ff9df-447e-458a-83ce-1ec28ebba5b0`。
  - `cargo nextest run -p golish-agent-kit -E 'test(verification_gate) |
    test(candidate_disposition) | test(candidate_v2_stage_metadata) |
    test(verification_metadata) | test(stage_findings_policy) | test(verification_pass)'
    --no-tests=fail --status-level fail` → **13/13 passed**，run
    `1088b9bc-be51-490f-ac8b-1cac86b6690a`。
  - `cargo nextest run -p golish-agent-app -E 'test(candidate_attempt) |
    test(attack_stage_findings_policy) | test(vuln_stage_keeps_findings) |
    test(submit_uses_trusted_execution)' --no-tests=fail --status-level fail` → **3/3 passed**，
    run `9eea5903-b06f-48f1-a976-5aea6700d73f`。
  - `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets --
    -D warnings` → exit 0；`python3 -m unittest scripts.tests.test_check_repo_ownership -v`
    → 1/1 passed；`python3 scripts/check_repo_ownership.py --finding-writes-only` → exit 0；
    三份 attack stage spec + phases JSON parse、`python3 scripts/check_dag.py`（58 crates）、
    scoped `git diff --check` 全 exit 0。
  - 安全复核 fresh focused GREEN：`golish-db` terminalizer/CVSS/lineage/evidence freeze、partial
    current-wave、reason-only blocked → **3/3 passed**，run
    `ad62dcd9-64e4-47b7-953e-3e7696d349d2`；`golish-agent-kit + golish-agent-app`
    authority completeness、双 contract、submit preview → **4/4 passed**，run
    `63b28949-5891-4853-813e-d0b4c66cd45a`；`golish-agent-runtime` 双 contract specialist
    与 V2 vuln-scanner writer hiding/legacy retention → **3/3 passed**，run
    `b7dde43d-ce45-4bc0-a32b-1c5d25cba586`。
  - 最后一条 dispatch seam 按 TDD 收口：RED 定向 nextest compile exit 101，首错为缺失
    `effective_stage_run_specialist` / resume 仍使用旧签名；接线 fresh prompt、resume force、
    persisted-double-contract tool-list 与 durable `attack_analyst` worker mapping 后 GREEN
    **3/3 passed**，run `f7e353ba-be23-4d8e-853b-31de4b1ac57c`。随后双 contract、authority、
    submit preview、vuln writer boundary 聚焦 **9/9 passed**，run
    `72f1a527-a6f1-4784-b62a-a6f16dcd8c57`；DB terminalizer/partial sibling/reason-only
    **3/3 passed**，run `99d23b1b-a1c7-4348-abda-59b438033a9e`。
  - 全包回归暴露两条旧 fixture 仍在 terminal 后追加 Attempt evidence / 重用 attempt ordinal；
    fixture 已改为 running 时绑定 evidence、合法 unique ordinal 后 terminalize。组织删除又验证
    lineage-bound Finding 需要保留既有 FK live-pointer nulling：实现只允许 nested FK trigger
    把 legacy `target_id` 置空，frozen lineage target identity 不变，direct UPDATE 仍 P0001。
    exact fixture GREEN **2/2**，run `f80bf2bc-668e-484a-a505-37d6334ae185`；最终五包
    Candidate focused GREEN **18/18**，run `163f7fd4-dd31-4674-a0dd-40413f7aa699`；
    `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app
    -p golish-sub-agents --all-targets -- -D warnings` → exit 0。Finding write ratchet、JSON parse、
    全树 `git diff --check`、`scripts/check_dag.py`（58 crates）同样 exit 0；P1 foundation
    SHA-384 现场重算仍为冻结值 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`。
- **当前状态 / 下一步**：Task 8/9 已实现并有 fresh focused + full baseline 证据，但整个 feature
  必须保持 `in_progress`。Task 10 的正确实现需要新增 exact FactDelta entry provenance schema；
  未获确认前停在这里，不用复用首波 handoff 的假 lineage 冒充多波完成。
- **提交记录**：本轮未 commit、未 push；共享工作树仍含所有并行 V2 改动。

#### 2026-07-12 · Runtime memory and Candidate pipeline V2 implementation

- **本轮目标**：按用户明确授权实施
  `docs/design/2026-07-12-runtime-memory-candidate-pipeline-v2.md` 与八包路线图，先完成
  P1 Runtime Foundation，再按依赖推进 Candidate V2、Memory Fabric、KG/RAG、typed
  post-exploit、cleanup 和 reporting。
- **开工状态**：原工作树位于 `main` 且已有 155 个 tracked 脏文件、33 个 untracked
  文件；已原样切到 `codex/runtime-memory-candidate-v2`，未回滚、暂存或提交既有改动。
  旧 target-surface feature 已转 `blocked` 等待单独 live acceptance，V2 成为唯一
  `in_progress`。
- **运行过的验证 / 已记录证据**：`./init.sh` → exit 0；fmt、check-fe、test-fe、
  lint-rust、test-rust-all、check-types 全绿。该结果只证明开工基线，不证明 V2 已实现。
- **checkpoint**：按用户“可以先 commit”的指令，先跑 `just precommit` → exit 0、
  `✓ All checks passed!`，修正两处文档 trailing whitespace 后完成整树 checkpoint：
  `ab7b0c4a feat(harness): checkpoint deterministic stage closure and V2 design`（188 files，
  29012 insertions / 3001 deletions）；分支 `codex/runtime-memory-candidate-v2`，未 push。
- **P1 TDD / 守卫基线**：四态 rollout policy、operation contract 持久化、operation insert
  fail-closed、canonical workspace identity 均已先 RED 后 GREEN；`cargo check -p
  golish-agent-app -p golish-agent-runtime -p golish-agent-bridge -p golish` → exit 0。
  `python3 scripts/check_repo_ownership.py` 当前既有基线为 `174 ownership + 14 raw-sql`
  violations（exit 1）；本轮新 repo 模块必须全部登记且不得增加该计数。
- **P1 foundation 当前证据**：`20260712000001_runtime_memory_foundation.sql` 经 legacy /
  hostile / concurrency 审计后冻结，SHA-384
  `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`；
  exact embedded migration suite 由最初 4 GREEN → 11 expected RED → 当前 30 tests（含冻结
  checksum）全绿。覆盖 legacy duplicate/unknown stage status、direct rollout jump/delete、
  nullable FK bypass、cross-worker/submission/handoff、scope seal/worker lease 并发锁、Scoping
  tool→submission 顺序 bind、payload immutable 与 durable history RESTRICT。
- **P1 atomic operation 当前证据**：golish-db focused `runtime_memory_store` 8/8、app bridge
  4/4、orchestrator atomic failure 1/1、project-scope authorization 1/1 均 GREEN；GUI/CLI fresh
  path 使用 canonical workspace registration，Task+operation 同 transaction，resume 对比 frozen
  `project_scope_id`（仅 LegacyV1 NULL binding 兼容）。`cargo check -p golish-agent-app -p
  golish-agent-runtime -p golish` → exit 0。
- **安全边界**：本次实现授权覆盖设计中列明的 additive migration、`golish-db` 和 IPC；
  不授权真实扫描、exploit、外部 embedding、Graphiti 或付费 API 请求。
- **下一步最佳动作**：完成 P1 当前树审查，按 TDD 从 rollout mode contract 和 additive
  runtime schema 开始。
- **提交记录**：`ab7b0c4a` 已 commit；未 push。P1 后续实现尚未提交。

#### 2026-07-12 · Enumeration browser stale recovery terminal closure

- **本轮目标**：按用户授权修复最新 run `pentest-chat-1783829178322-1` 中
  `https://moresec.cn:443` 的 JS/PARAM/JSAPI 长期停在 partial、导致 Enumeration
  `submit_stage_deliverable` 无法通过的问题；保持真实重复失败 breaker 与 evidence/gate
  语义不变。
- **根因**：第一次 `script_body` 读取失败会记录 failure 并把所属 page 放进 checkpoint；
  后续 page 已被消费且 script 已由同 provenance manifest 成功缓存时，浏览器不再产生该
  script response，因此既不增加 failure count，也不调用 success clear。最终形成
  `pending_pages=[] + recovery_failures[count=1] + recovery_pending` 的零网络空 resume；现场
  `checkpoint_resume_count=13` 仍 `recovery_exhausted=false`。
- **已完成**：恢复初始化在载入同 provenance cached scripts 后，清除已有成功落盘 entry
  对应的 stale `script_body` failure；无成功 cache 的真实重复 failure 仍在第二次达到
  `recovery_exhausted=true`。新增现场形状回归测试，并同步 pentest bridge 模块卡和索引。
  这是简单局部 bugfix，未切换或新增 `feature_list.json` 条目；唯一 `in_progress` feature
  保持不变。
- **运行过的验证 / 已记录证据**：
  - TDD RED：`node --test --test-name-pattern='cached script clears a stale script-body recovery failure' scripts/browser_collect_js_api.test.mjs`
    → exit 1，`actual='partial'`、`expected='complete'`。
  - TDD GREEN：同命令 → 1/1 passed；既有
    `repeated navigation failure exhausts one retry signature without a third request` → 1/1 passed。
  - `node --test scripts/browser_collect_js_api.test.mjs` → exit 0，20/20 passed。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api --status-level fail`
    → exit 0，run `1a1830be-f4b7-4c7a-9648-22145dbf`，43/43 passed。
  - `node --check scripts/browser_collect_js_api.mjs`、对应 test 文件与 `git diff --check`
    → exit 0。Biome 明确忽略 `scripts/*.mjs`，因此其 no-files exit 1 不作为代码失败。
  - `just precommit` → exit 0，`✓ All checks passed!`；fmt、check-fe、test-fe、lint-rust、
    test-rust-all、check-types 全绿。
- **提交记录**：未 stage、未 commit、未 push；用户未要求提交。
- **已知风险 / 未解决问题**：旧 live run 的 partial outcome 不会因源码修改原地改写；必须
  重启/重编译 backend 后 fresh resume/rerun 才能生成 terminal outcome。最终 submit payload
  还应删除 `enumeration_blocked` claim 的空 `technique` 字段，否则即使 coverage 关闭也会被
  taxonomy 校验拒绝。
- **下一步最佳动作**：重启 backend/app，在 Test1 对同 operation 或 fresh Enumeration
  重跑 `https://moresec.cn:443`，随后用
  `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db`
  核对三格转 terminal、`ready_to_submit=true`，并以合法 claim payload 重新提交。
- **以下当前任务文件已修改但未提交**：
  `scripts/browser_collect_js_api.mjs`、`scripts/browser_collect_js_api.test.mjs`、
  `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`、
  `agent-progress.md`。仓库其余大量脏改动均为既存用户/其他会话工作，本轮未回滚或改写。

#### 2026-07-12 · Target Surface fingerprint / three-failure exclusion closure

- **本轮目标**：按用户“直接修闭环”的授权，同时修复截图中的
  `directory_entry_list` ambiguous `id`、Target Surface 单源失败清空全部数据、
  IP Target 指纹缺少可见入口/exact-origin 归属，以及 exact Web Origin 网络/TLS
  失败的三次重试、独立复核和 Enumeration 排除闭环。
- **开工诊断**：live DB 只读核对确认 `115.175.6.207` 已有 25 条指纹，
  `ti.moresec.cn` / `webshell.moresec.cn` 各有 13 条，数据并非未落库；JOIN 查询
  使用未限定 `id`，而 frontend 8-source `Promise.all` 会让该单点错误清空成功返回的
  fingerprints/origins/hierarchy。WhatWeb evidence 还缺 exact origin、旧 object evidence
  被前端丢弃、多 origin 只挂首项、0..1 confidence 被显示成 0..1%。
- **合同/计划**：新增
  `docs/design/2026-07-12-target-surface-fingerprint-network-failure-closure.md` 与
  `docs/superpowers/plans/2026-07-12-target-surface-fingerprint-network-failure-closure.md`；
  计数严格绑定 operation epoch/org/target/exact-origin/technique/failure-class。第 3 次只停止
  WhatWeb；只有固定独立 transport probe 也不可达，才写 trusted handoff 并仅从
  Enumeration 排除该 exact origin。绝不删除资产或把 open port 改 closed。
- **已完成**：
  - `directory_entries` current-owner JOIN 全量改成 `de.<column>` 投影；Target Surface
    八个 optional source 改成逐源 settle，单源失败保留 fingerprints/origins/backend
    hierarchy，并返回精确 `sourceErrors`。
  - WhatWeb fingerprint evidence 记录 canonical exact origin/root/original URL，并在同一
    fingerprint identity 上去重保留多 origin observations；legacy object evidence 继续读。
    IP Surface 新增 `Fingerprints` 页，分开显示 `Web Origin fingerprints` 与
    `Target-level / unassigned fingerprints`，confidence 兼容 `0..1` 与旧百分数。
    nmap/non-WhatWeb evidence 保持顶层 object/port 合同。
  - EAS WhatWeb 只统计完整归因的 connect/timeout/refused/reset/EOF/TLS failure；同
    operation EAS epoch/org/target/exact-origin/technique/failure-class 的 attempt 1/2 写
    retryable `error`，attempt 3 才封 producer。成功清 slot，未知 stderr、truncation、
    tool/config/DB error 不计数；同 origin 执行锁覆盖网络与 publication，跨 session
    continuation 在 launch 前复用 seal 并补当前 run outcome，不会产生 attempt 4。
  - 第三次失败复用固定 direct/proxy HEAD→GET-Range 独立探针；任一 HTTP response 只封
    WhatWeb。只有全部可用策略仍失败且 independent guarded evidence 落库，Enumeration
    才按同 operation/org/target/canonical origin 排除。target/open port/兄弟 Host/SNI
    均不删除、不降级。operation-aware coverage 已贯通 worklist、submit preview、Task
    close、org gate 与 runtime stage_run。
  - Target Evidence tab 用现有 target timeline 安全解析 WhatWeb transport payload，显示
    failure class、Attempt N/3、producer error/blocked；第三次说明 WhatWeb 已停止，且只在
    `independently_confirmed=true` 时明确显示 exact origin 已从 Enumeration 排除。
    `detail.subject` 参与 origin badge 归属，普通/损坏 JSON evidence 保持原样。
- **运行过的验证 / 已记录证据**：
  - TDD RED：qualified JOIN、directory rejection 保留 sibling data、legacy/current
    fingerprint evidence/多 origin/confidence、attempt 1/2/3、independent-only exclusion
    断言在实现前分别失败；修复后进入以下 GREEN。
  - `cargo check -p golish-db -p golish-pentest -p golish-pentest-app -p golish-agent-app -p golish-agent-kit -p golish-agent-runtime` → exit 0。
  - `cargo nextest run -p golish-db -E 'test(directory_entry) | test(operation_state)' --status-level fail` → 9/9；配置本机 migrated PG 后同一 suite 对 counter/clear/seal/producer read/handoff read/checkpoint write/stage transition 共 7 条 SQL `EXPLAIN` 全通过。另以当前 embedded PG 执行 qualified directory query → `QUALIFIED_DIRECTORY_QUERY_OK 0`，无 ambiguous column。
  - `cargo nextest run -p golish-pentest output_store --status-level fail` → 63/63；完整 crate 回归曾通过 199/199（7 skipped），WhatWeb SQL 另做只读 PG `EXPLAIN_OK 10`。
  - `cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail` → 46/46；`cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 90/90；`cargo nextest run -p golish-agent-kit -E 'test(external_attack_surface) | test(enumeration) | test(web_origin)' --status-level fail` → 93/93。
  - `pnpm exec vitest run frontend/components/TargetPanel frontend/lib/api/security-analysis.test.ts` → 14 files / 108 tests passed；Evidence status TDD 首轮 2 RED/1 pass，随后 `EvidenceTab.test.tsx` → 3/3；`pnpm typecheck` → exit 0；relevant Biome checks → no fixes/errors。
  - six affected crates `cargo clippy ... --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`jq empty`（stage spec + feature list）与 `git diff --check` 均 exit 0。
- **提交记录**：本轮未 stage、未 commit、未 push；用户没有要求提交。
- **已知风险 / 未解决问题**：未跑真实外网 EAS，也未跑 full `just precommit`，所以 feature
  保持 `in_progress`。Target Evidence 已显示 operation-scoped失败/剔除；独立 GUI
  coverage matrix command 没有 operation context，因此矩阵本身仍不显示该 exclusion
  diagnostic，真实 worklist/submit/gate/stage_run 全部已带 operation id。WhatWeb per-origin
  序列锁是单 desktop backend 进程内合同；未来多进程
  executor 需要升级 DB generation/CAS。breaker namespace 达 512 slots/256 KiB 时安全返回
  retryable error，不会误 terminal。
- **下一步最佳动作**：重启/重新编译 backend 后做一个受控 EAS live acceptance，核对同一
  exact origin 的 attempt 1/2=`error`、attempt 3 producer seal + independent decision，随后
  检查 Enumeration worklist 确实只遗漏 independently-confirmed origin，且 `targets` 与
  `ports[].state=open` 原值不变；若用户解除当前验证限制，再跑 full `just precommit`。
- **以下当前任务文件已修改但未提交**：
  `backend/crates/golish-db/src/repo/{directory_entries,operation_state}.rs`；
  `backend/crates/golish-pentest/src/output_store/targets.rs`；
  `backend/crates/golish-pentest-app/src/pentest_bridge/{eas_capabilities,enum_preflight_web_origins}.rs`；
  `backend/crates/golish-agent-app/src/ai/{commands/stage_coverage,db_bridge/{recon,mod,evidence},harness_submit_tool}.rs`；
  `backend/crates/golish-agent-kit/src/{db_traits/repo,harness/{org_gate,resources,stage_spec},tool_executors/security,task_orchestrator/subtask_phases/execute}.rs`；
  `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`；
  TargetPanel hook/hierarchy/fingerprint/Evidence tab/API files及测试；EAS/Enumeration methodology、
  EAS spec、相关模块卡/INDEX、本 design/plan、`feature_list.json` 与本记录。其余脏树改动为
  既有用户/前序会话内容，本轮未回滚。

#### 2026-07-12 · EAS WEB target-side transport terminal recovery

- **本轮目标**：修复 live EAS `pentest-chat-1783829178322-1` 最后两个 exact Web
  Origin 因 WhatWeb EOF / TLS reset 无法产生 guarded WEB 终态、导致 gate 永久停在
  306/308 的合同死锁。保持 exact-origin gate 严格，不接受模型自报或 LIVENESS
  evidence 冒充 WEB。
- **设计 / 计划**：
  `docs/design/2026-07-12-eas-web-origin-transport-terminal-recovery.md`、
  `docs/superpowers/plans/2026-07-12-eas-web-origin-transport-terminal-recovery.md`。
- **已完成**：
  1. WhatWeb exit 1 现在只对 ANSI-stripped、exact authorization 匹配且 reason
     命中 target-side EOF/reset allowlist 的 `ERROR Opening:` 做逐 origin 分类；
     每个 batch member 都必须由 stdout record 或精确错误覆盖。成功 siblings 正常
     structured landing，失败 origin 单独写 target-bound guarded `blocked` evidence /
     outcome；unknown/runtime/timeout/truncated/unbound/missing-member 继续 fail closed。
  2. 全部 landing/evidence/outcome durable write 成功后，外层 wrapper 返回 complete，
     同时保留 `wrapped_exit_code` / `wrapped_stderr` /
     `terminal_blocked_origins` 审计；任一持久化失败仍非终态。
  3. DB bridge、coverage read model、exact-origin barrier、org gate 与 rule engine
     只接受 WhatWeb producer-owned blocked：tool/kind/source、current owner、exact
     scheme+host+port、technique/outcome/positive evidence id 必须完整一致。模型自报、
     LIVENESS、host-level 或其他 EAS blocked 都不能关闭 WEB gate。
  4. read model 新增 `blocked_origins`；全部 exact origins 终态时，任一 found 保持
     parent found（blocked 仍可见），无 found 但有 blocked 为 blocked，全 empty 才是
     checked_empty。方法论、三张模块卡与 INDEX 已同步。
- **TDD RED 证据**：pentest-app 新 parser tests 因 classifier/blocked verdict 不存在
  编译失败 exit 101；agent-kit 新 gate tests 11 passed / 3 failed、agent-app 新投影
  tests 2 failed，均在实现前复现缺口。
- **GREEN / 验证证据**：
  - `cargo nextest run -p golish-pentest-app -E 'test(/pentest_bridge::eas_capabilities::tests/)' --status-level fail`
    → 42/42 passed。
  - `cargo nextest run -p golish-agent-app -E 'test(/ai::commands::stage_coverage::tests/) | test(/ai::db_bridge::evidence::tests/)' --status-level fail`
    → 102/102 passed。
  - `cargo nextest run -p golish-agent-kit -E 'test(/harness::gate::eas_web_origin_check::tests/) | test(/harness::org_gate::tests/) | test(/harness::gate::rule_engine::tests/)' --status-level fail`
    → 165/165 passed。
  - `cargo clippy -p golish-pentest-app -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings`、
    backend workspace `cargo fmt --all --check`、JSON parse、`git diff --check` 均 exit 0。
  - 独立只读审查发现并驱动修复两个额外 fail-open：reason 子串误匹配、exit 1
    全 stdout/零 blocked 误归一；两项均新增 RED→GREEN guard。最终复审其余
    wrapper→evidence→coverage→gate 链路无新增 blocking finding。
- **风险 / 下一步**：没有 schema/migration/generated IPC 变化。按既有用户约束未跑
  `./init.sh` / full `just precommit`，也未发起真实外部扫描。需要重启/加载新 backend
  后由用户授权 fresh EAS continuation，确认两个 live origins 形成 guarded blocked 且
  final gate PASS；在此之前 feature 保持 `in_progress`，不宣称产品验收完成。
- **未提交的半成品**：本轮 6 个 Rust seam、EAS methodology、3 张模块卡、INDEX、
  design/plan、feature/progress 均已修改但未提交；工作树另有用户既存大批改动，未回退。

#### 2026-07-12 · EAS WEB gate / repair / continuation closure

- **本轮目标**：修复 live continuation `pentest-chat-1783829178322-1` 已新增
  WhatWeb evidence 却仍卡 gate 的三层闭环：asset-level terminal exception 让
  preflight 假绿、repair worker 改写 DB exact-origin 参数、以及
  `retry_budget_exhausted` 后自动 turn 继续重复 check/submit。
- **现场基线**：显式“继续”确实派发一次 Prober 并新增 18 条 WhatWeb evidence；
  无 terminal exceptions 的真实 coverage 仍为 308 cells / 288 done / 11 partial /
  9 pending，20 个 IP 共缺 188 个 exact origins。后续自动 turn 均
  `reentry_blocked=true`，没有再次派发 specialist；asset-level blocked preview
  虽显示 `ready_to_submit=true`，但标明 `preview_only=true` / `persisted=false`，
  final exact-origin gate 继续正确 BLOCK。
- **设计 / 计划**：
  `docs/design/2026-07-12-eas-web-origin-gate-recovery-closure.md`、
  `docs/superpowers/plans/2026-07-12-eas-web-origin-gate-recovery-closure.md`。
- **已完成**：
  1. `check_stage_asset_coverage` 对 `details.missing_origins` 非空的 EAS WEB
     parent exception 逐项 reject：不改 snapshot、不进入 `coverage_to_submit`，并返回
     `rejected_terminal_exceptions`；其他 EAS technique exception 仍可 accepted。
  2. repair mode 新增可持久化 `eas_web_repair_targets` exact lock。成功刷新
     `stage_worklist_next` / `check_stage_asset_coverage` 后，只允许 DB 页中的
     `target_id + canonical target_url` object，bare string 也必须精确匹配 origin；
     scheme/host/port 偏移均 fail closed。final barrier 对每个缺失 origin 生成
     `missing_exact_origin` action，避免 parent cell 全 partial 后 repair 无入口。
  3. `AgentExecutor` 暴露 request-scoped `StageRunReentryGuard` 状态；orchestrator 的
     text-only 与 gate-BLOCK reflector 都在 budget exhausted 后停止自动重启，同时
     保留最终 deterministic BLOCK。新的显式用户 request lease 仍 reset budget。
  4. worklist/coverage 刷新的 exact lock 在同一 assistant tool-call batch 内立即生效；
     bounded empty page 不会清掉既有锁，只有显式 `ready_to_submit=true` 才能关闭。
     runtime observer 用真实 `tool_call_id` 把 refined mode 原位写回 `agent_run`
     checkpoint，保留 repair directive/runtime corrections/sibling state；无 execution
     monitor 但有 operation DB sink 时也会持久化。后续 stage retry 仅在新旧 WEB
     action identity 集合一致时继承；缺口 A 变 B 会丢弃 A 的 stale lock 并强制刷新。
  5. 15:06 live continuation 证明新 binary 已加载并把 exact-origin 缺口从 188 降至
     80，但暴露同 worker 状态覆盖：15:15 worklist 已建立 exact lock；随后重复 submit
     `needs_fix` 把它重置为 host-level，导致 15:16:22 模型终于照抄的 5 个正确
     `{target_id,target_url}` 仍被要求重新刷新。executor 的 submit update、runtime
     durable checkpoint 与 stage-run retry 现统一复用 same-gap retain helper；gap identity
     不变就保留 exact lock，变化才 fail closed 丢锁。
- **TDD RED 证据**：
  - preflight 假绿 run `a0b19bc6-7ea5-46bc-9853-c5a02e0da2b8` exit 100；
    reject 契约 run `ae38f305-0be6-4f33-bc86-b6ec0a82a9a5` exit 100。
  - host-level repair 错放行 scheme 改写 run
    `b2cda7e6-9d53-44c7-9556-6e1cd1330cdd` exit 100；exact barrier recovery
    action 缺失 run `62c7984a-3b66-48c1-9c19-8de966ac4cbd` exit 100。
  - auto-stop kit / bridge RED 均 exit 101：分别缺 retry policy helper 与
    `AgentExecutor` request-scoped exhausted signal。
  - repeated `needs_fix` exact-lock RED run
    `f30864b5-6ebd-4e03-856d-bc23f5af0eda` exit 100：实际得到 `None`，预期保留
    worklist 的 `Some([target_id,target_url])`。
- **GREEN / 已记录证据**：
  - 主代理四 crate 组合回归 run `9f5ca240-70bc-46ef-93e0-218b1fdfbd84`：
    9/9 passed、1473 skipped。
  - sub-agents 全套 182/182（run `2d08bf44-e8d7-4521-b831-15d6a680f7d6`）；
    agent-kit + agent-bridge 全套 965/965（run
    `c5853728-6baf-416c-b279-e26c22a89584`）。
  - preflight security 38/38；repair 14/14；exact-origin gate 4/4；
    auto-stop focused 3/3。主代理四 crate `cargo clippy --all-targets -- -D warnings`
    exit 0；`cargo fmt --all -- --check`、feature/spec JSON、exactly-one-in-progress
    jq 与 `git diff --check` 均 exit 0。
  - repair-lock liveness + stale-action focused 5/5（run
    `fd41b4cf-9e90-4bf0-84c3-833c349c1bb1`）；no-monitor observer/checkpoint
    focused 3/3（run `8ff63e59-321f-4035-bb80-1b72dec6588d`）；sub-agents +
    runtime 全套 521/521（run `42b0c009-8c3d-4e7f-967e-98c6ccb635e8`）；最终四
    crate 1488/1488（run `c5fbea93-4014-4dc6-8982-98e92c74d76a`）。最终 scoped
    all-target Clippy `-D warnings` 与 rustfmt check exit 0。
  - 独立只读复核在 stale-action 与 no-monitor 回归落地后确认无新增
    P0/P1/P2 blocker；未修改文件。
  - repeated-needs-fix / durable / stale-action focused 4/4（run
    `8e3bbfba-a0bd-48fb-b6b6-4de0e790c447`）；sub-agents + runtime 524/524
    （run `39ba57bd-5e8d-4672-b2f1-be7358ec0280`）；最终四 crate 1489/1489
    （run `47310cec-b937-404e-8a27-7a8b57e5209f`）；最终 scoped all-target Clippy
    `-D warnings` exit 0。独立只读复核确认三条路径规则一致且无新增 P0/P1/P2。
  - `cd backend && cargo build -p golish` exit 0（1m09s）；最终 app binary mtime
    `2026-07-12 15:28:13`，晚于本次 source mtime。检查时 Tauri/Vite/golish 均未运行、
    1420 无监听，因此下一步是正常重新打开应用，不是新建任务。
- **提交记录**：未 stage、未 commit、未 push。
- **本轮新增/修改但未提交的路径**：
  `backend/crates/golish-agent-kit/src/{harness/gate/eas_web_origin_check.rs,tool_executors/security.rs,task_orchestrator/{types.rs,stage_refiner.rs,subtask_phases/execute.rs,subtask_phases/execute_harness_loop_tests.rs}}`、
  `backend/crates/golish-agent-bridge/src/{bridge_executor/trait_impl.rs,agent_bridge/task_request.rs}`、
  `backend/crates/golish-sub-agents/src/{lib.rs,executor_types.rs,executor/response_parsing.rs}`、
  `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{sub_agent_call.rs,stage_run_call.rs}`、
  EAS methodology、相关模块卡/索引、本轮 design/plan、`feature_list.json` 与本文件。
  工作树其余大量修改均为既有用户改动，未清理、未覆盖。
- **已知风险 / 下一步最佳动作**：running backend 不会热替换已加载 binary/worker
  chain；需确认 watcher 已重编译本次 repeated-needs-fix 修复，必要时重启 backend，
  然后在**原 Golish 对话**发送一条新的显式“继续”以获得 fresh request budget；不需要
  新建任务/会话。同批 worklist refresh 及同-gap needs_fix 后的 wrapper 现在都继续使用
  exact lock；取消/恢复或下一次 stage retry 也从 durable checkpoint 继续。当前 feature
  继续 `in_progress`，直到 fresh live EAS 得到 pass token；按用户此前约束未跑
  `./init.sh` / 全量 `just precommit`，也未由本开发会话发起真实外部扫描。

#### 2026-07-12 · EAS WEB exact-origin invocation closure

- **本轮目标**：修复 live run `pentest-chat-1783829178322-1` 中 Prober 已收到
  DB 权威 `https://IP:443` origin，却手工拼成 `http://IP:443`，导致
  `eas_fingerprint_web_stack` 在网络启动前报
  `target_url is not bound to one authorized in-scope target` 并连带跳过整批的问题。
  按用户此前约束未跑 `./init.sh` / 全量 `just precommit`，也未发起新的真实扫描。
- **现场证据 / 根因**：12:33:15 的 coverage snapshot 已明确返回
  `details.missing_origins=[https://113.240.117.106:443,...]`；12:34:33 Prober
  却传入 `http://113.240.117.106:443`，11ms 内被 exact-origin guard 拒绝。
  DB 核查确认截图中的 5 个 IP target 均为当前 org/workspace、`scope=in`、唯一行，
  且所有 443 origin 都是 HTTPS；三个 HTTP/80 sibling 本身合法，只因批次 fail-fast
  未启动。`stage_worklist_next` 当时丢失 cell `details`，repair guard 还会拒绝 wrapper
  已支持的 `{target_id,target_url}` object，Prompt 也没有禁止从端口猜 scheme。
- **已完成**：
  1. EAS WEB coverage 现在生成
     `details.recommended_args.target_urls=[{target_id,target_url}]`，只包含尚未完成的
     DB canonical origins；`stage_worklist_next` 原样保留 `details/recommended_args`，
     WEB focus/contract 明确禁止重建 scheme。
  2. Prober Prompt、repair directive 与 EAS methodology 要求优先原样复制
     `recommended_args.target_urls`，否则使用同 item 的 `target_id + missing_origins`；
     coverage-gap repair 允许 object entry，同时仍按 action asset 做围栏并由 wrapper
     做最终 owner/origin 校验。
  3. 通用 exact-origin resolver 保持严格。仅 `eas_fingerprint_web_stack` 可在 DB
     证明当前 org/workspace/scope 中恰好一个 confirmed HTTPS origin 与错误的
     `http://IP:port` 具有同 IP、同 effective port 时进行 HTTP→HTTPS 纠正；不依据
     443 推断、不改 host/port、不降级 HTTPS、不采用 alias。纠正后再次经过 strict
     authorization，WhatWeb input、launch guard、landing、evidence/outcome 使用同一
     effective origin，并返回 `target_url_corrections` 审计信息。
- **TDD RED 证据**：
  - Prober prompt focused test：exit 100，缺少 `details.missing_origins`。
  - repair object exact test：exit 101，`{target_id,target_url}` 被错误 block。
  - agent-app WEB coverage test：exit 100，`details.recommended_args.target_urls` 为 Null。
  - agent-kit worklist test：exit 100，worklist item `details` 为 Null。
  - pentest-app reconciliation focused build：exit 101，12 个预期缺失实现符号。
- **GREEN / 已记录证据**：
  - pentest-app reconciliation focused：5/5；`eas_capabilities`：39/39。
  - agent-app `stage_coverage`：87/87；agent-kit `stage_worklist`：5/5。
  - sub-agents prompt/object/string focused regressions：全部通过。
  - 主代理四 crate 合并回归：run `356772e3-9da5-4ca7-b718-da68ca24f315`，
    9/9 passed、1705 skipped。
  - scoped Clippy（四个相关 crate）exit 0；`cargo fmt --all -- --check`、
    `python3 -m json.tool feature_list.json`、exactly-one-in-progress jq 与
    `git diff --check` 均 exit 0。
- **提交记录**：未 stage、未 commit、未 push。
- **本轮新增/修改但未提交的路径**：
  `backend/crates/golish-pentest-app/src/pentest_bridge/{target_resolver.rs,eas_capabilities.rs}`、
  `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、
  `backend/crates/golish-agent-kit/src/tool_executors/security.rs`、
  `backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、
  `resources/harness/stages/external_attack_surface/methodology.md`、相关 `docs/modules/`、
  新 design/plan、`feature_list.json` 与本文件。工作树其余大量修改属于既有用户改动，
  未清理、未覆盖。
- **已知风险 / 下一步最佳动作**：旧运行中的失败批次不会自动恢复，当前 feature
  继续 `in_progress`。重新编译/重启 backend 后，用同一授权 org 做一次 fresh EAS
  WEB batch acceptance，确认 worklist 直接给出 HTTPS object、WhatWeb 不再因可纠正
  scheme 失败、`target_url_corrections` 可审计，并从 DB 验证每个 exact origin 的
  fingerprint/evidence/technique_outcome；歧义/错 host/错 port 负例仍应零启动。

#### 2026-07-12 · EAS SERVICE-FINGERPRINT outer-timeout / trusted Nmap landing closure

- **本轮目标**：修复最新 EAS run `pentest-chat-1783826001542-1` 中
  `eas_fingerprint_services` 被 Prober 固定 300 秒外层 timeout 截断，以及后续真实
  Nmap service/version stdout 被误识别为 Naabu、无法 guarded landing 的两个独立故障。
  按用户此前约束未跑 `./init.sh` / 全量 `just precommit`。
- **现场证据 / 根因**：32-IP wrapper 在 `11:20:53` 发起、`11:25:53` 精确 300 秒
  返回 `Sub-agent tool 'eas_fingerprint_services' timed out after 300s`；Prober
  `idle_timeout_secs=300` 被 executor 复用为普通 tool wall-clock timeout，而四个
  `eas_*` wrapper 不在 long direct-tool exemption。wrapper 同时按不同 DB open-port
  集合串行运行 nmap，每个底层调用默认约 120 秒。拆小批后 Nmap 已输出 OpenSSH/nginx/
  OpenResty 等有效服务，但 generic detector 先把 banner 的 `HH:MM` 命中 Naabu 的宽泛
  `host:port` regex，结果为 `wrapped output was not recognized as structured EAS data`、
  `outcome_persisted=false`、SERVICE evidence/outcome 仍为 0。
- **已完成实现**：
  - 四个 guarded EAS wrapper 全部绕过 sub-agent 通用 outer tool timeout；shared
    cancellation/User Stop 与各工具自身 bounded command timeout 继续生效。
  - `eas_fingerprint_services` 未显式传 timeout 时给每个底层 Nmap batch 注入 600 秒；
    显式较小 timeout 原样保留，`background=false` / `__foreground_only=true` 不变。
  - `StoreContext` 新增 `trusted_tool_id`；guarded EAS landing 传入
    `wrapped_tool_name`，output-store 精确加载同 id toolsconfig，缺失时 fail closed，
    不再回退 heuristic。普通 legacy/generic 调用仍保持 command→stdout 两阶段检测。
  - 已新增设计/计划，并同步 EAS methodology、sub-agent executor、pentest/output-store、
    pentest bridge 模块卡和模块索引；未改 schema/migration/generated IPC。
- **TDD RED 证据**：
  - `cd backend && cargo nextest run -p golish-sub-agents long_direct_bridge_tools_bypass_sub_agent_outer_timeout --status-level fail`
    → exit 100，断言 `eas_discover_ports should keep running...`，0 passed / 1 failed。
  - `cd backend && cargo nextest run -p golish-pentest-app service_fingerprint_runs --status-level fail`
    → exit 101，新 service helper 尚不存在（同时并行编译时 trusted field 尚未落下）。
  - `cd backend && cargo nextest run -p golish-pentest trusted_nmap_identity_wins_over_timestamp_that_matches_naabu --status-level fail`
    → exit 100，trusted Nmap result 在旧 heuristic 下返回 None。
- **GREEN / 已记录验证证据**：
  - 三 crate 合并 focused：4/4 passed，run
    `ed77f8e9-0921-47a7-9887-71c0b52ef346`。
  - `golish-sub-agents executor` → 112/112 passed，run
    `35faae3d-1c1d-403f-8406-4be60ff36eb9`；`golish-pentest output_store` →
    62/62 passed，run `093fe6ba-fe2e-4966-8693-031e61228035`；
    `golish-pentest-app eas_capabilities` → 37/37 passed，run
    `510e8f99-ab2f-4307-92be-7ed46f9827d0`。
  - `cd backend && cargo clippy -p golish-sub-agents -p golish-pentest -p golish-pentest-app --all-targets -- -D warnings`
    → exit 0，零 warning。
  - `cd backend && cargo fmt --all -- --check`、`python3 -m json.tool feature_list.json`、
    `git diff --check` → exit 0；feature `in_progress` count=1。
- **当前状态 / 风险**：代码与聚焦门禁已闭合，但当前正在运行/旧的 Golish backend
  不会热加载新二进制，且旧 EAS request 不会原地重放。尚未 fresh live 验证 Nmap
  service rows、fingerprints、evidence、technique_outcomes 与 32 个 SERVICE coverage cells
  真正落库，因此 feature 保持 `in_progress`，不虚报 passing。未执行标准启动检查或全量
  precommit，原因是用户此前明确要求跳过。
- **以下本轮文件已修改但未提交**：本轮在既有 dirty 文件中新增的实现位于
  `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、
  `backend/crates/golish-pentest/src/output_store/mod.rs`、
  `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`；另有
  `resources/harness/stages/external_attack_surface/methodology.md`、受影响模块卡/索引、
  `docs/design/2026-07-12-eas-service-fingerprint-runtime-closure.md`、对应 plan、
  `feature_list.json` 与本文件。工作树此前已有大量其它未提交改动，本轮未恢复或清理。
- **下一步最佳动作**：重启/重编 Golish backend 后 fresh 跑一次 EAS；用
  `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db`
  核对不再出现 outer `timed out after 300s`，并确认 Nmap 产出的 SERVICE rows、
  `source=nmap` fingerprints、evidence ids、terminal outcomes 与 coverage pending 数同步下降。
- **提交记录**：未 commit、未 stage、未 push。

### 2026-07-15 · Candidate Verification migration checksum drift 修复

- **本轮目标**：修复 Target 页触发 `organization_list: Database failed to start`；保留现有持久化数据，不删除 pgdata，不无条件篡改 migration metadata。
- **用户授权**：用户在确认根因是已执行 migration checksum drift 后明确回复“修啊”，授权本轮 DB/migration 修复。
- **根因证据**：backend startup 在 PostgreSQL ready 后先拒绝 `20260714000002`：数据库 checksum=`5228caa9...b80b855`、当前文件=`bedba079...9ab955b`。第一条修复后真实重启继续暴露 `20260714000003`：数据库=`43af87b8...9e6f8e60`、当前文件=`cc615057...887f4b58`。SQLx 每次只报告首个 drift；Target 页只是最先调用依赖 DB 的 `organization_list`。
- **schema 审计**：手动启动现有 embedded PG，仅做 catalog/data audit；临时库顺序应用到当前 `00002/00003`。`00002` 的 9 张相关表/约束/索引/trigger 与数据后置条件均一致，仅两个 Candidate 函数为旧定义。`00003` 的 catalog diff 证明旧库缺 `stage_team_recovery_decisions`、`stage_team_unit_gaps`、`stage_team_repair_generations`、deliverable submitter trigger/function，并有三个旧函数定义；缺失表不存在待迁移业务行。
- **修复实现**：`pool.rs` 只登记上述两个 version+description+exact old/new SHA-384 pair；`20260715000001` 重放两个 Candidate 函数，`20260715000002` 只安装 Stage Team catalog diff。任一不匹配、dirty row 或 forward SQL 失败仍 fail closed。设计与计划分别见 `docs/design/2026-07-15-candidate-verification-migration-checksum-repair.md`、`docs/superpowers/plans/2026-07-15-candidate-verification-migration-checksum-repair.md`。
- **TDD RED**：`candidate_verification_recovery_known_checksum_drift_is_exactly_repairable` 按预期因 `not explicitly allowlisted` 失败（nextest `4f7ad980-0cb2-4713-bd8d-e81f328da8d6`）；Candidate forward 文件缺失 RED 为 `87587f5a-30d6-45d2-a10f-6e3f31ed9b89`；Stage Team exact pair + forward objects 两个 RED 为 `df526bae-9234-443d-8ac6-d5856a546c3e`。首次 clean-install 集成跑出 4/4 RED（`75d369a6-d49b-4b05-b7e7-445aa3ef8a34`）：当前 `00003` 已建表，forward 初稿重复 `CREATE TABLE`；改成幂等 SQL 后，初稿→当前 checksum 未登记的 RED 为 `97f78ce1-16a7-4056-9fe8-2ad48b60db89`。
- **GREEN / clone / clean-install 验收**：最初 `pool::tests` 8/8 passed（`acfda881-e122-47fb-a9c4-079a614f9888`）。从真实库建立临时 clone 后，以 `psql -1 -v ON_ERROR_STOP=1` 顺序执行 missing `20260714000004`、`20260715000001`、`20260715000002` 全部 exit 0；Stage Team 11 张相关表 dump 与 14 个函数定义对 fresh audit 库的两个 `diff` 均 exit 0。幂等修复后的 `pool::tests` + Candidate/Stage Team clean-install integration 最终 13/13 passed，run `52bbe19d-dc71-4cdc-9b72-f9cd5fa2361f`。临时数据库已删除。
- **真实 DB / UI 验收**：15:14:14 开发应用真实启动同时修复 `00002/00003`，随后 migration complete。15:24:18 对 forward 初稿→幂等版本执行第三条 exact checksum repair；15:24:57 再次冷启动直接记录 `Database migrations complete`（没有 `Migration failed` / repair），随后 `Embedded PostgreSQL is fully ready`。`_sqlx_migrations` 中 `00002/00003/00004/15000001/15000002` 均 `success=true` 且 checksum 等于当前文件；三张 Stage Team repair 表均存在，`organizations` 可读且 count=5。15:14 后无新的 `organization_list/Database failed to start/Failed to load targets`；当前 dev app 保持运行。
- **环境/验证约束**：所有 Cargo 测试/build 前均执行 `just space-guard` exit 0；targeted `rustfmt --check` 与 scoped `git diff --check` exit 0。`cargo clippy -p golish-db --all-targets -- -D warnings` 被共享 dirty tree 既有 8 个告警阻断（`attack_candidate_work_items.rs`、`runtime_memory_tx.rs`、`stage_teams.rs`），本轮未顺手修改。曾按 AGENTS 启动 `./init.sh`，用户要求停止后立即 SIGINT，最终 exit 130；此后没有再次运行 init。未运行全量 `just precommit`，父 feature 保持唯一 `in_progress`。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-15 · Scoping slice 收口（用户确认的 CLI 公司名快通）

- **口径更正**：本任务早期“company-only `--org` 不预创建组织、与 GUI 对齐为 `UnconfirmedSubject`”的记录已被用户后续明确决策取代。fresh CLI `--org='广州有创网络科技有限公司'` 现在直接确认 organization identity，在当前 project 内 get-or-create exact root，并通过 shared typed launch 进入 `ConfirmedOrganizationIntake`；该 Scoping intake 不要求与 GUI prompt 交互对齐。
- **已完成行为**：公司名只确认组织，current-invocation target 仍为空，不从公司名/provider/历史 org 推导 domain/IP/CIDR/URL；custom `-e` objective 也会追加 trusted org name/UUID。`--auto-approve` 只可处理 exact target `scope_review` 和 context 中 `decision=subsidiary_scope` + exact seeded organization UUID 的 choice；默认选择 DB gate 可解析的“不纳入子公司（仅母公司）”分支。double-encoded context、宽泛“仅根组织/纳入控股单位”、generic confirmation/phase、ordinary choice、unit_review/credentials/freetext/unknown 全部 decline。Scoping create 不预冻结 runtime scope；trusted gate PASS 后、发布 `stage_passed` 前调用 V2 finalizer，原子绑定 decision/sealed snapshot/passed root Unit/trusted submission，同 submission 以 UUIDv5 稳定重放，预绑定 org、返回 identity 或存储任一异常都先 BLOCK。
- **主动边界**：`ConfirmedOrganizationIntake` 投影 `Some(false)`，TargetIntel→EAS 在读历史 target rows、phase approval、stage transition 与 executor/tool work 前 HOLD。headless exact resume 只在合法 marker 存在时恢复其值；marker 缺失一律收紧为 `Some(false)`，malformed 拒绝。不依赖非原子 post-create state-blob 写入，因为该路径在 V2Only 会零行更新却返回成功。安全代价是旧 exact-target operation 没有 marker 时也会 HOLD，需带本次 exact `--target` 新起 fresh run。
- **profile**：本 slice 和全部 fixture 固定 `red_team`，其他 profile 不计验收。
- **聚焦验证**：
  - `just space-guard` → exit 0。
  - `cargo nextest run -p golish-agent-app -p golish -p golish-agent-kit -E '<12 个 Scoping/typed launch/fresh+resume barrier/loopback tests>' --no-tests=fail --status-level fail` → 12/12 passed，run `eaf281bb-7b15-4e5f-9ccd-099c56bbe290`。
  - CLI typed subsidiary response 与现有 DB persisted parser 合同联合验证 → 2/2 passed，run `7397e3b5-680a-4e8c-9c0a-a9995c0cce3a`。
  - `cargo nextest run -p golish-agent-kit -E '<5 个 v2_scoping finalization tests>' --no-tests=fail --status-level fail` → 5/5 passed，run `2a455917-1d8a-4476-a723-6644877fe669`；覆盖首次冻结、同 submission exact replay、存储失败、返回 identity mismatch 与 CLI 预绑定 org mismatch。
  - `cargo nextest run -p golish-db --test runtime_scope_freeze -E 'test(finalize_scoping_scope_atomically_binds_submission_and_replays_without_closing_execution)' --no-tests=fail --status-level fail` → 1/1 passed，run `315bd2d6-be87-4511-af51-cc0daf83e888`。
  - 最终 `golish-agent-app + golish + golish-agent-kit + golish-db` Scoping/authority/finalizer 联合 selector → 20/20 passed，run `31c6372d-3123-4a26-bfd0-ba5912f0f207`。
  - AskHuman focused Vitest → 60/60 passed；相关 8 个前端文件 Biome → exit 0；`pnpm typecheck` → exit 0。Ask 模式不再倒计时自批；只有 Run Everything 的原始 low-risk typed choice/confirmation 可倒计时，phase/scope boundary 永远人审。
  - 7 个 Scoping Rust 文件 targeted `rustfmt --check`、scoped `git diff --check`、`feature_list.json` JSON parse 均 exit 0。构建仅报共享 dirty tree 既有 `merge_source_query_row` dead-code warning，本 slice 未修改该函数。
- **完成边界**：Scoping 实现 slice 已收口并有 focused evidence；未运行真实 LLM/provider/外部目标，所以不宣称 live E2E。按用户指令未运行 `./init.sh` / `just precommit`，未继续 Candidate，未改 schema/migration/generated IPC，未 commit/stage/push。父 parity feature 仍为唯一 `in_progress`；换模型后从 Candidate/完整 fixture 继续，live active-stage 验收仍需用户另给 exact target 与外部请求授权。

#### 2026-07-13 · Runtime Memory / Candidate V2 C9 canonical cited Reporting closeout

- **本轮目标**：实现并收口 corrected closeout plan C9 的 Reporting read model 核心：canonical facts/evidence citations、完整 source snapshot/CAS、validation/publication 双轴、Cleanup fail-closed 与 retained immutable history；每条 Cargo/Rust test 命令前执行 `just space-guard`。
- **已完成实现**：
  - 新增 `golish-reporting-domain`：统一 `ReportSourceVersion(kind, CanonicalRowId, row_version, content_hash)`、完整稳定排序 source-set SHA-256、typed section/claim/citation/residual/revision，以及 current revision/source、same-org resolvable evidence、Finding lineage、secret、Cleanup missing/nonterminal/residual 的确定性 validator。
  - 新增 `golish-reporting-app`：REPEATABLE READ/完整重读 truth port、build→validate→exact compare、renderer claim-set fence、递归 redaction、content-addressed artifact port 与 explicit user publication finalizer；文件/LLM 在 DB transaction 外，短事务只做 ownership/current/source/CAS/artifact refs/outbox。
  - 新增 migration `20260712000011_reporting_read_model.sql` 与 8 个 reporting repos：reports/revisions/source manifest/sections/claims/citations/blob/revision-artifact。final/superseded 历史 `RESTRICT` + trigger immutable；validated 与 final 分轴；多个 revision 可共享 blob。mutable reportable sources补单调 `row_version`。
  - Reporting authority bridge 只读 frozen scope、Cleanup-owned `CleanupCloseoutPort` 和 final-sealed/non-invalidated StageHandoff canonical refs；Cleanup port 的同一权威查询同时返回 missing/nonterminal/undisclosed/invalid-terminal-truth 四计数与 disclosed residual obligation ids，Reporting 不再复制 Cleanup status SQL。TechniqueOutcome 要求 exact operation composite key、current content SHA-256、row_version 与 evidence ids 全匹配；unsealed/drifted/unresolvable 行 fail closed，不从自由文本 run id 猜 ownership。
  - 新增 concrete `PgReportTruthPort`：build 在一个 `REPEATABLE READ READ ONLY` transaction 内冻结完整 canonical source set；deterministic DB projection 覆盖 StageEpisode、Finding/current lineage、CandidateAttempt disposition、sealed TechniqueOutcome、Post-Exploit/Foothold/InternalAsset/AttackPath/Objective 与 Cleanup residual，所有 claim 只由 exact source + same-operation evidence relation生成。source content hash 同时编码 row、evidence membership 与 AttackPath edges，防止只改 relation 不 bump owner row 时 citation 漂移。persist 在独立 `REPEATABLE READ` 写事务内重跑同一完整查询，再 create/begin/store/validate revision CAS，并返回持久化 row version。
  - 新增 `reporting_build_read_model` 与既有 read/history/artifact/finalize 四命令，按 command→facade→registry→frontend wrapper→ts-rs 类型链接线；build 只接受 operation id，server 重验 active project scope 与 local principal，幂等复用 source 未变的 current validated revision。finalize request 不含 actor/project path/storage key，文件在 transaction 外 stage/promote/read-back，DB publish transaction 内重查 source/CAS。
  - `golish-projects/file_storage` 提供 report staging/content-addressed promote/verify/discard/grace-period GC；`golish/reporting_artifact_store.rs` 只做 composition adapter，不复制路径 sanitization。GUI 与 CLI 都在 DB ready 后启动 orphan GC，并在 pool shutdown 前停止。
  - 前端新增 typed `api/reporting.ts` 与 `ReportReadModelView`：loading/error/empty、显式 DB truth build/rebuild、revision/superseded history、claims/canonical versions/evidence ids、artifact refs，以及二次确认 final publish 全部可见。
  - Reporting stage seam 改为 server-first：进入 stage 时通过 `reporting_build_validated_revision` 从 complete canonical source set build/reuse validated revision，失败则在 agent turn 前 BLOCK；close Gate 与 submit preview 重读 `ReportingGateTruth`，只认 current+validated revision、exact source hash、claim/citation/attestation 与 Cleanup closeout。Reporting 跳过 generic enrich/plan/wiki prior，stage 无 artifact/finalize 能力，Gate PASS 与显式 final publication 保持分轴。
  - `ReportRevisionFinalized.v1` 当前没有已组合 consumer，catalog 不生成 placeholder delivery，也不路由 Assertion/Document/Embedding/Graph；RAG/KG 不作为 Gate authority。
  - 新增两张模块卡并同步 DB/repo/agent-app 卡、架构图、INDEX；DAG 登记 reporting-domain=L1、reporting-app=L3，ownership map 登记 reporting repos/source root。
- **验证证据**：
  - scoped `rustfmt --edition 2021 ...` → exit 0。
  - `python3 scripts/check_dag.py` → exit 0，`DAG clean across 58 crates`。
  - `git diff --check`、`python3 -m json.tool feature_list.json` → exit 0。
  - `python3 scripts/check_repo_ownership.py` 当前仍因共享 dirty tree 的 233 ownership + 17 raw-SQL violations 失败；C9 raw projection 保持在既有 allowlisted `db_bridge/reporting.rs`，新 `db_bridge/reporting_gate.rs` 不含 raw SQL，未新增 raw-SQL violation。
  - frozen `20260712000001` 由主任务按 SHA-384 复核仍为既定 `ffda87b5...6047ba4`；本轮未修改 00001/00004。
  - `just space-guard && cargo check -p golish-reporting-domain -p golish-reporting-app` → exit 0；`just space-guard && cargo check -p golish-db -p golish-agent-app` → exit 0。另修复此前 C2/C7 runtime-memory adapter 四处 `&Arc<PgPool>` Executor seam 为 `pool.as_ref()`。
  - `just space-guard && cargo nextest run -p golish-reporting-domain -p golish-reporting-app --no-tests=fail --status-level fail` → 7/7 passed，run `7dca066e-3da6-4847-9d3c-4f7b23060491`；覆盖完整 source-set 新增 stale、same-org citation/secret/Finding lineage、validation/publication 双轴、Cleanup missing/nonterminal/缺整 org closeout row fail-closed、redaction。
  - C9 concrete adapter TDD RED：`just space-guard && cargo nextest run -p golish-agent-app --test reporting_authority --status-level fail` → exit 101，`PgReportTruthPort` 尚不存在（`E0432 unresolved import`）。GREEN：run `6dba15d6-8660-44e6-b585-b76294d5be0f` → 1/1 passed；真实 embedded PG 覆盖 canonical StageEpisode+evidence → cited validated revision，并在新增 canonical source 后确认 publication 返回 `report_source_snapshot_stale`。
  - `just space-guard && cargo nextest run -p golish-reporting-domain -p golish-reporting-app --status-level fail` → run `dec67733-dee8-4c3b-a1f5-6e5e16d3b207`，8/8 passed；新增 `invalid_terminal_truth_count` fail-closed 覆盖。
  - C9 Reporting Gate TDD RED：`just space-guard && cargo nextest run -p golish-agent-kit reporting_read_model_gate --no-tests=fail --status-level fail` → exit 100，run `0563b85c-f1cf-40f6-ae56-f775f62fbcc7`，stub validator 6 tests 中 1 pass/5 fail。GREEN：run `dadb8ee1-b19e-482b-af3f-03fd11c31634`，6/6 passed。随后 `just space-guard && cargo check -p golish-agent-kit -p golish-agent-app --lib` → exit 0（31.74s），确认 rule/spec/builder、stage entry/close 与 submit/app adapter 编译接通。
  - Reporting stage/submit focused：首轮 run `8b2c1a33-c46c-4f0a-bb24-56d5a4006ba8` 为 26/27，唯一 RED 是 fixture 空 deliverable 被结构 vacuous check 拒绝；补单条非权威 `report_read_model_ready` acknowledgement 后单测 run `61f9152b-a714-47d8-a2a9-cf5107ab63b0` 1/1，完整 `cargo nextest run -p golish-agent-kit -p golish-agent-app -E 'test(reporting)' ...` run `b6660c3e-88c2-422a-bc87-3248f1abbef0` 27/27 passed。覆盖 stage-entry build/reuse、build failure 在 agent turn 前 BLOCK、close fresh reload、无 generic enrich/wiki/report finalizer、submit preview foreign operation fail-closed 与真实 PG source-stale Gate。
  - 七包全量 nextest 首轮 run `dbf4112c-0ad6-4151-847d-e584ba82fa26` 的两条 Candidate-owned stale fixture 失败已由 Candidate owner 修复；随后在最终共享树重跑 `just space-guard && cd backend && cargo nextest run -p golish-reporting-domain -p golish-reporting-app -p golish-projects -p golish-db -p golish-agent-kit -p golish-agent-app -p golish --no-fail-fast --status-level fail` → exit 0，2035/2035 passed，run `993b11e2-003b-4c2a-9d0d-189f3bb8181f`。
  - 同七包 `cargo clippy --all-targets -- -D warnings` → exit 0（56.34s），零 warning。
  - `pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx` → exit 0，2/2 passed；覆盖 citation/source version/evidence 展示、explicit confirm final publish 与 loading/error/empty→build。
  - `just check-fe` → exit 0；`just test-fe` → exit 0（2026-07-13 Reporting freeze 后 fresh full frontend gates）。
  - 最终 shared generated 收口：`just space-guard && just gen-types` → exit 0；随后统一 `git add -- frontend/lib/generated backend/crates/golish-agent-app/bindings`；`just space-guard && just check-types` → exit 0，ts-rs export tests 全绿且 `frontend/lib/generated/` 无 unstaged drift。
  - `python3 -m json.tool feature_list.json`、`python3 -m json.tool resources/harness/stages/reporting/spec.json`、migration numeric-prefix duplicate check、`python3 scripts/check_dag.py`（58 crates）与全树 `git diff --check` → exit 0。
  - `just space-guard && cargo clippy -p golish-reporting-domain -p golish-reporting-app --all-targets -- -D warnings` → exit 0，零 warning。
  - `just space-guard && cargo nextest run -p golish-db --test reporting_read_model_migrations --no-tests=fail --status-level fail` → 3/3 passed，run `99b747ea-4149-4875-95a5-ce90737b94e6`；覆盖 schema/row_version、两个 revision 共享 blob、finalized claim/revision UPDATE 被 DB trigger 拒绝。
  - `just space-guard && cargo nextest run -p golish-memory-domain finalized_report_has_no_rag_or_graph_gate_route --no-tests=fail --status-level fail` → 1/1 passed，run `ddfb7d56-2c6a-4331-b893-374466331955`。
  - agent-app bridge no-retrieval test 首轮因 `include_str!` 测试自己包含 forbidden literal 而失败（run `190ab822-8463-4c6e-982c-261c8179121d`）；已修成运行时拼接 token。GREEN 复跑因主任务要求给全仓 `export_bindings` 让 Cargo lock 而主动 Ctrl-C（exit 130），未把取消当通过证据；bridge 源码已随 agent-app focused check 编译通过。
- **当前状态 / 风险**：C9 domain/app/DB、concrete build/persist/publication adapters、IPC/UI、artifact filesystem adapter、GUI/CLI GC lifecycle、server-first Reporting stage 与 deterministic Gate 均已落盘；七包全量 2035/2035、all-targets Clippy、全量前端门禁与最终 generated/type drift gate 已 fresh 通过。共享树 `just precommit` 仍待主线程执行，因此父 feature 继续 `in_progress`，不提前宣称 passing；本轮未发起 live scan、exploit、embedding、Graphiti、LLM 或其它外部请求。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-13 · Memory Fabric C7 scoped ContextPack / C2 promoter continuity

- **本轮目标**：实现 C7 operation/org/stage scoped RAG 与固定 1536 维 embedding 合同；同时修复 `StageEpisodeClosed.v1` 因要求预写 Assertion 而错误 suppress 的 C2 projector 断点，并把 side-effect action+cleanup obligation 纳入同事务 typed Memory event。
- **已完成实现**：
  - 新增 pure ContextPack domain：8 个 knowledge layer、四级 classification、`KnowledgeValue::{Text,Json,VaultRef}`、非 Deserialize 的 server-runtime subject；VaultRef 是 value kind，不是 classification。
  - memory-app 新增字段私有、无 public constructor/Deserialize 的 `TrustedAuthorizationContext`，由 DB authorization snapshot + server-owned principal/data policy 交集生成；retrieval 固定 `scope → classification → canonical → runtime → handoff → episode → assertion → document → temporal graph → vector`，optional graph/vector 显式 degrade，mandatory canonical/runtime 超预算 fail closed，无 legacy global memories/wiki fallback。
  - 新增 prompt-safe renderer/redaction：ContextPack 始终标为 untrusted data；secret key/marker、Bearer/private-key/API-key 样式及嵌套 JSON secret fail closed，prompt markup 转义，VaultRef 只保留 opaque UUID。
  - 新增 1536 维 `EmbeddingProjector` 合同与 1024 provider fail-closed test；production embedding worker也会精确读取同 event 的 `document-projector@1` delivery，只有 `succeeded` 才可能调用 provider，`succeeded_suppressed`/其它状态不调用。
  - 新增 `knowledge_context` DB read model：authorization 绑定 task-owned operation、sealed scope snapshot/member、stage execution/unit/optional worker；所有 assertion/document/vector SQL 先按 project/org/classification/validity过滤，vector distance 只在 scope-filtered CTE 后计算。
  - agent-app 用 `operator_principals::current_local` 组合 local-only policy + DB/temporal graph source；bridge/runtime 把同一 `ContextPackProvider` 注入 active harness RuntimeSupervisor 与 specialist briefing。active harness 禁止回退 legacy global briefing，普通 non-harness sub-agent 保留兼容路径。
  - `assertion-promoter@1` 对 `StageEpisodeClosed.v1` 重读 persisted Episode，生成 deterministic、evidence-backed、graph-projectable organization Assertion并幂等 insert；没有 evidence 时 retry/fail closed，不再走 `memory_event_has_no_promoted_assertions` suppress。
  - closed catalog 新增 `PostExploitActionPrepared.v1` / `PostExploitAction`。`cleanup_obligations::record_action_and_obligation` 在 action+obligation+双方 exact evidence 同一事务内调用 `append_action_prepared_event_with_connection`；helper重读 canonical rows，只把 action/obligation id、capability、side-effect class、plan/resource hash、evidence ids 写入 event。promoter随后重读 persisted rows/evidence/sealed scope hash并幂等派生 graph-projectable Assertion；event/outbox或约束失败会回滚整个 prepare transaction。
  - 同步 memory-domain/app、DB repo、agent-app/kit/runtime/bridge 模块卡、`docs/modules/INDEX.md` 与 `docs/architecture.md`；未修改 frozen migration 内容，未发外部 embedding/Graphiti/LLM 请求。
- **TDD RED 证据**：
  - `cd backend && cargo nextest run -p golish-memory-domain --test context_contract -p golish-memory-app --test scoped_rag_contract --status-level fail` → exit 101；首次 RED 因缺少 `KnowledgeValue` / `VaultCredentialRef`，证明四级 classification + value-kind 合同尚未实现。
  - 首次尝试 trybuild UI test 因 crates.io SSL 失败（exit 102），已移除新网络依赖，改为 test 内调用本机 `rustc` 的 compile-fail fixture；未把网络失败当功能证据。
- **当前静态验证证据**：
  - C7/C2 相关 Rust 文件 `rustfmt --edition 2021 --check` → exit 0；scoped `git diff --check` → exit 0。
  - `python3 scripts/check_dag.py` → `[check_dag] ✓ DAG clean across 56 crates`；`cargo metadata --no-deps --format-version 1` → exit 0；agent-kit 对 memory-domain 的 direct dependency 与 lock 已同步。
  - `jq -e . feature_list.json` → exit 0。
  - frozen foundation SHA-384：`ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17dcba7d1cc760c007f7328d1725b6047ba4`（与固定值一致）；未改 00001/00004 migration。
  - 只读静态复核确认 promoter/domain/repo API、cleanup deferred constraint 事务顺序、ContextPack borrow/type 形状无明显错误；这不是编译通过的替代证据。
- **未运行门禁 / 原因**：当前 data volume 仅约 1.7 GiB 可用，`backend/target` 先前观测约 191 GiB；按主任务指令停止新的 Cargo workspace/nextest/clippy/check 大构建，且未获授权删除缓存。最终源码因此尚无 fresh nextest/all-target Clippy/`just precommit` 证据，feature 必须保持 `in_progress`，不能标 `passing`。
- **下一步最佳动作**：用户确认清理 build cache 或释放磁盘后，先跑 C7 三个 focused tests（domain contract、scoped RAG、opaque trusted-context compile-fail）和 agent-app promoter focused tests，再跑 memory-domain/app/agent-app/DB all-target Clippy；最后由主任务统一 `just precommit` 与 checkpoint commit。
- **以下文件已修改但未提交**：C7 domain/app/repo/adapters/bridge/runtime/renderer 相关 Rust/Cargo 文件、C2/C6 event/promoter seams、对应 tests、模块卡/索引/architecture 与本 progress 条目；共享工作树仍含其他并行任务改动。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-13 · Runtime Memory specialist contract / four-stage final-seal StageEpisode seam

- **本轮目标**：完成 Runtime Memory V2 cutover 前的 Task 9 静态实现：让四个信息阶段显式声明 exact runtime contract，并把 P1 final seal 接入 caller-owned Memory Episode transaction seam；不创建 cutover migration。
- **已完成实现**：
  - 新增 closed typed `StageRuntimeContract` / `RuntimeUnitIdentity` / `RuntimeScopeSource`，并给 `StageSpec` 增加向后兼容的可选 `runtime_memory` 字段。
  - `target_intel`、`external_attack_surface`、`enumeration`、`vuln_triage` 四份 embedded spec 精确声明 schema v2、`stage_execution_organization`、`frozen_operation_snapshot`、required worker lease 与 final-seal handoff；Scoping/Reporting 不声明该 specialist contract。
  - 四信息阶段 `runtime_memory_tx::finalize_unit_pass` 在原 caller-owned compound transaction 内调用 `stage_episodes::close_episode_with_event_with_connection`，原子写 immutable `StageEpisode`、`StageEpisodeClosed.v1` event 与 catalog deliveries。Memory event/delivery 失败会连同 Unit/Worker/Handoff/completion/legacy mirror 一起回滚；response-loss exact replay 不重复创建 episode/event/delivery。Assertion 仍由异步 deterministic promoter 产生，producer 不直接插 Assertion，也没有 after-commit 补写。
  - 增加 DB integration assertions：legacy mirror failure 与 Memory outbox failure 均不残留 handoff/episode/event/delivery；成功 seal + replay 预期恰好 1 episode、1 event、4 deliveries。该测试代码因当前磁盘 blocker 尚未执行。
  - 同步 `golish-agent-kit` harness、`golish-db` repo、`golish-memory-app` 与模块索引卡片。
- **静态验证证据（实跑）**：
  - Task 9 五个 Rust 文件 `rustfmt --edition 2021 --check` → exit 0。
  - Python 精确解析四份 specialist spec 并比对完整 `runtime_memory` object；同时确认 Scoping/Reporting 字段缺席 → exit 0。
  - `python3 -m json.tool feature_list.json` → exit 0。
  - `git diff --check -- <Task 9 scoped files>` → exit 0。
  - foundation migration SHA-384 保持 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`；`20260712000002_runtime_memory_v2_cutover.sql` 不存在。
  - C5 scoped Clippy 已定位两处 `explicit_auto_deref`；本轮将 final-seal helper 调用从 `&mut **tx` 改为 `tx` 并重新 rustfmt。没有为此重跑大型 build。
- **当前状态 / 风险**：未运行新 DB integration test、Cargo package suite、`./init.sh` 或 `just precommit`。本机 `backend/target` 约 191G、`debug/incremental` 约 122G，fresh Cargo link 已因 `No space left on device` 失败；删除缓存属于高风险删除，必须等待用户明确授权。四阶段 fresh acceptance 与完整门禁未绿前不创建 `00002` cutover migration，feature 保持 `in_progress`。
- **提交记录**：未 commit、未 stage、未 push；根 agent 统一处理用户要求的 checkpoint commit。

#### 2026-07-13 · Candidate V2 Task 6 durable review / DB-backed resume wakeup

- **本轮目标**：只执行 corrected Candidate V2 计划 Task 6：实现 durable Candidate review 四命令、exact DB barrier、trusted operation resume、startup stale-dispatch reaper、trace refresh hint 与 `attack_candidate` detail review UI；未进入 Task 7。
- **已完成实现**：
  - `attack_candidate_approvals` 以 `operation_id + wave_run_id` 锁定 frozen project/snapshot/WaveUnit/org ownership，在事务内解析 active local operator；request 不接受 actor/project/snapshot/org/action/budget authority。approve/reject 逐 Candidate 校验 exact plan hash + row version + expiry，并支持等值 response-loss replay；sibling/stale/expired/drift 均稳定错误码 fail closed。
  - review close 只写 durable `resume_pending`。resume command CAS 到 `dispatching` 后复用从 chat 抽出的 trusted TaskOrchestrator continuation；同步启动失败 CAS 回 pending 并写 `last_error`，成功写 `resumed`。DB startup reaper 只重置超时 dispatch，不重开已写 decisions。
  - kit 新增纯 `review_barrier` 决策与 fail-closed DB seam；`attack_candidate` 只有 exact snapshot 为 `resumed|terminal` 才允许流向 verification，其他状态/DB 错误均 hold。新增 `CandidateReviewRequired/Resumed` trace，并同步 op-trace/transcript exhaustive consumers。
  - 四个固定 Tauri commands、独立 attack facade/registry、ts-rs DTO、typed frontend API 与五个 `ATTACK_*` code 已接齐。`AttackCandidateReview` mount/read DB，显示 frozen target、exact plan hash/actions/budget/expiry，resume 失败保留 durable decisions 并允许幂等重试；trace handler 只递增 refresh hint。面板挂在 `attack_candidate` 的 `stage_run` detail。
  - 同步 agent-app/kit/db/golish/core 与 frontend components/lib/services/store 模块卡及 `docs/modules/INDEX.md`。
- **TDD RED 证据**：
  - `pnpm vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx`（repo root）→ exit 1；新组件不存在，Vite 明确报 `Failed to resolve import "./AttackCandidateReview"`。实现后同 suite 转绿。
  - backend hostile tests 先分别以缺 review API/barrier/reaper 行为 RED，再实现 exact snapshot/CAS；最终合并结果见下。
- **GREEN / 已记录证据**：
  - `cd backend && cargo nextest run -p golish-db --test attack_execution_v2_migrations -E 'test(review_) | test(stale_dispatching_wakeup_reopens_without_reopening_review_decisions)' --no-tests=fail --status-level fail` → exit 0；6/6 passed，run `1d6afd72-30d4-448e-9da7-f1d6fbd1b70b`。
  - `cd backend && cargo nextest run -p golish-agent-kit review_barrier --no-tests=fail --status-level fail` → exit 0；2/2 passed，run `03115868-6623-4d25-94e4-6cbf0ac5f877`。
  - `cd backend && cargo nextest run -p golish-agent-app attack_review --no-tests=fail --status-level fail` → exit 0；2/2 passed，run `e47b4e18-4ac4-48e0-ab3b-2dd3fe29e1c6`。
  - `pnpm vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/services/ai-events/harness-handlers.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts` → exit 0；21/21 passed。
  - `just gen-types` → exit 0；新增 Candidate review/Attempt DTO 与 trace union 全部由 ts-rs 生成，未手改 `frontend/lib/generated/`。trace variant 顺序静态调整后，`CARGO_INCREMENTAL=0 cargo test -p golish-core export_bindings -q` → exit 0，14/14 passed，仅定向重生 core wire types。
  - `just check-fe` → exit 0；Biome + TypeScript 均通过。`cargo check -p golish-agent-app --all-targets`、`cargo check -p golish --all-targets` 在磁盘告警前均 exit 0。`python3 scripts/check_dag.py` → exit 0，56 crates DAG clean；最终 `git diff --check` → exit 0。
- **未完成门禁 / 阻塞证据**：
  - package clippy 已按计划启动，但共享 `backend/target` 达 191G、磁盘只余约 2.6GiB；主 agent 明确要求立即停止大型 Cargo。中止前唯一 diagnostic 来自并行 C7 的 `golish-memory-app/src/ranking.rs:20 clippy::manual_div_ceil`，不是 Task 6 文件；因此不能把 clippy 记为通过。
  - 未跑 `./init.sh` / `just precommit` / 全 workspace tests；没有 fresh app live review→resume→verification 证据。Task 6 代码与 focused tests 已收口，但整个 feature 必须保持 `in_progress`，待用户授权清理 target/cache 后补全门禁。
- **以下本轮文件已修改但未提交**：Candidate review DB repos/DB tests/startup reaper；agent-kit review barrier、DB trait、orchestrator gate/tests；agent-app attack commands、shared operation resume 与 DB bridge；core/events consumers；golish attack facade/registry；ts-rs generated Candidate DTO/trace types；frontend attack API/error map、review component/tests、detail mount、harness handler/store hint；相关模块卡、索引与本进度记录。
- **提交记录**：未 commit、未 stage、未 push。
- **下一步最佳动作**：获得用户对清理 `backend/target`/incremental cache 的明确授权后，重跑 Task 6 scoped clippy、`cargo fmt --all -- --check` 与仓库 `just precommit`；随后重启 app 做一次 open review reload、exact decision、resume failure/retry 与 verification continuation 的 live DB 证据。不要在门禁补齐前把 feature 标 `passing`。

#### 2026-07-13 · Candidate V2 Task 7 foreground verifier / action journal

- **本轮目标**：执行 corrected Candidate V2 Task 7；实现 opaque Attempt context、每 action DB re-authorization、foreground-only verifier、compound scheduler/action journal 与同 Attempt crash recovery；严格停在 Task 8 submit validator/Finding terminalizer 之前。
- **已完成实现**：
  - `CandidateAttemptContextRef` 只含 candidate/approval/attempt/plan-hash，沿 main/sub-agent trusted context 传播；core dependency-floor guard 与 agent-kit pre-action authorizer 都只允许 ordinal wrapper、recent evidence、Attempt submit，并拒绝 identity override、raw `pentest_run`、Finding writer、scanner、nested/background controls。
  - background manager 新增 typed Result spawn；Candidate attributed context 返回 `ATTACK_VERIFIER_FOREGROUND_REQUIRED`。唯一 production caller 已切换，foreground-only 执行仍允许且超时 kill，不产生 background handle。
  - DB claim 复合拥有 CandidateAttempt、P1 WorkerRun、durable message chain 与 global lane；每 action 重验 frozen contracts、project scope hash、Candidate/Approval/Attempt、current plan/hash/expiry、ordinal、capability/action/canonical args、budget、worker lease 与 lane，再以 `planned -> started -> completed|failed` journal 包住 side effect。
  - expired lane 无 active started action 时保留同 WorkerRun/Attempt/checkpoint 并 requeue；`started` 无 terminal outcome 原子改为 `outcome_unknown` + `recovery_required`，禁止盲重放。已完成的 terminal action只做等值 replay。
  - hardcoded/registry 同时注册 reasoning-only `attack_analyst` 与 closed `candidate_verifier`；唯一 `verify_execute_candidate_action(action_ordinal)` 从 trusted context/DB 重载 canonical action、固定 foreground runner recipe并写 journal。
  - Verification `stage_run` 进入 Candidate compound scheduler；专用 heartbeat 同事务续 WorkerRun+lane。verifier 返回后显式停在 `CANDIDATE_TERMINALIZER_TASK8_REQUIRED`，未接 legacy StageDeliverable/Finding writer、未实现 Task 8 terminalization/release。
- **测试先行 / 静态证据**：新增 opaque context、candidate authorizer、foreground spawn、closed registry、wrapper schema、scheduler Task8 boundary/heartbeat tests；由于磁盘从 1.7GiB 降至 **361MiB**，按主 agent 明确约束全部标为未执行，未运行任何 Cargo/frontend build。逐文件 `rustfmt --edition 2021 --config skip_children=true ...` → exit 0；`git diff --check` → exit 0；静态 `rg` 已核对 13 个 `AgentToolContext` literal 都设置 `candidate_attempt`，7 个 `BoundWorkerChainContext` literal 均显式设置。
- **当前状态 / 风险**：没有 fresh Cargo typecheck/test/clippy、`./init.sh`、`just precommit` 或 live verifier run 证据，feature 必须保持 `in_progress`。Task 8 必须补 exact `submit_candidate_attempt` validator、Attempt terminalizer/Finding writer closure 与成功 compound release；在此之前 scheduler 故意 fail closed。
- **提交记录**：未 commit、未 stage、未 push；未删除 cache/target，也未改 schema migration。

#### 2026-07-13 · Runtime Memory V2 — C3 Structured Temporal Knowledge Graph

- **本轮目标**：实现按 `project_scope_id + organization_id_at_time` 精确隔离的结构化时态知识图投影、查询、重建与 IPC；保持 legacy Graphiti 路径不变，并严格禁止原始 evidence / VaultRef / 任意 JSON 进入图。
- **已完成实现**：
  - 新增 migration `20260712000007_structured_temporal_graph.sql`：generation / entity / relation / assertion-lineage 表、scope/validity/classification CHECK、canonical assertion 复合 FK、lineage 镜像触发器、单 active/building generation fencing 与内容 attestation。
  - `golish-db` 新增 exact-scope generation/reconciliation/query/rebuild repository；节点与边都要求当前 assertion lineage，边额外要求两个 endpoint 当前可见；同 stream 高版本压低版本，跨 stream 共享 canonical identity 保留多 lineage。
  - `golish-graphiti` 新增独立 `TemporalGraphClient`，未修改 legacy client；支持幂等 replay、stale delivery、building-generation 隐藏、失败 generation 不可激活及原子 cutover。
  - `golish-memory-app` 新增 closed entity/relation predicate、属性 allowlist、bounded scalar/array 校验、global sanitized technique 限制、outbox projector、invalidation 与 assertion-driven rebuild。
  - 完成 Tauri 五步链：后端 `knowledge_graph_query_scoped` / `knowledge_graph_rebuild_scope`、facade、registry、ts-rs bindings、前端 `temporalGraph` API。组织 scope 由服务端 operator principal 与 DB exact binding 授权，调用方不能传 actor/project path；rebuild 只读取 canonical active assertions。
  - 同步 repo ownership、DAG/架构文档及 `golish-db`、`golish-graphiti`、`golish-memory-app`、`golish-agent-app`、facade、frontend lib 模块卡。
- **已记录验证证据**：
  - `cd backend && cargo nextest run -p golish-memory-app --status-level fail` 的 graph projection/projector 聚焦测试 → 10/10 passed，run `6b55ed5b-9d25-4a8a-8329-4312ae0a6065`。
  - `cd backend && cargo nextest run -p golish-graphiti --test temporal_graph --status-level fail` → 1/1 passed，run `9f6a08eb-33c7-45d1-8130-6da352891273`；覆盖 legacy 隔离、exact scope、multi-lineage、edge endpoint visibility、freshness、hostile cross-scope FK、global sanitization、stale/replay、generation fencing/attestation。
  - `cd backend && cargo nextest run -p golish-agent-app -E 'test(knowledge_graph_)' --status-level fail` → 3/3 passed，run `5bd7edc7-481f-4c3c-81ea-d97235d50f2c`；另有 ts-rs/command 单测 11/11 passed。
  - `pnpm exec vitest run frontend/lib/api/temporal-graph.test.ts` → 2/2 passed；targeted Biome 与 `pnpm typecheck` 均 exit 0。
  - `cd backend && cargo check -p golish-agent-app --all-targets`、`cargo check -p golish --lib`、`cargo clippy -p golish-memory-domain -p golish-graphiti -p golish-memory-app -p golish-agent-app --all-targets --no-deps -- -D warnings` 均 exit 0。
  - `python3 scripts/check_dag.py` → DAG clean（53 crates）；`git diff --check` 与 `python3 -m json.tool feature_list.json` 均 exit 0。
  - frozen foundation migration SHA-384 复核为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`，未修改 `20260712000001` / `20260712000004`。
- **当前状态 / 风险**：C3 聚焦实现与验证已完成并停止，未继续 C2/C7。整个 `runtime-memory-candidate-pipeline-v2-2026-07-12` epic 仍有其他工作，故 feature 保持 `in_progress`；未发起外部 Graphiti/embedding/LLM 请求，未跑全量 `just precommit`。ownership guard 仍有仓库既存基线（207 ownership + 15 raw-SQL），但 C3 的 `knowledge_graph` ownership 已登记且无新增未注册违规。
- **提交记录**：此前 checkpoint commit 为 `ab7b0c4a feat(harness): checkpoint deterministic stage closure and V2 design`；本轮 C3 批次仍未 commit、未 stage、未 push，等待主任务完成全量门禁后统一提交。

#### 2026-07-13 · Runtime Memory V2 — C2 Production Memory Fabric / process supervisor

- **本轮目标**：只完成 corrected plan 的 C2：production canonical UoW/outbox/document/
  embedding/graph adapters、进程级 Memory Supervisor、desktop/CLI 生命周期接线与
  automatic-memory 单写入口审计；不进入 C7，也不改 `runtime_memory_tx` 的既有生产者。
- **已完成实现**：
  - `golish-db` 增加 caller-owned connection/transaction inner seams：episode close、assertion
    promotion、projection-chain invalidation 都可在 canonical row 与 multi-consumer outbox
    event/deliveries 的同一事务中完成；outbox 支持 paused→enabled 激活、typed event 回读、
    source/scope/status/kind 交叉校验与精确 event-source assertion/document 查询。
  - `PgKnowledgeMemory` 实现 production `KnowledgeUnitOfWork` 及 assertion/document/
    embedding/graph 四类 projector worker。每类 delivery 独立 claim/ack/fail；外部 embedding
    I/O 不跨 DB transaction；无 embedder 或 restricted document 会确定性记为
    `succeeded_suppressed`；embedding 维度固定 1536，document/embedding id 可重放确定。
  - `KnowledgeProjectorSupervisor` 是 process-global owner：并发 `start()` exactly once、四个
    projector 共享 cancellation、panic 后保留 lease 并等待 DB 到期重试、shutdown 停止接新
    batch 并等待 in-flight batch。desktop 在 DB ready 后启动并由 `AppState` 取消/join；普通
    CLI 与 stage-run CLI 使用同一个 side-effect-free constructor 并在 embedded PG 停止前
    graceful shutdown。
  - `bridge_config` / `BridgeBackends` 只注入共享 `KnowledgeUnitOfWork` handle，绝不按 session
    创建或启动 supervisor；静态回归测试固定该边界。
  - automatic-memory policy 只在 trusted harness operation context 使用 cutoff；全仓审计确认
    自动调用者仅 `single_tool_call.rs:516` 的一次 `maybe_store_tool_memory`，显式 memory tools
    仍直接走 `store_memory_with_*`，没有 subagent/explicit bypass writer。
  - 同步 `golish-memory-app`、`golish-agent-app`、`golish-agent-bridge`、`golish-db/repo`、
    `golish/{app,state,cli}` 模块卡、主索引与架构文档。
- **已记录验证证据**：
  - `cd backend && cargo nextest run -p golish-agent-app --test knowledge_memory_runtime --status-level fail`
    → 2/2 passed，run `bd09319b-55c3-4079-aee4-0af5f6eeb3a0`；覆盖 canonical insert 后
    outbox route 故障整笔 rollback，以及 projector crash/lease expiry/reclaim、两 session
    单进程 owner、单 deterministic document 与 graceful shutdown。
  - `cd backend && cargo nextest run -p golish-memory-app --status-level fail` → 15/15 passed，
    run `c40ca23c-6ddc-43e6-bd7a-39b3799fccea`；supervisor focused 3/3 passed，run
    `33d1f757-47c9-4a10-8593-6acd09069b3f`。
  - bridge composition static tests → 2/2 passed，run
    `4c7ec223-54cf-4cb2-8852-a4f958a485a5`；policy/runtime focused → 3/3 passed，run
    `64bad155-70d3-468c-ba20-602989f3fb87`；`golish-db --test memory_fabric_core` → 1/1
    passed，run `ea00885d-4b80-4ac6-b9f6-3487c1660934`。
  - `cd backend && cargo check -p golish --all-targets` → exit 0；C2 scoped
    `cargo clippy -p golish-memory-app -p golish-agent-bridge -p golish-agent-app --all-targets --no-deps -- -D warnings`
    → exit 0；最终 `cargo clippy -p golish --all-targets --no-deps -- -D warnings` → exit 0
    （并行 P1 dependency 仍打印一条 `operation_resume.rs` dead-code warning，不属于 C2）。
  - C2 相关 14 个 Rust 文件 `rustfmt --edition 2021 --check`、`git diff --check` → exit 0；
    `python3 scripts/check_dag.py` → DAG clean across 54 crates。
  - frozen foundation migration SHA-384 仍为
    `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`；
    未修改 frozen `20260712000001` / `20260712000004`，验证期间未发起 Graphiti、embedding
    或 LLM 外部请求。
- **当前边界 / 风险**：C2 的 adapter、inner seam 与 lifecycle 已落地，但不能虚报 producer
  atomic closure：P1 Task 9 的 final-seal producer 与 P2 Task 8 的 Attempt terminalizer 仍须在
  各自 compound transaction 内调用本轮 connection-level inner seam；在那两处集成前，epic
  保持 `in_progress`。`scripts/check_repo_ownership.py` 当前因共享 dirty tree 的 189 ownership +
  14 raw-SQL 基线失败；输出无 `knowledge_assertions/documents/embeddings/outbox/graph` 的 C2
  ownership 新违规。本轮未跑全量 `just precommit`。
- **提交记录**：此前 checkpoint 为 `ab7b0c4a feat(harness): checkpoint deterministic stage closure and V2 design`；
  本轮 C2 未 commit、未 stage、未 push，且已按边界停止，未继续 C7。

#### 2026-07-13 · Runtime Memory / final-seal / Candidate handoff V2 Task 7

- **本轮目标**：完成 V2 Unit/Worker 最终封口、wave 原子 close、可继承 StageHandoff，以及 `attack_candidate` frozen manifest 与 final PASS transaction 的权威接缝；不进入 Task 8。
- **已完成实现**：
  - V2 Gate PASS 不再直接计数；四个信息阶段从 exact operation/org/wave coverage snapshot 聚合 server seal，Candidate 从 frozen manifest + server-classified acceptance 构造 canonical work-item refs、typed terminal decisions、decision evidence 与 watermark。Candidate 不能借空 coverage 过关，Verification 在 Attempt snapshot seam 前 fail closed。
  - `CandidateAcceptance` 在 `seal_material_sha256` 计算前进入 final-seal input；DB 独立核对 exact manifest ids、typed claims、watermark、decision evidence，并在同一 transaction 写 Worker/Unit PASS、immutable handoff、completion 与 Candidate batch。
  - Candidate canonical projection 仅 hash frozen manifest 的不可变字段；acceptance 更新 work-item terminal fields 后重新 resolve 的 content hash 保持不变。旧 evidence 只有同时属于 exact final-sealed `vuln_triage` entry handoff 与 frozen manifest evidence links 才允许；same-owner unlinked/foreign/invalidated 均 fail closed。
  - wave close 以一个 DB transaction 完成当前 wave，并互斥选择 child wave + `WaitingBackground` 或 final seal；response-loss replay 核 exact parent/child/checkpoint/watermark/terminal wave，失败注入不会留下 partial completion。
  - live unexpired Running lease 在 provider/reaper 前返回等待；expired 无 active tool 才恢复；stage inherited handoff 按 spec source/evidence kinds 精确过滤并有界注入。
- **验证证据**：
  - `cd backend && cargo check -p golish-db -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-db --test runtime_memory_worker_transactions --status-level fail` → 14/14 passed，run `bfd3431f-9728-415b-a1b6-f0cb43538ee5`；主任务随后独立合并复验相关 DB 30/30 passed。
  - Candidate atomic final-seal hostile + predecessor provenance + post-accept canonical hash：1/1 passed，run `3075cb31-4182-40d8-a9b8-58ebac41b789`。
  - `cargo test -p golish-agent-runtime --lib stage_run_call::tests::v2_ -- --nocapture` → 5/5 passed；live lease 与 inherited handoff focused tests各 1/1 passed。
  - `cargo test -p golish-agent-kit --lib handoff_catalog -- --nocapture` → 3/3 passed；`cargo test -p golish-agent-app --lib ai::db_bridge -- --nocapture` → 41/41 passed。
  - `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings` → exit 0；`git diff --check`、`feature_list.json` parse → exit 0。
  - frozen foundation migration SHA-384 仍为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`。
- **当前状态 / 风险**：Task 7 code/tests green；未跑 `./init.sh` / 全量 `just precommit`，整个 V2 feature 仍保持 `in_progress`，Task 8 尚未开始。当前工作树含并行 Task 1/2/3 的共享改动，应由主任务统一 checkpoint commit。
- **提交记录**：本子任务未独立 commit/stage/push；用户已授权先 commit，由主任务在汇总并行改动后执行。

#### 2026-07-13 · Runtime Memory V2 — CLI one-operation / resume-reaper / dev reset Task 8

- **本轮目标**：只完成 corrected plan Task 8：headless CLI 收敛到一个 V2 operation/snapshot，按 persisted contract 重写 resumable/startup reaper，dev reset compound supersede，以及 `run_tree.py --db` runtime-memory 诊断；不进入 Task 9，不修改 frozen foundation migration。
- **已完成实现**：
  - V2-writing CLI 在 operation create 前一次解析 root/descendants/51% threshold，`CreateRuntimeOperation` 在同一事务创建 task/operation/initial execution 并冻结 `CliFlags` decision + sealed snapshot；同一 frozen scope 只调用一次 `run_stage`。parent run 后回读 session 唯一 operation 的 frozen contract，deployment rollout 并发漂移会 fail closed。`OrgFleetExecutor` 与 production scheduler adapter 都把 per-org child operation 限定为 `LegacyV1`。
  - `latest_resumable_by_session` / startup reaper 按 operation-frozen contract 分源：Legacy完整 JSON；`DualWriteV2Preferred` 完整 V2 优先、整条 legacy fallback；`V2Only` 只读 relational truth。V2 覆盖 Scoping pre-freeze、specialist 每 frozen org 一 Unit/Worker、non-specialist 一 root Unit/零 Worker、live lease wait、expired/no-tool requeue、expired/active-tool `recovery_required`，并拒绝 duplicate execution、stale active tool、partial/cross identity。startup reconcile/pause/fail 共用一个 transaction，按 operation lock 顺序执行，dual-write worker状态同步 legacy mirror。
  - `harness_dev_reset_stage_checkpoint` 改走 `runtime_memory_tx::supersede_stage_checkpoint`：同事务结束 lease、supersede受影响 Unit/Worker、invalidate handoff、关闭并替换 active execution、重建 selected stage runtime、更新 cursor/legacy mirror，保留历史。foundation migration 的 `stage_runs.status` CHECK 尚无 `superseded`，因此旧 execution 暂以 `failed` compatibility status 落库，并在 state blob 写 semantic `superseded` marker；Task 9 再做 schema cutover。显式 `restart_from_stage_purge` 仍保留独立事实清理事务。
  - `run_tree.py --db` 输出 rollout/frozen contract、scope decision/hash/units、stage execution/unit、worker lease/epoch/active tool/chain/checkpoint、submission、handoff、selected read source/legacy fallback，并诊断 duplicate/cross-org/stale-tool anomaly。同步 stage_run、golish-db/repo、agent-app/ai、agent-kit/task_orchestrator 模块卡与主索引。
- **TDD RED → GREEN 证据**：
  - 新增 embedded DB root-only regression 后首次运行稳定 RED：multi-org frozen scope reset 到 non-specialist stage 时 `latest_resumable_by_session` 返回 `None`，断言期望同 operation。重写 relational predicate 为“specialist 每 member”或“non-specialist exact root-only”两条互斥完整 shape 后转绿。
  - startup bulk SQL 首次 RED 曾因 PostgreSQL `UPDATE` target alias 在 `FROM JOIN ON` 中不可见而失败；将 worker identity 移入 `WHERE` 后转绿。
  - `cd backend && cargo test -p golish-db --lib cli_descendants_share_one_operation_and_snapshot -- --nocapture` → exit 0，1/1 passed；真实 embedded PG 覆盖一 operation/一 snapshot/三 org units、expired no-tool/active-tool/live lease、pause/latest selection、stale active-tool rejection、specialist reset、root-only reset、Scoping pre-freeze与 duplicate-active fail closed。
- **已记录验证证据**：
  - `cd backend && cargo nextest run -p golish -E 'test(cli_descendants) | test(resumability) | test(v2_adapter)' --no-tests=fail --status-level fail` → 3/3 passed，run `7e6e5dbd-c080-4bce-a678-16a8a1eb048e`。
  - `cd backend && cargo nextest run -p golish-db startup_reaper --no-tests=fail --status-level fail` → 1/1 passed，run `4940a4af-2357-4106-a957-d4de02e5f6bb`；`cargo test -p golish-db --lib repo::tasks::tests -- --nocapture` → 11/11 passed。
  - `cd backend && cargo nextest run -p golish-agent-app harness_dev --no-tests=fail --status-level fail` → 4/4 passed，run `2f1af1c0-4f81-4879-a8be-5b6e220689ba`。
  - `python3 -m py_compile scripts/run_tree.py && python3 -m unittest scripts.tests.test_run_tree_runtime_memory` → exit 0，4/4 passed。
  - `cd backend && cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish` → exit 0；Task8 14 个 Rust files 的 scoped `rustfmt --check`、`git diff --check`、`feature_list.json` parse → exit 0；`python3 scripts/check_dag.py` → DAG clean across 56 crates。
  - `scripts/check_repo_ownership.py` 当前共享 dirty tree 为 190 ownership + 14 raw-SQL baseline violations，低于本任务接手时 203 + 15；Task8 新 SQL 全部在 repo，`stage_run/runtime_v2.rs`/fleet/scheduler 无 raw `sqlx::query`。guard仍因全仓既有基线失败，未扩大 allowlist掩盖问题。
  - frozen `20260712000001_runtime_memory_foundation.sql` SHA-384 仍为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`。
- **当前状态 / 阻塞**：最后一次在 frozen-contract回读与 startup dual-write mirror 加固后重跑 `golish` focused suite，Rust linker 因 `No space left on device` exit 101；当前磁盘仅余约 2.2–2.6 GiB，`backend/target` 约 191 GiB，其中 incremental约 122 GiB。按 AGENTS 高风险删除规则未擅自清缓存；主任务已要求停止大型 cargo build并等待用户授权清理。Task8 scoped `rustfmt --check` 与 JSON parse仍绿，但最终共享 `git diff --check` 又被并行 Task 6 新生成的两个 TypeScript binding trailing whitespace 拦住，已通知对应 owner处理。因此未跑最终 scoped Clippy、`./init.sh` 或全量 `just precommit`，feature必须保持 `in_progress`，不得标 `passing`。
- **提交记录**：本子任务未 commit、未 stage、未 push。用户已授权先 commit，但主任务将在获得构建缓存清理授权、补齐最终门禁并汇总并行改动后统一 checkpoint commit。

#### 2026-07-13 · Cleanup P7a obligation kernel C5

- **本轮目标**：执行 corrected closeout plan Task C5：先交付无 Tool、无外部 I/O 的
  Cleanup obligation/attempt/absence/waiver kernel，打破 Post-Exploit side-effect 与
  Cleanup 的依赖循环；不进入 C6/P6b Tool、C8 worker/organization deletion。
- **已完成实现**：
  - 新增 `golish-cleanup-domain`（L1）与 `golish-cleanup-app`（L3）。领域状态固定为
    Attempt `claimed/executing/cleaned_pending_verification` 三个 live 状态；
    `verified_absent/verification_failed/execution_failed` 都关闭当前 Attempt。
    `inconclusive/still_present` 把 Attempt 置 `verification_failed`、obligation 恢复 `open`，
    下一 claim 使用新 ordinal；不得把不确定结果伪装为已清理。
  - migration `20260712000009_cleanup_obligation_ledger.sql` 建立 retained
    obligation/attempt/absence/waiver/evidence 表，全部绑定 frozen operation/project/
    scope snapshot/org-at-time，不 FK live organization/target。deferred circular FK +
    constraint trigger 强制每项 side-effect action 恰好回指一项 exact obligation；无 obligation
    side effect 和给 read-only action 伪造 obligation 均由 DB 拒绝。C4 P6a app 仍拒绝
    mutation，C5 也不注册 Tool。
  - production compound repo 在一笔短事务验证 active server principal、sealed scope、exact
    evidence、action/plan/resource/strategy/proof 后共同写 action + obligation；任一写失败全部
    rollback。response-loss replay 必须行内容和 evidence set 完全相等，不能追加 evidence。
  - claim/transition、independent absence 与 waiver 均为 DB authoritative：one-live-attempt
    partial unique 只含三个 live 状态；absence verifier 必须与 executor/cleanup evidence
    独立且 resource hash 相同；waiver 只接 opaque principal + expected row version + residual
    evidence，live attempt 存在时拒绝。
  - 新增两张模块卡；repo ownership 登记四个 cleanup repos，DAG 登记两个 crate。
- **TDD RED 证据**：
  - `CARGO_TARGET_DIR=/tmp/golish-cleanup-domain-red cargo test -p golish-cleanup-domain inconclusive_absence_closes_attempt_but_keeps_obligation_retryable --lib -- --nocapture`
    → exit 101，缺 `apply_absence_result`；实现后 domain/app 7/7 转绿。
  - `cargo nextest run -p golish-db --test cleanup_obligation_kernel cleanup_obligation_kernel_schema_is_installed --status-level fail`
    → exit 100，明确 `missing cleanup table cleanup_obligations`；首次 migration 又因缺 composite
    unique RED，修正后 schema 转绿。
- **GREEN / 已记录证据**：
  - `CARGO_INCREMENTAL=0 cargo nextest run -p golish-db --test cleanup_obligation_kernel --status-level fail`
    → 4/4 passed，run `952ef55a-25e1-42d3-a417-4645a04a9c16`；覆盖 action+obligation
    原子提交/失败回滚、exact replay/evidence immutable、unpaired/read-only hostile SQL、
    one-live attempt、inconclusive retry ordinal、trusted waiver/CAS/residual/replay。
  - `CARGO_INCREMENTAL=0 cargo nextest run -p golish-cleanup-domain -p golish-cleanup-app --status-level fail`
    → 7/7 passed，run `8828bc75-1eb4-4a34-bcfd-bc97e1268e9f`。
  - `CARGO_INCREMENTAL=0 cargo check -p golish-cleanup-domain -p golish-cleanup-app -p golish-db --all-targets`
    → exit 0；cleanup domain/app scoped `cargo clippy --all-targets --no-deps -- -D warnings`
    → exit 0；new files targeted rustfmt、scoped diff check、`check_dag.py`（56 crates）通过。
  - repo ownership guard 仍因共享树历史基线失败，但输出没有 cleanup repo unregistered/
    cross-owner/raw-SQL 新违规；C5 app 不发 raw SQL。
- **当前状态 / 阻塞**：尝试连同 `golish-db` 跑 scoped Clippy 时只命中并行 P1 Task9
  `runtime_memory_tx.rs` 两处 `explicit_auto_deref`，已通知 owner 静态修正；C5 自身无
  diagnostic。全量 Clippy/precommit 尚不能运行：`backend/target` 约 191 GiB、磁盘仅余约
  2 GiB，已按高风险删除规则向用户请求清理 `backend/target/debug/incremental` 授权，未擅删。
  epic 保持 `in_progress`。
- **提交记录**：未 stage、未 commit、未 push；checkpoint 仍是 `ab7b0c4a`。

#### 2026-07-13 · Cleanup P7b worker / capability / two-phase deletion C8

- **本轮目标**：执行 corrected closeout plan Task C8：在 C5 kernel 上开放 lease-fenced
  Cleanup wrapper、DB-global recovery worker、trusted waiver/Gate IPC 与可恢复两阶段组织删除；
  不配置或执行真实 cleanup side effect。
- **已完成实现**：
  - Cleanup stage 四个 wrapper 只接受 exact obligation id 或 bounded waiver suggestion，运行时
    authority 全部从 awaited tool call + operation/execution/unit/org + Worker lease 读取并由 DB
    重载；execute/absence 没有 typed adapter 时写明 unavailable 并 fail closed，AI suggestion 永不
    写 waiver。真实 waiver 只走 local-desktop principal provider + exact row-version CAS。
  - desktop 与 ordinary CLI 在 DB ready 后启动同一 `CleanupCloseoutRuntime`，DB lease 是跨进程
    authority；claim transaction 提交后才调用幂等 artifact cleaner。artifact cleanup result 先
    durable 落为 `artifact_cleanup_succeeded`，hard delete 是独立事务；进程在两者之间退出时，
    下一 tick 优先恢复 DB-only continuation。
  - organization deletion request 在一笔事务锁 subtree/targets、验证 missing/open obligation、
    冻结 organization/target snapshot、按 event catalog 为每项 source invalidation 冻结 projector
    manifest，并将 subtree 置为 organization + target identity read-only。重叠 parent/child job、
    request 后 target 增删/改绑均 fail closed。
  - claim 在 waiting-ready 与 expired-pending 两类 job 中统一按 `requested_at,id` 选择，避免持续
    新 waiter 饿死旧 retry；delivery readiness 只看 request 时冻结的 manifest，后来注册的
    manifest 外 pending projector 不会反向阻塞历史 deletion job。
  - migration `20260712000010_cleanup_closeout.sql` 将 Candidate/Approval/Finding lineage 补为
    `target_id_at_time` + nullable `live_target_id` + canonical snapshot；兼容 trigger 双写 P2 暂留
    的 `target_live_id` API，约束保证两个 live alias 只能等于 exact at-time target。live target/org
    删除后 retained decision/lineage 不丢失。
  - Cleanup obligation/Gate 面板已挂到 `stage_run(stage=cleanup)` 标准工具详情页，具备 loading /
    error / empty 三态；waiver request 不含 actor identity。同步 cleanup-app、golish-db/repo、
    recon organizations 模块卡与主索引。
- **TDD / 审计证据**：
  - 全量 baseline 首次执行 frozen-manifest hostile test 时，在 2727 条通过后稳定 RED：测试夹具
    直接插入未注册 future projector，先触发 `knowledge_projector_registry` FK。修复只是在 registry
    注册合法但不属于 frozen manifest 的 projector/version，再插 pending delivery；没有放宽 FK。
    exact test 随后 exit 0，证明 closeout 仅等待冻结 manifest。
  - 新增并转绿的 DB hostile cases：older unready waiter 不饿死 later ready、older expired retry 不被
    newer waiter 饿死、artifact cleanup success 在 hard delete 前可重启恢复、request 后 target drift
    与重叠 subtree deletion 被拒绝、完整 request → claim → cleanup result → independent hard delete。
- **已记录验证证据**：
  - `just space-guard && cd backend && cargo nextest run -p golish-db --test cleanup_obligation_kernel --status-level fail`
    → exit 0，Cleanup DB integration 全套通过。
  - `just space-guard && cd backend && cargo nextest run -p golish-cleanup-domain -p golish-cleanup-app -p golish-recon-app -p golish-pentest-app -p golish-agent-app -p golish-agent-runtime -p golish -E 'test(cleanup)' --no-tests=pass --status-level fail`
    → exit 0。
  - 同 8 package `cargo check --all-targets` → exit 0；`cargo clippy --all-targets -- -D warnings`
    → exit 0，零 warning。
  - `pnpm exec vitest run frontend/components/Engagement/CleanupObligationList.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts`
    → 14/14 passed；相关 5 文件 Biome → exit 0。
  - 冻结共享树第 19 次 `./init.sh` → exit 0，包含 Rust 5185/5185 passed、前端门禁与类型生成
    门禁；`python3 scripts/check_dag.py` → 58 crates clean；scoped/full `git diff --check` → exit 0。
  - foundation migration SHA-384 保持
    `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`。
- **当前状态 / fail-closed 限制**：production 尚未配置 typed cleanup executor 或 independent
  absence verifier，因此 model-visible execute/verify 会明确 unavailable，不会降级 raw shell/network；
  本轮未发起真实文件清理、外部 API、扫描或 exploit。C8 focused 与共享 init 已绿，但整个
  runtime-memory/candidate epic 仍有并行包收尾，feature 继续 `in_progress`，不单独标 `passing`。
- **以下 C8 文件已修改但未提交**：`backend/crates/golish-db/{migrations/20260712000010_cleanup_closeout.sql,src/repo/organization_deletion_jobs.rs,tests/cleanup_obligation_kernel.rs}`、
  `backend/crates/golish-cleanup-{domain,app}/`、`backend/crates/golish-pentest-app/src/pentest_bridge/cleanup.rs`、
  `backend/crates/golish-agent-app/src/ai/commands/cleanup.rs`、`backend/crates/golish-recon-app/src/organizations/{mod.rs,artifact_cleanup.rs}`、
  `backend/crates/golish/src/{app/bootstrap.rs,app/window_lifecycle.rs,cli/bootstrap/mod.rs,state/mod.rs,commands_facade/cleanup.rs}`、
  `frontend/{lib/api/cleanup.ts,components/Engagement/CleanupObligationList.tsx,components/Engagement/CleanupObligationList.test.tsx}`
  及同一 C8 contract 的 registry/spec/generated type/module-card 改动；共享树还有其他并行包改动，
  本子任务未还原、暂存或提交。
- **下一步最佳动作**：主任务等 Candidate/Reporting focused 验证全部结束后，在冻结树上再跑一次
  `just precommit`，把总 feature verification/evidence 逐项写回 `feature_list.json` 后统一 commit；
  若未来启用真实 cleanup，必须先新增 closed typed executor + 独立 verifier 并单独完成授权/副作用测试，
  不能直接开放 raw runner。
- **提交记录**：未 stage、未 commit、未 push；由主任务统一汇总提交。

#### 2026-07-13 · Post-Exploit P6b capability/router C6

- **本轮目标**：执行 corrected closeout plan Task C6：在 C5 cleanup obligation kernel 之上开放四个 typed Post-Exploit capability，补齐 stage capability/spec、tool registry、trusted runtime fence 与 prepare/execute/reconcile seam；不发起真实 exploit/side effect。
- **已完成实现**：
  - `golish-post-exploit-app::P6bActionService` 增加两阶段 command：prepare 必须是 side-effect action + exact obligation，复用 C5 compound transaction 写 action/obligation/两侧 evidence/`PostExploitActionPrepared.v1` deliveries，再幂等建 pending approval；execute 只接受 persisted action/approval ids + exact approval row version。
  - `post_exploit_actions::begin_approved_execution` 在一笔短事务锁 exact action/obligation，校验 frozen operation/project/snapshot/org、open obligation、active server principal、plan hash、status、expiry 与 CAS version，一次性消费 approval 并把 action 置 `executing`。外部 executor 只在 commit 后调用；response-loss 不可再次 begin，unknown outcome 转 `recovery_required`，stale executing 只 reconcile、不得自动重放。
  - 新增四个 `golish_core::Tool` adapter：`post_exploit_validate_access`、`post_exploit_record_internal_observation`、`post_exploit_build_objective_path`、`post_exploit_execute_action`。全部从 `AgentToolContext` 读取 awaited tool call + operation/execution/unit/org + Worker lease fence，并由 DB 重载 current `v2_only` operation、sealed scope、exact stage/unit/live lease/active tool call。schema 为 `deny_unknown_fields`/foreground-only，不接受 caller-selected operation/project/org/lease/actor、background、raw shell/URL/credential/exploit recipe。
  - 四个 stage 共用 `post_exploit_operator` per-org specialist，hardcoded/registry builder 保持同一 closed tool surface；默认无 raw runner/Finding writer/background control/nested delegation。stage whitelist 每次只显示当前 stage 的一个 wrapper，从而确保这些工具实际运行在 durable Worker lease 下而非无 fence 的主 agent context。
  - 第一、二、三工具分别持久化 Foothold、InternalAssetObservation、AttackPath；identity/hash/row id 由服务端确定性派生。第四工具的 prepare 只允许 closed action/cleanup/proof 枚举；production `DisabledPostExploitActionExecutor` 在 approval 消费前 fail closed，未安装 typed adapter 时绝不降级 raw command。
  - response-loss/跨 org identity 已加固：Foothold/AttackPath/Edge/Observation/Action/Obligation/Approval ids 按 operation+organization+canonical material 稳定派生；deadline/proof/cleanup 全部进入 plan hash，同 plan replay复用 persisted deadline；InternalObservation replay保留首个 server observed_at。Access wrapper 只接 opaque work key，DB 从 verified CandidateAttempt/pending-or-validated FootholdCandidate 重载 frozen target type/value/hash，repo 再做一次 source snapshot equality，模型不能替换 Foothold identity。
  - `stage_capability`/tool taxonomy/四份 stage spec+methodology 已切到每 stage 恰好一个 P6b wrapper；Objective Simulation 声明 cleanup-bound side-effect classes 和 `human_approval.required_before=post_exploit_execute_action`。模块卡、INDEX 与 architecture 已同步。
- **TDD / 已记录证据**：
  - `CARGO_INCREMENTAL=0 cargo check -p golish-post-exploit-app --lib` → exit 0。
  - `CARGO_INCREMENTAL=0 cargo check -p golish-pentest-app --lib` → exit 0；中途依次发现并修正 Candidate context re-export、sub-agent DAG back-edge、C7 memory-domain dependency、SHA-256 encoding与 plan move 等真实 compile diagnostics。
  - `CARGO_INCREMENTAL=0 cargo test -p golish-pentest-app pentest_bridge::post_exploit::tests --lib -- --nocapture` 已完成 test binary 构建；首跑两条 schema tests 因测试函数缺 Tokio runtime 2/2 RED，已改为 `#[tokio::test]`。修复后因磁盘只余约 361 MiB，未冒险重新链接；这是待复跑项，不记录为 GREEN。
  - 新增 DB integration regression（待磁盘解锁后运行）：prepare event + 4 deliveries 同事务、exact approval 只消费一次、response-loss begin 不重放、result evidence 后 terminal success。
  - scoped rustfmt、四个 spec `jq empty`、相关 `git diff --check` 已通过；frozen foundation migration 未改。
- **当前状态 / 阻塞**：最近一次 test link 将磁盘从约 1.7 GiB 压到约 361 MiB；`backend/target/debug/incremental` 仍约 122 GiB。已再次向用户请求清理纯 Cargo incremental cache 授权；未获确认前不删除。C6 不能标完成，尚需复跑两条工具测试、DB integration、scoped Clippy、全量 `just precommit`。
- **提交记录**：未 stage、未 commit、未 push；checkpoint 仍是 `ab7b0c4a`。

#### 2026-07-12 · Runtime memory / Candidate execution / knowledge / closeout V2 planning package

- **本轮目标**：根据用户已确认的攻击顺序与短期/长期记忆方向，先完成一套可交给后续 agent 直接实施的详细设计和计划；本轮只规划，不实施 schema、IPC、runtime、扫描或 exploit。
- **当前树审计结论**：确认 2026-07-02 三阶段攻击骨架已经部分落地，不能从零重做；当前主要缺口是 mutable org subtree、共享 `operation_state.state_blob.agent_run`、per-org worker 缺少 durable identity、Candidate 缺 approval/Attempt/evidence/wave 权威状态、Verification Gate 仍可依赖 deliverable、长期 memory/KG scope/provenance/temporal 边界不足、post-exploit/cleanup/reporting 尚无 typed domain。
- **已完成文档**：
  - 总体设计：`docs/design/2026-07-12-runtime-memory-candidate-pipeline-v2.md`。
  - 总路线图：`docs/superpowers/plans/2026-07-12-runtime-memory-candidate-pipeline-roadmap.md`。
  - 八个独立实施包：Runtime Foundation、Candidate Verification V2、Memory Fabric Core、Structured KG、Scoped RAG、Post-exploit Domain、Cleanup Ledger、Reporting Read Model。
  - 旧 2026-07-02 攻击设计/计划顶部已加“部分被替代”说明；三阶段骨架保留，Candidate/runtime/wave 以后按 V2 继续。
- **关键决策**：operation 在 Scoping PASS 后冻结公司集合；每 stage 有 StageRun/StageRunUnit，每可恢复 worker 有独立 WorkerRun；母子公司只聚合状态，不共享 raw chain/evidence/authorization；确定性命中先进入 Candidate，逐 Candidate approval，Verification 内一次领取一个 CandidateAttempt；只有 terminal Verification 创建 Finding；只有 accepted evidence-backed FactDelta 打开下一波；RAG/KG 只做可重建 prior/projection，不参与 Gate；post-exploit、cleanup、reporting 分别建立 typed domain。
- **独立评审后补齐的硬边界**：P1 统一 stable `project_scope_id`、stage execution/submission identity、worker lease/CAS 与 final-sealed handoff；P2 增加 exact plan approval、DB CandidateReviewBarrier、attack wave unit/global lane、scanner→Candidate→Attempt→Finding 唯一路径；P3-P8 增加 multi-consumer delivery、KG identity/assertion lineage 分离、trusted RAG context/pre-action authorizer、side-effect 前 action+obligation 原子登记、missing-obligation Gate、report claim/citation 与 validation/publication 双轴、历史 retention matrix。
- **feature 状态**：新增父条目 `runtime-memory-candidate-pipeline-v2-2026-07-12`，保持 `not_started`；没有切换当前唯一 `in_progress` 的 `target-surface-fingerprint-network-failure-closure-2026-07-12`。
- **验证**：`jq -e empty feature_list.json` exit 0；唯一 `in_progress` count=1，仍是 `target-surface-fingerprint-network-failure-closure-2026-07-12`；九份计划均各有且仅有一组目标/架构/技术栈头，所有 Markdown code fence 数为偶数；静态扫描结果 `empty_tests=0`、`forbidden_placeholders=0`、`filtered_nextest_without_guard=0`、`broad_git_add=0`、`legacy_single_consumer_outbox=0`、`disallowed_live_org_target_fk=0`、`trailing_whitespace=0`，`CREATE TABLE project_scopes` 仅在 P1 出现一次；八个 child plan 文件全部存在；每个计划中的 `git commit` 前均有 `just precommit`；本轮 tracked scoped `git diff --check` exit 0。按用户本轮 planning-only 约束，未运行 `./init.sh`、代码测试、Clippy、`just precommit` 或真实外部请求。
- **风险/授权**：实施 P1/P2/P3/P4/P6/P7/P8 会新增 migration、修改 `golish-db` 或 IPC，必须先向用户取得明确授权；真实 LLM/scan/exploit/Graphiti/embedding acceptance 也必须单独授权。
- **提交记录**：未 commit、未 stage、未 push；工作树原有大量用户改动，本轮未清理或覆盖。

### 2026-07-11 · Intel / EAS 资产身份与 CLI 闭环

- **本轮目标**：按用户要求梳理域名与 IP 的正确存储/关联方式，修改 Scoping、Target Intel、External Attack Surface 的逻辑，并用“默安科技 / moresec.cn”跑非交互全流程 CLI，最终验证向 Enumeration 的交接。
- **已完成**：
  - 固化资产模型：`targets` 只保存组织明确授权的 domain/url/ip/cidr 身份；`dns_records` 保存 Domain→IP 多对多观测；`real_ip` 仅作确定性显示缓存，不参与授权、关系、存活或 gate。`network_endpoints` 以 IP:port 为身份，`web_origins` 以 exact `scheme://host:port` 为身份；`www`、apex、sibling vhost 永不整资产折叠。
  - 收口 Scoping：stage 内不再暴露 `manage_targets`；只有 stage 前 trusted UI/CLI seed 可授权，scope review 必须是当前窗口且 lifecycle finished 的结构化 value/type/scope，任何模型编辑只是 proposal。
  - 收口 Target Intel：保存全部 A/AAAA；DNS 不自动生成 IP target；URL 型 provider 字段先规范成 concrete hostname；wildcard 仅授权 strict child passive discovery；provider/WHOIS error 保持非终态可重试，缺 provider/key 显式 terminal blocked；freshness 只看本次运行候选，不拿历史 target 配对冒充新发现。
  - 收口 EAS：domain/url 只做 Host/SNI liveness 与 exact-origin Web fingerprint；IP 做 port/liveness 和逐开放端口 service；CIDR 只做 liveness/port，child concrete IP 在下一 wave 自己承担 service/Web；所有 confirmed origin 必须有 WhatWeb。共享 confirmed-origin resolver 被 DB/evidence/worklist 共用，display name、foreign URL port、CIDR child origin 均不能投影成目标真值。
  - 固化工具边界：EAS specialist 只看到 `eas_probe_http_liveness`、`eas_discover_ports`、`eas_fingerprint_services`、`eas_fingerprint_web_stack`。AI 不直接调用 raw nmap/httpx/WhatWeb；wrapper 内部可执行固定 recipe。generic Pentester 在 specialist 外仍可能暴露 `pentest_run`，两者不混为一谈。
  - 修复 wave 超过 200 条时的预存资产丢失、WhatWeb runtime/parse 假成功、URL→host promotion、source-error terminal 夹具等回归；同步三阶段 spec/methodology、设计/计划及 17 张模块卡。无 schema/migration、无生成 IPC 类型变更。
  - 按用户授权清掉 `backend/target/debug/incremental`（约释放 50 GiB），后续全程使用 `CARGO_INCREMENTAL=0`；最终 incremental 为 0B。清理 3 个 live smoke 遗留的临时 pg-embed 进程，无 stage-run/nmap/naabu/httpx/WhatWeb 残留。
- **运行过的验证**：
  - URL→host TDD：run `3df6a978-8bf1-4a1c-b8ea-018404ce4fb4` → 11/11；独立复核 run `da70f54b-dfaa-435e-898c-6feeecdf1c33` → 4/4。
  - EAS/identity 跨 crate 独立复核 run `f7f96765-8d25-4da3-ab11-fa9225bbceb2` → 199/199（1492 skipped）。
  - 最终 11-package 相关全量 run `314df415-d20f-4db0-9e38-171c2d8c5ca5` → 3101/3101（11 skipped，16 slow）；selected `cargo clippy --all-targets -- -D warnings` → exit 0；ScopeReviewTable Vitest → 16/16。
  - `CARGO_INCREMENTAL=0 python3 scripts/stage_smoke.py --profile red_team --provider deepseek --model deepseek-v4-flash --from scoping --to external_attack_surface --workspace /tmp/golish-moresec-full-20260711-9 --org '默安科技' --target moresec.cn --json --run-tree ...` → exit 0；session `stage-run-5e665476-5f96-4c90-99b0-e4e94c692faa`。
  - `CARGO_INCREMENTAL=0 just precommit` → exit 0，最终打印 `✓ All checks passed!`；其中 fmt、check-fe、test-fe、workspace lint-rust、test-rust-all、check-types、第二套 test-rust 全部通过。`git diff --check` → exit 0；`feature_list.json`、全部 stage spec、全部 `resources/toolsconfig/*.json` → JSON valid。
  - 按用户明确指令没有重复运行 `./init.sh`；其覆盖的最终门禁已由本轮完整 `just precommit` 实跑。
- **已记录证据**：
  - MoreSec 三阶段均 PASS：Scoping run `bc7d77f8-350c-4728-80e1-ca705c55722b`、Target Intel specialist run `687a1c39-5db0-4855-9340-bd275688f105`、EAS specialist run `88fb4ad4-2718-4f96-aa52-e09466967d56`；executor 只访问这三阶段，Enumeration 计数为 0。
  - live DB：唯一 target 为 `domain moresec.cn`（source `stage-run-seed`），`real_ip=115.28.135.55` 但未授权为 executable IP；六条 DNS（1 A、2 MX、3 TXT），IP/CIDR target=0；exact origin=`https://moresec.cn:443`；EAS LIVENESS found=1、WEB-FINGERPRINT found=1。
  - EAS transcript 只调用 `eas_probe_http_liveness` 与 `eas_fingerprint_web_stack`；wrapper 内部分别落 httpx evidence id 9、WhatWeb evidence id 12。`eas_discover_ports`、`eas_fingerprint_services`、`pentest_run`、raw nmap、raw naabu 均为 0。
  - ASN/CT provider/credential 不可用被写成显式 terminal blocked outcome；ENScan 本次未发现新 subdomain。因此本证据证明可用 provider 与显式 gap 下的确定性闭环，不夸大为全球资产发现完整性。
- **commit 记录**：无；未 stage、未 commit、未 push。
- **风险/边界**：generic Pentester 仍可在 EAS specialist 边界之外使用 `pentest_run`；这不是本阶段 AI 直调 raw nmap。后续若补 ASN/CT credential，可重跑 Intel 获得更广 passive coverage，但当前缺口已可审计且不会伪装 checked-empty。没有用 DNS 关系扩大主动扫描授权。
- **本轮修改但未提交**：`backend/Cargo.lock`；`backend/crates/{golish-agent-app,golish-agent-kit,golish-agent-runtime,golish-app-core,golish-db,golish-pentest-app,golish-pentest-domain,golish-pentest,golish-recon-app,golish-sub-agents,golish-tools,golish}/**` 中本轮 Intel/EAS/scoping/harness/output-store/stage-run 文件；新增 `golish-agent-kit/src/harness/gate/eas_web_origin_check.rs`；`frontend/components/AIChatPanel/ScopeReviewTable{,.test}.tsx`；`resources/harness/stages/{scoping,target_intel,external_attack_surface}/{spec.json,methodology.md}` 与 `resources/toolsconfig/{httpx,naabu,nmap,whatweb}.json`；17 张 `docs/modules/**` 卡与 INDEX；新设计/计划 `docs/{design,superpowers/plans}/2026-07-11-intel-eas-asset-identity-closure.md`；两份被 supersede 的旧设计/计划标注；`feature_list.json`、`agent-progress.md`。
- **下一步最佳动作**：Intel/EAS 本身无需继续补逻辑；下一次红队主流程可直接从这份 EAS exact-origin/IP endpoint 真值进入已通过的 Enumeration。若要落版本，先由用户决定是否提交当前大工作树；本轮不擅自 commit/push。

### 2026-07-11 · 信息收集闭环、durable chain 与 MCP 安全收口

- **本轮目标**：按用户“后面的事情你自己解决、P0/P1/P2 全部跑完、继续”的授权，复核现有信息收集逻辑，修完确定性正确性/恢复/性能与启动问题，在真实 Test1 上得到 Enumeration PASS，并完成全仓验证与交接。
- **已完成**：
  - Enumeration 的单一事实口径已收口到“每个 EAS-confirmed exact Web Origin × JS/DIR/PARAM/JSAPI 四轴”：origin、current run/stage cutoff、producer owner、evidence source、terminal outcome、submit preview、org gate、pass token 与前端 read model 使用同一合同；partial/error 不再伪装 terminal。
  - producer 链已具备 bounded continuation：50 roots / 200 cells 的批次、transport preflight、browser/JS/route durable checkpoint、attempt generation、blocked exhaustion、wave/cascade 与 372-origin worklist 都能从 DB truth 恢复；legacy `js_collect` 的 7 个既有授权删除保持不变。
  - durable chain 已从“过长文本历史”改成 DB-backed 可寻址恢复：初始 checkpoint 在任何 provider 请求前写入；assistant+完整 ToolResults 原子成批 checkpoint；只保留完整 newest unit 的连续后缀并受约 512 KiB provider budget 约束；structured `chain_id` 穿过 sub-agent result、outer timeout、runtime 与 stage_run，generic/finalize/context-limit error 也携带最后一个成功 checkpoint。hard kill 时不伪造半个 tool batch，而由 DB worklist/最后成功 checkpoint 恢复。
  - 解释了此前耗时根因：历史 chain 曾膨胀到约 3.47 MB / provider 侧约 120 万 token，超过 DeepSeek 约 1M 上限；旧 compaction 会留下历史洞、checkpoint 时点不原子、generic retry 丢 chain identity，再叠加 1488 个 coverage cell 与错误续跑，表现为“很久没搞完”。这些不是单一模型慢，而是数据规模、上下文合同和恢复身份三个问题叠加。
  - startup/recovery 修复包括：跨 stage worker graph/chain ownership reaper、合法 durable operation 不再被误判 abandoned、exact resume CLI、`just dev <workspace>` 双 `--` 分隔符；一次磁盘不足通过清理可重建 Cargo 产物释放约 190 GB，没有删除业务数据。
  - Test1 live Enumeration 已在同一个 durable run 中最终 PASS；之后审计又发现并修复独立 MCP P0/P1/P2：未信任 `.golish/mcp.json` 不再自动进入 GUI/CLI manager，project/user/builtin 来源按真实 precedence，builtin setup 不再接受 override path，`QBIT_WORKSPACE`/cwd 不能冒充 builtin，缺少 generated runtime 的 `js-reverse` fail closed。
  - 新增 MCP 设计/计划与模块卡同步；`enumeration-four-axis-ip-web-2026-07-01`、`mcp-trust-builtin-provenance-2026-07-11` 均已转 `passing`，`feature_list.json` 当前 0 个 `in_progress`。
- **运行过的验证**：
  - Test1：`python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 stage-run-476558c3-c22a-4009-a82e-17e086a005de --full --db` → exit 0；同 run 的 `stage_run` 返回 `passed=true`、`gaps=[]`、`passed_orgs=1/1`。
  - 只读 Postgres strict audit → origins 372、rows/terminal 1488/1488、nonterminal/unexpected/bad_source/bad_evidence/bad_origin/stale 均 0；每个 origin 恰好 4 轴；worklist 337 done + 35 evidence-backed blocked + 0 pending；completion token 独立重算一致。
  - 浏览器/覆盖：`node --check scripts/browser_collect_js_api.mjs && node --test scripts/browser_collect_js_api.test.mjs` → 19/19；`pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 26/26；`just check-fe` → exit 0。
  - durable chain：selected sub-agent/runtime combined nextest → 495/495；最终 Enumeration 相关四 crate selection → 1520/1520，4 skipped；selected six-crate clippy `-D warnings` → exit 0。
  - MCP TDD：旧 loader 的 untrusted project server 断言先失败；旧 resolver 的 QBIT workspace 断言先失败；source/setup helper 先编译失败；只有 `mcp.js`、缺 transitive runtime 的 source/build 两测先失败。修复后 `cargo nextest run -p golish-mcp -p golish -E 'package(golish-mcp) | test(mcp)'` → 57/57；`cargo clippy -p golish-mcp -p golish --all-targets -- -D warnings` → exit 0。
  - 最新二进制：`just dev /Users/christopherzheng/golish-platform/Test1` 成功编译并运行；`curl http://127.0.0.1:1420` 成功；Postgres `SELECT 1, COUNT(*) FROM _sqlx_migrations` → `1|74`；06:29–06:30 启动窗口有 No MCP servers configured / migrations complete / frontend ready / pgvector ready，且无 js-reverse、ERR_MODULE_NOT_FOUND、connect failure、ERROR、panic。Ctrl-C 后 1420 与 dev processes 均 clear。
  - 最终门禁：`CARGO_INCREMENTAL=0 ./init.sh` → exit 0（fmt/check-fe/test-fe/lint-rust/test-rust-all 105s/check-types 全绿）；`CARGO_INCREMENTAL=0 just precommit` → exit 0，最终输出 `✓ All checks passed!`。
- **已记录证据**：
  - live identity：org `0a431390-7726-48e5-b0a8-e692a9070e33`；operation/task `462b6c9f-2a0d-48af-8ff0-8b5c08416196`；DB session `a15c0b0f-23ff-42f9-b950-7dcaf25de860`；durable chain `552240a7-6050-460b-876b-bd51a4ccba5f`；stage attempt cutoff `2026-07-11T01:23:54.697594Z`。
  - gate PASS token：`ab585f3b4e828ca92dacdb690715f8210f6d7b6b151cbfd7f70022753e0f1365`；DB completion row、latest gate_decision 与独立 token recomputation 三者一致。
  - clean-state 条件核对：未新增 Tauri command、CRUD、手写 IPC/generated type 或事务内外部 I/O；未改 release/tag；没有业务 DB 手工写入；module cards/INDEX 已同步；`git diff --check` 与 JSON/状态审计在收尾后复跑。
- **提交记录**：用户已于 2026-07-11 明确要求提交当前改动；本轮以单一 `feat(harness): close enumeration and harden durable execution` checkpoint commit 落库，未 push，也未创建/切换 branch。migration `20260710000001_technique_outcomes_org_scoped_unique.sql` 已在 Test1 嵌入式 PG 应用（migration 总数 74）：它是 row-preserving 的 owner identity 扩展与空 owner backfill，但约束名改变后旧 binary writer 不兼容，必须使用最新 binary。用户的“我去睡觉了，后面的事情你自己解决，要全部搞完”和连续“继续”被本轮视为完成 P0/P1/P2、live Test1 与必要 migration 的广泛明确授权；因为没有另一次只针对 migration 的窄确认，这一偏差在此显式留档。
- **已知风险或未解决问题**：
  - live PASS 由最终 P1 chain-addressability/MCP 审计补丁之前正在运行的 producer/gate binary 产出；后置补丁有独立红绿测试、全套 nextest/clippy/init/precommit 和最新 binary startup smoke，但不能把历史 transcript 说成“由最终源码逐字构建”产出。
  - 进程在 provider/tool batch 正中间被 hard-kill 时只恢复到最后一个完整 checkpoint，并继续读取 DB worklist；这是刻意的 fail-safe，不会把未完成 batch 伪造为成功。
  - MCP 项目审批/预览 UI、trust 后 hot reload、content hash/revoke 尚未实现；当前未信任项目配置安全地不可执行，批准后需下次 MCP 初始化（当前为重启）。startup reaper 的正确 SQL 在当前数据上约 2.5s，属于可观测性能后续，不是正确性 blocker。
  - 本次 checkpoint 很大但已完全盘点：308 项 = 245 modified + 7 authorized deletes + 56 new files；未发现 target/log/capture/temp、密钥、数据库文件、缓存或二进制误入。
  - **本次 checkpoint 涵盖：** `backend/**` 178 项（Enumeration/DB/evidence/gate/worker/runtime/CLI/MCP 与 1 个已应用 migration）；`frontend/**` 13 项（coverage/read-model/测试）；`resources/**` 8 项（stage/tool contracts）；`scripts/**` 7 项（browser helper/tests/run audit）；`docs/**` 99 项（design/plan/module cards）；根目录 `agent-progress.md`、`feature_list.json`、`justfile` 各 1 项。7 个删除全部是 `backend/crates/golish-pentest-app/src/pentest_bridge/js_collect{.rs,/**}`，按用户此前明确“删掉”保留。
- **下一步最佳动作**：保持本地 checkpoint 不 push；若继续开发新功能，从 `feature_list.json` 重新选择一个最高优先级 `not_started` 项并设为唯一 `in_progress`。优先处理跨阶段 `stage_runs` 终态、Attack Candidate/Verification DB 权威闭环，不要把尚未实现的 MCP 审批 UI 混回已 passing 的 Enumeration。

### 2026-07-08 · Enumeration base_url/root_url TDD 修复

- **本轮目标**：回应用户“你确定吗？先写测试再修”，把最新 Test1 Enumeration 47 个 DIR error 的主要根因（错误 origin/默认端口、worklist 不给 base_url）先落成失败测试，再修到绿灯。
- **先红灯确认（实跑）**：
  - `cd backend && cargo test -p golish-pentest-app canonical_candidate_corrects_inferred_https_to_confirmed_http_non_default_port --lib` → fail；`best_web_service_candidate` 返回 `None`，不能把模型推导的 `https://package.moresec.cn/` 纠到 DB-confirmed `http://package.moresec.cn:8080/`。
  - `cd backend && cargo test -p golish-agent-kit web_root_url_from_meta_prefers_confirmed_open_url_over_filtered_default_port --lib` → fail；当前 root 推导把 filtered `443/https` 拼成 `https://43.248.78.209/`，没有选 open `http://43.248.78.209:8080/`。
  - `cd backend && cargo test -p golish-agent-kit enumeration_preflight_gap_examples_include_base_url --lib` → fail；`gap_examples[0].base_url` 为 `Null`。
  - `cd backend && cargo test -p golish-agent-kit stage_worklist_next_includes_base_url_for_enumeration_items --lib` → fail；`items[0].base_url` 为 `Null`。
- **已完成**：
  - `target_resolver` 的 canonicalize 现在优先读取 `targets.ports[].url` confirmed-open HTTP(S) origin，并忽略 filtered/closed 端口；`target_assets` 仍作兜底。
  - 保留显式端口保护：调用方显式带端口时不随便改；模型推导的默认 `https://host/` 可纠到 DB-confirmed 非默认 `http://host:8080/`。
  - `web_root_url_from_meta` 优先使用 `ports[].url`，并只接受 open/空状态端口；filtered/closed 不再成为 Enumeration root。
  - `stage_worklist_next.items` 与 `check_stage_asset_coverage.gap_examples` 在 Enumeration 下直接带 `root_url` / `base_url` / `scheme` / `port`，DIR gap 可直接喂给 `route_probe_paths`。
  - 同步模块卡：`golish-agent-kit/tool_executors.md`、`golish-pentest-app/pentest_bridge.md`。
- **修后验证（实跑）**：
  - `cd backend && cargo fmt --package golish-agent-kit --package golish-pentest-app` → exit 0。
  - `cd backend && cargo test -p golish-pentest-app pentest_bridge::target_resolver::tests:: --lib` → 12 passed / 163 filtered out。
  - `cd backend && cargo test -p golish-agent-kit tool_executors::security::tests:: --lib` → 17 passed / 825 filtered out。
  - `cd backend && cargo check -p golish-agent-kit -p golish-pentest-app` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-kit/Cargo.toml backend/crates/golish-agent-kit/src/tool_executors/security.rs backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs docs/modules/backend/golish-agent-kit/tool_executors.md docs/modules/backend/golish-pentest-app/pentest_bridge.md backend/Cargo.lock` → exit 0。
- **未跑 / 风险**：未跑 `./init.sh` / full `just precommit`；未重启 app 做 fresh Test1 live Enumeration rerun。当前验证是针对本次根因的窄单测 + package check。工作树进入本轮前已有大量未提交改动，本轮没有回退或整理无关文件。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-kit/Cargo.toml`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs`、`docs/modules/backend/golish-agent-kit/tool_executors.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`backend/Cargo.lock`、`agent-progress.md`。

### 2026-07-08 · EAS confirmed-open 端口补扫与 empty outcome 防覆盖

- **本轮目标**：回应用户“新跑一次怎么还是 retry 两次 / 你再核对一下 / 修”，修复最新 Test1 EAS run 中 confirmed-open 端口仍被漏扫、以及空 naabu batch 覆盖旧 open-port DB truth 的问题。
- **诊断结论**：
  - 最新 run `pentest-chat-1783439658234-1` 前两次 BLOCK 不是 9001 parser 同一个根因，而是 DB/read-model 不一致：`targets.ports[]` 仍保留 `222.186.129.58:82 state=open service="" source=naabu evidence_id=19880`，但后续 `nmap -sV` 只扫了 `22,80,8083`，没有扫 `82`。
  - 同一 run 后续 naabu batch 对 `222.186.129.58` 没命中，于是旧逻辑写了 `GOLISH-EAS-PORT empty` / port-derived `GOLISH-EAS-LIVENESS empty`，但 `targets.ports[]` 里仍有 confirmed-open `82`，造成 technique_outcomes 与 DB truth 互相打架，repair/preflight 继续反复。
- **已完成**：
  - `coverage_truth.rs` 新增 `confirmed_open_service_ports_for_assets` 和 JSON 解析 helper，统一解析当前 in-scope `targets.ports[]` 的 open non-53 服务端口；弱服务名 `open/unknown/tcpwrapped` 仍算缺 service surface。
  - `eas_fingerprint_services` 的 `ports` 从硬必填改为可选/可补：即使模型只传 `80,443` 或漏传，wrapper 也会从当前 workspace DB 自动合并同 IP 的 confirmed-open 端口，再跑 `nmap -sV`；结果回传 `effective_ports/db_open_ports/auto_added_ports`。
  - 后台 `naabu` / `masscan` batch completion 写 PORT/LIVENESS empty 前会查当前 org 的 confirmed-open ports；如果 DB 仍有 open port，则跳过 empty outcome，避免 top-ports/分批扫描的空输出覆盖旧事实。
  - `StageAssetCoverageCell` 增加 Rust-only `details`，SERVICE-FINGERPRINT pending/error 会带 `details.missing_open_ports` 和推荐 `eas_fingerprint_services` 参数；`check_stage_asset_coverage` compact `gap_examples` 透传 details。
  - 同步模块卡：`golish-db/repo.md`、`golish-agent-app/ai.md`、`golish-agent-kit/tool_executors.md`、`golish-pentest-app/pentest_bridge.md`；`feature_list.json` 追加本轮 scoped evidence，状态仍保持 `in_progress`。
- **运行过的验证（实跑）**：
  - `git diff --check -- backend/crates/golish-db/src/repo/coverage_truth.rs backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs` → exit 0。
  - `rustfmt --edition 2021 backend/crates/golish-db/src/repo/coverage_truth.rs backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs` → exit 0（第一次裸 `rustfmt` 因默认 Rust 2015 解析 async 失败，随后用 edition 2021 成功）。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-pentest-app service_ports --lib` → 5 passed / 166 filtered out。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-db confirmed_open_service_ports_json --lib` → 1 passed / 207 filtered out。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-db weak_service_names_are_missing_service_fingerprint_json --lib` → 1 passed / 207 filtered out。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-agent-kit coverage_preflight_preserves_gap_details --lib` → 1 passed / 838 filtered out。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-agent-app empty_port_outcome_is_skipped_when_db_still_has_open_ports --lib` → 1 passed / 167 filtered out。
  - `cd backend && CARGO_TARGET_DIR=/tmp/golish-codex-target cargo test -p golish-agent-app eas_service_pending_exposes_missing_open_ports --lib` → 1 passed / 167 filtered out。
  - `rm -rf /tmp/golish-codex-target` → exit 0，验证产生的临时编译产物已清理。
  - `rm -rf backend/target` → exit 0，仓库内 Rust 编译产物已按用户要求清理；复查 `test ! -e backend/target` / `test ! -e /tmp/golish-codex-target` 均通过。
- **未跑 / 风险**：未跑 `./init.sh` / full `just precommit`，因为用户刚要求清理编译产物且本轮只做 EAS 窄修；未重启 app 做 fresh Test1 live rerun，需后端重启后再跑一次 EAS 观察 retry 是否消失。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/coverage_truth.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`、`backend/crates/golish-agent-app/src/ai/commands/{bridge_config.rs,stage_coverage.rs}`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`docs/modules/backend/{golish-db/repo.md,golish-agent-app/ai.md,golish-agent-kit/tool_executors.md,golish-pentest-app/pentest_bridge.md}`、`feature_list.json`、`agent-progress.md`。

### 2026-07-07 · EAS 9001 filtered 端口反复修复

- **本轮目标**：回应用户“为什么 9001 有问题就反复跑，改吧”，修复 nmap 已看到 `filtered/closed` 但未落入 `targets.ports[].state` 导致 EAS SERVICE-FINGERPRINT 反复补洞的问题。
- **已完成**：
  - `resources/toolsconfig/nmap.json` 的真实输出 parser 从只匹配 `open` 改为匹配 `open/closed/filtered`，并把状态写入 `state` 字段。
  - `output_parser` 增加真实 nmap toolsconfig 回归：`9001/tcp filtered tor-orport` 和 `3306/tcp closed mysql` 都能生成带 `state`、`service`、`host` 的 port record。
  - `docs/modules/backend/golish-pentest/output_store.md` 同步说明：nmap terminal state 要覆盖旧裸端口，避免已 filtered/closed 的端口继续进入 EAS SERVICE 分母。
  - 保持现有 output-store merge/gate 口径不变：同 port/proto 的新 JSON 会覆盖旧 JSON；`filtered/closed` 不是强服务指纹，也不会写成 informative fingerprint。
- **运行过的验证（实跑）**：
  - `python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null` → exit 0。
  - `cd backend && cargo fmt -p golish-pentest --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest nmap --status-level fail` → 6 passed / 165 skipped。
  - `cd backend && cargo nextest run -p golish-pentest output_store --status-level fail` → 35 passed / 136 skipped。
  - `git diff --check -- resources/toolsconfig/nmap.json backend/crates/golish-pentest/src/output_parser.rs docs/modules/backend/golish-pentest/output_store.md` → exit 0。
- **未跑 / 风险**：未跑 full `just precommit`；未重启 app 做 fresh Test1 live EAS smoke。当前修复是窄链路：让 nmap terminal port state 进 DB 真值，针对本次 9001 filtered 复发。
- **本轮修改但未提交（本 scope）**：`resources/toolsconfig/nmap.json`、`backend/crates/golish-pentest/src/output_parser.rs`、`docs/modules/backend/golish-pentest/output_store.md`、`feature_list.json`、`agent-progress.md`。

### 2026-07-07 · Target Surface 移除 Crawl tab

- **本轮目标**：按用户确认删除 Web Origin detail 中旧 Katana/crawler 结果展示 tab，避免把 seed/observation 误读为 Enumeration 最终成果。
- **已完成**：
  - `WebOriginsTab` 移除 `Crawl` detail tab、`CrawlObservationList`、Web Origins 总表里的 `Crawl` 计数列，以及相关未使用 helper/import。
  - 后端 `crawl_observations` / 前端 DTO 兼容数据不动；主展示继续走 Sitemap / APIs / JS / Params / Evidence。
  - `docs/modules/frontend/components.md` 同步说明：`crawlObservations` 仍是兼容 observation 数据，但不再作为主 surface tab 展示。
- **运行过的验证（实跑）**：
  - `pnpm exec biome check frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx docs/modules/frontend/components.md` → exit 0。
  - `pnpm exec tsc --noEmit --pretty false` → exit 0。
  - `pnpm exec vitest run frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts` → 3 files passed / 41 tests passed。
  - `git diff --check -- frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx docs/modules/frontend/components.md` → exit 0。
- **未跑 / 风险**：未跑 full `just precommit`；未启动 dev app 做视觉截图。当前为窄前端删除，已通过类型检查与相关 surface 单测。
- **本轮修改但未提交（本 scope）**：`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。

### 2026-07-07 · Katana seed 反哺 browser_collect_js_api

- **本轮目标**：按用户要求实现“Katana 发现值得真实浏览器访问的入口 → 喂给 `browser_collect_js_api` → Playwright 真实打开触发更多 runtime JS/XHR/fetch”，并本地实测效果。
- **已完成**：
  - `enum_crawl_same_origin_urls` 仍固定包装 `katana -list {{input_file}} -jc -silent -d N`，但现在接受 worklist object（保留 `target_id`），并在结果里追加 `browser_seed`。
  - `browser_seed.target_urls` 会按 root 分组输出 `{target_id,target_url,recipe:{routes,script_urls}}`：Katana stdout 中同源 page routes 进入 `recipe.routes`，JS URLs 进入 `recipe.script_urls`，API-ish/query URL 作为 `api_candidate_urls` 低置信提示保留，不当作最终 browser 观测。
  - `browser_collect_js_api` batch 现在支持每个 target entry 自己携带 `recipe`，多 root 批量时不会把 A 站 Katana routes 喂给 B 站。
  - Enumerator methodology / prompt 从旧的“browser → katana supplement”改成“katana seed discovery → browser_collect with `browser_seed.target_urls` → js_extract → route_probe”。
  - 同步模块卡：`golish-pentest-app/pentest_bridge.md`、`golish-sub-agents.md`。
- **实测结果（本地 fixture）**：
  - fixture：`/` 只加载 `/assets/main.js`；`main.js` 暗藏 `/settings/billing`；只有打开 `/settings/billing` 才加载 `/assets/billing.chunk.js` 并发 `GET /api/billing/invoices?limit=10`。
  - `katana -u http://127.0.0.1:63801/ -jc -silent -d 2` → 发现 `/settings/billing` 和 `/assets/billing.chunk.js`。
  - baseline：`node scripts/browser_collect_js_api.mjs --url http://127.0.0.1:63801/ --workspace /tmp/golish-browser-direct.tGTZF4 --max-pages 1 --max-actions 0 --ai-assist false --hard-timeout-ms 30000 --timeout-ms 8000` → `scripts_saved=1`、`api_requests_total=0`、只访问首页。
  - seeded：同参数但加 `--max-pages 3 --recipe-json '{"routes":["/settings/billing"],"script_urls":["/assets/billing.chunk.js"]}'` → `scripts_saved=2`、`api_requests_total=1`、`pages_visited` 包含 `/settings/billing`，捕获 `GET /api/billing/invoices?limit=10`。
- **运行过的验证（实跑）**：
  - `rustfmt --edition 2021 backend/crates/golish-pentest-app/src/pentest_bridge/enumeration_capabilities.rs backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-sub-agents/src/defaults/tests.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app --status-level fail enumeration_capabilities browser_collect_js_api` → 23 passed / 144 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents --status-level fail defaults` → 25 passed / 99 skipped。
  - `git diff --check -- <本轮触及路径>` → exit 0。
- **未跑 / 风险**：未跑 full `just precommit`；本轮没有新增 `browser_visit_queue` DB 表或 migration，先用工具 payload 里的 per-target `recipe` 表达 visit queue，避免在实测前引入 schema 风险。真实 stage smoke 还需在 dev app / Test1 上跑完整 Enumeration 才能观察模型是否稳定采用 `browser_seed.target_urls`。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{enumeration_capabilities.rs,browser_collect_js_api.rs}`、`backend/crates/golish-sub-agents/src/defaults/{prompts/execution_planning.rs,tests.rs,builder/mod.rs}`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-sub-agents.md`、`agent-progress.md`。

### 2026-07-07 · 删除 js_collect 静态 JS 采集工具

- **本轮目标**：按用户确认“删啊 js_collect 能有啥用”，移除旧 `js_collect` bridge 工具，只保留 `browser_collect_js_api` 采集 JS/API + `js_extract_apis` 静态分析路径。
- **已完成**：
  - 删除 `backend/crates/golish-pentest-app/src/pentest_bridge/js_collect.rs` 及 `js_collect/` 子模块，`create_pentest_bridge_tools` 不再注册 `JsCollectTool`，`golish-pentest-app` 去掉仅供该工具使用的 `golish-projects` 依赖。
  - execution mode / prompt render / tool taxonomy / stage refiner / sub-agent 默认工具、prompt、response parsing 全部去掉 `js_collect` 正向暴露；UI 工具名映射和 pentest system prompt 改为 `browser_collect_js_api` → `js_extract_apis`。
  - `js_extract_apis` 的描述和 no-captures 提示改为“先跑 `browser_collect_js_api`”；`browser_collect_js_api` 不再把 `js_collect` 描述为静态 fallback。
  - 同步当前模块卡：`golish-pentest-app/pentest_bridge`、`golish-agent-kit/harness`、`golish-projects/file_storage`、`golish-sub-agents`；历史 archive/design/plan 旧记录未改。
- **运行过的验证（实跑）**：
  - `./init.sh` → pre-edit baseline 失败；失败点是既有 unrelated clippy：`golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs:1012-1014`、`golish-recon-app/src/asset_intel/agent_intel.rs:126-130` 的 `unnecessary_map_or`，未进入本轮修复 scope。
  - `rg -n "js_collect|JsCollect" backend/crates frontend resources scripts docs/modules --glob '!target' --glob '!node_modules'` → 只剩 3 个负向测试断言字符串与 `kind: "js_collection"` evidence kind，无正向工具暴露。
  - `rustfmt --edition 2021 <本轮触及 Rust 文件>` → exit 0。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/components/AIChatPanel/pentestSystemPrompt.ts` → exit 0。
  - `python3 -m py_compile scripts/gen_pentest_completeness_template.py scripts/check_repo_ownership.py` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app -p golish-agent-runtime -p golish-agent-kit -p golish-sub-agents -p golish-prompts -p golish-pentest -p golish-db -p golish-app-core` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime --status-level fail execution_mode` → 23 passed / 269 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit --status-level fail tool_taxonomy` → 20 passed / 818 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents --status-level fail defaults` → first run exposed 2 stale prompt assertions; after assertion update, 25 passed / 99 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents --status-level fail response_parsing` → 33 passed / 91 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app --status-level fail browser_collect_js_api` → 17 passed / 147 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app --status-level fail js_extract_apis` → 32 passed / 132 skipped。
  - `python3 scripts/check_repo_ownership.py` → exit 1；当前全仓已有 133 ownership + 8 raw-sql violation(s)，输出已无被删除的 `js_collect` 路径，但该 guard 仍是全仓红灯。
- **未跑 / 风险**：未跑 full `just precommit`；本轮 `./init.sh` baseline 已被 unrelated clippy 阻塞。未清理历史 docs/archive/design/plan 中的旧 `js_collect` 考古引用。
- **本轮修改但未提交（本 scope）**：删除 `backend/crates/golish-pentest-app/src/pentest_bridge/js_collect*`；修改 `backend/Cargo.lock`、`backend/crates/golish-pentest-app/{Cargo.toml,src/pentest_bridge/{mod.rs,browser_collect_js_api.rs,js_extract_apis.rs},src/pentest_ai/list_tools.rs}`、`backend/crates/golish-agent-runtime/src/execution_mode/*`、`backend/crates/golish-agent-kit/src/{harness/tool_taxonomy.rs,task_orchestrator/stage_refiner.rs,tool_executors/security.rs}`、`backend/crates/golish-sub-agents/{prompts/*.tera,src/defaults/**,src/executor*,src/executor_types.rs}`、`backend/crates/golish-prompts/src/system_prompt/team_delegation.rs`、`frontend/{lib/tools.ts,components/AIChatPanel/pentestSystemPrompt.ts}`、`scripts/{check_repo_ownership.py,gen_pentest_completeness_template.py}`、相关模块卡与 `agent-progress.md`。

### 2026-07-07 · browser_collect_js_api 默认 JS closure 限制取消

- **本轮目标**：按用户要求取消 `browser_collect_js_api` 默认 JS closure 限制：`max_recursive_scripts=1000`、单 JS `max_script_bytes=5MB`、fetch/body timeout、整体 `hard_timeout_ms`。
- **已完成**：
  - `scripts/browser_collect_js_api.mjs` 默认改为 unlimited：`max_recursive_scripts` / `max_script_bytes` / fetch/body timeout / hard deadline 缺省均不封顶，结果 JSON 用 `null` 表示 unlimited，实时 stderr 显示 `unlimited`。
  - Rust wrapper 默认用 `0` 表示 unlimited，并把 `max_script_bytes` 作为可选显式限流参数传给 helper；只有 `hard_timeout_ms > 0` 时 Rust 外层才启用 `hard_timeout_ms + 5s` kill fail-safe。
  - 同步 `scripts/js_api_ai_recipe_probe.mjs` / `scripts/js_api_pipeline_test.mjs`，避免 probe/test 脚本继续塞旧 `1000/120000` 默认限制。
  - `js_extract_apis` 默认取消 AI/HaE 分类侧截断：不再默认限制 AI 文件数、AI 源码字节数、每文件 network window 数、HaE route triage 候选数、返回的 `rule_matches` / `hae_route_candidates` 数；新增显式限流入参 `max_ai_files` / `max_ai_bytes` / `max_ai_chunks_per_file` / `max_hae_route_triage_candidates` / `max_hae_route_candidates_returned` / `max_rule_matches_returned`。
  - 共用 `ai_oneshot::call_llm_json` 默认不再有 60s 工具层 timeout；`browser_collect_js_api` / `js_extract_apis` 新增显式 `ai_timeout_ms`，正数才限时，0/缺省为 unlimited。
  - 更新模块卡 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`，把旧“必须有硬截止”改为“默认 unlimited，显式正数才限流”。
- **运行过的验证（实跑）**：
  - `node --check scripts/browser_collect_js_api.mjs` → exit 0。
  - `node --check scripts/js_api_ai_recipe_probe.mjs` → exit 0。
  - `node --check scripts/js_api_pipeline_test.mjs` → exit 0。
  - `cd backend && cargo fmt -p golish-pentest-app --check` → exit 0。
  - `cd backend && cargo test -p golish-pentest-app browser_collect_js_api --lib` → 17 passed / 170 filtered out。
  - `cd backend && cargo test -p golish-pentest-app js_extract_apis --lib` → 31 passed / 158 filtered out。
  - `cd backend && cargo test -p golish-pentest-app ai_oneshot --lib` → 7 passed / 182 filtered out。
  - `git diff --check -- scripts/browser_collect_js_api.mjs scripts/js_api_ai_recipe_probe.mjs scripts/js_api_pipeline_test.mjs backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/ai_oneshot.rs backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_extract.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
  - `node scripts/browser_collect_js_api.mjs --url 'data:text/html,<html><body>golish</body></html>' --workspace /tmp/golish-js-unlimited-smoke --max-pages 1 --max-actions 0 --ai-assist false` → exit 0；stderr 显示 `hard_timeout_ms=unlimited max_script_bytes=unlimited max_recursive_scripts=unlimited`，最终 JSON 中 `hard_timeout_ms/max_script_bytes/max_recursive_scripts=null`。
  - 本地 1105 chunk synthetic 站点（不传 `hard_timeout_ms/max_script_bytes/max_recursive_scripts`）→ exit 0；`scripts_saved=1106`、`scripts_recursive_downloaded=1105`、`recursive_queue_remaining=0`、`closure_complete=true`、`recursive_limit_hit=false`。
  - 同一 synthetic 站点显式 `--max-recursive-scripts 1000` 对照 → exit 0；`status=closure_partial`、`scripts_saved=1001`、`scripts_recursive_downloaded=1000`、`recursive_queue_remaining=105`、`recursive_limit_hit=true`，证明新默认已越过旧 1000 边界。
  - 外站 smoke（均 `--max-pages 1 --max-actions 0 --ai-assist false`，不传 closure 限制，外层测试 harness 90s 兜底）：`https://vite.dev/` → ok / 10.2s / 20 JS / 3 API / closure_complete=true；`https://react.dev/` → ok / 13.9s / 35 JS / 7 API / 24 recursive / closure_complete=true；`https://docs.astro.build/en/getting-started/` → ok / 7.2s / 3 JS / 0 API / closure_complete=true；`https://angular.dev/overview` → 外层 90s timeout，stderr 仍在抓 recursive chunk（约 113 个），说明 unlimited 在巨大 chunk 图上会持续跑更久。
  - 用户相关站点 smoke（同参数，外层 120s 兜底）：`https://dayu.moresec.cn/` → ok / 80.2s / 12 JS / 1 API (`/api/iam/v2/login/types`) / closure_complete=true；`https://yapi-dayu.moresec.cn:443/` → ok / 6.2s / 2 SSO JS / 0 API / closure_complete=true。
- **未跑 / 风险**：遵用户要求未跑 `./init.sh`，也未跑 full `just precommit`。默认 unlimited 会更完整，但在异常站点上也可能让 browser collector 跑得更久；需要限流时显式传正数。
- **本轮修改但未提交（本 scope）**：`scripts/browser_collect_js_api.mjs`、`scripts/js_api_ai_recipe_probe.mjs`、`scripts/js_api_pipeline_test.mjs`、`backend/crates/golish-pentest-app/src/pentest_bridge/{ai_oneshot.rs,browser_collect_js_api.rs,js_ai_extract.rs,js_extract_apis.rs}`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。

### 2026-07-05 · vuln_triage stage_run 扫描路由与 coverage worklist 修复

- **本轮目标**：回应用户“这个阶段应该是先扫描 / stage_run 怎么会 block / 你要改”，修复 Enumeration 通过后 `vuln_triage` 无法真正进入公式化扫描 worker、preflight 误报可提交的问题。
- **诊断结论**：
  - `vuln_triage` spec 的 `specialist` 是 `vuln_scanner`，但默认 sub-agent 集没有 `sub_agent_vuln_scanner`；`stage_run` 之前直接拼 `sub_agent_{specialist}`，导致该阶段扇不出真实 worker。
  - Orchestrator/planner prompt 的通用规则仍写着安全任务优先直接 `sub_agent_pentester`；在 active specialist stage 里该直接调用会被 tool guard 正确拦住，应该走 `stage_run`。
  - `ai_get_stage_asset_coverage` 的 read-model 没有给 `StageKind::VulnTriage` 生成 10 个公式化扫描 technique cell，导致 worklist/status 可能拿到空矩阵并返回 `ready_to_submit=true`，而 gate 仍按 spec 看见大量 `(asset × technique)` 缺口。
- **已完成**：
  - `stage_run_call.rs` 新增 `vuln_scanner -> sub_agent_pentester` runtime 映射：保留 `vuln_scanner` 的 stage/UI label、agent_path/checkpoint key，但用现有 Pentester worker 工具面执行扫描。
  - `stage_coverage.rs` 给 `vuln_triage` 生成 10 个公式化扫描轴（WSTG-INPV-05/01/12、ATHZ-04、ATHN-02、SESS-02、CONF-05、CRYP-03、INFO、GOLISH-NDAY），pending/error cell 带 capability/tools hint。
  - `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage` 对 `vuln_triage` 空 denominator 返回 `ready_to_submit=false` + `coverage_denominator_missing=true`，不再把空矩阵当作可提交。
  - `vuln.run_formulaic_sweep` capability 与 `vuln_triage/spec.json` 允许 `sqlmap` 所需的 `web/injection`，保持 methodology / deterministic outcome hook / stage whitelist 一致。
  - Orchestrator/planner prompt 增加 active stage-specialist override：处在 harness stage 时，主 agent 不直接调用 `sub_agent_pentester` 补洞，而是调用 `stage_run`。
  - 同步模块卡：`golish-agent-runtime/agentic_loop.md`、`golish-sub-agents/defaults.md`、`golish-agent-kit/{tool_executors,harness}.md`。
- **运行过的验证（实跑）**：
  - `rustfmt --edition 2021 <本轮触及 Rust 文件>` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_label_and_role_label_title_case --status-level fail` → 1 passed / 291 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app vuln_triage_exposes_formulaic_scan_axes --status-level fail` → 1 passed / 157 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit vuln --status-level fail` → 21 passed / 809 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents stage_run_override --status-level fail` → 1 passed / 119 skipped。
  - `python3 -m json.tool resources/harness/stages/vuln_triage/spec.json >/dev/null` → exit 0。
  - `git diff --check -- <本轮触及路径>` → exit 0。
- **未跑 / 风险**：
  - 按用户要求未再跑 `./init.sh` / full `just precommit`。本轮最初的 `./init.sh` 已在用户叫停前结束，失败点是既有 `golish-recon-app/src/asset_intel/agent_intel.rs:126` clippy `map_or` 可简化，不属于本次 vuln stage 修复证据。
  - 未重启 dev app、未对 Test1 做 live rerun；当前修复是 contract/read-model/routing 层，需重启后端后重跑 `vuln_triage` 才能看到 worker 真正执行 nuclei/sqlmap/wpscan/searchsploit 等公式化扫描。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-kit/src/{tool_executors/security.rs,harness/stage_capability.rs,harness/stage_spec.rs}`、`backend/crates/golish-sub-agents/src/defaults/{prompts/orchestration.rs,prompts/execution_planning.rs,tests.rs}`、`resources/harness/stages/vuln_triage/spec.json`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`docs/modules/backend/golish-agent-kit/{tool_executors.md,harness.md}`、`agent-progress.md`。

### 2026-07-05 · red_team Enumeration 后误进 Reporting 的 DAG 修复

- **本轮目标**：回应用户“我用的 red team，不是全阶段吗？为什么刚刚进 Reporting”，用最新 Test1 run + DB truth 判断是 profile 问题还是 stage graph 问题，并修复。
- **诊断结论**：
  - 最新 Test1 session `pentest-chat-1783230370145-1` 的 `operation_state.profile` 实际是 `red_team`，不是 assessment；`red_team.json` 也确实允许全阶段。
  - DB `operation_state.state_blob.graph_flow` 显示 `enumeration` 已 PASS 且 `made_progress=true`，但 `next_node=reporting`。
  - 根因是 `resources/harness/graph/operation_graph.json` 中 `enumeration` 分支边顺序写成了 `reporting` 在前、`vuln_triage` 在后；graph-flow 规则是 `made_progress=true` 走第一条主路，因此 Red Team/Pentest 在 Enumeration 有内容枚举进展时反而直接进了 Reporting。
- **已完成**：
  - 调整基础 DAG：`enumeration -> vuln_triage` 作为 progress/main branch，`enumeration -> reporting` 作为 no-progress bail branch。
  - 补 `operation_graph` 单测锁住 attack-capable profile 下 `Enumeration` 后继顺序为 `[VulnTriage, Reporting]`。
  - 更新 `operation_flow` mock：Enumeration 的 DB/ledger-backed 内容枚举结果算 progress，progress 应进入 vuln_triage。
  - 更新 `docs/modules/backend/golish-agent-kit/harness.md`，记录 operation graph 分支边顺序是运行时语义。
  - 更新 `feature_list.json` 的 `attack-stage-formulaic-candidate-exploit-2026-07-02` 验证项与 evidence。
- **运行过的验证（实跑）**：
  - `python3 -m json.tool resources/harness/graph/operation_graph.json >/dev/null` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit operation_graph operation_flow --status-level fail` → 40 passed / 788 skipped。
  - `git diff --check -- resources/harness/graph/operation_graph.json backend/crates/golish-agent-kit/src/harness/operation_graph.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs docs/modules/backend/golish-agent-kit/harness.md` → exit 0。
- **未跑 / 风险**：未跑 full `just precommit`；未重启 app 重新跑 live red_team。已存在的 Test1 operation checkpoint 仍停在 `reporting`，这次代码修复不会自动改写已持久化的 `next_node`；要从当前 run 继续进 `vuln_triage` 需要显式 reset/checkpoint 迁移或开新 run。
- **本轮修改但未提交（本 scope）**：`resources/harness/graph/operation_graph.json`、`backend/crates/golish-agent-kit/src/harness/{operation_graph.rs,operation_flow.rs}`、`docs/modules/backend/golish-agent-kit/harness.md`、`feature_list.json`、`agent-progress.md`。

### 2026-07-05 · target_intel recon_map_assets 自动 apex 扩展

- **本轮目标**：回应用户“资产为什么少了 / 再改”，把 `recon_map_assets(domain=...)` 从模型可选补跑改成普通 `recon_map_assets(organization_id=...)` 内置的确定性 bounded owned-apex expansion，减少 DeepSeek Flash 因未主动补 domain survey 导致的资产数波动。
- **已完成**：
  - `asset_intel::run_passive_intel` 拆成外层编排 + 单次 provider run；普通 enrich/org-company survey 完成后，从新写入的 `organizations.domains` / `intel.app_domains` 提取注册 apex，去除 IP/噪声/公共非资产域，最多自动补 5 个 domain-keyed provider runs。显式 `config.domain` 仍只跑定点 domain-keyed templates，且不会递归扩展。
  - `PassiveIntelSummary` 新增 `domainExpansions[]`，每个 expansion 带 `domain/run_id/status/targets/providers/providerStatus`。
  - runtime `recon_source_query_rows` 解析 nested `domainExpansions[*].providerStatus`，写成 `source_query_log(query=map_assets,target=<apex>)`，避免覆盖主 org survey 的 provider row；duplicate guard 对 `recon_map_assets(domain=...)` 改为只按对应 target 去重。
  - 更新 `recon_map_assets` 工具描述/schema、target_intel methodology、prompt/refiner 简短提示，以及 `golish-recon-app` 的 `asset_intel` / `agent_tools` 模块卡。
- **运行过的验证（实跑）**：
  - `cargo fmt --manifest-path backend/Cargo.toml --package golish-recon-app --package golish-agent-runtime --package golish-agent-kit` → exit 0。
  - `cargo test --manifest-path backend/Cargo.toml -p golish-recon-app domain_expansion_roots_extracts_apexes_and_skips_noise` → 1 passed / 214 filtered。
  - `cargo test --manifest-path backend/Cargo.toml -p golish-recon-app map_assets_schema_has_optional_domain` → 1 passed / 214 filtered。
  - `cargo test --manifest-path backend/Cargo.toml -p golish-agent-runtime domain_expansion_provider_status_rows_keep_domain_target` → 1 passed / 291 filtered；该次也重新编译了 `golish-agent-kit`。
- **未跑 / 风险**：未跑 full `just precommit`；未重启 dev app 跑新的 DeepSeek Flash/Test1 live target_intel。当前验证覆盖了 Rust 编译、apex 提取去噪/限流、schema、source_query_log row shape；真实 provider 数量回升需重启后端后再跑一次小企业/MoreSec live 才能确认。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`、`backend/crates/golish-recon-app/src/agent_tools/mod.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/refiner.rs`、`resources/harness/stages/target_intel/methodology.md`、`docs/modules/backend/golish-recon-app/{asset_intel,agent_tools}.md`、`agent-progress.md`。

### 2026-07-05 · enumeration route_probe 前台预算 + batch compaction 优化

- **本轮目标**：继续优化 DeepSeek Flash live smoke 暴露的 enumeration 卡在 `route_probe_paths` 后无清晰收束的问题，并用本地 fixture 复跑验证能否落库 + 过 gate。
- **已完成**：
  - `route_probe_paths` 新增 `max_requests` 与环境默认 `GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS` / `GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS`；命中请求预算时返回 `status="request_limited_partial"`、`request_limited=true`、`queue_completed=false`，并把 `max_requests` / `candidate_generation_limited` 写进结果、audit detail、live complete 与 evidence summary。
  - `scripts/stage_smoke.py` 新增 `--route-probe-max-runtime-ms` / `--route-probe-max-requests` / `--full-route-probe`，enumeration smoke 默认给 `route_probe_paths` 注入前台预算，避免本地验证无限等。
  - Enumerator prompt 与 enumeration methodology 改为批量调用 `route_probe_paths` 时显式带前台预算，并说明 `timeout_partial` / `request_limited_partial` 只代表队列未跑空，不代表已有 DIR 发现无效；提交前应刷新 coverage/worklist。
  - `golish-sub-agents` 的 model-visible route_probe compaction 改成 batch-aware：顶层 batch 不再被压成 `matches_count=0`，而是保留每个 nested target 的 `matches_count`、`outcome_persisted`、`request_limited` 与 `max_requests`。
  - 同步模块卡：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-sub-agents/{defaults,executor}.md`、`docs/modules/backend/golish/stage_run.md`。
- **活体结果（实跑 DeepSeek Flash enumeration）**：
  - 命令：`python3 scripts/stage_smoke.py --provider deepseek --model deepseek-v4-flash --profile assessment --only enumeration --fixture-web --org 'Golish Local Fixture Enum Optimized' --route-probe-max-runtime-ms 20000 --route-probe-max-requests 400 --objective 'Run only the enumeration smoke stage against the local fixture. Use the stage worklist, run real browser/js/route tools, keep route_probe_paths bounded with max_runtime_ms=20000 and max_requests=400, refresh DB-backed coverage after partial route probe, then submit only when ready.' --run-tree` → exit 0。
  - Workspace：`/private/var/folders/3m/qkq1qzkn2lsgffy1nvdc8k880000gn/T/golish-stage-workspace-isdcmck2`；session：`stage-run-bb2e5f56-7510-4f65-8028-932d156f9a10`；fixture：`http://127.0.0.1:49501`。
  - Final report：`GATE PASS findings=0`、`submits=1 needs_fix=0`。DB/tool evidence：`browser_collect_js_api` 写 JSAPI/PARAM/JS evidence ids 3/4/5；`katana` 写 crawler evidence id 7；`js_extract_apis` 写 JSAPI/PARAM evidence ids 11/12；`route_probe_paths` 写 DIR evidence id 14。
  - `route_probe_paths` 实际调用为 batch + `max_runtime_ms=60000` / `max_requests=2000`（prompt 显式预算优先于 smoke objective 文案）；nested target 返回 `status="request_limited_partial"`、`request_limited=true`、`requests_sent=2000`、`queue_remaining=13`，但已验证 `/api`、`outcome="found"`、`outcome_persisted=true`、`persisted_directory_entries=1`，所以 gate 能依据 DB truth 通过。
  - 仍观察到本地 fixture 的 claim 文本含 `127.0.0.1` 时会先触发 SSRF guardrail BLOCK，模型删 raw URL 后可 PASS；这是 smoke/claim polish 问题，不应放宽生产 SSRF 规则。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents --check` → exit 0。
  - `python3 -m py_compile scripts/stage_smoke.py` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app route_probe --status-level fail` → 14 passed / 153 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents defaults response_parsing --status-level fail` → 50 passed / 66 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents route_probe_model_visible response_parsing defaults --status-level fail` → 51 passed / 66 skipped。
  - `cd backend && cargo check -p golish-pentest-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-sub-agents/src/defaults/tests.rs resources/harness/stages/enumeration/methodology.md scripts/stage_smoke.py docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish/stage_run.md` → exit 0。
- **未跑 / 风险**：未跑 full `just precommit`；本次证明本地 fixture enumeration 已能经真实 DeepSeek Flash + 临时 DB + DB evidence 过 gate，但 Test1/真实复杂目标上的非 fixture rerun 还没跑。EAS wrapper runner 仍 pending；本地 fixture 的 localhost claim/SSRF guardrail 需要单独 polish。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`backend/crates/golish-sub-agents/src/{executor/response_parsing.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、`resources/harness/stages/enumeration/methodology.md`、`scripts/stage_smoke.py`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-sub-agents/{defaults,executor}.md`、`docs/modules/backend/golish/stage_run.md`、`feature_list.json`、`agent-progress.md`。

### 2026-07-05 · DeepSeek Flash stage-smoke 活体矩阵

- **本轮目标**：按用户“试一下 deepseek flash / 找一个很小的企业跑一下，看每个阶段是不是改成这个样式”，用真实 `golish --stage-run` + 临时 DB + DeepSeek Flash 跑阶段 smoke。安全边界：不随机挑真实企业做未授权测试；外部目标只用公开授权的 `scanme.nmap.org`，目录/API 枚举改用本地 fixture。
- **代码修复 / 调整**：
  - `golish --stage-run` 入口改为在专用 32MiB 大栈线程里创建 Tokio runtime，解决 DeepSeek Flash 活体工具调用在主线程栈上溢出的问题。复测前失败点：`manage_organizations` / `list_in_scope_targets` 第一次工具调用触发 `thread 'main' has overflowed its stack`。
  - 撤掉临时 `list_in_scope_targets` 绕路，只保留 `execute_security_analysis_tool` / registry future 的 `Box::pin` 降低 future 尺寸；行为仍走统一工具 executor。
  - `scripts/stage_smoke.py` 增加 `--provider` / `--model`，可复用 `--provider deepseek --model deepseek-v4-flash` 跑隔离 DB smoke。
- **活体结果（实跑 DeepSeek Flash）**：
  - `target_intel` on `scanme.nmap.org`（授权公开测试目标）→ **PASS**。工具：`manage_organizations`、`list_in_scope_targets`、`stage_run`、`check_stage_asset_coverage`、`list_recent_evidence`、`submit_stage_deliverable`。DB summary：`targets=1`、`dns_records=2`、`source_query_log=7`、`audit_log=4`、`evidence_audit_log=2`、`tool_calls=7`、`org_stage_completions=1`。
  - `external_attack_surface` on `scanme.nmap.org` → **PASS after one repair**。真实工具/落库：`httpx` 写 `stored_targets=1 stored_endpoints=1 stored_origins=1 stored_fingerprints=3`；`naabu -top-ports 1000` 写 `stored_targets=4 stored_endpoints=4`；`nmap -sV` evidence id 15；`whatweb` 写 `stored_fingerprints=5`。DB summary：`targets=2`、`audit_log=22`、`evidence_audit_log=4`、`technique_outcomes=3`、`tool_calls=10`。第一次 submit 空 coverage 被 `coverage_complete` BLOCK，repair 后提交显式 coverage PASS；这是后续可优化点（EAS DB-derived found 与 submit contract 仍有摩擦）。
  - `scoping` on 本地 fixture → **PASS**。`ask_human(scope_review)` 被 headless auto-approve；第一次 submit 因 claim 文本含 `127.0.0.1` 被 SSRF guardrail 拦截，模型删掉 raw URL 后 PASS。DB summary：`organizations=1`、`targets=1`、`tool_calls=6`、`audit_log=1`。
  - `enumeration` on 本地 fixture → **未完成，人工中断(exit 130)**。已验证部分：主 agent 直接尝试 `run_pty_cmd` / `sub_agent_pentester` 被阶段 tool boundary 拦住；随后转 `stage_run` 扇出 `enumerator`。DB/工具 evidence 已出现：`katana` 三轮分别写 endpoint records `2/2`、`7/7`、`6/6`；`js_collect` 找到 `app.js`；`browser_collect_js_api` 观察到 `GET /api/users?limit=10` 和 `POST /api/orders`；`js_extract_apis` 提取 2 个 API。卡点：`route_probe_paths` 后长时间无日志进展，最终 Ctrl-C；未拿到 final gate/report/db_smoke_summary。
- **运行过的验证（实跑）**:
  - `cd backend && cargo fmt -p golish -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo check -p golish -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish stage_run --status-level fail` → 25 passed / 203 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime single_tool_call tool_execution --status-level fail` → 49 passed / 240 skipped。
  - `python3 -m py_compile scripts/stage_smoke.py` → exit 0。
  - 清理了本轮中断/早前崩溃留下的临时 `golish-stage-run-db-*` embedded PG 进程；默认 app DB 未动。
- **未跑 / 风险**：未跑 full `just precommit`；`enumeration` live smoke 暴露出前台长跑/无进度问题仍需修（尤其 `route_probe_paths` timeout/background handoff + final gate）。EAS 虽 PASS，但仍要把 DB truth 与 submit coverage 合同再收紧，减少空 coverage 的 repair 循环。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish/src/main.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`scripts/stage_smoke.py`、`docs/modules/backend/golish/stage_run.md`、`feature_list.json`、`agent-progress.md`。

### 2026-07-05 · stage-run 临时 DB smoke harness

- **本轮目标**：回应用户“有没有真的跑的测试方式，可以一个阶段一个阶段测试，能不能真的过、能不能落数据库；可新建测试数据库”的要求，在现有真 `golish --stage-run` 上补隔离 DB + DB truth 摘要 + 脚本化入口，不造 mock runner。
- **已完成**：
  - `golish --stage-run` 新增 `--ephemeral-db` / `--keep-ephemeral-db` / `--db-smoke-summary`：测试时使用临时 pgdata + 随机本地端口，不污染默认 app DB；`db_smoke_summary` 在 embedded PG 停止前查询 sessions/tasks/tool_calls/audit_log/organizations/targets/target_assets/api_endpoints/technique_outcomes/source_query_log/org_stage_completions 等关键表，并按 run/project/org 维度输出计数。
  - `app/bootstrap` 增加可注入 `DbConfig` 的 lazy pool / embedded PG owned handle seam；GUI 默认行为不变，普通 `just stage` 仍使用默认 DB。
  - 新增 `scripts/stage_smoke.py`：薄包装真实 `cargo run -p golish -- --stage-run --ephemeral-db --db-smoke-summary`，可创建临时 workspace、可起本地 HTTP fixture 并 seed 为 target、可选 `--run-tree`。
  - 新增 `just stage-smoke <profile> <to-stage> "<objective>"`，默认带本地 HTTP fixture。
  - 同步 `docs/modules/backend/golish/stage_run.md` 与 `docs/modules/INDEX.md`；`feature_list.json` 的 `headless-single-stage-runner-2026-06-06` 追加 P2 smoke harness 证据，状态仍 `in_progress`（未跑真实 LLM 活体）。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish` → exit 0。
  - `python3 -m py_compile scripts/stage_smoke.py` → exit 0。
  - `cd backend && cargo nextest run -p golish stage_run --status-level fail` → 25 passed / 203 skipped。
  - `cd backend && cargo check -p golish` → exit 0。
  - `just --dry-run stage-smoke assessment target_intel "smoke target_intel"` → exit 0，展开为 `python3 scripts/stage_smoke.py --fixture-web --profile assessment --to target_intel --objective "smoke target_intel"`。
  - `cd backend && cargo clippy -p golish --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本轮触及文件>` → exit 0。
  - `jq empty feature_list.json` → 待本轮最后复跑。
- **未跑 / 风险**：
  - 未跑真实 `scripts/stage_smoke.py` 活体阶段，因为会调用真实 LLM provider / 可能调用外部情报源或主机工具，需要用户明确授权目标与 API 成本。
  - 未跑 `just precommit` 全量；全仓 `git diff --check` 目前会被既有 `frontend/lib/generated/Target.ts` trailing whitespace 阻断，本轮 scope 的定向检查已通过。
- **下一步**：
  - 用户授权后可跑：`python3 scripts/stage_smoke.py --fixture-web --profile assessment --to target_intel --objective "smoke target_intel"`，观察终端 report + `db_smoke_summary` + workspace `.golish/transcripts/<session>/`。
  - 如果要做真正回归矩阵，可按阶段依次跑 `--to scoping`、`--to target_intel`、`--to external_attack_surface`、`--to enumeration`，每次都用 `--ephemeral-db --db-smoke-summary` 隔离验证。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish/{Cargo.toml,src/app/bootstrap.rs,src/cli/args.rs,src/stage_run/mod.rs}`、`scripts/stage_smoke.py`、`justfile`、`docs/modules/backend/golish/stage_run.md`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`。

### 2026-07-05 · stage capability tools metadata implementation

- **本轮目标**：按用户“现在帮我实现吧”，先落 capability-first 的 metadata/read-model/repair/UI 合同，不做 EAS runner wrapper 和 live 扫描。
- **已完成**：
  - 新增 `golish-agent-kit/src/harness/stage_capability.rs`：定义 `StageCapabilitySpec` / `StageCapabilitySuggestion`，覆盖 scoping、target_intel、EAS、enumeration、vuln_triage、attack_candidate、verification；target_intel 不暴露 scan CLI，enumeration 不暴露 ffuf/arjun 等外部目录/隐藏参数爆破。
  - `CoverageGapAction` 新增 `suggested_capabilities`，gate recovery、`ai_get_stage_asset_coverage`、`stage_worklist_next/status`、compact coverage、StageRefiner、sub-agent `SubmitRepairMode`、`stage_run` worker objective 全部改为 capability-first，同时保留 legacy `suggested_tools`。
  - 前端 `StageAssetCoveragePanel` 以运行时扩展类型读取 `suggested_capabilities`，只在 cell title/tooltip 展示 `capability: ...`，不改 `frontend/lib/generated` 类型链。
  - 同步模块卡：`golish-agent-kit/harness`、`task_orchestrator`、`golish-agent-app/ai`、`golish-agent-runtime/agentic_loop`、`golish-sub-agents`、`golish-sub-agents/executor`、`frontend/components`。
  - `feature_list.json` 更新 `stage-capability-tools-2026-07-05` 的实现证据并保持 `in_progress`；历史上已有多个旧 `in_progress` 条目，本轮未擅自重写其它 feature 状态。
- **运行过的验证（实跑）**：
  - `cd backend && cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail` → 25 passed / 801 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 40 passed / 117 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_refiner coverage_gap worklist --status-level fail` → 14 passed / 812 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run_call --status-level fail` → 29 passed / 260 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap submit_repair --status-level fail` → 11 passed / 105 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents defaults --status-level fail` → 22 passed / 94 skipped。
  - `pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 23 passed。
  - `pnpm exec tsc --noEmit --pretty false` → exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents --all-targets -- -D warnings` → exit 0；期间修了一个既有等价 lint：`bridge_config.rs` 测试断言 `first().is_some()` → `!is_empty()`。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents` → exit 0。
  - `pnpm exec biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `jq empty feature_list.json` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents --check` → exit 0。
  - `git diff --check -- <本轮 tracked scope>` → exit 0；`git diff --no-index --check /dev/null backend/crates/golish-agent-kit/src/harness/stage_capability.rs` 包装检查 → exit 0。
- **未跑 / 风险**：未跑 `just precommit`；未启动 dev app 做 Test1 live EAS smoke；EAS wrapper runner 尚未实现，所以当前只是“能力建议 + worklist/refiner/UI 合同”，还不是“模型调用一个 capability wrapper 由后端拼命令”的最终形态。
- **下一步**：在 Test1 上重启后端后跑一次 EAS/repair smoke，确认 `stage_worklist_next` 返回 `suggested_capabilities`，worker objective 实际按能力闭环；随后实现最小 EAS wrapper runner（先 LIVE/PORT/SERVICE 三个 wrapper），让模型不再手写 broad `nmap/httpx/naabu` 命令。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,mod.rs,types.rs,gate/rule_engine.rs,org_gate.rs}`、`backend/crates/golish-agent-kit/src/{task_orchestrator/stage_refiner.rs,tool_executors/security.rs}`、`backend/crates/golish-agent-app/src/ai/commands/{stage_coverage.rs,bridge_config.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call.rs,sub_agent_call.rs}`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,lib.rs}`、`frontend/components/Engagement/StageAssetCoveragePanel.{tsx,test.tsx}`、相关模块卡、`feature_list.json`、`agent-progress.md`。

### 2026-07-05 · stage capability tools 设计文档

- **本轮目标**：按用户“能力包装成工具，先写文档”的要求，先设计 capability-first harness contract，不写实现代码。
- **已完成**：
  - 新增 `docs/design/2026-07-05-stage-capability-tools.md`：定义 stage capability registry、`suggested_capabilities`、EAS/Enumeration/Vuln/Attack/Verification 分阶段能力、wrapper runner 方向、安全约束与迁移顺序。
  - 新增 `docs/superpowers/plans/2026-07-05-stage-capability-tools.md`：按 Phase 0-8 拆实现计划，先 metadata-only，再 worklist/refiner/UI，最后 EAS wrapper runner。
  - `feature_list.json` 新增 `stage-capability-tools-2026-07-05` 为 `not_started`，未切 `in_progress`，避免扰动当前已有 in-progress 工作。
- **验证**：文档/feature tracking 变更；未跑代码验证或 `just precommit`。
- **风险 / 下一步**：若开始实现，第一步新增 `golish-agent-kit/src/harness/stage_capability.rs` 纯 registry + 单测；先保留 `suggested_tools` 兼容字段，再引入 `suggested_capabilities`。
- **未提交**：本轮新增 docs/feature/progress 文件未提交；工作树还有大量既有未提交改动，未触碰 unrelated code。

### 2026-07-04 · enumeration DIR/PARAM terminal outcome fix

- **本轮目标**：接用户“最后一次日志，枚举阶段总是过不了”的诊断，修复两个会让 enumeration worklist 长期降不完的运行阻塞：`route_probe_paths` 坏证书/长批次/DIR timeout 误判，以及 PARAM 跑过但无参不落 `checked_empty`。
- **根因确认**：
  - 最新 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1783070503216-1`；`scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1783070503216-1 --full --db` 显示 EAS PASS，但 enumeration worklist 为 93 web roots × 4 axes = 372 cells，最后状态 `done=80,error=16,pending=276,total=372,ready=false`。
  - enumerator transcript 最后卡在一个 18 target `route_probe_paths` batch；`run.log` 尾部持续刷 `invalid peer certificate ... "api-sentry.moresec.cn" certificate is using a broken key size`。
  - DB truth 显示 `GOLISH-ENUM-DIR` 有 18 个 `error`，`GOLISH-ENUM-PARAM` 只有 4 个 `found`、没有 clean no-param 的 `empty` outcome，导致大量已执行过的根仍被 gate 当作 pending。
- **已完成**：
  - `route_probe_paths`：reqwest client 接受无效/弱证书；batch `max_runtime_ms` 改为整批共享总预算，预算耗尽返回 `timeout_partial` 并把未启动目标放入 `skipped`；DIR outcome 改为有 match→`found`、候选请求全传输失败→`error`、timeout/无 match 但已有成功采样→`empty`。
  - `js_extract_apis`：完成一次 JS extraction 后总是写 PARAM outcome（`found`/`empty`/`error`），不再只有传 `param_hints` 才能落 `GOLISH-ENUM-PARAM`；`param_hints` 只作为 body/form 参数增强。
  - `browser_collect_js_api`：browser-observed 带参 API 会写 PARAM `found`；跑完无带参 API 写 PARAM `empty`；响应和 audit detail 新增 `param_outcome` / `param_outcome_persisted` / `param_bearing_api_requests`。
  - 已同步 `docs/modules/backend/golish-pentest-app/pentest_bridge.md` 的 bridge 工具合同。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app route_probe_outcome --status-level fail` → 3 passed / 161 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app param_outcome --status-level fail` → 7 passed / 157 skipped。
  - `cd backend && cargo fmt -p golish-pentest-app --check` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
- **未跑 / 风险**：未跑 `just precommit`；未重启 dev app 做 live enumeration rerun。`route_probe_paths` 的 batch 仍是逐 target 调 `execute_single`，本轮只保证共享总预算/不无限占 foreground，尚未做真正后台 continuation。
- **下一步**：重启后端后继续/重跑 Test1 enumeration，重点看 `stage_worklist_status` 的 `pending` 是否下降，`GOLISH-ENUM-PARAM` 是否开始出现 no-param web root 的 `empty` outcome，DIR 的 broken-key 站点是否不再批量落 `error`。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{route_probe_paths.rs,js_extract_apis.rs,browser_collect_js_api.rs}`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。

### 2026-07-04 · enumeration crawler 外链不再自动 promotion 为 target

- **本轮目标**：回应用户对“未解析域名/第三方外链为什么进入目标树”的确认，修复 `katana -list ... -jc -silent` / crawler endpoint 输出把第三方绝对 URL host 自动建成当前 org `active_discovered` target 的边界问题。
- **根因确认**：`endpoint_add` 落库旧逻辑在缺少 `__command_base_host` 时 `unwrap_or(true)` 放行；`katana -list` 批量命令没有 `-u/--url`，因此没有 base host，导致 `github.com`、`lodash.com`、`momentjs.com` 等第三方 URL 被 `find_or_create_target_scoped` 创建成当前 org target，前端因 `real_ip=''` 显示在“未解析域名”。
- **已完成**：
  - `golish-pentest/src/output_store/mod.rs`：为 `endpoint_add` 从命令提取 canonical origins；支持单 `-u/--url` 和 `-list/--list/-l` roots 文件，写入内部 `__command_base_urls`，并保留单 origin 的 `__command_base_host` 兼容字段。
  - `golish-pentest/src/output_store/endpoints.rs`：scoped stage 下不再盲目创建 endpoint host target；有 command origins 时只允许 exact origin 命中；没有 origins 时只承接当前 org 已存在 target。第三方 crawler URL 会跳过，不会 promotion 为 `targets(source=active_discovered, scope=in)`。
  - 同步 enumerator prompt、enumeration methodology、`golish-pentest/output_store` 与 `golish-sub-agents/defaults` 模块卡，明确 katana 只补 same-origin/current-org target 的 crawler endpoints，第三方外链是 crawler context 不是新 target。
- **运行过的验证（实跑）**：
  - `cd backend && cargo check -p golish-pentest -p golish-sub-agents` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest endpoint_ --status-level fail` → 10 passed / 155 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents defaults --status-level fail` → 22 passed / 94 skipped。
  - `cd backend && cargo clippy -p golish-pentest -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest output_store --status-level fail` → 31 passed / 134 skipped。
  - `git diff --check -- <本轮触及文件>` → exit 0。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit` / 活体 enumeration rerun；没有清理当前 Test1 DB 里已污染的第三方 target（这是 DB mutation，需用户明确确认后再做）。
- **风险 / 下一步**：新 run/retry 需要重启后端后验证：`katana -list` 输出中的第三方外链不再新增 target，且 same-origin endpoints 仍进入对应已存在 web root 的 API/Param/JSAPI DB truth。若要在 UI 中保留第三方外链可见性，后续应加“外部引用/爬取上下文”模型或字段，而不是塞进 in-scope target。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest/src/output_store/{mod.rs,endpoints.rs}`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`agent-progress.md`。

### 2026-07-04 · 枚举吞吐计划/commit 审阅 + 未提交 PR-A/C 收口

- **本轮目标**：审阅用户贴出的另一 session 结果（commit `6cfdeaa2` + `ac6f73b7` + `docs/superpowers/plans/2026-07-03-enumeration-throughput-optimization.md`），核对当前工作树真实状态；不跑 precommit、不 push。
- **核对结论**：
  - `ac6f73b7` 确为 HEAD；但 HEAD 后工作树已有未提交 PR-A/C 改动（coverage snapshot 携带 web 元数据、`list_enumeration_web_roots` 生成完整 `root_url`、alive-first worklist 排序、enumerator prompt/methodology 更新）。
  - 当前计划已勘误：`browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` 不走 sub-agent 外层 timeout，PR-B 不应直接后台化；吞吐主线应先看 PR-A + batch-katana + PR-C。
- **本轮小修**：
  - `backend/crates/golish-agent-kit/src/tool_executors/security.rs`：补测试模块 import，使新增 `web_root_url_from_meta` 测试能编译。
  - `backend/crates/golish-sub-agents/src/defaults/tests.rs`：把 enumerator prompt 断言从旧文案改为新合同（batch `target_urls`、完整 `root_url`、禁止逐个 `query_target_data` 拼 URL）。
  - `backend/crates/golish-app-core/src/domain/targets.rs`：新增共享 `web_root_url` helper + 单测，并保留 `rank_enumeration_web_roots` 为 alive-first 排序/cap 预备；不折叠同 IP vhost。
  - `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`：删除私有 `root_url_for`，改用 `app-core::domain::targets::web_root_url`。
  - `docs/superpowers/plans/2026-07-03-enumeration-throughput-optimization.md` + `docs/modules/backend/golish-app-core/domain.md`：对齐依赖边界和真实 PR-C 语义（agent-kit 不依赖 app-core，只本地镜像 adapter；gate 分母 cap/折叠延后）。
- **运行过的验证（实跑）**：
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app -p golish-app-core -p golish-sub-agents` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit enumeration_web_roots --status-level fail` → 1 passed。
  - `cd backend && cargo nextest run -p golish-app-core web_root_url rank_enumeration_web_roots --status-level fail` → 2 passed。
  - `cd backend && cargo nextest run -p golish-agent-app web_roots stage_coverage --status-level fail` → 40 passed。
  - `cd backend && cargo nextest run -p golish-sub-agents enumerator --status-level fail` → 2 passed。
  - `git diff --check` → exit 0。
- **未跑 / 风险**：未跑 `just precommit`；未做 live enumeration rerun；`feature_list.json` 仍有大量历史 `in_progress`，本轮未改状态避免扩大范围。当前 PR-C 只做 worklist 排序和 cap helper，不改 gate 分母；同 IP vhost 折叠/真实 denominator cap 需另配 wave/backlog 设计与 gate parity 测试。

### 2026-07-03 · EAS 阶段报错根治 P0 + 死资产 liveness P1/P2/P3/P4 全链（专注执行，未跑 commit/precommit）

- **本轮目标**：接 MCP-5 转交的上下文，按 `docs/superpowers/plans/2026-07-03-eas-stage-optimization.md` 把 EAS 阶段「一直报错」根治（P0）+ 落死资产标记 Phase 1 inert 部分（P1）。**用户明确指令高于 rule：只落地、不跑 commit、不跑任何耗时验证（precommit/nextest 一律不跑），没修完也不主动发消息，靠 plan 文档做断点续传。**
- **已完成（本会话验证 = 只跑 ReadLints，均无 lint）**：
  - **P0-A（已是既有状态，本轮核实）**：`golish-sub-agents/src/defaults/builder/registry.rs` 的 recon/prober/enumerator 三个子 agent 已直接用 `build_recon/prober/enumerator_prompt()`（`:138/:164/:193`），不再走 `tmpl_or_fallback!` render 缺失模板 → 消除 `Template 'recon'/'prober'/'enumerator' not found` 刷屏。
  - **P0-B（已是既有状态，本轮核实）**：`migrations/20260703000001_extend_agent_type_stage_agents.sql` 已存在，`ALTER TYPE agent_type ADD VALUE IF NOT EXISTS` 补 recon/prober/enumerator/browser/refiner/orchestrator → 消除 `invalid input value for enum agent_type`。
  - **P1-Task1.1**：新建 `migrations/20260703000002_targets_liveness_state.sql`——加 `liveness_state`/`liveness_reason` 两 nullable 列 + CHECK(alive|dead|unreachable|NULL) + 一次性回填（只对 `liveness_checked_at` 非空行，判据同 `coverage_truth::build_liveness_values_sql`）+ 部分索引。I10 expand-first、additive、可 replay/回滚。
  - **P1-Task1.2**：`golish-db/src/models/pentest.rs` `Target` 加两列；`repo/targets.rs` `TARGET_ROW_COLS`（const `:98` + 两处测试常量）三处同步补列。
  - **P1-Task1.3**：`golish-app-core/src/domain/targets.rs` `Target`（ts-rs 源）加 `liveness_state`/`liveness_reason`（`#[ts(optional)]`）+ 纯函数 `compute_liveness_state` + 4 个单测。
  - **连带对齐（plan 未逐条列，但为编译/数据贯通必须）**：两个 `TargetRow` 适配器（`golish-app-core/src/ports/recon/targets.rs`、`golish-recon-app/src/targets/types.rs`）struct + `From` impl 补两列；`golish-recon-app/src/targets/cmds.rs` 4 处显式 SQL 投影补列；3 个测试 fixture（`golish-agent-app/.../db_bridge/recon.rs`、`golish-recon-app/.../organization_recon/active.rs`、`golish-pentest-app/.../target_surface_hierarchy.rs`）补两字段。
- **续（用户「没搞完继续搞」→ 继续落 P2 + P3，仅跑 ReadLints，全无 lint）**：
  - **P2 死资产写点蓋 alive**：`golish-db/repo/targets.rs` 把 `update_recon_extended_by_id` 抽出 `build_update_recon_extended_sql` + `eas_hit_alive_predicate_sql`，hit-landing 蓋 `liveness_state='alive'`（real_ip/http_status/开放埠任一），ELSE 保留原值（**绝不**在 per-hit 落库标 dead）；`set_real_ip_by_id` 蓋 alive；`golish-pentest/output_store/targets.rs` AI-tool recon 落库同款蓋 alive。+ 2 SQL 单测。
  - **P3 下游 gate 分母剔 dead（gray-switch）**：`coverage_truth.rs` 新增 `dead_asset_values`（只剔 `liveness_state='dead'`，不剔 unreachable）+ SQL 单测；`db_traits/repo.rs` trait `dead_asset_values`（默认空）+ `db_bridge/{mod,recon}.rs` impl；`stage_spec.rs` 加 `skip_dead_assets` flag + `finding_verification_check.rs` 字面补；`enumeration`/`vuln_triage` spec.json 开、**EAS 不开** + spec 单测；`org_gate.rs`（权威 per-org gate）+ `execute.rs`（subtask gate `exclude_dead_assets_if_opted_in`）两处分母剔 dead（**guarded 不清空非空轴**）；seed JSON（`in_scope_targets_impl`/`attack_surface_seeds_impl`）带 `liveness_state`。
  - 按 `docs/design/2026-07-02-recon-gaps-followups.md §4`：本次只加独立 bool flag + 新增 `dead_asset_values` 查询、**不动** wave next-dispatch（问题二 B）与 crediting 判据（问题三），属 §4 判定的低冲突 additive 部分。
- **未跑的验证（用户指令，留给后续 session / 用户择机）**：`cargo check -p golish-db -p golish-app-core -p golish-agent-app -p golish-agent-kit -p golish-pentest`、`cargo nextest -p golish-app-core compute_liveness_state`、`cargo nextest -p golish-db targets coverage_truth`、`cargo nextest -p golish-agent-kit stage_spec`、`just precommit`。**ts-rs 未重生成** → `frontend/lib/generated/Target.ts` 仍缺 `liveness_state?`/`liveness_reason?`（跑 `cargo test -p golish-app-core` 或 `just check` 会自动补；inert，P4 之前前端不消费，不影响编译）。
- **续 2（用户再「继续」→ 补 ongoing dead 标记，仅 ReadLints，全无 lint）**：
  - `golish-db/repo/targets.rs`：`mark_dead_if_no_signal_by_id`（+SQL 测）——**guarded** UPDATE：只在 row 仍无 alive 信号（http_status NULL + real_ip 空 + 无开放埠）且非 'alive' 时蓋 `liveness_state='dead'`，幂等、与 P2 alive 蓋值/naabu 落埠**顺序无关**（有埠者恒 alive）。
  - `golish-agent-app/ai/commands/bridge_config.rs`：EAS 批量 liveness outcome 落库（httpx）判 `!found` 时经新 helper `mark_eas_liveness_dead_asset`（复用 `load_eas_landing_targets_for_asset`+`prefer_exact_landing_targets`）标 dead。**至此 dead 对新 run 也生效**，不再只靠 P1 backfill——用户「死域名不再灌分母」的核心诉求闭环。
  - 侷限：批量 liveness 只分 found/empty，DNS-fail/WAF-block 一律标 'dead'（非 'unreachable'）；guard 保证有埠/后续命中翻回 alive（self-correcting）。
- **续 3（用户「狠下 一口气全部搞完」→ 补 P4 前端 + unreachable primitive，仅 ReadLints，全无 lint）**：
  - **P4 前端徽章**：`frontend/lib/pentest/types.ts` 视图模型 `Target` 加 FE-only `liveness_state?`/`liveness_reason?`（ts-rs 重生成前过渡，**不改** generated `Target.ts`，I5）；新 `LivenessBadge.tsx`（alive 绿/dead 红/unreachable 黄/未探不渲染）；`TargetTreeRow` 名称旁挂徽章；`TargetDetail` Recon Facts 加 Liveness 行（带 reason）。过渡期安全：backend 重建前字段 undefined → 徽章不渲染、不影响编译。
  - **unreachable primitive**：`golish-db` 加 `mark_unreachable_if_no_signal_by_id`（与 dead 共用 `NO_ALIVE_SIGNAL_GUARD_SQL` builder + SQL 测），但**刻意不从批量 httpx 路径接线**——批量无 per-asset error 信号，且 P3 不剔 unreachable → 标 unreachable 会把不可达资产留在分母（与「死域名不灌分母」目标相悖）；批量一律标 'dead'（denominator-correct），unreachable 留给未来有真 probe-error 信号的 per-asset 探测路径。
- **本任务 P0/P1/P2/P3/P4 全部落地闭环。下一步（后续 session · 均非阻塞）**：① 跑 `just precommit` 收口 + ts-rs 自动重生成（`Target.ts` 补 optional 字段）；② unreachable per-asset 接线（待 probe-error 信号路径）；③ global delta runner（原 §3 P2，与本任务无关）。**未 commit、未 push。**

### 2026-07-03 · route_probe_paths 前端 live output 降噪

- **本轮目标**：回应用户反馈 `Using Route Probe Paths` 工具前端输出过多导致卡顿；只做稳定化/可视化降噪，不改扫描语义、不改 DB/schema。
- **已完成**：
  - 前端 store 新增 `slices/live-output.ts::appendLiveToolOutput`，统一给 main active tool、timeline `ai_tool_execution`、sub-agent tool 的 `streamingOutput` 保留 64KB live tail，避免高频 `tool_output_chunk` 把 React state 膨胀到几十万字符；完整结果仍从最终 result / transcript / run.log 追溯。
  - `route_probe_paths` live chunk 降频：progress 从每 100 requests 改为请求数/时间双阈值（1000 requests 或 15s），逐 prefix `recurse` 改为每 25 次递归扩展的 `recursion_progress` 摘要；complete/audit/result 增加 `recursive_expansions` 计数。
  - 同步模块卡：`docs/modules/frontend/store.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
- **运行过的验证（实跑）**：
  - `pnpm exec vitest run frontend/store/slices/live-output.test.ts` → 3 passed / exit 0。
  - `pnpm exec biome check frontend/store/slices/live-output.ts frontend/store/slices/live-output.test.ts frontend/store/slices/session-streaming.ts frontend/store/slices/workflow/sub-agent.ts frontend/store/slices/ai.ts` → exit 0。
  - `pnpm exec tsc --noEmit --pretty false` → exit 0。
  - `cd backend && cargo fmt -p golish-pentest-app --check` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app route_probe --status-level fail` → 9 passed / 148 skipped / exit 0。
- **未跑**：未跑 `just precommit`；当前工作树已有大量与本轮无关的未提交 in-progress 改动，本轮只做 route_probe/live-output 窄修复。
- **本轮修改但未提交（本 scope）**：`frontend/store/slices/live-output.ts`、`frontend/store/slices/live-output.test.ts`、`frontend/store/slices/session-streaming.ts`、`frontend/store/slices/workflow/sub-agent.ts`、`frontend/store/slices/ai.ts`、`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`docs/modules/frontend/store.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。

### 2026-07-03 · submit/gate 合同改为模型不写 evidence ids

- **本轮目标**：按用户拍板，把每个阶段的 `submit_stage_deliverable` / gate 合同改成“不要求模型填写 evidence ids”；ledger id 保留为内部证据/调试引用，若模型显式提交假 id 仍必须拦截。
- **已完成**：
  - `golish-agent-kit` gate 层移除/兼容旧的 id-presence 规则：`scope_check`、`freshness_check`、`finding_verification_check`、`contract_check`、`rule_engine` 不再因为 claim/finding/coverage 缺 `evidence_ids/evidence_refs` 而 BLOCK；`candidate_grounded` 保留 rationale 要求，`candidate_disposition_complete` 保留 terminal disposition 要求。
  - `golish-agent-app` submit tool schema/prompt/repair 文案改为 evidence ids optional；`fabricated_refs` 会收集顶层、claim、finding、coverage 中所有显式 id，只要不存在就 `needs_fix`，但结构性 block 不再诱导模型去 copy 真实 id。
  - `task_orchestrator` stage charter/refiner/execute repair 改为业务事实 + coverage/DB truth 导向；submit-only repair 不再要求“引用这些真实 ids”，只要求提交 claims/coverage 事实。
  - `resources/harness/stages/*` spec/methodology 中的旧“必须引用 evidence id”措辞改为 DB/ledger truth 裁决；no-tool stage 仍拒绝 invented coverage matrix（让 `scoping` 等提交 `coverage: []`）。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/{harness,task_orchestrator}.md`、`docs/modules/backend/golish-agent-app/ai.md`；`feature_list.json` 相关 EAS/evidence 条目仍保持 `in_progress`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit scope_check freshness_check finding_verification_check rule_engine refiner fabricated_evidence --status-level fail` → 149 passed / 668 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app harness_submit_tool --status-level fail` → 36 passed / 120 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit gate --status-level fail` → 226 passed / 591 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit harness --status-level fail` → 536 passed / 281 skipped。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app` → exit 0。
- **未跑 / 未完成**：遵用户“别跑 init.sh / 卡死”要求，本轮没有跑 `./init.sh`、`just precommit`、`just check`，也没有跑活体 EAS/enumeration rerun。
- **风险 / 下一步**：需要重启后端后跑一条新的 stage_run / enumeration 或 EAS，确认 submit/gate 不再要求模型写 ids，且工具落库的 DB/ledger truth 仍能让 coverage/gate PASS；若模型仍手写假 id，应看到 `needs_fix` 要求删掉该 id 字段。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-kit/src/harness/{types.rs,gate/*.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/{prompts/mod.rs,refiner.rs,subtask_phases/execute.rs}`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`resources/harness/stages/**/{spec.json,methodology.md}`、相关模块卡、`feature_list.json`、`agent-progress.md`。

### 2026-07-03 · enumeration JS evidence ledger 补账与 bridge 职责收口

- **本轮目标**：接用户贴出的另一 session 体检结论，修复 enumeration 阶段 JS/API/DIR/PARAM 工具结果没有可引用 evidence ledger 行，导致 gate 只能提示旧 EAS evidence id 并 retry BLOCK 的问题。
- **根因确认**：`technique_outcomes` 与内容表已有 ENUM-JSAPI/DIR/PARAM/JS 事实，但 gate/list_recent_evidence 只认 `audit_role='evidence' AND session_id=<run>` 的 ledger 行；普通 `PentestAudit` action timeline 行不能作为 StageDeliverable evidence id。当前 worktree 已有 pentest_bridge 层改动为 JSAPI/PARAM/DIR 写真实 bridge evidence，本轮 runtime 层只补 `browser_collect_js_api` 的 `GOLISH-ENUM-JS` evidence，避免重复 book 同一 technique。
- **已完成**：
  - `golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs` 在 active enumeration stage 的成功 `browser_collect_js_api` 结果中追加 `GOLISH-ENUM-JS` ledger evidence，asset 优先用当前 org 的 `target_id -> targets.value`，并把真实 ledger id 回填到 `technique_outcomes.evidence_ids`，同时在 tool result 上暴露 `_evidence_id` / `_evidence_ids` 供 agent 引用。
  - 保持 JSAPI/PARAM/DIR evidence 由 `golish-pentest-app/src/pentest_bridge` 的 `append_bridge_evidence` 路径负责；runtime 不再为 `js_extract_apis` / `route_probe_paths` 重复生成 evidence。
  - 修复 `golish-agent-kit/src/harness/operation_flow.rs` 中 `clippy::redundant_comparisons`（`DEFAULT_MAX_WAVES` 与 `DEFAULT_MAX_CHAIN_DEPTH` 同值导致 `&&` 左侧无效），让后续 targeted clippy 不被 unrelated lint 卡住。
  - 同步 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`，记录 runtime/bridge 的 evidence 分工。
- **运行过的验证（实跑）**：
  - `./init.sh`（本轮开工流程）→ 失败：前端检查此前通过，Rust clippy 卡在 `operation_flow.rs:383 clippy::redundant_comparisons`；本轮已修该点。
  - `cd backend && cargo fmt -p golish-agent-runtime -p golish-agent-kit -p golish-pentest-app -p golish-db` → exit 0。
  - `cd backend && cargo check -p golish-agent-runtime -p golish-agent-kit -p golish-pentest-app -p golish-db` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime enumeration_ --status-level fail` → 4 passed / 290 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit operation_flow chain_wave --status-level fail` → 23 passed / 794 skipped。
  - 复跑 `./init.sh` 获取完整证据时被用户要求停止；已在 `check-fe` 阶段 SIGINT，中断退出码 130，不计为通过证据。
- **未跑 / 未完成**：未跑 `just precommit`，未跑活体 enumeration rerun；速度问题（`stage_run` serial concurrency=1、browser 逐 target、route_probe timeout 快失败、批处理 worklist）本轮未改，留后续 slice。
- **风险 / 下一步**：需要重启后端/应用后对新 session 续跑或重跑 enumeration，确认 `audit_log.audit_role='evidence'` 中出现 `GOLISH-ENUM-JS/JSAPI/DIR/PARAM` 且 `list_recent_evidence` 能返回本 run id；再看 gate 是否不再要求引用旧 EAS id。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`backend/crates/golish-agent-kit/src/harness/operation_flow.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`agent-progress.md`、`feature_list.json`。

### 2026-07-03 · EAS prober B/C1/D 接力实现（按用户要求不跑测试）

- **本轮身份**：Codex，接手用户贴出的另一 agent 未完成尾段。用户明确要求：先 commit 当前东西，然后继续写 B/C1/D，**不要跑测试 / 不要跑 precommit**。
- **checkpoint**：先将上一位 agent 已写的 A/C2/E + 设计/计划 + 其它当前工作树内容做保护性 checkpoint commit：`e3890d04 checkpoint: save working tree before EAS follow-up`（使用 `git commit --no-verify`，未跑测试）。
- **已完成 B（异步落库屏障）**：`golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs` 在 `ctx.harness_stage.is_some()` 时 await structured-storage `PostShellHook`，让 harness stage 内的 scan output-store 落库（targets.ports/fingerprints 等）在工具结果返回前完成；非 harness path 保留原 `tokio::spawn`。
- **已完成 C1（SERVICE truth 收紧）**：`golish-db/src/repo/coverage_truth.rs` 的 SERVICE-FINGERPRINT truth 排除 `tcpwrapped/unknown/open/filtered/closed` 等伪服务；bare `service=domain` + `port=53` 不再单独算 SERVICE found，必须有 version/webserver/technologies 或 fingerprint 行才算强服务面。
- **已完成 D（DNS/53-only not_applicable 同源）**：
  - `coverage_truth::eas_service_not_applicable_assets` 以 DB truth 返回当前 wave 中只开 DNS/53 且无强服务面的 IP/CIDR 资产。
  - `stage_coverage.rs` 将该集合用于 `ai_get_stage_asset_coverage` / `check_stage_asset_coverage`，把 SERVICE-FINGERPRINT pending 派生为 `not_applicable` 并写 DNS/53 note。
  - `GateContext.not_applicable_coverage` + `GateContextBuilder` + `rule_engine::coverage_complete` 让 submit preview / per-org close gate 消费同一 `(asset, technique)` 终态集合。
  - `harness_submit_tool.rs` 与 `org_gate.rs` 均从 app/db trait 查询同一个 not_applicable 集合，避免前台 preflight 和权威 gate 分叉。
- **同步文档/状态**：更新 `docs/design/2026-07-02-eas-worker-evidence-and-service-fingerprint.md`、同名 plan、`feature_list.json`、相关模块卡，明确 A/B/C/D/E 代码已写但 feature 仍 `in_progress`，因为验证尚未允许执行。
- **验证状态**：遵用户要求，本轮**未跑** `just precommit`、`just check`、cargo build/check/nextest/clippy，也未跑 `git diff --check`。只做了只读 `rg`/`sed`/`git diff --stat` 级别核对。编译/测试/活体 EAS smoke 仍是下一步验证债。
- **风险**：B/C1/D 改的是 I7/I8 gate/DB truth 口径，必须在后续允许时跑目标 Rust 验证和活体 EAS smoke；当前不能标 `passing`。

### 2026-07-02 · EAS prober 重试死循环根因修复（A/C2/E 落地，B/C1/D staged）

- **本轮身份**：BajieAsk `agent-1-pppoysuf`（主控中心），分发开关 OFF → 用户选自执行；接手 MCP-3 转移的上下文（对最后一次 EAS run 的根因分析）。
- **背景**：最后一次 run（`~/golish-platform/Test1`，session `pentest-chat-1783002737901-1`，2026-07-02 22:35）EAS prober 重试 3/3 耗尽未过。用 `run_tree.py --db` 定位到 3 个叠加缺陷。
- **根因（已核对真实代码落点）**：
  - **A（真正 dealbreaker）**：EAS spec `every claim must cite evidence`（`external_attack_surface/spec.json:30`）要求每条 claim 带非空 `evidence_ids`，但 worker **没有工具能查本 run 真实 evidence id + 上下文**——`recent_evidence_ids`（`repo/audit/mod.rs:250`）只返裸 id 且仅 submit 工具内部用。
  - **B**：预检 vs 权威 gate 的**时序**竞态——扫描落库经 `tokio::spawn` fire-and-forget hook（`direct/mod.rs:500`），gate 可在 `-sV` 落库前评分。
  - **C**：`tcpwrapped` 满足 SERVICE-FINGERPRINT（`coverage_truth.rs:171` 计任意非空 `p->>'service'`），而真实 nmap `-sV` service/version **从不写入 `fingerprints`**（`output_store/targets.rs:264` 只读 webserver/technologies/cdn/os）。
  - **D**：只开 53 埠的 subdomain `real_ip`（共享 DNS/CDN 基絎）被强制过 SERVICE-FINGERPRINT。
  - **E**：`MAX_REFLECTOR_RETRIES=3` 太紧，吃不下异步落库 + evidence 引用磨合。
- **已完成（Phase 1 · 加性/安全 · 不收紧 gate）**：
  - **A · `list_recent_evidence` 只读工具全链路**：repo 查询 `recent_evidence_detailed_for_session` + `RecentEvidenceRow`（`golish-db/src/repo/audit/mod.rs`）→ trait `DbRepoProvider::recent_evidence_detailed`（默认空，`db_traits/repo.rs`）→ app impl（`db_bridge/{mod,evidence}.rs`）→ 工具定义（`security_tools.rs`，declarations 46→47）→ dispatch（`direct/mod.rs` is_security_analysis_direct_tool + `security.rs` is_sec_tool + handler arm）→ 暴露（registry.rs & builder/mod.rs 的 prober/enumerator/pentester、tool_list.rs READ_ONLY_QUERY_TOOLS、config.rs 安全组、selection_apply.rs 分组、prompt_render.rs ToolRow、list_tools.rs note、frontend/lib/tools.ts）→ prompt（execution_planning.rs prober 方法论）。同步更新相关单测断言（definitions/mod.rs 计数+名、tool_list.rs 成员/禁用、tests.rs prober has_tool、direct/mod.rs 路由）。
  - **C2 · nmap -sV service/version 落 fingerprints**：新 `is_informative_service`（`output_store/helpers.rs`，排除 tcpwrapped/unknown/open/filtered/closed/空）+ `store_fingerprints` 加写 `category="service"` 指纹（仅当 `-sV` 真解析出 version 时；`targets.rs`）。加单测。
  - **E · retry 预算**：`MAX_REFLECTOR_RETRIES` 3→5（`task_orchestrator/types.rs`，附 EAS 异步落库理由）。
- **已 staged（Phase 2 · 收紧/重定时权威 gate · 未写代码，精确定位在设计+计划）**：B（异步落库屏障，`direct/mod.rs`）、C1（`coverage_truth.rs` tcpwrapped 排除，收紧 gate，与 D 耦合否则死锁 53 埠 IP）、D（`stage_coverage.rs` + `rule_engine.rs` 端口无 informative service 时 SERVICE not_applicable 派生）。这三条触 I7/I8 权威 gate，**须 compile+test 循环后才可信**，本轮不盲推。
- **已跑证据**：`ReadLints` 对全部改动的 backend + frontend 文件 **无错**。**未跑** `just precommit` / cargo build / nextest（用户明确「中途不要跑 precommit 不要跑任何大的测试」）。
- **验证状态**：**编译未验证**（遵用户指令）。下一步 = `cargo check`/`just check` + 目标 crate nextest + EAS 活体 smoke（见 feature `eas-worker-evidence-and-service-fingerprint-2026-07-02` 的 verification）。
- **文档**：`docs/design/2026-07-02-eas-worker-evidence-and-service-fingerprint.md`（含风险分级 + Phase 1/2 拆分）、`docs/superpowers/plans/2026-07-02-eas-worker-evidence-and-service-fingerprint.md`；INDEX.md + feature_list.json（新条目 in_progress）已更新。
- **未提交**：以上全部改动 **未 commit、未 push**。

### 2026-07-02 · EAS 工具分流与 SERVICE gap 修复指令

- **本轮目标**：回应用户对最新 EAS run 的追问，确认 naabu/nmap/WhatWeb/httpx 的职责边界，并修正 gate/refiner/worklist 给 AI 的错误工具建议。
- **根因确认**：最新 `/Users/christopherzheng/golish-platform/Test1` run 中 `submit_stage_deliverable` 同时报 evidence 引用缺口和 13 个 `GOLISH-EAS-SERVICE-FINGERPRINT` gap；旧 `coverage_gap_actions.suggested_tools` 对 SERVICE 给 `["nmap -sV","whatweb"]`，且 `stage_refiner` 优先 evidence_refs，导致 repair 指令容易从真实 coverage gap 转去证据改写或把 WhatWeb 当通用 SERVICE 工具。
- **已完成**：
  - `rule_engine` / `stage_coverage` 的 EAS suggested tools 改为 canonical 分工：LIVENESS=`httpx`/`naabu`，PORT=`naabu`/`masscan`/`nmap`，SERVICE=`nmap`。
  - `stage_refiner` 在 submit 同时报 coverage gap 和 evidence_ref 时优先 `CoverageGap`；旧 gap 若仍带 `nmap -sV` 会归一成 `tool_name=nmap`；SERVICE command hint 明确 `nmap -Pn -sV` 只跑 confirmed open ports，WhatWeb 只用于 confirmed HTTP(S) endpoint。
  - `stage_worklist_status` / `stage_worklist_next` 给 EAS gap 增加 `eas_focus`、`worklist_semantics`、tool-boundary next_action，避免 agent 把 gap_examples 自由解释成“所有工具都跑一遍”。
  - 同步 `external_attack_surface/methodology.md` 与相关模块卡。
- **已记录证据 / 验证**：
  - `rustfmt --edition 2021 backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs` → exit 0。
  - `cd backend && cargo test -p golish-agent-kit eas_service_gap_suggests_nmap_only -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-kit submit_needs_fix_prioritizes_eas_coverage_gap_over_evidence_rewrite -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-kit stage_worklist_next_surfaces_eas_tool_boundary -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-app eas_port_found_keeps_service_pending_without_service_outcome -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-kit external_attack_surface_charter_surfaces_liveness_technique -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-kit eas_coverage_gap_instruction_is_batch_first -- --nocapture` → 1 passed / exit 0。
  - `cd backend && cargo test -p golish-agent-kit coverage_preflight_blocks_submit_when_cells_are_pending -- --nocapture` → 1 passed / exit 0。
  - `git diff --check -- <本轮触及文件>` → exit 0。
- **未跑全量**：未跑 `just precommit`；当前工作树已有大量与本轮无关的未提交改动，本轮只做 EAS 工具契约窄修复。
- **下一步建议**：重新触发/继续 EAS 时先看 `stage_worklist_status`，确认 SERVICE gap 只建议 `nmap` 且 `eas_focus` 出现；若旧 run 仍停在 needs_fix，可让 prober 用 confirmed open ports 重跑 `nmap -Pn -sV` 后再 submit。

### 2026-07-02 · submit_stage_deliverable 空字段 canonicalization

- **本轮目标**：回应用户对最新 scoping submit 反复 rejected 的追问，把 submit 契约改得更优雅：模型只填业务事实，空集合/内部默认字段由后端 canonicalize。
- **根因确认**：最新 run `pentest-chat-1782999688847-1` 的 scoping 最终已 accepted；前几次 rejected 是 `skipped_checks[].reason` 直接暴露内部 `SkipReason` tagged enum 后，模型误填 `user_requested` 缺 `user_msg_id`、`other` 缺/空 `evidence_ref`。这属于结构解析失败，不是 scoping gate 语义失败。
- **已完成**：
  - `StageClaim.evidence_ids`、`StageDeliverable.{claims,evidence_refs,findings}`、`HarnessFinding.evidence_refs` 支持省略或显式 `null`，统一归一为空集合。
  - `submit_stage_deliverable` tool schema 改为只要求 `stage_id` + `claims`；`evidence_refs`、claim `evidence_ids`、`findings`、`coverage`、`skipped_checks`、`required_checks_done` 均为空时可省略。
  - submit handler 新增模型输入 canonicalization：空集合字段为 `null` 时视为省略；scoping 里的 legacy/malformed `skipped_checks` 会被移除，普通 scope exclusion 只保留在 claim summary。
  - schema / prompt / scoping methodology 明确：scoping evidence-free claim 不填 evidence；普通 scope exclusion（如不纳入子公司）写进 `scope_confirmed` summary，不再诱导模型硬填 `skipped_checks` / 内部 `SkipReason`。
  - 新增回归：`parameters_make_empty_default_fields_optional`、`accepts_minimal_evidence_free_scoping_deliverable`、`scoping_canonicalizes_legacy_malformed_empty_fields`。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`。
- **运行过的验证（实跑）**：
  - `rustfmt --edition 2021 --check backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-kit/src/harness/types.rs backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs` → exit 0。
  - `cargo test -p golish-agent-app parameters_make_empty_default_fields_optional -- --nocapture` → 1 passed。
  - `cargo test -p golish-agent-app accepts_minimal_evidence_free_scoping_deliverable -- --nocapture` → 1 passed。
  - `cargo test -p golish-agent-app harness_submit_tool -- --nocapture` → 36 passed。
  - `cargo check -p golish-agent-kit -p golish-agent-app` → exit 0。
- **未完成 / 未跑**：
  - `cargo fmt -p golish-agent-kit -p golish-agent-app --check` 因当前工作树里已有其他未格式化改动失败（如 `bridge_config.rs`、`operation_flow.rs`、`execute_harness_loop_tests.rs`），本轮未格式化这些非本 slice 文件。
  - 未跑 `just precommit`，当前工作树本来已有大量其他未提交/未收口改动。
- **提交记录**：未 commit，未 push。

### 2026-07-02 · EAS batch 端口/指纹业务表落地

- **本轮目标**：回应用户发现的 EAS 扫出端口/指纹但 Target Surface 为空的问题。根因已由当前 DB 证实：`59.82.14.249` 的 `technique_outcomes` / `audit_log` 有 naabu 80/443 与 whatweb Tengine evidence，但 `targets.ports` / `fingerprints` / `network_endpoints` / `web_origins` 均为空。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：EAS background completion 在原有 `technique_outcomes` upsert 之外，新增业务事实落地：
    - `httpx` retained stdout(JSONL/URL) → `targets` http facts + `targets.ports` + `fingerprints` + `network_endpoints` + `web_origins` / `web_origin_observations`。
    - `naabu -list` / `masscan -iL` → 保留具体 open port，不再只有 count；写 `targets.ports` 与 `network_endpoints`。
    - `whatweb --input-file` → strip ANSI 后抽 `HTTPServer` / `PoweredBy` / `Title` / status；写 `targets`、`fingerprints`、`network_endpoints`、`web_origins` / observations。
    - `nmap -sV -iL` → 解析 `PORT STATE SERVICE VERSION` open rows；写 `targets.ports`、`fingerprints`、`network_endpoints`。
  - 所有业务表写入沿用当前 `organization_id` 的 in-scope `targets.value` / `targets.real_ip` allowlist，避免把 batch stdout 里的无关 host/IP 落到该 org。
  - 新增纯函数回归测试覆盖 naabu 端口保留、masscan transport、WhatWeb ANSI 指纹抽取、httpx JSONL、nmap `-sV`。
  - 同步模块卡 `docs/modules/backend/golish-agent-app/ai.md`，更新 `feature_list.json` 的 `intel-to-eas-handoff-2026-06-24` notes/verification。
- **运行过的验证（实跑）**：
  - `./init.sh` → 用户随后要求“别跑init”，已 SIGINT；停止在 `check-fe`，命令 exit 130，不计为完成验证。
  - `cargo fmt -p golish-agent-app --check`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo test -p golish-agent-app bridge_config -- --nocapture`（cwd `backend`）→ 24 passed / 113 filtered out，exit 0。
  - `cargo clippy -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `jq empty feature_list.json` → exit 0。
- **未跑 / 未完成**：按用户明确要求未继续跑 `init` / `just precommit`；未重启 app 活体验证，也未对已有历史 run 做一次性 DB repair/backfill。当前修复对后续 EAS background completion 生效；已存在的旧 evidence 若要立刻显示，需要 rerun EAS 或另做 evidence→business-table repair。
- **提交记录**：未 commit，未 push。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app 后 rerun/repair EAS，确认 `59.82.14.249` 的 Target Surface 出现 Network Endpoints 80/443、Web Origins 与 Tengine fingerprint；若要恢复已有 run，不要靠前端推断，单独加一个 evidence/audit_log repair path。

### 2026-07-02 · 攻击阶段三阶段重构 P2.4–P5（接前 session 半成品，BajieAsk agent-4）

- **本轮目标**：接手上一 session 的 `attack-stage-formulaic-candidate-exploit-2026-07-02` 半成品，按 `docs/superpowers/plans/2026-07-02-attack-stage-formulaic-candidate-exploit.md` 继续实现。用户明确要求**全程不跑 init/precommit**，做完再审。
- **接手勘验（先读真实代码）**：P1 骨架（StageKind::AttackCandidate + AttackCandidate 结构 + phase.rs ALL_STAGES 13 + resources.rs 13 臂 + operation_graph.json 13 节点/17 边 + phases.json vuln 段 + 6 profiles + technique_taxonomy/phase_flow/harness_dev.rs 穷尽 match）+ P2 gate op（rule_engine.rs 的 `CandidateGrounded`/`CandidateDispositionComplete` enum + eval + op_name/summary + 单测）已由前 session 写入。缺口：P2.4 未接线、P3/P4/P5 未做。
- **本轮完成（全部 targeted 测试实跑通过）**：
  - **P2.4**：`attack_candidate/spec.json` 加 `candidate_grounded`、`verification/spec.json` 加 `candidate_disposition_complete`。
  - **P3 A 公式化**：`vuln_triage/spec.json` 加 `specialist:vuln_scanner` + `coverage_axis` + `facts_from_db_truth` + `freshness_window` + `coverage_complete.derive_from_evidence`；`expected_techniques` 15→10（移出 SSTI/SSRF/LFI/认证绕过/业务逻辑到 attack_candidate）；`allowed_next_stages`→[attack_candidate,reporting]；新增 `vuln_triage/methodology.md` + `attack_candidate/methodology.md` + `verification/methodology.md`（3 份 playbook），接进 `resources.rs::stage_methodology_md`；同步更新耦合测试（technique_taxonomy 15→10、gate/mod.rs full_vuln_triage_coverage 10 格）+ 加守卫测试（stage_spec.rs 2 个、resources.rs 1 个）。
  - **P4.1/4.2 DB**：新迁移 `20260702000002_attack_candidates.sql`（加性、org FK CASCADE、UNIQUE(operation_id,target,hypothesis_hash)、CHECK priority/disposition）；新 `repo/attack_candidates.rs`（upsert_by_hash/create/list_by_operation/list_by_wave/update_disposition，全 IDOR org 隔离；`hypothesis_hash`=sha256(normalize(target|technique|hypothesis)) 十六进制），注册 repo/mod.rs，Cargo.toml 加 `sha2.workspace`。
  - **P4.3 chain-wave**：新 `harness/chain_wave.rs` 纯决策函数 `decide_chain_wave`（去重+燃料+链深三重收敛，DEFAULT_MAX_WAVES=5/DEFAULT_MAX_CHAIN_DEPTH=3）+ `candidate_dedup_key`，7 单测。
  - **P4.4 rag_prior**：核实 `execute.rs:167` 已对所有阶段（含 attack_candidate）通用注入 wiki prior（`rag_prior_renders_wiki_writeups_for_stage_prompt` 绿），attack_candidate charter 自动获得 PRIOR KNOWLEDGE 段——设计 §3.3 的落点已被既有通用接线覆盖。
  - **P5.2 reporting**：`reporting/spec.json` `inherits_evidence_from` 增补 vuln_triage(vuln_finding) + verification(poc/exploit_verified/attack_path)。
  - **P5.1 agent 面**：verification methodology 操作化「approved candidate→真打→disposition→finding 升格 + parent_finding_id 血缘」（gate 侧 candidate_disposition_complete 已由 P2.4 就位）。
  - **planner/router**：prompts/mod.rs planner stage 清单加 attack_candidate + 重写 vuln_triage/verification 描述；harness_backfill.rs `OTHER_STAGE_KEYWORDS` 加 AttackCandidate 关键词组 + vuln_triage 加「formulaic/公式化扫描」。
- **已记录证据（targeted，未跑 init/precommit——按用户指令）**：
  - `cargo nextest -p golish-agent-kit -E 'test(chain_wave)|test(candidate)|test(disposition)|test(vuln_triage)|test(attack_stage_playbooks)|test(phases_cover_all_13)|test(base_graph_has_13)|test(all_thirteen)'` → **33 passed**（run 451bf321）。
  - `cargo nextest -p golish-agent-kit -E 'test(harness::)|test(task_orchestrator::)'` → **589 passed / 0 failed**。
  - `cargo nextest -p golish-agent-kit -E 'test(resources::)|test(stage_spec::)|test(technique_taxonomy::)|test(gate::tests::)'` → **68 passed**。
  - `cargo nextest -p golish-db -E 'test(attack_candidates)'` → **6 passed**。
  - `cargo build -p golish-agent-app -p golish-db` → **exit 0**（Finished）。ReadLints 4 新/改文件无错。
- **未提交的半成品（本 scope，全未 commit）**：`resources/harness/stages/{attack_candidate,vuln_triage,verification}/*` + `reporting/spec.json`；`golish-agent-kit/src/harness/{chain_wave.rs(new),resources.rs,technique_taxonomy.rs,mod.rs,gate/mod.rs}` + `task_orchestrator/{prompts/mod.rs,harness_backfill.rs,stage_spec.rs test}`；`golish-db/{Cargo.toml, migrations/20260702000002_*.sql(new), src/repo/attack_candidates.rs(new), repo/mod.rs}`。
- **仍待完成（需运行中 app / 前端 / 控制流手术，本轮未做，明确留给后续）**：
  1. **P4.3 活体接线**：把 `decide_chain_wave` 的 `OpenNextWave` 接进 `execute.rs` graph-flow（metalcraft 引擎节点转移处覆写游标回 attack_candidate）——纯函数已就位+单测，但引擎级游标覆写有打断 harness loop 的风险，需能跑 app 观察循环才敢动。
  2. **P5.1 运行时落库**：`submit_stage_deliverable` 路径把 `StageDeliverable.candidates` 写进 `attack_candidates` 表 + verified→`HarnessFinding` 升格 + `parent_finding_id` 血缘（gate 侧已就位，缺 submit 路径的 repo 写接线）。
  3. **P5.3 ts-rs + 前端攻击链图**（AttackCandidate/Disposition 跨 IPC + 攻击链可视化组件）。
  4. **P5.4 端到端**（mock 资产集 --stage-run 跑三阶段 + 波次回流 trace）。
- **偏差（记入 plan/feature_list）**：P3 `authoritative_found` **未开**（只开 derive_from_evidence）——nuclei/dir/weakpw/tls 的 technique_outcomes 写路径尚未覆盖（evidence_facts.rs 无对应映射），此时开 authoritative_found 会让那几格永远无真值→活体 gate 永久 BLOCK；对齐设计 §11 开放问题 3 + plan 偏差 #2「写路径 deferred」，故本轮只做安全加性子集。
- **风险 / 下一步**：① 上述 4 项活体/前端/e2e 工作；② 本轮改动未跑 `just precommit`（用户指令），commit 前需用户点头并补跑全量门禁；③ feature 维持 `in_progress`（P5 未闭环、precommit 未跑，按 AGENTS.md §3 DoD 不得转 passing）。

### 2026-07-02 · Target Surface Phase 2.5C 轻量 legacy refs + in-app backfill 命令

- **本轮目标**：用户要求一次性把「资产详情重构（IP → NetworkEndpoint → WebOrigin → 内容层）」剩余逻辑补完。先扫描确认 Phase 2.1–2.5B 已实现（migration 三表、identity/queries/backfill/content_queries repo、`target_surface_hierarchy_get` 返回 identity+contentCounts、前端 adapter+fallback、IP 页无全局 Sitemap、Sitemap/API/JS/Params 只在 WebOrigin detail、IP-literal 不进 Related Domains、显式端口分开、相对 URL→unassigned、unmatched 不物化）。定位到两处缺口并补齐。
- **补的逻辑**：
  - **缺口 1（lightweight legacy refs）**：`target_surface_hierarchy_get` 原本只返回 contentCounts、不返回 refs（Phase 2.5A 明确 defer）。`surface_content_queries.rs` 新增 `SurfaceContentRef { kind,id,url,method?,status_code?,capture_path?,source? }`，扩展 `LIST_SURFACE_LEGACY_CONTENT_ROWS_SQL` 选出 method/status_code/capture_path/source（api=method+status+capture+source，js=file_path+source_tool，directory=status+tool，passive=tool_used），聚合出 `refs_by_origin` + `unassigned_refs`，各 capped `MAX_REFS_PER_BUCKET=200` 并按 legacy row id dedup；counts 仍是总数事实源，refs 只是指针，绝不伪造成完整 row。`target_surface_hierarchy.rs` 加 `WebOriginContentRefDto` + `WebOriginHierarchyDto.refs` + `UnassignedWebDataDto.refs`。
  - **缺口 2（backfill 无法在 app 触发）**：`backfill_surface_identity` 只是库函数、无任何 caller，导致真 app 里 identity 三表永远为空→backend hierarchy 恒回退 frontend。新增 `#[tauri::command] target_surface_identity_backfill(project_path?, organization_id?)`（golish-pentest-app），调用只读 backfill（additive/idempotent、只写 identity 三表、从不改 legacy）；facade 已 glob，registry 注册。
  - **前端消费**：`security-analysis.ts` 加 `BackendWebOriginContentRefDto` + origin/unassigned 的 `refs` + `surfaceIdentityBackfill` wrapper + `SurfaceIdentityBackfillSummary`；`surfaceHierarchy.ts` 加 `WebOriginContentRef` 类型 + `WebOriginVM.contentRefs`（createOrigin 默认 []）；`backendSurfaceHierarchy.ts` 把 backend refs 映射进 `contentRefs`（frontend-inferred/fallback origin 恒空）；`WebOriginsTab.tsx` 的 OriginDetail 在 frontend full rows 缺席时用新 `BackendRefList` 在 APIs/JS/Sitemap tab 渲染轻量 refs（标注「From backend content index」，不提升成完整 row），并把「backend 有 count 但 rows 未加载」提示改为「有 refs 就展示 refs、无 refs 才提示」；`TargetSurfaceWorkbench.tsx` IP header 加「Build identity from data」按钮触发 backfill 后 reload。
- **运行过的验证（实跑）**：
  - `cargo test -p golish-db surface_content_queries -- --nocapture`（cwd backend）→ 14 passed（含 3 新 refs 用例）。
  - `cargo test -p golish-pentest-app target_surface_hierarchy -- --nocapture`（cwd backend）→ 15 passed（含 refs flow-through 2 用例）。
  - `cargo test -p golish-db -p golish-pentest-app`（cwd backend）→ golish-db 181 passed（doc-test 1 ignored）+ golish-pentest-app 151 passed / 3 ignored。
  - `cargo check -p golish`（cwd backend）→ exit 0。
  - `cargo fmt -p golish-db -p golish-pentest-app -p golish --check`（cwd backend）→ exit 0。
  - `cargo clippy -p golish-db -p golish-pentest-app --all-targets -- -D warnings`（cwd backend）→ exit 0。
  - `cargo clippy -p golish --lib -- -D warnings`（cwd backend）→ exit 0。
  - `vitest run backendSurfaceHierarchy.test.ts surfaceHierarchy.test.ts` → 2 files / 25 passed（backendSurfaceHierarchy 20，含 Phase 2.5C 2 新用例）。
  - `npm run typecheck` → exit 0；`npm run check` → 793 files no fixes；`npm test -- --run` → 147 files / 1545 passed / 12 skipped。
  - `just check-fe` → exit 0；`just test-fe` → exit 0。
  - `jq empty feature_list.json` → exit 0。
- **提交记录**：未 commit，未 push。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/surface_content_queries.rs`、`backend/crates/golish-pentest-app/src/target_surface_hierarchy.rs`、`backend/crates/golish/src/commands_registry.rs`、`frontend/lib/api/security-analysis.ts`、`frontend/lib/security-analysis.ts`、`frontend/components/TargetPanel/surface/surfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts`、`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`、`frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app.md`、`docs/modules/frontend/{components,lib}.md`、`feature_list.json`、`agent-progress.md`。
- **未完成 / 待补验证**：按用户明确要求**未跑** `just precommit` 与 `./init.sh`（工作树仍有前序 harness 等大量未提交改动，precommit 全量绿门禁未在本轮验证）。真实 Tauri app 里点「Build identity from data」→ 观察 IP 页 backend hierarchy 从 fallback 变真实、WebOrigin detail 出现 backend refs 的视觉 QA 未做（需用户本地跑 app）。
- **明确 TODO**：① collector 写入时同步 identity/observation provenance（消灭对手动 backfill 的依赖）；② 若要 refs 更完整可加分页/按 kind 下钻；③ 用户放开后跑全量 `just precommit`（前序 `surface_identity_backfill.rs` clippy 已在更早轮修复，本轮 clippy -D 全绿）。
- **追加（同会话）· 写入对账 + backfill gap A/B 修复**：应用户「对一下 scoping/intel/EAS 写入 ↔ 现有表/展示」要求逐阶段核写入路径（scoping→targets/organizations；target_intel→dns_records+real_ip、target_assets(service `value="port/proto"` 挂域名 · `asset_intel/landing.rs:564`)、target_assets(subdomain · `persistence.rs:287`)、organizations.*；EAS→targets.ports(JSON, `output_store/targets.rs:50`+`helpers.rs:18`)+real_ip/http_status/…、fingerprints)。结论：**展示层全对得上**；identity 层两处回不去：A) 被动 service target_asset（`value="443/tcp"` 无 IP）、B) 写在域名 target 上的 EAS `targets.ports`，都因缺 IP 进不了 `network_endpoints`。用户「可以修」→ 改 `surface_identity_backfill.rs`（纯 additive）：`LIST_TARGET_ASSET_ROWS_SQL` 增选 `t.real_ip`；新增 `target_port_endpoint_ip`（域名/URL target ports 用 real_ip 键）+ `ip_for_target_asset`（service asset 无显式 IP 时用 real_ip）；real_ip 补出的端点标 inferred（`backfill:*.real_ip`、confidence 0.6、last_confirmed=false），real_ip 空则跳过。验证：`cargo test -p golish-db surface_identity_backfill` → 16 passed（+5 新）；`cargo test -p golish-db` → 195 passed；fmt --check + clippy -D → exit 0。**未改 harness gate**（覆盖门消费 identity = 单独设计轮）。方法学评估（回用户提问）：IP 扫网络面(端口/服务每 IP 一次)、域名扫 vhost Web 面(每个都要)、多域名同 IP=1 endpoint←N origin(已支持)、真别名站靠 observation body_hash/favicon_hash 去重(enumeration 后续增强,未实现)。

### 2026-07-02 · Target Surface Phase 2.5B 前端消费 backend contentCounts

- **本轮目标**：按用户贴的 Phase 2.5B 约束（不改后端 / DB / schema / migration / 采集器 / Tauri command / 旧接口返回结构，不删 surfaceHierarchy.ts 与 backendSurfaceHierarchy.ts fallback，不恢复 IP 全局 Sitemap，不要求跑 just precommit），让前端开始消费 backend `contentCounts`，同时保留现有 frontend legacy content fallback。
- **已完成**：
  - **第一步 DTO**：`frontend/lib/api/security-analysis.ts` 新增 `BackendWebOriginContentCountsDto`、`BackendUnassignedWebDataCountsDto`；`BackendWebOriginDto.contentCounts`（`| null`）、`BackendUnassignedWebDataDto.counts`（`| null`）、`BackendSurfaceSummaryDto` 加 `urlCount/apiCount/jsCount/paramCount/directoryEntryCount/passiveLogCount/evidenceCount/contentUnassignedCount/contentUnmatchedOriginCount`（均 `number | null`）。新增 `presentNumberField` 区分「字段缺失（null → 前端回退）」与「存在为 0」；整块 `contentCounts`/`counts` 缺失归一为 `null`。`frontend/lib/security-analysis.ts` 补 re-export。未手改 `frontend/lib/generated/`。
  - **第二步 adapter**：`backendSurfaceHierarchy.ts` 新增 `WebOriginVM.contentCountSource`（`backend_content_counts` / `frontend_content_inferred`）与 `counts.passiveLogs`；`mapBackendOrigin` 计数优先级 = backend contentCounts（存在时）> frontend origin 计数 > 0，`findings` 恒取 frontend（backend 不聚合）；detail rows 仍取 frontend arrays，绝不因 backend count 存在伪造 rows，也不 double count。
  - **第三步 summary**：新增 `mergeSummary`，IP Overview/summary 的 content counts 优先 backend summary，逐字段缺失回退 frontend；透出 `directoryEntryCount/passiveLogCount/contentUnassignedCount/contentUnmatchedOriginCount` 与 `contentCountSource`；`findingCount` 保持 frontend。
  - **第四步 WebOriginsTab**：URL/API/JS/Params/Evidence 列直接消费 `origin.counts`（已按优先级来自 backend）；URL 列 tooltip 显示 directory entries，Evidence 列 tooltip 显示 passive logs，Source 列 tooltip 标注 identity + content-count source。
  - **第五步 Origin detail**：Overview 显式列出 Identity source / Content count source（含 directory/passive 补充）；backend count > 已加载 frontend rows → 提示「still loaded from the legacy frontend data sources」，backend 有 count 但无 frontend rows → 提示「not loaded in this view yet」；Sitemap/APIs/JS/Params 仍只展示 frontend rows。
  - **第六步 unassigned/unmatched**：`TargetSurfaceWorkbench` IP Overview 在 backend 内容计数驱动且有 unassigned/unmatched 时显示「Backend content aggregation found X unassigned items and Y unmatched-origin items.」；unmatched-origin 不物化为 WebOrigin。
  - **第七步 测试**：`backendSurfaceHierarchy.test.ts` 更新 helper（`backendSummary`、`contentCounts`、`backendOrigin` 支持 contentCounts、`backendHierarchy` merge summary/unassigned），新增 Phase 2.5B A–J 用例。
  - 同步模块卡 `docs/modules/frontend/{components,lib}.md`、`feature_list.json`（target-surface-workbench notes 追加 2.5B）、本 progress。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts` → 2 files / 23 passed（其中 backendSurfaceHierarchy 18）。
  - `./node_modules/.bin/biome check --write <本轮 7 个前端文件>` → exit 0，Fixed 2 files。
  - `npm run typecheck`（`tsc --noEmit`）→ exit 0。
  - `npm run check`（`biome check ./frontend`）→ exit 0，Checked 793 files，No fixes。
  - `npm test -- --run` → exit 0，147 test files passed，1543 passed / 12 skipped（较 2.5A 的 1534 +9）。
  - `just check-fe` → exit 0；`just test-fe` → exit 0。
  - `jq empty feature_list.json` → exit 0。
- **提交记录**：未 commit，未 push。
- **本轮修改但未提交（本 scope）**：`frontend/lib/api/security-analysis.ts`、`frontend/lib/security-analysis.ts`、`frontend/components/TargetPanel/surface/surfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts`、`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`、`frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`docs/modules/frontend/{components,lib}.md`、`feature_list.json`、`agent-progress.md`。
- **未完成 / 待补验证**：按用户要求未跑 `just precommit`（且工作树仍有前序后端 `surface_identity_backfill.rs` clippy warning 阻塞全量绿门禁，属本 slice 之外）；真实 Tauri app 中 IP 页面的 contentCounts/hint/aggregation 尚未做视觉 QA/截图。
- **Phase 2.5C 建议**：把 legacy content 的行级数据也按 origin 归属（写入时带 origin/observation provenance 或新增只读 refs），让 detail rows 与 backend counts 对齐、消除「backend 有 count 但 rows 未加载」提示；再考虑把 unmatched-origin 明细做成可下钻列表（仍不物化为 WebOrigin）。

### 2026-07-02 · Target Surface Phase 2.5A backend legacy content aggregation

- **本轮目标**：按用户贴的 Phase 2.5A 约束，在不改前端 / 不改 DB schema / 不改 migration / 不改采集器 / 不回写旧表的前提下，让 `target_surface_hierarchy_get` 返回 backend identity hierarchy 时附带 legacy web content 的按 WebOrigin counts 聚合。
- **已完成**：
  - 新增 `backend/crates/golish-db/src/repo/surface_content_queries.rs`：只读查询 legacy `api_endpoints` / `js_analysis_results` / `directory_entries` / `passive_scan_logs`，输入为 `SurfaceContentQuery { organization_id, project_path, root_target_id, root_ip, origin_keys, include_related }`。
  - candidate target ids 限定为 root IP target、同 org/project 且 `real_ip == root_ip` 的 domain/url/wildcard target、host 是 root IP 的 IP-literal URL target；`include_related=false` 时只用 root target；legacy content query 只按 candidate `target_id = ANY($3)` 读取，不扫整个 org/project。
  - URL 归属统一复用 Phase 2.1 `normalize_web_origin`：相对 URL / 缺失 URL / unsupported scheme / malformed URL 只进入 unassigned counts；解析出的 origin 不在 backend `webOrigins` 中时只进入 unmatched counts，不创建 WebOrigin。
  - `target_surface_hierarchy_get` DTO 扩展：`WebOriginHierarchyDto.contentCounts`；`SurfaceHierarchySummaryDto` 增加 `url/api/js/param/directoryEntry/passiveLog/evidence/contentUnassigned/contentUnmatchedOrigin` counts；`UnassignedWebDataDto.counts` 增加 relative/malformed/unsupported/missing/unmatched 明细。refs 暂不返回，避免本阶段把旧表行细节变成前端新契约。
  - 计数语义：`apiCount/jsCount/directoryEntryCount/passiveLogCount` 按对应表 `id` 去重；`paramCount` 对 `api_endpoints.params` 的 JSON array 计元素、object 计 key；`urlCount` 只统计 matched origin 下 api/js/directory 的 unique URL 字符串；`evidenceCount` 本阶段等于 matched `passiveLogCount`。
  - 顺手修复前序 `surface_identity_backfill.rs` 测试代码的两个 clippy lint（type alias + `slice::from_ref`），不改 backfill 行为。
  - 同步模块卡：`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app.md`；`feature_list.json` 的 `target-surface-workbench` notes 追加 Phase 2.5A 记录，未标 passing。
- **运行过的验证（实跑）**：
  - `./init.sh` 曾按 AGENTS 开始执行，用户随即要求“不要跑init”，已 SIGINT 停止；不作为本轮完成验证。
  - `cargo fmt -p golish-db -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `cargo test -p golish-db surface_content_queries -- --nocapture`（cwd `backend`）→ 11 passed。
  - `cargo test -p golish-db surface_identity_queries -- --nocapture`（cwd `backend`）→ 11 passed。
  - `cargo test -p golish-db surface_identity_backfill -- --nocapture`（cwd `backend`）→ 11 passed。
  - `cargo test -p golish-db -- --nocapture`（cwd `backend`）→ 181 passed，doc-test 1 ignored。
  - `cargo test -p golish-pentest-app target_surface_hierarchy -- --nocapture`（cwd `backend`）→ 13 passed。
  - `cargo test -p golish-pentest-app -- --nocapture`（cwd `backend`）→ 149 passed / 3 ignored。
  - `cargo check -p golish`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-db -p golish-pentest-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `npm run typecheck` → exit 0。
  - `npm run check` → exit 0，Biome checked 793 files。
  - `npm test -- --run` → exit 0，147 test files passed，1534 passed / 12 skipped。
  - `just precommit` → 用户要求“just precommit也别跑 太慢了”，已 SIGINT 停止；停止前已看到 `fmt` passed、`check-fe` passed，`test-fe` 已开始但未完成，本命令不计作绿门禁。
- **提交记录**：未 commit，未 push。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/surface_content_queries.rs`、`backend/crates/golish-db/src/repo/mod.rs`、`backend/crates/golish-db/src/repo/surface_identity_backfill.rs`、`backend/crates/golish-pentest-app/src/target_surface_hierarchy.rs`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app.md`、`feature_list.json`、`agent-progress.md`。
- **未完成 / 待补验证**：`just precommit` 按用户要求中断，未完成绿门禁；真实 Tauri app 中 IP TargetSurfaceWorkbench 尚未做视觉 QA。工作树里仍有多轮前序 Target Surface / harness 未提交改动，未回滚。
- **Phase 2.5B 建议**：在不破坏只读 Phase 2.5A 的前提下，下一步可以补 lightweight refs / capture refs，并考虑 collector 写入时同步 observation provenance；如果要把旧表行真正绑定到 WebOrigin，应走 schema/写入路径设计，而不是在 command 里临时伪造。

### 2026-07-02 · Target Surface Phase 2.4 前端接入 backend identity hierarchy

- **本轮目标**：按用户贴的 Phase 2.4 约束，在不改后端/DB/schema/migration/采集器/旧接口的前提下，让 IP TargetSurfaceWorkbench 调用 `target_surface_hierarchy_get`，并把 backend identity layer 与现有 frontend inferred legacy content layer 双层合成。
- **已完成**：
  - `frontend/lib/api/security-analysis.ts` 新增本地 DTO 与 `targetSurfaceHierarchyGet(targetId, includeRelated)` wrapper；`frontend/lib/security-analysis.ts` 同步 re-export。未手改 `frontend/lib/generated/`。
  - `useTargetSurfaceData` 对 IP target 额外加载 backend hierarchy，返回 `backendHierarchy` / `backendHierarchyStatus` / `backendHierarchyError`；backend command 报错或 fallback 不会让旧表数据加载失败。
  - 新增 `frontend/components/TargetPanel/surface/backendSurfaceHierarchy.ts` adapter：backend endpoints/webOrigins/observations 作为 identity layer，frontend `buildSurfaceHierarchy` 继续提供 Sitemap/APIs/JS/Params/Evidence content；按精确 `origin` (`scheme://host:port`) union，显式端口不合并。
  - `TargetSurfaceWorkbench` 对 IP 页面显示 backend/fallback 数据来源提示；domain/url target 仍走旧 `Identity / Surface / Sitemap / Sensitive / Evidence`。
  - `NetworkEndpointsTab` 支持 backend identity 列：IP / Port / Transport / State / Service / TLS / Web Origin count / Confidence / Source。
  - `WebOriginsTab` 支持合成 origin 列：Origin / Scheme / Host / Host Type / Port / Observed endpoint count / URL/API/JS/Params/Evidence counts / Confidence / Source；backend-only origin detail 常驻提示 `Backend identity exists, but legacy web content has not been linked to this origin yet.`
  - 新增 adapter 回归 `backendSurfaceHierarchy.test.ts`，覆盖 backend success、backend-only origin、frontend-only origin、backend unavailable fallback、legacy/domain fallback、IP-literal origin、不重复合并、显式端口不合并、IP 无全局 Sitemap tab。
  - 同步模块卡：`docs/modules/frontend/{components,lib}.md`；`feature_list.json` 的 `target-surface-workbench` notes 追加 Phase 2.4 记录，未标 passing。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；依赖安装、fmt、check-fe、test-fe 均通过，随后 `lint-rust` 在既有后端文件 `backend/crates/golish-db/src/repo/surface_identity_backfill.rs:579` 报 clippy `unnecessary_lazy_evaluations`（建议 `.or(Some(port == 443))`），本轮按“前端-only”约束未改后端。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check --write <本轮前端文件>` → exit 0，Fixed 4 files；随后 `./node_modules/.bin/biome check <本轮前端文件>` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts frontend/components/TargetPanel/TargetDetail.test.tsx` → 3 files / 15 tests passed，exit 0。
  - `npm run typecheck` → exit 0。
  - `npm run check` → exit 0，Biome checked 793 files。
  - `npm test -- --run` → exit 0，147 test files passed，1534 passed / 12 skipped。
  - `just check-fe` → exit 0。
  - `just test-fe` → exit 0。
  - `just precommit` → exit 101；fmt/check-fe/test-fe 通过，`lint-rust` 在同一个 `surface_identity_backfill.rs:579` clippy warning 失败。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --check -- <本轮相关文件 + docs/feature/progress>` → exit 0。
- **提交记录**：未 commit，未 push。
- **本轮修改但未提交（本 scope）**：`frontend/lib/api/security-analysis.ts`、`frontend/lib/security-analysis.ts`、`frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`、`frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`frontend/components/TargetPanel/surface/surfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.ts`、`frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts`、`frontend/components/TargetPanel/surface/tabs/NetworkEndpointsTab.tsx`、`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`feature_list.json`、`agent-progress.md`。
- **工作树里已有但本轮未改/未收口的未提交项**：Phase 2.1-2.3 后端 identity/backfill/query/command 文件仍未提交；`frontend/components/TargetPanel/{TargetGroupedView.tsx,surface/surfaceHierarchy.test.ts}`、`frontend/lib/target-panel/asset-groups.test.ts` 等前序 TargetPanel 改动仍在工作树中。未回滚。
- **已知风险 / 未解决问题**：全量 `just precommit` 仍不能绿，阻塞点是前序后端 `surface_identity_backfill.rs` clippy warning；若要严格按仓库门禁 passing，需要用户允许修该后端 lint 或由前序后端 slice owner 处理。尚未在真实 Tauri app 里点 IP 页面做视觉 QA/截图。
- **下一步最佳动作**：Phase 2.5 建议做 backend/content link：让 legacy web content（`api_endpoints` / `js_analysis_results` / `directory_entries` / params/capture）在写入时带可追溯 origin identity 或 observation 关联，并补一个只读 query 聚合 content counts；在此之前前端 adapter 继续作为兼容层保留。

### 2026-07-01 · Enumerator 最新 run route canonicalization 修复

- **本轮目标**：回应用户“再看看最新一次，确认问题就改”，复核当前仍在跑的 `pentest-chat-1782791610659-1` Enumerator transcript / DB truth，并修复确认的 route/browser/js URL canonicalization 问题。
- **最新日志 / DB 证据**：
  - 最新 active sub-agent：`enumerator-call_00_AgmiS5OHFC1pPOzr3vh24459::org::602c3fa6-dea9-42e8-a995-00281c517517`；已从 worklist-first 路径重新查询，`stage_worklist_status` 显示进展到 `27 done / 4 error / 165 pending`，但随后又准备重跑 error cells。
  - `route_probe_paths` 结果里 `package.moresec.cn:9443` 被重复跑两次：请求是 `http://package.moresec.cn:9443/`，effective 变成 `https://package.moresec.cn:9443/`，两次都是 `1893/1893` candidate requests 全 error。
  - `sso-test-dayu.moresec.cn` / `update-center.moresec.cn` 的请求原本是 `https://host/`，但 canonicalization 改成 `https://host:9443/` 后也全 error。
  - DB `target_assets` 证实：`package.moresec.cn` 的 service 是 `9443/tcp http`（plain HTTP hint），而 `sso-test-dayu` / `update-center` 是 `9443/tcp http/ssl`；同时 `dayu.moresec.cn` route probe 已有真实 found 进展，`directory_entries` 新增大量 `https://dayu.moresec.cn/api/...` 401 auth-wall 路径，`GOLISH-ENUM-DIR found` 变为 2 个 target / `result_count=3692`。
- **根因判断**：
  - `target_resolver::best_web_service_candidate` 对“调用方没有显式端口”的 URL 偏好 HTTPS + 非 80 端口，导致已有默认 443 service 的 `https://host/` 被错误提升到 `:9443`。
  - `scheme_for_service` 看到端口 `9443` 就强制 HTTPS，覆盖了 EAS service hint 里的 plain `http`，导致显式 `http://package.moresec.cn:9443/` 也被改坏。
- **已完成**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs`：无显式端口时，若请求 URL 的默认端口（`http:80` / `https:443`）已经是 EAS-confirmed service，优先保留该 root；只有默认端口没有确认服务时才回落到非默认 web 端口。
  - `scheme_for_service` 改为先信 service hint：`https` / `ssl` / `tls` → HTTPS，plain `http` → HTTP；hint 不明确时才用 443/8443/9443/10443 端口启发式。
  - 新增回归测试覆盖：plain `9443/tcp http` 保持 HTTP；`http/ssl` 仍为 HTTPS；`https://host/` 在 443 和 9443 同时存在时保留 443；`http://host/` 在 80 和 9443 同时存在时保留 80；默认端口不存在时仍能回落到 `https://host:9443/`。
  - `docs/modules/backend/golish-pentest-app/pentest_bridge.md` 同步 canonicalization 规则。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app target_resolver --status-level fail`（cwd `backend`）→ 8 passed / 131 skipped。
  - `cargo nextest run -p golish-pentest-app route_probe_paths --status-level fail`（cwd `backend`）→ 9 passed / 130 skipped。
  - `cargo clippy -p golish-pentest-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有多轮未提交 Enumerator/UI/DB 半成品，且此前 pnpm install/approve-builds 门仍在 progress 中记录为未解）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **风险 / 下一步**：当前正在跑的 backend/direct tool 可能仍是旧进程或旧调用；需要重启 app/backend 后重新跑/续跑 enumeration，确认 `package.moresec.cn:9443` 用 HTTP，且默认 `https://host/` 不再被擅自改到 `:9443`。DIR error cells 仍可能包含真实目标不可达，需要用新代码跑出的 `requested/effective URL + service hint` 再判断。

### 2026-07-01 · Enumerator worklist-first prompt 接线

- **本轮目标**：用户确认“可以改”后，把上一刀新增的 `stage_worklist_status` / `stage_worklist_next` 真正接进 stage_run/Enumerator 的 prompt 契约，避免工具已存在但专家仍按旧习惯看大 coverage 输出、做一点就 submit。
- **已完成**：
  - `stage_run_call::build_org_objective`：per-org specialist objective 的 mandatory pre-submit self-check 改为 worklist-first loop：先 `stage_worklist_status(stage, organization_id)`，`ready_to_submit=false` 时必须 `stage_worklist_next(prefer=["pending","error"])`，只处理 items 点名的 asset×technique cell；`check_stage_asset_coverage` 退为最终 compact sanity check。
  - 默认 `enumerator` 工具集新增 `stage_worklist_status` / `stage_worklist_next`（普通 builder 和 registry builder 两条路径都加）。
  - `build_enumerator_prompt` 改为每个 normal/repair pass 先读 stage-local worklist；`list_enumeration_web_roots` / `query_target_data` 只作为上下文，不再是权威计划。
  - `resources/harness/stages/enumeration/methodology.md` 改为 `stage_worklist_status -> stage_worklist_next -> small batch -> re-query -> ready_to_submit=true -> submit` 的停机条件。
  - 同步模块卡：`docs/modules/backend/golish-sub-agents.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-sub-agents -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
  - `cargo nextest run -p golish-sub-agents enumerator --status-level fail`（cwd `backend`）→ 2 passed / 112 skipped，exit 0。
  - `cargo nextest run -p golish-agent-runtime build_org_objective --status-level fail`（cwd `backend`）→ 2 passed / 287 skipped，exit 0。
  - `cargo nextest run -p golish-agent-kit stage_methodology --status-level fail`（cwd `backend`）→ 2 passed / 777 skipped，exit 0。
  - `cargo check -p golish-sub-agents -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo fmt -p golish-sub-agents -p golish-agent-runtime --check`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-sub-agents -p golish-agent-runtime --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
- **未跑 / 未完成**：未跑 `just precommit` / `./init.sh`（上一轮刚确认 `./init.sh` 被既有 frontend `asset-groups.test.ts` baseline 卡住）；未重启 app 做 live enumeration rerun；未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-sub-agents/src/defaults/{builder/mod.rs,builder/registry.rs,prompts/execution_planning.rs,tests.rs}`、`resources/harness/stages/enumeration/methodology.md`、上述模块卡、`agent-progress.md`。
- **下一步建议**：重启 backend/app 后跑新 enumeration；run_tree 里应该看到 Enumerator 开局/repair 先用 `stage_worklist_status` / `stage_worklist_next`，而不是直接从 `list_enumeration_web_roots(limit=100)` 自己挑一堆目标后试 submit。

### 2026-07-01 · stage worklist + sub-agent 工具结果压缩

- **本轮目标**：回应用户对 runstage / stage_run 的问题：枚举阶段没搞完就反复 submit，需要每阶段都有 stage-local worklist/plan 视角；同时把大工具结果压缩后再喂回模型，原始结果继续保留在 transcript/UI/evidence。
- **本轮日志结论（实查 Test1 最新 run）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --db` → 最新会话 `pentest-chat-1782791610659-1` 显示 `submits=11 needs_fix=9`，且提示 `repeated identical needs_fix likely`。
  - enumeration 的 `stage_run` 重试 3 次仍 blocked；run.log 里最后一轮 coverage gate 在 `2026-07-01T05:28:34Z` 报 `114` 个未 terminal cells，主要是 `GOLISH-ENUM-DIR` / `GOLISH-ENUM-PARAM` / `GOLISH-ENUM-JSAPI`。
  - 同一 run 里曾有 `route_probe_paths timed out after 300s` 与 `js_extract_apis timed out after 300s`。最近一轮虽然做了部分 `browser_collect_js_api` / `route_probe_paths`，但 agent 仍靠大而散的 `list_enumeration_web_roots` / `query_target_data` 自己挑目标，缺一个“小而确定”的下一批 gap list。
- **已完成**：
  - `golish-sub-agents`：新增模型可见工具结果压缩层。`AiEvent::SubAgentToolResult`、transcript 和 UI 仍写完整 raw JSON；只有下一轮喂给 sub-agent LLM 的 `ToolResult` 会被压成 counts / samples / next_action。
  - 已覆盖压缩：`route_probe_paths`、`list_enumeration_web_roots`、`browser_collect_js_api`、`js_extract_apis`，以及超大通用 JSON。route probe 会保留 `matches_count`、`rejected_candidates_count`、错误 top、样本、`queue_completed` / `timed_out` / `next_action`，不再把成百上千条候选完整塞进上下文。
  - 新增只读 DB-truth 工具 `stage_worklist_status` / `stage_worklist_next`。它们基于 `stage_asset_coverage` 输出当前阶段的 asset × technique work items，默认只返回 pending/error cell，带 `work_item_id`、target/asset、technique、state、evidence refs、suggested_tools、ready_to_submit 和下一步动作。
  - 两个 worklist 工具已接入 security tool executor、tool schema、main-agent 工具选择、active-stage read-only allowlist、direct-tool routing、execution-mode 文案和对应测试。
  - 同步模块卡：`golish-sub-agents` / executor、`golish-agent-kit` tool definitions/executors、`golish-tools` definitions、`golish-agent-runtime` agentic_loop/execution_mode。
- **设计判断**：
  - 不建议先让 runstage 专家再随意叫 subagent 来 plan；那会把“计划”继续放在模型自然语言里，仍然容易漏 cell。更稳的是每个阶段都暴露一个 deterministic worklist/status 工具，让专家按 DB truth 取下一批工作项，只有 `ready_to_submit=true` 时才 submit。
  - prompt 可以后续再补，但本轮先补工具契约：以后 prompt 只需要要求“先 `stage_worklist_status`，再循环 `stage_worklist_next`，ready 后 submit”，不用靠模型从超长 coverage JSON 自己推理。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1，基础环境验证在前端测试失败：`frontend/lib/target-panel/asset-groups.test.ts > groupTargetsByHost > groups an IP target with domains and URLs that resolve to it`，期望 `['domain','url']`，实际 `['domain']`；与本轮后端 worklist/压缩改动无关。
  - `cargo fmt -p golish-sub-agents -p golish-agent-kit -p golish-tools -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit tool_executors::security --status-level fail`（cwd `backend`）→ 8 passed / 771 skipped，exit 0。
  - `cargo nextest run -p golish-sub-agents response_parsing --status-level fail`（cwd `backend`）→ 26 passed / 88 skipped，exit 0。
  - `cargo nextest run -p golish-tools definitions --status-level fail`（cwd `backend`）→ 7 passed / 67 skipped，exit 0。
  - `cargo nextest run -p golish-agent-runtime tool_list direct_tool_routing_tests --status-level fail`（cwd `backend`）→ 12 passed / 277 skipped，exit 0。
  - `cargo check -p golish-sub-agents -p golish-agent-kit -p golish-tools -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo fmt -p golish-sub-agents -p golish-agent-kit -p golish-tools -p golish-agent-runtime --check`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-sub-agents -p golish-agent-kit -p golish-tools -p golish-agent-runtime --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑 / 未完成**：未跑 `just precommit`（`./init.sh` 已被既有前端测试 baseline 卡住）；未重启 app 做 live enumeration rerun；未改 prompt；未改 schema/migration；未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-tools/src/definitions/{security_tools.rs,mod.rs}`、`backend/crates/golish-agent-kit/src/tool_definitions/config.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/{tool_list.rs,tool_execution/direct/mod.rs}`、`backend/crates/golish-agent-runtime/src/execution_mode/{prompt_render.rs,selection_apply.rs}`、上述模块卡、`agent-progress.md`。
- **下一步建议**：下一刀再改 prompts/methodology：每个 specialist stage 开局必须 `stage_worklist_status`，每轮最多取 `stage_worklist_next(limit=N)` 的具体 work items，只有返回 `ready_to_submit=true` 才允许 `submit_stage_deliverable`。前端/trace 可再把 worklist status 渲染成“本阶段还剩多少格”。

### 2026-07-01 · Enumeration 四轴覆盖 + IP Web 资产入分母

- **本轮目标**：接用户“PR-1 应该搞完了，接着搞”的枚举阶段问题：PR-1 四轴（JS / DIR / PARAM / JSAPI）已基本落在当前树里，本轮补 PR-2——只有 IP 没域名但 IP 是 Web 资产时，也要进入 JS 收集/API/目录/参数枚举分母。
- **已完成**：
  - `coverage_truth.rs` 新增 `GOLISH-ENUM-JS` DB truth（`js_analysis_results` 有行）与 `web_capable_ip_assets`（in-scope IP/CIDR 且 `targets.http_status IS NOT NULL`）。
  - `StageSpec` 增加 `enum_ip_web_coverage`，`resources/harness/stages/enumeration/spec.json` 开启该开关并声明四个 expected techniques。
  - `technique_resolver` / `GateContextBuilder` / `rule_engine` / `org_gate` / `TaskOrchestrator` 全链路传 `web_capable_assets`：裸 IP 仍 not_applicable，EAS/httpx 证明为 Web 的 IP/CIDR 才要求 JS/DIR/PARAM/JSAPI 四轴。
  - `golish-agent-app` coverage read-model 与 agent `check_stage_asset_coverage` 同口径过滤 enumeration worklist：EAS live domain/url + web-capable IP，且 UI cell 顺序为 JS → Directory → Parameter → API。
  - `StageAssetCoveragePanel` 修正 `JS` / `JSAPI(API)` key 解析，避免 `GOLISH-ENUM-JSAPI` 被误归到 JS 列；补四轴渲染/运行中维度测试。
  - 顺手修掉 `golish-agent-app/src/ai/commands/harness_dev.rs` 测试 helper 的 clippy `field_reassign_with_default` 阻塞（此前 feature_list 已记录它挡住 agent-app clippy）。
  - 同步模块卡：`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-kit/{harness,task_orchestrator}.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/frontend/components.md`；新增 `feature_list.json` 条目 `enumeration-four-axis-ip-web-2026-07-01`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1，失败点在 `just install` → `pnpm install --silent`（依赖安装失败，未进入项目 check/test）。
  - `cargo fmt -p golish-db -p golish-agent-kit -p golish-agent-app --check`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-db coverage_truth --status-level fail`（cwd `backend`）→ 30 passed / 104 skipped。
  - `cargo nextest run -p golish-agent-kit technique_resolver org_gate rule_engine stage_spec context_builder --status-level fail`（cwd `backend`）→ 161 passed / 616 skipped。
  - `cargo nextest run -p golish-agent-app stage_coverage --status-level fail`（cwd `backend`）→ 38 passed / 94 skipped。
  - `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 22 passed。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0，No fixes applied。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --check -- agent-progress.md feature_list.json docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-app/ai.md docs/modules/frontend/components.md` → exit 0。
- **未跑 / 未完成**：`just precommit` 未跑（`./init.sh` 已在 pnpm install 阶段失败）；未重启 app 做真实 enumeration run / run_tree DB 核对；未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/coverage_truth.rs`、`backend/crates/golish-agent-kit/src/{db_traits/repo.rs,harness/{stage_spec.rs,technique_resolver.rs,org_gate.rs,gate/{context_builder.rs,mod.rs,rule_engine.rs}},task_orchestrator/subtask_phases/execute.rs}`、`backend/crates/golish-agent-app/src/ai/{commands/{stage_coverage.rs,harness_dev.rs},db_bridge/{mod.rs,recon.rs}}`、`resources/harness/stages/enumeration/spec.json`、`frontend/components/Engagement/StageAssetCoveragePanel.{tsx,test.tsx}`、上述模块卡、`feature_list.json`、`agent-progress.md`。
- **风险 / 下一步**：解决本机 pnpm install/approve-builds 门后跑全量 `just precommit`；重启 app 后重跑/续跑 enumeration，用 `scripts/run_tree.py --workspace <ws> --db` 和 StageAssetCoveragePanel 核对同一组资产上四列是否一致；重点看只有 IP 且 `http_status` 非空的目标是否出现 JS/DIR/PARAM/API pending/found，而非 Web IP 是否仍为 not_applicable。

### 2026-07-01 · 工具结果摘要改成 Key Findings

- **本轮目标**：回应用户截图反馈：`JS Signals` / `AI Pass` / framework/rule/AI call 详情太复杂，默认展示只需要“找到了什么 / 发现了什么 / 落库了什么”。
- **已完成**：
  - `frontend/components/ToolAiTraceSummary.tsx`：把原来的多块摘要（Browser Collection、Route Probe、JS Signals、AI Pass、Static Analysis Hints）收敛成单张 `Key Findings` 卡。
  - 默认摘要只展示结果型信息：runtime API、保存 JS 数、verified paths、JS API、参数、secrets、落库数、AI 新增端点/recipe 轮数；不再默认渲染 frameworks/libraries/rule_matches、AI request/response、candidate file/line hint 这类调试视角明细。
  - 具体 finding 行只列少量 API endpoint / verified path；详细 rule/framework/AI dialogue 仍留在 raw JSON，方便需要时追证据。
  - `frontend/components/ToolAiTraceSummary.test.ts`：重写为结果视角回归，覆盖 JS extraction、route probe found/empty、browser collection、隐藏 JS signals、AI pass 合并进 Key Findings。
  - `docs/modules/frontend/components.md`：同步模块卡，明确默认摘要只做 `Key Findings`，不再把 `JS Signals` / `AI Pass` 做成默认 UI 块。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts` → Checked 2 files，最终 No fixes applied，exit 0。
  - `./node_modules/.bin/vitest run frontend/components/ToolAiTraceSummary.test.ts` → 8 tests passed，exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit` / 全量 `just test-fe`（本轮为 scoped UI summary；当前工作树已有多处未提交改动）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`frontend/components/ToolAiTraceSummary.tsx`、`frontend/components/ToolAiTraceSummary.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：如果截图里还觉得英文文案太工程化，可以继续把 `Key Findings` 内部句子中文化/更短，例如直接显示“发现 4 个 API，2 个已落库；目录未发现有效路径”。

### 2026-07-01 · direct bridge tools 不再被 sub-agent 300s timeout 杀掉

- **本轮目标**：回应用户截图中 `Using Js Extract Apis` 300.1s 后报 `Sub-agent tool 'js_extract_apis' timed out after 300s` 的问题；确认这不是工具内 timeout，而是 sub-agent 外层 timeout drop 了 direct Rust 工具 future。
- **根因判断**：
  - `golish-sub-agents/src/executor/response_parsing.rs` 原先对 registry/router 工具统一套 `tokio::time::timeout(tool_timeout, ...)`；到点后 future 被 drop，`js_extract_apis` / `browser_collect_js_api` / `route_probe_paths` 会在还没产出最终结果和 DB truth 前被杀。
  - shell/pentest 命令已有上层 `golish-app-core::background_jobs` 软超时转后台路径；direct Rust bridge tools 尚未接入同一 background manager。由于 `golish-sub-agents` 是 L2，不能直接依赖 L5 `golish-app-core`，真后台化需要后续通过 `ToolProvider`/上层 runtime 注入适配 seam。
- **已完成**:
  - `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`：新增 `use_sub_agent_outer_tool_timeout`，对 `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` 关闭 sub-agent 外层 timeout；普通工具仍保留 timeout，取消信号仍生效。
  - 新增回归测试 `long_direct_bridge_tools_bypass_sub_agent_outer_timeout`，钉住三类长耗时 direct bridge tools 不被外层 timeout drop，`query_target_data` / `pentest_run` 仍走原 timeout 分支。
  - `docs/modules/backend/golish-sub-agents.md`：同步记录 direct bridge tools timeout 边界与后台 job 架构边界。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-sub-agents --check`（cwd `backend`）→ 初次发现 import 换行 diff，随后 `cargo fmt -p golish-sub-agents` 修复，再跑 `cargo fmt -p golish-sub-agents --check` → exit 0。
  - `cargo nextest run -p golish-sub-agents long_direct_bridge_tools_bypass_sub_agent_outer_timeout --status-level fail`（cwd `backend`）→ 1 passed / 111 skipped，exit 0。
- **未跑**：`just precommit` / 全量 `cargo nextest run -p golish-sub-agents` / 真实 enumeration 重跑。定向测试等待过一个已有 `cargo run --no-default-features` 编译锁，未杀用户进程。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`docs/modules/backend/golish-sub-agents.md`、`agent-progress.md`。
- **下一步建议**：这次修的是“不要 timeout 直接杀工具”。如果要做到 UI 里真正显示 `backgrounded/job_id` 并让 AI 后续 `wait_for_background_jobs`，下一步要给 direct bridge tool execution 加一个上层注入的 background adapter，复用现有 background job 语义，同时保持 sub-agents 的 L2 分层。

### 2026-07-01 · 工具结果人话摘要第一刀

- **本轮目标**：回应用户反馈“每个工具 output 太复杂太乱，看不懂”；先从已复现的 JS extract / route probe / browser collect 结果入手，在详情页上方增加可读聚合摘要，原始 JSON 继续保留用于证据追溯。
- **已完成**：
  - `frontend/components/ToolAiTraceSummary.tsx`：新增 Browser Collection、Route Probe、JS Signals 三类摘要 section。Browser 聚合 scripts/API/persisted/skipped；Route 聚合 outcome/requests/candidates/baselines/matches/rejected/errors/queue；JS Signals 按 name/group 聚合 frameworks/libraries/rule_matches，把重复的 `Webpack` chunk 明细折成 `Framework · Webpack · N hits · N files · conf ...`。
  - `frontend/components/ToolAiTraceSummary.test.ts`：新增 JS Signals 聚合与 Route Probe 摘要回归；现有 AI Pass / Static Analysis Hints 行为继续覆盖。
  - `docs/modules/frontend/components.md`：同步记录 `ToolAiTraceSummary` 现在承担工具结果的人话聚合层。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts` → Checked 2 files, Fixed 1 file，exit 0。
  - `./node_modules/.bin/vitest run frontend/components/ToolAiTraceSummary.test.ts` → 11 tests passed，exit 0。
- **验证注意**：直接跑 `pnpm exec biome ...` / `pnpm test:run ...` 时，本机 pnpm wrapper 先自动 install，被 `[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts: @swc/core/electron/esbuild` 卡住，命令未进入 Biome/Vitest；因此本轮改用已存在的 `node_modules/.bin` 工具做 scoped 验证。
- **未跑**：`just precommit` / 全量 `pnpm check` / 全量 `pnpm test:run`（本轮为 scoped UI summary；当前工作树已有大量非本轮未提交改动，且 pnpm wrapper 当前会被 approve-builds 状态阻塞）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`frontend/components/ToolAiTraceSummary.tsx`、`frontend/components/ToolAiTraceSummary.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：继续按工具补同一类人话 section，优先 `js_extract_apis` 的 endpoints/params/secrets 风险分组、`pentest_run` 的 stdout/stderr 摘要、stage coverage/gate 的 pending/found/error 中文映射。

### 2026-07-01 · route_probe_paths 默认速率提升

- **本轮目标**：回应用户对最新 enumeration run 的截图追问：`route_probe_paths` 已在吐 progress，但默认 `5/s` 对完整内置字典 + 递归队列过慢；确认 JS 是否还需要跟着修。
- **判断**：
  - JS 核心提取路径当前不因这张 route 截图需要修：最新 run 已走 HAE-style 分类、内部 AI review，并把 `api_endpoints.params` 写入本地 DB。另有 audit 行用 `PentestAudit::started` 写 completed 摘要的证据语义问题，需单独按 evidence ledger 契约修，不和本轮速率改混在一起。
  - `route_probe_paths` 代码确认默认 `DEFAULT_RATE_LIMIT_PER_SEC=5` 且显式传参被夹到最大 50；截图里的 `queued=7k+` 与完整字典/递归队列一致，不是卡死。
  - live run 复核：`scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --db` 仍停在 `m.moresec.cn` 的 route 调用；`run.log` 曾出现 `Sub-agent tool 'route_probe_paths' timed out after 300s`。当前源码已有 `tool_name != "route_probe_paths"` 的外层 timeout 放行，说明运行中的后端仍需重启才吃到最近几轮 route 修复。
- **已完成**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`：默认速率从 `5/s` 提到 `50/s`；新增 `MAX_RATE_LIMIT_PER_SEC=100`，显式传 `rate_limit_per_sec` 最高可到 100；工具 schema 描述同步。
  - `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：同步记录 route probe 默认 `50/s`、上限 `100`、仍受 per-request timeout 约束。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app route_probe_paths --status-level fail`（cwd `backend`）→ 9 passed / 123 skipped，exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（本轮为 scoped route probe 调参；当前工作树已有大量非本轮未提交改动）。未对真实目标发起新的 HTTP 重跑。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **下一步建议**：重启 backend/app 后新 run 才会吃到默认 `50/s`。如果真实站点响应延迟仍让有效吞吐上不去，再单独做并发 worker/pipeline；那是比本轮“速率默认/上限”更大的行为改动。

### 2026-07-01 · HAE-style JS signal 分类层扩展

- **本轮目标**：按用户要求先参考 `/Users/christopherzheng/Downloads/HaE-main`，把 JS extract 的正则基础前面补成 HAE-style 规则分组/分类层，再让 AI 基于分类命中做复核。
- **已完成**：
  - 复核 HaE 网络版规则结构：`group -> rule[]`，规则字段包括 `name/f_regex/s_regex/format/color/scope/engine/sensitive`；匹配流程按 request/response/header/body scope 取内容，命中后 dedupe/persist。
  - `backend/crates/golish-js-analyzer/src/signals.rs`：`RuleMatchCandidate` 增加兼容字段 `color` / `scope` / `severity`，新增 `RuleMatchSeverity` 并从 crate root re-export；旧 JSON 缺字段时默认可反序列化。
  - `resources/js-analysis/js-signal-rules.yml`：补入 HAE-inspired 规则：Shiro、Ueditor、Druid、PDF.js、Win.ini、Chinese IDCard、MAC、Windows path、Mobile Number Field、Userinfo In Link、All URL、Create Script、Router Push、302 Location、OSKeys 等；保持 Rust regex 兼容，不直接复制 Java DFA/NFA/二段 regex 语义。
  - `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`：AI review summary 改为突出 HAE-style `group/kind/severity/scope/color/source_rule`，medium/high 规则命中优先进入上下文分类；规则命中仍只是候选，不直接落 API、不直接生成 finding。
  - 同步模块卡：`docs/modules/backend/golish-js-analyzer.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-js-analyzer -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `cargo test -p golish-js-analyzer embedded_signal_rules_parse_and_compile`（cwd `backend`）→ 1 passed / 46 filtered out。
  - `cargo test -p golish-js-analyzer`（cwd `backend`）→ 47 lib tests + 1 bin test + 3 integration tests passed。
  - `cargo test -p golish-pentest-app js_extract_apis::tests`（cwd `backend`）→ 24 passed / 108 filtered out。
  - `cargo check -p golish-js-analyzer -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-js-analyzer/src/lib.rs backend/crates/golish-js-analyzer/src/signals.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs resources/js-analysis/js-signal-rules.yml docs/modules/backend/golish-js-analyzer.md docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（本轮为 scoped JS analyzer / bridge 修复；当前工作树已有大量非本轮未提交改动）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-js-analyzer/src/{lib.rs,signals.rs}`、`resources/js-analysis/js-signal-rules.yml`、`backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`、`docs/modules/backend/golish-js-analyzer.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **下一步建议**：重启/重跑 enumeration 后看 `js_analysis_results.raw.rule_matches` 是否出现 HAE-style `group/color/scope/severity`；前端可再按 severity/group 做筛选展示。后续若误报多，再基于真实 run 调低 `ai_review` 或 confidence，而不是扩大 API endpoint 落库口径。

### 2026-07-01 · enumeration JS/API evidence 落库修复

- **本轮目标**：回应用户对最新 enumeration run 的追问，修复已用日志/DB 确认的两个真实问题：audit evidence 因 `project_path=NULL` 无法落库、`js_extract_apis` 把外域 endpoint 投影成当前 target API。
- **根因证据**：
  - 最新 run 的 `api_endpoints` / `js_analysis_results` 已有 post-reset rows，但 `audit_log` 对 `js_extract_apis` / `browser_collect_js_api` / `route_probe_paths` 为 0；复现 insert 失败为 `audit_log.project_path` NOT NULL 约束。
  - `js_extract_apis::resolve_endpoint_for_api_table` 原本接受 `//api.example.com/...` / `https://metrics.example.net/...` 这类外域 URL，并绑定到当前 `target_id`。
- **已完成**：
  - `backend/crates/golish-db/src/repo/audit/mod.rs`：新增 `audit_project_path(None) -> ""`，所有 audit insert (`log` / `log_operation_with_lineage` / `log_evidence`) 统一避免显式 NULL。
  - `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`：`api_endpoints` 投影新增 same-origin 检查（scheme/host/port 一致）；JSAPI outcome 不再仅因 raw endpoint 被抽到就标 `found`，只有真实持久化/重复业务行才是 `found`。
  - 同步模块卡：`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-db -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `cargo test -p golish-db audit_project_path_defaults_to_empty_string`（cwd `backend`）→ 1 passed / 130 filtered out。
  - `cargo test -p golish-pentest-app js_extract_apis::tests`（cwd `backend`）→ 24 passed / 108 filtered out。
  - `cargo check -p golish-db -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-db/src/repo/audit/mod.rs backend/crates/golish-db/src/repo/audit/tests.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（本轮为 scoped 后端修复；当前工作树已有大量非本轮未提交改动）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/audit/mod.rs`、`backend/crates/golish-db/src/repo/audit/tests.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑/续跑 enumeration；新 run 里 JS/API audit rows 应正常出现，外域统计/SDK URL 不应再进入当前 target 的 `api_endpoints`。`route_probe_paths` 对 `www.moresec.cn` 本次是完整跑完但 verified matches=0，所以 DB `directory_entries=0` 是结果为空；`m.moresec.cn` 那次 transcript 只有 request 没 result，需重跑验证是否仍卡住。

### 2026-06-30 · route_probe_paths 使用用户提供的大字典

- **本轮目标**：用户提供 1964 行路径清单（附件 `pasted-text.txt`），要求 route probe 默认使用这份，而不是 47 条小 fallback。
- **已完成**：
  - `resources/wordlists/route_probe_1.txt` 替换为用户提供的 1964 行清单；默认 `include_str!(".../route_probe_1.txt")` 继续生效，`route_probe_paths` 未传 `wordlist_path` / workspace `1.txt` 时会加载它。
  - `.gitignore` 增加 `!resources/wordlists/route_probe_1.txt`，避免这个内置运行依赖继续被 `resources/wordlists/*` 忽略。
  - `normalize_wordlist_entry` 改为保留 query string：`/zabbix.php?action=problem.view&ddreset=1` 会作为带参路径探测；仍过滤空行、整行注释、`..`、空白/control char，fragment 仍不参与 HTTP 探测。
  - 同步 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`，记录内置字典来源、规模与 query 保留规则。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app route_probe --status-level fail --no-fail-fast`（cwd `backend`）→ 9 passed / 120 skipped。
  - `git diff --check -- .gitignore backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs resources/wordlists/route_probe_1.txt` → exit 0。
- **未跑**：`just precommit` / 真实目标 route probe 重跑（会发真实 HTTP 请求）。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`.gitignore`、`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`resources/wordlists/route_probe_1.txt`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。

### 2026-06-30 · route_probe_paths DIR 闭环与提示词修正

- **本轮目标**：回应用户对最后一次 enumeration 实跑的质疑：`route_probe_paths` 不应靠 AI 一层层记 `1.txt`/observed paths；DIR 应在 JS/API 落库后每个 live web root 调一次，由工具自己读 DB seed、跑完整本地/内置字典递归队列、先过滤软 404/统一页，再把 verified/auth-wall 入库。
- **实跑复核（不看代码先看 transcript）**：最新 workspace `/Users/christopherzheng/golish-platform/Test1` 没有 `1.txt`；最新 session `pentest-chat-1782791610659-1` 内共 13 个 `route_probe_paths` result，合计 `requests_sent=878`、`persisted_directory_entries=55`（transcript 自报），但所有 result 都是 `wordlist.path=null` / `entries_loaded=0`，且多数 request 显式带 `max_requests=100/200/300`；最新 retry 未再跑 DIR，gate 仍报大量 `GOLISH-ENUM-DIR` missing。`psql`/`sqlx` CLI 本机不可用，未直接 SQL 查表。
- **已完成**：
  - `route_probe_paths` 改为 DB-backed seed：默认按 `target_id` 读取 `api_endpoints` + 既有 `directory_entries`，`observed_paths` 只作为额外补充；结果新增 `seed_paths`。
  - `route_probe_paths` 对外 schema 删除 `max_requests` / `max_wordlist_entries`；执行时也忽略这两个 key，始终跑完整生成队列和完整去重字典；递归默认深度 3，最多 6；结果用 `queue_completed` 表达队列是否跑空。
  - 新增内置 `resources/wordlists/route_probe_1.txt`；字典读取顺序：inline entries + explicit `wordlist_path`，否则 workspace `1.txt`，否则内置 fallback；不再因为 workspace 没有 `1.txt` 就空跑。
  - 入库顺序改成：HTTP 探测队列跑完 → baseline 过滤 soft-404/uniform → `matches` 统一写 `directory_entries(tool='route_probe')` → upsert `technique_outcomes(GOLISH-ENUM-DIR)`。
  - `route_probe_paths` 增加 live `tool_output_chunk`：start / progress / recurse / complete，避免慢扫时 UI 只显示黑盒 “Using Route Probe Paths”。
  - `golish-sub-agents` 的 direct tool 外层 timeout 对 `route_probe_paths` 放行：不再用 Enumerator 300s idle timeout 杀该工具；仍保留取消信号，以及工具内部 per-request `timeout_ms` 防止单个 HTTP 请求挂死。
  - 更新 enumeration stage methodology/spec、Enumerator prompt、coverage gap hints、`pentest_bridge` 模块卡，明确：JS/API 先落库；DIR 每个 root 一次 `target_id + base_url`；默认完整跑队列和字典；用 `queue_completed=true` 判断这次工具调用跑闭环。
- **AI 边界**：DIR found/empty/error 不交给 LLM 判断；AI 可读过滤后的摘要做解释，但不能把 `rejected_candidates` 提升为 found。JS/API 的 LLM 只在 `browser_collect_js_api` / `js_extract_apis` 内建 pass，仍有锚定/脱敏/降级护栏。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --check` → exit 0。
  - `cargo check -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit` → exit 0。
  - `cargo nextest run -p golish-pentest-app route_probe --status-level fail --no-fail-fast` → 9 passed / 120 skipped。
  - `cargo clippy -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --all-targets -- -D warnings` → exit 0。
- **未跑**：全量 `just precommit`；未对真实外部目标重跑 route probe（会发真实 HTTP 请求，需要用户明确授权）。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`resources/wordlists/route_probe_1.txt`、`resources/harness/stages/enumeration/{methodology.md,spec.json}`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。

### 2026-06-30 · JS/API 内建 AI 工具 P4 可观测性（MCP-agent-5 接手 MCP-agent-4 半成品）

- **本轮目标**：上一会话（MCP-agent-4）做 P4 可观测性时**中途断线**，转交上下文给本会话（MCP-agent-5）。用户要求「看看还差什么」并接手完成。P4 = (a) 两工具 AI pass 加 `tool_output_chunk` 即时进度；(b) detail 渲染 `ai_used`/`ai_endpoints_added`/`ai_recipe_rounds`/`rationale` 成「AI Pass」摘要块。
- **盘点 / 判断（先读码核实，不靠记忆）**：P4 三块代码其实**已写入**——`js_extract_apis.rs` 的 `js_extract_progress` + `ai_pass*` 进度行（含 `summary.ai_used`/`summary.ai_endpoints_added`）；`browser_collect_js_api.rs` 的 `ai_recipe round/skip/needs_second_pass` 进度行（含顶层 `ai_recipe_rounds`/`ai_recipe_rationale`）；前端 `ToolAiTraceSummary.tsx::buildAiPassSection`（「AI Pass」section，chips=ai used / +N ai endpoints / N recipe rounds + Recipe rationale 样本）。`cargo check` 四 crate exit 0（编译没断坏）。
- **真正的缺口（"搞了一半"）**：① 前端 `ToolAiTraceSummary.test.ts` 对新「AI Pass」section **零测试覆盖**；② `ToolAiTraceSummary.tsx` **从未跑 biome 格式化** → `pnpm check` 失败（两处换行/逗号）；③ 整个 P4 **从没跑过任何验证**（clippy/nextest/前端 check/test）。
- **已完成（接手收尾）**：
  - 前端 `ToolAiTraceSummary.test.ts` 补 **5 个** vitest 用例：js_extract（ai_used + endpoints）、browser（recipe rounds + rationale 样本）、单数 `1 recipe round` 文案、AI Pass 排在 Static Analysis 之前、纯确定性 run 不渲染。
  - `biome check --write` 格式化 `ToolAiTraceSummary.tsx`（仅此一文件）。
  - `feature_list.json`：verification 加前端 P4 项；evidence/notes 记上 P4（含缺口与修复、本轮验证证据）。
- **运行过的验证（实跑，本轮）**：
  - `cargo check -p golish-pentest-app -p golish-js-analyzer -p golish-agent-app -p golish-app-core` → exit 0。
  - `cargo clippy -p golish-pentest-app -p golish-js-analyzer --all-targets -- -D warnings` → exit 0。
  - `cargo nextest -p golish-pentest-app js_ai_extract js_ai_recipe extract_json parse_ai browser_collect_js_api js_extract --status-level fail` → **50 passed / 76 skipped**。
  - `pnpm test:run`（前端全量）→ **1508 passed / 12 skipped**（`ToolAiTraceSummary.test.ts` **7** tests）。
  - `pnpm check`（biome）clean；`pnpm typecheck` clean。
- **未跑**：全量 `just precommit`（受既有未追踪 `harness_dev.rs:644` lint 阻塞 `golish-agent-app` clippy，非本 scope）；活体 app run（minified SPA 验 AI-pass 即时进度行 + 「AI Pass」块真渲染）。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/ToolAiTraceSummary.tsx`（biome 格式化）、`frontend/components/ToolAiTraceSummary.test.ts`（+5 用例）、`feature_list.json`、`agent-progress.md`。其余 P0–P4 后端/前端改动沿用 MCP-agent-4 未提交工作树。
- **风险 / 下一步**：① P4 代码 + 验证已绿，可与 P0–P3 一并 commit（待用户点头）；② 既有 `harness_dev.rs` lint 非本 scope，需另处理才能让 `just precommit` 全量绿；③ 真正触发 AI 需 `~/.golish/settings.toml [ai.deepseek].api_key` 字面 key；④ 重启 app 做活体确认即时进度行与「AI Pass」detail 块。

### 2026-06-30 · JS/API 两工具内建 AI（AI-A 收集补全 + AI-B 提取确认）P0–P2 落地

- **本轮目标**：用户要把原型脚本（`js_api_ai_recipe_probe.mjs` 的 AI 生 recipe、`js_api_pipeline_test.mjs --ai-filter` 的 AI 过滤）那两段自动 AI **焊进** `browser_collect_js_api` 与 `js_extract_apis` 两个生产工具，使其自给自足、不靠外层大 agent，且守住 I7/I8（证据确定性可追溯）。
- **判断 / 根因**：现状两工具纯确定性，AI 只在外层 agent / 离线脚本；`2026-06-09-js-api-extraction-ai-augmented.md` 的 AI-B hybrid 设计写过但未落地，且未覆盖 AI-A（收集补全）。vehicle 实查：`golish_llm_providers::LlmClient::one_shot_completion` + `create_client_for_model(AiProvider::Deepseek,…)`（配置在 `~/.golish/settings.toml [ai.deepseek]`，不在 DB），两工具只持 `Arc<PgPool>`，须在 `register_pentest_tools` 注入 LLM handle。
- **已完成（设计→计划→TDD 实作 P0–P2 + P3 文档）**：
  - 设计 `docs/design/2026-06-30-jsapi-ai-tools-design.md`、计划 `docs/superpowers/plans/2026-06-30-jsapi-ai-tools.md`（用户已审「设计 OK」「出实作计画」）。
  - **P0**：新 `golish-app-core::ports::llm::LlmOneShot` 端口（保持 pentest-app 不直依赖 LLM crate）；`golish-agent-app::ai::llm_one_shot::SettingsLlmOneShot`（固定 DeepSeek）；`create_bridge_tools` 端口 + 组合根 `golish/pentest_tool_factory.rs` + `create_pentest_bridge_tools` 全链加 `llm` 参数，注入两工具 struct；共用 `pentest_bridge/ai_oneshot.rs`（`call_llm_json` timeout+降级、`extract_json_object`）。
  - **P1（AI-B 提取）**：`golish-js-analyzer` `Endpoint` 加 `source: EndpointSource{Regex,Ai}`（`#[serde(default)]` 向后兼容，8 个 regex 建构点标 Regex）；`pentest_bridge/js_ai_extract.rs` 纯函数（触发闸/切片/幻觉护栏 ai_path_anchored/merge_dedup）；接入 `js_extract_apis::execute`（触发命中文件→切片→one-shot→`parse_ai_endpoint`(锚回才收,source=Ai)+`parse_ai_param_hint`(param 须见于源码)→merge_dedup→provenance `ai_added`/`ai_used`/`ai_endpoints_added`；预算 8 文件/256KB）。
  - **P2（AI-A 收集）**：`pentest_bridge/js_ai_recipe.rs` 纯函数（needs_more/compact_signals/sanitize_recipe 同源+上限/recipe_has_work）；重构 `browser_collect_js_api::execute` 抽出 `run_collector_once`（返回 `Parsed{result,stderr}`/`Terminal`）+ 有界 AI 循环（最多 3 轮，needs_more→one-shot 生 recipe→sanitize→带 recipe 复跑，超时保留上一好结果）；provenance `ai_recipe_rounds`/`ai_recipe_rationale`。`.mjs` 不改（仍二次校验同源）。
  - **P3（文档）**：更新 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`（新增「内建 AI pass」专条、修订旧「不是内部 LLM 调用」措辞、关键文件/依赖补 LlmOneShot）与 `golish-js-analyzer.md`（Endpoint.source/EndpointSource）；feature_list 加 `jsapi-ai-tools-2026-06-30`（in_progress）；加降级单测 `call_llm_json(&None)`（**未跑**）。
- **运行过的验证（实跑）**：
  - `cargo check -p golish-app-core -p golish-agent-app -p golish-pentest-app` → exit 0；`cargo check -p golish` → exit 0。
  - `cargo clippy -p golish-app-core -p golish-js-analyzer -p golish-pentest-app --all-targets -- -D warnings` → exit 0。
  - `cargo nextest -p golish-js-analyzer` → 50 passed；`cargo nextest -p golish-pentest-app`（全量）→ 122 passed（含 js_ai_extract/js_ai_recipe/extract_json/parse_ai/browser/js_extract）。
- **未跑**：`just precommit`（用户本轮要求「不要跑测试」）；P3 新增的 `call_llm_json(&None)` 降级单测未执行；`cargo clippy -p golish-agent-app` 因**未追踪的既有文件** `harness_dev.rs:644 field_reassign_with_default`（非本功能代码）阻塞，其余我新增代码 clippy 全绿。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-app-core/src/ports/{mod.rs,llm/mod.rs,llm/one_shot.rs,pentest/tools.rs}`、`golish-agent-app/src/ai/{mod.rs,llm_one_shot.rs,commands/bridge_config.rs}`、`golish/src/pentest_tool_factory.rs`、`golish-pentest-app/src/pentest_bridge/{mod.rs,ai_oneshot.rs,js_ai_extract.rs,js_ai_recipe.rs,js_extract_apis.rs,browser_collect_js_api.rs}`、`golish-js-analyzer/src/{lib.rs,patterns.rs,lib_tests.rs}`、`docs/design/2026-06-30-jsapi-ai-tools-design.md`、`docs/superpowers/plans/2026-06-30-jsapi-ai-tools.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-js-analyzer.md`、`feature_list.json`、`agent-progress.md`。
- **风险 / 下一步**：① 跑全量 `just precommit` + 降级单测；② `ai` 入参默认 true，但要真正触发需 `~/.golish/settings.toml [ai.deepseek].api_key` 为字面 key（`$DEEPSEEK_API_KEY` 不会被 ai.deepseek 自动展开）；③ 重启 app 做活体：对一个 minified SPA 确认 AI-B 召回 > 纯 regex、AI-A 把 closure 补全；④ 既有 `harness_dev.rs` lint 非本 scope，需另行处理才能让 agent-app 全量 clippy 绿；⑤ 确认无误后再 commit。

### 2026-06-30 · TargetPanel `www` 展示去重与 EAS IP 分母折叠

- **本轮目标**：回应用户截图里 `115.28.135.55` 下 `moresec.cn` / `www.moresec.cn` 同时出现、以及 EAS 覆盖表把域名别名和端口端点当分母的问题。
- **根因 / 判断**：
  - `target-panel/asset-groups.ts` 只按 `real_ip` 分组，没有在 IP 组子列表里做 `www.<apex>` 展示折叠，所以 apex 与 `www` 别名会重复显示。
  - `stage_coverage.rs` 的 EAS summary 仍是“除 organization 外所有 target 都计入分母”，没有利用 `targets.real_ip` / URL host 判断 domain/url/`http://IP:port/path` 已归属于某个 direct IP target；因此这些别名会继续显示 LIVE/PORT/SERVICE pending 并撑大 done/total/pending。
- **已完成**：
  - `frontend/lib/target-panel/asset-groups.ts`：IP 组的 `linkedTargets` 展示层折叠 `www.<apex>` 到 `<apex>`，优先显示 apex；`m.` / `api.` 等真实子域不折叠，底层 `targets` 与计数仍保留原始资产。
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：EAS 建 direct IP target 集；domain/url/端口 URL 若通过 `real_ip` 或 URL host 解析到已有 direct IP target，则 coverage cells 全部为 `not_applicable`，并且不计入 `done/total/pending/new` 分母；没有 direct IP target 时保持保守，继续作为直接覆盖资产。
  - `backend/crates/golish-app-core/src/domain/targets.rs`：`rank_attack_surface_seeds` 在 Prober handoff 前折叠解析到已有 direct IP target 的 domain/url/`http://IP:port/path` 别名；排序时提升承接了别名的 direct IP，避免 cap 较小时把真正扫描主体挤掉。
  - 同步模块卡：`docs/modules/frontend/lib.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core/domain.md`。
- **运行过的验证（实跑）**:
  - `cd backend && cargo fmt -p golish-app-core -p golish-agent-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-app-core domain::targets --status-level fail` → 11 passed / 38 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 34 passed / 94 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-app-core -p golish-agent-app --lib -- -D warnings` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/target-panel/asset-groups.test.ts` → 4 tests passed，exit 0。
  - `./node_modules/.bin/biome check frontend/lib/target-panel/asset-groups.ts frontend/lib/target-panel/asset-groups.test.ts docs/modules/frontend/lib.md` → exit 0（实际 checked 2 TS files）。
  - `git diff --check -- frontend/lib/target-panel/asset-groups.ts frontend/lib/target-panel/asset-groups.test.ts backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-app-core/src/domain/targets.rs docs/modules/frontend/lib.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-app-core/domain.md agent-progress.md` → exit 0。
- **未跑**：`just precommit` 未跑；`pnpm exec vitest ...` 仍被本机 `ERR_PNPM_IGNORED_BUILDS` install gate 阻断，已改用仓库本地 vitest 二进制完成 scoped 前端测试。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/lib/target-panel/asset-groups.ts`、`frontend/lib/target-panel/asset-groups.test.ts`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-app-core/src/domain/targets.rs`、`docs/modules/frontend/lib.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core/domain.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启/刷新 app 后重新打开 TargetPanel 与 EAS coverage；预期 IP 组里只展示 `moresec.cn` 与 `m.moresec.cn`，不再额外展示 `www.moresec.cn`；`list_attack_surface_seeds` 主扫 worklist 也会优先返回 `115.28.135.55` 这类 direct IP，不再把解析到它的域名/端口 URL 当主扫 seed；EAS summary 分母只按 direct IP/host 扫描主体计算，解析到已有 IP 的域名和 `http://IP:port/path` 行只作解释子行。

### 2026-06-30 · target_intel landing `www` / apex 去重补齐

- **本轮目标**：回应用户追问“intel 里面没有去重吗？就是落库的时候”，确认并补齐 target_intel provider landing 写 `targets` 前的 `www.<apex>` / `<apex>` 去重。
- **根因 / 判断**：
  - DB 层 `targets` 只有 exact `value + project_path` 去重；`moresec.cn` 与 `www.moresec.cn` 是两个不同 value，会各自落成 target。
  - `asset_intel/landing.rs::plan_promotable_assets` 原本只用 `seen_hosts` 对完全相同 host 去重；provider 返回 apex 与 `www` 同 IP 时不会合并。
- **已完成**：
  - `backend/crates/golish-recon-app/src/asset_intel/landing.rs`：target promote planning 对同一个 `(strip_www(host), surveyed_real_ip)` 只保留一个 domain target，优先 apex；`m.` / `api.` 等真实子域不折叠；同一 exact host 重复仍保持 first-IP-wins。
  - `hostnames_from_certificates` 也走同一展示/landing alias 去重，避免 CT 同时给 apex 和 `www` 时重复 promote。
  - `land_service_assets` exact host 找不到时 fallback 到去 `www.` 的 apex target，避免 target 已折叠后 provider 端口/服务情报丢失。
  - 同步模块卡：`docs/modules/backend/golish-recon-app/asset_intel.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-recon-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-recon-app asset_intel::landing --status-level fail` → 11 passed / 203 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-recon-app --lib -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-recon-app/src/asset_intel/landing.rs docs/modules/backend/golish-recon-app/asset_intel.md` → exit 0。
- **未跑**：`just precommit` 未跑；当前仓库仍有大量前序 WIP。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-recon-app/src/asset_intel/landing.rs`、`docs/modules/backend/golish-recon-app/asset_intel.md`、`agent-progress.md`。
- **风险 / 下一步**：这是 future-proof landing 去重；已有历史重复 `targets` 行不会自动删除或合并。若要清理当前 DB 里的 `www.moresec.cn` 旧 row，需要单独做“合并/迁移 target 关联数据”的受控操作，不能直接删。

### 2026-06-30 · target_intel 覆盖分母与预检瘦身修复

- **本轮目标**：回应用户“intel 资产覆盖 DNS 为什么 pending / 卡很久，改吧”，修复 target_intel coverage read-model 把公司名和派生 `www.*` 当作待枚举资产、以及 `check_stage_asset_coverage(include_assets=true)` 把整张资产矩阵灌回 LLM 上下文的问题。
- **根因 / 判断**：
  - `ai_get_stage_asset_coverage` 的虚拟 organization row 被正常计入 asset summary，且因 `AssetClass::Other` fail-safe 保留 6 个 target_intel 维度，导致 `公司名 × DNS/Subdomain/CT` 被画成 pending；这类行应该只解释组织情报，不是域名资产。
  - `www.moresec.cn` 被底层 registrable-apex helper 视为 apex，SUBDOMAIN 维度继续适用，导致被动发现的 `www.*` 叶子主机继续扩大 coverage 分母。
  - agent-facing `check_stage_asset_coverage` 允许 `include_assets=true`，当前 run 中返回 139KB / 6 万 token 级矩阵，触发上下文截断并拖慢最后一轮 reasoning。
- **已完成**：
  - `stage_coverage.rs`：organization row 不再计入资产 summary；organization row 只保留 WHOIS/ASN/OSINT 组织情报维度，DNS/CT/Subdomain 固定 `not_applicable`；source_query unmatched target 只允许 WHOIS/ASN/OSINT 回卷到 organization row，避免公司名生成 `DNS/Subdomain` 缺口。
  - `technique_resolver.rs`：target_intel SUBDOMAIN 仍要求真实 registrable apex，但 `www.*` 主机即使底层 helper 视作 apex，也在 coverage 维度上按派生叶子处理为不适用。
  - `security.rs`：agent-facing `check_stage_asset_coverage` 即使传 `include_assets=true` 也不再返回完整 `assets`；只返回 `cell_summary`、`gap_examples`、`next_action` 与 omitted 计数提示。
  - 同步模块卡：`golish-agent-app/ai.md`、`golish-agent-kit/{harness,tool_executors}.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机依赖安装 gate），本轮继续 scoped Rust 验证。
  - `cd backend && cargo fmt -p golish-agent-app -p golish-agent-kit` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-kit/src/harness/technique_resolver.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-kit/tool_executors.md docs/modules/backend/golish-agent-kit/harness.md` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-app -p golish-agent-kit --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 31 passed / 94 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit technique_resolver --status-level fail` → 15 passed / 757 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit tool_executors::security --status-level fail` → 6 passed / 766 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-agent-kit --lib -- -D warnings` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-agent-kit --all-targets -- -D warnings` → exit 101；被 pre-existing/untracked `backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs:489` 的 `clippy::field-reassign-with-default` 阻断，非本轮修改文件，未改动用户 WIP。
- **未跑**：`just precommit` 未跑；`./init.sh` 仍在安装 gate 失败，且当前仓库已有大量前序未提交 WIP。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/{harness,tool_executors}.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后重新跑 target_intel；预期覆盖表不再出现 `公司名 × DNS/Subdomain/CT pending`，`www.* × Subdomain` 不再 pending，`check_stage_asset_coverage` 不再把完整 assets 矩阵塞给 LLM。若仍有某个具体域名 `DNS pending`，下一步按 `targets.id -> dns_records.target_id` 查是否确实落库/是否 freshness window 过滤。

### 2026-06-30 · Task profile legacy `task` 归一修复

- **本轮目标**：回应用户反馈“之前选了 Red Team，有时候打开只停在 Task，里面没有默认选中项”，定位并修复 execution mode / harness profile 恢复链路里的 profile 丢失。
- **根因 / 判断**：
  - UI 的真实模型是 Chat 引擎 + Task profile（`assessment` / `pentest` / `red_team` 等）；裸 `task` 只是旧数据里的 engine alias，不是可选 profile。
  - `ExecutionModePicker` 旧逻辑在 `chatExecutionMode === "task"` 时把 `"task"` 当 active profile，并会写入 `golish.lastHarnessProfile`，污染掉原来的 `red_team` 记忆；因此按钮显示普通 `Task`，子菜单没有任何 profile 高亮。
  - restore / conversation activation / dev resume 路径仍可能把旧的 `executionMode: "task"` 直接灌回前端状态或后端 `set_execution_mode`，导致 profile 被降级。
- **已完成**：
  - `frontend/lib/ai/execution-mode.ts` 集中承载 execution-mode/profile helper；`normalizeExecutionModeId("task")` 会归一成 last profile，若没有则回退 `assessment`；localStorage 的 last execution mode 不再写裸 `task`。
  - `ExecutionModePicker` 遇到 legacy `task` 会显示并自修复成具体 profile，不再把 `task` 写进 last profile；`Task` row 点击仍优先复用 remembered profile。
  - `useChatModes`、`useChatSessionInit`、`conversationTerminalActivation`、`terminal-restore` 全部在写 store / backend 前归一 legacy `task`；`AIChatPanel` 现有 dev “继续/重跑当前阶段”按钮只在 Chat 时进入 Task，且进入具体 profile，不再把 `red_team` 降级为 `task`。
  - 新增/扩展回归测试：picker 自修复、last profile/localStorage 归一、conversation terminal activation、terminal restore。
  - 同步 `docs/modules/frontend/{components,lib}.md` 记录 execution mode/profile 约束。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate，沿用 scoped frontend 验证）。
  - `./node_modules/.bin/biome check --write frontend/lib/ai/execution-mode.ts frontend/components/AIChatPanel/executionModePicker.utils.ts frontend/components/AIChatPanel/ExecutionModePicker.tsx frontend/components/AIChatPanel/hooks/useChatModes.ts frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/conversationTerminalActivation.ts frontend/components/AIChatPanel/hooks/useChatSessionInit.ts frontend/lib/terminal-restore.ts frontend/components/AIChatPanel/executionModePicker.utils.test.ts frontend/components/AIChatPanel/ExecutionModePicker.test.tsx frontend/components/AIChatPanel/conversationTerminalActivation.test.ts frontend/lib/terminal-restore.test.ts docs/modules/frontend/components.md docs/modules/frontend/lib.md` → exit 0，fixed 3 files。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/executionModePicker.utils.test.ts frontend/components/AIChatPanel/ExecutionModePicker.test.tsx frontend/components/AIChatPanel/conversationTerminalActivation.test.ts frontend/lib/terminal-restore.test.ts` → 4 files / 25 tests passed，exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- <本轮 scope 文件>` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交改动，本轮按用户问题做 scoped frontend 修复与验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/lib/ai/execution-mode.ts`、`frontend/components/AIChatPanel/executionModePicker.utils.ts`、`frontend/components/AIChatPanel/ExecutionModePicker.tsx`、`frontend/components/AIChatPanel/hooks/useChatModes.ts`、`frontend/components/AIChatPanel/AIChatPanel.tsx`、`frontend/components/AIChatPanel/conversationTerminalActivation.ts`、`frontend/components/AIChatPanel/hooks/useChatSessionInit.ts`、`frontend/lib/terminal-restore.ts`、`frontend/components/AIChatPanel/executionModePicker.utils.test.ts`、`frontend/components/AIChatPanel/ExecutionModePicker.test.tsx`、`frontend/components/AIChatPanel/conversationTerminalActivation.test.ts`、`frontend/lib/terminal-restore.test.ts`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`agent-progress.md`。
- **风险 / 下一步**：需要刷新 dev 前端后确认截图里的 picker：如果旧状态是 `task` 且本机 last profile 是 `red_team`，按钮应直接显示 `Red Team` 且子菜单高亮；没有 last profile 时显示 `Security Assessment`，不再停在普通 `Task`。

### 2026-06-30 · route_probe_paths 软 404 / 统一错误页过滤

- **本轮目标**：按用户确认的枚举流程，先补 DIR 枚举最薄的一层：`route_probe_paths` 不能只看 200/401/403 就入库，必须识别软 404 / 统一错误页 / catch-all 页面，避免污染 `directory_entries` 和 `GOLISH-ENUM-DIR` coverage。
- **已完成**：
  - `route_probe_paths` 新增响应签名与验证：positive status 后会按 prefix 额外探测随机 baseline，比较 status、final URL、title、body SHA-256、template SHA-256、body length bucket。
  - 新增判定结果：`verified_positive` / `auth_wall` / `soft_404` / `uniform_response` / `ambiguous`；只有 `verified_positive` 和 `auth_wall` 会写入 `directory_entries(tool='route_probe')` 并触发 wordlist 递归。
  - 工具结果新增 `verify_responses`、`candidate_requests_sent`、`baseline_requests_sent`、`rejected_candidates`；软 404 / 统一错误页保留在 `rejected_candidates`，供 UI/日志/AI 解释但不作为 found。
  - 同步 `resources/harness/stages/enumeration/methodology.md`、Enumerator prompt、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`：明确 rejected candidates 不能由 AI 手工提升为 found。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate，沿用 scoped backend 验证）。
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app route_probe --status-level fail --no-fail-fast` → 8 passed / 93 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents test_enumerator --status-level fail --no-fail-fast` → 2 passed / 109 skipped，exit 0。
  - `cd backend && cargo check -p golish-pentest-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs resources/harness/stages/enumeration/methodology.md docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交改动，且 `./init.sh` 仍在 pnpm install gate 阻塞。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后重跑 enumeration；预期 `route_probe_paths` 对统一 200 错误页只返回 `rejected_candidates`，不会落 `directory_entries`，DIR coverage 不会被这种假阳性补成 found。下一步可继续补 API 参数 schema（需先确认是否允许 DB schema/migration）。

### 2026-06-30 · enumeration 默认移除 Arjun 参数爆破工具

- **本轮目标**：回应用户判断“Arjun 不需要，先把 json 删掉；它不是探针/JS 参数提取工具”，从默认枚举阶段移除 Arjun 工具入口，并清掉会继续诱导 AI 使用 Arjun 的运行提示。
- **判断 / 根因**：
  - Arjun 是主动隐藏参数爆破工具，语义不等于 browser/JS/crawler 已观察到的 parameter extraction；默认把 `GOLISH-ENUM-PARAM` pending 映射到 Arjun 会导致真实站点慢跑/超时，也偏离用户期望。
  - 只删 `resources/toolsconfig/arjun.json` 不够：enumerator prompt、coverage worklist、stage refiner、taxonomy 仍会继续推荐或允许它，模型会变成“调用已删除工具后报 not found”。
- **已完成**：
  - 删除 `resources/toolsconfig/arjun.json`。
  - `resources/harness/stages/enumeration/spec.json`：`allowed_tool_types` 去掉 `web/param`，保留 `recon/crawler` + `web/route-probe`；`GOLISH-ENUM-PARAM` 仍保留为 coverage 维度，但来源改为 browser/js_extract/crawler 已观察到的请求、query、form 与 `param_hints`。
  - `resources/harness/stages/enumeration/methodology.md`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`：枚举 PARAM repair/next_action/suggested_tool 改为 `browser_collect_js_api` / `js_extract_apis` / observed params，不再推荐 Arjun。
  - `backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`：`arjun` 不再归类为 allowed `web/param`；保留单测明确断言它不属于当前 taxonomy。
  - `backend/crates/golish-pentest-app/src/pentest_ai/run.rs`：移除 Arjun 专用 foreground-only 特判和对外 tool schema 文案，`pentest_run` 回到普通工具执行契约。
  - 清理 command_builder/output_store/模块卡里的 Arjun 示例文案，避免后续 agent 按旧卡片恢复它。
- **运行过的验证（实跑）**：
  - `rustfmt --edition 2021 <本轮 touched Rust files>` → exit 0。
  - `rg -n "arjun|Arjun" backend/crates/golish-agent-kit/src backend/crates/golish-sub-agents/src backend/crates/golish-pentest-app/src backend/crates/golish-pentest/src resources/harness docs/modules/backend resources/toolsconfig` → 只剩 `tool_taxonomy.rs` 两处否定性单测引用。
  - `git diff --check -- <本轮 scope 文件>` → exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-sub-agents -p golish-pentest-app -p golish-pentest` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit tool_taxonomy --status-level fail` → 20 passed / 752 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents test_enumerator_has_content_enum_tools --status-level fail` → 1 passed / 110 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-pentest command_builder --status-level fail` → 16 passed / 143 skipped，exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交改动，本轮做用户点名的 scoped 工具移除与后端定向验证。此前本轮 `./init.sh` 已在 `just install` / `pnpm install --silent` 处失败。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`resources/toolsconfig/arjun.json`（删除）、`resources/harness/stages/enumeration/{spec.json,methodology.md}`、`backend/crates/golish-agent-kit/src/{harness/tool_taxonomy.rs,task_orchestrator/stage_refiner.rs,tool_executors/security.rs}`、`backend/crates/golish-sub-agents/src/{defaults/builder/mod.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs,executor_types.rs}`、`backend/crates/golish-pentest-app/src/{pentest_ai/run.rs,pentest/packages/install/runtime.rs}`、`backend/crates/golish-pentest/src/{command_builder/tests.rs,output_store/mod.rs,output_store/endpoints.rs}`、`docs/modules/backend/{golish-agent-kit/harness.md,golish-sub-agents.md,golish-pentest/output_store.md,golish-pentest/command_builder.md,golish-app-core.md,golish-pentest-app/pentest_ai.md}`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后再跑 enumeration；预期工具列表不再出现 Arjun，coverage gap 对 PARAM 的建议会回到 JS/browser/crawler observed params。旧 transcript / 历史设计文档里的 Arjun 文字未改，作为历史记录保留。

### 2026-06-30 · 0.zone / Quake 资产归一化边界修复

- **本轮目标**：按用户给的 0.zone domain API 文档修正 Asset Intel provider 请求与归一化边界，避免 0.zone `site/code` 搜索命中、Quake `hostname` / PTR 噪声被提升成扫描资产，解决默安科技 run 中美好教育/网安通/PTR 等脏资产进入 enumeration 的根因。
- **根因/判断**：
  - `resources/intel-providers/0-zone.json` 之前用 `form` 发送，runtime 会强制 `application/x-www-form-urlencoded`；0.zone 文档要求 `Content-Type: application/json` + JSON body。
  - 0.zone latest raw 中 `query_type=domain` 为 `total=0`，`query_type=org` 返回正确官网 `www.moresec.cn`，但 `query_type=site/code` 返回的是 broad search 命中；旧配置把 `site/code` 的 `url/ip/asn` 同样写入 `organizations.domains/ip_ranges/asns` 和 target candidates。
  - Quake 请求语法本身是 JSON，但旧配置把 `hostname` 写入 `domains` / target / host-IP pair；现场记录中 `hostname=mail.bimlmvcg.cfd`、`hebei.22.121.in-addr.arpa` 属于 PTR/rDNS 噪声，真正资产在 `domain` 或 `service.http.host`。
- **已完成**：
  - `resources/intel-providers/0-zone.json`：所有请求改为 JSON body + `Content-Type: application/json`；停用 broad `site/code` 请求；保留 `domain/org/email/apk/member`；新增 domain-keyed 精确请求 `domain_root`，仅用于 `root_domain=={{domain}}`。
  - `backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs`：HTTP provider 与 native provider 一样按 survey mode gating；普通公司名模式跳过含 `{{domain}}` 的请求，domain-keyed 模式只跑含 `{{domain}}` 的请求。
  - `resources/intel-providers/0-zone.json` normalize：0.zone 不再把 top-level `url/ip/ip_addr/asn` 写入 scope-driving fields，不再从 APK title/app id 生成 target；domain/root_domain 才能产生 target/domains。
  - `resources/intel-providers/quake.json` + `asset_intel/landing.rs`：Quake `hostname` 不再作为 owned domain / target / host-IP pair；服务落库优先 `domain` / `service.http.host`，防止 PTR/rDNS 抢占真实 HTTP Host。
  - 补回归测试：0.zone JSON body + request set、Quake hostname 不提升、HTTP request domain-mode gating、Quake service host 优先级；同步 `docs/modules/backend/golish-recon-app/asset_intel.md`。
- **运行过的验证（实跑）**:
  - `python3 -m json.tool resources/intel-providers/0-zone.json >/dev/null && python3 -m json.tool resources/intel-providers/quake.json >/dev/null` → exit 0。
  - `cd backend && cargo fmt -p golish-recon-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-recon-app request_applies_gates_http_requests_by_domain_mode zone_config_uses_json_body_and_owner_semantic_requests quake_config_does_not_promote_hostname_as_asset_owner service_assets_prefer_http_host_over_quake_hostname_noise --status-level fail` → 4 passed / 206 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-recon-app asset_intel --status-level fail` → 85 passed / 125 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-recon-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- resources/intel-providers/0-zone.json resources/intel-providers/quake.json backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs backend/crates/golish-recon-app/src/asset_intel/landing.rs backend/crates/golish-recon-app/src/asset_intel/tests.rs docs/modules/backend/golish-recon-app/asset_intel.md` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交改动，本轮按用户要求做 asset_intel scoped 修复与验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`resources/intel-providers/0-zone.json`、`resources/intel-providers/quake.json`、`backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs`、`backend/crates/golish-recon-app/src/asset_intel/landing.rs`、`backend/crates/golish-recon-app/src/asset_intel/tests.rs`、`docs/modules/backend/golish-recon-app/asset_intel.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后重新跑 Asset Intel / target_intel；旧 Test1 DB 中已写入的脏 `organizations.domains` / `targets` 不会被代码自动删除，若要复用旧 workspace 需要单独清理或 reset stage 后重跑。

### 2026-06-30 · Target IP related-domain 点击退回主页修复

- **本轮目标**：回应用户反馈 Target 页面里“点下面有 domain 的 IP 会跳到主页，其他 IP 不会”，定位并修复 related-domain IP workbench 崩溃。
- **日志证据 / 根因**：
  - `~/.golish/frontend.log` 在用户点击时多次记录 `[ErrorBoundary] Caught error: undefined is not an object (evaluating 'asset.assetType.toLowerCase')`，栈为 `buildSitemapItems -> TargetSurfaceWorkbench`。
  - 只有带 related domain 的 IP 更容易复现，是因为 `TargetSurfaceWorkbench` 合并当前 IP target 与 related domain target 的 `target_assets/api_endpoints/js_analysis_results/directory_entries` 后，拉到了后端 `target_assets` 原始 snake_case 行（`asset_type`），而前端接口声明和 `buildSitemapItems` 只读 camelCase `assetType`。
  - ErrorBoundary 自动恢复整棵 app，看起来像“跳回主页/主界面”，实际是 Target workbench render crash。
- **已完成**：
  - `frontend/lib/api/security-analysis.ts`：在 IPC wrapper 边界把 `target_assets`、`api_endpoints`、`fingerprints`、`js_analysis_results`、`passive_scan_logs`、`audit/timeline` 等后端 snake_case 行归一为前端 camelCase 视图模型，避免组件直接消费未规整 serde_json。
  - `frontend/components/TargetPanel/surface/surfaceModel.ts`：`buildSitemapItems` 兼容 `assetType` / `asset_type`，并跳过缺少 asset type 的坏行，防止单条旧数据炸掉整个 Target 页面。
  - 新增 `frontend/lib/api/security-analysis.test.ts`；补 `frontend/components/TargetPanel/surface/surfaceModel.test.ts` 回归；同步 `docs/modules/frontend/{lib,components}.md`。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/vitest run frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts` → 2 files / 12 tests passed，exit 0。
  - `./node_modules/.bin/biome check --write frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts` → exit 0（fixed 2 files）。
  - `./node_modules/.bin/biome check frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交 backend/frontend/docs 改动，本轮做 scoped frontend 修复与验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/lib/api/security-analysis.ts`、`frontend/lib/api/security-analysis.test.ts`、`frontend/components/TargetPanel/surface/surfaceModel.ts`、`frontend/components/TargetPanel/surface/surfaceModel.test.ts`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`agent-progress.md`。其中 `surfaceModel.ts/test.ts` 与模块卡已有前序未提交改动，本轮只追加本问题的边界归一与崩溃防护。
- **风险 / 下一步**：需要刷新 dev 前端后重新点击“下面有关联域名的 IP”；预期不再触发 ErrorBoundary，也不会退回主页，Sitemap/JS/API 继续显示合并后的 IP + related domain DB surface 数据。

### 2026-06-30 · JS/API params 落库合并与 Target 可见性补全

- **本轮目标**：回应用户确认“`api_endpoints.params` 正常应该存数据库，这些数据都应该落库”，补齐 JS/browser API endpoint 重复写入时参数不丢、以及 Target 侧 map/detail/surface 可见性。
- **判断 / 根因**：
  - `api_endpoints.params` 已是 DB 字段，`js_extract_apis` / `browser_collect_js_api` 都会从 URL query 解析参数；但部分普通写入路径遇到 `(target_id,url,method)` duplicate 时只计 duplicate/跳过，后续带参结果可能没有并入已存在 row。
  - 前端 `ApiEndpoint.params` 已从后端拉到，但 `JsApiTab` / `TargetDetail` 没渲染参数；`Sitemap/Paths` 只看 `directory_entries` / `target_assets`，没把已落库 endpoint path/url 纳入；Topology graph 只吃 `targets`/org/ports，看不到 JS/API/params/path summary。
- **已完成**：
  - `browser_collect_js_api.rs` 与 `js_extract_apis.rs`：API endpoint 落库统一走 `api_endpoints_upsert_merge_params`，同一 endpoint 后续发现 query/body/form params 时 union 到 `api_endpoints.params`，不再因 duplicate 丢参数证据。
  - `TargetSurfaceWorkbench` / `useTargetSurfaceData` / `surfaceModel`：Paths tab 合并 `api_endpoints.url/path`；surface summary 新增 params 指标；相关 JS/API/route/pentest 工具返回后自动 reload target surface 数据。
  - `JsApiTab` / `TargetDetail`：API endpoint 行展示 method、path/url、source/risk 和 DB params chips；JS 文件显示 endpoints/secrets/source map 信号。
  - `TargetGraphView` / topology model/inspector：Topology 从 DB 轻量拉 `api_endpoints`、`js_analysis_results`、`directory_entries` 摘要；target 节点、surface/evidence 节点、Inspector 和左侧 stats 显示 API / Params / Paths / JS 计数。
  - 新增 `frontend/components/TargetPanel/surface/{endpointParams.ts,EndpointParamChips.tsx}`，并同步 `docs/modules/frontend/components.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（既有本机 pnpm install gate）。
  - `./node_modules/.bin/biome check --write <TargetPanel surface/topology files>` → exit 0，fixed 6 files。
  - `cd backend && cargo fmt -p golish-pentest-app` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/topology/buildTopologyModel.test.ts` → 2 files / 14 tests passed，exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api js_extract_apis --status-level fail` → 26 passed / 75 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings` → exit 0。
  - `./node_modules/.bin/biome check <TargetPanel surface/topology files + docs>` → exit 0。
  - `git diff --check -- <本 scope 文件>` → exit 0。
- **未跑**：`just precommit` 未跑；全量仍受本机 pnpm install/ignored-build approval gate 和当前大工作树影响，本轮做 scoped 前后端验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs}`、`frontend/components/TargetPanel/{TargetDetail.tsx,TargetGraphView.tsx,TargetSurfaceWorkbench.tsx,hooks/useTargetSurfaceData.ts,surface/EndpointParamChips.tsx,surface/endpointParams.ts,surface/surfaceModel.ts,surface/surfaceModel.test.ts,surface/tabs/JsApiTab.tsx,surface/tabs/SurfaceTabView.tsx,topology/TopologyCanvas.tsx,topology/TopologyControls.tsx,topology/TopologyInspector.tsx,topology/buildTopologyModel.ts,topology/buildTopologyModel.test.ts,topology/types.ts}`、`docs/modules/frontend/components.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启/刷新 dev app 后重跑或打开已有 enumeration 结果核对：JS/API tab 与 Target detail 应显示 params chips，Sitemap/Paths 应显示 API endpoint URL/path，Topology 切 Surface/Evidence 后应出现 API/Params/Paths/JS summary。若目标量很大，Topology 当前按 target 拉轻量摘要，后续可再抽后端 bulk summary API 降 IPC 数。

---

### 2026-06-30 · Arjun 不再盲后台化

- **本轮目标**：回应用户指出 enumeration repair 里 `pentest_run(background=true, tool_name="Arjun")` 会在参数/输出未确认前直接转后台，导致 AI 看不到状态、stage closeout 被无输出 job 卡住。
- **日志证据 / 根因**：
  - 最新 run `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782738572850-1/`：00:08 左右 7 个 Arjun job 被 `background:true` 启动，`soft_ms=30000` 但 `hard_ms=1800000`；submit 被 `still_running=7` 连续拦截。
  - OS 层确认当时存在真实 `python3.11 .../bin/arjun` 进程；00:18:44 repair mode 才调用 `kill_job` 清掉旧 job，后续用 `-t 5 -T 15` 重跑才几秒完成。
  - 根因是 `background:true` 对所有工具同形态处理：短启动确认后直接返回 `job_id`，不区分 Arjun 这类必须先观察 stdout/stderr/timeout 的参数发现工具。
- **已完成**：
  - `backend/crates/golish-app-core/src/pty_interactive.rs`：新增 foreground-only 执行模式；超时会 kill 进程并返回当前 stdout/stderr/`status="timeout"`，不产生 background job handle，不参与 stage closeout background barrier。
  - `backend/crates/golish-pentest-app/src/pentest_ai/run.rs`：`Arjun` 强制走 foreground-only；即使模型传 `background:true`，返回也带 `execution_mode="foreground_only"` / `background_overridden=true`，让 AI 当前 tool call 内看到结果或超时。
  - `docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`：同步后台执行契约。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-app-core -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-app-core -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-app-core foreground_only_timeout_kills_instead_of_backgrounding background_command_that_survives_startup_returns_job_handle background_command_that_fails_during_startup_returns_inline_error --status-level fail`（cwd `backend`）→ 3 passed / 44 skipped。
  - `cargo nextest run -p golish-pentest-app arjun_requires_foreground_confirmation input_lines_become_stdin_payload input_file_placeholder_writes_target_file --status-level fail`（cwd `backend`）→ 3 passed / 98 skipped。
  - `cargo nextest run -p golish-app-core --status-level fail`（cwd `backend`）→ 47 passed。
  - `cargo nextest run -p golish-pentest-app --status-level fail`（cwd `backend`）→ 98 passed / 3 skipped。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交改动，且此前 `./init.sh` / pnpm install gate 已多次阻塞，本轮做 scoped backend 验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-app-core/src/pty_interactive.rs`、`backend/crates/golish-pentest-app/src/pentest_ai/run.rs`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后再续跑 enumeration；预期 Arjun 不再直接返回后台 job，旧式无 `-T` 的 Arjun 会在当前 tool call 内超时并返回可见 stdout/stderr，模型应缩窄参数后再跑。

---

### 2026-06-29 · pip Python 工具命令解析修复

- **本轮目标**：修复 `pentest_run(arjun)` 预检/运行时报 `Python script not found: ~/Library/Application Support/golish-platform/tools/arjun`，并确认这不是只改 `arjun.json` 的单点问题。
- **根因/判断**：
  - `resources/toolsconfig` 的现有约定是 pip 工具写 `runtime="python"` + `runtimeVersion="3.11"` + `install.method="pip"`；`arjun`、`netexec`、`sherlock`、`maigret` 等都是同形态，没有任何 JSON 写 `runtime="pip3.11"`。
  - `command_builder::build_run_command` 原先只按 `runtime` 分派；`runtime="python"` 被当成 app-managed Python script，去找 `tools/<executable>`，因此 `arjun` 被错误解析成 `tools/arjun`，而不是 conda env 里的 `python3.11_env/bin/arjun`。
- **已完成**：
  - `backend/crates/golish-pentest/src/command_builder/mod.rs`：`install.method == "pip"` 现在优先走 `build_pip_command`；`runtime_with_config_version` 对 `python` / `pip` 也会合并 `runtimeVersion`，避免落到 `latest`。
  - `backend/crates/golish-pentest/src/command_builder/tests.rs`：新增回归测试，固定 `runtime="python"` + `method="pip"` 的工具会解析到 conda env CLI，而不是 `tools_dir/arjun`。
  - `docs/modules/backend/golish-pentest/command_builder.md`：同步记录 pip 工具解析契约。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest command_builder --status-level fail` → 16 tests passed / 143 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-pentest --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest/src/command_builder/mod.rs backend/crates/golish-pentest/src/command_builder/tests.rs docs/modules/backend/golish-pentest/command_builder.md` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有大量前序未提交 frontend/backend/docs 改动，本轮做 scoped resolver 验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest/src/command_builder/mod.rs`、`backend/crates/golish-pentest/src/command_builder/tests.rs`、`docs/modules/backend/golish-pentest/command_builder.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后再跑一次 `pentest_run(tool_name="arjun", ...)` 或对应 stage repair；如果 conda env 里尚未安装 Arjun，下一步错误应变成 `Pip tool 'arjun' not installed in conda env 'python3.11_env'`，而不是再找 `tools/arjun`。

---

### 2026-06-29 · 直连 JS 工具 detail 动态 Output 可见性

- **本轮目标**：回应用户截图中 `Using Js Extract Apis` 展开后只看到静态 `Input`、看不到动态效果的问题；让 `js_extract_apis` / `browser_collect_js_api` 这类非 shell-like 直连工具在运行中也能显示实时 Output。
- **根因/判断**：
  - 后端/事件层已经把 direct bridge tools 的 `tool_output_chunk` 写进 `streamingOutput`；问题在前端 detail renderer。
  - `ToolCallDetailView` / `SubAgentDetailView` 之前只给 shell-like 工具（`run_command` / `run_pty_cmd` / `pentest_run` / wrapper）固定渲染 running Output 区；`js_extract_apis` 被当成普通结构化工具，运行中只显示 Input + 标题 spinner，直到最终 JSON result 出现。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：非 shell-like 工具在 `running` / `backgrounded` 时也显示 Output 区；有 `streamingOutput` 就实时追加，没有 chunk 时显示 `Waiting for output...` 和高对比 spinner；完成后再显示结构化 result / `ToolAiTraceSummary`。
  - `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`：主工具详情页同样支持非 shell-like running live Output，避免从 ChatPanel 点进详情时仍看不到动态。
  - 新增/扩展前端测试：`js_extract_apis` running sub-agent 展开后能看到 Output + pending placeholder；`getLiveOutputForDetail` / `getSubAgentLiveOutputForDetail` 覆盖 pending 与 streamed chunk。
  - 同步 `docs/modules/frontend/components.md`，记录 direct bridge tools 的动态应来自 `tool_output_chunk` 的 Output 区，不应只靠最终 JSON 摘要。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate，沿用本地 `node_modules/.bin` 做 scoped 验证）。
  - `./node_modules/.bin/biome check --write frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md` → exit 0（fixed 1 file）。
  - `./node_modules/.bin/biome check frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 61 tests passed，exit 0；stderr 仍有既有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md` → exit 0。
- **未跑**：`just precommit` 未跑；全量仍受 pnpm ignored-builds/install gate 和当前大工作树影响，本轮做 scoped frontend 验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`、`frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。这些文件已有前序未提交改动，本轮只在其上追加本问题的最小修复。
- **风险 / 下一步**：需要刷新 dev 前端后重新展开截图中的 `Using Js Extract Apis`；预期展开后在 Input 下方出现 Output 区，运行中先显示 `Waiting for output...`，随后实时追加 `[js_extract_apis] ...` progress 行。

---

### 2026-06-29 · JS 工具实时动态输出与 AI 文案澄清

- **本轮目标**：修复用户反馈的三个 JS 枚举体验问题：`browser_collect_js_api` / `js_extract_apis` 不能只在结束后一次性吐总结，前端必须实时看到工具正在看什么；确认这两个“AI Assist / AI Analysis”字段是否真的调用了 AI；超时不能表现成黑盒杀进程，而要让 agent/用户看到卡在哪一步。
- **判断 / 根因**：
  - 旧 `browser_collect_js_api` Rust wrapper 用 `command.output()` 等子进程结束后一次性拿 stdout/stderr；Node helper 的 stdout 是最终 JSON，stderr 没有被流式转发，所以 UI 只能看到最终结果摘要。
  - `js_extract_apis` 是同步静态分析工具，之前也只返回最终 JSON；没有实时告诉前端“读了几个 JS、分析到多少 endpoint、正在落库哪个文件”。
  - `ai_assist` / `ai_analysis` 都不是工具内部 LLM 调用：前者是 browser collector 给外层 agent 的 bounded recipe hints，后者是 static analyzer 给外层 agent 的 source_file/line-range review hints。真正调用 AI 的只有外层 agent 读这些 hints 后决定下一步。
  - 前端已有 `tool_output_chunk` → `streamingOutput` → Tool/SubAgent detail Output 的实时显示链路，缺的是 backend direct/bridge tools 没有拿到当前 tool card 的 output sender。
- **已完成**：
  - `golish-core::agent_session` 新增 `with_agent_tool_output_sender`、`current_agent_tool_output_sender`、`emit_current_agent_tool_output_chunk`，让普通 `Tool` 实现不用改 trait 也能发 `AiEvent::ToolOutputChunk`。
  - main-agent `single_tool_call` 和 sub-agent `response_parsing` 的 registry/router tool execution 都包上 output sender；因此主聊天工具卡和 stage 子 agent 详情页都能看到 bridge tool 的实时输出。
  - `browser_collect_js_api.rs` 改为 spawn Node helper 后并发读取 stdout/stderr：stdout 继续只承载最终 JSON，stderr 每行转发为 `tool_output_chunk`；Rust 外层 timeout 只作为 `hard_timeout_ms + 5s` fail-safe，超时返回已收集 stdout/stderr tail。
  - `scripts/browser_collect_js_api.mjs` 新增实时 progress：`start`、`launch_browser`、`browser_ready`、`goto`、`api_observed`、`script_saved`、`page_exercised`、`recursive_script_saved`、`close_browser`、`summary` 等写 stderr，前端 Output 能实时看到页面/JS/API/递归 chunk 动态。
  - `js_extract_apis.rs` 新增实时 progress：`start`、`loaded_sources`、`analysis_complete`、`persist_start`、前 40 个 `persisted_file`、`persist_skipped`、`summary`；静态分析和落库过程不再只有最终 JSON。
  - `ToolAiTraceSummary` 可见标题从 `AI Assist` / `AI Analysis` 改为 `Collector Hints` / `Static Analysis Hints`，图标去 Bot 化；模块卡同步说明这些字段不是内部 AI 调用，真正动态看 Output 的 `tool_output_chunk`。
  - 同步模块卡：`golish-core`、`golish-agent-runtime`、`golish-sub-agents`、`golish-pentest-app/pentest_bridge`、`frontend/components`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt --all --check` → exit 0。
  - `cd backend && cargo check -p golish-core -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-core tool_output_chunk_emits_inside_scope --status-level fail` → 1 passed / 206 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents response_parsing --status-level fail` → 23 passed / 88 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api --status-level fail` → 8 passed / 91 skipped，exit 0。
  - `node --check scripts/browser_collect_js_api.mjs` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/ToolAiTraceSummary.test.ts` → 2 tests passed，exit 0。
  - `git diff --check -- <本 scope 文件>` → exit 0。
- **未跑**：`just precommit` 未跑；当前工作树已有多轮未提交改动，本轮按用户“先修这几个”做 scoped 后端/前端验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-core/src/{agent_session.rs,lib.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs}`、`scripts/browser_collect_js_api.mjs`、`frontend/components/{ToolAiTraceSummary.tsx,ToolAiTraceSummary.test.ts}`、`docs/modules/backend/{golish-core.md,golish-agent-runtime.md,golish-sub-agents.md,golish-pentest-app/pentest_bridge.md}`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后重跑 enumeration 观察 UI：`browser_collect_js_api` / `js_extract_apis` 的 Tool/SubAgent detail Output 应实时出现 `[browser_collect_js_api] ...` 和 `[js_extract_apis] ...` 行；最终 JSON 里的 Collector/Static Analysis Hints 只是复核提示，不代表工具内部已经调用 AI。

---

### 2026-06-29 · TargetPanel IP/JS 详情与 stage_run repair 修复

- **本轮目标**：修复最新默安科技 run 后的四个问题：IP 下面挂 IP、JS/API 已落库但 Target 详情看不到、点击 target 退回主界面、取消/重跑后 repair 退化成泛化 BLOCK 导致精确工具清单丢失。
- **现场判断**：
  - 最新 run 为 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782738572850-1/`；DB 显示 `js_analysis_results` / `api_endpoints` / `directory_entries` 已有落库。
  - `targets` 中大量 `target_type=ip` 行同时带另一个 `real_ip`，前端按 `real_ip` 优先分组会把 IP target 挂到另一个 IP 下面。
  - `TargetSurfaceWorkbench` 只读当前 target_id，IP 聚合详情不会看到 related domain target 上的 JS/API/目录证据；同时直接访问 `target.ports`，后端 wire target 没带 ports 时会触发 runtime 报错。
  - enumeration 子 agent 曾拿到带 96 个 `coverage_gap_actions` 的 `submit_stage_deliverable needs_fix`，但随后 stage_run 在没有可验收 `StageDeliverable` 时把它折叠成 “sub-agent completed without accepted deliverable” 泛化 BLOCK；这会让 repair 丢失 target/technique 级工具清单，看起来像“到了 repair 没工具能调用”。
- **已完成**：
  - `frontend/lib/target-panel/asset-groups.ts`：IP target 现在按自身 `value` 成组；domain/url 才按 `real_ip` 挂到 IP 组，避免“IP 下面挂 IP”。
  - `frontend/components/TargetPanel/hooks/useTargetData.ts` 与 `TargetSurfaceWorkbench.tsx`：target / related domain 进入详情前统一兜底 `ports: []`，避免点击目标时因 `ports` undefined 报错。
  - `frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`：支持读取当前 target + related domain target ids，并合并 `target_assets`、`api_endpoints`、`js_analysis_results`、`directory_entries`、timeline/logs 等 surface 数据；IP/host 详情现在能看到相关域名上落库的 JS/API/目录内容。
  - 新增 `frontend/lib/target-panel/asset-groups.test.ts` 覆盖 IP target 带异源 `real_ip` 时仍归入自身 IP 组；同步 `docs/modules/frontend/{components,lib}.md`。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：stage_run 在 fallback/no-deliverable retry 路径保留 sub-agent observer 写下的 `SubmitRepairMode.coverage_gap_actions`，并优先把结构化 actions 传给 StageRefiner / retry checkpoint；取消或重跑后不会退化成无 actions 的泛化 repair。
  - 同步 `docs/modules/backend/golish-agent-runtime.md`，记录 `stage_run` / `sub_agent_call` 共享 per-org checkpoint 时必须保留 repair mode。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/lib/target-panel/asset-groups.ts frontend/lib/target-panel/asset-groups.test.ts frontend/components/TargetPanel/hooks/useTargetData.ts frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx docs/modules/frontend/components.md docs/modules/frontend/lib.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/target-panel/asset-groups.test.ts frontend/lib/target-panel/org-tree.test.ts` → 2 files / 12 tests passed，exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo fmt --all --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail --no-fail-fast` → 35 passed / 254 skipped，exit 0。
- **未跑**：`just precommit` 未跑；本轮为 scoped 修复，且当前工作树仍有大量前序未提交 backend/frontend/docs 改动。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/lib/target-panel/asset-groups.ts`、`frontend/lib/target-panel/asset-groups.test.ts`、`frontend/components/TargetPanel/hooks/useTargetData.ts`、`frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`、`frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`docs/modules/backend/golish-agent-runtime.md`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 dev 前端/后端后在 Target 面板点默安科技资产核对：IP 组不再嵌套 IP；点 IP/host 详情时 JS/API tab 能显示 related domain 的落库结果；点击 target 不再退回主界面。重新跑/续跑 enumeration 时，预期 repair checkpoint 保留具体 `coverage_gap_actions`，worker 只补 gate 点名的 target × technique。

### 2026-06-29 · 工具内部 AI 辅助结果前端可见性

- **本轮目标**：回应用户“前端看不到工具内部 AI 干了什么”的问题；让 `browser_collect_js_api` / `js_extract_apis` 这类工具结果里的 `ai_assist` / `ai_analysis` 不再埋在原始 JSON 里，而是在聊天工具卡和 detail 面板中可见。
- **判断 / 根因**：
  - 当前 transcript 能看到外层 `browser_collect_js_api` / `js_extract_apis` 的 tool request/result，但前端只把结果按 key/value 或 raw JSON 展示。
  - `browser_collect_js_api(ai_assist=true)` 当前产出的 `ai_assist` 是工具整理的辅助上下文 / recommended / reasons / next_step，不是一次额外 LLM API 调用；`js_extract_apis.ai_analysis` 是静态分析 handoff 摘要。它们都需要独立视觉块，否则用户只能翻 JSON。
- **已完成**：
  - 新增 `frontend/components/ToolAiTraceSummary.tsx`：识别 `ai_assist` / `ai_analysis`，渲染 AI Assist / AI Analysis 摘要块，展示 chips、reasons、next step、script observations、candidate files、line hints 和 samples。
  - `frontend/components/AIChatPanel/ToolCallSummary.tsx`：聊天消息里的工具展开结果在 raw key/value 前显示 AI 摘要。
  - `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`：主工具详情页在结构化结果前显示 AI 摘要。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：sub-agent 工具详情同样显示 AI 摘要，便于查看 `stage_run` 内部 Enumerator/Prober 工具。
  - 新增 `frontend/components/ToolAiTraceSummary.test.ts`，并同步 `docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**：
  - `pnpm exec vitest run frontend/components/ToolAiTraceSummary.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts` → exit 1；被本机 `ERR_PNPM_IGNORED_BUILDS` install/approval gate 阻塞。
  - `pnpm exec biome check ...` → exit 1；同样被 pnpm ignored-builds gate 阻塞。
  - `./node_modules/.bin/vitest run frontend/components/ToolAiTraceSummary.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts` → 3 files / 18 tests passed，exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `git diff --check -- frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
- **未跑**：`just precommit` / `just check-fe` 全量未跑；本机 pnpm approval gate 仍会阻塞 `pnpm exec` / install 路径，用户此前也要求避免 precommit。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`frontend/components/ToolAiTraceSummary.tsx`、`frontend/components/ToolAiTraceSummary.test.ts`、`frontend/components/AIChatPanel/ToolCallSummary.tsx`、`frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树已有其它未提交文件，本轮未回滚。
- **风险 / 下一步**：刷新前端后，展开 `browser_collect_js_api` / `js_extract_apis` 工具结果，或进入 `stage_run` 的 Enumerator sub-agent detail，应先看到 AI Assist / AI Analysis 摘要块，再看到原始 JSON。若后续要展示真正“工具内部调用 DeepSeek”的完整 request/response，需要后端或脚本额外发 `ai_call_trace` 一类事件/字段；本轮只把现有结果字段可视化。

---

### 2026-06-29 · SubAgent detail 全角 DSML 泄漏修复

- **本轮目标**：回应用户截图中 EAS Prober detail 在 submit 后把一大段 narrative 和 `<｜｜DSML｜｜tool_calls>` 原始工具帧显示出来的问题，确认原因并补 UI 清洗回归。
- **根因/判断**：
  - 具体 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782738572850-1/`；`run_tree.py --full --db` 显示 Prober 在 `route.moresec.cn` 的 PORT / SERVICE-FINGERPRINT coverage gap 后，submit 被 accepted，再输出了一段 full-width DSML 伪工具调用：`<｜｜DSML｜｜tool_calls> ... pentest_run naabu ...`。
  - 6 月 28 已有 `stripAgentXmlTags` 清理 ASCII DSML 的修复，但 matcher 只认 ASCII `|`；DeepSeek 这次吐的是全角 `｜`，所以 detail 渲染前没有剥掉。
  - 前半段很长是 `sub_agent_text_delta.accumulated` 的当前 response narrative，被 store 合并为一条正文；这本身是预期，真正不该显示的是内部 DSML tool-call 帧。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：DSML tag matcher 同时支持 ASCII `|` 和全角 `｜`，继续只作用于 `tool_calls` / `invoke` / `parameter` 伪标签剥离。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增 `route.moresec.cn` / `pentest_run naabu` 全角 DSML 泄漏回归，确保正文保留、内部工具帧不显示。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（既有本机 pnpm install gate）。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782738572850-1 --full --db | rg -n "route\\.moresec\\.cn|DSML|submit_stage_deliverable|coverage gap|accepted|naabu|Prober|external_attack_surface" -C 2` → exit 0；确认泄漏原文为全角 DSML。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 1 file / 48 tests passed；stderr 仍有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
- **未跑**：`just precommit` 未跑；全量仍会被本机 pnpm ignored-builds/install gate 拦住，本轮做 scoped 前端验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新/重启 dev 前端后重新打开该 detail；预期 DSML 工具帧不再显示，只剩 agent narrative 和正常工具调用卡。

---

### 2026-06-29 · JS 敏感候选 AI triage 脱敏与采样验证

- **本轮目标**：回应用户对“JS 里的敏感信息抓取 + AI 复核那块是否还没改/测试”的疑问，核对 `js_extract_apis` / `golish-js-analyzer` / `js_api_pipeline_test --ai-filter` 的真实链路，并补上缺失的脱敏与采样约束。
- **判断 / 根因**：
  - `js_extract_apis` 工具本身不直接调用外部 AI；它返回 `secret_candidates` / `config_candidates` / `rule_matches` / `ai_analysis`，让外层 agent 只读 source_file + line range 做局部复核。
  - 真正调用 DeepSeek/AI 做候选 triage 的是 `scripts/js_api_pipeline_test.mjs --ai-filter true`。旧逻辑会截断 endpoints/secrets/rules 给模型，但没有把“这是 sample”写进 payload/结果；另外 `js_api_extract` 的 `context_snippets` / `rule_matches.context` 可能在同一行旁路带出 raw Authorization/token/password。
- **已完成**：
  - `scripts/js_api_pipeline_test.mjs`：AI payload 新增 `sampling` 元数据（endpoints / secret_candidates / config_candidates / context_snippets / rule_matches 的 total/included/limit/truncated），AI 返回结果带 `input_sampling`；prompt 明确 AI 分类只适用于 sample，不能冒充 deterministic 全量统计；新增 `--skip-collection --js-dir`，可不启浏览器直接测试已有 JS 捕获目录的静态抽取 + AI triage。
  - `backend/crates/golish-js-analyzer/src/bin/js_api_extract.rs`：AI context snippets 输出前统一脱敏 access token / API key / password / Authorization / JWT / cloud key / `sk_live` 等敏感值；新增 snippet 脱敏测试。
  - `backend/crates/golish-js-analyzer/src/signals.rs`：`redacted_line_context` 不再只脱敏当前 regex 命中值，还会脱敏同一行邻近的 token/password/Authorization 等，避免 Linkfinder/interesting rule 的 context 泄露旁边 header；新增邻近敏感值脱敏测试。
  - 同步模块卡 `docs/modules/backend/golish-js-analyzer.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`，并修正 `docs/modules/backend/golish-sub-agents/defaults.md` 里残留的 fast/deep 旧描述。
- **运行过的验证（实跑）**：
  - `node --check scripts/js_api_pipeline_test.mjs` → exit 0。
  - `cd backend && cargo fmt -p golish-js-analyzer --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-js-analyzer redacts_sensitive_values_from_context_snippets rule_match_context_redacts_neighboring_sensitive_values signals --status-level fail` → 8 passed / 40 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app js_extract_apis --status-level fail` → 17 passed / 82 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-js-analyzer --all-targets -- -D warnings` → exit 0。
  - 本地 fake-AI smoke（仅 localhost fake DeepSeek，不调用外部 API）：手工 JS 捕获含伪 `accessToken` / `dbPassword` / `Authorization: Bearer ...` / internal URL；`js_api_pipeline_test --skip-collection --js-dir ... --ai-filter true` → exit 0，分析结果 `files_scanned=1`、`api_base_path=/admin-api`、`endpoints_total=1`、`secret_candidates_total=5`、`rule_matches_total=10`、`secret_triage_count=5`；fake AI 接收 payload 中 `leaked_raw_secret=false`、`leaked_raw_password=false`、`leaked_raw_bearer=false`，并看到 `sampling` 元数据。
- **未跑**：`just precommit` 未跑；本轮做 scoped Rust/Node 验证，未调真实外部 AI 服务。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`scripts/js_api_pipeline_test.mjs`、`backend/crates/golish-js-analyzer/src/{signals.rs,bin/js_api_extract.rs}`、`docs/modules/backend/{golish-js-analyzer.md,golish-pentest-app/pentest_bridge.md,golish-sub-agents/defaults.md}`、`agent-progress.md`。
- **风险 / 下一步**：真实 DeepSeek triage 尚未重跑（本轮刻意用 localhost fake AI 避免外部请求）；若要看模型真实分类质量，可在用户明确授权后用真实 key 跑同一 `--skip-collection --js-dir` smoke 或对最新 enumeration 捕获目录跑 sample triage。

---

### 2026-06-29 · enumeration DIR 内置小字典递归探测

- **本轮目标**：按用户澄清，枚举阶段目录发现不默认用外部 ffuf/gobuster，而是基于 JS/API 已发现路径去重、派生每一级 prefix，再用本机小字典（如 workspace `1.txt` 或显式 `wordlist_path`）做 bounded 递归探测。
- **已完成**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`：新增 `wordlist_path`、`wordlist_entries`、`max_wordlist_entries`、`wordlist_recursion_depth`、`include_curated_rules` 参数；相对 `wordlist_path` 按 workspace 解析，未传时尝试 `workspace/1.txt`；字典项会去重、过滤注释/空行/`..`/空白字符，默认最多 256 条。
  - `route_probe_paths` 现在把小字典叠到从 observed JS/API paths 派生出的每一级 parent prefix 上，并可从 positive wordlist hit 做 bounded recursion；所有请求仍受 `max_requests`、`rate_limit_per_sec`、`timeout_ms` 和 same-origin 约束。positive 仍以 absolute URL + `target_id` 写 `directory_entries(tool='route_probe')` 并 upsert `GOLISH-ENUM-DIR` outcome。
  - 枚举阶段 authorization 也收紧：`resources/harness/stages/enumeration/spec.json` 的 `allowed_tool_types` 从 `web/fuzzer` 改为 `web/route-probe`；`tool_taxonomy.rs` 把 `ffuf/gobuster/dirb/dirsearch/feroxbuster` 归为 `web/dir-fuzzer`，不再被 enumeration allow-list 解析出来。`route_probe_paths` 是 DIR 的唯一默认/允许路径；PARAM 的 `arjun/katana` 仍经 `pentest_run`。
  - 枚举阶段 prompt/refiner/preflight/methodology 改为默认 `route_probe_paths + observed JS/API prefixes + small local wordlist`；明确不要在 enumeration 调外部目录工具。同步模块卡 `docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-sub-agents.md`。
  - 本地 smoke：起本地 HTTP 服务，页面加载 `/static/app.js`，JS 里既有静态 route seed 也有 `fetch("/api/v1/users/list?team=red")`；`js_api_pipeline_test` 能保存 JS、捕获 2xx fetch，并把拼接路径作为 route seed 交给后续 route probe。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --check` → exit 0。
  - `jq empty resources/harness/stages/enumeration/spec.json` → exit 0。
  - `node --check scripts/browser_collect_js_api.mjs` → exit 0；`node --check scripts/js_api_pipeline_test.mjs` → exit 0；`node --check scripts/js_api_ai_recipe_probe.mjs` → exit 0。
  - `node scripts/js_api_pipeline_test.mjs --url http://127.0.0.1:54507/ --workspace /tmp/golish-js-route-probe-smoke --max-pages 1 --max-actions 0 --timeout-ms 10000 --hard-timeout-ms 30000 --endpoint-limit 100 --signal-limit 50 --context-limit 10` → exit 0；`scripts_saved=1`、`api_requests_total=1`、`status=200`、`rule_matches_total=4`。
  - `cd backend && cargo nextest run -p golish-pentest-app route_probe_paths --status-level fail` → 7 passed / 92 skipped，exit 0；包含本地 HTTP dry-run：从 observed `/api/v1/users/list` + wordlist `admin/health` 命中 `/api/admin` 并递归命中 `/api/admin/health`。
  - `cd backend && cargo nextest run -p golish-agent-kit allowed_tool_names_enumeration_selectors_include_direct_enum_tools category_lookup_known_and_aliases deny_by_default_for_unmatched --status-level fail` → 3 passed / 769 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair_allows_direct_enumeration_tools_for_listed_gap_targets test_enumerator_has_content_enum_tools test_enumerator_prompt_is_content_enum --status-level fail` → 3 passed / 108 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api --status-level fail` → 8 passed / 91 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本 scope 文件>` → exit 0。
- **未跑**：`just precommit` 未跑；本机全量仍受 pnpm ignored-builds gate/现有大工作树影响，本轮做 scoped JS/Rust 验证。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`resources/harness/stages/enumeration/spec.json`、`backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`backend/crates/golish-sub-agents/src/defaults/{builder/mod.rs,prompts/execution_planning.rs,tests.rs}`、`backend/crates/golish-sub-agents/src/{executor/response_parsing.rs,executor_types.rs}`、`backend/crates/golish-sub-agents/prompts/pentester.tera`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/{golish-agent-kit/harness.md,golish-pentest-app/pentest_bridge.md,golish-sub-agents.md}`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 backend/app 后真实跑一条 enumeration；预期 DIR gap 的下一步提示会让 Enumerator 调 `route_probe_paths`，如 workspace 有 `1.txt` 会自动加载小字典，否则可显式传 `wordlist_path`。本轮没有定位用户电脑上的 `1.txt` 具体路径，避免长时间扫全 home。

---

### 2026-06-29 · ChatPanel dev 继续/重跑入口收口

- **本轮目标**：按用户反馈，把测试用的“继续/重跑”入口放到 ChatPanel 输入栏，而不是资产覆盖详情；同时删掉资产覆盖面板里的两个 dev-only 按钮。
- **已完成**：
  - `frontend/components/AIChatPanel/AIChatPanel.tsx`：Task 模式下输入工具区新增 dev-only `RotateCcw` 按钮；点击后先按当前 roadmap 的第一个未通过阶段调用 `harness_dev_reset_stage_checkpoint(mode="restart_stage")` 清本阶段 checkpoint/repair 状态，再走 `useChatSend` 发出可见用户消息 `继续跑`，保持 conversation/user bubble/Task mode/streaming 链路一致。
  - `frontend/components/AIChatPanel/hooks/useChatSend.ts`：支持程序化 prompt override；不会误清用户输入草稿，也不会把图片附件带入 dev 继续消息。
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：移除资产覆盖 panel header 里的“清当前组织 repair 卡点 / 从此阶段重跑”两个 dev 按钮和 reset 调用入口；相关测试用例、前端 API wrapper、隐藏事件桥已撤掉。
  - `frontend/lib/api/harness-dev.ts`：新增 dev-only harness checkpoint reset wrapper，供 ChatPanel 按钮调用；不在 coverage/detail 组件暴露。
  - `backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`：`session_id` 现在同时支持 DB UUID 和 ChatPanel 的 `chat_session_key`（如 `pentest-chat-...`），避免前端按钮传真实 AI session 字符串时报 `invalid session_id`。
  - `docs/modules/frontend/components.md`：同步记录 ChatPanel 是 operation 继续/重跑提示的唯一发送入口，coverage/detail 组件不直接发 AI prompt。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；仍卡在 `just install` / `pnpm install --silent` 的本机 `ERR_PNPM_IGNORED_BUILDS` gate。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/hooks/useChatSend.ts frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/lib/api/stage-coverage.ts docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/AIChatPanel/hooks/useChatSend.test.ts` → 2 files / 22 tests passed，exit 0。
  - `cd backend && cargo fmt -p golish-agent-app -p golish --check` → exit 0。
  - `cd backend && cargo test -p golish-agent-app harness_dev --lib` → 3 passed，exit 0。
  - `cd backend && cargo check -p golish-agent-app -p golish` → exit 0。
  - `cd backend && cargo test -p golish-agent-app export_bindings -q` → 10 passed，exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit` 未跑；全量仍会被本机 pnpm ignored-builds approval gate 拦住。
- **提交记录**：未提交。
- **风险 / 下一步**：需要刷新 dev 前端/后端后确认 Task 模式输入栏中 Context ring 与图片按钮之间出现旋转箭头按钮；点击后应先用当前 chat session key 解析到 DB session，并把当前未通过阶段（例如 enumeration）从 repair/checkpoint 状态重置为 stage restart，再在 ChatPanel 里出现一条用户消息 `继续跑`，由后端 resume 当前 task operation，不退回 scoping。

---

### 2026-06-29 · browser_collect_js_api 失败噪声与 JSAPI 入库收紧

- **本轮目标**：回应用户截图中 `browser_collect_js_api` 结果里大量 404/502/403 类失败项，确认这些内容是否被错误当作 JS/API 收集结果，并收紧工具输出/入库口径。
- **根因/判断**：
  - 最新 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782643983045-1/`，当前在 `enumeration`，Enumerator 对根组织调用了 4 次 `browser_collect_js_api`。
  - 截图里的 `recursive_errors[100]` 对应 `life.pingan.com` 的递归 JS chunk 候选拉取失败：`api_requests_total=0`、`persisted_api_rows=0`、`jsapi_outcome=empty`；它不是 `api_endpoints` 本身。
  - DB 现场核对：最近 8 小时 `api_endpoints` 里没有 403/404/502 或 `{{version}}` 这类错误 URL；本次新落库只有 `b.pingan.com.cn` 5 条 2xx XHR/fetch 与 `www.pingan.com` 1 条 status=null XHR（修复后 status=null 不再入库/计 found）。
  - 真问题是工具结果把最多 100 条递归失败 URL 原样暴露给模型/UI，且 Rust 持久化旧逻辑只看 observed XHR/fetch，不区分响应状态与静态/页面资源，容易把失败/页面/静态资源误算为 JSAPI found。
  - 这不是 WAF：`life.pingan.com` 上 135 个 `./af.js` / `./ar-dz.js` 等引用来自 bundled webpack/moment locale context；把它们解析成 `https://cdn.life.pingan.com/.../af.js` 去拉，本身就是递归逻辑误判，真实 HTTP 结果是 404。
- **已完成**：
  - `scripts/browser_collect_js_api.mjs`：顶层 `recursive_errors` 改为最多 20 条 sample，新增 `recursive_errors_total`、`recursive_errors_by_status`、`recursive_errors_truncated`；`ai_assist.context.signals` 也带 status 汇总，避免 100 条 404/502 噪声淹没真实采集结果。
  - `scripts/browser_collect_js_api.mjs`：递归 JS ref 扫描收紧，非 `import()` 的 `./...js` / `../...js` 视为 bundle 内部模块 specifier，不再当外部 chunk 去 CDN 拉取；这些 refs 进入 `ai_review_refs` 供外层 AI 复核，不能自动污染 `scripts` / `recursive_errors`。
  - `scripts/browser_collect_js_api.mjs` 与 Rust bridge：移除 fast/deep 行为分叉，统一为 `crawl_mode=standard` 默认策略（12 pages / 12 actions / 1000 recursive scripts / 60s timeout）；`fast`/`deep` 仅兼容入参，不改变行为。sub-agent prompt、stage refiner hint、browser prompt 测试已同步，避免模型继续“先 fast 再 deep”。
  - 新增 `scripts/js_api_ai_recipe_probe.mjs`：先跑确定性 browser collector；只有存在 `ai_review_refs` 或显式 `--force-ai` 时才调 DeepSeek，让 AI 判断是否需要 bounded recipe 二跑；AI 不能直接声明 endpoint/JS 存在，二跑仍由 collector 真实 HTTP/浏览器验证。
  - `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`：`api_request_for_table` 只接受 2xx/3xx 的 HTTP(S) XHR/fetch，并过滤 `.js/.css/.html/.map/图片/字体/媒体/pdf/zip` 等静态或页面资源；403/404/502、status=null 不再写 `api_endpoints`，也不再让 `GOLISH-ENUM-JSAPI` outcome 变 found。
  - 同响应新增 `persistable_api_requests` / `skipped_api_requests`，区分浏览器原始观测和真正可入库 API 请求；同步模块卡 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` → exit 0；确认最新 session / stage / sub-agent 调用树与 DB 自诊断。
  - Python `psycopg2` 直连 `postgres://golish:golish_local@localhost:15432/golish` 查询 `api_endpoints` / `technique_outcomes` → 确认截图失败项未作为 API endpoint 入库，4 个目标 JSAPI outcome 分别为 bank empty、life empty、www found(旧 status=null)、b found。
  - `node --check scripts/browser_collect_js_api.mjs && node --check scripts/js_api_ai_recipe_probe.mjs && node --check scripts/js_api_pipeline_test.mjs` → exit 0。
  - Node 本地对照脚本读取 `/Users/christopherzheng/golish-platform/Test1/.golish/captures/life.pingan.com/443/js/ilifecore/pc-official-website/js/659a9a57_app.652c352b.js` → old_count=136、new_count=1，确认批量 locale false-positive 被过滤。
  - `node scripts/js_api_ai_recipe_probe.mjs --url https://life.pingan.com/ --workspace /tmp/golish-jsapi-ai-recipe-life-pingan --no-ai true` → exit 0；确定性抓取 `scripts_saved=12~15`（站点动态加载波动）、`api_requests_total=0`、`closure_complete=true`、`recursive_errors_total=9`，剩余错误均为真实 404 chunk/public-path 候选。
  - `DEEPSEEK_API_KEY=<redacted> node scripts/js_api_ai_recipe_probe.mjs --url https://life.pingan.com/ --workspace /tmp/golish-jsapi-ai-recipe-life-pingan` → exit 0；最终结果 `scripts_saved=15`、`recursive_errors_total=9`、`ai_review_refs_total=135`、AI `needs_second_pass=false`，rationale 明确 135 个相对 refs 是 bundle module specifier，9 个 `/dist/...` / `fullpage...` 候选已真实 404，不需要补抓。
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api --status-level fail` → 8 passed / 88 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents test_browser_prompt_prefers_browser_closure_collection --status-level fail` → 1 passed / 110 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本 scope 文件>` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`scripts/browser_collect_js_api.mjs`、`scripts/js_api_pipeline_test.mjs`、`scripts/js_api_ai_recipe_probe.mjs`、`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`、`backend/crates/golish-sub-agents/prompts/browser.tera`、`backend/crates/golish-sub-agents/src/defaults/prompts/{execution_planning.rs,orchestration.rs}`、`backend/crates/golish-sub-agents/src/defaults/tests.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 backend/app 后重新跑 enumeration 验证：预期 `life.pingan.com` 这类 100 个 404 只显示摘要且 JSAPI=empty；403/404/502/status=null 不再把 JSAPI 打成 found。历史已入库的 status=null/static/page `api_endpoints` 本轮没有删除，避免未授权清历史数据；如要修 UI/coverage 历史污染，需要单独做 DB repair。

---

### 2026-06-29 · 全量 checkpoint commit

- **本轮目标**：按用户“帮我commit一下现在所有的东西”要求，把当前工作树里已完成/已记录的 EAS、enumeration、coverage UI、ChatPanel 间距等混合改动作为一个 checkpoint 提交，避免后续继续开发时被大块未提交 diff 干扰。
- **提交范围**：当前 `git status` 中全部 tracked 修改 + untracked `docs/design/2026-06-29-enumeration-tool-boundary.md`；不做拆分提交，因为当前变更已经跨 backend/frontend/docs/resources 且多处互相关联。
- **运行过的验证（实跑）**：
  - `git diff --check` → exit 0。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --stat` / `git diff --name-status` → 已复核本次会被提交的文件范围。
- **未跑**：`just precommit` 本轮未重跑；前序会话已实跑并卡在本机 `pnpm approve-builds` / `ERR_PNPM_IGNORED_BUILDS` gate，用户随后明确“不要跑pre commit”，后续只做 scoped targeted checks。
- **提交记录**：本条随 checkpoint commit 一起提交；最终 hash 以 `git log -1 --oneline` 为准。
- **风险 / 下一步**：需要重启 backend/app 后分别实测 EAS legacy wave skip、enumeration 工具边界、JSAPI/DIR/PARAM repair direct tools、coverage UI 合并同 run evidence。仓库仍本地 ahead，未 push。

---

### 2026-06-29 · enumeration worklist / repair 工具边界修复

- **本轮目标**：按用户给出的 enumeration 北极星流程收紧当前实现：先消费 EAS 存活 web root，再做 browser/static JS 收集与落地，提取路径/敏感信息/依赖，第三方 JS 做版本记录，自有 JS 做 endpoint/route/secret 落库，最后基于 JS/route seed 做目录分层枚举；同时修复 repair 阶段把 CLI 工具当直接函数、直接枚举工具又被 repair lock 挡住的问题。
- **根因/判断**：
  - Enumerator 旧 prompt/默认工具仍从 `list_in_scope_targets` 起步，并暴露 `manage_targets`，会诱导它在 enumeration 阶段重新看泛资产/改资产状态，而不是只消费 EAS confirmed live web roots。
  - `pentest_list_tools` 输出没有明确说明返回的 `ffuf/katana/arjun/...` 是 `pentest_run.tool_name`，模型容易把 CLI 名字当 function call。
  - `SubmitRepairMode` 的 coverage-gap repair 对 enumeration 只允许 `pentest_run`/coverage/query 等工具，没放行 `browser_collect_js_api` / `js_collect` / `js_extract_apis` / `route_probe_paths`，导致 gate 点名 JSAPI/DIR/PARAM gap 后 worker 不能用最该用的 direct tools。
- **已完成**：
  - 新增 security bridge 工具 `list_enumeration_web_roots`：复用 `stage_asset_coverage` snapshot，返回 EAS-confirmed live web roots、pending/terminal techniques、suggested tools、执行顺序和 direct-vs-`pentest_run` 工具边界；加入 LLM schema 与 tool config allow-list。
  - Enumerator 默认工具改为 `list_enumeration_web_roots` + query/coverage/JS/route/pentest_run，不再暴露 `list_in_scope_targets` / `manage_targets`；prompt 改成 web-root worklist first，并明确 `browser_collect_js_api/js_collect/js_extract_apis/route_probe_paths` 直接调用，`ffuf/gobuster/feroxbuster/arjun/katana` 只能经 `pentest_run(tool_name=...)`。
  - `SubmitRepairMode` 对 structured enumeration `coverage_gap_actions` 自动放行 direct enum tools，并校验 `target_url`/`base_url` 必须落在 gate 点名 asset 内；越界 target 或缺少 target arg 会被 deterministic block。
  - `pentest_list_tools` 返回新增 `execution_contract`、`direct_ai_tools_note` 和每个工具的 `call_via: "pentest_run"`，减少模型把 CLI 名当 direct function。
  - `StageRefiner` 的 enumeration coverage repair 白名单拆出 direct enum tools；`GOLISH-ENUM-JSAPI` pending suggested_tools 补 `js_collect`。
  - 同步模块卡：`golish-agent-kit/tool_executors`、`golish-sub-agents`、`golish-sub-agents/executor`、`golish-tools`、`golish-agent-app/ai`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate，沿用 scoped Rust 验证）。
  - `cd backend && cargo fmt --all` → exit 0。（此前 `cargo fmt --manifest-path backend/Cargo.toml` 因 virtual workspace target 解析失败，换用 backend 内常规命令。）
  - `cd backend && cargo test -p golish-pentest-app pentest_ai::list_tools::tests::list_tools_exposes_params_and_batching_not_only_skills -- --nocapture` → 1 passed，exit 0。
  - `cd backend && cargo test -p golish-sub-agents defaults::tests::test_enumerator -- --nocapture` → first run exit 101（测试断言找 `pentest_run(tool_name=...)`，实际 prompt 为 `pentest_run(tool_name=..., args=...)`）；修断言后 rerun 2 passed，exit 0。
  - `cd backend && cargo test --target-dir /tmp/golish-codex-target -p golish-tools test_build_function_declarations_returns_all_tools -- --nocapture` → first run exit 101（新增工具后 declarations count 43→44）；修计数后 rerun 1 passed，exit 0。
  - `cd backend && cargo test --target-dir /tmp/golish-codex-target -p golish-agent-kit tool_executors::security::tests::enumeration_web_roots_worklist_returns_live_root_contract -- --nocapture` → 1 passed，exit 0。
  - `cd backend && cargo test --target-dir /tmp/golish-codex-target -p golish-sub-agents coverage_gap_repair_allows_direct_enumeration_tools_for_listed_gap_targets -- --nocapture` → 1 passed，exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-tools/src/definitions/{mod.rs,security_tools.rs}`、`backend/crates/golish-agent-kit/src/{tool_definitions/config.rs,tool_executors/security.rs,task_orchestrator/stage_refiner.rs}`、`backend/crates/golish-sub-agents/src/{defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs,executor/response_parsing.rs,executor_types.rs}`、`backend/crates/golish-pentest-app/src/pentest_ai/list_tools.rs`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、相关模块卡、`agent-progress.md`。当前工作树仍有大量上一轮/其它未提交文件，本轮未回滚。
- **风险 / 下一步**：需要重启 backend/app 后跑一条 enumeration 实测，确认 worker 第一调用变为 `list_enumeration_web_roots`，repair 中 JSAPI/DIR/PARAM gap 能走 direct enum tools，CLI 枚举工具只经 `pentest_run`。`just precommit` 未跑；本轮 full gate 受 `./init.sh` pnpm install gate 阻塞。

### 2026-06-29 · enumeration 工具边界与第三方 endpoint 污染修复

- **本轮目标**：回应用户截图中 enumeration 阶段 `browser_collect_js_api` EACCES、fallback 到 `whatweb` 指纹识别、以及“数据库没内容吗”的疑问；修复 enumeration 阶段不该跑 EAS 指纹工具，以及 crawler/endpoint 输出把第三方域名提升成当前 org target 的问题。
- **根因/判断**：
  - `browser_collect_js_api` 截图里的 EACCES 是 helper 执行路径/旧进程表现；当前源码已是 `node scripts/browser_collect_js_api.mjs` 启动，重启后不会依赖脚本执行位。`scripts/browser_collect_js_api.mjs` 本机文件 mode 仍是 `-rw-r--r--`，若旧构建直接 spawn 脚本会被 macOS 拒绝。
  - DB 不是没内容：当前 Ping An operation `5d03ba91-52f6-42ac-9bba-2699633d483e` 已在 `enumeration`，root org 下 EAS outcome 有 LIVENESS found 171、PORT found 51、SERVICE-FINGERPRINT found 240；enumeration 自身 `GOLISH-ENUM-JSAPI` 因 browser helper 失败落 error 4 次，`GOLISH-ENUM-DIR` empty 3 次，`directory_entries=0`。
  - `enumeration/spec.json` 仍允许 `recon/http`，而 `whatweb` 属于 `recon/http`，所以 browser 工具失败后模型 fallback 到 service fingerprint 仍被 stage whitelist 放行。
  - `output_store::endpoint_add` 会对任意绝对 URL 调 `find_or_create_target_scoped`；katana/crawler 输出第三方 URL 时，会把 `hm.baidu.com` / `www.googletagmanager.com` / `open.weixin.qq.com` 等写成当前 org 的 `active_discovered` target，污染后续分母。
- **已完成**：
  - `resources/harness/stages/enumeration/spec.json`：移除 `recon/http`，enumeration 只允许 `recon/crawler`、`web/fuzzer`、`web/param`；HTTP 探活/服务指纹留在 EAS。
  - `tool_taxonomy.rs` 测试收紧：明确 `browser_collect_js_api` / `js_collect` / `js_extract_apis` / `route_probe_paths` 可用于 enumeration，同时断言 `httpx` / `whatweb` / `curl` / `wget` / `nmap` / `naabu` 不可见、不可跑。
  - `output_store::fields_with_command_target` 给 `endpoint_add` 提取命令里的 `-u` / `--url` base host；`endpoints::store_endpoint` 在有 base host 时只落同 host endpoint，第三方 URL 直接跳过，不再创建 scoped target。
  - 新增设计记录 `docs/design/2026-06-29-enumeration-tool-boundary.md`；同步模块卡 `golish-agent-kit/harness.md`、`golish-pentest/output_store.md`。
- **运行过的验证（实跑）**：
  - `jq empty resources/harness/stages/enumeration/spec.json` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-pentest -p golish-sub-agents --check` → first run exit 1（rustfmt 排版差异）；`cargo fmt ...` 后 rerun exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit allowed_tool_names_enumeration_selectors_include_direct_enum_tools --status-level fail` → 1 passed / 770 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-pentest endpoint --status-level fail` → 8 passed / 149 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit tool_taxonomy --status-level fail` → 20 passed / 751 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents test_enumerator --status-level fail` → 2 passed / 108 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-pentest -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-pentest -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `just precommit` → exit 1；用户随后明确“不要跑pre commit”，后续不再跑。失败点仍是 `fmt-fe`，展开 `pnpm biome format ...` 后为 `[ERR_PNPM_IGNORED_BUILDS] @swc/core@1.15.21, electron@23.3.13, esbuild@0.25.12`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`resources/harness/stages/enumeration/spec.json`、`backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`、`backend/crates/golish-pentest/src/output_store/{mod.rs,endpoints.rs}`、`backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`、`docs/design/2026-06-29-enumeration-tool-boundary.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-pentest/output_store.md`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 app/backend 后重新跑 enumeration；预期 browser helper 用当前代码经 Node 启动，若 JSAPI 仍失败，模型只能走 `js_collect` / `js_extract_apis` / `route_probe_paths` / `arjun` 等内容枚举路径，不能 fallback 到 `whatweb`。既有 DB 里已经污染的第三方 `active_discovered` targets 本轮没有做清理，避免未授权删/改历史数据；后续可单独做 DB repair。用户已要求不要跑 `precommit`，后续只做 scoped targeted checks。

### 2026-06-29 · EAS legacy wave / org pass 兼容修复

- **本轮目标**：回应用户“之前 gate 已经过了，重启后为什么又重新跑之前提交过的”，修复 EAS 资产波次账本上线后与旧 org-level gate completion 不兼容导致的重复跑。
- **根因/判断**：
  - 最新 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782643983045-1/`；operation `5d03ba91-52f6-42ac-9bba-2699633d483e` 当前阶段 `external_attack_surface`。
  - DB 里旧的 `org_stage_completions` / gate pass 记录仍在；问题不是 gate 丢了，而是新加的 `stage_asset_waves` 没有历史 `wave_items`，重启后把旧 pass 前已经存在的 assets 当成“未分配 delta wave”重新派发。
  - 现场证据：珠海营业部、大连平安大厦相关 wave 在 2026-06-29 09:12 后新建/运行，但 wave items 的 targets 创建时间在 2026-06-28 22:15-22:27，早于旧 org pass；属于 legacy ledger 不对齐。
- **已完成**：
  - `stage_asset_waves` repo：当 org 没有历史 wave 时，`create_next` 只会挑 `org_stage_completions.passed_at` 之后新创建的 targets；避免把旧 pass 前资产重新排队。
  - `stage_asset_waves` repo 新增 `all_items_created_at_or_before(wave_id, cutoff)`，用于判断当前 running wave 是否只是旧 completion 覆盖过的历史资产集合。
  - `DbRepoProvider` / `AgentDbRepo` 打通只读 current running wave 与 legacy item coverage 查询；`stage_run_call` 在已有 fresh org pass 时只读取当前 wave，不再因为 skip 判断而创建新 wave。
  - `stage_run_call` skip 逻辑：如果 org 已通过，且当前 running wave 的所有 target items 都早于/等于该 pass 时间，则把这个 running wave 视为 legacy covered 并 skip/complete，不再重新跑子 agent。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-db/repo.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate）。
  - `cd backend && cargo fmt -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-db stage_asset_wave --status-level fail` → 5 passed / 113 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime resume_skip --status-level fail` → 2 passed / 285 skipped，exit 0。
  - `cd backend && cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave --status-level fail` → 1 passed / 286 skipped，exit 0。
  - `cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` / `pnpm --silent format`，展开后真实原因是 pnpm install deps-status check 报 `[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts: @swc/core@1.15.21, electron@23.3.13, esbuild@0.25.12`，需用户侧 `pnpm approve-builds` 或本机依赖策略处理后才能跑全量前端 recipe。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-db/src/repo/stage_asset_waves.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,orchestration.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-db/repo.md`、`feature_list.json`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 app 后续跑该 EAS operation 验证：预期已 pass 的旧 wave 会被自动视为 covered 并跳过；只有 pass 之后新增的真实 target 才会进入 delta batch。full `just precommit` 已尝试但卡在本机 pnpm ignored-builds gate，未完成全量前端/后端套件。

### 2026-06-29 · enumeration worklist / deliverable contract 收紧

- **本轮目标**：回应用户“那就先改这几个”，先落地枚举阶段当前不合理点：EAS-confirmed web root 分母、提交前预检说明、瘦交付 claim/coverage 口径、Enumerator 工具面、direct enum tools taxonomy；不改枚举后的 graph 路由。
- **已完成**：
  - `org_gate.rs`：`enumeration` per-org gate 进入 `coverage_complete` 前，优先把 in-scope 资产轴收敛到已有 `GOLISH-EAS-LIVENESS` found truth 的 web-capable target（domain/url）；没有任何 EAS live truth 时保持原资产轴 fail-safe，避免空分母假通过。
  - `stage_coverage.rs`：UI/agent 预检 read-model 使用同样的 EAS live web-root worklist 口径；保留 endpoint liveness key，确保 URL/port/path 资产和 EAS 探活事实能对齐。
  - `security.rs`：`check_stage_asset_coverage` 在 enumeration 输出 `worklist_semantics` / `deliverable_contract`，并把 gap 标注为 EAS-confirmed live web root，明确不要重扫端口或手写 DB-derived found cells。
  - `tool_taxonomy.rs`：把 `browser_collect_js_api` / `js_collect` / `js_extract_apis` 归为 `recon/crawler`，`route_probe_paths` 归为 `web/fuzzer`，让 stage whitelist 能管住 direct enum tools。
  - `golish-sub-agents` 默认构造两条路径都从 Enumerator 移除 `record_finding`；prompt 明确先 `check_stage_asset_coverage`，交 `findings: []`，claims 用 `web_root_enumerated` / `directories_discovered` / `api_endpoints_discovered` / `params_discovered` / `js_candidates_reviewed`。
  - `harness_submit_tool` schema 与 `resources/harness/stages/enumeration/methodology.md` 同步枚举瘦交付契约；模块卡同步 `golish-agent-kit/harness.md`、`tool_executors.md`、`golish-sub-agents.md`、`golish-agent-app/ai.md`。
- **运行过的验证（实跑）**：
  - `rustfmt backend/crates/...` → exit 1（直接调用默认 Rust 2015，提示需传 edition；随后改用 `--edition 2021` 成功）。
  - `rustfmt --edition 2021 backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/tests.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs` → exit 0。
  - `rustfmt --edition 2021 backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/tests.rs` → exit 0。
  - `cd backend && cargo test -p golish-agent-kit enumeration_worklist --lib` → 2 passed / 769 filtered，exit 0。
  - `cd backend && cargo test -p golish-agent-kit enumeration_preflight_surfaces_worklist_contract --lib` → 1 passed / 770 filtered，exit 0。
  - `cd backend && cargo test -p golish-agent-kit allowed_tool_names_enumeration_selectors_include_direct_enum_tools --lib` → 1 passed / 770 filtered，exit 0。
  - `cd backend && cargo test -p golish-sub-agents test_enumerator --lib` → first run 1 failed（direct builder still had `record_finding`），fix 后 rerun 2 passed / 108 filtered，exit 0。
  - `cd backend && cargo test -p golish-agent-app enumeration_worklist_read_model --lib` → 2 passed / 119 filtered，exit 0。
  - `cd backend && cargo test -p golish-agent-app parameters_describe_enumeration_slim_deliverable_contract --lib` → 1 passed / 120 filtered，exit 0。
- **未跑**：`just precommit`（本轮是枚举阶段 scoped 后端/文档调整；仓库已有大量未提交改动，且此前记录本机 `pnpm install` gate 会阻塞全量 precommit）。
- **提交记录**：未提交。
- **本轮修改但未提交（enumeration scope）**：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`、`backend/crates/golish-sub-agents/src/defaults/builder/{mod.rs,registry.rs}`、`backend/crates/golish-sub-agents/src/defaults/{tests.rs,prompts/execution_planning.rs}`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-agent-kit/{harness.md,tool_executors.md}`、`docs/modules/backend/golish-sub-agents.md`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。
- **风险 / 下一步**：没有改 `operation_graph` 的 `enumeration` 后续路由，按用户说明后续阶段尚未开始改；后面做 vuln_triage/reporting 串联时再处理。需要真实跑一条 EAS→enumeration operation 验证：`check_stage_asset_coverage(stage=enumeration)` 应只列 EAS live web roots，submit 不应要求/接受 findings。

---

### 2026-06-29 · 福州零资产 EAS 猜测扫描修复

- **本轮目标**：回应用户“福州平安这个公司 IP 不是没有吗，为什么还在扫资产，资产哪里来的”，定位 `149.120.175.217` 来源并修复。
- **根因/判断**：
  - 最新 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782643983045-1/`；福州平安房地产有限公司 org id `18db9012-c40a-4237-bb5c-59e1ceaa5542`。
  - `target_intel` 对该 org 明确是零资产：`manage_targets list` / `list_attack_surface_seeds` / `list_in_scope_targets` 都为空；DB `targets` 里也没有 `149.120.175.217`。
  - 资产来源不是情报阶段，而是 EAS prober 在 repair 中猜了域名 `fzpingan.cn` / `www.fzpingan.cn`，`httpx` 解析到 `149.120.175.217`，随后 `naabu` / `httpx` 直扫 IP；`manage_targets add` 被 repair mode 拦住，所以 `targets` 没新增，但 background batch completion 仍把 `technique_outcomes` 写到了当前 org，造成覆盖/UI 看起来像这个 org 有资产。
  - 触发链条：`check_stage_asset_coverage` 对 0 assets 返回 ready，但 `submit_stage_deliverable` 的 `coverage_complete` 对空 coverage + expected techniques BLOCK；repair mode 在没有结构化 `coverage_gap_actions` 时仍允许 `pentest_run`；EAS batch outcome 写入未按 org in-scope allowlist 过滤。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`：`coverage_complete` 区分“模型没交矩阵”和“调用方权威注入 `in_scope_assets=Some([])`”；后者 vacuous pass，与 `check_stage_asset_coverage` 一致。
  - `backend/crates/golish-sub-agents/src/executor_types.rs` / `executor/response_parsing.rs`：coverage needs_fix 若没有结构化 `coverage_gap_actions`，repair 只允许 coverage/DB 查询、后台 job 控制和 resubmit，不允许 `pentest_run` / guessed-domain probes；有 action list 时仍允许严格目标内的 batch probing。
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：后台 EAS batch LIVENESS/PORT/SERVICE outcome 写入前，按当前 `organization_id` 的 in-scope `targets.value` + `targets.real_ip` 建 allowlist；LIVENESS 用 endpoint key，PORT/SERVICE 用 host key；org 下没有 in-scope 资产则跳过 upsert。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`、`docs/modules/backend/golish-sub-agents/executor.md`；`feature_list.json` 追加本次 scoped evidence，状态仍 `in_progress`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate）。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-sub-agents --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit coverage_complete --status-level fail` → 21 passed / 746 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair --status-level fail` → 6 passed / 104 skipped，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 19 passed / 99 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `jq empty feature_list.json` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`、`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`、`docs/modules/backend/golish-sub-agents/executor.md`、`feature_list.json`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 app 后重新跑/续跑一个零资产 org EAS 验证：预期 `check_stage_asset_coverage` 0 assets 后可直接 submit pass；repair 不会再猜域名扫描；即使后台工具被误跑，outcome 也不会写到该 org。full `just precommit` 未跑，仍受本机 pnpm install gate 影响。

---

### 2026-06-29 · EAS coverage UI 合并同 run evidence 终态

- **本轮目标**：解释并修复用户截图中“深圳平安金融科技咨询有限公司”EAS 已过 gate，但资产覆盖面板仍显示 `7/9 done`、`157.240.9.36` / `www.google...pinganjrkj.com` 两行状态怪异的问题。
- **根因/判断**：
  - 最新 run：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782643983045-1/`；org id `d8f38de2-56bb-4fa9-9db9-8a1440fbb3d4`。
  - gate 最终 PASS 是因为同 session `audit_log` / accepted deliverable 已有终态负结果：`157.240.9.36` LIVE 查空，long domain PORT 解析失败/无目标后可解释为查空，SVC 因无开放端口不适用。
  - UI read-model 只合并 `technique_outcomes` + `source_query_log`，没有合并同 session `audit_log` evidence facts；所以 accepted deliverable 能过，覆盖面板却还少两格。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：`stage_outcomes` 在有 `session_id` 时额外读取同 session `audit_log` evidence facts，并按现有 EAS key 规则合并到 asset × technique projection。
  - 同文件：仅对 EAS LIVE/PORT 的“failed to resolve / no valid targets / no targets specified / 0 IP addresses”类 evidence error，在 UI read-model 中显示为 `checked_empty`；普通 error 仍保持 `error`。
  - 补回归：IP LIVE evidence fact 能填 missing cell；不可解析长域名 PORT error 显示为 `checked_empty` 并派生 SVC `not_applicable`；generic evidence error 不被吞掉。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 `ai_get_stage_asset_coverage` 的数据源扩展和 no-target 归一规则。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（既有本机 pnpm install/approval gate）。
  - `cd backend && cargo fmt -p golish-agent-app --check` → exit 0。
  - `cd backend && cargo test -p golish-agent-app ai::commands::stage_coverage -- --nocapture` → 28 passed / 88 filtered out，exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 28 passed / 88 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启/热重载后刷新同一 EAS 详情页，预期这两行会收敛为 `157.240.9.36 LIVE 查空 / PORT 查空 / SVC 不适用`，长域名 `LIVE 查空 / PORT 查空 / SVC 不适用`，summary 不再停在 `7/9 done`。full `just precommit` 未跑，仍受本机 pnpm install gate 影响。

---

### 2026-06-29 · ChatPanel 对话片段间距微调

- **本轮目标**：按用户反馈，先不处理 `stage_run` 重复状态问题，只把 ChatPanel 里 Thought/正文贴近后整体“每句对话”的间距稍微拉高一点。
- **已完成**：
  - `frontend/components/AIChatPanel/MessageBlock.tsx`：外层 segment stack 从 `gap-2` 调整为 `gap-2.5`；保留正文紧跟 Thought 时的 `-mt-1` compact 规则。
  - `docs/modules/frontend/components.md`：同步记录 Thought/正文连续出现时既要避免双重 margin，也要保留略宽整体 segment gap。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/MessageBlock.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/messageSegments.test.ts` → 1 file / 11 tests passed。
  - `git diff --check -- frontend/components/AIChatPanel/MessageBlock.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/AIChatPanel/MessageBlock.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新 ChatPanel 看 10px segment gap 是否刚好；`stage_run` 底部重复状态仍按用户要求暂不处理。

---

### 2026-06-28 · 平安养老 EAS repair / coverage key 优化

- **本轮目标**：按用户要求先做 checkpoint commit，再分析“平安养老保险股份有限公司”这次 EAS 已过 gate 但 submit/repair 很绕的问题，并落一刀低风险优化。
- **先行提交**：按用户“开始之前先 commit 一下”要求，已把开始前工作树 checkpoint 成 `b7b05043 checkpoint: EAS harness and UI updates`。
- **日志 / DB 诊断**：
  - 目标 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782643983045-1/`；平安养老 org id `edf818cc-5699-40fe-a0e7-e0aad5b0afbe`。
  - 该 org EAS 最终 PASS 不是假阳性，但路径低效：prober 侧约 `pentest_run=33`、`submit_stage_deliverable=14`、`needs_fix=10`；最后的修复主要被 evidence/coverage 细胞映射拖慢。
  - 关键问题 1：`http://115.159.235.124:8080` / `https://139.199.48.27` 这类 URL-wrapped IP target 的 PORT/SERVICE terminal outcome 写在裸 IP key 上，coverage read-model 之前按原 URL 查，容易把已有 terminal outcome 画成 gap。
  - 关键问题 2：submit repair mode 进入 coverage gap repair 后会挡住 `check_stage_asset_coverage`，导致 agent 在最需要看 gap 表时看不了，只能盲交或重复扫。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：EAS PORT / SERVICE-FINGERPRINT coverage lookup 改为 `canonical_asset_key` host key；LIVENESS 继续保留 endpoint key（port/path），与 `technique_outcomes` 写入侧对齐。
  - `backend/crates/golish-sub-agents/src/executor_types.rs`：`SubmitRepairMode` 的三类 repair allow-list 都允许只读 `check_stage_asset_coverage`；coverage-gap repair 仍保留 target/list-file/CIDR 限制。
  - 补回归：URL-wrapped IP target 能匹配裸 IP PORT outcome；coverage-gap repair 不再 block `check_stage_asset_coverage`。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`；`feature_list.json` 追加 scoped evidence，状态仍 `in_progress`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app host_level_eas_outcome_matches_url_wrapped_ip_asset outcome_row_matches_liveness_endpoint_alias empty_outcome_is_checked_empty --status-level fail` → 3 passed / 110 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_needs_fix_enters_targeted_gap_closure_mode evidence_ref_needs_fix_enters_repair_mode_and_blocks_scans background_jobs_needs_fix_enters_wait_only_repair_mode --status-level fail` → 3 passed / 107 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 25 passed / 88 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents response_parsing --status-level fail` → 22 passed / 88 skipped。
  - `cd backend && cargo check -p golish-agent-app -p golish-sub-agents` → exit 0。
- **未跑**：本轮未重跑 `./init.sh` / `just precommit`；开始前已经确认 `./init.sh` 会在 `pnpm install --silent` / ignored-build approval gate 阶段失败，本轮做 scoped 后端验证。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`、`feature_list.json`、`agent-progress.md`。
- **风险 / 下一步**：需要重启 app 后用新的 EAS run 验证 submit 次数下降；这刀不会放松 gate，只减少 URL/IP key 漂移和 repair 阶段盲修。

---

### 2026-06-28 · SubAgent detail DSML 工具调用泄漏清理

- **本轮目标**：回应用户截图问题：“detail 里这一大段是谁的，为什么样式很奇怪”；定位归属并修复 `DSML` 文本工具调用标记泄漏到子 agent 正文的问题。
- **根因/判断**：
  - 这段属于 `stage_run` 下的 EAS `Prober` 子 agent 普通 narrative，不是工具 stdout，也不是主 agent 最终报告。
  - 样式怪有两层：一是多轮 `sub_agent_text_delta.accumulated` 被作为正文连续渲染，读起来像一整块日志；二是 provider 退化出的 `DSML` 文本工具调用（`submit_stage_deliverable` 参数/coverage JSON）没有被 detail 清洗函数识别，直接混进了 Markdown 正文。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：`stripAgentXmlTags` 增加 DSML 伪标签兜底，剥离完整/未闭合的 `tool_calls` / `invoke` / `parameter` 文本工具调用块。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增完整 DSML submit block 和未闭合 DSML streaming block 回归；顺手把该测试里的 session mock `mode` 从旧的 `"chat"` 对齐为当前 `SessionMode` 的 `"agent"`。
  - `docs/modules/frontend/components.md`：同步记录 provider 文本工具调用标记不属于 agent prose，detail 渲染前必须剥掉。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（既有本机 pnpm install/approval gate）。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0，fixed 1 file。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 1 file / 47 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新同一 detail 面板确认 DSML submit 参数不再出现在正文里；大段 narrative 仍会按子 agent 输出展示，只是内部工具调用标记会被剥掉。full `just precommit` 未跑，仍受当前本机 pnpm install/approval gate 与 dirty tree 影响。

---

### 2026-06-28 · ChatPanel Thought / 正文间距收紧

- **本轮目标**：回应用户截图反馈：ChatPanel 里 Thought 和正文之间距离过大；另一个 `stage_run` 工具卡 + 底部 `Running stage run` 重复状态先讨论，不直接改。
- **已完成**：
  - `frontend/components/AIChatPanel/ThinkingBlock.tsx`：默认 message variant 不再自带 `mb-2`，避免 Thought 自身 margin 与 MessageBlock segment gap 叠加。
  - `frontend/components/AIChatPanel/MessageBlock.tsx`：正文紧跟 Thought 时加 compact top spacing（`-mt-1`），只收紧 Thought→正文这条相邻关系，不改变工具卡与其它 segment 的常规间距。
  - `docs/modules/frontend/components.md`：同步记录 ChatPanel Thought / 正文连续出现时的 spacing 约束。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/AIChatPanel/MessageBlock.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/messageSegments.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/AIChatPanel/ThinkingBlock.tsx`、`frontend/components/AIChatPanel/MessageBlock.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新 ChatPanel 视觉确认 Thought 与正文距离是否合适；`stage_run` 重复状态样式需用户确认方案后再改。

---

### 2026-06-28 · EAS PORT 查空时 SVC 覆盖派生终态

- **本轮目标**：回应用户截图里平安信托 `36/43 done`、7 个 IP 行显示 `查空 LIVE/PORT` 但仍 `未查 SVC` 的问题；判断这些 IP 没有开放端口时，SERVICE-FINGERPRINT 不应继续作为 pending 缺口展示。
- **根因/判断**：
  - `ai_get_stage_asset_coverage` 的 read-model 只认 found truth 和 terminal outcomes；IP/domain 结构上适用 PORT/SERVICE，所以当 PORT 已 `checked_empty` 且没有显式 SERVICE outcome 时，SVC 仍落到 `pending`。
  - gate 可以接受模型/交付物里的 `not_applicable` 终态，但 UI snapshot 没有把“无开放端口 => 无服务指纹面”做确定性派生，导致 pass 后仍显示 `未查 SVC`。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：EAS coverage cells 生成后，若 PORT cell 已 terminal `checked_empty/not_applicable`，且 SERVICE-FINGERPRINT 仍是 `pending`，则把 SERVICE-FINGERPRINT 派生为 `not_applicable`，并清空 suggested tools；显式 SERVICE outcome（found/empty/error/blocked/not_applicable）不会被覆盖。
  - 同文件新增回归测试：PORT 查空派生 SVC not_applicable；PORT found 时 SVC 仍 pending；显式 SERVICE outcome 优先。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 EAS SVC read-model 派生规则。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 24 tests passed / 88 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：full `just precommit` 未跑；本机该工作流此前仍受 pnpm ignored-build approval gate 影响。需要重启/热重载后刷新平安信托详情页，预期这些 `查空 PORT` 的 IP 不再显示 `未查 SVC`，summary 从 `36/43` 收敛为当前 batch 全 done（除非还有别的真实 pending/error）。

---

### 2026-06-28 · EAS per-org wave 改为 global delta expansion backlog

- **本轮目标**：按用户澄清修正 EAS wave 口径：不要在单个子公司 gate PASS 后立即 promote/continue 下一 wave；所有 org 先完成当前 seed batch，新发现 HTTP(S) 入口 / 新 host 作为 expansion backlog，后续由全局 delta pass 统一处理。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：移除 per-org gate PASS 后的自动续跑；当前 durable wave 只作为 current batch denominator freeze，PASS 后 mark completed 并写 `org_stage_completions`。
  - 同文件：所有 org seed batch 都 PASS 后，统一为有新增 target 的 org queue durable delta batch；只要 queue 出 delta batch，本轮不发 close `pass_token`，要求主 agent 再跑一次 `stage_run` 处理全局 delta。
  - 同文件：worker objective / current wave instruction 改为 `next_wave_pending` 是 global delta expansion backlog，不是“马上下一 wave”。
  - `docs/design/2026-06-28-stage-expansion-wave-barrier.md` 与对应 plan 标记 superseded；新增 `docs/design/2026-06-28-eas-global-delta-expansion.md`、`docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md` 记录新方向。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-db/repo.md`；`feature_list.json` 的功能条目改成 global delta expansion 口径。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime --check` → exit 0。
  - `jq empty feature_list.json` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-app/ai.md docs/design/2026-06-28-stage-expansion-wave-barrier.md docs/design/2026-06-28-eas-global-delta-expansion.md docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md feature_list.json` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch --status-level fail` → 1 test passed / 286 skipped，exit 0。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/design/2026-06-28-eas-global-delta-expansion.md`、`docs/design/2026-06-28-stage-expansion-wave-barrier.md`、`docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md`、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、上述 4 张模块卡、`feature_list.json`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：这次完成了“seed 全部 org 收口后统一 queue delta batch，并暂停 close token”的调度修正；HTTP(S) endpoint promotion classifier / `expansion_queue` processed/skipped 状态仍待实现。full `just precommit` 未跑，仍受本机 pnpm ignored-build approval gate 影响。

---

### 2026-06-28 · EAS 资产覆盖 summary / wave 口径修复

- **本轮目标**：回应用户质疑“主资产 288/294 done 怎么也过 gate”：核对 Ping An EAS DB/run_tree 真相，并修复前端资产覆盖 summary 与 wave cutoff 口径不一致造成的误导。
- **根因/判断**：
  - DB/run_tree 显示 root org `0e9753e6-3cbb-40bf-9510-8d1bda7193f1` 的 EAS 不是在 `288/294` 时最终 PASS；当时确有 6 个 `GOLISH-EAS-PORT` pending，后续补成 `empty/naabu` 后继续跑 wave #2/#3，最终 `org_stage_completions` 于 `2026-06-28 21:30:32 +08:00` 写入。
  - UI 问题是 `StageAssetCoverageBlock` 没传 `stageStartedAt`，并且 compact/header summary 从 rows 自行重算，容易把 `new_in_stage`/下批资产混进当前分母或展示旧 attempt 快照，造成“288/294 也过了”的错觉。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：coverage API 请求透传 `stageStartedAt`；summary/compact strip/full panel 统一使用后端 `snapshot.summary`，删除前端 rows 重算分母逻辑。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：从父 `stage_run` 工具块读取 `startedAt`，传给资产覆盖块，供后端按 stage/wave cutoff 标记 `next_wave_pending`。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：回归锁定 `stageStartedAt` 传参，并把 summary 期待改为以后端 summary 为准。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 coverage summary/wave 口径约束。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782643983045-1 --db` → exit 0；确认 Ping An root EAS 中间有 `6 cells never reached a terminal state`，后续 wave #1/#2/#3 完成后 root org PASS。
  - Python 只读 DB 查询 embedded Postgres → exit 0；root org EAS waves = 200 / 108 / 1 all completed；6 个域名的 `GOLISH-EAS-PORT` 均已有 `empty` outcome（source `naabu`，evidence `[12113]`）。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 65 tests passed。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：full `just precommit` 未跑；本机此前仍有 pnpm ignored-build approval gate。需要刷新同一 EAS 历史/详情 UI，确认最终 PASS 上下文不再显示旧的 `288/294` 口径，并能把下批资产以 `下批`/`next_wave_pending` 表达。

---

### 2026-06-28 · SubAgent detail Thought / Agent Output 视觉统一

- **本轮目标**：回应用户截图反馈：detail 里的 `Thought` 和 `Agent Output` 看起来像两个不同层级，希望视觉更统一。
- **已完成**：
  - `frontend/components/AIChatPanel/ThinkingBlock.tsx`：新增 `variant="detail"`，只在 detail 场景调整 Thought 的标题权重、间距、展开内容字号/行高；默认聊天消息里的 Thought 样式保持不变。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：抽出统一 narrative block class，让 Thought 与 Agent Output 共用左侧 rail、背景、内边距；Agent Output 标题从强分区标题降为与 Thought 同级的紧凑标题。
  - 同文件：修复 `parentStageRunTool` Zustand selector 每次返回新对象导致 detail 挂载时可能触发 React `Maximum update depth exceeded`；拆成 status / startedAt 两个 primitive selector。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增 stage-run backed detail 挂载回归，锁住 selector 不再触发无限更新。
  - 根据用户复看截图再次收紧视觉：去掉 Thought/Agent Output 外层连续左侧 rail，压缩 narrative block 上下间距，并让 Agent Output 正文缩进到标题文字列下方，避免正文从图标列起头造成错位。
  - 根据用户继续反馈：进一步淡化 detail Thought（使用 muted foreground、normal weight），并移除普通正文前的 `Agent Output` 标题；保留时间顺序，不把 output 倒排到 thought 上方。
  - 根据用户最新截图：修正 Thought 后紧跟正文时的“双 padding”问题；正文块在前一条是 Thought 时使用 compact top spacing，让 Thought 和正文更贴近同一段叙述。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 Thought / Agent Output 共用紧凑 narrative chrome。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（无代码编译阶段）。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 55 tests passed。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0，fixed 1 file。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts` → 2 files / 56 tests passed；stderr 有 test-only `react-i18next:: useTranslation` missing i18n instance warning。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 2；失败在既有 dirty 文件 `frontend/components/Engagement/StageAssetCoveragePanel.tsx(186,10): 'coverageRowsSummary' is declared but its value is never read`，非本轮修改文件。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/ThinkingBlock.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/AIChatPanel/ThinkingBlock.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新实际 detail 面板看视觉效果；全量 typecheck / precommit 仍受当前 dirty tree 的资产覆盖面板未使用变量与 pnpm install gate 影响。

---

### 2026-06-28 · stage_run 提交前 coverage 自检提示收口

- **本轮目标**：回应用户“能不能提交之前告诉 AI 先看少了什么，而不是交了看报错”：把已有 `check_stage_asset_coverage` 从可选提醒强化为 coverage-gated worker 的提交前 mandatory self-check。
- **根因/判断**：
  - `check_stage_asset_coverage` 已能返回 `ready_to_submit` / `gap_examples` / `cell_summary` / `next_action`，但此前主要靠 methodology 文案和 submit 后的 `needs_fix`，worker objective 本身没有强制“提交前先查缺口”。
  - 这会让弱模型继续先调 `submit_stage_deliverable`，再从 gate 报错里学习缺什么；用户看到的体验就是“每次过 gate 很麻烦”。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：coverage-gated per-org objective 新增 `PRE-SUBMIT SELF-CHECK (mandatory)`，要求 `submit_stage_deliverable` 前先调用 `check_stage_asset_coverage(stage, organization_id)`；`ready_to_submit=false` 时按 `gap_examples` / `cell_summary` / `next_action` 补洞或终态收口，不允许试提交。
  - `resources/harness/stages/{target_intel,external_attack_surface,enumeration}/methodology.md`：同步说明 preflight 是 required self-check，不是 trial submit。
  - 模块卡同步：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime build_org_objective --status-level fail` → 2 tests passed / 285 skipped。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本轮 runtime/stage methodology/doc/progress 文件>` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`resources/harness/stages/external_attack_surface/methodology.md`、`resources/harness/stages/target_intel/methodology.md`、`resources/harness/stages/enumeration/methodology.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：这是 prompt/objective 层收口，能显著减少“先提交再看 gate 报错”的行为；如果后续还想做到硬约束，需要在 submit tool 里记录最近一次 `check_stage_asset_coverage ready_to_submit=true` 的 preflight stamp，再拒绝未预检的提交，这会是更大一层 runtime 状态改造。

---

### 2026-06-28 · EAS Prober 后台等待与 batch SERVICE 落库收口

- **本轮目标**：回应用户对最新 Test1 EAS run 的诊断结论：主公司 Prober 因 broad `nmap -sV -iL` 后台任务和 SERVICE coverage 未终态而看起来卡死；先修低风险 runtime/landing 问题，暂不做 stage_run 并发大改。
- **根因/判断**：
  - `whatweb --input-file='/abs/path'` 这类 equals+quoted 绝对路径会被 batch input parser 当成带引号的相对路径，拼成 `workspace/'/abs/path'`，导致后台 batch SERVICE outcome 读不到 input file，工具跑完也不补 `GOLISH-EAS-SERVICE-FINGERPRINT` terminal rows。
  - EAS Prober / StageRefiner 文案仍容易把 SERVICE 缺口引向对 raw in-scope 大列表跑 broad `nmap -sV -iL`，而不是基于确认开放端口的 host:port 分组。
  - `wait_for_background_jobs` 需要按 Cursor/Codex 式 wait/check loop 表达：总等待可长，但 idle 无新输出时应返回可操作状态，让 agent `check_job` 一次，有进展继续等，无进展再 kill/缩窄/终态收口；不能静默卡住整个 org，也不能误杀有进展的长任务。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：batch input file parser 先剥 `--input-file='/abs/path'` 参数值两侧引号，再判断绝对路径；新增回归测试。
  - `backend/crates/golish-app-core/src/pty_interactive.rs`：`wait_for_background_jobs` 新增 idle-progress 跟踪，默认总等待仍 300s；如果 stdout/stderr 在 idle 窗口内无新进展，返回 `still_running` + `wait_reason=idle_timeout` + 推荐 `check_job`/按需 `kill_job`，有进展则继续等到总窗口。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：EAS Prober objective 明确禁止 raw 大列表 broad `nmap -sV -iL`；SERVICE 只能基于确认开放端口的 host:port 分组；后台等待按 visible wait/check loop。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs` 与 `resources/harness/stages/external_attack_surface/methodology.md`：SERVICE repair 提示改为 confirmed-open-ports 分组；不可解析/无开放端口/批次过宽用具体 terminal note 收口。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 仍失败（本机 pnpm ignored-builds approval gate）。
  - `cd backend && cargo fmt -p golish-app-core -p golish-agent-runtime -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-app-core pty_interactive --status-level fail` → 10 tests passed / 35 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 17 tests passed / 92 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime build_org_objective --status-level fail` → 2 tests passed / 285 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_refiner --status-level fail` → 3 tests passed / 763 skipped。
  - `cd backend && cargo check -p golish-agent-app -p golish-app-core -p golish-agent-runtime -p golish-agent-kit` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-app-core -p golish-agent-runtime -p golish-agent-kit --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本轮后端/runtime/doc 文件>` → exit 0。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-app-core/src/pty_interactive.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`resources/harness/stages/external_attack_surface/methodology.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：本轮没有实现 stage_run per-org 并发，因为当前 `stage_run_call.rs` 明确使用共享 side-channel 串行执行；若要并发，需要先隔离 deliverable sink / worker state。需要重启 dev app 后重新跑 Test1 EAS，确认主公司 Prober 不再 broad service sweep 卡住，且 WhatWeb/nmap batch SERVICE terminal rows 能落入 coverage。

---

### 2026-06-28 · target_intel 组织情报 source target 汇总修复

- **本轮目标**：回应用户截图反馈：后端 target_intel 阶段按理已经通过，但前端组织情报行里 DNS / ASN / CT / 子域 / OSINT 仍显示未查，只有 WHOIS 显示查空。
- **根因/判断**：
  - 用户判断成立：这里不是单纯前端样式问题，而是 read-model key 对不上。
  - `ai_get_stage_asset_coverage` 的 organization row 用公司名当 asset key；但 `source_query_log` 里的 terminal rows 常常记录在实际查询目标上（例如 `pingan.com`），当前 org 又可能还没有登记真实 asset row，于是 `merge_source_query_row` 只匹配空 target 或完全相同 asset value，导致 DNS/ASN/CT/子域/OSINT 这类 source/provider terminal rows 被漏投影。
  - gate/后端可通过不等于 UI 的组织情报行都应是 `found`；UI 应解释 source/provider 的 terminal 状态，不能因 target key 不同画成未查。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：对 target_intel source/provider terminal rows 增加 organization-row rollup；当 source row target 无法匹配任何已登记资产，但当前 snapshot 有 organization row 时，按 technique 汇总到组织情报行。
  - 同文件：新增回归测试覆盖 `target="pingan.com"` 这类 unmatched source row 会汇总到组织行；同时锁定没有 organization row 时不能误映射到普通资产。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 target_intel source/provider terminal rows 的 organization row 汇总语义。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 21 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 dev app / 后端进程后刷新同一组织情报行；DNS/ASN/CT/子域/OSINT 不应再因为 source target 是域名而显示未查。full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · target_intel 组织情报维度标签可读性修复

- **本轮目标**：回应用户截图反馈：Intel 阶段资产覆盖里“组织情报”只显示一排 `? / ✓` 小格，用户不知道每一类分别代表什么。
- **根因/判断**：
  - 后端 `target_intel` organization row 实际有 6 个被动情报维度：DNS、WHOIS、ASN、CT、Subdomain、OSINT。
  - 前端之前复用真实资产矩阵的紧凑小状态格，只把维度名放在 `title` hover 里；默认视觉上看不到维度名。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：新增 organization 专用 coverage chip，把组织情报维度直接显示为 `DNS` / `WHOIS` / `ASN` / `CT证书` / `子域` / `OSINT`，并保留每格状态符号与 hover 详情。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增 target_intel organization-only snapshot 回归，锁定 6 个维度标签必须可见。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 target_intel 组织情报不允许只画无标签小状态格。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 20 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要刷新资产覆盖面板确认截图里的组织情报行不再是一排无标签小格；full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · 资产覆盖 evidence_refs fallback SQL 修复

- **本轮目标**：回应用户截图反馈：Target Intel 的资产覆盖面板显示“加载失败”，API 报 `[API trace=...] ai_get_stage_asset_coverage: no column found for name: evidence_refs`。
- **根因/判断**：
  - 这不是资产为空，而是 `ai_get_stage_asset_coverage` 后端读模型失败。UI fallback 查询 `technique_outcomes` / `source_query_log` 最新 terminal rows 时，SQL 选出列名 `evidence_ids`，但 `TechniqueOutcomeProjectionRow` / `SourceQueryProjectionRow` 的 `sqlx::FromRow` 字段名是 `evidence_refs`。
  - `sqlx::FromRow` 按列名取值；一进 latest fallback 就因为找不到 `evidence_refs` 列而直接让整个覆盖面板失败。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：把 latest fallback 两条 SQL 抽成常量，并将 `evidence_ids AS evidence_refs` 显式 alias 给投影结构。
  - 同文件：新增单测锁住 `evidence_ids AS evidence_refs` alias，避免后续改 SQL 又把 UI fallback 打断。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 19 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：需要重启 dev app / 后端进程后刷新资产覆盖面板；该面板不应再因 `evidence_refs` 列名错误加载失败。full precommit 仍需先处理 pnpm `approve-builds` gate。

---

### 2026-06-28 · stage_run 历史详情持久化修复

- **本轮目标**：回应用户截图反馈：关掉/重开后，`stage_run` 之前的调用记录、子 agent 对话/工具详情看起来丢失，只剩聊天里的 `Running specialist agents` 工具卡。
- **根因/判断**：
  - `Running specialist agents` 不是后端重新跑出来的阶段名，而是 `frontend/lib/tools.ts` 给 `stage_run` 的人类化工具标题。
  - DB autosave 的 conversation fingerprint 之前只看 timeline block 数量和最后一块 id/type；`sub_agent_activity` 旧 block 内部的 `entries`、`toolCalls`、`result`、`thinking` 变化，以及 `ai_tool_execution.streamingOutput/result` 变化不一定触发保存。关窗后恢复端只能拿到轻量 `stageRunJson`，完整子 agent 运行流可能没写进 `timeline_blocks`。
  - `terminal_state.stage_run_json` 之前只保存 session 当前 `stageRun`，没有保存 `stageRuns[requestId]` 历史 map；连续 `stage_run` / continue 后，旧工具行可能找不到自己 requestId 对应的 rows。
- **已完成**：
  - `frontend/lib/conversation-db-sync.ts`：新增 timeline 内容指纹，覆盖 `sub_agent_activity.entries/toolCalls/result/thinking`、`ai_tool_execution.streamingOutput/result`、command output 等关键内容；旧 block 内容变化也会触发 autosave。
  - 同文件：`stage_run_json` 改为兼容 v2 包 `{ current, byRequestId }`，保存当前 run 和 request-scoped 历史 map；`stageRuns[requestId]` 变化也进入 autosave fingerprint。
  - `frontend/lib/terminal-restore.ts`：恢复端兼容旧的单个 `SessionStageRun` JSON，并能把 v2 `byRequestId` map 放回 session。
  - `frontend/lib/conversation-db-sync.test.ts`：新增回归覆盖 sub-agent 旧 block 更新、非最后一条工具 streaming output 更新、stage_run v2/legacy 持久化形状。
  - `docs/modules/frontend/lib.md`：同步记录 conversation DB autosave / stage_run restore 约束。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（沿用本机 `ERR_PNPM_IGNORED_BUILDS` approval gate）。
  - `./node_modules/.bin/biome check --write frontend/lib/conversation-db-sync.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/conversation-db-sync.test.ts frontend/store/stage-run.test.ts` → 2 files / 21 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/conversation-db-sync.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.ts docs/modules/frontend/lib.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（stage_run 历史详情持久化 scope）**：`frontend/lib/conversation-db-sync.ts`、`frontend/lib/conversation-db-sync.test.ts`、`frontend/lib/terminal-restore.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 下一步**：该修复保证后续保存/恢复不再只剩 `stage_run` 轻量卡；已经被关窗前未写进 DB 的旧子 agent 详情，仍需要从 `{workspace}/.golish/transcripts/<session>/run.log` / `transcript.json` 或 `scripts/run_tree.py` 追，不会凭空从 DB 里恢复。

---

### 2026-06-28 · Chat / 调用树调试编号与工具次数隐藏

- **本轮目标**：回应用户要求：ChatPanel 和左侧调用/详情区域里可见的 `Txx` 调试编号，以及“调用步骤/工具调用了多少次”的次数徽标不要再展示。
- **已完成**：
  - `frontend/components/ui/AnchorChip.tsx`：保留组件和调用点兼容性，但不再渲染可见 anchor chip；requestId 仍留在 store / detail navigation 内部使用。
  - `frontend/components/AIChatPanel/SubAgentInlineCard.tsx`、`frontend/components/SubAgentCard/*`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentTreeView/SubAgentTreeView.tsx`：移除 inline card、sub-agent card、modal、detail header、左侧调用树 header/agent row 中的工具次数汇总；具体工具调用行仍可展开查看。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 request-id 锚点和工具次数汇总只作为内部导航/调试数据保留，不作为产品 UI 展示。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/AIChatPanel/messageSegments.test.ts frontend/components/AIChatPanel/InlinePlanCard.test.tsx` → 3 files / 57 tests passed, exit 0。
  - `git diff --check -- frontend/components/ui/AnchorChip.tsx frontend/components/AIChatPanel/SubAgentInlineCard.tsx frontend/components/SubAgentCard/SubAgentCard.tsx frontend/components/SubAgentCard/SubAgentDetailsModal.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentTreeView/SubAgentTreeView.tsx docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`frontend/components/ui/AnchorChip.tsx`、`frontend/components/AIChatPanel/SubAgentInlineCard.tsx`、`frontend/components/SubAgentCard/SubAgentCard.tsx`、`frontend/components/SubAgentCard/SubAgentDetailsModal.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentTreeView/SubAgentTreeView.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新 ChatPanel / sub-agent detail / 左侧调用树确认不再显示调试编号和工具次数；full precommit 仍需先处理本机 pnpm `approve-builds` gate。

---

### 2026-06-28 · EAS 覆盖 UI session fallback 收口

- **本轮目标**：接续崩溃 session 的截图/对话，排查「深圳平安人寿保险公司」资产覆盖 UI 仍显示未查，但 DB/gate 已有终态的问题。
- **现场结论**：
  - 用户质疑成立：当前 embedded PG 中 `深圳平安人寿保险公司` 的 `124.196.57.222`、`202.69.19.167` 等 IP 已有 `technique_outcomes` 终态；例如 `LIVENESS=empty/httpx`、`PORT=empty/naabu`，不是未查。
  - 崩溃 session 留下了半改状态：`stage_asset_coverage_snapshot` 已把 `session_id: Option<&str>` 传给 `stage_outcomes`，但 `stage_outcomes` 签名仍是 `&str`，导致当前 `golish-agent-app` 会编译失败。
  - UI 解释层和 agent 预检层需要分开：UI 可以在 session id 缺失/对不上时用同 org 最新 terminal outcome 做显示兜底，避免把已查空画成 pending；`check_stage_asset_coverage` 仍必须 strict session，不允许旧 run 结果帮 agent 通过提交前预检。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：补完 `stage_outcomes(... session_id: Option<&str>, allow_latest_fallback)`；Tauri UI 命令开启 latest terminal fallback，DB trait/agent preflight 关闭 fallback。
  - 同文件：`technique_outcomes` merge 时统一走 `coverage_lookup_asset`，修正 EAS LIVENESS URL endpoint key（`http://x:90` ↔ `x:90`）读模型匹配；latest fallback SQL 按 org + technique 取每个 `(asset, technique)` 最新 terminal row。
  - `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`：共享 snapshot helper 调用接入 strict 模式。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 UI fallback 与 agent preflight strict session 的边界。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - embedded PG 查询 `technique_outcomes` / `org_stage_completions`：确认 `深圳平安人寿保险公司` 的 `124.196.57.222`、`202.69.19.167` 等已有 `empty` terminal rows；latest fallback SQL 对 `124.196.57.222` / `202.69.19.167` 返回 4 行 `empty` outcome。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 18 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：重启 dev app 让后端 read-model 生效；刷新这家公司的资产覆盖面板后，已有 `empty` terminal rows 的 IP 不应再显示为 `? / 未查`。full precommit 需先处理 pnpm `approve-builds` gate。

### 2026-06-28 · EAS 覆盖 gate 与 UI read-model 口径排查

- **本轮目标**：回应用户追问“UI 里还有未查，为什么 gate 能过；政策资产覆盖是否应该 48/48 才算过”。
- **现场结论**：
  - `external_attack_surface` gate 的完整性不是“全绿色命中”，而是当前 wave 的每个适用 `(asset × technique)` 都有 terminal 状态；terminal 包括 `found`、`checked_empty`、`blocked`、`not_applicable`。本阶段新发现并排入下一批 wave 的资产不计入当前 wave 分母。
  - 该 org (`41f0a556-1176-43ec-b854-5cef2005494b`) 的 `org_stage_completions` 显示 EAS 在 `2026-06-28 16:21:25 +08` 通过；`stage_asset_waves` 有 wave 0/1/2 三批 completed。
  - transcript 显示早期 submit 确实被 `coverage_gap_actions` 拦过，不是“未查也放行”；后续 submit 在 `2026-06-28T08:16:45Z` accepted，其中 20 个 Pingan internal-only 域名提交了 60 个 `not_applicable` coverage cells（LIVENESS/PORT/SERVICE-FINGERPRINT），后续 wave #2/#3 也 accepted。
  - UI 截图里仍像 pending 的主要问题是 read-model 表达不完整：accepted deliverable 的 `not_applicable` terminal cells 目前没有稳定物化到 `technique_outcomes` 读模型；另有一个确定 bug 是 UI 适用性曾只看 `targets.type`，而 gate 用 `target_type + value` 的 value-aware 分类，URL 形态值可能被 UI 多显示假 PORT/SERVICE pending。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：覆盖矩阵适用性改用 `AssetClass::classify(Some(target_type), value)`，与 gate 口径一致；URL 形态 hostname 即使存成 `domain` 也不会冒出假的 PORT/SERVICE pending。
  - 同文件：`outcome_state` 识别 `not_applicable`，避免未来物化进 `technique_outcomes` 后又被 UI 读成 pending。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 `ai_get_stage_asset_coverage` 必须 value-aware classification，并说明 checked_empty/error/blocked/not_applicable 的读模型来源。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` → 查到该 run 的 DB 自诊断、gate block/pass 线索。
  - DB 查询 `org_stage_completions` / `stage_asset_waves` / `targets` / `technique_outcomes` → EAS completion `2026-06-28 16:21:25 +08`；wave 0/1/2 均 completed；targets 为 65 seed domain + 59 seed IP + 2 active domain + 3 active IP。
  - transcript grep `prober-call_00_iTmmAVW57YDu8fCqG1yq1095::org::41f0a556.../transcript.json` → `2026-06-28T08:16:45Z` submit accepted，内部域名 `not_applicable` cells 存在；`2026-06-28T08:19:08Z` / `08:21:18Z` 后续 wave submit accepted。
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs docs/modules/backend/golish-agent-app/ai.md` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail` → 15 tests passed / 87 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（coverage gate/read-model scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：后续需要把 accepted deliverable 里的 `blocked/not_applicable` terminal coverage 物化到稳定读模型（或单独的 coverage projection），否则旧 run/自报 terminal cell 仍可能在 UI 里看起来像 pending。

---

### 2026-06-28 · StageAssetCoverage 状态语义可读性修复

- **本轮目标**：回应用户截图反馈：资产覆盖矩阵里的小点容易被误读成“查空”，不确定哪些是未查、哪些是查空、哪些是新增/下批数据。
- **根因/判断**：
  - 后端读模型语义是正确的：`empty` → `checked_empty / 查空`；没有 terminal outcome 的格子 → `pending / 未查`；本阶段新增且排到下一 wave 的资产 → `next_wave_pending / 下批`。
  - 前端之前用弱化小点 `·` 表示 `pending`，顶部 chip 仍写英文 `pending`，图例没有列出 `next_wave_pending`；解析 IP 聚合行写“未登记 IP direct 行”，容易让用户误以为该 IP 行也是未查/查空。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：`pending` 状态格从弱点号改为 `?`，顶部 chip 改成 `N 未查`；图例补上 `下批`。
  - 行副标题追加状态摘要，例如 `未查 LIVE/PORT/SVC`、`下批待查 LIVE`、`查空 PORT`，让每一行不用 hover 就能区分未查和查空。
  - 解析 IP synthetic group 行文案改为 `仅分组，不计覆盖`，明确这类行不是 direct 覆盖行，也不代表未查/查空。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增/更新回归，锁定 pending 行摘要、next-wave 行摘要、synthetic IP group 不计覆盖。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 pending 必须有行级状态摘要、next_wave_pending 必须可见、synthetic IP group 只能作为分组行。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 19 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：待提交。
- **本轮修改但未提交（资产覆盖状态语义 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新资产覆盖面板后，截图中小点位置应变成 `?` 并在行副标题看到 `未查 ...`；真正查空才显示 `∅ / 查空`。若仍觉得矩阵太密，可以再把未查列加淡色背景或按状态筛选。

---

### 2026-06-28 · AIChatPanel 恢复期 loading 空态修复

- **本轮目标**：回应用户截图反馈：载入/恢复时右侧 AI 面板显示“今天要做点什么呢 / 工具可用”，看起来不像正在加载。
- **根因/判断**：
  - `AIChatPanel` 之前在 `messages.length === 0` 时无条件显示真实空会话提示；没有区分 `workspaceDataReady=false`、`pendingTerminalRestoreData`、`terminalRestoreInProgress`，以及 conversation 已恢复但 `activeSessionId` 尚未绑定的中间状态。
  - 截图中左侧仍是 `No active session`，右侧却可见空会话提示，正是 conversation/terminal restore 的绑定空窗。
- **已完成**：
  - `frontend/components/AIChatPanel/restoreLoadingState.ts`：新增 `shouldShowChatRestoreLoading`，把 workspace 未就绪、pending restore、restore in progress、active session 未绑定统一判为恢复 loading。
  - `frontend/components/AIChatPanel/AIChatPanel.tsx`：空消息区先判断恢复 loading，显示 spinner + “正在载入工作区 / 正在恢复会话和终端...”；恢复完成且确实空会话时才显示“今天要做点什么呢”。
  - `frontend/lib/i18n/{en,zh-CN}.json`：新增 loading 文案。
  - `frontend/components/AIChatPanel/restoreLoadingState.test.ts`：新增 4 条回归测试覆盖 workspace 未 ready、pending/running restore、conversation-terminal binding gap、正常空态。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 AIChatPanel 空态必须区分真实空会话与恢复期 loading。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/restoreLoadingState.test.ts` → 1 file / 4 tests passed, exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `jq empty frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/restoreLoadingState.ts frontend/components/AIChatPanel/restoreLoadingState.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需要 `pnpm approve-builds`。
- **提交记录**：未提交。
- **本轮修改但未提交（UI loading scope）**：`frontend/components/AIChatPanel/AIChatPanel.tsx`、`frontend/components/AIChatPanel/restoreLoadingState.ts`、`frontend/components/AIChatPanel/restoreLoadingState.test.ts`、`frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有大量此前任务留下的其它未提交文件，本轮未回滚。
- **风险 / 下一步**：未启动 dev app 做截图 QA；真实恢复窗口需要刷新/打开项目时观察。full `just precommit` 仍被本机 pnpm approval gate 阻塞。

---

### 2026-06-28 · EAS LIVENESS endpoint key gate loop 修复

- **本轮目标**：用户要求在确认最后一次跑的原因后直接修改；修复 `pentest-chat-1782574914157-1` 中 `http://linquankuaipin.com:90` / `http://ytzp.top:90` 已跑探活但 submit gate 仍报 `GOLISH-EAS-LIVENESS never attempted` 的问题。
- **现场结论**：
  - `run.log` 显示 `httpx` 已对 `http://linquankuaipin.com:90` 执行，completion 也记录了 `background batch liveness outcomes stored stored=1`；随后 gate 仍按 `http://linquankuaipin.com:90 × GOLISH-EAS-LIVENESS never attempted` 拦截。
  - DB 里 `technique_outcomes` / evidence fact 写成 `linquankuaipin.com`，而 gate 对 in-scope URL endpoint 的 join key 是去 scheme 后保留 port 的 `linquankuaipin.com:90`。因此事实存在，但落在 host-only key 上，关不掉 URL:port cell。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/evidence_facts.rs`：新增 `eas_liveness_asset_key`，专门给 EAS LIVENESS 使用；去 scheme/大小写，但保留 URL endpoint 的 port/path。`httpx -u http://x:90` 现在派生 `GOLISH-EAS-LIVENESS` fact `x:90`。
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：后台批量 liveness completion 写 `technique_outcomes` 时改用 endpoint key，避免 `http://x:90` 被 `canonical_asset_key` 折叠成 `x`。
  - `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`：`upsert_technique_outcome_impl` 对 `GOLISH-EAS-LIVENESS` 走 endpoint key；PORT / SERVICE-FINGERPRINT 继续 host-level canonicalization。
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：coverage snapshot / `check_stage_asset_coverage` 对 LIVENESS 使用同一 endpoint key，前端矩阵和 submit preview 与 gate 口径一致。
  - `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`：加回归证明裸 host liveness fact 不能关闭 `http://host:90`，endpoint fact `host:90` 可以关闭。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit --status-level fail -E 'test(eas_liveness_asset_key_preserves_url_endpoint_port) | test(coverage_maps_eas_liveness_tools) | test(coverage_complete_liveness_fact_must_preserve_url_port_endpoint)'` → 3 tests passed, 763 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app --status-level fail -E 'test(batch_liveness_input_file_is_recovered_from_httpx_l_flag) | test(batch_liveness_input_is_recovered_from_httpx_quoted_heredoc) | test(batch_liveness_and_service_commands_are_classified_by_intent) | test(eas_url_asset_only_requires_liveness) | test(outcome_merge_keeps_stronger_terminal_state_and_evidence)'` → 5 tests passed, 95 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机仍有前序已记录的 pnpm ignored-build approval gate（`@swc/core` / `electron` / `esbuild`），且本轮是后端 targeted 修复。
- **提交记录**：未提交。
- **本轮修改但未提交（本 bugfix scope）**：`backend/crates/golish-agent-kit/src/harness/evidence_facts.rs`、`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`、`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`agent-progress.md`。当前工作树仍叠有本日此前 wave/UI 等未提交改动，本轮未回滚。
- **下一步建议**：重启 dev app 让 Rust 代码生效；对同一目标再跑 EAS liveness 后，`http://linquankuaipin.com:90` 应写成 `linquankuaipin.com:90` 的 outcome，submit gate 不应再因 host/endpoint key mismatch 报 never attempted。

---

### 2026-06-28 · 资产覆盖快速滚动黑底与卡顿残留修复

- **本轮目标**：回应用户截图反馈：资产覆盖完整矩阵滑动太快时，列表下方会露出黑底/空白；用户复测后确认黑底可用但滚动仍有点卡，继续优化滚动路径；随后用户反馈每次 polling/live 更新会突然刷新列表、打断正在看的资产，继续加阅读稳定窗口；最后用户反馈快滑仍偶发黑底，继续把当前 332 资产规模退出虚拟化路径。
- **根因**：
  - 上一轮已做 group 虚拟化，但虚拟窗口的 scroll 读数仍走 rAF；快速甩动滚动条时，浏览器可能先把 scrollTop 移到新位置，而 React 仍渲染旧窗口，出现一帧黑底。
  - `CoverageGroupsList` 通过 `RefObject.current` 读外层 scroll 容器；ref 赋值本身不触发 effect，存在监听绑定时序空窗。
  - active/all 或内容缩短时，旧 `scrollTop` 的夹取发生在 effect + 下一帧，也可能短暂落在空白区。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：scroll 容器改用 callback ref + state 传给虚拟列表，确保节点出现后重新绑定监听；scroll 事件同步读取 metrics，不再等 rAF；resize 仍保留 rAF 合并。
  - 同组件：内容缩短时在 `useLayoutEffect` 内立即夹住 `scrollTop` 并同步刷新 metrics；虚拟 spacer/scroll body 增加稳定背景；overscan 从 8 组提高到 12 组，降低快速滚动边缘露空概率。
  - 同组件：复测后将虚拟化阈值提高到 160 组，截图里的 89 组 running slice 直接渲染，不再在滚轮事件里频繁触发 React 虚拟窗口更新；每个 group 加 `content-visibility: auto`，让 Chromium 跳过屏幕外绘制；大矩阵 overscan 提到 24 组。
  - 同组件：新增 `ASSET_COVERAGE_READING_FREEZE_MS=8000` 阅读冻结窗口；用户滚动/滚轮/拖动矩阵后，`snapshot` 与 live work 更新先排队，当前可见矩阵保持稳定，停下后再应用最新数据。
  - 同组件：虚拟化阈值再提高到 512 组；当前 332 资产完整矩阵也走直接渲染 + `content-visibility`，只把 600+ 超大矩阵留给虚拟化，彻底避开当前页面快滑时虚拟窗口追不上的黑底。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增快速滚动回归，模拟超大矩阵从顶部直接甩到底部，锁定虚拟窗口会立即切到尾部资产；新增 89 组中等列表和 332 组当前完整矩阵直接渲染回归，锁住平滑滚动路径；新增滚动后 polling 新快照不立即替换矩阵的阅读稳定回归。
  - `docs/modules/frontend/components.md`：同步记录资产覆盖虚拟列表的 scroll 同步刷新 / layout clamp / spacer 背景 / 500 组以下直接渲染 / 阅读冻结约束。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 被 `ERR_PNPM_IGNORED_BUILDS` 阻断。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 18 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需 `pnpm approve-builds`。
- **提交记录**：待提交。
- **本轮修改但未提交（资产覆盖快速滚动 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **下一步建议**：刷新完整资产覆盖矩阵后快速上下甩动；89 组左右的 running slice 应走直接渲染并明显更顺。滚动后 8 秒内 polling/live 更新不应替换正在看的矩阵。若完整 300+ 资产全量视图仍卡，再上浏览器 performance 采样看是 row 绘制、spinner 动画还是外层详情页布局造成。

---

### 2026-06-28 · EAS stdin batch liveness outcome 落库修复

- **本轮目标**：排查用户反馈的最新 Task run `pentest-chat-1782574914157-1` 仍在 EAS coverage gate 里反复被拦。
- **现场结论**：
  - `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 显示最后不是新资产扩分母，而是当前 wave 里 14 个资产缺 `GOLISH-EAS-LIVENESS` 终态。
  - `~/.golish/backend.log` 里 2026-06-28T04:32:29Z 的 `httpx <<'GOLISH_STDIN'` stdin 列表正好是这 14 个资产；04:32:51Z completion 后只看到 `background job structured output not detected`，没有 `background batch liveness outcomes stored`。
  - 根因：`commands/bridge_config.rs` 之前只从 `httpx -l <file>` / `nmap -sn -iL <file>` 读取批量探活输入，未识别 `httpx` 直接 stdin/heredoc 批量输入；当 httpx 零输出时，没有逐资产写 `empty`，gate 就一直按 `never attempted` 拦。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：新增 heredoc/stdin batch input 解析，`httpx <<'GOLISH_STDIN'` 也归类为 batch liveness；completion 写入每个 stdin 目标的 `GOLISH-EAS-LIVENESS` `found/empty` outcome。复用同一个 input-text helper 给 port/service batch 路径，保留原 input-file 行为。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步记录 `httpx` stdin/heredoc 批量探活也必须落 `technique_outcomes`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app batch_liveness --status-level fail` → 4 tests passed, 96 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机仍有前序记录的 pnpm ignored-build approval gate，且本轮是后端 targeted 修复。
- **提交记录**：未提交。
- **下一步建议**：重启 dev app 让该修复生效；当前正在跑的旧进程不会自动加载这段 Rust 代码。若继续同一 run，需要让 prober 再跑一次这批 stdin httpx，completion 后 coverage 应从 `never attempted` 收敛为 `found/empty`。

### 2026-06-28 · Stage expansion durable wave 自动续批

- **本轮目标**：继续用户确认的“新资产发现按批次汇总，当前批全部做完后再检查下一批，再集体跑一次 stage_run”的方案；在 Phase 1/2 的 no-schema cutoff + UI/read model 之上，落 Phase 3/4 durable wave 表和 runtime 自动续批。
- **已完成**：
  - `backend/crates/golish-db/migrations/20260625000001_stage_asset_waves.sql`：新增 `stage_asset_waves` / `stage_asset_wave_items`，纯 additive；一条 wave 固定一个 operation×org×stage 的 target 集合。
  - `backend/crates/golish-db/src/repo/stage_asset_waves.rs`：新增 repo helper，支持读取 running wave、创建 initial wave、promote 未分配 in-scope targets 到下一 wave、完成 wave；asset hash 用稳定摘要，仅作批次指纹。
  - `backend/crates/golish-agent-kit/src/db_traits/{types.rs,repo.rs}` + `backend/crates/golish-agent-app/src/ai/db_bridge/{orchestration.rs,mod.rs}`：新增 `StageAssetWaveView` 和 `DbRepoProvider` wave seam，app bridge 接到 golish-db repo。
  - `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：per-org gate 支持 durable wave asset override；有 wave 时用 wave asset list 冻结 `GateContext.in_scope_assets`，并同步过滤 typed asset map；无 durable wave 时回退 Phase 1 的 `stage_started_at` cutoff。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：wave-aware stage/org 先准备 current wave；specialist objective 显示当前批资产；gate PASS 后先 mark wave completed，再 promote 下一批并继续同 org；只有没有下一批时才写 `org_stage_completions`。达到自动 wave cap 时会创建下一批并 blocked，让后续 `stage_run` 从 running wave 接上。
  - 同步模块卡：`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-kit/db_traits.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`；更新计划和 feature evidence。
- **运行过的验证（实跑）**：
  - `cargo fmt`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-db stage_asset_wave --status-level fail`（cwd `backend`）→ 3 passed / 113 skipped。
  - `cargo nextest run -p golish-agent-kit external_attack_surface_enables_asset_wave_barrier_only coverage_preflight_does_not_block_on_next_wave_pending_cells --status-level fail`（cwd `backend`）→ 2 passed / 762 skipped。
  - `cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch --status-level fail`（cwd `backend`）→ 1 passed / 284 skipped。
  - `jq empty feature_list.json` → exit 0；`git diff --check` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；底层 `pnpm install` 被 `ERR_PNPM_IGNORED_BUILDS` 阻塞：`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`。
- **未跑/未通过**：full `just precommit` 未绿（原因如上）；live app rerun 尚未做，migration 需要 app 重启/apply 后才能验证真实 DB rows。
- **提交记录**：未 commit。
- **本轮修改但未提交（durable wave scope）**：`backend/crates/golish-db/migrations/20260625000001_stage_asset_waves.sql`、`backend/crates/golish-db/src/repo/{mod.rs,stage_asset_waves.rs}`、`backend/crates/golish-agent-kit/src/db_traits/{repo.rs,types.rs}`、`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,orchestration.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、相关模块卡、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、`feature_list.json`、`agent-progress.md`。
- **已知风险 / 下一步**：pass-token closeout 仍用既有 org completion token，尚未把 `wave_id/asset_hash` 折进 token；`run_tree.py` wave summary 也还没补。下一步应重启 app 让 migration apply，重新跑 EAS，确认 `stage_asset_waves` 生成、当前批 PASS 后 next wave 自动继续，且 UI next_wave_pending 不再挡当前提交。

---

### 2026-06-28 · stage expansion wave barrier Phase 1

- **本轮目标**：回应用户确认的方向：新发现资产不要实时撑大当前 EAS 覆盖分母；当前批次先全部完成，再检查新资产总和并集体触发下一批 `stage_run`。
- **已完成**：
  - 新增设计文档 `docs/design/2026-06-28-stage-expansion-wave-barrier.md`：定义 wave / seed asset / new asset / expansion barrier；明确当前问题是 UI 已有 `seed_assets/new_assets`，但 gate 仍用 live `targets.scope='in'` 分母。
  - 新增实现计划 `docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`：拆 Phase 1 无 schema 当前 wave freeze、Phase 2 barrier read model、Phase 3 durable wave tables、Phase 4 自动下一批 dispatch。
  - `feature_list.json` 新增 `stage-expansion-wave-barrier-2026-06-28`，状态 `in_progress`；notes 标明 Phase 3 涉及 migration，必须在动 DB schema 前再次确认。
  - 完成 Phase 1 no-schema current-wave freeze：
    - `StageSpec.asset_wave_barrier` + `external_attack_surface/spec.json` 开关。
    - `golish-db::repo::targets::list_in_scope_values_created_before` + `ReconTargetsPort::in_scope_values_created_before` + `DbRepoProvider::in_scope_assets_created_before`。
    - `submit_stage_deliverable` 预检、`stage_run` per-org gate、Task-mode stage close gate 三条路径都用 active `operation_state.stage_started_at` 冻结 EAS 当前 wave 资产轴；DB truth freshness 同步按该 cutoff 收敛。
    - `AgentBridge` 暴露 `harness_active_operation_id_handle` 给 submit tool 注册层读取 active operation id。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-bridge/agent_bridge.md`、`docs/modules/backend/golish-app-core/ports.md`、`docs/modules/backend/golish-db/repo.md`。
  - 完成 Phase 2 no-schema read model：
    - `ai_get_stage_asset_coverage` 仍展示本阶段新发现资产，但将 wave cutoff 后的新资产 cell 标为 `next_wave_pending`，并从当前 wave `total_assets` / pending / done 分母中排除。
    - `check_stage_asset_coverage` 压缩预检不再把 `next_wave_pending` 当作当前 gap，`ready_to_submit` 可在当前 wave 已完成时返回 true，并在 `next_action` 提醒下一批资产。
    - `StageAssetCoveragePanel` 将 `new_in_stage` 行显示为“下批”，summary `done/total` 只计算当前 wave；下批资产仍留在完整矩阵里可见。
  - 同步模块卡 Phase 2 行为：`docs/modules/backend/golish-agent-kit/tool_executors.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**:
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败，未进入 `just check` / `just test`。该阻塞与近期记录一致：本机 pnpm ignored-build approval gate。
  - `cd backend && cargo fmt` → exit 0。
  - `cd backend && cargo check -p golish-db -p golish-app-core -p golish-agent-kit -p golish-agent-app -p golish-agent-bridge -p golish-agent-runtime` → exit 0（8.09s；前一轮冷 check 36.56s 也 exit 0）。
  - `cd backend && cargo nextest run -p golish-agent-kit external_attack_surface_enables_asset_wave_barrier_only --status-level fail` → 1 test passed, 762 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-db list_in_scope_values_before_sql_adds_wave_cutoff --status-level fail` → 1 test passed, 112 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app next_wave --status-level fail` → 1 test passed, 97 skipped, exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit coverage_preflight_does_not_block_on_next_wave_pending_cells --status-level fail` → 1 test passed, 763 skipped, exit 0。
  - `cd backend && cargo check -p golish-agent-app -p golish-agent-kit -p golish-agent-runtime` → exit 0。
  - `pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx && pnpm exec tsc --noEmit --pretty false` → exit 1；仍被 pnpm ignored-build approval gate 拦截（`@swc/core` / `electron` / `esbuild`）。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx && ./node_modules/.bin/tsc --noEmit --pretty false` → 15 tests passed + typecheck exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
- **未跑**：`just precommit`；`./init.sh` 仍在 `pnpm install --silent` 的 ignored-build approval gate 处失败，未进入全量 check/test。
- **提交记录**：未提交。
- **本轮修改但未提交（本需求 scope）**：`docs/design/2026-06-28-stage-expansion-wave-barrier.md`、`docs/superpowers/plans/2026-06-28-stage-expansion-wave-barrier.md`、`feature_list.json`、`agent-progress.md`、`resources/harness/stages/external_attack_surface/spec.json`、`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`、`backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs`、`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-kit/src/tool_executors/security.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,recon.rs,evidence.rs}`、`backend/crates/golish-agent-app/src/ai/commands/{bridge_config.rs,stage_coverage.rs}`、`backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`、`backend/crates/golish-app-core/src/ports/recon/targets.rs`、`backend/crates/golish-db/src/repo/targets.rs`、`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、上述模块卡。当前工作树仍有此前任务留下的其它未提交文件，本轮未回滚。
- **下一步建议**：实现 Phase 3/4 前需要用户确认 migration：durable wave tables、current/next wave promotion ledger、当前 wave PASS 后自动 dispatch 下一批 `stage_run`。

---

### 2026-06-28 · Target 管理本级/子树计数口径优化

- **本轮目标**：回应用户截图反馈：Target 目标管理页左侧主公司显示 813，但右侧本公司只有 338，页面口径混乱且视觉别扭；先优化数量语义和右侧默认资产视图。
- **已完成**：
  - `frontend/lib/target-panel/org-tree.ts`：新增 `TargetCountSummary` / `summarizeTargetCounts` / `findOrgTreeNode`，把本组织 own 计数和含子公司 subtree 汇总拆开；保留 `countAllTargets` 作为递归汇总兼容入口。
  - `frontend/components/TargetPanel/OrgTreeSidebar.tsx`：左树主数字改为本组织自己的目标数，in-scope chip 也只看本组织；含子公司汇总只用弱化 `Σ` chip 展示，避免 813 被误读成本公司资产数。
  - `frontend/components/TargetPanel/{TargetGroupedView,OrgWorkspacePanel}.tsx`：右侧 workspace 同时接收本公司资产和子树资产；默认展示本公司资产，父公司有子公司资产时提供“本公司 / 含子公司”切换；顶部指标显示本公司、范围内、含子公司、子公司数。
  - `frontend/lib/i18n/{en,zh-CN}.json`：Target workspace tab 文案从“总览/字段”收紧为“资产/组织资料”，新增本公司/含子公司指标文案。
  - 同步模块卡：`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败。
  - `./node_modules/.bin/biome check --write frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json` → exit 0。
  - `./node_modules/.bin/biome check frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md docs/modules/frontend/lib.md` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.test.ts frontend/lib/target-panel/asset-groups.test.ts frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → 4 files / 51 tests passed。
  - `git diff --check -- frontend/lib/target-panel/org-tree.ts frontend/lib/target-panel/org-tree.test.ts frontend/components/TargetPanel/OrgTreeSidebar.tsx frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/OrgWorkspacePanel.tsx frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json docs/modules/frontend/components.md docs/modules/frontend/lib.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe`。
  - `pnpm format` → exit 1；`ERR_PNPM_IGNORED_BUILDS`，ignored build scripts: `@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`，需 `pnpm approve-builds`。
- **已记录证据**：`org-tree.test.ts` 新增回归锁定 `P` 本级 1 个目标、子树 4 个目标（含 1 个 out-of-scope）、2 个 descendant org；scoped typecheck / biome / vitest 全绿；full precommit 未绿的原因是本机 pnpm approval gate。
- **提交记录**：待提交。
- **本轮修改但未提交（TargetPanel UI scope）**：`frontend/lib/target-panel/org-tree.ts`、`frontend/lib/target-panel/org-tree.test.ts`、`frontend/components/TargetPanel/OrgTreeSidebar.tsx`、`frontend/components/TargetPanel/TargetGroupedView.tsx`、`frontend/components/TargetPanel/OrgWorkspacePanel.tsx`、`frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`agent-progress.md`。当前工作树仍有此前任务留下的其它未提交文件，本轮未修改也未回滚。
- **风险 / 未解决问题**：未做动作区收纳（hover 多 icon 仍偏挤），这是下一刀视觉优化；未启动 dev server 做截图 QA；`just precommit` 仍被本机 pnpm ignored-builds 阻塞。
- **下一步建议**：刷新 Target 页面，主公司左树应显示本公司 own 数（例：338）+ 弱化 `Σ 813` 汇总；右侧默认“本公司”，需要集团口径时点“含子公司”。若继续优化视觉，下一步收纳左树 hover actions 到更多菜单。

---

### 2026-06-28 · Task 模式 AI transcript 归档与 progress 瘦身

- **本轮目标**：回应用户澄清“task 模式的 AI 日志，不是删”；把旧 Task transcript 移出默认判断路径，同时精简 `agent-progress.md`，避免后续修改判断被旧日志噪声带偏。
- **已完成**：
  - `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts`：保留最新 8 个 `pentest-chat-*` + 2 个 `stage-run-*`；74 个旧 session / `title-gen-*` session 移到 `_archive/2026-06-28-task-transcripts/`，并写入 `ARCHIVE_MANIFEST.json`。
  - `scripts/run_tree.py`：默认 latest-session 候选跳过 `_*/.*` 归档目录和 `title-gen-*` 噪声；显式传 session 名或路径仍可追溯旧 transcript。
  - `agent-progress.md`：从 8047 行瘦到 635 行；288 条旧会话归档到 `docs/archive/agent-progress-archive-2026-06-28.md`；主文件保留最近 20 条和归档链接。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 | sed -n '1,18p'` → exit 0；默认命中最新真实 Task run `pentest-chat-1782574914157-1`。
  - `python3 -m py_compile scripts/run_tree.py` → exit 0。
  - `git diff --check -- scripts/run_tree.py agent-progress.md docs/archive/agent-progress-archive-2026-06-28.md` → exit 0。
  - `wc -l agent-progress.md docs/archive/agent-progress-archive-2026-06-28.md` → `agent-progress.md` 635 行，archive 7465 行。
- **未跑**：`just precommit`；本轮是 transcript 归档 + progress 文档瘦身 + 脚本默认候选过滤，且本机前序仍有 `pnpm` ignored-build approval gate。
- **提交记录**：待提交。
- **本轮修改但未提交（归档/降噪 scope）**：`scripts/run_tree.py`、`agent-progress.md`、`docs/archive/agent-progress-archive-2026-06-28.md`；另有本机 Test1 transcript 目录移动到 `_archive/2026-06-28-task-transcripts/`。
- **下一步建议**：后续排查 Task run 优先用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db`；需要旧日志时显式传 archive 下的 session 路径，不让默认 latest 被旧 `title-gen` / 历史 run 抢走。

---

### 2026-06-28 · fast resume thinking-mode tool_choice 兼容修复

- **本轮目标**：回应用户截图：输入裸 `继续` 后连续报 `ProviderError: Invalid status code 400 Bad Request ... "Thinking mode does not support this tool_choice"`。
- **根因**：
  - 上一轮为裸 resume 加了 native `tool_choice` lock，把第一轮强制到 `stage_run`。
  - 当前 provider/model 开了 thinking/reasoning mode，而该模式拒绝 API 层 `tool_choice`；于是请求在到达模型执行前被 provider 400 拒绝，UI 连续显示红错。
  - 二次截图后查 `~/.golish/backend.log`：运行代码已经带 `native_tool_choice_allowed` 日志，但 `provider=deepseek model=deepseek-v4-flash` 的 thinking 是 provider/model 默认行为，不一定体现在 `enable_thinking=true` / `reasoning` request 参数里；上一版只看 request 参数，因此漏判成 `native_tool_choice_allowed=true`。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`：当请求已经启用 explicit thinking/reasoning（OpenAI reasoning effort、`enable_thinking=true`、`chat_template_kwargs.enable_thinking=true`、非 excluded `reasoning`）时，不再发送 native/API `tool_choice`。
  - 同文件：新增 provider/model 默认 thinking 兼容判断；`deepseek-v4-flash` 这类 DeepSeek thinking-capable model 即使 request 没显式 thinking 参数，也不发送 native `tool_choice`。
  - 同文件：如果 provider 仍返回 `tool_choice + thinking/reasoning` 不兼容错误，立即剥掉 native `tool_choice` 并重试同一轮请求；prompt 级 forced-tool directive 和 dispatch 层拦截仍保留，所以 fast resume 语义不撤回。
  - 新增回归测试覆盖 thinking-mode suppress、错误识别和 `additional_params.tool_choice` 剥离。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only thinking_mode --status-level fail` → 12 passed / 271 skipped。
  - `cd backend && cargo fmt -p golish-agent-runtime && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only thinking_mode deepseek --status-level fail` → 13 passed / 271 skipped。
  - `cd backend && cargo check -p golish-agent-runtime -p golish-agent-bridge -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime -p golish-agent-bridge -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序 `pnpm install` / `./init.sh` 仍被 `ERR_PNPM_IGNORED_BUILDS` approval gate 阻断，本轮做 scoped backend hotfix 验证。
- **提交记录**：待提交。
- **本轮修改但未提交（hotfix scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`、`agent-progress.md`。
- **下一步建议**：重启/热重载后再输入裸 `继续`；不应再出现 thinking-mode `tool_choice` 400。首轮仍会收到 forced-tool prompt，dispatch 层会拒绝非 `stage_run` 工具。

---

### 2026-06-28 · 裸继续直进 stage_run fast resume

- **本轮目标**：回应用户“既然 resume 没问题，为什么点/说继续还要先 Thought、读 organizations/targets，能不能直接继续跑”的 UI/执行语义问题。
- **根因**：
  - `commands/core/chat.rs` 已经能把短“继续/continue”路由到同 chat session 的 checkpointed `TaskOrchestrator::resume`，所以断电/重启后的真续跑没问题。
  - 但 resume 后回到 active specialist stage 时，depth-0 primary 仍然先进完整 agentic loop；模型可能先 `manage_organizations` / `list_in_scope_targets` / 思考，再调用 `stage_run`，于是 UI 看起来像“继续前又重新思考/读库”。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`：新增“裸继续”窄口径识别；`继续/接着跑/continue the previous stage` 会启用 fast path，带“先解释/看日志/不要扫”等 steering 的继续仍走普通 resume。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/`：新增一次性 `stage_run` resume hint；仅在当前 stage 有 specialist 且已绑定 engagement root 时生效，非 specialist/rootless resume 不强制。
  - `backend/crates/golish-agent-bridge/src/`：把 `harness_forced_tool` 从 orchestrator side-channel 透传到 runtime，并在 isolated loop 返回后清空，避免 stale tool lock 泄漏到后续普通聊天。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/`：completion 阶段把 tool_choice 锁到 forced tool 并注入高优先级指令；dispatch 阶段拒绝同一批里的其它 allow-listed 工具。`stage_run` fast path 指令使用 `{"orgs":[]}`，由 runtime 按 bound engagement root 自动展开 authoritative subtree。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-agent-bridge/{agent_bridge,bridge_executor}.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bare_resume --status-level fail` → 1 passed / 95 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime forced_tool tool_choice submit_only --status-level fail` → 9 passed / 271 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit specialist_stages non_specialist --status-level fail` → 2 passed / 760 skipped。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`；本机前序 `./init.sh` / `pnpm install` 仍被 `ERR_PNPM_IGNORED_BUILDS` approval gate 阻断，本轮只做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交（fast resume scope）**：`backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,subtask_phases/execute.rs,types.rs}`、`backend/crates/golish-agent-bridge/src/{agent_bridge/{mod.rs,constructors/mod.rs,prepare.rs},bridge_executor/trait_impl.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,llm_stream_start.rs,turn/{executor.rs,state.rs,phases/{completion.rs,tool_dispatch.rs}}}`、`backend/crates/golish-agent-runtime/src/eval_support/{multi_turn.rs,single_turn.rs}`、`backend/crates/golish-agent-runtime/src/test_utils/context.rs`、相关模块卡、`agent-progress.md`。
- **下一步建议**：重启/热重载后，在已有 checkpoint 的 specialist stage 里输入裸 `继续`，首个可见动作应直接是 `stage_run` dispatch；如果输入“继续，但先看日志/解释原因”，仍应保持普通 resume 语义。

---

### 2026-06-28 · 资产覆盖大矩阵滚动卡顿修复

- **本轮目标**：回应用户截图反馈：资产覆盖完整矩阵快速滚动很卡，滚快后列表下方出现大片黑色空白。
- **根因**：
  - `StageAssetCoveragePanel` 在完整矩阵里直接渲染全部资产 group/row；EAS 批量扫描时常见 80+ 组、上百资产，每行还有 grid/border 与 live spinner。
  - 快速滚动时 Tauri/Chromium 需要持续重绘整张覆盖表，容易 checkerboarding；同时 live slice 变短时旧 `scrollTop` 可能落在新内容之外，看起来像底部黑掉。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：资产 group 数超过阈值时改为窗口化渲染，只挂可视窗口 + overscan 内的 group；小列表仍走原直接渲染路径。
  - 同组件：滚动/resize 读数用 rAF 合并；active/all 或 live slice 变化后夹住旧 `scrollTop`，避免内容缩短后留在越界滚动位置。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增 80 资产大矩阵回归，锁定虚拟列表路径只渲染可视 group。
  - 同步模块卡：`docs/modules/frontend/components.md`，记录完整覆盖矩阵大列表必须窗口化渲染。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败，底层仍是本机 `ERR_PNPM_IGNORED_BUILDS` approval gate。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 14 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 58 tests passed。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx docs/modules/frontend/components.md` → exit 1；底层 `pnpm install` 被 `ERR_PNPM_IGNORED_BUILDS` 阻断（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12`）。
- **未跑/未通过**：全量 `just precommit` 未绿；阻塞原因是本机 pnpm approve-builds gate，本轮用直接二进制完成 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（资产覆盖滚动性能 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入资产覆盖完整矩阵，80+ 资产时滚动应只重绘窗口内 group；如果仍看到黑块，再用同一截图定位是否是 active slice 内容本身过短，而不是渲染卡顿。

---

### 2026-06-28 · stage_run resume-skip 行误显示 queued 修复

- **本轮目标**：回应用户截图反馈：上一轮只有 3 个 org blocked，第二次补洞时后面已经 passed 的 org 也显示成 `Queued`，解释是否因为 gate/pass token/hash，并修掉 UI 误导来源。
- **根因**：
  - 最新 run.log 明确显示这轮模型实际只传了 3 个 blocked org：`stage_run filled missing requested org(s) ... requested_orgs=3 total_orgs=12 auto_added=[...]`。
  - runtime 会把 `stage_run` 入参补回当前 engagement root 的完整 organization subtree，这是为了保持 fan-out 分母与最终 pass-token/closeout gate 一致，避免模型漏传子公司导致阶段假通过。
  - 但旧实现为了让 UI 立刻看到完整分母，会先把所有 org seed 成 `queued`，然后 serial loop 轮到某个 org 时才查 `org_stage_completions` 并 resume-skip 为 `passed`。因此已经通过但排在 blocked org 后面的 rows，会短暂显示成 `Queued`，看起来像要重跑。
- **已完成**：
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：在初始 seed 前预查每个 org 的 `resume_skip_passed_at`；fresh PASS 的 org 直接 emit `passed` + skip 活动文案，只有真正待跑/待补的 org 才 emit `queued`。
  - 后续 serial loop 复用同一份 `resume_skips`，不再重复查询，也不会把已通过 worker 临时降级成 queued。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail` → 28 passed / 248 skipped（首次运行出现 unused warning，随后已修复）。
  - `cd backend && cargo check -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序多次记录 `pnpm` ignored-build approval gate（`ERR_PNPM_IGNORED_BUILDS`）会阻断全量前端安装/检查，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`agent-progress.md`。
- **结论给用户**：正常语义确实是补 blocked org；runtime 补回全树是为了完整阶段 closeout，不是要求重跑 passed org。hash/pass token 只在最终 close 阶段从 DB ledger 重算，不要求把后面的 passed worker 再排队跑一遍。

---

### 2026-06-28 · EAS 批量 liveness coverage 落库修复

- **本轮目标**：回应用户“那就修一下”并解释关机后续跑是否是真 resume；修补最新 Test1 run 中批量 `httpx -l` / `nmap -sn -iL` 探活空结果不落 `GOLISH-EAS-LIVENESS` 的问题。
- **根因**：
  - 最新 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782574914157-1/run.log` 显示 sub-agent 多次跑 `nmap -sn -iL ...` 与 `httpx -l ...` 后，gate 仍报同一批 `(asset × GOLISH-EAS-LIVENESS) never attempted`。
  - 旧后台 completion 只对 `naabu`/`masscan` 批量 PORT 和 `whatweb`/`nmap -iL` 批量 SERVICE 写 `technique_outcomes`；批量探活零输出只有 evidence，没有按 input file 每个目标写 terminal outcome。
  - `nmap -sn -iL` 是探活命令，不应被泛化的 `nmap -iL` service 分支误记为 SERVICE-FINGERPRINT。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：新增 `maybe_store_background_batch_liveness_outcomes`；后台成功完成 `httpx -l` / `nmap -sn -iL` 后读取 input file，对每个非 CIDR host/IP 写 `technique_outcomes(GOLISH-EAS-LIVENESS)`，输出命中为 `found`，无命中为 `empty`。
  - `bridge_config.rs`：新增批量命令意图分类；`nmap -sn/-sP -iL` 只走 LIVENESS，`nmap -sV/-A -iL` 才走 SERVICE-FINGERPRINT，避免探活扫描误落服务识别。
  - `bridge_config.rs`：补 `httpx -l` input-file 解析与分类回归测试。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 14 passed / 80 skipped。
  - `cd backend && cargo check -p golish-agent-app` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`；本机前序多次记录 `pnpm` ignored-build approval gate（`ERR_PNPM_IGNORED_BUILDS`）会阻断全量前端安装/检查，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`agent-progress.md`。
- **续跑解释**：当前代码通过 `latest_resumable_by_session` 找同 chat session 的 non-terminal operation，并从 `operation_state.state_blob` 恢复；`stage_run_workers[stage][org_id]` 持久化每个 org 的 sub-agent chain id，所以断电/关机重启后能继续未完成 worker，而不是只靠日志重放。
- **下一步建议**：重启 app 后继续当前 EAS；新的后台 completion 日志应出现 `background batch liveness outcomes stored`，再用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 看 LIVENESS pending cells 是否下降。

---

### 2026-06-28 · EAS submit retry 批量 service coverage 修复

- **本轮目标**：回应用户“刚刚跑了一次逻辑，为什么一直报错提交过不去”，诊断最新 Test1 run，并修掉 EAS repair/submit 在批量服务指纹阶段继续卡住的问题。
- **根因**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db` 显示本轮 submit 并不是 JSON schema 主因；最终 deterministic gate 卡在 `external_attack_surface coverage_complete`，最后仍有 130 个 cell 没有 terminal state，主要是 `GOLISH-EAS-SERVICE-FINGERPRINT`，另有部分 `LIVENESS`。
  - DB 自诊断显示本 session 已有 `GOLISH-EAS-LIVENESS found:165 / empty:33`、`GOLISH-EAS-PORT found:60 / empty:5`、`GOLISH-EAS-SERVICE-FINGERPRINT found:40`，说明工具确实跑了一部分，但 service fingerprint 分母还远未闭合。
  - run_tree 中 repair 阶段多次出现 `coverage-gap repair blocks list-file probes`；`StageRefiner` 文案要求 batch-first（`input_lines + {{input_file}}`），但 `SubmitRepairMode` 又拦 list-file/multi-target，导致模型只能单目标 nmap/whatweb，面对数百资产必然循环。
  - 批量 `whatweb --input-file` / `nmap -iL` 的后台 evidence 会存在，但旧 coverage fact 只能从命令行解析单个 target；命令行里只有 input file 路径，不能给每个 input target 写 `GOLISH-EAS-SERVICE-FINGERPRINT` terminal outcome。
- **已完成**：
  - `backend/crates/golish-sub-agents/src/executor_types.rs`：coverage-gap repair 允许 `pentest_run` 用 `input_lines` / list-file 批量处理 sibling gap targets；仍阻止 CIDR/range sweep、隐藏 list file（没给可校验 `input_lines`/`stdin`）以及 coverage_gap_actions 外的目标。
  - `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：后台 completion 新增批量 service outcome 写点；`whatweb --input-file` / `nmap -iL` 成功完成后读取 input file，为每个 host/URL 写 `technique_outcomes(GOLISH-EAS-SERVICE-FINGERPRINT)`，输出命中为 `found`，无命中为 `empty`。
  - `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`bridge_config.rs`：补回归测试，锁住批量 input_lines 允许、隐藏 list file 阻止、`--input-file=...` 解析、service output target 匹配，以及 IP 前缀不误命中。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db > /tmp/golish-run-tree-1782574914157-full.txt` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 12 passed / 80 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair --status-level fail` → 6 passed / 104 skipped。
  - `cd backend && cargo check -p golish-agent-app -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-agent-app -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
- **未跑**：`just precommit`；本机前序 `./init.sh` / `just install` 已稳定被 pnpm `ERR_PNPM_IGNORED_BUILDS`（`@swc/core` / `electron` / `esbuild`）阻塞，本轮做 scoped backend 验证。
- **提交记录**：待提交。
- **本轮修改但未提交**：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-sub-agents.md`、`agent-progress.md`。
- **下一步最佳动作**：重启 app 后继续/重跑 EAS；repair 阶段应能用 `input_lines + {{input_file}}` 批量扫 gate 点名资产，后台 whatweb/nmap 完成后 service fingerprint terminal outcomes 应进入 `technique_outcomes`，再用 `check_stage_asset_coverage` 看 pending cells 是否下降。

---

### 2026-06-28 · SubAgent detail refiner 指令卡片化

- **本轮目标**：回应用户截图里 `Resuming submit repair: STAGE REFINER DIRECTIVE...` 被当成普通 `Agent Output` 展示，导致 detail 里系统纠错指令看起来很怪的问题。
- **根因**：
  - `golish-sub-agents` 在恢复 submit repair mode 时会发一条 `SubAgentTextDelta`，内容是 `Resuming submit repair: ... STAGE REFINER DIRECTIVE...`。
  - 前端 `SubAgentDetailView` 之前把所有 text delta 都渲染为普通 `Agent Output`，没有区分 StageRefiner / harness repair 指令和 agent prose。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：新增 `parseStageRefinerDirectiveSummary`，识别 `STAGE REFINER DIRECTIVE` 并解析 stage、repair kind、gap/action 数、allowed/forbidden tools、batch-first 标记。
  - `SubAgentDetailView`：StageRefiner directive 现在渲染成紧凑 `Stage Refiner` 修复卡，默认折叠原始长指令，只显示 `Coverage Gap` / `289 gaps` / `Batch-first` / allowed tools 等摘要。
  - `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增 3 个回归，锁定普通输出不受影响、CoverageGap 指令摘要、EvidenceRefs 指令摘要。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 submit-repair / StageRefiner directive 不应再作为普通 Agent Output 展示。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `./node_modules/.bin/biome check --write frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 1 file / 44 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `just check-fe` → exit 1；包装层只输出 recipe failure。
  - `pnpm check` → exit 1；底层为 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未跑；`./init.sh` / `just check-fe` 均被本机 pnpm ignored-build approval gate 阻断，本轮做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（refiner detail UI scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，截图里那类 `Resuming submit repair` 长段落应显示为一条 `Stage Refiner` 卡；点 `Details` 才展开完整 directive。

---

### 2026-06-28 · pentest_run 工具卡摘要降噪修复

- **本轮目标**：回应用户截图里卡片显示 `Running Nmap nmap -sV -iL [input file] ...`，标题和后缀重复、后缀命令过长的问题。
- **根因**：
  - 上一轮把标题从 raw tool id 改成动作文案后，`pentest_run` 的参数摘要仍然返回完整 `<tool> <args>` 命令串；因此标题 `Running Nmap` 后面又出现 `nmap ...`。
  - `SubAgentDetailView` 的 coverage 维度推断之前间接依赖摘要里出现 `-sV`；如果直接把命令串从摘要里删掉，需要让推断显式读取 raw args/action label，避免 SERVICE 维度丢失或误判。
- **已完成**：
  - `frontend/lib/tools.ts`：`getToolActionLabel("pentest_run")` 改成意图文案：`nmap -sV` → `Probing services`，`naabu/masscan` → `Scanning ports`，`httpx` → `Checking web services`，`whatweb` → `Fingerprinting web services` 等。
  - `frontend/lib/tools.ts`：`getToolPrimaryArg("pentest_run")` 不再返回完整原始命令；现在返回短上下文，例如 `Nmap · batch 3 targets (...) · ports 80,443,10180`、`Naabu · batch ... · top 1000 ports`。
  - `frontend/components/ToolExecutionCard/ToolExecutionCard.tsx`：标题也接入 `getToolActionLabel`，和聊天卡 / sub-agent detail 保持一致。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：coverage 维度推断显式看 action + raw args；隐藏 `-sV` 后仍能推断 SERVICE，同时 `Checking web services` 不会误判 SERVICE。
  - `frontend/lib/tools.test.ts`、`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：新增/更新回归，锁定 nmap service probe、naabu batch 摘要、coverage 推断。
  - `docs/modules/frontend/{components,lib}.md`：同步模块卡，记录工具卡标题用动作文案、`pentest_run` 摘要避免 `Running Nmap nmap ...`。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 3 files / 66 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx docs/modules/frontend/lib.md docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/components/ToolExecutionCard/ToolExecutionCard.tsx` → exit 1；底层在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡摘要降噪 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`frontend/components/ToolExecutionCard/ToolExecutionCard.tsx`、`frontend/components/SubAgentDetailView/{SubAgentDetailView.tsx,stripAgentXmlTags.test.ts}`、`docs/modules/frontend/{components,lib}.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，截图里的行应类似 `Probing services  Nmap · batch ... · ports 80,443,10180`，而不是 `Running Nmap nmap -sV -iL ...`。

---

### 2026-06-28 · 工具卡片人类动作文案修复

- **本轮目标**：回应用户反馈工具卡片直接显示 `wait_for_background_jobs` 这类下划线内部名很难受，希望像 Cursor 一样显示“正在做什么”。
- **根因**：
  - 聊天流工具卡和 pending approval 卡片头部直接或间接展示内部 tool id；`SubAgentDetailView` 折叠工具行更是直接渲染 `tool.name`。
  - `getToolPrimaryArg` 只负责参数摘要（如 timeout / command），没有单独的人类动作文案层，导致“工具是什么”和“正在做什么”混在一起。
- **已完成**：
  - `frontend/lib/tools.ts`：新增 `getToolActionLabel`，把内部 tool id 转成动作句子；例如 `wait_for_background_jobs` → `Waiting for background jobs`，`pentest_run` + `tool_name=whatweb` → `Running WhatWeb`，未知工具也 fallback 为 `Using Custom Internal Tool` 而不是露下划线。
  - `frontend/components/AIChatPanel/ToolCallSummary.tsx`：聊天工具卡和 pending approval 卡头部改用 `getToolActionLabel`；raw tool id 只放在 `title` 里用于 hover/debug。
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：sub-agent detail 折叠工具行头部改用动作文案，参数摘要继续显示在后面（如 `wait up to 180s`）。
  - `docs/modules/frontend/{components,lib}.md`：同步模块卡，记录折叠工具卡不直接展示 `snake_case` tool id。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0（fixed 1 file）。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 3 files / 63 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx docs/modules/frontend/lib.md docs/modules/frontend/components.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 1；底层在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡动作文案 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`frontend/components/AIChatPanel/ToolCallSummary.tsx`、`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`docs/modules/frontend/{components,lib}.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，工具卡标题应显示类似 `Waiting for background jobs` / `Running WhatWeb`，下一行或旁边再显示 `wait up to 180s` / batch targets 等参数摘要；不应再把 `wait_for_background_jobs` 作为卡片主标题。

---

### 2026-06-28 · wait_for_background_jobs 折叠摘要修复

- **本轮目标**：回应用户截图里 `wait_for_background_jobs` 不展开时看不到等待秒数，并确认 `timeout_secs` 的来源。
- **结论 / 根因**：
  - 后端 `WaitForBackgroundJobsTool` 的 `timeout_secs` 是可选参数；不传时默认 `DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS=300_000`（300s），最大 900s。
  - 截图里的 `timeout_secs: 180` 是模型本次实际传入的参数，不是前端默认值；前端之前只在展开 Input 后显示完整 args，折叠摘要没有 `wait_for_background_jobs` 分支。
- **已完成**：
  - `frontend/lib/tools.ts`：`getToolPrimaryArg("wait_for_background_jobs", args)` 现在返回折叠态摘要：传参时显示 `wait up to 180s`，未传时显示 `default wait up to 300s`，自定义 `poll_interval_ms` 时追加 `poll ...ms`。
  - `frontend/lib/tools.test.ts`：新增 3 个回归，锁定显式 timeout、默认 timeout、poll interval 的折叠摘要。
  - `docs/modules/frontend/lib.md`：同步模块卡，记录 `wait_for_background_jobs` 折叠态必须显示实际 timeout / 默认 300s。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `./node_modules/.bin/biome check --write frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts` → 1 file / 11 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts docs/modules/frontend/lib.md` → exit 0。
  - `just precommit` → exit 1；失败在 `fmt-fe` recipe。
  - `just fmt-fe` / `just check-fe` / `just test-fe` → exit 1；just 包装层只输出 recipe failure。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` / `pnpm test:run frontend/lib/tools.test.ts` → exit 1；底层均在执行脚本前触发 `ERR_PNPM_IGNORED_BUILDS`（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑/未通过**：全量 `just precommit` 未绿，阻塞于既有 pnpm ignored-build approval gate；本轮已做 scoped 前端验证。
- **提交记录**：未 commit。
- **本轮修改但未提交（wait 折叠摘要 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，`wait_for_background_jobs` 折叠行应显示 `wait up to 180s`；若模型没传 timeout，则显示 `default wait up to 300s`，方便直接判断是模型自填还是默认。

---

### 2026-06-28 · EAS 批量扫描 coverage 落库与 live 匹配修复

- **本轮目标**：回应用户截图里 EAS 资产覆盖显示 `naabu -list [input file] ... | batch 226 targets` 正在跑，但覆盖面板显示 `0 组 / 0 资产`，并且“扫描完了提交过不了”的问题。
- **根因**：
  - 最新 Test1 run 的 submit gate 明确因为 EAS coverage incomplete BLOCK：仍有大量 `(asset × LIVENESS/PORT/SERVICE-FINGERPRINT)` 格子是 never attempted；这不是单纯前端显示问题。
  - 前端 `SubAgentDetailView` 的 live work 匹配只看命令文本/单目标参数，没读 `pentest_run.input_lines` / `stdin`；批量命令里目标都在输入文件/批量参数中，所以 UI 只能显示“运行中但尚未匹配到资产行”。
  - 后台 job completion 之前只拿 8KB `stdout_tail` 做 structured output 解析；批量 `naabu` / `whatweb` 这类长输出可能已经 append evidence，但完整结果没有写回 targets/ports/fingerprints，coverage truth 仍缺。
  - `naabu -silent` 对无开放端口资产是零输出；旧逻辑没把 input file 中“扫过但无结果”的 host/IP 写入 `technique_outcomes`，gate 会把它们当 never attempted，而不是 checked-empty。
- **已完成**：
  - `frontend/lib/tools.ts` 导出 `getPentestRunInputLines`；`SubAgentDetailView` live coverage 资产提取复用它，能从 `pentest_run.input_lines` / `stdin` 展开批量资产。
  - `backend/crates/golish-core/src/agent_session.rs` 的 `AgentToolContext` 增加 `organization_id`；主 agent 用 `harness_org_id`，sub-agent 用 `active_org_id_override`，后台 job completion 继承该 org。
  - `background_jobs::JobCompletion` 增加 `organization_id`；`bridge_config.rs` 的后台 structured landing 优先读取 `background_jobs::manager().snapshot(job_id).stdout`，fallback completion tail，并调用 `maybe_detect_and_store_via_context` 带 org context。
  - `bridge_config.rs` 对成功完成的 `naabu -list` / `masscan -iL` 批量端口扫描读取 input file，把每个非 CIDR host/IP 的 `GOLISH-EAS-PORT` outcome 写入 `technique_outcomes`：有开放端口 `found`，无开放端口 `empty`。
  - 同步模块卡：`docs/modules/frontend/{components,lib}.md`、`docs/modules/backend/{golish-core.md,golish-app-core.md,golish-agent-app/ai.md,golish-agent-runtime/agentic_loop.md,golish-sub-agents/executor.md}`。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check frontend/lib/tools.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → 2 files / 49 tests passed。
  - `cd backend && cargo fmt -p golish-core -p golish-app-core -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-app-core background_jobs --status-level fail` → 13 passed / 31 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 9 passed / 80 skipped。
  - `cd backend && cargo check -p golish-core -p golish-app-core -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit`（本工作区已有大量未提交跨模块改动，且前序记录显示 `./init.sh`/pnpm install 被 ignored-build approval gate 阻断；本轮做 scoped 前后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（EAS batch coverage landing scope）**：`frontend/lib/tools.ts`、`frontend/components/SubAgentDetailView/{SubAgentDetailView.tsx,stripAgentXmlTags.test.ts}`、`backend/crates/golish-core/src/agent_session.rs`、`backend/crates/golish-app-core/src/background_jobs.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`、上述模块卡、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑/继续 EAS；正在运行的 `naabu -list [input file]` 应能在资产覆盖里匹配到批量资产，后台完成后 PORT 的 found/empty outcome 会落入 `technique_outcomes`。SERVICE-FINGERPRINT 对“无开放端口”的 not_applicable 仍依赖 gate/submit 语义后续扩展或模型自报 note，不在本轮 DB 投影里伪造。

---

### 2026-06-28 · pentest_run 批量输入摘要修复

- **本轮目标**：回应用户截图里 `naabu -list {{input_file}} -top-ports 1000 -s c -silent` 连续显示 4 次，解释是否重复执行，并修掉 UI 摘要误导。
- **根因**：
  - 最新 Test1 transcript 里 4 次 `naabu` 并不是同一批目标：`input_lines` 分别为 96 / 76 / 55 / 34 条；实际执行时后端也替换成了不同的 `.golish/tool-inputs/pentest-input-*.txt` 临时文件。
  - 前端工具卡共用 `getToolPrimaryArg`，之前只显示 `tool_name + args`，没有显示 `input_lines` / `stdin`，所以所有 list-file 批量命令都看起来像同一条模板命令重复跑。
- **已完成**：
  - `frontend/lib/tools.ts`：`pentest_run` 摘要现在会统计 `input_lines` / `stdin`，显示 `batch N targets (first ... last)`；带批量输入时把 `{{input_file}}` / `{{targets_file}}` / `{{hosts_file}}` / `{{urls_file}}` / `{input_file}` / `$GOLISH_INPUT_FILE` 展示为 `[input file]`。
  - `frontend/lib/tools.test.ts`：补 `naabu -list {{input_file}}` 和 `httpx stdin` 批量摘要回归。
  - `docs/modules/frontend/lib.md`：同步模块卡，记录工具卡共享摘要入口的 batch 展示规则。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/lib/tools.test.ts` → 1 file / 8 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/lib/tools.ts frontend/lib/tools.test.ts docs/modules/frontend/lib.md agent-progress.md` → exit 0。
- **未跑**：`just precommit`（本机 `./init.sh` / `pnpm install` 仍受 `ERR_PNPM_IGNORED_BUILDS` 策略阻断；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（工具卡批量摘要 scope）**：`frontend/lib/tools.ts`、`frontend/lib/tools.test.ts`、`docs/modules/frontend/lib.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后，同样的 `naabu` 批量卡应显示类似 `naabu -list [input file] ... | batch 96 targets (113.105.78.99 ... 120.233.149.95)`，不再误以为同一命令重复执行。

---

### 2026-06-28 · stage_run 续跑 org 子树补齐修复

- **本轮目标**：回应用户指出的“继续逻辑有问题，有时 `stage_run` 总是少几个资产”，定位续跑/repair 阶段为什么漏部分 org/资产，并做 runtime 侧兜底。
- **根因**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db` 显示同一 run 中 `target_intel` 的 `stage_run` 入参有 12 个 org；后续 EAS continuation 的 `stage_run` 入参只有 10 个 org；repair 轮进一步只剩 6 个 org。
  - 旧 `stage_run_call.rs` 在已绑定 `harness_org_id` 时只会把 subtree 外的 org 丢掉，但不会把模型少传的 subtree 内 org 补回来；续跑/修复轮一旦靠模型重建 `orgs` 数组，就会让部分子公司及其资产完全不进入 fan-out 分母。
- **已完成**：
  - `golish-agent-kit::db_traits` 新增 `OrgScopeUnit` 与 `org_subtree_units` trait，保留默认 fallback 给测试 double。
  - `golish-db::repo::organizations::subtree` 新增 read-only recursive CTE，返回 root + descendants 完整 organization 行；无 schema / migration。
  - `golish-agent-app` 的 DB bridge 通过 `organizations::subtree` 实现 `org_subtree_units`。
  - `stage_run_call.rs` 在 `harness_org_id` 已绑定时以 DB organization subtree 作为权威 fan-out 集合：模型传入 `orgs` 只保留 ownership hint；缺失的 subtree org 会自动补回，subtree 外 org 会记录并拒绝；工具返回新增 `scope_source` / `requested_orgs` / `auto_added_orgs` / `rejected_orgs`。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-db.md`；`docs/modules/INDEX.md` 状态仍为 ✅，无需状态列变更。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate：`ERR_PNPM_IGNORED_BUILDS`）。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782574914157-1 --full --db > /tmp/golish-run-tree-1782574914157.txt` → exit 0；证据显示同 run 内 `stage_run` org 入参从 12 → 10 → 6。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime authoritative_subtree_fills_missing_requested_orgs --status-level fail` → 1 passed / 275 skipped。
  - `cd backend && cargo check -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail` → 28 passed / 248 skipped。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-db --all-targets -- -D warnings` → exit 0。
  - `python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 已在 pnpm install/build approval gate 阶段失败；本轮做 scoped Rust/JSON/doc 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（stage_run 续跑 org 子树补齐 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,recon.rs}`、`backend/crates/golish-db/src/repo/organizations.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-db.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app 后用同一个 Test1 engagement 继续跑 EAS；观察 `stage_run` tool result 的 `scope_source` 应为 `engagement_org_subtree`，`total_orgs` 应回到 DB root subtree 数量，即使模型只传 blocked org 或少传子公司，也会通过 `auto_added_orgs` 补齐。

---

### 2026-06-28 · EAS 批量探测路径修复

- **本轮目标**：回应用户发现 EAS 阶段 `httpx` 等工具被模型一个个调用、没有利用工具批量能力的问题。
- **根因**：
  - EAS `methodology.md` 已写 `httpx` 应批量跑，但 prober fallback prompt、primary stage 描述和 StageRefiner coverage-gap repair hint 仍会把 gap 拆成 `httpx -u <asset>` / `naabu -host <asset>` / `nmap ... <asset>` 这类单资产提示。
  - `resources/toolsconfig/httpx.json` 的 skills 推荐 `-json`，但 output config 仍是 `format=text`；批量 JSONL 输出可能出现“工具跑了但 parser 不解析、不落 targets/fingerprints”，导致 DB truth 缺口继续触发补洞。
  - `naabu` / `nmap` toolsconfig 没显式暴露 `-list` / `-iL` 批量参数和 bulk skills，模型看不到一等批量入口。
  - 更深一层：`pentest_run.args` 本来不是固定参数，但 `pentest_list_tools` 只把 `skills[].args` 暴露给模型，没暴露完整 `params`；同时 `pentest_run` 没有结构化 `stdin/input_lines`，导致模型即使想 batch 也很容易退化成一资产一调用。
  - `naabu` / `masscan` / `nmap` / `whatweb` / `gowitness` 这类工具的批量入口多是 list-file 参数，不是纯 stdin；之前没有 `{{input_file}}` 这类运行期占位，AI 没法可靠创建 hosts.txt，仍会抄单目标 recipe。
- **已完成**：
  - `resources/toolsconfig/httpx.json`：改为 `output.format=json_lines`，补 JSONL fields 映射（`ip` 取 `a[0]`，避免把 IP 数组字符串写进 `real_ip`），并保留旧文本 pattern fallback；批量 skills 改为带 `-json -sc -title -td -server`。
  - `resources/toolsconfig/naabu.json` / `nmap.json`：显式加入 `-list` / `-iL` 参数与 batch skills，batch skill 统一使用 `{{input_file}}`。
  - `resources/toolsconfig/masscan.json` / `whatweb.json` / `gowitness.json`：补 list-file batch 参数与 bulk skills，覆盖 EAS 端口发现、Web 指纹、截图工具，不只修 `httpx`。
  - `backend/crates/golish-pentest-app/src/pentest_ai/list_tools.rs`：`pentest_list_tools` 现在返回 `params`、`batching`、`usage_hint`，明确 skills 是示例 recipe，不是固定调用签名；bulk skills 会排在前面并带 `batch: true`。
  - `backend/crates/golish-pentest-app/src/pentest_ai/run.rs`：`pentest_run` schema 增加 `stdin` / `input_lines`；无 `{{input_file}}` 时用 quoted heredoc 喂 stdin，有 `{{input_file}}` 时自动写 workspace `.golish/tool-inputs/` 临时目标文件并替换占位符，支撑 `naabu -list` / `masscan -iL` / `nmap -iL` / `whatweb --input-file` / `gowitness file -f`。
  - `resources/harness/stages/external_attack_surface/methodology.md`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/{prompts/mod.rs,subtask_phases/execute.rs}`：统一 EAS/prober 为 batch-first 口径，primary 通过 `stage_run` 扇出 prober，并明确 list-file 工具使用 `{{input_file}} + input_lines`。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`：EAS coverage-gap directive 现在按 technique 聚合同类 gap，明确要求少量批量 `pentest_run`，并把 command hints 从单资产命令改成 batch hints。
  - `backend/crates/golish-pentest/src/output_parser.rs`：新增回归测试，锁定真实 `resources/toolsconfig/httpx.json` 同时解析 JSONL 和旧文本 fallback。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（延续本机 pnpm install gate）。
  - `python3 -m json.tool resources/toolsconfig/httpx.json` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/naabu.json` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/nmap.json` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-pentest` → exit 0。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-pentest -- --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_toolsconfig_parses_jsonl_and_text_fallback --status-level fail` → 1 passed / 153 skipped。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_json_parse --status-level fail` → 1 passed / 153 skipped。
  - `cd backend && cargo nextest run -p golish-pentest test_httpx_toolsconfig_parses_jsonl_and_text_fallback test_httpx_json_parse --status-level fail` → 2 passed / 152 skipped（补充锁定 `ip=a[0]` 解析为单 IP）。
  - `cd backend && cargo nextest run -p golish-agent-kit eas_coverage_gap_instruction_is_batch_first --status-level fail` → 1 passed / 759 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit external_attack_surface_charter_surfaces_liveness_technique --status-level fail` → 1 passed / 759 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents test_prober_prompt_is_active_surface --status-level fail` → 1 passed / 108 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills input_lines_become_stdin_payload stdin_payload_wraps_command_in_quoted_heredoc heredoc_delimiter_avoids_payload_collision --status-level fail` → 4 passed / 87 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills input_lines_become_stdin_payload stdin_payload_wraps_command_in_quoted_heredoc heredoc_delimiter_avoids_payload_collision input_file_placeholder_writes_target_file input_without_file_placeholder_uses_stdin shell_quote_handles_single_quotes --status-level fail` → 7 passed / 87 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app list_tools_exposes_params_and_batching_not_only_skills --status-level fail` → 1 passed / 93 skipped（锁定 bulk skills 排序 + `batch` 标记）。
  - `cd backend && cargo nextest run -p golish-agent-kit eas_coverage_gap_instruction_is_batch_first external_attack_surface_charter_surfaces_liveness_technique --status-level fail && cargo nextest run -p golish-sub-agents test_prober_prompt_is_active_surface --status-level fail` → 3 tests passed（锁定 `naabu` / `nmap` / `whatweb` / `gowitness` 的 `{{input_file}}` 批量提示）。
  - `cd backend && cargo check -p golish-pentest -p golish-agent-kit -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `cd backend && cargo check -p golish-pentest-app -p golish-pentest -p golish-agent-kit -p golish-sub-agents` → exit 0。
  - `cd backend && cargo clippy -p golish-pentest-app -p golish-pentest -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings` → exit 0。
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-agent-kit -p golish-sub-agents -p golish-pentest -- --check` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/httpx.json >/dev/null && python3 -m json.tool resources/toolsconfig/naabu.json >/dev/null && python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null && python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `python3 -m json.tool resources/toolsconfig/httpx.json >/dev/null && python3 -m json.tool resources/toolsconfig/naabu.json >/dev/null && python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null && python3 -m json.tool resources/toolsconfig/masscan.json >/dev/null && python3 -m json.tool resources/toolsconfig/whatweb.json >/dev/null && python3 -m json.tool resources/toolsconfig/gowitness.json >/dev/null && python3 -m json.tool feature_list.json >/dev/null` → exit 0。
  - `rg -n "{{hosts}}|{{urls}}|-host {{target}}|{{input_file}}" resources/toolsconfig/{httpx,naabu,nmap,masscan,whatweb,gowitness}.json backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs resources/harness/stages/external_attack_surface/methodology.md` → 只剩单目标 skills 仍含 `-host {{target}}`，所有 batch skills/prompt 均使用 `{{input_file}}`。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 仍在 pnpm install gate 失败；本轮做 scoped Rust/JSON 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（EAS 批量探测 scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/{prompts/mod.rs,stage_refiner.rs,subtask_phases/execute.rs}`、`backend/crates/golish-pentest-app/src/pentest_ai/{list_tools.rs,run.rs}`、`backend/crates/golish-pentest/src/output_parser.rs`、`backend/crates/golish-sub-agents/src/defaults/{prompts/execution_planning.rs,tests.rs}`、`resources/toolsconfig/{httpx,naabu,nmap,masscan,whatweb,gowitness}.json`、`resources/harness/stages/external_attack_surface/methodology.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-pentest-app/pentest_ai.md`、`docs/modules/backend/golish-sub-agents/defaults.md`、`agent-progress.md`、`feature_list.json`。
- **下一步建议**：重启 app 后重新跑 EAS；观察 prober 是否先调用 `pentest_list_tools` 读取 `params/batching`，再用少量 `pentest_run(args=..., input_lines=[...])`：`httpx` 可 stdin/`-l {{input_file}}`，`naabu` 用 `-list {{input_file}}`，`masscan`/`nmap` 用 `-iL {{input_file}}`，`whatweb` 用 `--input-file={{input_file}}`，`gowitness` 用 `file -f {{input_file}}`。

---

### 2026-06-28 · 资产覆盖运行态页面跳动修复

- **本轮目标**：回应用户截图里完整资产覆盖页运行时顶部/当前资产区域一直跳动、刷新感很强的问题。
- **根因**：
  - `StageAssetCoveragePanel` 之前只在 live work 全部清空时短暂保留上一帧；如果运行中的 work item 切换、事件批次短暂漏掉某个 item，active slice 会立即缩小/扩大，导致「正在做的资产」区域频繁重排。
  - 资产覆盖 summary chips、live count、顶部运行状态条和外层 panel header 的计数都依赖内容宽高自适应；running badge / 数字出现消失时会推动同一行元素位置，看起来像整块在刷新。
- **已完成**：
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：新增 `mergeDisplayLiveWorkItems`，运行中 work 切换时按 id 合并 incoming + 上一帧 display，并用 `LIVE_WORK_RETENTION_MS=3500` 延迟裁剪消失的 item；短暂轮询空隙不再让 active rows 立刻闪空或换位。
  - 同文件把完整矩阵 header、`LiveFocusBar`、panel/collapsible header 的 summary/live chips 改成固定高度 / 最小宽度 / `tabular-nums` 槽位；live count 为 0 时保留 invisible 槽，避免右侧 `Live` / summary 位置左右跳。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增回归，锁定 live work 从资产 A 切到资产 B 时短窗口内保留 A+B，窗口后再裁剪旧资产；更新旧保留窗口测试使用导出的常量。
  - `docs/modules/frontend/components.md`：同步模块卡，记录完整资产覆盖页运行态必须保留上一帧 active slice 并使用固定槽位。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate 延续既有环境问题）。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 1 file / 13 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0。
- **未跑**：`just precommit` / `just check-fe` / `just test-fe`（本机 pnpm wrapper 当前被 `ERR_PNPM_IGNORED_BUILDS` 阻断：`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（资产覆盖跳动修复 scope）**：`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入 EAS 资产覆盖完整矩阵，运行中顶部 `15/332 done` / live count / `Live` 和下方「正在做的资产」区域应只更新内容，不再反复挤动整块布局。

---

### 2026-06-28 · ask_human Confirm 后卡片残留修复

- **本轮目标**：回应用户截图里进入 `external_attack_surface` 后，阶段边界 `AI Needs Your Input` 点 Confirm 仍残留的问题。
- **根因**：
  - `AIChatPanel` 同时渲染 hook 本地 `askHumanRequest` 和全局 `pendingAskHuman` store 兜底；同一个 `ask_human_request` 可能同时被 hook 和 app-level AI event pipeline 记录。
  - 点 Confirm 走本地 hook 分支时只清了本地态，没有同步清 store；下一帧 `visibleAskHumanRequest` 又从 store 兜底拿到同一 `requestId`，所以卡片看起来“确认了还有”。
- **已完成**：
  - 新增 `frontend/components/AIChatPanel/askHumanStore.ts`，按 `requestId` 清理同一 ask_human 请求在 AI session / terminal session / conversation key 下的 store 副本；不会误清同 session 上更晚的新 prompt。
  - `frontend/components/AIChatPanel/AIChatPanel.tsx` 的 Confirm / Skip 两条路径都在 finally 里清理匹配的 store 副本；store-only 兜底路径仍直接响应对应 session。
  - 补 `frontend/components/AIChatPanel/askHumanStore.test.ts` 回归；同步 `docs/modules/frontend/components.md` 模块卡。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（本机 pnpm install gate 延续既有环境问题）。
  - `./node_modules/.bin/biome check --write frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts` → exit 0。
  - `./node_modules/.bin/biome check frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/AIChatPanel/askHumanStore.test.ts frontend/components/AIChatPanel/AskHumanInline.test.tsx frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx` → 3 files / 42 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/AIChatPanel/askHumanStore.ts frontend/components/AIChatPanel/askHumanStore.test.ts docs/modules/frontend/components.md agent-progress.md` → exit 0。
  - `just check-fe` / `just test-fe` → exit 1；底层 `pnpm check` / `pnpm typecheck` / `pnpm test:run ...` 均在执行脚本前被 `ERR_PNPM_IGNORED_BUILDS` 阻断（`@swc/core@1.15.21`、`electron@23.3.13`、`esbuild@0.25.12` 需要 `pnpm approve-builds`）。
- **未跑**：`just precommit`（`./init.sh` / `just check-fe` / `just test-fe` 已在 pnpm install/build-approval gate 阶段失败；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（ask_human 残留修复 scope）**：`frontend/components/AIChatPanel/AIChatPanel.tsx`、`frontend/components/AIChatPanel/askHumanStore.ts`、`frontend/components/AIChatPanel/askHumanStore.test.ts`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新/重启前端后复测阶段边界 prompt；点 Confirm 后卡片应立即消失，EAS 阶段继续跑。

---

### 2026-06-27 · Scoping REUSE 扩树导致资产爆炸诊断与门禁修复

- **本轮目标**：回应用户“怎么越搞资产越多、很多乱七八糟的”，复盘前一次 run 为什么从平安 scope 膨胀到大量资产，并修复 scoping REUSE mode 被硬门禁逼着重复 create 的问题。
- **日志证据 / 根因**：
  - 最新相关 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782571959315-1/`。
  - `run_tree.py --full` 显示 scoping 明明识别为 REUSE mode，但仍执行 `manage_organizations(action="create_batch")`，一次新增/复用 18 个子公司；后续 org tree 变成 27 个 org。
  - 根因是 prompt/gate 冲突：`resources/harness/stages/scoping/methodology.md` 写着 “REUSE mode: do NOT re-create”，但 `prompts/mod.rs` / `execute.rs` 的红队硬门禁仍写死 “必须 propose_candidates → unit_review → manage_organizations(create)”。
  - 另一个放大器：`golish-db::repo::tool_calls::scoping_actions_for_session` 只统计 `action='create'`，不统计推荐的 `create_batch`。模型用 `create_batch` 批量扩树后，gate 审计还可能认为没有 create，诱发更多纠错/重跑。
  - target_intel 阶段对 27 个 org 扇出；部分 org 的 provider survey 极大，例如 root org 记录 `subdomains=143 / subdomain_hosts=676`，root summary 自述 “499+ in-scope assets”；平安证券自述注册 168 targets；后续又出现 `blocked-org-1` 占位补跑并注册 158 assets。资产多不是单纯“扫出来了”，而是 scoping 扩树 + 大 org passive provider 泛匹配 + retry 占位补跑共同放大。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`：scoping charter 改为 REUSE mode 下不要为了 gate 调 `create`/`create_batch`；只有 root 缺失或用户显式新增/确认单位时才创建。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：red-team scoping anti-shortcut gate 改为只强制真实 `unit_review`；已有 org tree 经人审确认即可通过，不再因缺 create BLOCK。
  - `backend/crates/golish-db/src/repo/tool_calls.rs`：`create_batch` 的 `created` / `existing` id 也会被 scoping action audit 识别为真实组织记录，避免未来真正批量新增后被误判。
  - `backend/crates/golish-agent-kit/src/db_traits/repo.rs`：同步 trait 注释，明确 `organization_created` 对 REUSE mode 只是 audit 信息，不是必须条件。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-db.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit -p golish-db`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit red_team_scoping_flow --status-level fail`（cwd `backend`）→ 1 passed / 758 skipped。
  - `cargo nextest run -p golish-db create_result create_batch_result --status-level fail`（cwd `backend`）→ 4 passed / 108 skipped。
  - `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-db -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`（本机 pnpm ignored-builds/install gate 仍是全量前置阻塞；本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（scoping reuse gate scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-db/src/repo/tool_calls.rs`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-db.md`、`agent-progress.md`。
- **下一步建议**：数据库清空后重启 app 再跑“搞一下平安”。第一轮如果 root 不存在，可以按 unit_review 新建确认的 org；之后再次跑同一 root 时应该只复用/确认现有树，不应再自动 `create_batch` 扩到几十个 org。若还出现单 org 落几百资产，下一刀应收紧 `recon_map_assets` provider result 的 ownership/domain relevance threshold。

---

### 2026-06-27 · StageRun active-stage completion floor + pass-token submit preview

- **本轮目标**：回应用户“最后一次日志还是过不去”，诊断最新 run 的 target_intel submit loop，并修复新一轮 active stage 被旧 `org_stage_completions` 短路的问题。
- **日志证据 / 根因**：
  - 最新真实 session：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782570596001-2/`。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782570596001-2 --full --db` 显示 scoping 已 PASS，`operation_state.engagement_org_id=e51a6ae1-c6f4-4dc7-9c57-d8263d9fc107`，`current_stage=target_intel`，`stage_started_at=2026-06-27 22:31:32 +0800`。
  - 但 target_intel `stage_run` 8/8 org 都从旧 completion 跳过：completed at `2026-06-27 13:25 UTC` 等，早于本次 `stage_started_at=2026-06-27 14:31 UTC`；所以本轮没有新的 worker/evidence/source rows。
  - 随后的 `check_stage_asset_coverage` 仍有 `pending_cells=2119`，`source_query_log: none for this run`；`submit_stage_deliverable` 一直被 `coverage_complete` / `source_coverage` 打回，不是 askman 没走，也不是 scoping root 没绑。
  - 另一个下游症状：主 agent 提交 `stage_run_pass_token` claim 时，submit preview 先按普通 claim 要 evidence，返回 `every claim must cite evidence`，导致它继续乱补 invalid skipped_check；该 pass token 应由 final fan-out closeout 从 DB ledger 重算验证。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：新增 `completion_is_fresh_for_stage`，在 TTL 之外支持 active-stage `not_before` floor；补单测锁定旧 completion 不能跨 stage start 复用。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：`stage_run` resume-skip、pass-token generation 都使用当前 `operation_state.current_stage == stage` 时的 `stage_started_at` floor；旧 completion 不再短路当前 active stage worker。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：fan-out closeout 验 pass_token 时同样用 current active-stage floor 过滤 `org_stage_completions`，避免旧 ledger 生成当前 token。
  - `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`：specialist stage 的 `stage_run_pass_token` claim 在 submit preview 阶段只做结构/伪造 evidence-id 检查并收进 side-channel；最终由 orchestrator closeout 重算 DB token 判定。
  - 同步模块卡：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`。
  - `feature_list.json`：更新 operation-continuity evidence / verification，状态仍 `in_progress`（全量 precommit 仍受 pnpm install gate 阻塞）。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit completion_fresh_for_stage fanout_completion_scope --status-level fail`（cwd `backend`）→ 4 passed / 755 skipped。
  - `cargo nextest run -p golish-agent-runtime resume_skip_floor active_stage_skip_floor --status-level fail`（cwd `backend`）→ 2 passed / 273 skipped。
  - `cargo nextest run -p golish-agent-app stage_run_pass_token --status-level fail`（cwd `backend`）→ 1 passed / 86 skipped。
  - `cargo check -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
  - `python3 -m json.tool feature_list.json` → exit 0。
- **未跑**：`just precommit`（前面 `./init.sh` 已在 `pnpm install --silent` / ignored-builds gate 卡住：`@swc/core`、`electron`、`esbuild`；本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（active-stage completion floor scope）**：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-agent-app/ai.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑这条 operation；target_intel 的 `stage_run` 不应再显示“已完成于 13:25 UTC 跳过重跑”，而应实际 dispatch worker 或只跳过本次 `stage_started_at` 之后新写的 completion。拿到新的 pass_token 后，`submit_stage_deliverable` 应先 accepted，再由 closeout 重算 DB ledger 判定。

---

### 2026-06-27 · Continuity rootless adoption 全库污染修复

- **本轮目标**：回应用户“最后一次日志一直跑不通”，诊断最新复用流程为什么又卡住，并修复没有绑定 engagement root 时跳过 scoping 导致的全库污染。
- **日志证据 / 根因**：
  - 最新真实 session 是 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782566555331-1/`；默认 `run_tree.py` 会先捞到 title-gen，因此本轮指定了 `pentest-chat-1782566555331-1`。
  - 这次 AskHuman 正常：transcript 里有 `ask_human_request`，用户选择了“复用已有数据继续”。
  - `stage_run` 也没有再全 org skip：run.log 里出现 continuity entry stage 的 resume-skip floor，worker 实际跑起来了。
  - 当前 blocker 是新的：`operation_state.engagement_org_id = NULL`，复用 scoping 后没有把“中国平安”的 root org 绑定进 operation。于是 `list_in_scope_targets` / pass-token closeout / coverage preflight 都落到 legacy 全库口径。
  - 结果 first `target_intel` stage_run 对 13 个平安 org passed，但 `pass_token=null`；后续 main agent 从全库历史资产里挑了 `AngularDocs` / `JsRuleFilter8080` / `example.org` / `8.138.179.62:8080` 等目标继续补洞，gate 一直报这些测试资产缺 `GOLISH-INTEL-*` 终态，不是平安本身没采完。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/continuity.rs`：没有 `engagement_root` 时，`scoping` summary 改为 `Missing`，明确要求先跑 scoping 绑定当前任务；不再把 legacy `in_scope_org_ids(None)` 当作可安全 adopt 的 scope。
  - 同文件新增 `non_empty_adoption_cursor`，如果没有任何前缀 stage 真正能被 adopt，就不弹 continuity 选择框，避免“问复用但实际从 scoping 开始”的误导。
  - 补单测：无 root 的 scoping 不能 adopt；有 root 才能复用 scoping；即便后续 `target_intel` completion fresh，只要 scoping/root 缺失，也不会生成空 adoption plan。
  - 同步模块卡：`docs/modules/backend/golish-agent-kit/{task_orchestrator,harness}.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit continuity --status-level fail`（cwd `backend`）→ 10 passed / 748 skipped。
  - `cargo check -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check` → exit 0。
- **未跑**：`just precommit`（此前同日 `./init.sh` / `just install` 已被本机 pnpm `ERR_PNPM_IGNORED_BUILDS` 卡住：`@swc/core` / `electron` / `esbuild` build scripts 未 approve；本轮做 scoped 后端验证）。
- **提交记录**：待提交。
- **已知风险或未解决问题**：
  - 运行中的 app 需要重启后才会加载这次 Rust 改动。
  - 这刀是 fail-safe：没有 root 时不跳 scoping。更好的后续增强是从用户目标文本/旧 scope 精确解析出唯一 root org 后，再允许带 root 的 continuity adoption。
  - 当前工作树已有大量非本轮脏改动，本轮未回退或清理。
- **下一步最佳动作**：重启 app 后重新发“搞一搞平安”。如果没有可靠 root，系统应直接进入 scoping 重新绑定 root；如果未来传入 root，才允许安全跳过 scoping。用 `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 <session> --full --db` 确认 `operation_state.engagement_org_id` 不再为 NULL。

---

### 2026-06-27 · continuity 复用确认走 ask_human 卡片

- **本轮目标**：回应用户截图里 DB progress 复用确认显示成普通 Golish AI 文本、没有走 ask_human/AskHumanInline 的问题。
- **根因**：
  - `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs` 的 continuity preflight 在 `AskBeforeReuse` 且发现 `ContinuityAdoptionPlan` 时，直接 `render_continuity_offer()` + `emit_immediate_task_response(... Completed ...)` 后 `return Ok(message)`。
  - 这条路径没有 `CoordinatorHandle::register_approval`，也没有发 `AiEvent::AskHumanRequest`；前端只有收到 `ask_human_request` 事件才会渲染 `AskHumanInline`，所以截图里只出现普通 assistant 文本。
- **已完成**：
  - `commands/core/chat.rs`：continuity ask-before-reuse 改为有 coordinator 时注册 approval、发 `AiEvent::AskHumanRequest(input_type="choice")`，等待用户选择；选择“复用已有数据继续”才把 `ContinuityAdoptionPlan` 交给 orchestrator，选择“重新开始”/Skip/timeout 走 `start_fresh`。
  - 同路径保留无 coordinator 的文本 fallback（单测/降级环境）。
  - 因前端现有 `choice` 会自动提交第一个选项，选项顺序用“重新开始”在前，避免静默复用旧 DB facts。
  - `docs/modules/backend/golish-agent-app/ai.md`：同步模块卡，明确 continuity preflight 必须走共享 ask_human/approval coordinator。
  - `feature_list.json`：给 `operation-continuity-adoption-2026-06-27` 追加本轮 scoped evidence，状态仍 `in_progress`。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm ignored-builds/install gate，延续此前环境限制）。
  - `cargo fmt -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-app chat_title_tests --status-level fail`（cwd `backend`）→ 22 passed / 64 skipped。
  - `cargo nextest run -p golish-agent-app start_operation continuity --status-level fail`（cwd `backend`）→ 8 passed / 78 skipped。
  - `cargo check -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-agent-app/src/ai/commands/core/chat.rs` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 仍被 pnpm install gate 卡住；当前工作树已有大量前序未提交改动，本轮做 scoped 后端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（continuity ask_human scope）**：`backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`、`docs/modules/backend/golish-agent-app/ai.md`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启/刷新 app 后重新触发 fresh Task/Profile operation；发现旧 DB progress 时应出现 `AI Needs Your Input` 的 choice 卡片，而不是普通 Golish AI 文本。点“复用已有数据继续”后才采用旧 DB facts 并从第一个未满足 stage 接着跑。

---

### 2026-06-27 · 信息收集阶段 progress 路由修复

- **本轮目标**：回应用户发现 EAS 完成后跳到 `reporting` 而不是 `enumeration`；确认信息收集阶段不应靠 `findings` 判断进展。
- **日志证据 / 根因**：
  - 最新 session `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782488490399-1`：EAS gate `2026-06-27T11:20:36Z` PASS 后，下一 turn `2026-06-27T11:20:55Z` 进入 `reporting` 的 `Final Report Compilation`。
  - `check_stage_asset_coverage` 曾明确显示 EAS 有 `615/825` done、`210` pending；后续补 blocked cells 后 PASS，说明不是 UI 误显，而是 graph-flow 路由走了 `external_attack_surface -> reporting` 短路。
  - 代码根因：`consume_gate_outcome` 把 `made_progress` 写死为 `outcome.findings_count > 0`；而 `external_attack_surface` / `target_intel` / `enumeration` 都是 `findings_allowed=false` 的信息收集/覆盖矩阵阶段，正常交付就是 `findings=[]`。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：新增 `gate_outcome_made_progress`，blocked outcome 永不算进展；vuln 阶段继续用 `findings_count`；`findings_allowed=false` 的 recon/info 阶段改按 evidence refs、handoff summary、engagement org binding 判断有无阶段产出，避免 EAS 因无 findings 跳 `reporting`。
  - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`：新增回归，锁定 EAS 无 findings 但有 evidence refs 算进展；`vuln_triage` 无 findings 不算进展。
  - `docs/modules/backend/golish-agent-kit/task_orchestrator.md`：同步模块卡，记录 graph-flow progress 不能再 findings-only。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm install gate，延续此前环境限制）。
  - `cargo fmt -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-kit info_stage_evidence_counts_as_progress_without_findings vulnerability_stage_without_findings_is_not_progress --status-level fail`（cwd `backend`）→ 2 passed / 747 skipped。
  - `cargo nextest run -p golish-agent-kit pass_emits_stage_passed_progress block_emits_no_stage_passed --status-level fail`（cwd `backend`）→ 2 passed / 747 skipped。
  - `cargo check -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `cargo clippy -p golish-agent-kit --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs` → exit 0。
  - `git diff --check -- backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs docs/modules/backend/golish-agent-kit/task_orchestrator.md agent-progress.md` → exit 0。
- **未跑**：`just precommit`（`./init.sh` 已被 pnpm install gate 卡住；当前工作树已有大量前序未提交改动，本轮做 scoped backend 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（progress routing scope）**：`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`agent-progress.md`。
- **下一步建议**：重启/刷新 app 后，EAS PASS 且有 evidence/handoff 时 graph-flow 应走主路 `enumeration`；当前已停在 `reporting` 的旧 operation 仍是旧 checkpoint 状态，需重新跑或修复 operation cursor 才能从 `enumeration` 接上。

---

### 2026-06-27 · SubAgent detail 资产覆盖二级视图

- **本轮目标**：回应用户看完整左右布局后的确认：右侧已经是 ChatPanel，资产覆盖不做右侧 drawer；在左侧 `SubAgentDetailView` 内改成 summary 进入的轻量二级视图，默认保持 Codex 风格的干净运行流。
- **已完成**：
  - `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：默认运行流只显示任务块、轻量资产覆盖 summary strip、Thought/Agent Output/tool call 时间线；去掉「运行流 / 资产覆盖」两个大 tab，点 summary 进入完整矩阵，避免矩阵挤占 agent 叙事流。
  - `frontend/components/Engagement/StageAssetCoveragePanel.tsx`：`StageAssetCoverageBlock` 增加 `summary` / `panel` 呈现模式；summary 模式只加载并显示 done/live/current-tool 摘要，不渲染矩阵，右侧只留小箭头；panel 模式渲染完整 coverage matrix，并在卡 header 右侧提供小号「运行流」返回按钮，避开页面左上角返回上级 Agent。独立 coverage view 改为占满 detail 内容区、列表自身滚动，不再显示底部拖拽高度 handle。
  - `frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`：新增回归，锁定 summary 模式不渲染矩阵且点击进入覆盖视图、panel 模式渲染完整矩阵、小号返回动作，并确认独立 panel 不显示高度调节控件。
  - `docs/modules/frontend/components.md`：同步模块卡，记录 coverage matrix 不能再 inline 展开在运行流里，也不要铺两个大 tab；完整矩阵由 summary 进入、卡内小按钮返回运行流，独立覆盖视图不显示高度拖拽控件。
- **运行过的验证（实跑）**：
  - `./init.sh` → exit 1；Step 2 `just install` / `pnpm install --silent` 失败（pnpm install gate，延续此前环境限制）。
  - `./node_modules/.bin/biome format --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/Engagement/StageAssetCoveragePanel.tsx frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx` → exit 0（fixed import order）。
  - `./node_modules/.bin/vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts` → exit 0（2 files / 51 tests passed）。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
- **未跑**：`just precommit` / `just check-fe` 全量（`./init.sh` 仍被 pnpm install gate 卡住；本轮做 scoped 前端验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（coverage detail view scope）**：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.tsx`、`frontend/components/Engagement/StageAssetCoveragePanel.test.tsx`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：刷新前端后进入正在运行的 EAS/target_intel specialist detail；默认应只看到一条资产覆盖摘要和下方 Thought/Output/tool stream；点击摘要进入完整矩阵，点矩阵 header 右侧小号「运行流」返回时间线。

---

### 2026-06-29 · enumeration JS extract 大 bundle 超时修复

- **本轮目标**：回应用户追问最后一次枚举阶段里 `js_extract_apis` 为什么 300s 超时，修掉 dayu.moresec.cn 这类大 bundle 让 JS 静态增强卡死 worker 的问题。
- **日志证据 / 根因**：
  - 最新 run `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1782738572850-1`：`js_extract_apis` 在 `2026-06-29T13:40:59Z` 被 sub-agent tool timeout 300s 打断。
  - `browser_collect_js_api` 已对 `https://dayu.moresec.cn/` 成功落 `api_endpoints(source='crawler')`：`/api/iam/v2/login/types`，并写 `GOLISH-ENUM-JSAPI found`；卡住的是后续静态 `js_extract_apis`。
  - dayu capture 里实际唯一 JS 文件 13 个，其中 `82c2e636_umi.0058c760.js` 为 3.2MB；旧 `js_api_extract` 对该目录 120s 无输出，说明问题在同步 AST/regex/signals 静态分析大 bundle，而不是 AI 思考。
- **已完成**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`：新增 `max_file_bytes`（默认 1.5MB）和 bounded loader；超过阈值的 bundle 返回 `status="partial"`、`files_skipped`、`skipped_js_files`，并把 skipped 计入 JSAPI outcome 的 error/partial 语义，避免工具等到 sub-agent 300s timeout。
  - `scripts/browser_collect_js_api.mjs`：补 `scripts_observed`、`script_manifest_entries`、`unique_scripts_saved`，并让 `scripts_saved` 表示唯一落盘文件数，避免重复 chunk 引用数误导静态分析规模。
  - `backend/crates/golish-js-analyzer/src/bin/js_api_extract.rs`：CLI 增加 `--max-file-bytes`，默认同 bridge，独立 pipeline 也能快速 partial 返回。
  - `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-js-analyzer.md`：同步工具契约。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-js-analyzer -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `node --check scripts/browser_collect_js_api.mjs` → exit 0。
  - `cargo nextest run -p golish-js-analyzer --status-level fail`（cwd `backend`）→ 48 passed。
  - `cargo nextest run -p golish-pentest-app js_extract_apis browser_collect_js_api --status-level fail`（cwd `backend`）→ 26 passed / 74 skipped。
  - `cargo build -p golish-js-analyzer --bin js_api_extract`（cwd `backend`）→ exit 0。
  - `backend/target/debug/js_api_extract --js-dir /Users/christopherzheng/golish-platform/Test1/.golish/captures/dayu.moresec.cn/443/js --target-url https://dayu.moresec.cn --endpoint-limit 5 --signal-limit 5 --context-limit 0`（via python wrapper, cwd repo root）→ exit 0, 4.1s；输出 `status=partial`、`files_scanned=12`、`files_skipped=1`，跳过 `82c2e636_umi.0058c760.js`（3239778 bytes）。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮做 scoped JS analyzer / pentest bridge 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（JS extract timeout scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`、`scripts/browser_collect_js_api.mjs`、`backend/crates/golish-js-analyzer/src/bin/js_api_extract.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-js-analyzer.md`、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑/续跑 enumeration；`dayu.moresec.cn` 这类站点应由 browser runtime API 先让 JSAPI found，静态 extract 对超大 bundle 返回 partial 而不是卡死 worker。

---

### 2026-06-30 · Enumeration worklist / Target Surface 结构化展示修复

- **本轮目标**：回应用户对刚跑完的 `/Users/christopherzheng/golish-platform/Test1` 最新 run 的 EAS / Enumeration 逻辑疑问；重点修剩余问题：枚举 worklist 仍露出旧工具、sitemap 不是树、JS/API 不能按 JS 文件看详情、`crawl_mode` 还出现 `fast`。
- **日志证据 / 根因**：
  - 最新 run `pentest-chat-1782791610659-1` 的 `list_enumeration_web_roots` 返回 `tool_boundary` 已写明不用 `ffuf/gobuster/feroxbuster`，但每个 coverage cell 的 `suggested_tools` 仍带 `ffuf` / `arjun`；根因是 `stage_coverage.rs::suggested_tools()` 没跟 methodology/prompt 同步。
  - 枚举 sub-agent transcript 统计：`browser_collect_js_api` 共 29 次请求，其中 15 次仍传 `crawl_mode="fast"`、14 次传 `standard`；helper 已统一归一到 standard 策略，但 tool schema 仍把 legacy `fast/deep` 暴露给模型。
  - Target Surface 的 `buildSitemapItems` 只生成扁平 `SitemapItem[]`，`SitemapTab` 直接渲染列表；没有树模型。
  - JS/API tab 只显示全局 `api_endpoints` 列表和 JS 文件计数 badge；每个 JS 文件自己的 `js_analysis_results.endpoints_found` 没有展开展示，`raw_analysis.ai_review` 也不可见。
- **已完成**：
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：枚举 pending cell 建议工具改为当前一方工具：DIR → `route_probe_paths`；PARAM → `browser_collect_js_api` + `js_extract_apis`；JSAPI → `browser_collect_js_api` + `js_extract_apis`；新增回归，确保 `ffuf/arjun` 不再进入 suggested_tools。
  - `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`：tool schema 的 `crawl_mode.enum` 只暴露 `["standard"]`；legacy `fast/deep` 入参仍在 helper 内部归一为 standard，新增 schema 回归。
  - `frontend/components/TargetPanel/surface/surfaceModel.ts` / `SitemapTab.tsx` / `TargetSurfaceWorkbench.tsx`：Sitemap/Paths 改为按 origin/path segment 建树；`api_endpoints` 的 URL/path 也合入 sitemap 树。
  - `frontend/components/TargetPanel/surface/tabs/JsApiTab.tsx`：JS 文件行改为可展开，展示 per-file endpoint candidates、secrets/framework/library 计数和 `raw_analysis.ai_review` 摘要；不从全局 `api_endpoints` 硬反推来源 JS 文件。
  - 同步模块卡：`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-app enumeration_pending_cells_only_suggest_current_first_party_tools --status-level fail`（cwd `backend`）→ 1 passed / 128 skipped。
  - `cargo fmt -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app schema_only_advertises_standard_crawl_mode --status-level fail`（cwd `backend`）→ 1 passed / 101 skipped。第一次运行因测试本身缺 Tokio context 失败，已改为 `#[tokio::test]` 后重跑通过。
  - `./node_modules/.bin/biome format --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/tabs/JsApiTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/tabs/JsApiTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts` → exit 0，1 file / 11 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（本轮按 scoped 修复验证；当前工作树已有大量非本轮未提交改动，且此前同日 full init/precommit 仍受 pnpm ignored-builds gate 影响）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`、`frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`frontend/components/TargetPanel/surface/{surfaceModel.ts,surfaceModel.test.ts,types.ts}`、`frontend/components/TargetPanel/surface/tabs/{SitemapTab.tsx,JsApiTab.tsx}`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/frontend/components.md`、`agent-progress.md`。
- **下一步建议**：重启 app 后重新跑/续跑 enumeration；新 run 的 `list_enumeration_web_roots` 不应再给 `ffuf/arjun`，`browser_collect_js_api` 新调用不应再出现 `crawl_mode="fast"`，Target Surface 的 Sitemap 应显示树，JS/API tab 可展开每个 JS 文件查看端点和静态审阅摘要。真实 AI 是否介入仍以 sub-agent 工具流为准：看 `ai_assist.recommended` 后是否出现带 `recipe` 的第二次 `browser_collect_js_api`，以及 `js_extract_apis` 是否带 `param_hints`。

#### 追加修正 · Sitemap 口径收窄为 JS/runtime API evidence

- **本轮目标**：回应用户进一步澄清：Target Surface 的 Sitemap 不要放 route probe / 乱扫描目录，应只放 JS 里确定性的路径/API，并且点开能看到响应包证据；独立 `JS / API` tab 先撤掉。
- **已完成**：
  - `frontend/lib/api/security-analysis.ts`：`ApiEndpoint` 前端类型和 normalize 增加 `capturePath`，读取后端已有 `api_endpoints.capture_path`。
  - `frontend/components/TargetPanel/surface/surfaceModel.ts`：`buildSitemapItems` 改为只从 `api_endpoints` 的 `crawler` / `js_analysis` / JS 类来源构建，不再消费 `directory_entries` / `target_assets` / `route_probe_paths`。
  - `frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx`：Sitemap 改为左侧树 + 右侧 endpoint 详情；点击 endpoint 显示 method/path/url/status/content-type/params/headers/capturePath。当前 DB 若未落完整 response capture，则显示 `No response capture stored`。
  - `frontend/components/TargetPanel/surface/types.ts` / `TargetSurfaceWorkbench.tsx`：移除顶部 `JS / API` tab；删除 `frontend/components/TargetPanel/surface/tabs/JsApiTab.tsx`，Sitemap 成为 JS/runtime API 路径入口。
  - `scripts/browser_collect_js_api.mjs` / `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs` / `backend/crates/golish-db/src/repo/api_endpoints.rs`：浏览器采集同源 XHR/fetch 时写 bounded request/response capture 到 `.golish/captures/<host>/<port>/api/`，并把 `capture_path` / headers / status / content-type 回填到 `api_endpoints`。
  - `docs/modules/frontend/components.md`：同步模块卡，明确 Sitemap 不接收目录扫描/route probe path。
  - `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：同步 `browser_collect_js_api` response capture 契约。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome format --write frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/lib/api/security-analysis.ts frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/lib/api/security-analysis.test.ts` → exit 0，2 files / 11 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `node --check scripts/browser_collect_js_api.mjs` → exit 0。
  - `cargo fmt -p golish-db -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app browser_collect_js_api --status-level fail`（cwd `backend`）→ 9 passed / 93 skipped。
  - `cargo nextest run -p golish-db api_endpoints --status-level fail`（cwd `backend`）→ 1 passed / 119 skipped。
  - `git diff --check -- <本轮前端相关文件>` → exit 0。
- **已知限制**：旧 run 已经落库的 endpoint 没有 retroactive response capture；需要重新跑/续跑 `browser_collect_js_api` 才会生成新的 `.golish/captures/.../api/*.json` 并在 Sitemap 详情里显示 capture path。

#### 追加修正 · Sitemap endpoint 反查 JS 来源

- **本轮目标**：回应用户截图里 Sitemap 右侧只能看到 endpoint/capture，看不到“这个 API 来自哪个 JS 文件”的问题。
- **已完成**：
  - `frontend/components/TargetPanel/surface/surfaceModel.ts`：新增 `buildSitemapJsSources`，用 selected `SitemapItem` 的 method/path 反查 `js_analysis_results.endpoints_found`，支持新对象格式（`source_file` / `line` / `confidence` / `kind`）和旧字符串 endpoint 格式。
  - `frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx` / `TargetSurfaceWorkbench.tsx`：Sitemap 详情右侧新增 `JavaScript Source` 区块，显示 source file、line、kind、confidence；继续单独显示 capture/headers。
  - `docs/modules/frontend/components.md`：同步 Target Surface Sitemap 契约，明确会从 `js_analysis_results.endpoints_found` 反查 JS source_file/line/confidence。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome format --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/types.ts` → exit 0。
  - `./node_modules/.bin/vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts` → exit 0，1 file / 10 tests passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx frontend/components/TargetPanel/surface/types.ts docs/modules/frontend/components.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮按截图问题做 scoped 前端验证）。
- **已知限制**：旧 run 如果已有 `js_analysis_results.endpoints_found`，刷新/重启前端后可看到 JS source；但旧 run 没有 `capture_path` 的返回包不能从现有 DB 逆推出，需要重新跑/续跑 `browser_collect_js_api` 才会生成 response capture。

#### 追加修正 · 移除 Target Surface 手动测试按钮

- **本轮目标**：回应用户指出 Target Surface 顶部 `Run baseline recon` / `Collect JS` / `Match vulns` 是早期测试入口，现在采集和匹配应由 AI/harness 流程发起，前端不要再展示这些按钮。
- **已完成**：
  - `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`：移除三枚手动 stage/action 按钮及对应 `StageButton`、`Radar`、`FileCode2`、`Search` imports；header 只保留本地 surface data refresh。
  - `docs/modules/frontend/components.md`：同步模块卡，明确 target surface header 不放手动扫描按钮。
- **运行过的验证（实跑）**：
  - `./node_modules/.bin/biome format --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx` → exit 0。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `git diff --check -- frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx docs/modules/frontend/components.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮按 UI 按钮 removal 做 scoped 前端验证）。

#### 追加修正 · route_probe 并发窗口与 JSAPI partial outcome 口径

- **本轮目标**：回应用户“没问题”后继续修最新 run 暴露的两个真实问题：`route_probe_paths` 参数已是 50/s 但实现仍串行导致前台长期卡住；`js_extract_apis` 对仅跳过超大 JS bundle 的 partial 结果写成 `GOLISH-ENUM-JSAPI error`，让 `dayu.moresec.cn` 这类已有浏览器 API 证据的目标看起来像静态提取错误。
- **日志 / DB 证据**：
  - 最新 run `pentest-chat-1782791610659-1` 里最新 Enumerator 对 `https://dayu.moresec.cn/` 发起 `route_probe_paths`，入参已含 `rate_limit_per_sec=50`；但 transcript 最后一条仍是 tool request，无 tool result，backend/run.log 持续发 `tool_output_chunk`，DB `directory_entries` 对 dayu 仍为 0。
  - DB 已有 `dayu.moresec.cn` 浏览器采集 API：`GET /api/iam/v2/login/types`，source=`crawler`，且有 `.golish/captures/dayu.moresec.cn/.../types.json`；同一目标的 `js_extract_apis` 结果因超大 bundle partial 写了 `GOLISH-ENUM-JSAPI error`。
- **已完成**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`：把候选请求主循环从串行请求 + sleep 改为按 `rate_limit_per_sec` 窗口并发发起；默认 50、最大 100 继续生效。新增整体 `max_runtime_ms`（默认 180000，最大 1800000），超预算返回 `status="timeout_partial"`、`timed_out=true`、`queue_completed=false`、`queue_remaining=N`，并把这些字段写进返回 JSON 和 audit detail；timeout partial 无命中时 DIR outcome 写 `error`，不误写 empty。
  - `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`：`jsapi_outcome_from_extract` 区分真实 read/persist error 与 skipped-large-file；仅跳过超大 bundle 时结果仍可 `status=partial`，但 JSAPI outcome 走 `empty` 而不是 `error`。新增回归覆盖该口径。
  - `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：同步 route probe 并发/总预算，以及 JS extract partial outcome 语义。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app route_probe_paths jsapi_outcome --status-level fail`（cwd `backend`）→ 18 passed / 115 skipped，exit 0；首次启动时等待 artifact lock，随后编译通过并跑完。
  - `cargo fmt -p golish-pentest-app --check`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮做 scoped pentest bridge 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{route_probe_paths.rs,js_extract_apis.rs}`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`agent-progress.md`。
- **下一步建议**：需要重启 app/backend 后续跑 enumeration 才会吃到新 route 并发和 partial outcome 口径；当前正在跑的旧 direct tool 不会自动切到新代码。

#### 追加诊断 · moresec.cn JS 收集数量变少的真实口径

- **本轮目标**：回应用户观察“之前 JS 能发现好几百个，现在变少”，用 `moresec.cn` 做独立抓取 + 当前 Golish collector 对比，定位是抓取退化还是计数/落库口径问题。
- **moresec 实测结论**：
  - 独立 Node 抽取首页 direct scripts + Next `_buildManifest.js`：候选 union 117，全部有效 JS；其中 Next shared chunks 12、page chunks 102、manifest/runtime 2、other 1。
  - 当前 `scripts/browser_collect_js_api.mjs` 对 `https://moresec.cn`：`scripts_observed=120`、`script_manifest_entries=120`、`unique_scripts_saved=119`、`scripts_recursive_downloaded=105`、`closure_complete=true`、`recursive_queue_remaining=0`。多出来的主要是 Baidu 统计类 `.js` 0-byte/text/plain 噪声，不是有效业务 JS。
  - 对已保存的 119 个文件跑 `js_api_extract`：`files_scanned=119`、`endpoints_unique=0`、`frameworks_total=115`、`rule_matches_total=2261`。所以 moresec 主站“JS 文件收集”并没有塌，真正少的是可确定 API endpoint。
  - 最新测试 workspace run `pentest-chat-1782791610659-1` 里，`moresec.cn` / `www.moresec.cn` 也都保存约 119 个唯一脚本；`dayu.moresec.cn` 曾有 `scripts_observed=1373` 但 `unique_scripts_saved` 只有 13-17，说明“几百/上千”更多是 observed/重复 chunk 引用口径，而不是唯一落盘 JS 文件数。
- **根因判断**：
  - 主要问题不是 `browser_collect_js_api` 抓不到 moresec 主站 JS，而是 UI/coverage 读 `js_analysis_results` 时，当前 collector 只保存文件和 manifest，没有马上写 JS 资产行；只有后续 `js_extract_apis` 成功跑完时才会出现 DB 可见 JS 行。于是静态分析未跑、超时、或只跑了 subset 时，前端会看起来“JS 很少/没有”，即使 `.golish/captures/.../js` 已经有文件。
  - 另一个口径差异是 `scripts_observed` / `script_manifest_entries` vs `unique_scripts_saved` / DB row。历史 run 里 dayu 的 1373 属 observed，去重后真实唯一文件很少。
- **已完成代码修正**：
  - `browser_collect_js_api` 对成功落盘且非空的 JS 写 `js_analysis_results` placeholder（`raw_analysis.collected_by="browser_collect_js_api"`、`analysis_pending=true`），让 JS 资产收集即时进入 DB/UI 可见口径；0-byte/越界路径/非成功状态脚本不落 placeholder。
  - `js_analysis::insert` 改成按 `(target_id, filename)` 幂等更新最新行：collector placeholder 可先落库，`js_extract_apis` 后续原地升级为完整分析；如果完整分析已存在，新的 placeholder 不会降级覆盖。
  - `js_analysis::list_by_target` 返回每个 filename 最新行，避免历史重复行放大前端 JS 数。
  - 机械修正两个 scoped clippy 阻断：`RuleMatchSeverity` 改 derive `Default`；`route_probe_paths` 合并两个等价 `"error"` 分支。
  - 模块卡已同步：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-db/repo.md`。
- **运行过的验证（实跑）**：
  - `node scripts/browser_collect_js_api.mjs --url https://moresec.cn --workspace /tmp/golish-moresec-prod --timeout-ms 30000 --hard-timeout-ms 120000 --max-recursive-scripts 2000 > /tmp/moresec-prod.json 2> /tmp/moresec-prod.stderr` → exit 0，`unique_scripts_saved=119`、`closure_complete=true`。
  - `backend/target/debug/js_api_extract --js-dir /tmp/golish-moresec-prod/.golish/captures/moresec.cn/443/js --target-url https://moresec.cn --endpoint-limit 50 --signal-limit 20 --context-limit 0 > /tmp/moresec-js-extract.json` → exit 0，`files_scanned=119`、`endpoints_unique=0`。
  - `cargo fmt -p golish-db -p golish-pentest-app -p golish-js-analyzer`（cwd `backend`）→ exit 0。
  - `CARGO_TARGET_DIR=/tmp/golish-codex-target-jsfix cargo check -p golish-db -p golish-pentest-app`（cwd `backend`）→ exit 0。
  - `CARGO_TARGET_DIR=/tmp/golish-codex-target-jsfix cargo nextest run -p golish-pentest-app browser_collect_js_api js_extract_apis --status-level fail --no-fail-fast`（cwd `backend`）→ 34 passed / 99 skipped。
  - `CARGO_TARGET_DIR=/tmp/golish-codex-target-jsfix cargo nextest run -p golish-db coverage_truth js_analysis --status-level fail --no-fail-fast`（cwd `backend`）→ 30 passed / 104 skipped。
  - `CARGO_TARGET_DIR=/tmp/golish-codex-target-jsfix cargo clippy -p golish-db -p golish-pentest-app --all-targets -- -D warnings`（cwd `backend`）→ exit 0。
  - `CARGO_TARGET_DIR=/tmp/golish-codex-target-jsfix cargo nextest run -p golish-pentest-app route_probe_paths --status-level fail --no-fail-fast`（cwd `backend`）→ 9 passed / 124 skipped。
  - `cargo fmt -p golish-db -p golish-pentest-app -p golish-js-analyzer --check`（cwd `backend`）→ exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动，且本地有长期 `cargo run` 持有默认 target 锁；本轮用 scoped 验证 + 临时 `CARGO_TARGET_DIR` 避免打断本地进程）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`、`backend/crates/golish-db/src/repo/js_analysis.rs`、`backend/crates/golish-db/src/repo/coverage_truth.rs`、`backend/crates/golish-js-analyzer/src/signals.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-db/repo.md`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后重新跑/续跑 enumeration；新的 `browser_collect_js_api` 结果应该在 collector 完成后立刻让 Target Surface/JS 资产计数看到 collected-but-pending-analysis 脚本行，再由 `js_extract_apis` 原地升级为完整静态分析。

#### 追加修正 · target_intel DNS 空结果落 checked_empty

- **本轮目标**：回应用户“没查出来就标记”，修最新 target_intel run 里 `route.moresec.cn × GOLISH-INTEL-DNS` 已经尝试但仍被 UI/precheck 画成 pending 的问题。
- **已完成**：
  - `organization_recon::refresh_per_asset_landing_summary` 返回 `dns_empty_hosts`：只包含真实发起 DNS 解析、但没有 A/AAAA/CNAME/MX/TXT answer 的 in-scope domain。
  - `DbRepoProvider::mark_target_intel_dns_empty_outcomes` 新增 app-side hook；生产实现用真实 evidence id upsert `technique_outcomes(GOLISH-INTEL-DNS, empty)`，不在 agent-kit 引入 golish-db/sqlx。
  - `record_recon_passive_evidence` 在 `recon_map_assets` evidence append 成功后调用该 hook，把未解析出的 domain 标为 checked_empty DB fact。
  - 对当前最新 run `pentest-chat-1782997699389-1` 做了一次窄 DB backfill：`route.moresec.cn / GOLISH-INTEL-DNS / empty`，`source=resolver`，`evidence_ids=[13923,13921]`。
  - 同步模块卡：`golish-recon-app/organization_recon`、`golish-agent-app/ai`、`golish-agent-kit/db_traits`、`golish-agent-runtime/agentic_loop`。
- **运行过的验证（实跑）**：
  - `cargo check -p golish-recon-app -p golish-agent-app -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo test -p golish-recon-app organization_recon::persistence -- --nocapture`（cwd `backend`）→ 14 passed / 200 filtered out，exit 0。
  - `cargo fmt --check --package golish-recon-app --package golish-agent-app --package golish-agent-runtime --package golish-agent-kit`（cwd `backend`）→ exit 1；失败项均为当前工作区已有未格式化改动（`bridge_config.rs`、`harness/gate/mod.rs`、`operation_flow.rs`、`execute_harness_loop_tests.rs`），本轮新增文件不再出现在 diff 中。
  - DB verification（cwd repo）：查询 `technique_outcomes` 确认 `pentest-chat-1782997699389-1 / route.moresec.cn / GOLISH-INTEL-DNS` 已为 `empty`，`evidence_ids=[13923,13921]`。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1782997699389-1 --db | tail -n 80` → exit 0；DB 自诊断可见 `GOLISH-INTEL-DNS empty` 计数已包含该类 negative fact。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮按 target_intel DNS negative fact 做 scoped Rust 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-recon-app/src/organization_recon/{persistence.rs,mod.rs}`、`backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/{evidence.rs,mod.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`docs/modules/backend/golish-recon-app/organization_recon.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/db_traits.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`agent-progress.md`。
- **本地 DB 变更**：embedded Postgres `golish` 已 backfill 当前 run 的 `technique_outcomes` 一行（`route.moresec.cn / GOLISH-INTEL-DNS / empty`）。
- **下一步建议**：重启 app/backend 后重新跑/续跑 target_intel；新的 `recon_map_assets` evidence 写入后，类似 `route.moresec.cn` 这种解析为空的 domain 应在 `technique_outcomes` 中出现 `GOLISH-INTEL-DNS=empty`，`check_stage_asset_coverage` 不应再把它当作 never-attempted。

#### 2026-07-03 · moresec 今晨 task 运行体检 + run_tree 诊断加厚

- **本轮目标**：检查 `/Users/christopherzheng/golish-platform/Test1` 今晨 `pentest-chat-1783043250419-1` 的 task 运行过程，确认是否存在 harness / evidence / enumeration 异常；如 `scripts/run_tree.py` 不够详细则补强。
- **发现的问题**：
  - EAS prober 的 submit-repair EvidenceRefs 模式连续 3 次挡掉 `list_recent_evidence`，导致 worker 在需要真实 evidence id 时只能猜，曾引用不存在的 `14148`。
  - Enumeration 真实做了 JS / JSAPI / DIR / PARAM 工作，`technique_outcomes` 已写入 `GOLISH-ENUM-*` 且内容表有新数据，但对应 audit evidence 行 `14161..14197` 的 `session_id` 为 `NULL`；最终 gate/repair 只看到旧的 `14125..14158`，所以误提示“只需引用这些旧 evidence id”。
  - Enumeration worker 有 1 次拼错工具名 `re_run_route_probe_paths_for_m_moresec`，以及 1 次等待 `route_probe_paths` 时被取消；`m.moresec.cn` / 部分 `:9443` 服务产生 timeout/error 终端 outcome，仍有大量 pending/error cell。
- **已完成**：
  - `scripts/run_tree.py` 增加 runtime anomalies 摘要，直接统计 submit-repair blocked tool 与 cancelled tool。
  - `scripts/run_tree.py --db` 增加 org_id、audit session window、session evidence ledger rows、`technique_outcomes(run_id)`、ENUM ledger 断链告警、fresh content rows/top targets。
- **运行过的验证（实跑）**:
  - `python3 -m py_compile scripts/run_tree.py` → exit 0。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1783043250419-1 --db > /tmp/golish_run_tree_1783043250419_db_new.txt` → exit 0；输出包含 `submit_repair blocked prober.list_recent_evidence: x3` 与 `ENUM outcomes exist but this session has no ENUM evidence ledger rows`。
  - `git diff --check -- scripts/run_tree.py` → exit 0。
- **未跑**：`just precommit` / `./init.sh`（本轮是诊断脚本加厚 + live run 体检，且工作树已有多处非本轮后端 dirty 文件；保持 scoped 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`scripts/run_tree.py`、`agent-progress.md`。

#### 2026-07-03 · enumeration submit_repair worklist 解卡

- **本轮目标**：回应用户指出的 latest enumeration repair 模式问题：`submit_stage_deliverable` 后进入 `submit_repair`，扫描类工具被锁到 gate 点名的 IP gap 是对的，但 `stage_worklist_status` / `stage_worklist_next` 也被挡，导致 agent 难以重新拿 DB worklist。
- **已完成**：
  - `SubmitRepairMode::effective_allowed_tools` 对 coverage-gap repair 永远保留只读 `stage_worklist_status` / `stage_worklist_next`，包括 StageRefiner 传入 `allowed_tools_override` 的 resume/retry 路径。
  - coverage-gap repair 文案改为鼓励用 `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage` / `query_target_data` 刷新 DB truth；扫描类工具仍只允许 coverage_gap_actions 里的 target。
  - `StageRefiner` 的 Enumeration coverage-gap allowed tools 显式加入 `stage_worklist_status` / `stage_worklist_next`，避免模型看到的 allowed list 与 executor guard 不一致。
  - 回归测试覆盖：worklist 工具可用；`browser_collect_js_api` 打 `https://package.moresec.cn` 这种未点名目标仍会被 `not in coverage_gap_actions` 拦截。
  - 同步模块卡：`docs/modules/backend/golish-sub-agents/executor.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-sub-agents -p golish-agent-kit`（cwd `backend`）→ exit 0。
  - `git diff --check -- backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-agent-kit/task_orchestrator.md` → exit 0。
  - `cargo test -p golish-sub-agents coverage_gap_repair -- --nocapture`（cwd `backend`）→ 7 passed / 107 filtered out。
  - `cargo test -p golish-agent-kit enumeration_coverage_gap_directive_preserves_worklist_refresh_tools -- --nocapture`（cwd `backend`）→ 1 passed / 817 filtered out。
- **未跑**：`just precommit` / `./init.sh`（当前工作树已有大量非本轮未提交改动；本轮保持 scoped Rust 验证）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`docs/modules/backend/golish-sub-agents/executor.md`、`docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`agent-progress.md`。

#### 2026-07-03 · enumeration 批次化 + JS 轴终态工具自负（根治死循环）

- **本轮目标**：修 enumeration 反复 block 的根治缺口（`GOLISH-ENUM-JS` 在 enumerator 子代理路径不落 checked_empty → gate 数学上不可能通过），并把三个单 target 内容采集工具改成**批次多 target**（分步骤，非大一统工具），katana 作补充语料合并去重。设计/计划：`docs/design/2026-07-03-enumeration-batch-and-terminal-coverage.md`、`docs/superpowers/plans/2026-07-03-enumeration-batch-katana.md`。
- **根因定位（本轮核对）**：DIR/PARAM/JSAPI 三轴的 evidence + technique_outcome 前几轮已由 bridge 工具自负（`route_probe_paths::upsert_dir_outcome`、`js_extract_apis::upsert_param_outcome/upsert_jsapi_outcome`、`browser_collect_js_api::upsert_jsapi_outcome`）；**唯一真缺口** = `GOLISH-ENUM-JS` 只由 runtime `record_enumeration_bridge_evidence` hook 投影，而该 hook 只在主 agent direct 路径跑、不在 enumerator 子代理跑，导致子代理抓 0 JS 时 JS 轴永远 `not_attempted`（DB 实测 JS 0 行印证）。
- **已完成（P0 根治）**：
  - `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`：新增 `js_outcome_from_browser`（found/empty/error 判定）+ `record_js_outcome` + `upsert_js_outcome`（`append_bridge_evidence(technique=TECH_ENUM_JS)` + `technique_outcomes::upsert`），在 `execute_single` 结尾与 JSAPI 并列落 `GOLISH-ENUM-JS` 终态；结果新增 `js_outcome` / `js_outcome_persisted`。
  - `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`：删除 `record_enumeration_bridge_evidence` 调用 + 函数本体 + `enumeration_evidence_projections` / `EnumerationEvidenceProjection` / `resolve_enumeration_target_asset` / `enumeration_subject` / `browser_js_outcome` / `compact_enumeration_raw_output` / `host_asset_from_subject` / `value_str` / `value_bool` / `count_value` / `array_count` 及其测试（现全部由 bridge 工具自负，避免主 agent 路径 JS 双写）；`use serde_json::{json, Value}` 收窄为 `json`。
- **已完成（P1 批次多 target）**：
  - `browser_collect_js_api` / `js_extract_apis` 加 `target_urls: []`，`route_probe_paths` 加 `targets: [{target_id, base_url}]`（各 ≤50 去重）；`execute` 拆分派器 + `execute_batch`（循环复用 `execute_single`，per-target 各自落终态，单个失败进 `errors` 不中断）；单 target 入参保留向后兼容。批次解析 helper：`browser_collect_js_api::batch_target_urls`（js_extract 复用）、`route_probe_paths::batch_probe_targets`。
  - `backend/crates/golish-sub-agents/src/executor_types.rs`：`coverage_gap_direct_tool_target_block_reason` 改为对批次入参（`target_urls` / `targets[].base_url`）逐项对照 `coverage_gap_actions`，任一未点名即整批 block。
- **已完成（P2 katana + prompt）**：`resources/harness/stages/enumeration/methodology.md`、`backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`（enumerator prompt）改批次口径：批次 browser → katana `-list` 补充 → 批次 js_extract → 批次 route_probe；katana 输出落 `api_endpoints(source='crawler')` 靠 `(target_id,url,method)` 唯一索引自动合并去重。
- **运行过的验证（实跑）**：
  - `cargo check -p golish-pentest-app -p golish-sub-agents -p golish-agent-runtime`（cwd `backend`）→ exit 0（修掉 1 个 unused-import warning 后 0 warning）。
  - `cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-runtime`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app -p golish-sub-agents -E 'test(js_outcome) or test(jsapi_outcome) or test(coverage_gap_repair_batch) or test(coverage_gap_repair_blocks) or test(batch_target)' --status-level fail`（cwd `backend`）→ **18 passed / 255 skipped**，含新增 `js_outcome_*`（3 态）+ `coverage_gap_repair_batch_*`（批次逐项校验）测试。
- **未跑**：`just precommit` / `./init.sh` / 全量 test（用户明确要求「中途不跑 precommit / 大测试」；当前工作树已有大量非本轮未提交改动）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs,route_probe_paths.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/prompts/execution_planning.rs}`、`resources/harness/stages/enumeration/methodology.md`、`docs/design/2026-07-03-enumeration-batch-and-terminal-coverage.md`、`docs/superpowers/plans/2026-07-03-enumeration-batch-katana.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-sub-agents/executor.md`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后续跑 moresec enumeration。验证点：① enumerator 用 `browser_collect_js_api(target_urls=[...])` 一次覆盖整份 web root；② `run_tree.py --db` 中 `GOLISH-ENUM-JS` 出现 empty/found 行（不再 0 行）；③ 无 JS 的 web IP 落 `checked_empty` 而非 `not_attempted`，gate 能通过；④ katana 经 `pentest_run(-list)` 落 `api_endpoints(source=crawler)` 并合并。

#### 2026-07-03 · enumeration not_applicable 降噪（P3，接续上条）

- **本轮目标**：用户追加「补 P3 not_applicable 降噪」。给 enumeration 补一个确定性 not_applicable：端口真值证明「只开 DNS/53 且无 web 服务面」的 in-scope IP，即使有陈旧/残留 http_status 误入 web-capable 分母，也对内容枚举四轴 not_applicable，避免共享 DNS/CDN IP 楔住 gate。
- **方案（复用现有判定，0 新 trait/SQL）**：数据源复用 EAS 已有的 `eas_service_not_applicable_assets`（SQL = `only_dns_port_without_service_surface`）——同一批「只开 53 无 web 面」的 IP 对 EAS 是 SERVICE not_applicable、对 enumeration 是四轴 not_applicable，语义一致。
- **已完成**：
  - `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：新增 `ENUM_CONTENT_TECHNIQUES` 常量；`not_applicable_coverage` 构造从 `if EAS` 改为 `match stage`，加 `Enumeration` 分支 = `eas_service_not_applicable_assets` × 四个 ENUM 技术。
  - `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`：submit 预检 `gate_context` 加 enumeration not_applicable 分支（与 org_gate 对称，预检=stage-close 口径一致，防 preview-PASS→close-BLOCK）。
  - `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：新增 `apply_enum_content_not_applicable` + `cell_technique_static`，UI/worklist 读模型对这些 IP 把仍 pending 的 ENUM 轴改 not_applicable + note，与 gate 一致。
  - `rule_engine::coverage_complete` 的 `context_not_applicable_ok`（已有）据此终态化 cell，无需 agent 自报。
- **运行过的验证（实跑）**：
  - `cargo check -p golish-agent-kit -p golish-agent-app`（cwd `backend`）→ exit 0，0 warning。
  - `cargo fmt -p golish-agent-kit -p golish-agent-app`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-agent-app -E 'test(enum_content_not_applicable) or test(submit_preview_feeds_enumeration)'` → 2 passed / 155 skipped（含新增 `enum_content_not_applicable_terminalises_pending_web_ip_axes`）。
  - `cargo nextest run -p golish-agent-kit -E 'test(org_gate) or test(coverage_complete) or test(enumeration_worklist) or test(verdict) or test(not_applicable)'` → **42 passed / 776 skipped**（确认 EAS not_applicable 与 gate 判定无回归）。
- **未跑**：`just precommit` / `./init.sh` / 全量 test（用户要求中途不跑）。
- **提交记录**：未 commit。
- **本轮修改但未提交（P3 scope）**：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`、`backend/crates/golish-agent-app/src/ai/{harness_submit_tool.rs,commands/stage_coverage.rs}`、`docs/design/2026-07-03-enumeration-batch-and-terminal-coverage.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`agent-progress.md`、`feature_list.json`。
- **下一步建议**：与上条批次改动一起在重启后实跑验证；`run_tree.py --db` 观察只开 53 的 web IP 是否落 not_applicable、不再作为 pending gap。

#### 2026-07-04 · crawler URL 来源归属（crawl_observations + Web Origin Crawl tab）

- **本轮目标**：回应用户要求“爬出来的数据也记录，但归属到哪个 IP/domain/Web Origin，不要直接加进 target”。把 katana/gau/wayback 这类 `endpoint_add` 输出拆成两条语义：same-origin 可继续进入 `api_endpoints(source='crawler')` 作为 ENUM gate truth；外链/三方 URL 只作为来源 origin 的 crawl observation 展示。
- **已完成**：
  - 新增 additive migration `20260704000001_crawl_observations.sql` + `golish_db::repo::crawl_observations` + `CrawlObservation` model。唯一键 `(origin_target_id, observed_url, source_tool, kind)`，只 upsert/list，不写 `targets` / `api_endpoints`。
  - `golish-pentest/output_store/endpoints.rs`：`endpoint_add` 先按 command base URL 写 `crawl_observations`；same-origin/current-org 目标才继续写 `api_endpoints`。单根 crawl 的外链归属到该 root；多根 `-list` 的外链若无法从输出行判断来源 root，不复制到所有 root，避免新污染。
  - `target_surface_hierarchy_get` 读取 candidate origin targets 的 `crawl_observations`，按 `origin_key == web_origins.origin` 挂到每个 `WebOriginHierarchyDto.crawlObservations`。
  - 前端 `security-analysis` normalizer、`backendSurfaceHierarchy` adapter、`WebOriginVM` 与 `WebOriginsTab` 已接新字段；Web Origin detail 新增 `Crawl` tab，总表新增 Crawl 计数列。
  - 同步设计/计划与模块卡：`docs/design/2026-07-04-crawl-observations-origin-ownership.md`、`docs/superpowers/plans/2026-07-04-crawl-observations-origin-ownership.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-pentest/output_store.md`、`docs/modules/backend/golish-pentest-app.md`、`docs/modules/frontend/lib.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**：
  - `jq empty feature_list.json` → exit 0。
  - `cd backend && cargo fmt -p golish-db -p golish-pentest -p golish-pentest-app` → exit 0。
  - `cd backend && cargo check -p golish-db -p golish-pentest -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-db crawl_observations --status-level fail` → 2 passed / 201 skipped。
  - `cd backend && cargo nextest run -p golish-pentest endpoint_ --status-level fail` → 10 passed / 158 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app target_surface_hierarchy --status-level fail` → 16 passed / 149 skipped。
  - `pnpm exec vitest run frontend/lib/api/security-analysis.test.ts frontend/components/TargetPanel/surface/backendSurfaceHierarchy.test.ts frontend/components/TargetPanel/surface/surfaceHierarchy.test.ts` → 31 passed。
  - `./node_modules/.bin/tsc --noEmit --pretty false` → exit 0。
  - `pnpm exec biome check ...selected files...` → exit 0（先用 `biome check --write` 机械格式化了 2 个文件）。
  - `cd backend && cargo clippy -p golish-db -p golish-pentest -p golish-pentest-app --all-targets -- -D warnings` → exit 0。
  - `git diff --check -- <本功能相关非 generated 文件>` → exit 0。
- **未跑 / 已知阻塞**：
  - 未跑 `just precommit`。
  - 全局 `git diff --check` 目前因 `frontend/lib/generated/Target.ts` 的 ts-rs 生成漂移尾随空格失败（`liveness_state` / `liveness_reason` 字段注释生成导致），该文件不是本功能手写范围；未手改 generated。
  - 未做旧污染数据清理：已有被错误加入的第三方 targets / api_endpoints 需要单独 DB cleanup/backfill 策略。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`feature_list.json`、`agent-progress.md`、`backend/crates/golish-db/migrations/20260704000001_crawl_observations.sql`、`backend/crates/golish-db/src/{models/pentest.rs,repo/mod.rs,repo/crawl_observations.rs}`、`backend/crates/golish-pentest/src/output_store/endpoints.rs`、`backend/crates/golish-pentest-app/src/target_surface_hierarchy.rs`、`frontend/lib/{api/security-analysis.ts,api/security-analysis.test.ts,security-analysis.ts}`、`frontend/components/TargetPanel/surface/{surfaceHierarchy.ts,backendSurfaceHierarchy.ts,backendSurfaceHierarchy.test.ts,tabs/WebOriginsTab.tsx}`、设计/计划/模块卡文件。
- **下一步建议**：重启 app/backend 让 migration apply 后跑一轮 enumeration；检查 Web Origin 的 `Crawl` tab 是否显示从该 origin 爬出的外链，同时 `api_endpoints` / coverage 不再新增 github/lodash/ted/wiki 等三方域。若要清掉旧污染，另起一次只读统计 + 确认后删除/降级。

#### 2026-07-05 · EAS 能力 wrapper runner（Prober 不再手写 nmap/httpx）

- **本轮目标**：回应用户确认的“把能力包装成工具”方向，把 EAS 的 liveness / port discovery / service fingerprint 从模型手写 `pentest_run(tool_name,args)` 改成 backend-owned wrapper 工具：模型只选业务动作和目标，底层 recipe / target guard / evidence DB 落地由后端控制。
- **已完成**：
  - 新增 `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`：`eas_probe_http_liveness` 包装固定 `httpx -json -sc -title -td -server -silent`；`eas_discover_ports` 包装 `naabu` / `nmap` / `masscan` list-file recipe，且只接受 IP/CIDR；`eas_fingerprint_services` 包装 `nmap -sV -Pn -iL ... -p ... -T3`，且只接受 concrete IP + ports。
  - `PentestRunTool::from_config_manager` 让 wrapper 复用现有 `pentest_run` 执行、后台任务、audit、output-store 路径；wrapper result 追加 `wrapper_tool` / `capability` / `wrapped_tool_name` / `wrapped_args` / `targets_count`。
  - `create_pentest_bridge_tools` 注册三个 EAS wrapper；`tool_taxonomy` 把 wrapper 归入 `recon/http` / `recon/port-scan`；`stage_capability` 的 EAS 三个能力改为 `BackendWrapper` 并建议 `eas_*` 工具。
  - Prober 默认工具集去掉 `pentest_run` / `pentest_list_tools`，加入 `eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services`；Prober prompt 和 EAS methodology 改为禁止 raw `httpx` / `nmap` / `pentest_run`。
  - `golish-agent-runtime` 与 `golish-sub-agents` 的 structured-storage hook 识别 EAS wrapper，并从 result 的 `wrapped_tool_name` / `wrapped_args` 还原底层命令，确保 wrapper stdout 仍走原有 output-store / evidence / technique_outcomes 落库链路。
  - Coverage-gap repair 对 `GOLISH-EAS-*` 缺口允许 `eas_*` wrapper，且要求 `targets[]` 都出现在 `coverage_gap_actions`；raw `pentest_run` 在 EAS repair 中被直接挡掉。Enumeration/katana 这类仍需 `pentest_run` 的路径保持原 guard。
  - 同步模块卡：`golish-pentest-app.md`、`golish-pentest-app/pentest_bridge.md`、`golish-agent-kit/harness.md`、`golish-agent-runtime/agentic_loop.md`、`golish-sub-agents/defaults.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-pentest-app -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail`（cwd `backend`）→ 2 passed / 167 skipped。
  - `cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail`（cwd `backend`）→ 25 passed / 801 skipped。
  - `cargo nextest run -p golish-sub-agents defaults coverage_gap eas_wrapper --status-level fail`（cwd `backend`）→ 34 passed / 84 skipped。
  - `cargo nextest run -p golish-agent-runtime eas_wrapper pentest_run_result_feeds_structured_storage_hook --status-level fail`（cwd `backend`）→ 2 passed / 288 skipped。
- **未跑**：`just precommit` / `./init.sh` / live DeepSeek Flash wrapper smoke（本轮只做 scoped Rust verification；当前工作树已有大量非本轮未提交改动，且 live 扫描需单独选择授权目标）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_ai/run.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,eas_capabilities.rs}`、`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,tool_taxonomy.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、`resources/harness/stages/external_attack_surface/methodology.md`、相关模块卡、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后跑一个授权的小目标 EAS smoke，检查 transcript 中 Prober 是否真实调用 `eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services`，并用 `scripts/run_tree.py --db` 验证 liveness/ports/fingerprints/evidence/technique_outcomes 都由 wrapper run 落库；再跑允许范围内的 full `just precommit`。

#### 2026-07-05 · Enumeration 能力 wrapper runner（Enumerator 不再手写 katana/pentest_run）

- **本轮目标**：回应用户“先改 enumeration”，把 Enumeration 的 katana crawler supplement 从模型手写 `pentest_run(tool_name="katana", args="-list ...")` 改成 backend-owned wrapper 工具。模型只传 web-root URL 和 depth；底层 katana recipe、URL guard、output-store/evidence/DB 落地由后端控制。
- **已完成**：
  - 新增 `backend/crates/golish-pentest-app/src/pentest_bridge/enumeration_capabilities.rs`：`enum_crawl_same_origin_urls(target_urls, depth, timeout_secs?, background?)` 包装固定 `katana -list {{input_file}} -jc -silent -d N`；只接受 http(s) URL，去重、去 fragment、拒绝 credentials，最多 50 个目标，depth 默认 2、最多 5。
  - `create_pentest_bridge_tools` 注册 `EnumCrawlSameOriginUrlsTool`；wrapper result 追加 `wrapper_tool` / `capability` / `wrapped_tool_name` / `wrapped_args` / `targets_count`，继续复用原 `pentest_run` 执行、后台任务、audit、output-store 路径。
  - `stage_capability` 的 `enum.crawl_same_origin_urls` 改为 `BackendWrapper` 并建议 `enum_crawl_same_origin_urls`；`tool_taxonomy` 把 wrapper 纳入 `recon/crawler` 和 canonical tool set。
  - Enumerator 默认工具集去掉 `pentest_run` / `pentest_list_tools`，加入 `enum_crawl_same_origin_urls`；Enumerator prompt 与 enumeration methodology 改成禁止 raw katana / raw `pentest_run`。
  - Coverage-gap repair 对 `GOLISH-ENUM-*` 缺口会把旧 `katana` hint 映射到 `enum_crawl_same_origin_urls`，允许 wrapper 批量 `target_urls` 逐项校验，且挡掉 raw katana/pentest_run。
  - `golish-agent-runtime` 与 `golish-sub-agents` 的 structured-storage hook 从 EAS-only 改为通用 wrapper 识别：只要 result 带 `wrapped_tool_name` / `wrapped_args`，就按底层 pentest 输出触发 output-store/evidence 落库。
  - 前端 `frontend/lib/tools.ts` 对新 wrapper 显示 `Crawling same-origin URLs`，折叠摘要显示 `target_urls` 批量数量/首尾目标和 depth，不暴露 snake_case 内部名。
  - 同步模块卡：`golish-pentest-app(.md/pentest_bridge.md)`、`golish-agent-kit/harness.md`、`golish-agent-runtime/agentic_loop.md`、`golish-sub-agents(.md/defaults.md/executor.md)`、`frontend/lib.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt -p golish-pentest-app -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents`（cwd `backend`）→ exit 0。
  - `cargo check -p golish-pentest-app -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents`（cwd `backend`）→ exit 0。
  - `cargo nextest run -p golish-pentest-app enumeration_capabilities --status-level fail`（cwd `backend`）→ 3 passed / 169 skipped。
  - `cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail`（cwd `backend`）→ 26 passed / 801 skipped。
  - `cargo nextest run -p golish-sub-agents defaults coverage_gap enum_wrapper --status-level fail`（cwd `backend`）→ 34 passed / 85 skipped。
  - `cargo nextest run -p golish-agent-runtime enum_wrapper eas_wrapper pentest_run_result_feeds_structured_storage_hook --status-level fail`（cwd `backend`）→ 3 passed / 288 skipped。
  - `pnpm exec vitest run frontend/lib/tools.test.ts` → 19 passed。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
- **未跑**：`just precommit` / `./init.sh` / live DeepSeek Flash enumeration wrapper smoke（本轮做 scoped verification；当前工作树已有大量非本轮未提交改动，live 扫描需重启 app/backend 后选择授权目标）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,enumeration_capabilities.rs}`、`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,tool_taxonomy.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、`resources/harness/stages/enumeration/methodology.md`、`frontend/lib/{tools.ts,tools.test.ts}`、相关模块卡、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后跑一个授权的小 enumeration smoke，检查 transcript 中 Enumerator 是否真实调用 `enum_crawl_same_origin_urls`，并用 `scripts/run_tree.py --db` 验证 katana crawler output 经 wrapper 落到 `api_endpoints(source='crawler')` / evidence / technique outcomes；再跑允许范围内的 full `just precommit`。

#### 2026-07-05 · Test1 latest enumeration block 诊断 + target_id batch repair

- **本轮目标**：回应用户“这次为什么一直 block、跑不完、太慢”，重新读取 `/Users/christopherzheng/golish-platform/Test1` 最新 run，定位 enumeration gate block 根因，并做最小 contract 修复。
- **诊断结论**：
  - 最新 run 是 `pentest-chat-1783239100928-1`，当前 profile 为 `red_team`，已完成 `scoping` / `target_intel` / `external_attack_surface`，当前卡在 `enumeration`，不是此前 `enumeration -> reporting` DAG 顺序问题。
  - gate block 原因是 `coverage_complete`：第一次 39 个 ENUM coverage cell 未终态，第二次降到 35 个；主要是 `ecs-123-60-169-120.compute.hwclouds-dns.com`、`hebei.22.121.IN-ADDR.ARPA` 和多个 `zta-*.moresec.com.cn` 的 `GOLISH-ENUM-JS/DIR/PARAM/JSAPI`。
  - DB 证明工具其实跑了很多：`directory_entries=414`、`api_endpoints=2474`、`js_analysis_results=374`，但 run.log 大量出现 `[browser_collect_js_api] no target_id, skipping API DB persistence`，导致结果没有稳定绑定到 gate 正在检查的 target_id，coverage truth 仍看成 never-attempted。
  - submit-repair lock 还挡过正确修复工具：`list_enumeration_web_roots`、`list_recent_evidence`、`enum_crawl_same_origin_urls`、`browser_collect_js_api`、`js_extract_apis`、`route_probe_paths`；之后模型退化为逐个 URL 猜测和单点 `js_collect`，单次 JS collection 可跑数分钟，所以整体显得“慢且一直 block”。
- **已完成**：
  - `browser_collect_js_api` / `js_extract_apis` 的 batch `target_urls` 除字符串外，现在接受 worklist 对象 `{target_id, target_url|root_url|base_url|url}`，并在 batch loop 中保留 `target_id` 传给 single-target 执行路径，避免 URL 自动匹配失败导致 no-target-id 落库缺口。
  - `SubmitRepairMode` coverage-gap repair 保留 `list_recent_evidence`；遇到 ENUM gap 时保留 `list_enumeration_web_roots`；direct tool fence 支持对象型 `target_urls` 并逐项校验目标仍在 `coverage_gap_actions` 内。
  - `StageRefiner` Enumeration repair allowlist 补 `list_recent_evidence` 和 `enum_crawl_same_origin_urls`，与 executor guard 保持一致。
  - Enumerator prompt 改为推荐 `target_urls=[{target_id, target_url}, ...]`，让 DeepSeek Flash 从 worklist/root context 直接带 DB 主键跑 batch，不再只传裸 URL 字符串。
  - 同步模块卡：`golish-pentest-app/pentest_bridge.md`、`golish-sub-agents.md`、`golish-agent-kit/task_orchestrator.md`。
- **运行过的验证（实跑）**：
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` → exit 0；确认最新 run `pentest-chat-1783239100928-1` 的 enumeration block 为 39/35 个 coverage cell，DB 有大量内容行但日志有 `no target_id`。
  - `tail -n 220 /Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1783239100928-1/run.log` / `rg ... run.log` → exit 0；确认 repair block 工具和 `no target_id` 行。
  - `cd backend && cargo nextest run -p golish-pentest-app batch_target_urls --status-level fail` → 1 passed。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair --status-level fail` → 9 passed。
  - `cd backend && cargo nextest run -p golish-agent-kit enumeration_coverage_gap_directive_preserves_worklist_refresh_tools --status-level fail` → 1 passed。
  - `cd backend && cargo nextest run -p golish-sub-agents test_enumerator_prompt_is_content_enum --status-level fail` → 1 passed。
  - `cd backend && cargo nextest run -p golish-pentest-app browser_collect_js_api js_extract_apis --status-level fail` → 42 passed。
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --check` → exit 0。
  - `git diff --check -- <本轮相关文件>` → exit 0。
- **未跑**：`just precommit` / `./init.sh` / live rerun（当前工作区已有大量非本轮未提交改动；本轮做 scoped log/DB diagnosis + targeted Rust validation。新契约需要重启 app/backend 后重新跑/续跑 enumeration 才会生效）。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs}`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`docs/modules/backend/{golish-pentest-app/pentest_bridge.md,golish-sub-agents.md,golish-agent-kit/task_orchestrator.md}`、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后重新跑或续跑 Test1 的 enumeration；预期 repair path 会先用 worklist/root context 拿 `{target_id,target_url}`，批量调用 browser/js_extract/route_probe/crawler wrapper，`run_tree.py --db` 不应再出现成片 `no target_id, skipping API DB persistence`，剩余 gap 应快速收敛。

#### 2026-07-05 · Vuln-triage wrapper runner（Vuln Scanner 不再手写 nuclei/sqlmap）

- **本轮目标**：回应用户指出“这个阶段也应该像之前阶段一样把功能参数包装起来，而不是让 AI 自己输命令”。把 `vuln_triage` 从临时借 `sub_agent_pentester` + raw `pentest_run` 改成专门的 `vuln_scanner` + backend-owned formulaic sweep wrapper。
- **已完成**：
  - 新增 `vuln_run_formulaic_sweep(targets, techniques, timeout_secs?)`，后端固定封装 `nuclei` / `sqlmap` / `wpscan` recipe，并强制走 background job，让现有 background outcome listener 写 `technique_outcomes`。
  - `vuln_triage` spec / stage capability / taxonomy 改为只建议并允许 wrapper 工具；raw `nuclei` / `sqlmap` / `wpscan` 不再作为 stage 允许工具暴露给模型。
  - 新增真正的 `sub_agent_vuln_scanner` 默认 worker：工具集只包含 worklist、target data、wrapper、background wait/check、evidence、coverage、submit/record finding 等，移除 `pentest_run` / `pentest_list_tools`。
  - `stage_run` 不再把 `vuln_scanner` 临时映射到 `sub_agent_pentester`；stage_run objective 也改成“优先直接调用 backend wrapper/direct tool”，不再默认要求经 `pentest_run`。
  - Coverage-gap repair 对 vuln 缺口允许 `vuln_run_formulaic_sweep`，挡掉 raw `pentest_run`，并要求 wrapper 参数显式带 `targets[]` + `techniques[]`；每个 target x technique 必须都来自当前 gap action，避免 wrapper 扫描越权扩大范围。
  - `bridge_config` 支持从 `nuclei -l/-list` 与 `sqlmap -m/--bulk-file` 识别批量 input file，保证 wrapper 底层输出仍可回到原 output-store / evidence / DB 落库链路。
  - 前端工具标签/摘要新增 `vuln_run_formulaic_sweep`，显示 targets 与 techniques，而不是暴露底层命令细节。
  - 同步模块卡：`golish-agent-kit/harness.md`、`golish-agent-kit/tool_executors.md`、`golish-pentest-app/pentest_bridge.md`、`golish-sub-agents/defaults.md`、`golish-agent-runtime/agentic_loop.md`。
- **运行过的验证（实跑）**：
  - `rustfmt --edition 2021 <本轮 touched Rust files>` → exit 0。
  - `pnpm exec biome format --write frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app vuln_capabilities --status-level fail` → 3 passed / 173 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit vuln --status-level fail` → 22 passed / 809 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit tool_taxonomy --status-level fail` → 20 passed / 811 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_capability --status-level fail` → 7 passed / 824 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_refiner --status-level fail` → 6 passed / 825 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents defaults --status-level fail` → 25 passed / 98 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents coverage_gap_repair_uses_vuln --status-level fail` → 1 passed / 122 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_label_and_role_label_title_case --status-level fail` → 1 passed / 291 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app batch_vuln --status-level fail` → 2 passed / 158 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app vuln_triage_exposes_formulaic_scan_axes --status-level fail` → 1 passed / 159 skipped。
  - `pnpm vitest run frontend/lib/tools.test.ts` → 21 passed。
  - `cd backend && cargo nextest run -p golish-sub-agents discovery --status-level fail` → 5 passed / 118 skipped。
  - `python3 -m json.tool resources/harness/stages/vuln_triage/spec.json` / `python3 -m json.tool feature_list.json` → exit 0。
  - `git diff --check -- <本轮 touched files>` → exit 0。
- **未跑**：`./init.sh`（用户明确不要跑）/ `just precommit` / 全量测试 / live Test1 vuln_triage rerun。当前只是代码契约和 scoped tests 通过，真实阶段还需要重启 app/backend 后继续或重跑。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,vuln_capabilities.rs}`、`backend/crates/golish-agent-app/src/ai/commands/{bridge_config.rs,stage_coverage.rs}`、`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,stage_spec.rs,tool_taxonomy.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`backend/crates/golish-sub-agents/src/{defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/execution_planning.rs,defaults/prompts/mod.rs,defaults/tests.rs,executor_types.rs,executor/response_parsing.rs}`、`resources/harness/stages/vuln_triage/spec.json`、`frontend/lib/{tools.ts,tools.test.ts}`、相关模块卡、`feature_list.json`、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后在 Test1 继续或重跑 `vuln_triage`；期望 transcript 中出现 `sub_agent_vuln_scanner` 调 `vuln_run_formulaic_sweep`，不再出现 vuln repair raw `pentest_run`/手写 nuclei/sqlmap；随后用 `scripts/run_tree.py --db` 验证 `technique_outcomes` 覆盖 WSTG/GOLISH cells，再跑允许范围内的 full `just precommit`。

#### 2026-07-06 · EAS service/web fingerprint contract tighten

- **本轮目标**：回应用户指出“所有扫描出来的新端口，只要能跑都要跑一次；同 IP 多域名怎么办”，把 EAS SERVICE 与 web stack fingerprint 拆清楚：SERVICE 必须覆盖每个 confirmed-open IP:port，WhatWeb 只按 confirmed web origin 做 Host/SNI-aware enrichment。
- **已完成**：
  - 新增并注册 `eas_fingerprint_web_stack` wrapper：封装 `whatweb -a <aggression> --input-file {{input_file}} --max-threads <n>`，只接受 absolute HTTP(S) URL，落地走现有 `pentest_run` / output-store / fingerprints / Target UI 链路。
  - Prober 默认工具、repair allow-list、stage capability、taxonomy、前端工具标签同步暴露 WhatWeb wrapper；prompt/methodology 明确禁止 raw `whatweb` / raw `pentest_run`。
  - 收紧 EAS SERVICE gate truth：`GOLISH-EAS-SERVICE-FINGERPRINT` 不再被任意 `fingerprints` 行、target-level found outcome 或 DNS-only shortcut 满足；必须基于 `targets.ports[]` 中每个 open port 的 service/version/product/banner/webserver/technologies 等 port-level surface，或同 target/port 的 nmap service fingerprint。
  - WhatWeb 不再写 `GOLISH-EAS-SERVICE-FINGERPRINT` outcome，evidence fact mapping 也不再把 WhatWeb 映射到 SERVICE；它只做 web-origin/fingerprint enrichment，不替代 nmap IP:port service fingerprint。
  - EAS worklist / refiner / charter / methodology 全部改成：端口发现扩展后，只对新增 confirmed-open ports 补跑 `eas_fingerprint_services`；同一个 IP:port 有多个 domain/vhost 时，nmap 对 IP:port 跑一次，WhatWeb 对每个 confirmed `scheme://host:port` web origin 分别跑一次。
  - 同步模块卡：`golish-db/repo.md`、`golish-agent-app/ai.md`、`golish-agent-kit/harness.md`、`golish-pentest-app/pentest_bridge.md`、`golish-sub-agents/defaults.md`、`frontend/lib.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo nextest run -p golish-agent-kit evidence_facts tool_executors stage_refiner prompts org_gate --status-level fail` → 104 passed / 728 skipped。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail` → 28 passed / 804 skipped。
  - `cd backend && cargo nextest run -p golish-agent-app stage_coverage bridge_config harness_submit_tool --status-level fail` → 115 passed / 45 skipped。
  - `cd backend && cargo nextest run -p golish-sub-agents defaults executor_types response_parsing --status-level fail` → 57 passed / 66 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail` → 首次遇到本地 test link cache 缺 `librig...rlib`，直接重跑后 3 passed / 174 skipped。
  - `cd backend && cargo nextest run -p golish-db coverage_truth --status-level fail` → 32 passed / 171 skipped。
  - 默安 Test1 DB 只读验证：`python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 确认 latest run/org；定制 SQL 复算 strict SERVICE truth，发现旧口径会让 `115.175.6.207` / `36.42.77.170` 的 target-level SERVICE found outcome 盖过 port gap；修正后 nmap port-specific fingerprints 可关闭 `115.175.6.207:8082/8083`，剩余真实 gap 只剩 `36.42.77.170:{888,1935,2002,5100,9001}`。
  - `pnpm exec vitest run frontend/lib/tools.test.ts` → 22 passed。
  - `cd backend && cargo fmt --all` / `cd backend && cargo fmt --all -- --check` → exit 0。
  - `pnpm exec biome check frontend/lib/tools.ts frontend/lib/tools.test.ts` → exit 0。
  - `git diff --check -- <本轮 touched files>` → exit 0。
- **未跑**：`./init.sh` / `just precommit` / live Test1 rerun。本轮做 contract-level code fix + scoped verification；真实 run 需要重启 app/backend 后重新跑或续跑 EAS。
- **提交记录**：未 commit。
- **本轮修改但未提交（本 scope）**：`backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,eas_capabilities.rs}`、`backend/crates/golish-agent-kit/src/harness/{stage_capability.rs,tool_taxonomy.rs,evidence_facts.rs,org_gate.rs}`、`backend/crates/golish-agent-kit/src/{db_traits/repo.rs,tool_executors/security.rs,task_orchestrator/prompts/mod.rs,task_orchestrator/stage_refiner.rs}`、`backend/crates/golish-db/src/repo/coverage_truth.rs`、`backend/crates/golish-agent-app/src/ai/commands/{bridge_config.rs,stage_coverage.rs}`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`backend/crates/golish-sub-agents/src/{executor_types.rs,executor/response_parsing.rs,defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/execution_planning.rs,defaults/tests.rs}`、`frontend/lib/{tools.ts,tools.test.ts}`、`resources/harness/stages/external_attack_surface/methodology.md`、相关模块卡、`agent-progress.md`。
- **下一步建议**：重启 app/backend 后重跑授权 EAS smoke；用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 看 Prober 是否对每个 confirmed-open IP:port 调 `eas_fingerprint_services`，对每个 confirmed web origin 调 `eas_fingerprint_web_stack`，并确认 WhatWeb 只增加 fingerprints/web-origin evidence，不再把 SERVICE gate 盖绿。

#### 2026-07-06 · EAS IP-first live validation on Test1 / 默安

- **本轮目标**：按用户要求用 Test1 中杭州默安科技有限公司的真实 DB/worklist 跑 EAS 阶段，确认当前逻辑是否已经变成“IP/CIDR 先端口发现，端口落库后按服务 / web origin 分别跑 nmap / WhatWeb”。
- **已完成**：
  - 实测旧/第一次新 run 仍有问题：`stage-run-ba720b94-26cd-4442-b357-f10773839639` 中 Prober 仍把 bare IP 混入 `eas_probe_http_liveness` / httpx，说明只改 prompt/gate 不够。
  - 加强 wrapper 级硬拦：`eas_probe_http_liveness` 现在拒绝 bare IP、CIDR、bare `IP:port`；sub-agent prompt / builder / worklist 文案同步明确 concrete IP/CIDR 的 LIVENESS 必须先由 `eas_discover_ports` 关闭。
  - 第二次真实 run `stage-run-b5f5a708-da14-4ffd-9aab-044ac77aa8bb` 已验证主路径：httpx 输入只有 domain；naabu 输入 concrete IP；naabu outcome 同时写 `GOLISH-EAS-PORT` 和 IP 侧 `GOLISH-EAS-LIVENESS`；WhatWeb 写 `GOLISH-EAS-WEB-FINGERPRINT`；nmap 写 `GOLISH-EAS-SERVICE-FINGERPRINT`。
  - 追加修正一条误导性日志：WhatWeb completion 文案从“web fingerprints persisted without SERVICE-FINGERPRINT outcome”改为“WEB-FINGERPRINT outcomes stored without SERVICE-FINGERPRINT outcome”。
  - 发现剩余设计点：真实 run 会持续把 DNS 解析 / 扫描中新出现的 IP 也拉进当前 repair loop；这符合“扫出来能跑的都跑一次”的方向，但与 durable wave / next-wave backlog 的边界还需要单独收紧，否则 EAS 可能在一轮里不断扩张。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest-app -p golish-sub-agents -p golish-agent-kit --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest-app liveness_rejects_bare_ip_targets_but_allows_web_origins port_targets_are_ip_or_cidr_only web_fingerprint_targets_are_absolute_http_urls_only --status-level fail` → 3 passed。
  - `cd backend && cargo nextest run -p golish-sub-agents test_prober_prompt_is_active_surface test_prober_has_active_surface_tools --status-level fail` → 2 passed。
  - `cd backend && cargo nextest run -p golish-agent-kit stage_worklist_next_surfaces_eas_tool_boundary --status-level fail` → 1 passed。
  - `cd backend && cargo fmt -p golish-agent-app --check` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-app bridge_config --status-level fail` → 38 passed / 126 skipped。
  - `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 stage-run-b5f5a708-da14-4ffd-9aab-044ac77aa8bb --full --db` → exit 0；DB outcome snapshot before stopping: `GOLISH-EAS-LIVENESS found via naabu: 33 assets`, `GOLISH-EAS-LIVENESS empty via naabu: 16 assets`, `GOLISH-EAS-PORT found via naabu: 33 assets`, `GOLISH-EAS-PORT empty via naabu: 16 assets`, `GOLISH-EAS-SERVICE-FINGERPRINT found via nmap: 41 assets`, `GOLISH-EAS-WEB-FINGERPRINT found via whatweb: 60 assets`, `GOLISH-EAS-WEB-FINGERPRINT empty via whatweb: 20 assets`。
  - 真实 run evidence ids included `httpx #17234`, `naabu #17235/#17256/#17262`, `nmap #17241/#17244/#17259`, `whatweb #17240/#17251/#17252/#17253`。
  - `git diff --check` → exit 0。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit`。第二次真实 run 为讨论/验证流程而启动，已在进入继续扩张的 repair loop 后手动 `SIGINT` 停止；没有宣称整轮 EAS gate PASS。
- **提交记录**：未 commit。
- **下一步建议**：讨论并明确 EAS wave 边界：同一轮是否允许 DNS 解析出的新 IP 立即进入当前 batch；若允许，要把 worklist/gate 语义改成“expanding wave”；若不允许，要让 `check_stage_asset_coverage`/repair loop 把新资产明确标为 next-wave backlog，避免 ready_to_submit 被新发现资产反复拖住。

#### 2026-07-06 · EAS supplemental wave barrier direction + 默安 live check

- **本轮目标**：回应用户“要不换一个逻辑，改成这个阶段新增的再跑一次 runstage；明确只补这个阶段才进入数据库的资产，无论端口还是 web”，把 EAS wave 从“当前 run 内不断扩张”改成“当前 wave 冻结，新增资产排 supplemental wave，下次 stage_run 只处理新增资产”的方向，并用 Test1 默安真实 DB 跑到关键路径验证。
- **已完成**：
  - `StageAssetWaveView` 暴露 `parent_wave_id`，`stage_run` 在当前 durable wave PASS 后创建带 `parent_wave_id` 的 supplemental delta wave，并把 `asset_values` 放进返回的 `expansion_batches`，提示用户重新跑 `stage_run`，下一次只处理新增 batch。
  - `stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next` 支持当前 wave asset_values 作为显式分母；当前 wave 外的新资产在 coverage UI/notes 中标成 supplemental/backlog，不把当前 gate 变成无限扩张。
  - `list_in_scope_targets` / `list_attack_surface_seeds` 在 active durable wave 下过滤到当前 wave，并返回 `current_wave_filtered`，避免子 agent 从普通列表入口拿到下一波资产。
  - `resume_skip_covers_current_wave` 对 supplemental wave 不再被旧的 stage pass ledger 误跳过，确保“新增资产再跑一次 stage_run”真的会跑。
  - 同步 EAS spec / 模块卡 / 前端 coverage test 文案。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch resume_skip_covers_current_or_legacy_backfilled_wave -p golish-agent-app next_wave_cells_are_marked_without_suggested_tools explicit_current_wave_assets_override_created_at_cutoff explicit_current_wave_assets_defer_assets_outside_wave --status-level fail` → 5 passed。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave resume_skip_covers_current_or_legacy_backfilled_wave -p golish-agent-app next_wave explicit_current_wave -p golish-agent-kit stage_worklist coverage_preflight --status-level fail` → 14 passed。
  - `cd backend && cargo fmt -p golish-agent-kit && cargo nextest run -p golish-agent-kit current_wave_filter_limits_listing_rows_to_wave_values stage_worklist coverage_preflight --status-level fail` → 8 passed。
  - `pnpm test:run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 23 passed。
  - 默安真实 run：`cd backend && cargo run -q -p golish --bin golish -- --stage-run --profile red_team --only external_attack_surface --auto-approve --db-smoke-summary --org "杭州默安科技有限公司" ... /Users/christopherzheng/golish-platform/Test1`，session `stage-run-313fb3e7-0789-4444-880f-7a6aeca5cd23`。
  - 真实 run 观察到：sub-agent `list_attack_surface_seeds` / `list_in_scope_targets` 均为 `current_wave_filtered=true`；DB 当前 operation `88f714dc-938f-4271-a2f1-525aca5db3ec` 的 wave 0 为 `running` 且 `items=130`；httpx/naabu/nmap/whatweb 结果均写入 evidence/target surface；WhatWeb 落地示例包括 `stored_origins=31 stored_fingerprints=192` 与 `stored_origins=90 stored_fingerprints=203`。
  - 真实 run 中后续 prober 扫描的 IP 经 SQL 验证均在当前 wave_items 内；没有发现它从列表入口越界扫 supplemental 资产。
  - 用户中途问“wave 方向是什么”，按暂停处理，手动 `Ctrl-C` 结束 CLI；随后 `ps -ax -o pid=,ppid=,command= | rg 'stage-run-313fb3e7|nmap|naabu|httpx|whatweb|golish -- --stage-run'` 只剩检查命令自身，没有遗留扫描进程。
- **发现的问题 / 未完成**：
  - 该 live run 未跑到 current wave PASS，因此还没有用真实 run 看到 `expansion_batches` / supplemental wave 创建；这部分目前只由单元测试和 DB 代码路径覆盖。
  - 第二个 prober 中仍出现裸 `whatweb` 空命令和 `eas_fingerprint_web_stack` 被 submit-repair mode 拦掉的日志；说明 WhatWeb wrapper/repair allow-list 还需要继续收紧，让模型只能走包装工具。
  - 未跑 `./init.sh` / `just precommit`；本轮为 scoped tests + live validation。
- **提交记录**：未 commit。
- **下一步建议**：先和用户确认 wave 语义：当前 run 冻结当前 wave；本阶段新入库的端口/web/service 生成 supplemental wave；下一次 `stage_run` 只补这批新增资产。确认后继续修 wrapper gate：让 EAS repair mode 显式允许 `eas_fingerprint_web_stack`，同时减少/禁止 raw `whatweb` 空跑路径。

#### 2026-07-06 · EAS supplemental wave + repair wrapper tightening follow-up

- **本轮目标**：继续完成用户目标“第一次 runstage 提交后查看本阶段新增资产，再进入补充 runstage；补充 runstage 只补本阶段新增的端口/web 资产，并实际验证”。重点修复上一轮 live check 暴露的两点：supplemental wave 不能把旧未分配资产误当新增；EAS repair mode 不能挡掉 WhatWeb wrapper 或诱导 raw whatweb/pentest_run。
- **已完成**：
  - `stage_asset_waves::create_next` 的 delta 候选从“本 operation/stage 未进过 wave 的 target”收紧为“`parent_wave.started_at` 之后新入库、且未进过本 operation/stage wave 的 in-scope target”。没有 parent wave 时才回退 legacy `org_stage_completions.passed_at` floor。这样补充 runstage 只吃当前阶段运行中新落库资产，不会把 current wave limit 截断之外的老 target 混进去。
  - StageRefiner 的 EAS coverage-gap repair 从 raw `httpx/naabu/nmap/whatweb/pentest_run` hint 统一映射到 backend wrappers：`eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services` / `eas_fingerprint_web_stack`；EAS repair allowed_tools 不再暴露 raw `pentest_run` / `pentest_list_tools`。
  - `SubmitRepairMode` 的 EAS wrapper guard 补 `GOLISH-EAS-WEB-FINGERPRINT` → `eas_fingerprint_web_stack` 自动放行，即使 gate action 没有 legacy `suggested_tools` 也不再挡 wrapper；raw `whatweb` / `pentest_run` 仍会被 repair lock block。
  - 同步模块卡：`golish-db/repo.md`、`golish-agent-kit/task_orchestrator.md`、`golish-sub-agents.md`、`golish-sub-agents/executor.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-db -p golish-agent-runtime -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-db stage_asset_wave -p golish-agent-kit eas_coverage_gap submit_needs_fix_prioritizes_eas eas_web_fingerprint_repair -p golish-sub-agents coverage_gap_repair --status-level fail` → 22 passed / 1143 skipped。
  - `cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave resume_skip_covers_current_or_legacy_backfilled_wave -p golish-agent-app next_wave explicit_current_wave -p golish-agent-kit stage_worklist coverage_preflight --status-level fail` → 14 passed / 1281 skipped。
  - `cd backend && cargo nextest run -p golish-db stage_asset_wave -p golish-agent-runtime stage_asset_wave resume_skip_covers_current_or_legacy_backfilled_wave -p golish-agent-app next_wave explicit_current_wave -p golish-agent-kit stage_worklist coverage_preflight eas_coverage_gap submit_needs_fix_prioritizes_eas eas_web_fingerprint_repair -p golish-sub-agents coverage_gap_repair --status-level fail` → 36 passed / 1587 skipped。
  - 本地 Postgres 事务内验证（rollback）：临时插入 org/operation_state/parent wave/targets，包含 parent wave 前老资产、parent wave 已有资产、parent wave 后新资产；用当前 next-wave SQL 查询只返回 `new-after-wave.example.test`，证明 old un-waved target 与 parent item 均被排除。输出：`candidate_count 1 ... proof only parent-wave-new target selected; old un-waved target excluded; parent item excluded`。
  - 本地 Postgres 两轮 wave 事务验证（rollback）：临时插入 parent wave 并标 completed，再按当前 create-next SQL 插入 supplemental wave；随后用 current-running 等价查询读回 wave_index=1、`parent_wave_id=<parent>`，items 只有 `203.0.113.10` 与 `https://app.example.test`，证明第二次 run 的 running wave 只包含本阶段新入库的 port/web 资产。输出：`inserted_delta_items ['203.0.113.10', 'https://app.example.test'] ... proof second run wave contains only stage-new port/web assets and links to parent wave`。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-sub-agents -p golish-db -p golish-agent-runtime -p golish-agent-app -- --check` → exit 0。
  - `pnpm test:run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → 23 passed。
  - `git diff --check` → exit 0。
  - 最终副作用检查：`select count(*) from organizations where name in ('codex-wave-proof-org','codex-wave-two-pass-proof-org')` → 0；`ps ... rg 'stage-run-313fb3e7|nmap|naabu|httpx|whatweb|golish -- --stage-run'` 只返回检查命令自身，无遗留扫描进程。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit` / 新一轮完整默安 EAS live run 到 PASS。CLI `--stage-run` 每次自建 operation 并走真实 LLM orchestration，没有现成无扫描参数接入预置 operation/wave；上一轮 live run 已证明 current wave filtering、naabu/nmap/whatweb 落库和无越界扫描，本轮用事务 DB proof 覆盖 supplemental 两轮 wave 语义，避免再触发大范围真实扫描。
- **提交记录**：未 commit。
- **下一步建议**：若要最终用真实 run 证明 `expansion_batches`，需要挑一个小 scope/小 org 或人工造一个只含 1-2 个 target 的测试 org，再跑两次 `stage_run`：第一次 PASS 后确认返回 supplemental `asset_values`，第二次确认 `current_wave_filtered=true` 且只处理这批 delta。

#### 2026-07-06 · EAS nmap PTR hostname containment

- **本轮目标**：回应用户截图中第二轮 supplemental wave 被描述为“通过反向 DNS 发现”的疑问，修掉 EAS 服务指纹阶段把 nmap 自动反解 hostname/PTR 提升成新 domain target 的路径。
- **已完成**：
  - `eas_fingerprint_services` 与 EAS 内 nmap port discovery recipe 固定加 `-n`，避免 nmap 主动做 DNS 反解；`resources/toolsconfig/nmap.json` 与 `naabu -> nmap` recipe 也加 `-n`，减少 raw recipe 复现同类问题。
  - `golish-pentest::output_parser` 在 `target_update_recon` 多行输出上下文里优先把 `ip` 注入为 canonical `host`，所以 `Nmap scan report for hostname (ip)` 后续 port/service records 会落到 IP target。
  - `golish-pentest::output_store::targets` 对 `tool_name=nmap` 再兜底优先使用 `ip` 作为 target key，防止 parser/legacy path 仍把 PTR hostname 创建为 in-scope domain target。
  - 模块卡同步记录：nmap 反解名只能作为 alias/observation 线索，不能扩大 EAS wave 分母。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest -p golish-pentest-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest nmap_parse_uses_ip_not_rdns_hostname_as_recon_host nmap_recon_target_value_prefers_ip_over_rdns_hostname --status-level fail` → 2 passed / 168 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app nmap_wrappers_disable_dns_resolution liveness_rejects_bare_ip_targets_but_allows_web_origins web_fingerprint_targets_are_absolute_http_urls_only port_targets_are_ip_or_cidr_only --status-level fail` → 4 passed / 175 skipped。
  - `python3 -m json.tool resources/toolsconfig/nmap.json >/dev/null && python3 -m json.tool resources/toolsconfig/naabu.json >/dev/null` → exit 0。
  - `cd backend && cargo nextest run -p golish-pentest output_parser output_store --status-level fail` → 48 passed / 122 skipped。
  - `cd backend && cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail` → 5 passed / 174 skipped。
  - `cd backend && cargo fmt -p golish-pentest -p golish-pentest-app -- --check` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs backend/crates/golish-pentest/src/output_parser.rs backend/crates/golish-pentest/src/output_store/targets.rs resources/toolsconfig/nmap.json resources/toolsconfig/naabu.json docs/modules/backend/golish-pentest/output_store.md docs/modules/backend/golish-pentest-app/pentest_bridge.md agent-progress.md` → exit 0。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后用下一次默安 EAS run 确认 nmap 命令带 `-n`，并用只读 SQL 确认不会再新增 `compute.hwclouds-dns.com` / `IN-ADDR.ARPA` 这类 PTR domain target。

#### 2026-07-06 · EAS SERVICE-FINGERPRINT terminal weak-service closeout

- **本轮目标**：回应用户“最后一次为什么一直报错、怎么解决、是否确定”，用默安最新真实 run/DB 确认 EAS SERVICE-FINGERPRINT gap 的根因，并修复 `tcpwrapped` / DNS/53 造成的永久 pending。
- **已完成**：
  - 用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1783330408427-1 --full --db` 复核最新 run，gate 最后只剩 `222.186.57.10 × GOLISH-EAS-SERVICE-FINGERPRINT`。
  - 只读 SQL 查 DB：`222.186.57.10` 的 nmap fingerprint 已落库，端口 `5550` 为 `tcpwrapped`；ports 中还有 bare `53/domain`。旧 coverage truth 会把 `53` 与 `5550` 同时判为 blocking ports。
  - `coverage_truth.rs` 保持 `tcpwrapped/unknown/open/...` 不算强服务面，但允许同 target/port 的 `source=nmap` fingerprint 行作为 terminal attempt 关闭该 port；多端口主机上的 bare DNS/53 不再阻塞 SERVICE-FINGERPRINT，DNS-only 主机仍走 not_applicable/强表面判断。
  - 同步 `docs/modules/backend/golish-db/repo.md` 与 EAS methodology/spec 文案，避免后续 agent 继续按“所有 open port 一刀切”重扫。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-db` → exit 0。
  - `python3 -m json.tool resources/harness/stages/external_attack_surface/spec.json >/tmp/eas-spec-check.json` → exit 0。
  - `cd backend && cargo nextest run -p golish-db coverage_truth --status-level fail` → 34 passed / 172 skipped。
  - `cd backend && cargo fmt -p golish-db -- --check` → exit 0。
  - `git diff --check -- backend/crates/golish-db/src/repo/coverage_truth.rs docs/modules/backend/golish-db/repo.md resources/harness/stages/external_attack_surface/methodology.md resources/harness/stages/external_attack_surface/spec.json` → exit 0。
  - 当前默安 DB 复刻修后判断：旧逻辑 blocking ports = `['53', '5550']`；新逻辑 `new_blocking_required_ports = None`，说明 `222.186.57.10` 这个实际卡点会闭合。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit` / 重启 backend 后的完整默安 EAS live run 到 PASS；当前改动需要 app/backend reload 才会影响正在运行的进程。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后重新跑默安 EAS stage_run，确认 SERVICE gap 不再卡在 `222.186.57.10`；如果仍有 WEB-FINGERPRINT gap，再按 WhatWeb wrapper/target surface 落库单独查。

#### 2026-07-06 · EAS PORT-empty SERVICE not_applicable gate consistency follow-up

- **本轮目标**：回应用户“你再看看”，复核上一轮修复后的最新默安 live run/DB，确认当前剩余 SERVICE gap 的真实原因，并补齐 gate 路径一致性。
- **已完成**：
  - 复跑 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1783330408427-1 --full --db`：`222.186.57.10` 已不再是最新卡点；最新旧二进制 gate 最后停在 15 个 `GOLISH-EAS-SERVICE-FINGERPRINT` gap。
  - DB 只读核对：13 个 IP 有 `GOLISH-EAS-PORT empty via naabu`（evidence `18040`）且 `targets.ports=[]/ports_scanned_at=NULL`，说明无开放端口；前端 read-model 已会把 SERVICE 派生为 `not_applicable`，但 `org_gate` / submit preview / closeout gate 没有把这条依赖注入 gate context。
  - `org_gate` 新增 `eas_service_not_applicable_from_port_outcomes`：`GOLISH-EAS-PORT empty/not_applicable` → 同资产 `GOLISH-EAS-SERVICE-FINGERPRINT not_applicable`；同时继续过滤 target-level `SERVICE found`，SERVICE found 仍由端口级 DB truth 决定。
  - `harness_submit_tool` submit preview 与 `task_orchestrator/subtask_phases/execute.rs` closeout gate 复用同一个 helper，保证 sub-agent 提交预检、主 closeout、per-org org_gate 三条路径一致。
  - 端口级复核当前 DB：`115.28.135.55` 已闭合；`115.175.6.207` 还缺 `9998` 端口指纹，因为 `job_d0db4784` 只有 spawn 日志，没有 background completion / structured output / evidence 落库。下一次新二进制运行时，13 个 no-open IP 不应再卡 SERVICE；若还卡，只应剩真实未落库的 `115.175.6.207:9998`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app` → exit 0。
  - `cd backend && cargo nextest run -p golish-agent-kit eas_port_empty_outcome_makes_service_not_applicable eas_service_found_outcome_is_not_a_gate_fact hook_passes_through_unparseable_content no_harness_stage_skips_gate --status-level fail` → 4 passed / 834 skipped。
  - `cd backend && cargo nextest list -p golish-agent-app | rg 'submit|harness|stage' | head -80` → exit 0；编译 `golish-agent-app` 成功。
  - `cd backend && cargo nextest run -p golish-agent-app eas_port_checked_empty_makes_service_not_applicable eas_accepts_surface_claim_without_model_evidence_ids --status-level fail` → 2 passed / 164 skipped。
  - `cd backend && cargo fmt -p golish-agent-kit -p golish-agent-app -- --check` → exit 0。
  - `git diff --check -- <touched EAS gate files/docs>` → exit 0。
  - 当前进程检查：`ps ... rg 'nmap|naabu|whatweb|httpx|golish -- --stage-run'` 只返回检查命令自身，无遗留扫描进程。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit` / 重启后的完整默安 EAS live run 到 PASS。当前 Golish backend 进程是旧二进制，必须重启/重新编译后才会加载新 gate 逻辑。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后重新跑默安 EAS；预期 13 个 no-open IP 不再生成 SERVICE gap。若仍有 SERVICE gap，优先检查 `115.175.6.207:9998` 的 nmap completion/structured output 落库路径。

#### 2026-07-07 · dayu HTTPS origin JS visibility bugfix

- **本轮目标**：回应用户 `https://dayu.moresec.cn/` 在浏览器里有大量 JS，但 Target Surface 前端显示 JS/API/URL 为空的问题，区分站点行为与 Golish 平台采集/落库问题，并修复平台把 HTTPS 错误降级为 HTTP 的路径。
- **已完成**：
  - 只读复核 Test1 最新 transcript/DB：`https://dayu.moresec.cn:443` backend identity 存在，但该 origin 下内容聚合为 0；历史 `route_probe_paths` 结果落在 `http://dayu.moresec.cn:80`，且 `browser_collect_js_api` 对 dayu 写了 empty outcome。
  - 现场复现站点行为：`https://dayu.moresec.cn/` 无登录态也能加载 `/umi.3681c70d.js`、多个 `dayu-cdn.moresec.cn` async chunk，并观察到 `GET /api/iam/v2/login/types`；`http://dayu.moresec.cn/` 采集为 `scripts_seen=0` 且出现 403。结论：不是用户看错，也不是站点主动降级；是 Golish 平台 wrapper 的 canonicalize 把 HTTPS 错误改成 HTTP。
  - `target_resolver::best_web_service_candidate` 禁止将明确请求的 HTTPS URL 降级成 HTTP candidate；仍允许 HTTPS default origin 切到 HTTPS 非默认端口。
  - `scripts/browser_collect_js_api.mjs` 不再把 `text/html` / XHTML 响应的 `.js/.mjs` URL 当成 JS 保存，避免 SPA fallback chunk 污染 manifest。
  - `route_probe_paths` 对 `.js/.json/.map/.css/.env` 等静态/配置路径返回 HTML fallback 的候选直接按 soft-404 拒绝，避免 `config.js` / `openapi.json` 这类 SPA fallback 假阳性继续进入 `directory_entries`。
  - 同步 `docs/modules/backend/golish-pentest-app/pentest_bridge.md` 的工具契约说明。
- **运行过的验证（实跑）**：
  - `curl -k -L -I --max-time 20 https://dayu.moresec.cn/` → exit 0；HTTP 200，`content-type: text/html`，`content-length: 3609`。
  - `curl -k -L --max-time 20 --compressed -sS -D /tmp/dayu_headers.txt https://dayu.moresec.cn/ -o /tmp/dayu_body.html && rg '<script|src=|href=|umi' ...` → exit 0；HTML 中包含 `/static/cdn-load.js`、`/static/css-doodle.min.js`、`/umi.3681c70d.js`。
  - `curl -k -L -I --max-time 20 https://dayu.moresec.cn/umi.3681c70d.js` → exit 0；HTTP 200，`content-type: application/javascript`，`content-length: 3240791`。
  - `curl -k -L --max-time 20 --compressed -sS https://dayu.moresec.cn/config.js` 与首页 body SHA-256 相同；`https://dayu.moresec.cn/__golish_random_20260707__.js` 也与首页 body SHA-256 相同，证明该站对不存在静态路径返回 SPA fallback。
  - patch 前 `node scripts/browser_collect_js_api.mjs --url https://dayu.moresec.cn/ --workspace /tmp/dayu-golish-probe ...` → exit 0；能抓真 JS/API，但也把 `text/html` fake chunk 写进 JS manifest。
  - patch 前 `node scripts/browser_collect_js_api.mjs --url http://dayu.moresec.cn/ --workspace /tmp/dayu-golish-probe-http ...` → exit 0；`scripts_saved=0`、`api_requests_total=0`、console 403，复现错误降级后的空结果。
  - `cd backend && cargo fmt -p golish-pentest-app` → exit 0。
  - `cd backend && cargo test -p golish-pentest-app canonical_candidate --lib` → 7 passed。
  - `cd backend && cargo test -p golish-pentest-app static_js_path_returning_html_is_fallback_not_positive --lib` → 1 passed。
  - `cd backend && cargo test -p golish-pentest-app html_admin_route_can_still_be_positive_candidate --lib` → 1 passed。
  - patch 后 `node scripts/browser_collect_js_api.mjs --url https://dayu.moresec.cn/ --workspace /tmp/dayu-golish-probe-fixed ...` → exit 0；`status=ok`、`scripts_saved=12`、`script_manifest_entries=12`、`api_requests_total=1`、`html_scripts=0`，真实 JS/API 保留且 HTML fallback 不再进 scripts。
  - `cd backend && cargo test -p golish-pentest-app route_probe_tool_ --lib` → 2 passed，耗时约 92s。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/target_resolver.rs backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs scripts/browser_collect_js_api.mjs docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑 / 未完成**：按用户要求未再跑 `init` / 全量 `just precommit`。本轮开始时误触发的 `./init.sh` 已结束，失败在既有无关 clippy：`crates/golish-recon-app/src/asset_intel/agent_intel.rs` 的 `map_or` 可改 `is_none_or`，未在本修复里修改。未做历史 DB 清理；旧 `http://dayu.moresec.cn:80` 的假阳性行仍是存量数据，需要单独数据修复/重跑采集。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后对 Test1 重新跑 dayu 所在 Enumeration 或至少 `browser_collect_js_api`/`js_extract_apis`/`route_probe_paths` 对该 target 的修复路径；预期 `https://dayu.moresec.cn:443` 下能看到 JS/API，且 `config.js` / `openapi.json` 这类 HTML fallback 不再作为真实目录命中。

#### 2026-07-07 · Target Surface Sitemap port display / capture audit

- **本轮目标**：回应用户“sitemap 里面端口没有显示，或者爬到的返回包是不是没保存”，确认 Target Surface Sitemap 的端口展示与响应包保存链路。
- **已完成**：
  - 只读核对当前代码链路：Sitemap 由 `api_endpoints`、`js_analysis_results`、`directory_entries` 合成；`api_endpoints.capture_path` 可进入 HTTP request/response Inspector，JS 文件通过 `.golish/captures/<host>/<port>/js/...` 读取源码，`directory_entries` 当前无 `capture_path` 字段且前端明确置为 `null`。
  - 只读核对本机 DB：`directory_entries` 表没有抓包列；当前库里 `api_endpoints` 共 2297 条，只有 16 条带 `capture_path`，均来自 `source='crawler'`；dayu 相关数据里 `dayu.moresec.cn` 本身有 6 条 `route_probe` URL、0 条 response capture，dayu 相关 JS 文件只挂在 `sso/jira/sso-test` 等子域 target 上且带 JS 文件路径。
  - 前端 `buildSitemapTree` 的 origin root 改为显式端口显示：默认 HTTPS/HTTP 也展示为 `:443` / `:80`，非默认端口继续保留，例如 `:8443`。
  - 同步 `docs/modules/frontend/components.md` 记录 Sitemap 树根显式端口展示契约。
- **运行过的验证（实跑）**：
  - 只读 PostgreSQL 查询 `directory_entries` columns / `api_endpoints` capture counts / dayu rows → 确认目录 URL 目前无 response capture 存储字段，dayu route_probe 行无抓包。
  - `pnpm exec vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts` → 15 passed。
  - `pnpm exec biome check frontend/components/TargetPanel/surface/surfaceModel.ts frontend/components/TargetPanel/surface/surfaceModel.test.ts` → exit 0。
- **未跑 / 未完成**：按用户要求未跑 `init` / 全量 `just precommit`。没有给 `directory_entries` 增加抓包持久化；那需要后端 schema/采集链路设计，不能当成本次 UI 小修顺手改。
- **提交记录**：未 commit。
- **下一步建议**：如果希望 Sitemap 对 route_probe/目录 URL 也能看返回包，需要新增“目录探测响应 capture”链路：保存 bounded request/response JSON 到 `.golish/captures/<host>/<port>/...`，并给 `directory_entries` 增加可回读的 capture 引用或旁路关联表。

#### 2026-07-07 · oversized JS literal API prescan

- **本轮目标**：回应用户“这些 JS 抓不到 API 吗，里面没怎么有 API 数据”的疑问，修掉 UMI/Vite 大 bundle 因超过 `js_extract_apis.max_file_bytes` 被完整跳过后，明显 `/api` / `/iam` 字符串完全不落 API 表的问题。
- **已完成**：
  - `js_extract_apis` 对超过完整分析大小上限的 JS bundle 增加轻量 literal API path 预扫：默认最多读取 10MB 的 skipped bundle，只接受 `/api`、`/iam`、`/auth`、`/oauth`、`/graphql`、`/v1` 等 API-like 字符串，跳过静态资源后缀与普通页面路由。
  - 预扫命中的 same-origin 候选会进入工具响应 `summary.literal_prescan_endpoints_added`，写入 skipped JS row 的 `endpoints_found` / `raw_analysis.literal_prescan`，并按 `source='js_analysis'` 投影到 `api_endpoints`，从而让 Target Surface 的 APIs/JSAPI coverage 能看到低置信候选。
  - 保留原安全语义：大包仍不做完整 AST/regex/signal 分析，`raw_analysis.skipped=true` 与 `skipped_reason=exceeds_max_file_bytes` 继续存在；literal prescan 可用 `max_literal_prescan_file_bytes=0` 关闭。
  - Target Surface Sitemap 的 script detail 在 skipped bundle 有 literal prescan 命中时显示 `literal APIs N` 标记，区分“轻量 literal 候选”和完整 call-site 分析结果。
  - 同步模块卡：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-pentest-app` → exit 0。
  - `pnpm exec biome check frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx` → exit 0。
  - 真实 Test1 bundle 文件级复测（无 DB 写入）：用与 Rust 预扫同等的 API-path regex 扫 `/Users/christopherzheng/golish-platform/Test1/.golish/captures/dayu-test.moresec.cn/8443/js/b5a7bcdd_umi.6fb20ef4.js`、`sso-dayu.moresec.cn/443/js/60a89f37_umi.js`、`jira-dayu.moresec.cn/443/js/60a89f37_umi.js`、`sso-test-dayu.moresec.cn/8443/js/60a89f37_umi.js` → dayu-test 大包抽到 155 个候选；sso/jira/sso-test 大包各抽到 6 个候选（如 `POST /api/auth/login`、`GET /api/auth/verifyCode`、`POST /api/auth/resetPassword`），并且 `/v3`、`/sso` 这类短前端路由不再计入。
  - `cd backend && cargo test -p golish-pentest-app literal_prescan --lib` → 3 passed。
  - `cd backend && cargo test -p golish-pentest-app load_js_sources_bounded_skips_large_bundles --lib` → 1 passed。
  - `cd backend && cargo test -p golish-pentest-app js_extract_apis --lib` → 29 passed。
  - `pnpm exec vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts` → 15 passed。
  - `pnpm typecheck` → exit 0。
  - `git diff --check -- backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/frontend/components.md` → exit 0。
- **未跑 / 未完成**：按用户要求未跑 `./init.sh`；未跑全量 `just precommit`。没有对现有 Test1 历史 DB 做回填，旧的 skipped JS row / API 为空需要重启 backend 后重新跑 `js_extract_apis` 或单独做数据回填。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后对 `dayu-test` / `sso` / `jira` 等已保存大 bundle 的 target 重新跑 `js_extract_apis`；预期 skipped UMI bundle 仍标红 skipped，但 JS detail 会出现 `literal APIs N`，API tab 中也会看到 `/api/auth/...`、`/api/projects/...`、`/iam/...` 这类 same-origin 候选。

#### 2026-07-07 · JS extract HaE candidates + AI triage supersedes literal prescan

- **本轮目标**：按用户纠正，撤回 `literal_prescan` 分支：抓 JS 工具只负责保存 JS；`js_extract_apis` 才负责读取已保存 JS、用 HaE/Linkfinder 风格正则生成候选、再由 AI 判断候选是否提升为 API 事实。
- **已完成**：
  - `js_extract_apis` 删除 `max_literal_prescan_file_bytes` / `raw_analysis.literal_prescan` / `summary.literal_prescan_endpoints_added` 逻辑；默认不再用 1.5MB cap 跳过大 bundle，`max_file_bytes` 仅作为调用方显式 safety cap。
  - 复用现有 `resources/js-analysis/js-signal-rules.yml` 的 HAE-style `kind=route` 规则命中，整理为 `hae_route_candidates` 并写入 `js_analysis_results.raw_analysis`；候选本身不写 `api_endpoints`。
  - 新增工具内 `hae_route_triage` AI pass：只允许模型按候选 id 返回 `likely_api` / `rejected`；promoted 候选仍需过静态资源后缀过滤和 same-origin 投影后，才作为 `EndpointSource::Ai` 进入 `api_endpoints(source='js_analysis')`。确定性 fetch/axios/ajax/new Request 和 browser runtime observed API 逻辑保持原语义。
  - Target Surface script detail 将旧 `literal APIs N` 改为 `HAE candidates N`；`ToolAiTraceSummary` 在 Key Findings 展示 `HAE candidates N` / `HAE promoted N`，避免把候选误读成已落库 API。
  - 同步模块卡：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/frontend/components.md`。
- **运行过的验证（实跑）**：
  - `cargo fmt --manifest-path backend/Cargo.toml --package golish-pentest-app` → exit 0。
  - `./node_modules/.bin/biome check --write frontend/components/ToolAiTraceSummary.tsx frontend/components/ToolAiTraceSummary.test.ts frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx` → exit 0。
  - `cd backend && cargo test -p golish-pentest-app js_extract_apis --lib` → 29 passed / 157 filtered out。
  - `./node_modules/.bin/vitest run frontend/components/ToolAiTraceSummary.test.ts` → 8 passed。
- **未跑 / 未完成**：按用户要求未跑 `./init.sh` / 全量 `just precommit`。未对历史 DB 做回填；旧 literal-prescan 产物如果已写入本机数据，需要重启 backend 后重新跑 `js_extract_apis` 或单独数据清理。
- **提交记录**：未 commit。
- **下一步建议**：重启 app/backend 后对 dayu/sso/jira 等已保存 JS 的 target 重新跑 `js_extract_apis`；预期 JS row 出现 `hae_route_candidates`，API 表只增加确定性 call-site 或 AI-promoted 且 same-origin 的 endpoint。

#### 2026-07-07 · JS extract deterministic HaE API promotion

- **本轮目标**：按用户新判断，把 `js_extract_apis` 先改成机械式抽取：明显 API-shaped 的 HaE/Linkfinder 候选直接入库，AI 不再是默认 promotion 门。
- **已完成**：
  - `js_extract_apis` 默认 `ai=false`；只有显式传 `ai: true` 才跑 AI extract / HaE triage。
  - 新增 `EndpointSource::Hae` 与 `CallSiteKind::HaeRoute`；`/api`、`/iam`、`/auth`、`/oauth`、`/graphql`、`/sys/...`、`/.../v1/...` 等 API-shaped HaE route 机械提升为 endpoint，并走原有 `api_endpoints(source='js_analysis')` upsert 链路。
  - 增加 method-prefix 字符串扫描，覆盖 `"POST /api/auth/login"` / `"PUT /sys/v1/..."` 这类压缩 wrapper 常见形态；结果合并进 `hae_route_candidates`，再由 deterministic merge 去重。
  - 响应 / audit / raw analysis 增加 `hae_method_literal_candidates`、`hae_direct_promoted` 计数；工具输出新增 `hae_method_literal_candidates`、`hae_direct_promotions` 进度。
  - 同步 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：HaE direct promotion 是确定性抽取入口，AI 只是显式开启的增强。
- **运行过的验证（实跑）**：
  - `cd backend && cargo fmt -p golish-js-analyzer -p golish-pentest-app -- --check` → exit 0。
  - `cd backend && cargo test -p golish-pentest-app hae_route_candidates --lib` → 2 passed。
  - `cd backend && cargo test -p golish-pentest-app js_extract_apis --lib` → 32 passed。
  - `cd backend && cargo test -p golish-js-analyzer --lib` → 47 passed。
  - `cd backend && cargo test -p golish-pentest-app js_ai_extract --lib` → 9 passed。
  - `git diff --check -- backend/crates/golish-js-analyzer/src/lib.rs backend/crates/golish-js-analyzer/src/patterns.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_ai_extract.rs resources/js-analysis/js-signal-rules.yml docs/modules/backend/golish-pentest-app/pentest_bridge.md` → exit 0。
- **未跑 / 未完成**：按用户要求未跑 `./init.sh` / 全量 `just precommit`。未对历史 DB 做回填；需要重启 backend 后重新跑 `js_extract_apis` 才会把已保存 JS 里的 HaE direct endpoints 投影到 `api_endpoints`。
- **提交记录**：未 commit。
- **下一步建议**：用 dayu/yapi 已保存 JS 重新跑一次 `js_extract_apis` 实体检查；预期 yapi 的 `/api/auth/...`、dayu 的 `/sys/v1/...`、`/iam/...`、`/.../v1/...` 这类 API-shaped 候选会直接进 API 表，不再等待 AI。

#### 2026-07-08 · EAS service fingerprint headless regression

- **本轮目标**：按用户要求“不起 GUI，搞测试数据库，把已过前置阶段的数据/事实 seed 进去跑一次”，验证 EAS SERVICE-FINGERPRINT retry 修复是否能在 headless + ephemeral DB 下闭合。
- **已完成**：
  - 发现并修正前一版修复的不足：`eas_fingerprint_services` 不能只把所有 target 的 DB confirmed-open ports 做全局 union；否则 A 主机的 `9001` 仍可能扩散到 B 主机。现在 wrapper 按 target 的 `targets.ports[]` confirmed-open ports 约束扫描，并把相同端口集的 targets 分批执行 nmap；target 没有 DB confirmed ports 时才退回显式 `ports`。
  - 给 `stage_run` 增加 smoke-only seed 钩子 `GOLISH_STAGE_RUN_SEED_OPEN_PORTS='host=80,443;host2=9001'`，在 ephemeral DB seed target 后写入 `targets.ports[]`、`ports_scanned_at`、`liveness_checked_at`，用于复现“DB 已确认 open port 但 SERVICE-FINGERPRINT 未闭合”的 retry 类问题；默认不开启，不影响正常 GUI/CLI。
  - 同步模块卡：`docs/modules/backend/golish/stage_run.md` 与 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`。
  - 跑了一次真实 headless smoke：workspace `/private/tmp/golish-eas-headless-20260708-195607`，ephemeral DB，seed `222.186.129.58=82;222.186.57.10=9001`，profile `pentest`，only `external_attack_surface`，DeepSeek Flash。最终 deterministic gate PASS。
- **已记录证据 / 验证（实跑）**：
  - `CARGO_TARGET_DIR=/tmp/golish-stage-target cargo test -p golish-pentest-app eas_capabilities --lib` → exit 0；11 passed，覆盖 DB confirmed ports 优先、宽泛 range 不扩散、按 target confirmed ports 分批。
  - `CARGO_TARGET_DIR=/tmp/golish-stage-target cargo test -p golish stage_run --lib` → exit 0；26 passed，覆盖 stage-run seed parser。
  - `CARGO_TARGET_DIR=/tmp/golish-stage-target GOLISH_STAGE_RUN_SEED_OPEN_PORTS='222.186.129.58=82;222.186.57.10=9001' python3 scripts/stage_smoke.py --workspace /tmp/golish-eas-headless-20260708-195607 --profile pentest --only external_attack_surface --org 'EAS Headless Regression' --target 222.186.129.58 --target 222.186.57.10 --provider deepseek --model deepseek-v4-flash --objective '...' --run-tree` → exit 0。
  - headless report：`[PASS] external_attack_surface (findings=0)`；evidence booked `#4 port_probe`、`#11 nmap`、`#12 nmap`、`#15 http_probe`、`#19 http_probe`、`#22 http_probe`；DB summary `targets=2`、`technique_outcomes=8`、`tool_calls=8`、`org_stage_completions=1`。
  - run.log 关键分批证据：两个 nmap service batches 分开执行：
    - `222.186.57.10` batch ports `22,80,443,2222,5050,5550,5555,6666,8000,8001,8002,8080,8081,9001`
    - `222.186.129.58` batch ports `22,80,82,8083,9090,50002`
    - 关键点：`9001` 没有混进 `222.186.129.58` 那组，`82` 也独立留在对应 target 的组里。
  - `cd backend && cargo fmt --check` → exit 0。
  - 清理：`/tmp/golish-stage-target` 18G 临时编译目录已删除；保留 headless workspace/transcript 作为本轮证据。
- **未跑 / 未完成**：未跑 `./init.sh` / `just precommit` / 全量测试；当前工作树已有大量非本轮改动，未做 commit。headless smoke 中 prober 仍先跑了一次 `naabu` 扩展端口，说明 prompt/worker 策略仍倾向“重新确认端口”，但 service fingerprint 批次已不再发生跨 target 全局端口污染。
- **提交记录**：未 commit。
- **下一步建议**：重启实际 app/backend 后跑 Test1 真实 EAS 或 continuation；若再出现 SERVICE retry，优先看 `check_stage_asset_coverage.details.missing_open_ports` 与 run.log 中 nmap 批次是否按 target 分组。

#### 2026-07-10 · Enumeration Web Origin / terminal closeout P0

- **本轮目标**：复核当前信息收集闭环后，按用户“开始”指令修复三个 P0：Enumeration gate/worklist 对 error 的分裂、target-level 分母折叠多个 Web Origin，以及 partial 工具结果伪装成 found/empty 导致的假 PASS。
- **当前合同**：
  - 分母 = `scheme://host:port × GOLISH-ENUM-{JS,DIR,PARAM,JSAPI}`；默认端口显式化，同 host 的 HTTP/HTTPS/不同端口互不串格。
  - `found/empty` 只有当前 stage-run 的 exact-origin `technique_outcomes` 且引用真实 evidence id 才能闭格；`error/partial` 始终未完成。
  - `blocked/not_applicable` 只有带具体 note 且当前格不存在 error/partial marker 才可作为策略终态。
  - `directory_entries`、`api_endpoints`、`js_analysis_results` 保留原始发现，但不再独立关闭 Enumeration coverage。
- **已完成**：
  - 新增共享 `golish-pentest-domain::web_origin`，统一解析 HTTP(S) exact origin、显式默认端口、IPv6，并拒绝 credentials、无 host 与非 HTTP(S) URL。
  - 三条 gate 路径和覆盖读模型改成同一权威合同：Enumeration 固定四轴；缺 freshness cutoff 时 fail-closed；不回退同 session 历史 outcome；业务表/source-query 兼容事实不能闭格；partial/error marker 优先否决交付物自报终态；权威 `assets: []` 仍允许真实零分母 PASS。
  - `stage_worklist` / coverage 按 target URL 与 `ports[].url` 展开所有 exact origins；无法解析的 origin 保持 `missing_exact_web_origin`，不猜 80/443；repair 返回 pending/error/partial。
  - `browser_collect_js_api` capture 改为 `.golish/captures/{host}/{port}/{scheme}/{js|api}`；浏览器导航与显式 fetch 的每一跳都在发往越界 origin 前阻断；全导航失败/error、部分导航/队列/体积截断/越界为 partial；found/empty 无 evidence 时拒绝 upsert。
  - `js_extract_apis` 只读取 exact-scheme capture，不回退无法证明 origin 的 legacy 目录；read error/skipped 写 JSAPI/PARAM partial；`route_probe_paths` redirect 限 exact origin、最多 5 hop，队列未闭写 DIR partial；两者 terminal outcome 同样 evidence fail-closed。
  - Katana `-cs` 改为 anchored exact-origin union，约束实际调度的 scheme/host/port；输出过滤继续作为纵深防御。
  - `StageAssetCoveragePanel` 使用 `target_id + exact origin` 匹配 live work；同 host HTTP/HTTPS 不再同时误转圈；partial 显示“部分完成”，且 partial 数不再重复计入“未查”。
  - 新设计/计划与 Enumeration methodology/spec、模块卡、索引已同步；旧 2026-06-23 与 2026-07-03 设计已标明被本设计定向 supersede。`feature_list.json` 现只有本功能一个 `in_progress`。
- **运行过的验证**：
  - `CI=true ELECTRON_MIRROR=https://npmmirror.com/mirrors/electron/ pnpm install` → exit 0。
  - `./init.sh` → exit 0；启动基线的 fmt/check-fe/test-fe/lint-rust/test-rust-all/check-types 全绿。
  - `node --check scripts/browser_collect_js_api.mjs && node --test scripts/browser_collect_js_api.test.mjs` → exit 0，7/7 passed。
  - `cd backend && cargo nextest run -p golish-pentest-domain -p golish-pentest-app -p golish-agent-kit -p golish-agent-app --status-level fail` → exit 0，1285 passed / 3 skipped。
  - `cd backend && cargo clippy -p golish-pentest-domain -p golish-pentest-app -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings` → exit 0。
  - `pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx` → exit 0，26/26 passed。
  - `just check-fe` → exit 0。
  - `just precommit` → exit 0；fmt、check-fe、全前端测试、workspace Rust lint、workspace Rust tests、ts-rs drift 检查及重复 `just test` 全部通过；`test-rust-all` 阶段 279s。
  - `python3 -m json.tool resources/harness/stages/enumeration/spec.json`、`python3 -m json.tool feature_list.json`、`git diff --check` → exit 0。
- **已记录证据**：
  - Gate TDD：Enumeration focused tests 初始 RED(exit 101)，实现后 41 passed；`golish-agent-kit --lib` 856 passed；`golish-agent-app --lib` 184 passed。
  - Browser producer TDD：Node/Rust 初始 RED；真实 Playwright redirect 曾复现 foreign target 被请求，修复后 foreignHits=0；Node 7/7、Rust scoped 25/25 passed。
  - JS extract / route TDD：focused 初始 RED(exit 101)；新合同 6/6、相关组 56/56、`golish-pentest-app` full 190/190 passed（3 skipped）。
  - 统一回归与仓库门禁见上方实跑命令；没有用“工具被调用”替代 gate/DB 合同测试。
- **提交记录**：未 commit、未 stage、未 push。

- **已知风险或未解决问题**：
  - 尚未对 `/Users/christopherzheng/golish-platform/Test1` 发起 fresh Enumeration；该动作会对授权目标产生主动 HTTP/浏览器/route 请求，必须先得到用户明确确认。因此功能保持 `in_progress`，不标 `passing`。
  - 新 extractor 故意不读取无 scheme namespace 的 legacy capture；Test1 必须先用新 browser collector 重抓，才能形成可证明 exact-origin 的新鲜结果。
  - 当前工作树在本轮开始前已有大量跨模块未提交改动和若干删除项；本轮没有清理、恢复或提交它们，也没有修改 schema/migration 或主动改 `golish-db` crate。
- **以下本轮文件已修改但未提交**：
  - Domain/后端核心：`backend/crates/golish-pentest-domain/{Cargo.toml,src/lib.rs,src/web_origin.rs}`；`golish-pentest-app/src/pentest_bridge/{browser_collect_js_api,js_extract_apis,route_probe_paths,enumeration_capabilities,evidence}.rs`；`golish-agent-kit/src/harness/`、`task_orchestrator/subtask_phases/execute.rs`；`golish-agent-app/src/ai/{commands/stage_coverage.rs,db_bridge/evidence.rs,harness_submit_tool.rs}`。
  - 前端/脚本：`frontend/components/Engagement/StageAssetCoveragePanel.{tsx,test.tsx}`；`scripts/browser_collect_js_api.{mjs,test.mjs}`；`scripts/js_api_pipeline_test.mjs`。
  - 合同/状态：`resources/harness/stages/enumeration/{spec.json,methodology.md}`、`docs/design/2026-07-10-enumeration-origin-terminal-closeout.md`、`docs/superpowers/plans/2026-07-10-enumeration-origin-terminal-closeout.md`、相关 `docs/modules/` 卡片/索引、`feature_list.json`、本文件；两处启动基线 Clippy 与两个 runtime 测试期望也有最小修正。
- **下一步最佳动作**：用户明确授权主动请求后，重启当前 backend/app，在 Test1 跑一次 fresh Enumeration；随后执行 `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db`，逐 origin 核对四轴 outcome、正数 evidence id、partial/error 未闭格、worklist/UI 与 gate/pass-token 一致。只有这轮现场证据通过，才把 feature 切到 `passing`。

#### 2026-07-12 · Scoping session lifecycle / trusted-review contract

- **本轮目标**：修复 `pentest-chat-1783786690236-1` Scoping 在两次 deliverable accepted 后仍重复 `unit_review/scope_review` 的问题，并对齐 company-only 与 customer-provided trusted intake 合同。
- **已确认根因**：TaskMode durable session UUID 与 DbTracker 随机 UUID 分裂；gate 查询前者时看不到实际落在后者的 26 条 tool lifecycle。另有四个叠加合同缺口：empty trusted snapshot 被无条件拒绝；`customer_provided` 未进入 snapshot/Target Intel 白名单；tool-call repo 只取 latest `scope_review`，可用第二次确认洗掉第一次编辑；org-bound customer batch import 被 project-wide value 去重吞掉。
- **已完成实现**：
  - `DbTracker.session_uuid` 改为 clones 共享的同步 identity；TaskMode durable session upsert 后、构造 stage executor 前，通过 immutable `AgentBridge` 绑定正式 UUID。所有异步写先 snapshot getter，`ToolCallGuard` 固定 start UUID，finish 不会因后续 rebind 串到另一 session。
  - Scoping close gate 先加载 trusted target snapshot，再决定是否要求 target lifecycle：非空 snapshot 必须 exactly one persisted + parseable + canonical exact `scope_review`；repo/App 保留并聚合本轮全部 review attempts，第二次确认不能洗掉第一次编辑/拒绝。空 snapshot 是合法 organization-only 路径，不制造空 target review。红队 `unit_review` 仍独立按当前 operation lifecycle fail-closed 校验。
  - Scoping snapshot 与 Target Intel trusted roots 均加入 `customer_provided`；`asset_intel` / `discovered` / provider-derived source 继续不能升级为授权根。
  - `target_batch_add` 的 org-bound customer intake 改为 exact project + type + value + current-org/NULL identity：current org 优先，legacy NULL 用 CAS 认领并按需升级 provider source；sibling org 永不改绑、为当前 org 插独立行；写前校验 organization project ownership。
  - 已同步 prompt、stage spec/methodology、设计/计划及相关模块卡/索引；未改 schema/migration 或 generated IPC 类型，未执行任何 model-driven target mutation。
- **TDD RED 证据**：
  - `cargo nextest run -p golish-agent-kit organization_only_scope_does_not_require_an_empty_target_review --status-level fail` → exit 100，run `15a8ee6b-46c5-408d-8769-e0c05b4d10ee`，旧逻辑对空 snapshot 返回 blocker。
  - `cargo nextest run -p golish-agent-app task_mode_tracker_rebind_updates_existing_runtime_clones --status-level fail` → exit 100，run `7b8bdad9-b5fb-49db-816f-f0acb3d1e169`，旧 clone 保留随机 UUID。
  - `customer_provided` SQL/Target Intel 两条测试在加入 allowlist 前均 exit 100，随后转绿。
  - `cargo nextest run -p golish-agent-kit repeated_scope_review_cannot_replace_an_edited_response --status-level fail` → exit 100，run `a9c22e7c-59ae-46ab-a22f-494570e5c03d`，旧 gate 接受第二次 exact review 洗掉早先决定。
  - `cargo nextest run -p golish-recon-app customer_intake_dedupe_is_org_and_identity_scoped --status-level fail` → exit 101（缺少 identity-scoped claim seam），随后实现 current-org/NULL CAS CTE 并转绿。
- **GREEN / 静态验证证据**：
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(organization_only_scope_does_not_require_an_empty_target_review) | test(scope_review_must_exactly_match_trusted_seed_snapshot) | test(scope_review_skip_or_free_text_cannot_be_replaced_by_claim) | test(repeated_scope_review_cannot_replace_an_edited_response)' --status-level fail` → 4 passed，run `db0da6bb-576c-42da-8ad3-97d65c5dfdcf`。
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(stage_charter_scoping_appends_human_gate_when_policy_requires) | test(scoping_subtask_prompt_varies_by_policy)' --status-level fail` → 2 passed，run `171d8a8b-92ab-4509-a6c2-3efff9f57b72`。
  - `cd backend && cargo nextest run -p golish-db scoping_review_query_preserves_every_attempt_in_order --status-level fail` → 1 passed，run `ffcc0a58-0284-479e-9f54-c7bcad46322e`。
  - `cd backend && cargo nextest run -p golish-agent-app -E 'test(task_mode_tracker_rebind_updates_existing_runtime_clones) | test(scoping_snapshot_trusts_customer_intake_but_not_discovery_sources) | test(authoritative_type_queries_are_org_scoped)' --status-level fail` → 3 passed，run `948869b9-34c3-407b-ab50-74719b74e893`；`repeated_review_keeps_all_rows_but_is_never_approved` → 1 passed，run `32474470-e5ae-4c53-af4d-95a694d3ee7c`。
  - `cd backend && cargo nextest run -p golish-recon-app -E 'test(customer_intake_dedupe_is_org_and_identity_scoped) | test(sibling_project_value_does_not_swallow_org_bound_customer_intake) | test(provider_discovered_domains_never_become_authorization_roots_on_retry)' --status-level fail` → 3 passed，run `d89ed672-bd79-45bf-a305-e4647ff668d4`。
  - `cd backend && cargo check -p golish` → exit 0；`cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-bridge -p golish-agent-app -p golish-recon-app --all-targets -- -D warnings` → exit 0。
  - 相关 13 个 Rust 文件 `rustfmt --edition 2021 --check`、`feature_list.json` / Scoping spec JSON parse、相关 diff check 均 exit 0。`scripts/check_repo_ownership.py` 仍因当前 dirty tree 既有 170 ownership + 13 raw-SQL baseline violations失败；输出未新增 `targets/cmds.rs` blocker，本轮未顺手清理这些跨 scope 基线项。
- **当前状态 / 风险**：代码与聚焦验证已完成；进程核查显示 app/backend 当前已停止，仅 embedded Postgres 仍在，旧 waiting run 不会原地继续或自动获得修复。fresh no-GUI/live TaskMode Scoping 会调用当前配置的外部 LLM，未获额外授权未执行；同时按用户要求未跑 `./init.sh` 或全量 `just precommit`，因此 feature 保持 `in_progress`，不虚报 `passing`。
- **以下本轮文件已修改但未提交**：`backend/crates/golish-agent-kit/src/{db_traits/repo.rs,db_tracking/{mod.rs,recording.rs,memory/store.rs},task_orchestrator/{prompts/mod.rs,subtask_phases/execute.rs}}`、`backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`、`backend/crates/golish-agent-app/src/ai/{commands/core/chat.rs,db_bridge/{mod.rs,recon.rs}}`、`backend/crates/golish-db/src/repo/tool_calls.rs`、`backend/crates/golish-recon-app/src/{targets/cmds.rs,asset_intel/agent_intel.rs}`、Scoping spec/methodology、相关设计/计划/模块卡/索引、`feature_list.json` 与本文件。工作树此前已有大量 dirty 改动，本轮未恢复或清理。
- **下一步最佳动作**：重启 app/backend 载入新代码后，得到用户对真实外部 LLM 调用的明确授权，再用新 TaskMode session 跑一次 organization-only Scoping；随后用 `scripts/run_tree.py --full --db` 证明 `tool_calls.session_id == sessions.id(chat_session_key)`、一次 `unit_review` 后 gate PASS 且无整段重试，再补全仓库完成门禁后切 `passing`。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-12 · Scoping parent-only branch / scope-choice auto-confirm follow-up

- **本轮目标**：处理用户重开后的 `pentest-chat-1783791527002-1` 仍在 submit 后回到 Scoping 的问题；按用户要求不跑 `./init.sh`。
- **现场根因与 DB 真相**：本轮 17 条 `tool_calls` 全部写入 durable session `e103d0f8-989f-434f-a071-75fe8375cf6c`，证明 UUID 修复已加载。第一次 submit accepted、baseline outer gate PASS，随后 red-team post-check 因无条件要求 `ask_human(input_type="unit_review")` 翻成 BLOCK；但用户已经两次选择“不纳入子公司（仅母公司）”。retry 又做了 `propose_candidates(recorded=0)` 并停在冗余空 `unit_review`。日志：`/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1783791527002-1/{run.log,transcript.json}`。
- **已完成实现**：
  - red-team subsidiary gate 改为两分支：同一 trusted root 的 persisted parent-only choice 直接完成分支；只有纳入子公司才要求成功 `propose_candidates` 后接同 org、non-skipped、可解析的 `unit_review`。
  - `scoping_actions_for_session` 新增 trusted root 参数；structured choice 校验 `decision=subsidiary_scope + organization_id`。为已经运行中的旧 prompt 保留“必须精确出现当前 root 全名”的窄 fallback；另一 org、skipped/error、乱序 review 均不能通过。
  - prompt / synthesized subtask / methodology / spec 全部统一为上述分支；parent-only 不再 discovery、proposal 或制造空 review。
  - 前端识别 structured 与 legacy subsidiary-scope choice，关闭普通 60 秒倒计时自动首选项；该 scope boundary 必须等用户实际点击。
- **TDD RED 证据**：
  - `cd backend && cargo nextest run -p golish-agent-kit explicit_subsidiary_exclusion_needs_no_empty_unit_review --status-level fail` → 0 passed / 1 failed，run `68f06032-ba21-4968-b310-054868c9a840`；旧 helper 仍要求 unit review。
  - `pnpm exec vitest run frontend/components/AIChatPanel/AskHumanInline.test.tsx -t 'never auto-confirms a subsidiary scope decision'` → exit 1；fake timer 到期后实际调用 `onSubmit("不纳入子公司（仅母公司）")`，稳定复现非人工授权。
- **GREEN / 静态验证证据**：
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(explicit_subsidiary_exclusion_needs_no_empty_unit_review) | test(red_team_scoping_flow_blocks_when_steps_skipped) | test(scoping_subtask_prompt_varies_by_policy) | test(stage_charter_scoping_appends_human_gate_when_policy_requires)' --status-level fail` → 4 passed，run `db640bf5-7839-4a6a-adbe-ebfa7b882822`。
  - `cd backend && cargo nextest run -p golish-db -E 'test(persisted_parent_only_choice_is_a_deterministic_subsidiary_exclusion) | test(included_subsidiary_flow_requires_same_org_success_and_order) | test(scoping_review_query_preserves_every_attempt_in_order)' --status-level fail` → 3 passed，run `a061186d-b5ba-419e-bb6a-918d95a39d0e`。
  - `cd backend && cargo nextest run -p golish-agent-app -E 'test(repeated_review_keeps_all_rows_but_is_never_approved) | test(task_mode_tracker_rebind_updates_existing_runtime_clones)' --status-level fail` → 2 passed，run `bcf9d0e9-12b2-437a-90e9-f228263f3938`。
  - 最终合并复跑 `cd backend && cargo nextest run -p golish-db -p golish-agent-kit -E '<上述 7 个 DB/gate/prompt tests>' --status-level fail` → 7 passed，run `a38107b5-2fc8-4698-b267-3b8c43552e0d`。
  - `pnpm exec vitest run frontend/components/AIChatPanel/AskHumanInline.test.tsx` → 35/35 passed（structured + legacy subsidiary choice 均不自动确认）；两文件 Biome check exit 0。
  - `cd backend && cargo check -p golish`、`cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings`、`cargo fmt --all -- --check` → exit 0；`pnpm typecheck`、Scoping/feature JSON parse 与 `git diff --check` 也均 exit 0。
- **当前状态 / 风险**：app/backend 当前未运行，仅 embedded Postgres 在；因此旧 pending `unit_review` 不会原地热加载。代码与聚焦测试已修好，但尚无重启后 fresh live Scoping 到 PASS 的证据；feature 继续 `in_progress`。未跑 `./init.sh`、未跑全量 `just precommit`，未发起新的外部 LLM 请求。
- **下一步最佳动作**：用户重启 Golish 后重新发起或继续该 operation；预期 persisted parent-only choice 被同 root gate 接受，不再返回空 `unit_review`。随后只读跑 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 确认 post-check PASS 且没有 Scoping retry。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-12 · Target Intel organization-only submit / final-gate parity

- **本轮目标**：修复 Scoping 已 PASS 后，Target Intel 在覆盖预检 `ready_to_submit=true`
  的情况下，`submit_stage_deliverable` 仍反复 `needs_fix` 并回头重交的问题。按用户要求
  未跑 `./init.sh` / `just precommit`。
- **现场证据**：最新 run `pentest-chat-1783794304742-1` 的 org
  `08ee537b-658e-4909-b0b1-042f41b46e8f` 没有真实 `targets` 行；preflight 报 6/6 done、
  `coverage_to_submit=[]`，但 submit preview 没有合成 `organization:<uuid>` 轴，连续五次
  拒绝。final per-org gate 虽已合成该轴及 DNS/CT/SUBDOMAIN N/A，仍让空 target 的
  Quake SUBDOMAIN error 反向否决同一 deterministic N/A cell。
- **已完成实现**：
  - 新增共享 `TargetIntelOrganizationContext`，统一 canonical key、organization typed axis
    与 DNS/CT/SUBDOMAIN deterministic N/A；final org gate 与 submit preview 复用。
  - submit preview 在 DB truth 查询前注入 organization context，因此零真实 target 时仍
    会查询 organization WHOIS/ASN/OSINT truth，并保持 slim `coverage: []` 合同。
  - `coverage_complete` 只让精确可信 N/A 抑制同 cell 的 source-query-derived error；
    精确 evidence Error、另一资产与真实 applicable cell 的 source error 继续 BLOCK。
  - 已同步设计/计划、agent-app/agent-kit 模块卡、模块索引与 feature 状态。
- **TDD RED 证据**：
  - `cd backend && cargo nextest run -p golish-agent-app target_intel_organization_only_preview_matches_org_gate_context --status-level fail`
    → exit 100，run `5ee94f2b-87ff-42c2-8951-9d20562cb488`；旧 preview 的
    `in_scope_assets=None`，预期为 canonical organization row。
  - `cd backend && cargo test -p golish-agent-kit deterministic_context_not_applicable_is_not_vetoed_by_source_error --lib -- --nocapture`
    → exit 101，0 passed / 1 failed；旧规则仍把 organization SUBDOMAIN N/A 判成
    `coverage incomplete: never attempted`。
- **GREEN / 静态验证证据**：
  - `cd backend && cargo test -p golish-agent-app target_intel_organization_only_preview_matches_org_gate_context --lib -- --nocapture`
    → exit 0，1/1 passed；测试实际调用 slim `submit_stage_deliverable` 并断言 `accepted`。
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(deterministic_context_not_applicable_is_not_vetoed_by_source_error) | test(deterministic_not_applicable_for_another_asset_does_not_hide_source_error)' --status-level fail`
    → exit 0，2/2 passed，run `b210cb97-b82f-4cfc-9c13-a183db2e01c0`。
  - 既有 fail-closed 回归（DB N/A、partial/error veto、wildcard source join、exact-source
    error、organization identity/display alias）→ 6/6 passed，run
    `7f5a48b6-4af8-446d-8b12-7b77d30e85ad`。
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app --lib -- -D warnings`
    → exit 0，零 warning。
  - 首轮共享-helper/context 合并验证 → 3/3 passed，run
    `cd585179-acd9-49dd-b946-a7ece390eab4`。
  - 三个相关 Rust 文件 `rustfmt --edition 2021 --check`（格式化后）与 `git diff --check`
    通过；`python3 -m json.tool feature_list.json` exit 0。
- **当前状态 / 风险**：代码级回归与 scoped Clippy 已转绿；用户启动的新 backend 已编译并
  载入修复。旧 run 不会原地重放已经失败的模型 turn；需要 fresh Target Intel run 才能给出
  live PASS 证据。因此 feature 保持 `in_progress`，不虚报 passing。
- **提交记录**：未 commit、未 stage、未 push。

#### 2026-07-12 · Asset Map current-run direct Target landing / candidate workspace removal

- **本轮目标**：按用户确认的产品合同，把 `recon_map_assets` 本轮从所有已配置 provider（0.zone / Quake / FOFA 等）获得的域名/IP 统一规范化、去重并直接落成组织绑定的 Targets；移除 TargetPanel 的候选人工审核入口。按要求未跑 `./init.sh`，也未发起真实 provider 请求。
- **现场根因与 DB 真相**：最新 run `pentest-chat-1783796952452-1`、org `a5551335-3cc5-4297-ac39-bb3df8e593d5` 已拿到业务观察：`subdomain_hosts=302`、`pairs=56`、`services=100`，但对应落地计数分别为 `subdomains=0`、`promoted=0`、`service_assets=0`；DB 同时是 `targets=0`、`target_assets=0`、`dns_records=0`。组织画像已有 domains/asns/certificates/intel，说明 Quake/provider 调用不是主因。真正断点是 company-only Scoping 没有 trusted target root，旧 landing 又只允许“已授权根的后代”晋升，于是 provider 结果被过滤；摘要里的 `targets=45` 实际还是候选观察数，不是 durable Target 数。
- **已完成实现**：
  - `recon_map_assets` 只消费本次调用的规范化结果：domain/IP/URL host canonicalize，拒绝 wildcard/malformed/CIDR，按 exact identity 稳定去重；每个 host-IP pair 都产生独立 IP Target 与 DNS row，domain 的 `real_ip` 按确定性 IPv4-first 选择。
  - 复用现有 exact org/project/type/value upsert；同 org/legacy NULL identity 可认领，sibling org 不改绑，既有 `scope=out` 不会被重新激活。新行写 `scope=in, source=asset_intel`，随后再落 DNS、service、subdomain 关系。
  - provider domain expansion 仍只读 trusted pre-stage roots，`source=asset_intel` 不会递归扩大查询面；WHOIS 可对本轮新域名做一次有界组织级补全。active scan 仍需 `human_approval.required_before=active_scan`。
  - Target Intel 的 asset denominator 冻结在 `stage_started_at`；本轮新 Target 是交给 EAS 的 handoff，不会把当前 stage 的 coverage 分母越跑越大并重新触发 submit 循环。coverage read model、subtask gate、submit preview 与 final org gate 使用同一 cutoff。
  - Enrich 的 durable target-candidate queue 改为 opt-in 且运行时关闭；删除 TargetPanel candidate tab、candidate review/promote handlers、新建 engagement 的 candidate checkbox 和相关文案/API wrapper。为兼容旧数据与 subsidiaries `unit_review`，保留 DTO、JSON 字段、backend candidate commands/schema；历史候选 JSON 未删除、未迁移。
- **TDD RED → GREEN 证据**：
  - current-run target planner、summary landed/observed 分离、phase candidate queue、candidate persistence 默认值、exact SQL write contract、WHOIS input split、Target Intel axis cutoff、coverage read model 都先以缺 helper/错误计数稳定 RED，再逐项转绿；对应 runs：`73699e38-c4d1-4cee-a936-4247a24f117a`、`26573e88-eee9-41a9-8e0c-63f681ccb74e`、`d7f5a5c5-eb9e-4f3d-9012-d483a0231d40`、`481b67c9-5751-4337-bf23-5a4d6b8405e4`、`ac376a3b-53da-467a-a6f1-2cf26095e390`、`db5b9710-0d82-4c31-819f-4d2b3ac8602a`、`f21cb176-cf6f-43ec-a954-942a2b77651c`、`0aaa4b5f-b4ed-4b7a-bee0-15ea72676632`。
  - 前端 RED 证明旧 action 仍路由到 `candidates`；删除路径后 focused suite 转为 85/85 passed。
- **最终局部验证**：
  - `cd backend && cargo nextest run -p golish-recon-app asset_intel` → 116/116 passed，run `218c11ed-b6be-47fa-9946-0288289522ba`；`organization_recon` → 71/71 passed，run `57f3c3fd-d34b-46f5-a913-63b0b9cf8358`。
  - `golish-agent-kit` cutoff/N/A focused → 3/3 passed，run `274f9652-6abb-4713-ae73-30980e362c18`；`golish-agent-app target_intel` → 9/9 passed，run `e1e96da4-24fb-4cc3-a833-95e2b2900287`。
  - TargetPanel/AskHuman focused Vitest → 85/85；`pnpm typecheck`、相关 64 files Biome → exit 0。
  - `cargo check -p golish-recon-app`、`cargo clippy -p golish-recon-app -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings`、`cargo fmt --all -- --check`、两个 JSON parse、`git diff --check` → exit 0。
- **当前状态 / 风险**：没有 schema/migration/generated IPC 改动，也没有清理历史候选 JSON。尚未在重启后的 app/backend 用真实配置跑 fresh Target Intel，因此还不能声称新 run 已实际写入 Targets/DNS；feature 继续 `in_progress`。未跑 `./init.sh` / 全量 `just precommit`，未 commit、stage 或 push。
- **下一步最佳动作**：重启 app/backend 后 fresh 跑一次 Target Intel，再用 `python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --full --db` 核对 `observedTargets`、`targets/landedDomains/landedIps/dnsRecords` 与 DB `targets/target_assets/dns_records` 一致，并确认 submit 不因本轮新 Targets 回头。
- **提交记录**：未 commit、未 stage、未 push。
### 2026-07-13 · Cargo build-space guard / incremental growth containment

- **本轮目标**：解决 AI 频繁运行 Cargo check/nextest 时 `backend/target` 无上限增长并写满系统盘的问题，同时避免日常 `cargo clean` 导致全冷启动。
- **诊断证据**：开始时 Data volume 仅余约 461 MiB，`backend/target` 约 193GB；首次 `cargo sweep --maxsize 40GB backend` 回收报告 129.05 GiB。进一步分解确认剩余约 129GB 中 `debug/incremental` 占 119GB，而可复用 `debug/deps` 仅 6.7GB，是增量快照而非依赖缓存主导膨胀。
- **已完成**：dev profile 禁用 incremental，并把 codegen-units 从 256 收敛到 64；新增 `scripts/cargo_space_guard.sh`，只在 Data volume 低于 80GB 且 cargo/rustc/nextest/cargo-sweep 全部空闲时运行，先 sweep、仍不足再清理已禁用的 stale dev incremental；新增 `just space-guard` 与 macOS LaunchAgent 安装/卸载入口；`AGENTS.md` 要求 AI 在 Rust 构建前先运行守卫。
- **本机安装状态**：`com.golish.cargo-space-guard` 已由 launchd 加载，每 600 秒运行；阈值 80GB、Cargo artifact cap 30GB，日志在 `~/.golish/cargo-space-guard.log`。
- **结果证据**：现有 incremental 快照清理后 Data volume 可用空间约 183GB，`backend/target` 约 10GB，`debug/deps` 仍保留约 6.7GB，`debug/incremental` 为 0B；未删除源码、数据库、transcript 或依赖缓存。
- **验证**：`bash -n scripts/cargo_space_guard.sh`、`just --list`、`git diff --check -- AGENTS.md backend/Cargo.toml justfile scripts/cargo_space_guard.sh` 均 exit 0；launchctl 成功加载并显示 `StartInterval=600`。本轮刻意未跑 `init.sh` / full `just precommit`，避免为磁盘治理立即触发全 workspace 重建；功能状态不改为 passing，也未 commit/push。

### 2026-07-13 · C8 Cleanup exact truth / deletion / waiver security hardening

- **本轮目标**：按只读安全审查结论，以 TDD 修复 C8 Cleanup 的全部 P1/P2：terminal status 伪造、删除 subtree 竞态、artifact root/symlink、sitemap 丢更新、waiver/delete IDOR、失败 job hot-loop/饥饿，以及 waiver 单击直提/跨 obligation draft 漂移。
- **已完成**：
  - `20260712000010_cleanup_closeout.sql` 增加 exact relational terminal-truth 函数与 deferred constraint triggers；`verified_absent` 要求 exact terminal Attempt、independent absence worker/evidence，waived/blocked 要求 retained local decision + residual/evidence。Gate 与删除 precheck 增加 `invalid_terminal_truth_count`，所有 Cleanup evidence/decision history 禁止 update/delete。
  - 删除 request 只从 active server-owned `project_scopes.canonical_project_path` 冻结 project root；锁定初始 subtree 后再次递归读取并 exact compare，active deletion subtree 禁止外部 organization reparent。Command 需要当前 workspace witness，foreign active project 与未注册 path 都拒绝且不创建 job。
  - Artifact cleaner 拒绝空/相对/不存在/非 canonical root，校验 `.golish/captures|analysis` 与 host directory canonical containment，拒绝 symlink escape 后才删除；target snapshot 的 path 不再参与 root 选择。Sitemap prune 改为 project-scoped JSONB compare-and-swap 重试，避免 delete+upsert 覆盖并发新增数据。
  - Artifact cleanup 与 hard-delete 各自保存 durable retry-not-before；claim/ready 查询跳过 backoff job并保持 requested-time 公平顺序，失败的最老 job 不再 hot-loop 饿死其它 ready job。
  - Waiver DTO/repo/domain 全链路绑定 exact operation/project scope/scope snapshot/organization/obligation + row-version CAS，actor 仍只由 local C0 provider 解析。前端按 obligation 保存独立 draft，首次 Review 冻结 exact scope/CAS/residual/evidence，第二次 Confirm 才调用 IPC；Review 后编辑不会改写 frozen payload，refresh/identity/CAS drift 会取消确认。
  - ts-rs 重新生成 `CleanupWaiverSubmitRequest` 与 `CleanupCloseoutGateView`；同步 Cleanup/DB/Recon/Agent/Frontend 模块卡与 `docs/modules/INDEX.md`，feature 保持唯一 `in_progress`。
- **TDD RED 证据**：
  - exact terminal/Gate/delete、subtree reparent/锁后扩张四测试先失败：旧 deferred boundary 接受伪造 `verified_absent`，Gate/删除接受 legacy invalid terminal，外部 org 可移入 active subtree，request 的锁前 snapshot 会漏掉并发新 child。
  - artifact root/symlink/CAS 三测试先失败：相对 root 静默返回 0、symlink 会触达 project 外数据、sitemap 无 atomic CAS API；canonical plan 测试证明 caller/target-owned `/tmp/...` 可进入旧 plan。
  - backoff 两测试 run `75fbfab3-5cd6-4c48-bee9-339459303e74` exit 100：artifact 与 hard-delete 失败后都会立即重选最老 job并饿死 newer ready job。
  - `CleanupObligationList` focused Vitest 初始 exit 1：旧 UI 不展示 invalid terminal truth，也没有 Review/Confirm seam而是直接 Submit。
- **运行过的验证 / 已记录证据**：
  - 每条 Cargo 命令前均运行 `just space-guard`。`cargo check -p golish-db --all-targets` → exit 0。
  - `cargo nextest run -p golish-db --test cleanup_obligation_kernel -E '<10 个 C8 exact-truth/IDOR/race/CAS/backoff tests>' --no-fail-fast --status-level fail` → 10/10 passed，run `28a81211-6c5a-4a0a-a0ff-92020953b9d9`。
  - 单项中间证据：waiver exact-scope 1/1、canonical artifact plan+sitemap CAS 2/2、artifact root/symlink 3/3、backoff fairness 2/2 均 GREEN。
  - `just gen-types` → exit 0；生成的 waiver request 含 exact 4-scope 字段，Gate view 含 `invalidTerminalTruthCount`。`just check-types` 的 export-binding tests 全绿，但最终 `git diff --exit-code frontend/lib/generated` exit 1，仅因为共享 index 中两份 staged 旧 Cleanup binding 与本轮预期再生成内容不同；未擅自改动共享 staging。
  - `just check-fe` → exit 0；`pnpm exec vitest run frontend/components/Engagement/CleanupObligationList.test.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts` → 31/31 passed。
  - Candidate 释放共享 Cargo 后，`cargo clippy -p golish-db -p golish-cleanup-domain -p golish-cleanup-app -p golish-recon-app -p golish-pentest-app -p golish-agent-app -p golish --all-targets -- -D warnings` → exit 0，零 warning。
  - 最新共享树 final focused：`cargo nextest run -p golish-db --test cleanup_obligation_kernel --no-fail-fast --status-level fail` → 17/17 passed，run `60038d7d-c61e-486b-b8f7-4daef862b41b`；`cargo nextest run -p golish-cleanup-domain -p golish-cleanup-app -p golish-recon-app -p golish-pentest-app -p golish-agent-app -p golish-agent-runtime -p golish -E 'test(cleanup)' --no-tests=pass --no-fail-fast --status-level fail` → 16/16 passed，run `f7344275-6086-43a3-a623-0bfc639125d9`。
  - 最终前端 focused：`pnpm exec vitest run frontend/components/Engagement/CleanupObligationList.test.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts` → 43/43 passed。
  - C8 相关前端 5 文件 Biome check、scoped `git diff --check`、`feature_list.json` / Cleanup spec JSON parse均 exit 0。冻结的 `20260712000001_runtime_memory_foundation.sql` SHA-384 仍为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`；reserved `00005` / `00012` 文件仍不存在且本轮未创建/修改。
- **提交记录**：未 commit、未 stage、未 push；未执行真实 filesystem cleanup、组织删除、外部 API/扫描/LLM 请求。
- **已知风险或未解决问题**：C8 scoped all-targets Clippy 与 final focused suites 已在最新共享树通过；全量 `just precommit` 尚未由本子任务重复执行。`check-types` 的最终 diff guard 仍需 root 在汇总 shared generated changes 后统一 restage/重跑。功能继续 `in_progress`，不虚报 passing。
- **以下文件已修改但未提交**：C8 migration/repo/tests、`golish-cleanup-{domain,app}` exact-scope ports、Recon artifact/deletion command、Pentest Cleanup bridge、Agent Cleanup command/Gate propagation、Target delete API/UI、Cleanup waiver UI/generated bindings，以及上列模块卡、`docs/modules/INDEX.md`、`feature_list.json`、本文件。共享文件还包含 Candidate/Reporting 等并行改动，本轮未回滚或拆走。
- **下一步最佳动作**：root 汇总并 restage generated bindings 后重跑 `just check-types`，再由主线程执行共享树 full `just precommit`。只有这些共享门禁与 feature verification 全部新鲜通过后，才可切 `passing`。

### 2026-07-13 · C9 EvidenceAudit authority / finalization and immutable-history hardening

- **本轮目标**：按 C9 安全审查结论，以 TDD 修复两个 P0：把所有被 canonical fact 引用的 `audit_log` evidence 作为完整 Reporting source authority；在 publish 前同事务重验 source/citation/Cleanup/attestation；补齐 final/superseded child 与 referenced blob 的 DB immutability。未创建新 migration，所有 SQL 修改均留在尚未 cutover 的 `20260712000011_reporting_read_model.sql`；未碰 reserved `00005` / `00012`、Candidate producer 或 Cleanup producer。
- **已完成实现**：
  - complete RR source query 现在枚举所有 canonical evidence membership，把 audit row 的完整 canonical JSON body 作为 `ReportSourceKind::EvidenceAudit`（int64 id、version 0、SHA-256）纳入 source snapshot；typed `EvidenceAuditTruth` 同时核对 exact operation run、`audit_role=evidence`、唯一 canonical source org、frozen allowed org 与 manifest source/hash。missing/foreign/sibling evidence 在 `persist_validated_revision` 前 fail closed，不留下 report/current validated revision。
  - `PgReportPublicationPort` 在一个 `REPEATABLE READ` transaction 内锁定 report/current revision、manifest、sections、claims、citations 与 cited audit rows，重跑 complete source query，并重验 stored claim hashes、citation evidence↔manifest binding、scope ownership、validation-result count attestation 与 `validated_at`；随后通过 Cleanup-owned `cleanup_closeout_gate_on` 在同一 snapshot 重验每个 frozen org 的 missing/nonterminal/undisclosed/invalid truth 与 residual disclosure，全部通过才 attach artifact/outbox 并 CAS final。
  - `20260712000011` 的 manifest/section/claim/citation/revision-artifact trigger 改为 `BEFORE INSERT OR UPDATE OR DELETE`；UPDATE 同时检查 OLD/NEW revision owner，且以 parent `FOR KEY SHARE` 和 finalizer `FOR UPDATE` 串行化 publication/child mutation，阻止把 final child 搬到 draft 或并发尾插绕过。被引用 blob 拒绝任何 UPDATE（含 no-op）；repo blob reuse 改为 `ON CONFLICT DO NOTHING` 后 SELECT + exact identity compare，仍允许两个 revision 共用一个 immutable blob。
  - 同步 `golish-reporting-domain`、`golish-reporting-app`、`golish-agent-app/ai`、`golish-db`、`golish-db/repo` 模块卡与 `docs/modules/INDEX.md`；父 feature 仍保持唯一 `in_progress`，未虚报 passing。
- **TDD RED 证据**：
  - `cargo nextest run -p golish-db --test reporting_read_model_migrations finalized_content_is_immutable_and_blob_is_shareable_across_revisions --status-level fail` → exit 100，final revision 的 manifest INSERT 实际成功；run `bffaa48e-b894-4e74-b0d3-68219c620095`。
  - `cargo nextest run -p golish-agent-app --test reporting_authority -E '<3 个 EvidenceAudit/finalize P0 tests>' --no-fail-fast --status-level fail` → 3/3 按预期失败：sibling-org evidence 仍持久化成功、audit body/org 漂移不改变 source hash、tampered citation 可 finalize；run `f6cdb242-0668-4278-b1a8-8bb6ebf85df1`。
- **GREEN / 已记录证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - 最终 `cargo nextest run -p golish-agent-app --test reporting_authority --no-fail-fast --status-level fail` → 5/5 passed，run `56f46a8d-d3cf-4c4c-9f16-4a5e97a53419`；覆盖 build 前 sibling-org、foreign run/non-evidence role rejection、EvidenceAudit body/org stale、citation/attestation finalize revalidation 与新增 canonical source stale。
  - 最终 `cargo nextest run -p golish-db --test reporting_read_model_migrations --no-fail-fast --status-level fail` → 3/3 passed，run `b720b59c-4d0a-4b8e-b342-e86d08792e37`；其中 immutable/blob 收紧中间单项为 1/1 passed，run `8a070319-d385-4aa2-a53f-5e94d71fd163`。
  - `cargo nextest run -p golish-reporting-domain -p golish-reporting-app --no-fail-fast --status-level fail` → 8/8 passed，run `8c445b4d-5d6e-4eee-9052-c8c9f0cb7f0c`。
  - `cargo clippy -p golish-reporting-domain -p golish-reporting-app -p golish-db -p golish-agent-app --all-targets -- -D warnings`、`cargo fmt --all -- --check` → exit 0，零 warning/格式差异；scoped `git diff --check` → exit 0。
- **当前状态 / 风险**：C9 两个 P0 的 scoped tests 与 all-targets Clippy 已在最新共享树全绿；全量 `just precommit` 尚未由本子任务执行，因此父 feature 继续 `in_progress`。未 commit、未 stage、未 push，也未执行外部请求或真实 final publish。
- **以下文件已修改但未提交**：Reporting domain/agent bridge、`golish-db` 00011/repo、两个 PG regression tests，以及上述模块卡、索引与本 progress 记录；共享树还包含 Candidate/Cleanup/artifact 等并行修改，本轮未回滚或覆盖。

#### 2026-07-13 · C9 P1 single-snapshot / blocked-decision / sealed-ref authority

- **本轮目标**：继续收口 C9 P1 #3/#4/#5：build/Gate/finalize 不能混读 PostgreSQL snapshot；blocked residual 必须来自 retained operator decision；TechniqueOutcome 必须与 final-sealed canonical ref 形成严格双向一一对应。未碰 commands/frontend/artifact、Candidate/Cleanup terminal producer，未新建 migration，SQL 只追加到尚未 cutover 的 `20260712000011_reporting_read_model.sql`。
- **已完成实现**：
  - `PgReportTruthPort::build_repeatable_read_snapshot` 现在在同一个 `REPEATABLE READ READ ONLY` transaction 内完成 complete source、claim seeds、typed EvidenceAudit、typed Cleanup blocked decision、frozen scope 与 Cleanup-owned closeout truth；不再 commit 后另开连接读取 Cleanup。
  - Reporting Gate 的 stored bundle（report/revision/sections/claims/citations/manifest/artifacts）、current complete source、integrity counts、frozen organizations 与 Cleanup closeout 全部在一个 RR transaction 中读取。真实 two-connection barrier 覆盖 state B=`valid attestation + stale source` 原快照与 state A=`exact source + invalid attestation` 新快照，证明 Gate 不能把两个都失败的状态拼成从未存在的 PASS。finalize 保持 P0 已有的单 RR 写事务、row locks 与同事务 source/citation/attestation/Cleanup 重验。
  - 新增 `CleanupBlockedDecisionTruth` 与 `ReportSourceKind::CleanupBlockedDecision`：source hash 冻结 decision row + 完整 evidence membership；claim 精确投影 `decidedByPrincipalId/reason/residualRisk`，citations 必须等于该 decision relation 的 evidence ids 且至少有 `role=decision`，绝不使用 obligation-creation evidence。
  - TechniqueOutcome authority 改为 ref-driven/keyed bijection：每个 final-sealed ref 必须有 exact row，每个 operation-bound row 必须有唯一 ref；duplicate key（即使内容相同）、missing row、extra/unsealed row、content SHA/evidence drift、foreign/unresolvable evidence全部 fail closed。承载 ref 的 `StageHandoff` 自身进入 manifest；00011 新 trigger 拒绝已被 validated/invalid/final/superseded Reporting history 保留的 handoff invalidation/delete（`REPORT_SEALED_REF_RETAINED`）。
  - 同步 `golish-reporting-domain`、`golish-reporting-app`、`golish-agent-app/ai`、`golish-db`、`golish-db/repo` 模块卡与 `docs/modules/INDEX.md`；`feature_list.json` 的父功能仍是唯一 `in_progress`。
- **TDD RED 证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - `cargo nextest run -p golish-agent-app --test reporting_authority --status-level fail` → exit 100，旧实现静默接受 sealed ref missing row，且 manifest 不含 StageHandoff；run `308e894c-c0a2-4a0f-ab4d-a3370e5a1798`。
  - 修正测试 fixture 的 frozen project-path 后，blocked residual focused test 仍按预期失败：source snapshot 不含 `cleanup_blocked_decision`；run `f066072a-4ec4-4cba-87d7-9da1038f150a`。
  - two-connection Gate barrier 在分裂快照实现上按预期失败：`validate_reporting_gate_truth` 实际返回 PASS，证明旧实现合成了数据库中从未存在的状态；run `fc0e05d3-f9a9-455a-b5fc-bee723530d47`。
- **GREEN / 已记录证据**：
  - 中间 #4/#5 `cargo nextest run -p golish-agent-app --test reporting_authority --status-level fail` → 8/8 passed，run `badb2f03-bf58-47c3-8545-c276c5b1e0fb`。
  - 最终同命令（含 single-RR barrier）→ 9/9 passed，run `8a731b21-6996-4e5e-b671-ba8da38fcb32`。
  - `cargo nextest run -p golish-db --test reporting_read_model_migrations --status-level fail` → 3/3 passed，run `abbd7d11-3125-481f-a7f7-f5d06eaff14b`。
  - `cargo nextest run -p golish-reporting-domain -p golish-reporting-app --status-level fail` → 8/8 passed，run `54acd271-cb35-4841-a8f2-46c76096dbe5`。
  - `cargo clippy -p golish-reporting-domain -p golish-reporting-app -p golish-db -p golish-agent-app --all-targets -- -D warnings` → exit 0，零 warning；相关 Rust 文件已用 `rustfmt --edition 2021` 格式化。
- **当前状态 / 风险**：C9 P1 #3/#4/#5 的 scoped PG/domain/app tests 与 all-targets Clippy 已绿；尚未由本子任务执行共享树全量 `just precommit`，因此父 feature 不切 `passing`。没有 commit/stage/push，也没有外部 API、真实扫描、真实 publish 或 Cleanup mutation。
- **以下文件已修改但未提交**：`golish-reporting-domain` source/validation contracts、Agent Reporting/Gate bridge 与 PG authority tests、`golish-db` 00011/source parser、上述模块卡/索引与本 progress；共享工作树的其他 Candidate/Cleanup/artifact 改动均未回滚或覆盖。

#### 2026-07-13 · C9 review closeout: terminal source freeze / validated history / IPC authority fence

- **本轮目标**：继续处理 C9 复审新增 P0/P1：终态 CandidateAttempt/CleanupObligation 不得因 no-op UPDATE 改变 deterministic event source version；validated+unpublished Reporting children 必须已经冻结；final→superseded 不得夹带历史字段修改；Reporting IPC 的 autocommit authorization 必须在 build persistence / publish transaction 内再次锁定并核对 project identity。未新建 migration，仍只修改未 cutover 的 `20260712000011_reporting_read_model.sql`。
- **已完成实现**：
  - 00011 新增统一 terminal canonical-source guard：`candidate_attempts` 的 verified/refuted/blocked/retryable_failed/abandoned 与 `cleanup_obligations` 的 verified_absent/blocked/waived_by_user 只允许首次 nonterminal→terminal；之后 UPDATE（含 no-op）/DELETE 统一拒绝 `TERMINAL_CANONICAL_SOURCE_IMMUTABLE`。两条 exact response-loss replay 回归同时证明原 Finding/lineage 或 Cleanup terminal event/deliveries 保持单份。
  - manifest/section/claim/citation child guard 从 publication terminal 前移到 validation terminal（`validated|invalid`）并对 INSERT/UPDATE/DELETE 全覆盖；artifact child 仍允许在 validated+unpublished finalizer transaction attach，final/superseded 后冻结。三个深层 finalizer corruption regression 先证明普通 SQL 已被 DB guard 阻止，再用事务内临时 disable 单一 guard 的显式 legacy fixture 注入历史损坏，确认 finalizer 仍独立 fail closed。
  - final→superseded transition 改为比较整行 `to_jsonb`，只排除 `publication_status` 与 trigger-owned `row_version`；纯 supersede 版本只加一，任何 combined validation_result/identity/timestamp 等修改都拒绝。
  - `authorize_reporting_scope` 额外冻结 active project canonical path、path SHA 与 row version，组成 server-only `ReportingProjectAuthority`（连同 project/snapshot id/hash）。`PgReportTruthPort` 在 build RR snapshot/current check 复核，并在 `persist_validated_revision` 写事务 `FOR SHARE` 锁定 exact operation/project/sealed snapshot；`PgReportPublicationPort` 在同一 finalization RR transaction 内执行同样锁与 exact compare。授权后的 rename/rebind/retire 只能得到 stale error，不能落 validated revision、artifact ref 或 final publication。
  - 同步 `golish-agent-app/ai`、`golish-db`、`golish-db/repo` 模块卡与 `docs/modules/INDEX.md`；父 feature 继续保持唯一 `in_progress`。
- **TDD RED 证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - Candidate/Cleanup terminal source mutation regression 在 guard 前 2/2 按预期失败（no-op UPDATE 成功）；run `3bf68398-f048-4b21-86eb-8d56847c86c1`。
  - 初版 guard 因旧 Cleanup retention trigger 排序先拦 DELETE，2 tests 中 1 fail；run `e9dadb6f-405f-4d6f-924e-f6fa4b7fdfcc`。新 guard 改为更早的稳定 trigger 名后统一错误合同。
  - `cargo nextest run -p golish-db --test reporting_read_model_migrations --status-level fail` 在 validated freeze / exact supersede guard 前 4 tests 中 2 按预期失败：validated manifest INSERT 成功、final→superseded 可夹带 `validation_result`；run `18005931-d5ad-49a1-baab-c65fc50d5bb7`。
- **GREEN / 已记录证据**：
  - Candidate/Cleanup terminal replay + immutability focused 2/2 passed；最终复跑 run `4bd27a20-d601-4226-aec8-21d3b587118a`（中间 GREEN run `43697720-f6e9-43c1-b461-0e4a9a7a198d`）。
  - Reporting migration 4/4 passed（含 validated manifest/section/claim/citation 各自 I/U/D、final→superseded exact freeze）；最终复跑 run `d63cfa06-7bc3-4839-8f16-9dcacd91b8f9`（中间 GREEN run `81552f1f-8676-476e-9a83-9ccb24e7f69c`）。
  - Reporting authority 11/11 passed（含 path rebind before build persistence、project retire before publish、legacy corruption deep-finalizer regression），run `b9f1fd23-05a7-45f3-afdb-135881bb944f`。
  - `cargo nextest run -p golish-reporting-domain -p golish-reporting-app --status-level fail` → 8/8 passed，run `a3bafadc-69d2-41ee-b60f-fe4a75d953a1`。
  - `cargo check -p golish-agent-app --tests` → exit 0；仅观察到共享树 `golish-projects` 已有 unused-import warning，本 scope 未改该 crate。
  - `cargo clippy -p golish-reporting-domain -p golish-reporting-app -p golish-db -p golish-agent-app --all-targets --no-deps -- -D warnings` → exit 0；本 scope 四包 all-targets 零 warning。去掉 `--no-deps` 的共享树命令被 artifact 并行工作尚未消费的 `golish-projects::{report_blobs_dir,report_staging_dir}` imports 阻塞，已交由 root 在 artifact owner 完成后统一重跑。
- **当前状态 / 风险**：新增复审项已有 fresh scoped Postgres 证据，但尚未在最终共享树重跑全量 `just precommit`，父 feature 不切 `passing`。未 commit、未 stage、未 push；没有执行真实 publish、外部 API、扫描或 Cleanup 操作。
- **以下文件已修改但未提交**：00011、三个 golish-db PG regression 文件、Agent Reporting command/bridge/Gate 与 authority regression、上述模块卡/索引和本 progress；共享树其他 agent 的 Candidate/Cleanup/artifact 改动均保留。

#### 2026-07-13 · C9 final review: production authority / final protocol / validation serialization

- **本轮目标**：按 C9 最终复审以 TDD 修复三个缺口：production `reporting_build_validated_revision` 不得走无 project authority 的内部路径；validated+unpublished revision 本体与 final transition 必须由 DB 完整守卫；child I/U/D 不得与 draft→validated 发生可夹带的 TOCTOU。仍只修改尚未 cutover 的 `20260712000011_reporting_read_model.sql`，未创建/修改 reserved `00005` / `00012`，未碰 artifact filesystem、Candidate 或 Cleanup producer。
- **已完成实现**：
  - production Reporting stage-entry 现在从 `operation_state` 服务端加载 active project + sealed scope snapshot + frozen canonical path + exact root unit，构造 `ReportingProjectAuthority`；build/reuse 只走 `PgReportTruthPort::with_project_authority`，并在 build snapshot/current read、validated revision persistence transaction 与 reuse 前后再次 exact compare。retired project、冻结后预先 rebind、witness 后 rebind 均在 report 落库前 fail closed。
  - 00011 将 validated+unpublished revision 本体完整冻结：普通 UPDATE 不能改 `validation_result`、source hash、transaction/revision metadata；唯一例外是保持其他列 exact 的 unpublished→final transition。新增 deferred finalization authority constraint，在 commit 核对 exact current revision、active `local_operator`、至少一个 content-addressed artifact ref、以及 operation/project/stream/source-version/payload exact 的 `ReportRevisionFinalized.v1` outbox。`report_revisions` repo 同时拒绝空 artifact set；旧 final-history fixture 改为正式 repo finalize，不再伪造 enum flip。
  - Reporting child guard 的 OLD/NEW parent lock 从 `FOR KEY SHARE` 提升为 `FOR UPDATE`；它与 draft→validated 的 parent row update 冲突。双连接 barrier 分别覆盖 section INSERT/UPDATE/DELETE：child transaction 未 commit 时 validation 必须阻塞，commit 后才可完成，因而不存在“先 validated、后夹带 child”的提交顺序。
  - 既有 Gate RR 测试不再依赖线上可篡改 validated attestation；改为显式、隔离的 pre-migration legacy-corruption fixture，并继续证明单个 RR Gate read 不能拼接两个失败状态。同步 `golish-db`、`golish-db/repo`、`golish-agent-app/ai`、`golish-reporting-app` 模块卡与 `docs/modules/INDEX.md`。
- **TDD RED 证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - `cargo nextest run -p golish-db --test reporting_read_model_migrations --no-fail-fast` → exit 100；三个新 regression 按预期失败：plain SQL 实际成功 final、validated attestation 实际成功 UPDATE、child INSERT transaction 未 commit 时 validation 已完成；run `195a2a94-6540-4a76-8057-f42e649a3ec4`。同次 characterization 中正式 repo finalize 已是 GREEN。
  - `cargo nextest run -p golish-agent-app --test reporting_authority production_stage_entry_rejects_retired_and_prebound_project_authority` → exit 100；retired project 仍实际创建 validated report；run `f41dcb4d-8a5c-4634-91b9-19af0169d500`。
- **GREEN / 已记录证据**：
  - production stage-entry retired + pre-entry rebind regression 1/1 passed，run `2480db24-dc39-49cd-90cb-253714eb543a`；既有 witness 后 rebind persistence regression包含在最终 authority suite。
  - `cargo nextest run -p golish-db --test reporting_read_model_migrations --no-fail-fast` → 8/8 passed，run `fdc3d5b5-829b-4963-bba3-76a776f6ae35`；direct-final fixture 已预挂 active principal、exact current 与 artifact ref，仍因缺 exact outbox 被拒绝，正式 repo finalize 同事务 artifact+outbox 则通过；I/U/D barrier 全绿。
  - `cargo nextest run -p golish-agent-app --test reporting_authority --no-fail-fast` → 12/12 passed，run `d8431b18-17ff-4137-9809-da54bfeb5022`。
  - `cargo nextest run -p golish-reporting-domain -p golish-reporting-app --no-fail-fast` → 8/8 passed，run `345fdf52-724e-4682-8a7c-26af8494c20b`；Reporting IPC authorization 2/2 passed，run `3280be3f-977e-4a51-a1db-d0bcd6fd12eb`。
  - 正式 filesystem stage/promote/verify + DB artifact/outbox closeout：`cargo nextest run -p golish-agent-app --test v2_closeout_replay candidate_to_report_closeout_is_replay_safe` → 1/1 passed，run `884897b4-ed00-4d40-b533-e94ab1a3c137`。
  - `cargo clippy -p golish-reporting-domain -p golish-reporting-app -p golish-db -p golish-agent-app --all-targets -- -D warnings` → exit 0，零 warning；`cargo fmt --all -- --check` 与 `git diff --check` → exit 0。冻结 `20260712000001_runtime_memory_foundation.sql` SHA-384 仍为 `ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4`；reserved `00005` / `00012` 仍不存在。
- **当前状态 / 风险**：三项 Reporting 最终复审缺口已有 fresh RED→GREEN、all-targets Clippy 与正式 closeout 证据；本子任务未执行共享树 full `just precommit`，父功能继续 `in_progress`，不虚报 passing。未 commit、未 stage、未 push，也未发起外部 API、扫描或真实用户项目 publish。
- **以下文件已修改但未提交**：`20260712000011_reporting_read_model.sql`、`report_revisions.rs`、Reporting migration regression、Agent Reporting/Gate bridge与authority regression、上述模块卡/索引及本 progress；共享树其他 agent 的 Candidate/Cleanup/artifact改动均保留。

#### 2026-07-13 · C9 artifact publication race / cross-platform filesystem hardening

- **本轮目标**：收口 Reporting artifact 最终复审：移除 Unix project-root check/use 窗口，保证 Finalizer duplicate/reverse-order 并发不会 self-lock/ABBA，stage replay 与 orphan GC 使用同一 content lock，并提供非占位的 Windows capability backend。未修改 Reporting SQL/migration、publication bridge、Candidate 或 Cleanup producer。
- **已完成实现**：
  - Unix 从 `/` 对调用方给出的原始绝对 project-root 逐组件 `openat(O_DIRECTORY|O_NOFOLLOW)`；不再执行 `symlink_metadata → canonicalize`。全部 ancestor dirfd 和 device/inode binding 均保留并复核，预存 root symlink 与检查后 root/parent swap 都 fail closed。
  - `ReportFinalizer` 先由 deterministic SHA-256 content key 去重、稳定排序并只对唯一 key stage/promote/verify/持锁；publication 使用同一唯一稳定序列，返回值按调用输入顺序重建并保留重复项。
  - stage/replay 在写 staging 前取得与 promote/GC 相同的 per-content reservation；重放校验同内容后在锁内刷新 staging mtime。GC 枚举后逐 key 等待同锁并重新打开、重新检查 grace，不能删除正在重放/发布的内容。
  - Windows 新增 `cap-std`/`cap-primitives` backend：从卷根逐组件 no-follow 打开并保留所有 ancestor handles，拒绝 symlink/reparse/junction；文件操作相对 capability directory，使用 hard-link put-if-absent、进程内 keyed mutex + `fs2` 跨进程独占锁、Win32 handle identity 与锁后 GC restat。`x86_64-pc-windows-gnu` 实际交叉编译和 Clippy 均已通过，不是文档占位。
  - 同步 `golish-projects[/file_storage]`、`golish-reporting-app`、`golish` 模块卡和模块索引；macOS 测试夹具显式传 canonical temp root，生产层仍不自行追随 symlink。
- **TDD RED 证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - Unix root swap regression 在旧实现稳定外写，run `9ffdd589-cc3c-4f2e-8f8c-80641f2337aa`。
  - duplicate key / reverse concurrent input regression 在旧 Finalizer 发生重复或逆序 reservation，run `f24fa788-4145-4fa1-972d-73bc2e077e73`。
  - 去掉 replay mtime refresh 的 characterization 会让等待同锁的 GC 删除 staging，run `bc82ef41-c82b-4644-a1f6-f2a449813648`；恢复刷新后转绿。
- **GREEN / 已记录证据**：
  - `cargo nextest run -p golish-projects -p golish-reporting-app --no-fail-fast --status-level fail` → 37/37 passed，run `8971a3cf-8191-4dea-8b91-749d13a10e4a`；覆盖 root symlink/swap、duplicate/reverse ordering、stage replay > grace 与 GC contention。
  - `cargo nextest run -p golish 'reporting_artifact_store::tests' --no-fail-fast --status-level fail` → 3/3 passed，run `22741d62-2be4-44a1-88af-efe6fec60c1e`；包含真实 migrated PG + Finalizer reservation/GC stale-snapshot seam。
  - `cargo check -p golish-projects --target x86_64-pc-windows-gnu --all-targets` → exit 0；`cargo clippy -p golish-projects --target x86_64-pc-windows-gnu --all-targets -- -D warnings` → exit 0。
  - `cargo clippy -p golish-projects -p golish-reporting-app -p golish --all-targets -- -D warnings` → exit 0，原生 scope 零 warning。
- **当前状态 / 风险**：实现与 scoped native/cross-target gates 已通过；本子任务未执行共享树 full `just precommit`，父 feature 继续 `in_progress`，不虚报 passing。Windows backend 已真实编译但当前 macOS host 不能执行 Windows runtime tests；未 commit、未 stage、未 push，也未发起外部服务或真实 publish。
- **以下文件已修改但未提交**：`golish-projects` report artifact API、Unix/Windows backend 与 target dependencies，`golish-reporting-app` artifact reservation/finalizer，`golish/reporting_artifact_store.rs`，上述模块卡/索引和本 progress；共享树其他 Candidate/Cleanup/Reporting DB 改动均未回滚或覆盖。

#### 2026-07-13 · C9 Windows artifact name-binding / verified-handle deletion P1 follow-up

- **本轮目标**：按最终安全复审以 TDD 修复 Windows artifact P1：普通 artifact/lock handle 不得共享 delete；content reservation 必须保留并复核 lock filename→same handle identity；verify hash 后必须复核 blob name binding；discard/GC 禁止关闭 handle 后按 pathname 删除。未修改 DB/memory/frontend、Unix backend 或 Reporting publication protocol。
- **已完成实现**：
  - 所有 production file opens 统一经过 no-follow helper，并显式把 share mode 收紧为 `FILE_SHARE_READ | FILE_SHARE_WRITE`；仅 name-binding 比较用短生命周期、允许 delete-share 的第二 handle，原 retained handle 全程阻止 rename/delete/replace。
  - Windows reservation 现在保存 lock directory、filename、retained handle 与 volume/file-index identity；acquire 后及 stage/promote/discard/GC mutation 边界同时复核 lock handle identity 与 filename→same identity。
  - verify 在 hash/read 完成后重新打开当前 blob name 并与 retained hash handle identity 比较；promotion 的 staging/blob 也在 mutation 前后复核 name binding。
  - discard/GC 以 `GENERIC_READ | DELETE` 打开目标，复核 name→handle identity 后通过 `SetFileInformationByHandle(FileDispositionInfo)` 标记同一 handle 删除；没有 `drop(file) → remove_file(name)` 路径。
  - promotion 保存已验证 `blob_identity`；释放初始 handles 并删除 staging 后，以只共享 read 的 final handle 重开当前 blob，核对原 identity、重算 hash/length 并再次复核 name binding，关闭 drop/reopen gap 的 false-attestation 路径。
  - 新增 Windows-only rename + direct replacement、真实 permissive lock inode split、verify-after-hash swap、GC-before-disposition swap、promotion drop/reopen swap regressions；race hooks 只按 directory identity + filename 匹配消费，不会被其他并行 artifact test 抢走。同步 `golish-projects[/file_storage]` 模块卡；模块职责/公开接口未变，因此 `docs/modules/INDEX.md` 状态列无需变更。
- **TDD RED 证据**：每条 Cargo 命令前均运行 `just space-guard`。
  - 首批四个 Windows regression 后，`cd backend && cargo check -p golish-projects --target x86_64-pc-windows-gnu --tests` → exit 101；旧实现缺少 `ContentKeyReservation::verify`、`verify_named_file` / `verify_named_identity` 与 verified-handle delete race seam，证明测试先于修复失败。
  - 新增 promotion drop/reopen gap regression 后，同一 Windows GNU tests check 再次 exit 101，缺少 `install_promote_before_staging_delete_hook`；旧流程在 blob/staging handles 同时释放后只核对 staging，不能证明返回的 blob 仍是先前已验证对象。
- **GREEN / 已记录证据**：
  - `cd backend && cargo check -p golish-projects --target x86_64-pc-windows-gnu --all-targets` → exit 0；五个 Windows-only regression 已由真实 GNU target 编译。
  - `cd backend && cargo test -p golish-projects --target x86_64-pc-windows-gnu --no-run` → exit 0；GNU linker 成功产出 Windows `.exe` test binaries（包含五个 race regressions）。
  - `cd backend && cargo clippy -p golish-projects --target x86_64-pc-windows-gnu --all-targets -- -D warnings` → exit 0，Windows target 零 warning。
  - `cd backend && cargo nextest run -p golish-projects file_storage --no-fail-fast --status-level fail` → 20/20 passed，run `a6dc07e5-405c-4ee1-8005-3bafdbc04f2c`；native Unix file-storage 回归全绿。
  - `cd backend && cargo clippy -p golish-projects --all-targets -- -D warnings` → exit 0，native all-targets 零 warning。
  - Windows 文件 `rustfmt --edition 2021 --check`、tracked progress/cards scoped `git diff --check` → exit 0。
- **当前状态 / 风险**：当前 host 是 macOS，Windows race tests 已真实交叉编译但不能在本机执行；需要 Windows CI/runner 执行 runtime regressions。共享树 full `just precommit` 尚未由本子任务执行，父 feature 继续 `in_progress`，不虚报 passing。未 commit、未 stage、未 push，也未执行外部请求或真实 artifact publish/GC。
- **以下文件已修改但未提交**：`backend/crates/golish-projects/src/file_storage/report_artifacts_windows.rs`、`docs/modules/backend/golish-projects.md`、`docs/modules/backend/golish-projects/file_storage.md` 与本 progress；共享树其余改动均未回滚或覆盖。

#### 2026-07-14 · runtime tool-call task-owner binding

- **本轮目标**：修复 Scoping 等 harness stage 的所有工具在 dispatch 前被 `tool_calls_runtime_task_owner_check` 拦截的问题；保持数据库 owner fence 不变。
- **已完成实现**：`DbTracker::start_tool_call_with_runtime` 现在从 trusted `RuntimeToolIdentity.operation_id` 派生 canonical `task_id`；tracker 未绑定 task 时正常落 exact operation owner，已绑定同一 owner 时接受，已绑定不同 owner 时在 dispatch 前 fail closed。legacy 无 runtime 的 tool tracking 语义保持不变。新增四个纯函数回归测试，覆盖 unbound、exact match、mismatch reject 与 legacy preserve；同步 `golish-agent-kit` / `db_tracking` 模块卡与模块索引。
- **验证状态**：按用户要求未运行修改后的测试。修改前执行 `just space-guard && ./init.sh`：`fmt`、`check-fe`、`test-fe`、`lint-rust` 已通过；用户要求停止后在 `test-rust-all` 阶段发送 SIGINT，命令 exit 130，因此本轮不能宣称完整验证或 passing。
- **当前状态 / 风险**：代码修复已落地但尚无 post-change fresh test evidence；未修改 DB schema/migration、未放宽 constraint、未 commit/stage/push。当前 `feature_list.json` 的既有唯一 `in_progress` 功能未改动。
- **以下文件已修改但未提交**：`backend/crates/golish-agent-kit/src/db_tracking/recording.rs`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-kit/db_tracking.md`、`docs/modules/INDEX.md`、`agent-progress.md`。

## 2026-07-14 - Persistent AIChatPanel context-compaction notice

- Goal: make short-term context compaction visible in the main ChatPanel in a Codex-like, persistent form.
- Implemented: successful compaction state is no longer cleared after five seconds; the existing notice now expands to explain the transition and show the pre-compaction token count.
- Privacy boundary: raw summarizer input and summary text are not rendered.
- Scope: frontend presentation and local event state only; no IPC, generated type, database, migration, or Memory Fabric backend changes.
- Verification: not run in this turn, following the user's instruction to edit directly without initialization or broad validation. This work is not claimed as `passing`.
- Feature tracking: `feature_list.json` was left unchanged so the repository's existing single `in_progress` feature was not displaced.
- Modified but uncommitted: `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`, `frontend/components/AIChatPanel/CompactionNotice.tsx`, `docs/modules/frontend/components.md`, `docs/modules/INDEX.md`, and `agent-progress.md`.

### 2026-07-15 · CLI/GUI Operation parity 与广州有创闭环恢复

- **本轮目标**：恢复 `cli-gui-operation-parity-company-closure-2026-07-14`，先让 CLI 与 GUI 复用同一 operation/session/profile/project/scope/approval/gate/evidence 内核，再以公司名-only 和 localhost fixture 验证 Scoping→Candidate 边界；取得 exact authorized target 后才做广州有创真实 active-stage 验收。
- **功能切换**：按用户本轮明确指令，把 Stage Team/Candidate→Verification 功能暂停为 `blocked`，保留全部 dirty-tree 实现；parity 功能恢复为唯一 `in_progress`。
- **授权边界**：`广州有创网络科技有限公司` 只作为 engagement subject。不得从公司名、公开 provider 结果或历史 workspace 猜测 domain/IP/CIDR/URL 授权；没有 exact target 时，两端必须在 EAS 前同样停住，且不得产生 scan evidence/Candidate rows。
- **环境验证**：`just space-guard` exit 0。随后启动 `./init.sh`；用户要求“不跑 init”后立即 Ctrl-C。终止前 `install`、`fmt`、`check-fe`、`test-fe` 已通过，命令最终在 `lint-rust`/`just check` 链路被 SIGINT，exit 130；本轮不把它当完整基线，也不再运行 init/precommit。
- **当前工作方式**：保留共享 dirty tree，不回滚/覆盖其他功能；仅运行 parity 直接相关的 focused RED/GREEN tests，不调用外部 LLM、企业 provider、扫描器或真实目标。
- **profile 约束**：用户明确要求本闭环只用 `red_team`；company-only、localhost、CLI/GUI parity 和后续 live run 全部固定该 profile，其他尚未完成的 profile 不作为验收证据。
- **已完成 · CLI fresh authority（后续用户决策已修订）**：direct EAS/Enumeration/Vuln/Candidate 等主动 slice 必须在本次 invocation 重传 exact `--target`，不能借同名组织的历史 target；从 Scoping 起跑且只有 `--org='广州有创网络科技有限公司'` 时现在会 get-or-create exact root 并用 `ConfirmedOrganizationIntake` 直接确认组织身份，但不预冻结 scope、不产生 target authority。明确 target 或 direct passive TargetIntel 仍走现有 bootstrap。
- **已完成 · GUI profile fail-closed**：发送、session 初始化和 picker 切换都必须先成功同步 backend profile；失败时不发 prompt、不标 initialized、不提交 UI/localStorage 状态，stage reset 也在 profile sync 失败时停止。
- **已完成 · smoke budget**：`scripts/stage_smoke.py` 现在按实际 slice 是否经过 Enumeration 决定 route-probe 小预算；`scoping -> attack_candidate` 会获得预算，Vuln/Candidate 起跑不会伪装经过 Enumeration。
- **focused evidence**：CLI direct-active/company barrier 2/2（nextest `33eb322a-c48a-4d7f-83ae-a6c9a5ff14fe`）；`red_team` shared company-only/direct-EAS barrier + TaskOperation 6/6（`acec8f5e-18e3-43e9-ae4b-e898ad49e078`）；GUI hooks 5/5、scoped Biome exit 0、`pnpm typecheck` exit 0；stage_smoke 5/5；Python combined diagnostics 6/6。新增 company-only CLI seed RED 为缺少 `should_seed_upstream` 编译失败，GREEN 2/2（nextest `bd4e5d99-4885-4ad6-befc-e5da5bafedf2`）。现有 `merge_source_query_row` dead_code warning 属共享 dirty tree，本功能未改。
- **本轮继续验证 / 修复**：按用户再次强调未运行 `./init.sh`。现有 `red_team_loopback_parity` 只证明 typed authority，nextest `98e0543e-463e-4f9c-8dfc-7b1a5bf33642` 为 2/2；production company-only HOLD、loopback pre-EAS、Candidate review barrier 与 CLI intake 合并 selector 为 6/6，nextest `b277d43e-a8b8-41cf-b1f5-930332e6eadd`。审计发现 shared fresh launch 接受未知 profile，而 CLI slice 会更早拒绝、orchestrator 又会静默 fallback `assessment`，可造成 GUI/CLI DAG 漂移；TDD RED nextest `b9580f62-2949-40e9-978f-d97d6fcb7f89` 0/1，随后在 `FreshTaskOperationLaunch::validate` 统一校验 embedded profile 并 fail closed，TaskOperation + loopback selector GREEN 10/10，nextest `dd86098d-3835-4b19-b3de-af9509b61445`。同步更新 `golish-agent-app/ai` 模块卡与索引。
- **live CLI acceptance**：在用户确认的 exact target 上以新二进制跑完 `scoping → attack_candidate`，session `stage-run-054467a0-e5a0-4b88-ac3e-57b386153772`、operation `7c86a7a5-38ca-476a-bc31-fda1fedee9ec`，CLI exit 0。Scoping、TargetIntel、EAS、Enumeration、Vuln、Candidate 全部最终 PASS；两个 phase boundary 都由显式 CLI authority 跨越；graph `next_node=__end__`、task `finished`，没有进入 Verification。TargetIntel 曾因 GOLISH-INTEL-ASN 使用 org-level identifier 得到一次 `needs_fix`，同一 run 修正后通过。
- **Enumeration 实况**：实际调用 `enum_preflight_web_origins`、`enum_crawl_same_origin_urls`、`browser_collect_js_api`、`js_extract_apis` 和三次 checkpoint-resume 的 `route_probe_paths`；落库 4 条目录/路径结果（`README.md=200`、`css/images/js=403`）、5 个 JS 文件 / 194691 bytes，0 promoted endpoint。Evidence 16/17/18/20 分别为 JS found、JSAPI empty、PARAM empty、DIR found。这是目录/路径枚举，不是目录遍历漏洞利用。当前 `max_requests=800` 是每次 wrapper invocation 上限而非 stage 总上限，三次恢复累计可超过 800，作为后续预算语义风险保留。
- **Vuln/Candidate 实况**：10 个 formulaic cell 均形成 terminal outcome。8 个 general Nuclei cell 是 evidence-backed `blocked`，精确原因是“本地没有可运行的可信模板”，`network_attempted=false`；GOLISH-NDAY 与 WSTG-ATHN-04 各为 `not_applicable`。Candidate 冻结 1 个 surface-analysis work item，最终 `no_candidate`、`candidate_count=0`、reason code `no_dynamic_attack_surface`；final handoff `f6a241bc-ef47-423c-9cd6-8d04377342d4` 引用 evidence 41–50。模型 rationale 错把 8 个 Nuclei tooling/template blockers 写成 WAF；已给 shared Candidate methodology 与 evidence-list contract 加 fail-closed 语义，并以 `candidate_reasoning_never_invents_a_blocker_cause` 回归覆盖。
- **Candidate 终点修复证据**：双 V2Only terminal slice 的 review barrier 顺序 TDD RED nextest `c1eae075-1504-4fa7-b7a3-225af0eb609e` 0/1，修复为先解析 successor；无 successor 直接 Allowed，完整 Candidate→Verification DAG 仍按 exact barrier HOLD。Candidate blocker semantics + terminal/full-DAG tests GREEN 3/3，nextest `f93f10d0-9ea8-40c5-b0cd-ec251c25ef89`。一次错误 filter run `6e91bbbe-4754-4070-83ff-0730d19e83e2` 选中 0 tests，不计 evidence。
- **已知未闭合**：Candidate unit/handoff/graph/task 均已终态，但 exact `stage_runs` row `b9eb3295-31a4-4463-bde3-affc17cfaf68` 仍为 `started`。根因是现有逻辑只在进入 successor 时关闭上一 stage execution，投影 DAG 终点没有 successor。正确修复需要在 `golish-db` 增加不改 schema/migration 的原子 terminal close transaction；按 AGENTS 高风险规则，等待用户明确授权后再动。WaveUnit 保持 `review`、consolidation `pending` 是本次停在 Candidate 的预期状态，不应被强行关闭。
- **仍缺**：用户授权后的 terminal `stage_runs` 原子 close、完整 GUI↔CLI normalized fixture，以及 AGENTS 要求但被用户明确排除的 full precommit；功能继续 `in_progress`，不宣称 passing。
- **提交记录**：未 commit、未 stage、未 push。

### 2026-07-15 · GUI Scoping Tokio worker stack overflow 修复

- **本轮目标**：修复 GUI Task 模式下 Scoping deliverable 已被 gate 接受、主 agent 已 `Turn complete` 后，后端 `tokio-rt-worker` 打穿 32 MiB stack 并 abort 的问题；不改 gate、DB schema、provider failover policy、授权边界或 stage 业务语义。
- **根因证据**：session `pentest-chat-1784100749109-1` 的 `run.log` 最后正常记录 `submit_stage_deliverable accepted` 与 `Turn complete`；`transcript.json` 缺少 Completed 事件；`run_tree.py --full --db` 显示 submission `c08ebfe5-...` 已落库，但 execution `4fb52720-...` 仍为 `started`、无 handoff/scope snapshot。macOS crash reports `golish-2026-07-15-153140.ips` 与 `golish-2026-07-15-153339.ips` 两次在同一 `AgentBridge::execute_with_context_inner` 指令 abort，faulting thread 均为 `tokio-rt-worker`，stack guard 两侧均为 32 MiB。旧 debug binary 反汇编显示 `execute_with_context_inner` poll frame 约 10.9 MiB、`maybe_failover_to_fallback_model` 约 10.3 MiB，成功结果也会进入后者并与 TaskOrchestrator/bridge 外层 frame 同步叠加。
- **已完成实现**：`execute_with_context_inner` 现在对 `Ok(response)` 直接保留成功结果；只有 `Err(primary_error)` 才构造并 poll `maybe_failover_to_fallback_model`。failover helper 改为只接受 `anyhow::Error`，默认关闭、eligibility、fallback client rebuild 和 error event 语义不变。没有通过继续增大 `RUST_MIN_STACK` 掩盖问题。
- **新鲜验证证据**：
  - `cargo fmt --manifest-path backend/crates/golish-agent-bridge/Cargo.toml -- --check` 与 scoped `git diff --check` → exit 0。
  - `cargo nextest run -p golish-agent-bridge failover --no-fail-fast --status-level fail` → exit 0（7 个 failover tests）；`cargo nextest run -p golish-agent-bridge --no-fail-fast --status-level fail` → exit 0（list 共 35 tests）。
  - `cargo clippy -p golish-agent-bridge --all-targets --no-deps -- -D warnings` → exit 0。
  - `cargo build -p golish --bin golish` → exit 0，新 debug binary UUID `E34E942C-976B-365F-B2F1-3657F999E07F`；LLDB/反汇编确认 failover future 的构造/poll 仅位于源码 `Err(primary_error)` 分支，success path 不再经过本次 crash instruction。
  - 用户要求停止 broad init 前，已经启动的 `./init.sh` 在 fmt/check-fe/test-fe 通过后，被共享 dirty tree 的七个既有 `golish-db` Clippy finding 阻塞；随后未再运行 init/precommit。一次带依赖的 scoped Clippy 同样只因这七项 DB finding exit 101，改用 `--no-deps` 后本 crate 通过。未修改这些无关 DB 文件。
- **当前状态 / 风险**：代码、focused tests、生产 debug build 与机器码分支证据均已闭合；尚未调用外部 LLM/provider 做 fresh GUI Scoping 实跑，因此不能把旧 execution `4fb52720-...` 当作已完成，也不把父 parity feature 标为 passing。需要重启应用后由用户发起新一轮 Scoping 验收；原样 resume 旧半完成 execution 前应先走产品现有 recovery/continuity 选择。
- **以下文件已修改但未提交**：`backend/crates/golish-agent-bridge/src/agent_bridge/execution.rs`、`docs/modules/backend/golish-agent-bridge/agent_bridge.md`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`。共享树其他改动均未回滚或覆盖；未 commit、未 stage、未 push。

### 2026-07-15 · CLI Company Controller parity 验证与 run_tree 诊断增强

- **本轮目标**：确认 headless CLI 与 GUI Task 是否进入同一 Company Controller 运行内核；增强 `scripts/run_tree.py`，让后续新 run 能直接暴露 Controller 计划、动态 SubAgent、同链恢复、Gate 与 AI 调用信息。未调用外部 LLM/provider、企业情报源或真实目标。
- **CLI/GUI 结论**：两端入口语义不同（GUI=`FullProfile`，CLI=`StageSlice`），但都进入共享 `prepare_task_operation`、`PreparedTaskOperation::run_fresh`、`TaskOrchestrator`、`BridgeAgentExecutor`、agentic loop 与唯一 `stage_run` tool handler。当前本机 `runtime_memory_rollout` 为 `v2_only` rank 3；当前源码在 `target_intel` 的新 operation 会进入 `company_controller_v1`。现有 session `pentest-chat-1784105910280-1` 虽为 V2Only，但其已持久化 TeamPlan 是改造前的固定 Producer/Aggregator 形态，不能作为新 Controller 验收证据。
- **run_tree 实现**：timeline 现在使用最新 observed stage；按真实 team prefix 把 `::lead:` Controller 与 `::worker:` 动态 child 关联，显示 Controller chain/turn/resume、`update_plan` 总览和最多 12 个具体步骤、dispatch→wait→child tool/prose/output、prepare/final submit、Gate 与异常。`run.log` 另汇总 main completed turns/provider/model/token 以及 SubAgent model starts（明确不等同于工具调用或完成 turn，且不伪造 child token）。旧固定 Team transcript/DB plan 明确标 `legacy-fixed (not Company Controller)`。
- **新鲜验证证据**：每次 Cargo 前 `just space-guard` exit 0；`cargo nextest run -p golish stage_run::tests --no-fail-fast --status-level fail` → 47/47，run `7ae232e0-2413-4ba5-9681-3a61c7bb77e8`；`cargo nextest run -p golish-agent-app --test red_team_loopback_parity --no-fail-fast --status-level fail` → 2/2，run `acb87a02-176d-4b80-b476-04b43245521f`；`cargo nextest run -p golish-agent-runtime company_controller --no-fail-fast --status-level fail` → 6/6，run `2c284c47-7f16-4c2a-af53-d9e6a434b90b`；`cargo nextest run -p golish-agent-runtime stage_team_scheduler --no-fail-fast --status-level fail` → 14/14，run `3733cd62-4dcd-4b65-8d05-52d4988532c2`；`cargo nextest run -p golish-agent-runtime stage_team_update_plan --no-fail-fast --status-level fail` → 3/3，run `1793570a-dc50-4748-91d8-6c0fc4a398eb`；run_tree Python 13/13、`py_compile`、scoped `git diff --check` 均 exit 0。
- **真实旧日志回放**：`python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 pentest-chat-1784105910280-1 --full --db` exit 0；header 正确显示 latest `target_intel`（seen `scoping,target_intel`），9 个旧 worker 均标 legacy，`controllers=0`，AI-call summary 显示 main 2 turns 与 SubAgent 9 starts，DB Team plan 标 `mode=legacy_fixed_team`。
- **边界 / 仍缺**：没有执行新的真实 CLI provider smoke，因为生产 bridge 拒绝 mock client，而 Target Intel 活体会访问外部 provider；需要用当前构建新建一次获授权的 run，才能现场证明每公司唯一 Controller、动态 child、同 Controller Gate repair。当前只有 `target_intel` stage spec 配置 `team_scheduler`；不能把本轮测试表述为所有 stage 均已切换。父功能继续 `in_progress`，未跑 `init.sh`/`just precommit`，未 commit/stage/push。
- **以下文件已修改但未提交**：`scripts/run_tree.py`、`scripts/tests/test_run_tree_company_controller.py`、`docs/modules/backend/golish/stage_run.md`、`docs/modules/INDEX.md` 与本 progress；共享 dirty tree 中其他既有改动未回滚或覆盖。

### 2026-07-15 · Company Controller live BLOCK 收口与当前 Plan UI

- **现场 / 根因**：最新 live session `pentest-chat-1784117461035-1` 使用 20:08 启动的旧 binary。Target Intel 已完成 5/6，唯一缺口是组织级 `GOLISH-INTEL-ASN`：Quake 返回 `HTTP 401 /quake/login`，FOFA/Hunter/Shodan 无凭据；这应作为 `credential_missing` 的 terminal `blocked` cell 收口，而不是伪装 found/checked_empty。旧 prepare-final 没有先 durable close request epoch 并绑定唯一 Controller final submitter，先报 `stage_team_submission_requires_unique_aggregator`；外层 Agent 越权直提随后报 `missing_stage_run_unit`；过期 Controller 又报 `stage_team_leader_claim_replay_mismatch`。首次 `update_plan` 同时给两步 `in_progress` 被严格校验拒绝，随后 Controller 已自行合并并成功更新，因此该红叉不是 Gate BLOCK 根因。
- **已完成实现**：当前源码的 Company Controller finalization 使用 `close_stage_request_epoch → bind_stage_team_leader_final_submitter → same Worker final turn`；本轮补空闲过期 Controller 的 exact operation/execution/unit/org/item/checkpoint fenced reclaim，保留原 WorkerRun/message chain，只递增 attempt epoch，active-tool split state仍 fail closed 到 recovery。Coverage terminal preview 复用 `StageDeliverable::ReasonKind` 反序列化，非法 `env_unavailable` 不再得到 `ready_to_submit=true`，合法 `credential_missing` 保留。Controller objective 与专属 `update_plan` schema 明确“plan 表示当前 focus 而非并发度”，并行工具/worker 必须合并成一个复合 `in_progress` step。Controller 详情页只显示最新有效 live/completed 的“当前计划”；旧版、失败和非法 `update_plan` 不再渲染普通工具卡，原始事件仍保留在 transcript/run.log/run_tree 诊断面。
- **TDD / targeted evidence**：expired Controller RED nextest `6c157e13-b804-4683-ba9b-0f89cd442108` 复现 replay mismatch，GREEN `4798d7c7-4ebc-497e-9580-09c6d5cbb878` 1/1。主线程合并验证：DB Company Controller + unique submitter/producer fence 4/4，nextest `7b542482-9122-4c2a-8a6b-d9c82743d1c5`；runtime/sub-agents Controller + plan 9/9，`4ebdea95-de67-4135-a744-bd03587267ed`；coverage reason parity 3/3，`37ef2e4e-4969-4629-b85a-4f3ab222ecf6`；Controller detail Vitest 59/59，Biome、`cargo fmt --all -- --check`、scoped diff check均 exit 0。`just check-fe` 与 `just test-fe` exit 0。dev backend 已于 21:48 重启到新 binary。
- **全量门禁**：`just precommit` 的 fmt、check-fe、test-fe 通过；lint-rust 被共享 dirty tree 中 7 个既有 Clippy finding 阻塞（`runtime_memory_tx.rs:3118/5820/5862/7507/8122`、`stage_teams.rs:1700/2075`），均不在本轮 scoped hunk，未越界顺手修改。因此功能继续保持唯一 `in_progress`，不宣称 passing。
- **边界 / 下一步**：本轮未调用外部 LLM/provider 或真实目标。旧 session 已进入错误重试链，不作为新代码验收证据；取消旧 run 后使用 21:48+ binary 新建 run。网络/认证失败可以让阶段以诚实的 blocked coverage 完成，但不能把采集本身显示为成功。未改 DB schema/migration，未 commit/stage/push。
- **以下文件已修改但未提交**：Controller reclaim repo/test、coverage preview、Controller prompt/tool schema、Controller detail Plan UI/test、相关模块卡/索引与本 progress；共享 dirty tree其它改动均未回滚或覆盖。

### 2026-07-15 · Intel Gate 最新日志纠偏与真实 CLI 最终闭环

- **用户纠偏 / 精确日志**：用户确认已经重新编译；本轮不再沿用旧 binary/session 结论。最初精确 GUI session 为 `/Users/christopherzheng/golish-platform/Test1/.golish/transcripts/pentest-chat-1784124538690-2`：Company Controller 已返回业务 JSON，但末尾兼容 `[sub_agent_session_id: ...]` 被 whole-response `serde_json::from_str` 当作 trailing characters；随后重试进入 unique finalizer/replay dead-end，Intel deterministic Gate 根本没有获得可接受的 aggregate closeout。
- **已完成修复**：durable structured `chain_id` 成为唯一 resume authority，只剥离 UUID 完全一致的 legacy marker；Stage Team claim 返回并刷新锁后当前 Unit，避免沿用 stale `row_version/status`；Company Controller success 从 current operation aggregate truth 生成 exact `pass_token/closeout_claim`；CLI terminal report 在 task finished 且 active execution 为零时确定性选择 current-stage 最新 completed execution。新实跑又暴露 Gate BLOCK 后 Controller coordination turn 可误用 generic `submit_result` 绕过 router、继而触发 barrier JSON parse/replay mismatch；executor 现返回非终态 `STAGE_TEAM_CONTROLLER_REQUIRES_ROUTER`，保持同一 chain，直到 trusted dispatch/prepare-final router 收回控制权。`scripts/run_tree.py --db` 使用相同 terminal selection 语义，不再把合法 finished run 误报为 missing active/incomplete V2。
- **TDD RED → GREEN**：claimed Unit 回归先因返回结构缺 `unit` 失败；Controller aggregate closeout 回归先缺 exact token/claim；terminal CLI report 回归先报 `V2 CLI requires one exact active stage execution, found 0`；Controller router 回归先因 rejection helper 尚不存在而编译失败；run_tree terminal 回归先复现 missing active，初版 selector 又以 row shape `IndexError` 失败。修复后 `cargo nextest run -p golish-sub-agents` 197/197、`cargo nextest run -p golish-agent-runtime company_controller_` 9/9、`python3 -m unittest scripts.tests.test_run_tree_runtime_memory` 8/8；affected crates all-target Clippy `-D warnings`、`cargo build -p golish`、`cargo fmt --all` 均 exit 0。
- **真实 CLI 最终验收**：fresh workspace `/private/tmp/golish-company-controller-marker-v2-20260715-5`，session `stage-run-bfd451bf-71b4-4a09-a805-65f5ca64d3fa`，operation `75186eb3-0e38-4074-a7ef-20d41a367e6f`，organization `75f1cbac-d4ba-477e-8a31-1f975de524ba`，Target Intel execution `80e23462-7695-4d55-b306-fa8bc4c8b290`。使用重新构建的 `./backend/target/debug/golish` 从 Scoping 跑到 Target Intel，显式 passive-only objective、`--auto-approve --approve-phase-boundaries --json --db-smoke-summary`，进程 exit 0。
- **业务 / DB 真相**：Scoping Gate PASS；Target Intel `stage_run` 返回 `success=true, passed=true, team_units_passed=1`，Controller submission `912de35a-f4a7-4bd3-a2b7-b78917d9a575`、handoff `5834efa4-1852-4781-a6d1-f33681cbc215` 与 pass token `2d1fb570eeceda4dcf2e7bb3349c17be43103762304bc9ee0c3e3731dc704837` 持久化；外层 Target Intel Gate PASS，graph 访问 Scoping/TargetIntel 后 Completed，task `finished`。DB 为 `v2_only`，两个 execution 均 completed，Unit passed row_version=2，Controller Worker passed，plan barrier ready，唯一 final submitter、submission/handoff 与 selected V2 read source一致。模型在调用 `stage_run` 前曾越权主提交一次并被 `missing_stage_run_unit` fail closed，随后自行走正确 stage_run；这不是最终失败。run_tree 仍记录一次错误 `query_target_data` 参数作为可恢复 model anomaly，但不影响 Gate/DB/exit 0。
- **全仓 RED → GREEN 补证据**：第一次 full precommit 依次暴露并修复 Candidate preview 未把 decision evidence 限定在 exact frozen manifest、EAS WEB `checked_empty` 未优先返回 exact-origin terminal rejection、以及三个旧 migration/test fixture 与新严格血缘/typed follow-on 语义不一致。随后全仓首轮 5731 tests 收齐 3 个独立失败：foundation-only submission repo 误查尚不存在的 Team 表、pre-attestation Worker fixture 使用后续 `work_item_id` 列、domain expansion 旧断言未反映显式 apex query 去重；均按真实 schema/contract 修正。`attack_execution_v2_migrations` 完整 76/76 passed（nextest `31e4d3fa-c1bf-43ab-9fe4-a8c613c19a20`）；最终 workspace 5731/5731 passed、11 skipped（nextest `f7912b77-af7c-4b85-a00a-28cc384cf0cc`）。
- **最终门禁 / 已记录证据**：`just precommit` 的 fmt、check-fe、test-fe、lint-rust、test-rust-all 全部通过。首次 `check-types` 仅因共享 dirty tree 的正确 ts-rs 生成结果尚未进入 Git index 而退出 1；未改真实 index，复制临时 index 并只登记 `frontend/lib/generated/` 后重跑同一个 `just precommit`，生成器若有任何二次漂移仍会 fail。进一步发现后半段 `just test` 会再次执行 export tests并恢复 ts-rs 的行尾空格，现由 `normalize-ts-rs-bindings.mjs` 在 gen-types及两种全 Rust recipe 后确定性归一化 aggregate unions。最终整条命令依次通过 `check`、独立 `test`、稳定 `check-types`、`git diff --check`、`jq empty`、唯一 in-progress 与真实 staged-list 断言，输出 `✓ All checks passed!` 和 `FINAL_GATE_OK real_cached_entries=0`，exit 0。
- **提交记录**：未 commit、未 stage、未 push；未改 DB schema/migration，未执行额外真实目标扫描或外部 provider 请求。
- **状态 / 已知风险**：Intel CLI slice 已用新 binary 真实闭环，不再需要新 task/chat，也不是只靠重启应用。当前共享父 feature 还包含更广的 Candidate→Verification recovery DoD 与需用户授权的新 forward migration，因此保持唯一 `in_progress`，不虚报整项 passing。共享 dirty tree 中其他功能改动均保留并已在本节和既有记录列明。
- **下一步最佳动作**：Intel incident 无后续补救项；后续若继续父 feature，按既有 design/plan 在获得 migration 授权后实现 Gate repair generation，再按 `feature_list.json.verification` 全项重验后决定是否转 `passing`。
- **本轮相关未提交文件**：`golish-agent-runtime` Stage Team scheduler/call、`golish-sub-agents` executor parsing、`golish-db`/`golish-agent-kit`/`golish-agent-app` runtime-memory claim contracts、`golish` CLI V2 report、`scripts/run_tree.py` 与回归测试、`justfile`、`frontend/scripts/normalize-ts-rs-bindings.mjs` 及同步生成 bindings，以及对应模块卡、索引、`feature_list.json` 和本 progress。
### 2026-07-16 · Active Recon 目标范围一次确认（实现中）

- **本轮目标**：修复 Target Intel Gate PASS 后 UI 只显示 `Waiting for approval`、实际却因缺少可信精确目标而停住的问题。采用一次明确授权：展示当前 operation 的 provider-discovered target 列表，用户确认全部或子集后直接进入 EAS，不再弹第二个通用 phase approval。
- **设计边界**：公司名继续只代表 engagement subject；provider discovery 不自动授权。只接受当前 operation/org/Target Intel window 的非空原样子集，新增、改写、空选、Skip、timeout、DB 错误、候选漂移全部 fail closed；direct EAS stage slice 仍要求预先可信目标。
- **文档**：新增 `docs/design/2026-07-16-active-recon-target-scope-confirmation.md` 与 `docs/superpowers/plans/2026-07-16-active-recon-target-scope-confirmation.md`，并并入既有唯一 `in_progress` 父 feature，未创建第二个 active feature。
- **验证约束**：用户明确要求不跑 `init.sh`。此前已启动的 init 在 fmt/check-fe 后进入 test-fe 时立即中断；本轮后续只跑 focused Rust/Vitest/format/diff checks，不跑 `just precommit` 或全量门禁，因此 feature 保持 `in_progress`。
- **当前状态**：设计与实现计划已落盘，下一步先写 RED tests，再实现 repository transaction、orchestrator boundary 与前端状态语义。未调用外部 LLM/provider、未扫描真实目标、未改 migration、未 commit/stage/push。
- **已完成实现**：`TargetIntel -> EAS` 先检查当前 operation/org 是否有本阶段 refreshed `asset_intel` row；有则展示该 org 当前完整 trusted + asset-intel active denominator。人工只能确认原样非空子集；DB 在 operation row lock 下重读并防 candidate drift，selected provider rows 原子升级为 `customer_provided/in`，未选 rows 改为 `out`，同事务写 operation-bound state marker 与 authorization audit。确认成功后直接进入 EAS，不再调用 generic `before_active_scan` approval。direct EAS 仍只有 trusted-target preflight，company-only resume 只接受同 operation marker 与当前 trusted set exact-match。
- **前端语义**：新增 `waiting_target_scope` stage marker，显示 `Review scan targets` 而不是 `Waiting for approval`；继续复用不可 auto-confirm 的 `scope_review` 表，删除行代表缩小范围，新增/编辑由后端拒绝。
- **新鲜 focused 验证**：
  - `cargo nextest run -p golish-agent-kit -E 'test(active_recon_scope)'` → 3/3，run `45c51a52-9ef0-4161-a362-e0588a7c0ba7`。
  - `cargo nextest run -p golish-agent-kit -E 'test(active_recon_scope) | test(pre_eas) | test(two_level_phase_gate) | test(direct_eas)'` → 12/12，run `54436c19-f787-49b4-9c5e-4a517861faf5`。
  - `cargo nextest run -p golish-agent-app -E 'test(active_recon_scope)'` → 2/2，run `be716aab-0b61-42de-a5c1-9a2cf56f6e25`。
  - AIChat target review/event 聚焦 Vitest → 72/72；affected Biome → exit 0。
  - `cargo clippy -p golish-agent-kit -p golish-agent-app --lib --no-deps -- -D warnings`、两个 affected crate rustfmt check、`jq empty feature_list.json`、scoped/new-file diff check → exit 0。每次 Cargo 前均运行 `just space-guard`。
- **未做 / 不能宣称**：没有运行 `init.sh`、`just precommit` 或全量测试；没有连接真实 embedded Postgres 做 transaction integration，也没有调用 provider/真实目标或发起 fresh GUI continuation。因此这里只证明编译、纯逻辑、SQL contract 与 UI 回归，不把父 feature 标为 passing。dev watcher 曾在 01:36 使用新代码完成重编译并成功启动 DB，但收尾检查时 Golish app process 已不在运行；用户需要重新打开应用，在原 task 发送“继续”即可验收，无需新建 task。
- **提交记录**：未 commit、未 stage、未 push；未改 DB schema/migration。共享 dirty tree 其它改动未回滚或覆盖。

### 2026-07-16 · Downstream Stage Team Company Controller 收敛（实现中）

- **本轮目标**：修复 `target_intel` 使用 durable Company Controller / 链内 `update_plan` / DB-backed Team UI，而 EAS、Enumeration、Vuln 仍回落 legacy specialist 的真实产品分叉；把这三个按公司执行的后续阶段接入同一 Controller 合同并跑隔离 CLI 到闭环。
- **已确认根因**：只有 `resources/harness/stages/target_intel/spec.json` 声明 `team_scheduler`；EAS、Enumeration、Vuln 均为 `null`。runtime 因而只为 Intel seed `leader:primary`，其余阶段走 legacy `Main Agent -> Specialist`；同时 `stage_team_executor_specialist` 还把所有 Controller/child role 硬编码成 `recon`，无法安全直接打开下游 rollout。
- **设计边界**：新增 `docs/design/2026-07-16-stage-team-downstream-convergence.md` 与对应实现计划，并入现有唯一 `in_progress` 父 feature。只统一 EAS/Enumeration/Vuln；Candidate/Verification 保留 Wave/CandidateAttempt 调度，Post-exploit/Reporting/Cleanup 保留 typed scheduler。无 migration、不放宽 scope/Gate、不扫描真实外部目标。
- **验证约束**：用户明确要求不运行 `init.sh`，并要求自行运行 CLI 直到闭环。本轮采用聚焦 TDD、scoped lint/format/type checks 与 ephemeral DB + localhost fixture CLI；未 commit/stage/push。
- **已完成实现**：EAS/Enumeration/Vuln StageSpec 均开启 Company Controller，runtime 从 durable Unit frozen specialist 精确映射 `prober` / `enumerator` / `vuln_scanner`；Controller 可自行收口或动态派发本阶段 child，只有 Controller 拥有计划与 final submit。EAS/Enumeration/Vuln 纳入 Team stage admission；Gate PASS 后只有 Intel/EAS 走兼容 terminal coverage materializer，Enumeration/Vuln 保持 producer-owned authoritative outcomes。Controller park 后先停 heartbeat 再 drain child，消除假 lease-lost 警告。当前无 migration schema 将 Controller Gate repair 冻结为 1 轮，避免第二条 gap source 违反唯一约束。
- **实跑才发现并修复的两个缺口**：`vuln_probe_anonymous_access` 的 server eligible endpoint 集合比通用 endpoint query 更窄，过去 Controller 只能猜 id 组合；现在 mismatch 保留 partial result 并返回排序后的 `eligible_endpoint_ids` / count / exact retry action。legacy `agent_logs.agent_type` 不接受 `vuln_scanner`；tracking bridge 现仅在 legacy telemetry 表将 `vuln_scanner` / `attack_analyst` / `candidate_verifier` 折叠为 `pentester`，runtime/UI 仍保留精确 role。
- **前端对齐**：`StageRunOrgRows` 在 exact Team pointer 存在时对 Intel/EAS/Enumeration/Vuln 一律渲染 DB-backed `StageTeamRunView`，抑制 legacy Main Agent 卡；compact summary/detail 共用 stage agent label，Company Controller / Prober / Enumerator / Vuln Scanner 口径一致。
- **TDD / 运行过的验证**：StageSpec RED `d74d4771-a281-4f46-b5d8-e649ed267085` → GREEN `c5eed411-47f4-4263-8860-30610ce8e124`，StageSpec focused 42/42（`f0f993a0-3fe2-4b3f-bc68-41269892cac8`）；runtime 聚焦 15/15（`dbae4668-9f00-4223-8e6e-267f265f47ba`）、27/27（`ac4e7b7a-3efb-44b3-b5a1-95134ca326d1`）、stage admission 3/3（`f85c3296-bd8c-4772-9ec7-2c279e860f28`）、Team wider 36/36（`2fff1b6a-7f02-4b26-a870-329f8975ca1a`）、latest `stage_team` 25/25（`39e5d633-e4d3-442c-ac17-c3e9b2b597e5`）；`golish-sub-agents` 197/197（`c583d23d-de0b-4cd9-b40c-c10c2ff468c3`）；Vuln bridge 32/32（`903811c2-be9b-4eef-9fbf-76c0e7b03bae`）；anonymous recovery RED compile → GREEN 1/1（`dd83cda7-7c57-4810-ac81-360d0bce37c6`）；Controller repair cap RED `e79557d4-9139-4408-9396-022394238633` → GREEN 1/1（`ef2ed890-6d1d-4c8d-8fa4-90c2b0cb622f`）；telemetry RED compile → GREEN 1/1（`3f72b3e1-100d-4483-9977-9e730bb1714d`）。Frontend tools/StageRunOrgRows/StageTeamRunView 39/39，`pnpm typecheck`、affected Biome、spec/feature JSON、scoped Clippy 与 `cargo build -p golish --bin golish` 均 exit 0。每次 Cargo 前均运行 `just space-guard`。
- **最终聚焦门禁**：`cargo fmt --manifest-path backend/Cargo.toml --all -- --check` exit 0；`cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app -p golish-pentest-app --all-targets -- -D warnings` exit 0（53.57s）；frontend tools/StageRunOrgRows/StageTeamRunView 39/39 + ToolCallSummary 9/9；`pnpm typecheck`、affected Biome 6 files、`jq empty` 四个 JSON、唯一 `in_progress` 断言与 `git diff --check` 均 exit 0。
- **全仓 `just precommit` 结果**：实际执行 `just space-guard && just precommit`，`fmt` passed（3s）、`check-fe` passed（10s）、`test-fe` passed（20s）、`lint-rust` passed（60s）、`test-rust-all` passed（292s）；最后 `check-types` exit 1。差异仅为共享 dirty tree 中已有的 ts-rs 源码→generated 漂移：`AttackCandidateApprovalView.startBefore`、`AttackCandidateReviewDecisionRequest.startBefore`、`GeneratedAiEvent` / `GeneratedHarnessTraceKind` 的 Stage Team pointer、pending enrichment 字段与空格归一化。本 scope 未手改 generated IPC。随后以当前工作树为基线、仅在临时 `GIT_INDEX_FILE` 中执行 `just check-types` 已 exit 0，证明 Rust→TS 当前生成内容语义一致；真实 `.git/index` 前后 SHA-1 均为 `286d5ee695f602166e45fc967b0ed1331cb52000`，未 stage 任何文件。普通 precommit 仍会因这些未提交 generated diff 相对真实 index 而退出 1，不虚报该退出码。
- **CLI 闭环 / 已记录证据**：最终使用当前编译 binary、fresh workspace `/private/tmp/golish-downstream-v2only-finalfix-20260716-bsYnCm`、仅绑定 `127.0.0.1:54610` 的 fixture，session `stage-run-2cebfd1b-87cf-4863-97b6-df263032aead`，operation `9599a356-58be-40f7-b34f-19754a607976`，CLI exit 0。报告为 EAS / Enumeration / Vuln 三阶段全部 PASS，Fleet 1/1 PASS；三次 `stage_run` 均 `scheduler=company_controller_v1, units=1, success=true`。`run_tree.py --full --db` 证明三个 Unit/Controller Worker 均 passed，frozen specialist 依次为 `prober` / `enumerator` / `vuln_scanner`，三个 durable submission + handoff 与 16 条 exact evidence 存在；Vuln 含 10 条 evidence-bound technique，anonymous access 产生 found evidence。最终 `run.log` 无 heartbeat loss、invalid agent enum、duplicate gap source、replay mismatch 或 Company Controller failure 签名。legacy telemetry DB 计数为 `primary/enumerator=4, primary/pentester=2, primary/prober=4`，证明折叠仅发生在 legacy 表。fixture 已停止，全程未接触外部目标。
- **诊断跑说明**：前一条 workspace `/private/tmp/golish-downstream-v2only-telemetry-20260716-Pnhx4p`、session `stage-run-e6f6e5dc-2209-4981-9dd0-d2dd2ae6e27b` 在确定 anonymous eligible-set、duplicate Controller gap 和手工再调用 seed epoch 问题后人工中断（exit 130），不算成功证据。本 slice 修复正常 Controller 一轮 repair 路径，没有宣称修复泛化的 terminal stage 手工重调用/resume seed 语义。
- **提交记录**：未 commit、未 stage、未 push；未改 DB schema/migration/generated IPC；未运行 `init.sh`。
- **已知风险 / 未解决**：这个 downstream slice 已闭环，但父 feature 仍包含 Candidate→Verification 更广的 recovery/migration/GUI DoD，因此继续保持唯一 `in_progress`。多于一轮的 Controller Gate repair 仍需用户单独批准向前 migration；泛化 terminal stage 手工重调用的 seed epoch continuation 不属于本次对齐范围。全仓唯一未绿门禁是上述跨 scope generated IPC 漂移，不影响本次 CLI/DB 闭环，但阻止父 feature 标 `passing`。
- **下一步最佳动作**：将这个已验收的下游合同保持不动；后续回到父 feature 时，先对 Candidate/Verification 未完成 DoD 建立独立验收，不要再把 CandidateAttempt 强行改成普通 Company Controller UI。
- **以下文件已修改但未提交（本 scope）**：三个 downstream stage specs；`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call.rs,stage_team_scheduler.rs}`；`backend/crates/golish-agent-app/src/ai/tracking_bridge/records.rs`；`backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs`；`frontend/lib/tools.ts`、`frontend/components/AIChatPanel/ToolCallSummary.tsx`、`frontend/components/Engagement/StageRunOrgRows.tsx` 及聚焦测试；对应 design/plan、模块卡、INDEX、`feature_list.json`、`agent-progress.md`。共享 dirty tree 中其它既有改动均未回滚或覆盖。

### 2026-07-16 · Company stage legacy runtime 删除前 checkpoint

- **本轮目标**：按用户要求，如果仍是兼容分叉就修到对齐；无必要后删除旧逻辑，但任何删除前先 commit 安全 checkpoint。
- **已完成 checkpoint**：对共享工作树 348 files、`+75,875/-9,712` 做了完整范围检查，无 `.env`/凭据/key/运行产物；修复 3 个旧文档 EOF 空行后 `git diff --cached --check` exit 0。
- **删除前门禁**：真实 index 下 `just space-guard && just precommit` 全绿：fmt 3s、check-fe 11s、test-fe 21s、lint-rust 4s、test-rust-all 218s、check-types 50s，最终打印 `━━━ OK ━━━`。未运行 `init.sh`。
- **提交记录**：`5af4f31a feat: checkpoint durable stage team runtime`，未 push。此 commit 成功后才开始 legacy 删除设计。
- **进一步根因**：不是单纯前端兼容。runtime 只在 frozen contract=`v2_only` 时 seed Team；DualWrite/Legacy 会落回 per-org specialist，而 CLI resume 还错误假设每 Unit 恰好一个 Worker，无法恢复拥有 Controller + dynamic child 的合法 Team。
- **DB/rollout 核验**：当前 deployment runtime rollout 已是 `v2_only rank=3 row_version=3`，attack rollout 保持 rank 1；因此本轮不需要也没有修改 `golish-db`、schema、migration、rollout row 或 frozen historical operation。非 V2 company operation 改为 typed rerun-required。
- **实现**：四个 company stages 在 generic specialist loop 前强制检查 Team policy/route，分别返回 `STAGE_TEAM_POLICY_REQUIRED`、`STAGE_TEAM_V2_RERUN_REQUIRED`、`STAGE_TEAM_ROUTE_INVARIANT`；已 final-sealed stage 从 operation-fresh aggregate completion 幂等返回且 `provider_dispatched=false`，不再 reseed/model dispatch。CLI resume 按 exact Team Plan/WorkItem/Worker identity 选择唯一 `leader:primary` Controller，并验证所有 child chain/tool fence。前端删除 `Main Agent -> Specialist`、legacy CollectorCard/CoverageChips；missing/mixed Team pointer 整体 fail closed 为 rerun-required，Candidate/Verification 与后续 typed view 不受影响。
- **红绿过程**：先复现 CLI resume 对多 Worker Unit 的拒绝，再补 Team owner selector；随后复现 completed Enumeration replay 再 seed 时的 `stage_team_dynamic_work_item_authority_mismatch`，改为在 seed 前读取 fresh aggregate pass token。期间修复一个 Rust 临时数组 borrow 编译错误和测试 fixture `created_by=test` 不满足 server-seed authority。第一次 full precommit 还确定性复现 `test_agentic_loop_cancellation_via_timeout` 栈溢出：新增 completion queries 扩大了本来就很大的 dispatch async future；将该 replay future heap-box 后，单测与 8 个相关用例全部转绿。
- **localhost CLI 验收**：fixture `http://127.0.0.1:54761`，workspace `/private/tmp/golish-company-runtime-retirement-20260716-ZFyWai`，session `stage-run-c6331c37-48e9-4ea1-a93c-e2082762c72d`，operation `b63c135a-eee1-46b3-87d1-fb5cc35c38e3`。首次 EAS/Enumeration 已 Company Controller PASS，在 intentional phase boundary 停止；exact `--stage-run-resume ... --resume-to vuln_triage --allow-orphan-running` 退出 0，Enumeration replay PASS、Vuln PASS。
- **DB/run-tree 证据**：`scripts/run_tree.py --full --db` 显示 EAS/Enumeration/Vuln 三个 `mode=company_controller` Team，三个 `leader:primary` Controller 均 passed/final submitter，Enumeration 的一个 dynamic Enumerator child passed；三份 durable submission/handoff 有 exact evidence ids；`selected_read_source=v2 legacy_fallback=forbidden`。本地 HTTP fixture 已停止。
- **最终门禁**：`just space-guard` exit 0；最终 `just precommit` 打印 `━━━ OK ━━━`：fmt 3s、check-fe 10s、test-fe 20s、lint-rust 40s、test-rust-all 294s、check-types 47s。聚焦验证另含 8 个 runtime/CLI tests、24 个前端相关 tests、TypeScript、Biome、Rustfmt 和 `golish + golish-agent-runtime` all-target Clippy。
- **未运行**：按用户要求没有运行 `init.sh`；未修改 DB/schema/migration；未 push。
- **下一步最佳动作**：提交本次 legacy-retirement closeout。父 feature 仍因 Candidate-to-Verification 的更大 DoD 保持 `in_progress`。

### 2026-07-16 · Company Controller terminal progress / compact card 收敛

- **本轮目标**：修复最新 `pentest-chat-1784137594582-1` 中 Target Intel 的 durable Team/Gate 已 PASS，但聊天里的 `Running specialist agents` 仍显示 `0/1 passed · 1 进行` 的矛盾状态；用户明确要求不运行 `init.sh`。
- **根因与证据**：该 transcript 对 request `call_00_Omv441BijaCHd18Ky2nO4536` 只记录 `queued → running` 两帧 `stage_run_org_progress`，随后直接得到 `stage_run {success:true, passed:true, team_units_passed:1}`、deliverable accepted 和 Gate PASS。DB read model 已显示 Unit/Controller Worker passed。当前 Company Controller scheduler 在 success match 里只递增 aggregate passed count，没有发 terminal per-org progress；前端 compact card 又只从最后一帧 progress tally active worker，因此绿勾与“进行”并存。
- **已完成实现**：`stage_run_call.rs` 对每个 runnable Controller result 用同一 exact operation/execution/unit/org/request pointer 发 terminal progress：final-sealed 发 `passed`，non-pass/error 发 `blocked`；保留 DB-backed `StageTeamRunView` 作为 authoritative truth。`tool-handlers.ts` 在 main `stage_run` terminal result 明确 `success=true, passed=true` 时把同 request snapshot 的残留非-passed rows 收敛为 passed，保证旧 transcript event replay。`ToolCallSummary.tsx` 还从已持久化 terminal result 做 render-time fallback，使不重新 replay handler 的旧 session/hot reload 也立即显示 `1/1 passed`、清掉 active/queued/blocked。普通 prose、失败 result 或非-main source均不能推导 PASS。
- **TDD / 已记录证据**：新增 frontend replay regression 后先 RED：`tool-handlers.test.ts` 8 tests 中 1 failed，`upsertStageRunRow` 调用数 0；实现后 focused frontend 5 files 36/36 passed，TypeScript `tsc --noEmit` exit 0，Biome 4 files exit 0。新增 backend terminal event identity regression；`cargo nextest run -p golish-agent-runtime -E 'test(company_controller_)' --status-level fail` → 10/10 passed、407 skipped，run `e6d117cb-e86a-429e-af11-d2bd53e0f44c`。`cargo clippy -p golish-agent-runtime --lib --no-deps -- -D warnings`、`cargo fmt --manifest-path backend/Cargo.toml --all -- --check` 均 exit 0；每次 Cargo 前均执行 `just space-guard`。收尾实际执行 `just space-guard && just precommit`：fmt、check-fe、test-fe、lint-rust、test-rust-all 均 passed，其中全量 Rust 测试耗时 552s；随后 `check-types` exit 1，原因是共享 dirty tree 中既有/并发的生成类型漂移（`AttackCandidateApprovalView.startBefore`、`AttackCandidateReviewDecisionRequest.startBefore`、`GeneratedAiEvent` / `GeneratedHarnessTraceKind` 新 Stage Team / attack consolidation 字段与 ts-rs 空格归一化），不属于本轮 terminal-progress scope。未回退或手改这些 generated IPC 文件。
- **提交记录**：未 commit、未 stage、未 push；未改 DB schema/migration、generated IPC 或外部服务/目标。
- **已知风险 / 未解决**：尚未用 fresh GUI run 生成一条新 terminal progress 做现场验收；现有截图对应的已完成 run 应在前端热更新/重载后由 render-time fallback 立即纠正。父 Stage Team/Candidate→Verification 功能仍包含其它未完成 DoD，因此 `feature_list.json` 继续保持唯一 `in_progress`，不虚报整项 passing。完整 `just precommit` 的唯一未绿步骤是上述跨 scope generated type drift；在其所有者同步/归一化生成文件前，当前共享工作树不能宣称全仓门禁通过。
- **下一步最佳动作**：刷新/重开当前应用并回到旧 Target Intel 工具卡，应看到 `1/1 passed` 且无“进行”；后续新 Company Controller run 的 transcript 还应新增 exact `status:"passed"` terminal progress。若现场仍不一致，先检查该消息块持久化的 `tc.result` 是否保留 `passed:true`，不要修改 Gate/DB truth。
- **以下文件已修改但未提交（本 scope）**：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`frontend/services/ai-events/tool-handlers.ts`、`frontend/services/ai-events/tool-handlers.test.ts`、`frontend/components/AIChatPanel/ToolCallSummary.tsx`、`frontend/components/AIChatPanel/ToolCallSummary.test.ts`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`。共享 dirty tree 其它既有改动均未回滚或覆盖。
