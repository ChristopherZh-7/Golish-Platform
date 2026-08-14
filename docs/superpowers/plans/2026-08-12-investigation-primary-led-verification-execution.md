# Investigation Primary-led verification execution 实现计划

> Superseded for orchestration topology by
> `2026-08-13-investigation-company-asset-queues.md`. Retain the completed execution-assignment,
> Oracle, FactDelta, evolution and CyberStrike slices as reusable implementation foundations.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 Investigation 的假设分析保持只读，同时由每个 VerificationTask 的持久 Primary 动态协调宽角色执行真实、授权且证据化的验证，并把新事实送回持续假设循环。

**架构：** 保留统一 Investigation、Registry、Campaign、Prepared Action/JIT、Oracle 与 FactDelta。新增 operation-frozen actor contract，把 Analysis cognition、Verification coordination、Verification reasoning 和 Verification execution 分开；工具可见性与调用守卫从 exact durable WorkItem execution binding 派生，而不是从 StageKind 或角色名推断。

**技术栈：** Rust 2021、Tokio、rig、embedded PostgreSQL/sqlx、Tauri 2、React/TypeScript；测试使用 cargo-nextest、Vitest、Biome。

## 实现进度（2026-08-12）

- Task 1 已完成：closed actor contract、role-name不得授权、exact execution allowlist与定向测试。
- Task 2 已完成：Analysis/Verification Primary合同分离、同一Primary history、全child synthesis、typed零hypothesis与正常DB seal/admission。
- Task 3 已完成：additive immutable execution assignment、exact materialize/claim/heartbeat/checkpoint/terminal/recovery authority与Worker双fence已落地。
- Task 4 已完成：Verification Primary按sealed intent的宽角色动态创建execution WorkItem，park/resume同一Primary，并把exact assignment绑定给执行Worker。
- Task 5 已完成首个治理闭环：directory四次真实loopback HTTP、capability receipt、DB重算Oracle、Campaign material FactDelta、pending evolution authority、Evolution Analysis Primary rearm及successor/fixed-point compiler均已实现并通过同一受控桥接验收。其它browser/CLI/script/PoC adapter是后续能力扩展。
- Task 6 已完成source-copy/manifest/trust/bounded retrieval/bundle slice；execution仍且只经过Task 3/4授权链。
- Task 7 已完成：controlled HTTP/assignment/receipt/Oracle/Campaign/FactDelta/adjudication/pending-evolution/replay、successor/fixed-point embedded-DB、动态宽角色、零假设、fail-closed负例、scoped Clippy与前端定向门禁均已通过；feature可据此标记`passing`。

## 文件结构

