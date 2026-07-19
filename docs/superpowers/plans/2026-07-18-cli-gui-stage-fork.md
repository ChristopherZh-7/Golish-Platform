# CLI/GUI 共享数据库阶段测试分叉实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 新增基于既有 GUI/CLI operation 的安全 stage fork，让 CLI 采用数据库中的 Scoping/前置 final seals并只执行所选阶段，覆盖 Target Intel 至 Attack Candidate。

**架构：** 新 operation 通过原子 `StageForkCreate` 克隆 source 的组织 scope，冻结当前 Target 与严格前缀 handoff lineage；普通前置读取优先当前 operation，否则仅回源到 manifest 指定 authority。Candidate 增加精确的 forked Vuln Wave entry，但仍使用原 Candidate scheduler/Gate/review内核。

**技术栈：** Rust 2021、Clap、Tokio、SQLx/PostgreSQL、Tauri shared app kernel、cargo-nextest。

## 实现校正

- Candidate entry 的 additive 变更单独落在 `20260718000002_candidate_stage_fork_entry.sql`，通用 lineage 与 Candidate 窄 evidence 例外不混在一个 migration。
- Scoping 采用真实 sealed scope authority，不要求不存在的 Scoping Worker handoff。
- typed 入口为 `PreparedTaskOperation::run_stage_fork`；它与 fresh GUI/CLI 一样调用 `TaskOrchestrator::run_stage -> run_from_stage`，只把 source lineage 交给原子 operation create。
- 从 EAS 及以后进入的非终态 fork 冻结已快照 Target 的 identity/scope/source/owner；端口、指纹等 enrichment 仍可更新，fork 终态后解除 identity edit fence。

## 文件结构

- 创建 `backend/crates/golish-db/migrations/20260718000001_operation_stage_forks.sql`：additive fork schema、immutability、Candidate Wave第三种entry和窄 evidence owner约束。
- 创建 `backend/crates/golish-db/src/repo/operation_stage_forks.rs`：source锁定、scope/Target/handoff manifest验证与 adopted predecessor resolver。
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：原子创建 stage-fork operation。
- 修改 `backend/crates/golish-db/src/repo/{stage_handoffs,attack_waves,attack_candidate_work_items,enumeration_surface_manifest}.rs`：adopted predecessor和Candidate fork entry。
- 修改 `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`：typed `StageForkCreate` 与 Candidate Wave fork variant。
- 修改 `backend/crates/golish-agent-app/src/ai/{task_operation.rs,db_bridge/runtime_memory.rs,db_bridge/recon.rs,commands/stage_coverage.rs}`：共享 kernel接线和 adopted reads。
- 修改 `backend/crates/golish/src/{cli/args.rs,stage_run/mod.rs,main.rs}`：CLI fork参数、selector、运行入口。
- 创建/修改 DB、app、golish focused tests；同步模块卡、INDEX、feature/progress。

## 任务 1：先锁定 CLI 与 typed launch 合同

**文件：** `backend/crates/golish/src/cli/args.rs`、`backend/crates/golish/src/main.rs`、`backend/crates/golish/src/stage_run/mod.rs`、`backend/crates/golish-agent-app/src/ai/task_operation.rs`

**步骤：**

1. 添加 parser RED tests：fork必须带 `--only` 或完整 `--from/--to`；拒绝 Scoping、fresh/resume/ephemeral/profile/org/target/subsidiary冲突。
2. 添加 source selector RED tests：operation UUID允许 terminal Task；chat/session 只有唯一 operation才成功。
3. 新增参数和 typed entry：

```rust
pub stage_run_fork: Option<String>;

pub struct StageForkTaskOperationLaunch {
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: StageKind,
    pub terminal_stage: StageKind,
    pub allowlist: HashSet<StageKind>,
    pub adopted_stage_kinds: Vec<StageKind>,
}
```

4. `main.rs` 把 fork路由到现有大栈 stage runner；`run_fork` 调用 `prepare_task_operation`，再由 typed `PreparedTaskOperation::run_stage_fork` 进入共享 `run_stage` 内核。

**验证：**

```bash
cd backend && cargo nextest run -p golish -E 'test(stage_run_fork)'
```

预期 parser/selector/route tests 全绿。

## 任务 2：建立 immutable fork schema与原子创建事务

**文件：** migration、`operation_stage_forks.rs`、`repo/mod.rs`、`runtime_memory_tx.rs`、`db_traits/runtime_memory.rs`、`db_bridge/runtime_memory.rs`、`backend/crates/golish-db/tests/operation_stage_forks.rs`

**步骤：**

1. 写 migration RED integration，要求 header/input/target表、复合 FK、immutable trigger和 Candidate entry XOR存在。
2. 定义 typed create input：

```rust
pub struct StageForkCreate {
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
}
```

3. 扩展 `CreateRuntimeOperation{,Row}` 为 `stage_fork: Option<StageForkCreate>`；规定 `cli_scope` 与 `stage_fork` XOR。
4. 在 `create_runtime_operation` 单一事务内锁 source、创建 target operation、以 `reuse_reconfirmed` 决策冻结同成员 scope、冻结当前 Targets、验证逐org final seals、写 fork manifest。
5. 添加故障注入测试，证明 manifest验证失败不会留下 Task/operation/scope/fork半状态。

