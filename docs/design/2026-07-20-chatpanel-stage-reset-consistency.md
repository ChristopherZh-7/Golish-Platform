# ChatPanel 阶段重置一致性设计

## 状态

- 日期：2026-07-20
- 范围：ChatPanel 右下角 dev-only「重置阶段」按钮
- 目标：消除 UI roadmap、operation cursor、V2 relational runtime 与阶段事实之间的半重置状态
- 非目标：修改 schema/migration；原地删除 Candidate、Verification、Post-Exploit、Cleanup 或 Reporting 的 immutable truth

## 现状与根因

当前 `restart_from_stage_purge` 把一次“重置”拆成两个独立提交：

1. `runtime_memory_tx::supersede_stage_checkpoint` 先提交 cursor、active execution、Unit、Worker 与 handoff 变更；
2. command 随后才在第二个事务清理阶段事实。

第二步失败时，operation 已指向新阶段，但事实仍是旧阶段的数据。更常见的是 V2 operation 已用 sealed `operation_org_scope_snapshot` 作为组织 authority，而旧 purge 仍读取 nullable legacy `operation_state.engagement_org_id`；该字段为空时，command 会成功返回 `purged_facts=true`，实际删除数为零。

另外，前端把 Rust 返回的 `affected_stages/current_stage/purge_counts/purge_note` 全部丢弃，未回卷 `stageOrder/passedStages/plansByStage`。旧 real plan 会继续拒绝新一轮的 v0 stage seed，因此数据库即使重置成功，ChatPanel 仍显示旧阶段已经通过。

最后，菜单用线性 index 判断“已经到过”，但 Operation Graph 有 `EAS → Reporting` 等分支。一个从未执行的阶段会因为排在 Reporting 前面而被错误解锁。

## 阶段族结论

| 阶段族 | 原地 destructive reset | 原因 / 路由 |
|---|---|---|
| Scoping | 禁止 | sealed scope、scope decision 与 replay identity 不可变；重测必须创建新 operation |
| Target Intel / EAS / Enumeration / Vuln Triage | 支持 | 现有 plan-first replacement Unit 与 Company Team seed/claim 契约一致 |
| Attack Candidate | 禁止 | passed Candidate Unit、Wave/Candidate/Attempt authority 不可变；使用 operation stage fork |
| Verification | 禁止 | replacement 必须来自 Wave subset/generation 且 typed handoff 不可原地清空；待扩展 fork protocol |
| Post-Exploit / Cleanup | 禁止 | canonical action、obligation、waiver/decision 是保留历史的安全 truth |
| Reporting | 禁止 | report revision 是 operation-scoped immutable history；普通 reset 会复用旧 validated revision |

本轮不扩大删除清单去“支持”后期阶段。那会破坏 evidence lineage，并且多个表用 `ON DELETE RESTRICT`/immutable trigger 明确拒绝这种语义。

## 安全重置协议

### 1. 后端 fail closed preflight

`restart_from_stage_purge` 只接受四个 Company stage，且：

- operation 必须是 V2Only 且已有 sealed frozen scope；
- current cursor 也必须仍在 Company stage；进入 Candidate/Reporting 等 immutable 边界后不得原地倒回；
- selected stage 必须在该 operation 的 `stage_runs` 历史中真实到达，不能仅凭 DAG reachability；
- selected stage 必须是 current stage 或 current 的真实 DAG ancestor，历史上曾到达但当前位于其祖先的 stage 不能被当作“向前跳”目标；
- operation 历史只能包含 Scoping 与四个 Company stage；未知 stage 名或任一 immutable stage 历史都 fail closed；
- unsupported stage 在任何 mutation 前返回明确 validation error。

其他 dev reset mode 保持既有窄用途；ChatPanel 只调用上述 full reset mode。

### 2. 单事务 compound reset

command 根据 embedded stage specs 计算：

- affected DAG stage kinds；
- stable technique union；
- 旧四 domain purge flags；
- selected stage 的 target status floor。

这些值组成不含 harness enum 的 `StageCheckpointPurgePlan`，交给 `golish-db` compound repository。repository 在锁住 `operation_state` 后，从 sealed `operation_org_scope_snapshot` 加载 exact frozen organization IDs（不重新展开 live subtree），锁住这些 live organization rows 并验证集合仍完整。若另一个 `created|running|waiting` operation 的 sealed scope 与该集合有任何交集，reset 在 mutation 前返回 `stage_checkpoint_reset_overlapping_active_operation`，避免 organization-owned mutable facts 被两个 live operation 同时解释。之后在同一 transaction 中完成：

1. 若受影响 Worker 仍有 `received|running` active tool，则在任何 mutation 前拒绝 reset；只把已经没有外部落库可能的 Worker/Unit supersede；
2. invalidate generic handoff、关闭旧 execution、建立 replacement execution/Units；
3. 清理 Company facts、completion/wave ledger、exact technique outcomes、status floor；
4. 写入 cursor/state blob 与 reset marker；
5. commit。

任一步失败，全部回滚。nullable legacy `engagement_org_id` 不再决定 purge scope；若它有值但与 frozen root 不一致则 fail closed。

reset 不能只把 active tool 的数据库行改成 failed 来假装外部进程已停止：EAS/Enumeration 工具可能在 reset commit 后继续 landing，再次写回刚删除的事实。repository 因此先以完整 Worker identity（含 exact `lease_token`）核对 active pointer；同一 identity 的 `received|running` tool 返回 `stage_checkpoint_reset_active_tool_in_flight`，要求先等待完成或走显式 stop/recovery，再重试 reset。旧 lease、错 Worker、错 epoch或未知 tool row继续 fail closed；同一 identity 已 terminal 的陈旧 pointer才允许随 Worker supersede收敛。