- `backend/crates/golish-agent-kit/src/db_traits/unified_investigation.rs`：SQLx-free actor/task/execution contract DTO 与 repository port。
- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`：VerificationTask Primary/worker claim、execution binding 和 recovery DTO。
- `backend/crates/golish-db/migrations/<timestamp>_investigation_actor_execution_contract.sql`：additive frozen contract、assignment、action/evidence fence（仅在现有表不能表达时创建）。
- `backend/crates/golish-db/src/repo/unified_investigation_runtime.rs`：actor contract、task plan、refiner和execution assignment持久化。
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：claim/lease/checkpoint/tool lifecycle compound mutations。
- `backend/crates/golish-agent-app/src/ai/db_bridge/{unified_investigation,runtime_memory,investigation_analysis_host,verification_campaign_scheduler,verification_send_authority}.rs`：SQL实现映射和Prepared Action/JIT/Oracle桥接。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call,stage_team_scheduler,sub_agent_call}.rs`：Primary任务循环、动态派工、actor-bound工具面和连续上下文。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`：depth-0 Investigation Coordinator闭集；不再以StageKind全局覆盖所有actor。
- `backend/crates/golish-sub-agents/src/{executor_types.rs,executor/tool_setup.rs,executor/prompt_assembly.rs,defaults/prompts/orchestration.rs}`：执行绑定、terminal schema、宽角色提示与工具面。
- `resources/harness/stages/investigation/spec.json`：新operation的actor-contract版本与策略元数据。
- `frontend/components/Engagement/InvestigationWorkspaceView.tsx`：只在后端投影需要时增加execution/blocked/new-hypothesis状态展示。
- `docs/modules/**`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`：单一事实源和证据。

## Task 1：冻结 actor contract 词汇并保持旧 operation fail-closed

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor_types.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`
- 测试：上述文件内 `#[cfg(test)]`

**步骤：**

1. 先写失败测试，表达四种闭集 actor contract：`analysis_primary`、`analysis_worker`、`verification_primary`、`verification_worker`；缺 binding 或 role/name-only 一律得到只读。
2. 运行：

   ```bash
   cd backend && just space-guard && cargo nextest run -p golish-agent-runtime -E 'test(investigation_actor_contract_)' --status-level fail
   ```

   预期：测试因 actor contract 类型/派生函数缺失而失败。
3. 增加 host-only `InvestigationActorContract` 与 exact binding；先不改变旧 frozen operation 的 cognition-only行为。
4. 重跑同一 selector，预期全部通过。
5. 运行 scoped Clippy：

   ```bash
   cd backend && just space-guard && cargo clippy -p golish-agent-runtime -p golish-sub-agents --lib --no-deps -- -D warnings
   ```

## Task 2：分离 Analysis 与 Verification 的 Primary/child durable contract

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- 修改：`backend/crates/golish-db/src/repo/unified_investigation_runtime.rs`
- 修改：`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 测试：`backend/crates/golish-db/tests/unified_investigation_topology.rs`

**步骤：**

1. 写 RED：Analysis child只能冻结read-only schema；Verification Primary的WorkItem identity与Analysis Primary不同；Verification child必须带task/assignment actor contract，不能只凭role获得execution。
2. 把`analysis_task`与`verification_task`的request kind/output contract分开，exact resume重验相同contract hash。
3. Primary每完成一个child都在同一durable chain上Refiner，不reset history。
4. final synthesis输入保留全部child output identity和proposal/action/residual对象；零hypothesis有合法sealed结果。
5. 定向运行：

   ```bash
   cd backend && just space-guard && cargo nextest run -p golish-agent-runtime -E 'test(investigation_primary_) | test(investigation_verification_)' --status-level fail
   cd backend && just space-guard && cargo nextest run -p golish-db --test unified_investigation_topology --status-level fail
   ```

## Task 3：建立 Prepared Action → execution assignment 权威链

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/db_traits/unified_investigation.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 可选新增：`backend/crates/golish-db/migrations/<timestamp>_investigation_actor_execution_contract.sql`
- 修改：`backend/crates/golish-db/src/repo/unified_investigation_runtime.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/verification_campaign_scheduler.rs`
- 测试：`backend/crates/golish-db/tests/unified_investigation_topology.rs`

**步骤：**

1. 写RED，证明未授权/过期/foreign scope/role-only assignment均不能claim execution worker。
2. 定义immutable assignment，绑定operation/stage/unit/org/hypothesis/task/Campaign/objective/PreparedAction/JIT/tool allowlist/canonical args/budget/conflict/lease/evidence contract。
3. 在一个短事务中从current PreparedAction/JIT truth生成或重放assignment；外部I/O不进事务。
4. 写claim/heartbeat/terminal/recovery compound seam；unknown external outcome保持held。
5. 运行embedded-PG focused正反例；如果新增migration，另跑fresh apply与checksum测试。

## Task 4：让 Verification Primary 动态派发执行型宽角色

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- 修改：`backend/crates/golish-sub-agents/src/defaults/prompts/orchestration.rs`

**步骤：**

1. 写RED：Verification Primary只看到plan/dispatch/evidence，execution worker按assignment看到exact工具；Analysis和role-only worker仍看不到。
2. Primary从unresolved assignments动态选择`pentester|browser|coder|researcher|installer|memorist|adviser`，不要求固定名单。
3. execution worker result允许fresh evidence/fact refs，必须等于本Worker新落ledger集合；cognition result继续强制空。
4. Coder脚本只能写task-scoped artifact目录；执行脚本重新走assignment和tool lifecycle。
5. 运行runtime/sub-agent focused nextest与scoped Clippy。

## Task 5：连接真实工具、Oracle、FactDelta 与新假设信号

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/verification_send_authority.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/verification_campaign_scheduler.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/investigation_analysis_host.rs`
- 修改：`backend/crates/golish-agent-kit/src/tool_executors/security.rs`
- 修改：相关 `golish-pentest-app` adapters

**步骤：**

1. 先用loopback fixture为HTTP/browser/CLI/script至少两类adapter写RED。
2. 把host direct-send改为assignment-bound worker tool；保留send-before-begin、JIT、budget、conflict-key、cancellation和unknown-outcome语义。
3. terminal execution evidence进入typed Oracle；仅Oracle可输出verified/refuted/inconclusive/blocked。
4. material evidence可产生typed `HypothesisSignal`，Registry reducer负责revision/derive/split/merge和successor generation admission。
5. 运行focused adapter、Campaign、Oracle、FactDelta和Registry测试；不访问真实外部目标。

## Task 6：接入本地 Skills methodology provider

**文件：**

- 新增：`resources/methodology/corpora/cyberstrike/{LICENSE,README.md,package.json,GOLISH_PROVENANCE.json,skills/**}`
- 修改/新增：Golish现有methodology corpus importer/provider与tests
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 完整复制用户提供的`.cyberstrike/skill`源树、LICENSE、README和package manifest，不改原文；记录无Git metadata事实和deterministic tree hash。
2. 写RED，覆盖exact file/SKILL count、tree hash、AGPL license/provenance、rich frontmatter归一化、metadata-trigger/prerequisite/chain检索与full-methodology按需加载。
3. 生成content-addressed corpus manifest；原始文件保持独立third-party subtree，普通workspace Skill discovery不得自动注入它。
4. 增加显式bundled-corpus trust policy，只接受该exact content-root/license/provenance tuple；hash/license/path漂移fail closed。
5. Analysis只读metadata/What-to-check；Verification reasoning按ref/hash读取bounded原文excerpt与methodology/evidence requirements。
6. 任何hit保持`instruction_authority=false`且不能进入proof refs；Skill中的命令只有映射到已授权Golish execution assignment后才可执行。
7. license/manifest/path/symlink/TOCTOU检查保持fail-closed，并记录分发需保留AGPL source/license。

## Task 7：投影、文档与受控验收

**文件：**

- 按实际改动更新 `docs/modules/**` 与 `docs/modules/INDEX.md`
- 按需要修改前端Investigation projection和focused tests
- 更新 `feature_list.json` 与 `agent-progress.md`

**步骤：**

1. UI区分proposed/strategy_ready/verified/refuted/inconclusive/blocked，并显示执行工具/evidence/new hypothesis lineage。
2. controlled loopback run证明Primary动态多角色、至少一种真实工具调用、ledger、Oracle和successor hypothesis loop。
3. 运行受影响crate nextest、scoped Clippy/rustfmt、focused Vitest/Biome/typecheck、JSON/diff checks。
4. 未经用户另行要求不运行`init.sh`、`just precommit`或全workspace测试。
5. 只有acceptance全部有新鲜证据才把feature改为`passing`；否则保持`in_progress`并记录精确下一步。

## 计划自检

- 规格覆盖：Primary-led Analysis、Primary-led Verification、动态宽角色、真实工具、Skills、持续假设循环、evidence/Oracle、安全授权和恢复均有对应Task。
- 类型一致性：actor contract、execution assignment、HypothesisSignal在全文使用同一命名；Prepared Action/JIT仍是执行前权威。
- 范围：分7个可独立验证的slice，当前从Task 1开始；不把外部Skills vendoring或真实目标扫描混入基础合同。
- 验证：每个Rust构建/测试前均先`just space-guard`；全量门禁保持用户显式授权。