**验证：**

```bash
cd backend && cargo nextest run -p golish-db -E 'test(operation_stage_fork)'
```

预期 schema、原子性、workspace/scope和tamper tests 全绿。

## 任务 3：接入通用 adopted predecessor和主动 Target authority

**文件：** `operation_stage_forks.rs`、`stage_handoffs.rs`、`enumeration_surface_manifest.rs`、`db_bridge/runtime_memory.rs`、`db_bridge/recon.rs`、`commands/stage_coverage.rs`

**步骤：**

1. 添加 `resolve_stage_input_authority(target_operation_id, stage, org)`：current final seal优先，否则读取 exact fork input并重验。
2. `load_inherited_stage_handoffs` 返回带 source provenance 的投影；runtime prompt owner校验接受 exact adopted target operation，拒绝任意 foreign operation。
3. Vuln coverage用 resolver读取 source Enumeration handoff origins与source enumeration surface manifest。
4. `active_recon_scope_review_authorized` 增加 fork Target分支：冻结 `scope=in` Target必须全部仍精确属于同 project/org/identity；空主动Target fail closed。

**验证：**

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(stage_fork) | test(final_sealed_enumeration)'
```

预期 adopted/current优先级、Target drift及Vuln inherited surface tests 全绿。

## 任务 4：实现 Candidate ForkedVulnHandoff

**文件：** migration、`attack_waves.rs`、`attack_candidate_work_items.rs`、runtime-memory DTO/bridge、Candidate DB tests。

**步骤：**

1. `attack_wave_units` 添加 `entry_stage_fork_input_id`，将 entry shape扩成 direct handoff / consolidation / fork input严格 XOR。
2. Rust enum增加：

```rust
AttackWaveEntry::ForkedVulnHandoff {
    stage_fork_input_id: Uuid,
    source_operation_id: Uuid,
    source_stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    source_deliverable_submission_id: Uuid,
}
```

3. initial authority在 Candidate-only fork中按 target scope逐org解析 exact Vuln fork input，并要求配对 Enumeration input。
4. manifest materializer从 source operation重算 Enumeration/Vuln连续性、TechniqueOutcomeSet、outcomes和evidence；fork seam替代普通路径的同-operation时间相等，但不放宽普通路径。
5. evidence owner trigger只允许 exact fork input冻结的 observation/support evidence进入seed/work item/Candidate support；Attempt proof/refutation/blocker/fact_delta仍只接受target operation evidence。
6. 添加 source reset/invalidated handoff、truncated watermark、outcome/hash/evidence漂移、foreign evidence tests。

**验证：**

```bash
cd backend && cargo nextest run -p golish-db -E 'test(candidate_stage_fork) | test(attack_wave_entry) | test(exact_formulaic_watermark) | test(foreign_or_non_evidence)'
```

预期 Candidate fork success与所有fail-closed tamper cases全绿。

## 任务 5：CLI 运行接线和共享内核回归

**文件：** `stage_run/mod.rs`、`task_operation.rs`、`orchestrator.rs`及focused tests。

**步骤：**

1. `run_fork` 使用默认 app DB，注册 canonical workspace，校验 source project scope，再创建新 `stage-run-*` session。
2. source profile/provider/model作为默认值；显式 provider/model只改变新operation执行配置，不改变 frozen scope/contracts。
3. 构造 `FreshOperationEntry::StageSlice` + typed fork create；最终仍执行：

```rust
prepared.run_stage_fork(launch).await
// -> TaskOrchestrator::run_stage
// -> TaskOrchestrator::run_from_stage
```

4. 添加 source operation不变、前置stage零dispatch、selected stage新execution/freshness/chain tests。

**验证：**

```bash
cd backend && cargo nextest run -p golish -p golish-agent-app -p golish-agent-kit -E 'test(stage_run_fork) | test(task_operation) | test(run_stage)'
```

预期 fork与既有 fresh/resume共享内核回归全绿。

## 任务 6：定向集成验收与收尾

**文件：** focused integration fixture、模块卡、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`

**步骤：**

1. 创建本地fixture DB中的 GUI-shaped completed source，分别运行 Vuln-only和Candidate-only fork；不发真实外部扫描请求。
2. 断言 source status/row counts/hashes不变，新operation只有选定slice的execution/Worker/tool/evidence。
3. 执行 scoped rustfmt、golish/golish-agent-app/golish-agent-kit/golish-db clippy `-D warnings`、JSON与diff检查。
4. 记录每条命令、退出码和关键输出；只有所有显式要求有新鲜证据后才将 feature改为 `passing`。

**验证：**

```bash
just space-guard
cd backend && cargo clippy -p golish -p golish-agent-app -p golish-agent-kit -p golish-db --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
git diff --check
```

预期全部 exit 0。按 AGENTS.md §0.1 不主动运行 `init.sh`、`just precommit` 或全 workspace tests。