### 3. 可证明 ownership 的事实清理

清理矩阵按数据实际 ownership 分层，而不是用“属于某阶段”的直觉扩大删除：

- organization-owned mutable Company facts 只作用于 frozen scope；因为这类表没有 operation key，所以与该 scope 重叠的 active operation 会阻断 reset；
- EAS 会显式删除 frozen organizations 的 `web_origin_observations → web_origins → network_endpoints`，Enumeration 会经 `origin_target_id → targets.organization_id` 删除 `crawl_observations`；否则 Gate freshness可能已经回卷，但通用 surface/origin/crawl read model仍会显示上一轮数据；
- `source_query_log`、`expansion_queue`、`technique_outcomes` 必须同时按 frozen organization 与当前 operation 的两个可信 run alias 过滤：operation UUID 与其 Task session UUID；历史/兄弟 run 保留；
- screenshots 只按 exact operation id 清理；completion按 frozen organization + affected stage清理，asset wave与runtime row再带 exact operation/stage authority；
- `findings`、`vuln_scan_history` 与 legacy sensitive-scan 数据没有足够可靠的 current-operation ownership，其中 Finding 还可能是 manual 或 immutable Candidate/Verification truth，因此一律保留；Vuln 重置只清 current-run technique outcomes，不伪造更宽的事实 ownership；
- `audit_log`、evidence、transcript 与 immutable runtime history始终保留。
- `targets`/organizations spine始终保留。尤其 Target Intel 发现并可能被人工提升为 customer scope 的 Target缺少可靠 operation provenance；reset会清它的可重建 intel facts/status，但新一轮 denominator从当前 durable target catalog开始，不承诺还原到最初 stage-entry 的逐字节快照。

### 4. stage-owned state blob namespace

V2 reset 除既有 legacy checkpoint namespace 外，还按 affected stages 删除：

- Target Intel：`active_recon_target_scope`
- EAS：`eas_web_transport_failures`
- Enumeration：`route_probe_checkpoints`

这些是阶段执行 authority/cache，不应跨 rerun epoch 保留。其他 sibling state 保留。

### 5. receipt-driven frontend rewind

TS wrapper 返回 ts-rs 生成的 reset receipt。backend mutation 一旦返回即视为 committed；前端即使发现 receipt 字段损坏，也不能把已经提交的 reset 误报成“未重置”。收到回执后、发送“继续跑”前：

- 验证 `mode/stage/currentStage/affectedStages/refreshedStageCursor/resetGraphFlow/purgedFacts/purgeScopeOrgCount/purgeCounts/purgeNote` 的完整 committed contract；未知 stage 名不进入本地清理集合；
- 从 `plansByStage` 删除可信 `affectedStages`，并立即为 selected stage 写入新的 v0 `in_progress` seed；不能只删除旧 plan 等待模型将来补 seed，否则自动继续失败时 roadmap 会消失；
- 从 `stageOrder` 与 `passedStages` 删除 `affectedStages`；
- 只选择一个真实存在、与事件路由优先级一致的 canonical roadmap session owner，不创建 AI/terminal alias session；conversation localStorage用自己的 `stageOrder`再计算一次 suffix，与内存 affected集合取并集后同步清理，防旧后继阶段刷新后复活；
- 再发送可见的“继续跑”。reset 期间 textarea、mode/model selector、附件和普通 send 共用一个互斥 gate；只有该 reset owner 的 auto-resume 能显式穿过 gate，避免用户消息或模式切换与 roadmap rewind 竞态。
- reset commit到auto-resume结束前同时冻结 conversation select/new/close/history，并在继续前复核原 conversation仍是active，避免 A会话reset把B会话的execution profile改掉。

菜单的可选性改为：

- stage 必须属于四个 supported Company stages；
- current stage 必须属于同一 supported family；
- stage 必须是 current 或出现在 `passedStages` 中。

`currentStage=null`、未知 current、从未 passed 的分支 stage、finished task 与 immutable stage 全部禁用，并展示原因。

## 事务与证据不变量

- evidence/audit log 永远保留；reset 只清 current mutable Company facts 与可重建 ledger。
- frozen scope 是 purge authority；不退回 live org subtree 或 nullable legacy binding。
- 无 operation ownership 的 Finding/Vuln history 不删；run-owned 行只认 current operation/session aliases。
- reset receipt 只有在 compound transaction commit 后返回；`purgedFacts=true` 必须同时有非空 frozen scope。
- state blob、facts、runtime rows、graph flow 与 cursor 必须同事务提交或同事务回滚。
- UI 状态只在 backend success 后 rewind；backend failure 不改变 roadmap。backend 已 commit 后的 malformed receipt 或 auto-resume failure 不得逆向声称数据库 reset 回滚。
- resume 仍是后续独立动作；如果自动 resume 失败，UI 保持已回卷状态并把“继续跑”留在输入框，用户可以重试，不伪装成 reset 失败回滚。

## 后续扩展

- Scoping：新建 fresh operation。
- Target Intel 至 Candidate 的 immutable 重测优先复用现有 `operation_stage_forks`，而不是扩大 destructive purge。
- Verification 以后阶段需先扩展 fork authority，再开放 UI；Reporting 若需要“重新生成”必须定义 force-new-revision epoch，不能复用普通 checkpoint reset。
