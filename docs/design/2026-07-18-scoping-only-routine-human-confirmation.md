# 仅 Scoping 保留常规人工确认

> Supersedes only the routine post-Scoping phase-confirmation decisions in
> `docs/design/2026-06-03-two-level-phase-stage-model.md` and the “other phase
> boundaries remain generic approvals” sentence in
> `docs/design/2026-07-16-active-recon-target-scope-confirmation.md`. Phase
> grouping, deterministic Gates, and typed target review remain current.

## 背景

当前 TaskOrchestrator 在每个 stage 的确定性 Gate PASS 后，还会根据 `phases.json`
的 `entry_approval` 在跨大阶段时打开一张通用 confirmation 卡。例如
Enumeration → Vuln Triage 会再次等待人工点击。Scoping 已经负责主体、子公司和可信
目标范围确认；对已经处于同一授权 operation 内的普通阶段流转，再重复要求通用确认，
会让长任务频繁停住。

用户要求：只有 Scoping 保留常规人工确认，之后的阶段在 Gate PASS 后自动继续。

## 决策

1. Scoping 的现有人工确认完全保留：
   - `subsidiary_scope` / `unit_review`；
   - 非空可信目标快照的 `scope_review`；
   - `scope_human_approved` 的确定性 Gate 校验。
2. Scoping 之后不再打开通用 phase-boundary confirmation 卡。阶段 Gate PASS 且没有
   其它专用安全阻断时，graph 自动推进到投影 DAG 的下一 stage。
3. 下列专用安全授权不是“常规阶段确认”，继续保留并 fail closed：
   - Target Intel 扩大主动扫描 denominator 时的 operation-bound 精确目标范围 review；
   - Candidate V2 的 exact review barrier / verification plan approval；
   - 单个高风险工具的 pre-action authorization、scope/owner/fence 校验；
   - 任意确定性 Gate BLOCK、operator recovery 或 outcome-unknown 状态。
4. `phases.json.entry_approval` 和 profile `approval_policy` 继续作为风险/兼容元数据；
   本轮不把它们改成工具授权，也不删除 CLI 兼容参数。当前内置 DAG 的
   Scoping → Target Intel 位于同一个 `prep` phase，因此运行时不会产生额外的通用
   Scoping 出口确认卡；实际唯一常规人工确认仍发生在 Scoping stage 内。

## 运行时落点

`TaskOrchestrator::two_level_phase_gate` 先执行现有专用安全 barrier：

1. Gate BLOCK：不推进；
2. Candidate V2 review barrier：未关闭则 HOLD；
3. Target Intel → EAS exact target review：未授权则 HOLD；
4. 其余 post-Scoping crossing：直接 `Allowed`；
5. 只有 `from_stage == Scoping` 才保留进入 legacy generic phase approval 判定的资格。

这样不改变 DAG、stage gate、evidence ledger、scope snapshot、Candidate 或 tool policy。

## 兼容性

- 不改 DB schema/migration、IPC 或前端事件类型。
- 前端现有 phase confirmation renderer 保留，用于读取历史 transcript 和兼容事件；新流程
  不再从 post-Scoping crossing 发出这类事件。
- `--approve-phase-boundaries` 继续被 CLI 接受，避免脚本破坏；当前内置流程不再需要它来
  批准 post-Scoping 的通用阶段流转。CLI 的 scope/candidate/tool 授权仍按各自 typed policy。
- 已经停在普通 phase confirmation 的历史内存请求不会被重写；重新开始或合法 resume 后
  使用新策略。

## 验证

- 先写回归证明 Enumeration → Vuln Triage 在没有用户回复时立即 `Allowed`，且不发
  `waiting_approval` / `AskHumanRequest`。
- 保留并运行 Target Intel → EAS target-scope fail-closed 测试。
- 保留并运行 Candidate V2 review barrier 测试。
- 运行 `golish-agent-kit` 聚焦 nextest、受影响 crate Clippy、package rustfmt、JSON 与 diff
  检查；按 AGENTS.md §0.1 不运行未获授权的全量门禁。
