# Investigation 详情读取双身份契约

> Supersedes the read-routing decision in `2026-08-11-investigation-stage-run-detail-identity.md`.

## 问题

Investigation 卡片同时涉及两组不能混用的身份：

- UI 选择身份：终端 pane session 与真实外层 `call_...`；
- 服务端 authority：conversation 的 `aiSessionId` 与稳定的 `stage_run:<stage_execution_id>`。

实体 operation `9ac7715d-6957-483a-94fd-e0a2385c866d` 已完成 Investigation 和 Reporting，但详情 IPC trace `5d037b23` 被拒。前端传入终端 session UUID `85fdc067-...`，operation 实际绑定 `pentest-chat-1786441282794-1`。即使修正 session，retained `investigation_run_heads` 的 exact owner 仍是 `stage_run:a1467779-...`，不是卡片的 `call_00_Oy...`。

## 决策

1. `ToolCallDetailView` 用 `conversationTerminals` 从 pane session 精确找到 conversation，并只把该 conversation 的 `aiSessionId` 送入 Investigation IPC；找不到映射时 fail closed。
2. outer `call_...` 继续负责选择卡片、恢复 timeline 和展示；不得拿它冒充 DB authority。
3. 已知 exact `operation_id + stage_execution_id` 后，读取 authority request 固定构造为 `stage_run:<stage_execution_id>`，与 runtime 的 durable identity helper一致。
4. resolver 仍校验 args/result/progress 中的 outer request 是否彼此冲突；发生冲突时不挂载详情。
5. 不按 current stage、时间或 latest head 推断，不改后端授权，不扩大跨 session/workspace 读取。
6. route 同时保留两个 session key：`aiSessionId` 只用于后端 exact read/stop authority，pane session 只用于读取前端实时 transcript 与 refresh hint。Zustand selector 的 absent 集合必须返回模块级稳定空值，不能在 selector 内创建新的 `[]`，否则 React 19 会把每次 snapshot 视为变化并无限重渲染。
7. stage-run exact summary 的 Main actor 只能是统一 Investigation Primary：`kind=investigation_primary`、`stable_key=leader:primary`、`role=investigation`、`created_by=server_seed`，并绑定同 operation/execution/unit/scope/root organization 下的最新 Worker。`company_stage_controller` 是其他阶段的角色，不得用它读取 Investigation Main。
8. 授权区分 materialized read 与 control/mutation。进程重启后，`TaskStatus::Finished` 的历史 Investigation 可在无内存 `AgentBridge` 时读取，但仍必须同时满足 current LocalDesktop principal、DB current-local principal、exact operation→Task→session key、active project scope、sealed frozen root scope。非终态读取、Stop 与 Prepared Action/control 继续必须有 live bridge 与 exact workspace；如果 bridge 存在但 workspace 不等，即使 Task 已完成也不得 fallback。
9. Registry read authority 必须接受 Candidate writer 自己生成的完整负向 managed-feed authority。仅当 operation 没有 live managed-feed contract，且 snapshot 同时具备 exact 5-member denominator、逐成员 `unavailable` snapshot binding 与逐成员 `feed_refresh` obligation 时，负向 authority 才等价于 live contract witness；缺任一 member、binding 或 obligation 继续 fail closed。它表示“本地 feed 未安装且已显式封存”，不是跳过 feed 校验。
10. 已 `closed|abandoned` 的 exact stage run 可以在 authority TTL 过期后作审计只读，但读取时间必须冻结为该 run 的 latest exact terminal event，并且 terminal event 必须不晚于 frozen authority 的最早 `effective_valid_until`。该快照标记为 `temporally_stale`；running/nonterminal、缺 terminal event、terminal 晚于 expiry 或 admission 仍开放时继续返回 stale。operation-level current read、Stop、Prepared Action/JIT 与任何 mutation 不得借用此历史通道。
11. projection head `envelope.changeSeq` 与 Investigation run-control head `controlProjection.changeSeq` 是两个独立单调域，不要求相等。前者只约束 summary/list/detail/timeline snapshot 与 refresh；后者只与 `investigationRunStateHead` 一起约束 Stop CAS。页面可以展示 outer call，但 exact identity 只比较 operation/execution/durable stage-run request，绝不能因两个合法序号不同而拒绝 summary，也不能用 projection seq 发 Stop。
12. Hypothesis projection reader 必须同时识别两个已经被同一 `HypothesisProjectionRecordV1` 冻结的生产形状：完整 read-model body，以及 Investigation compiler 实际写出的 compiler body。compiler body 只在 `origin_authority=investigation_compiler`、proposal/semantic key/readiness exact 一致、proof chunk 身份与 source hash 合法时兼容；`target_type_at_time=subject_identity_hash` 与 target value 只从同一冻结 semantic subject 推导，proof source role按闭集转换。未知字段/role、重复 predicate key、foreign org或模糊 scope继续 fail closed。stage-run list/detail 的 scope join必须绑定 selector 的 exact `scope_snapshot_id`，不得匹配同 operation 的其他历史 snapshot。
13. Main 与 Analysis Primary 在当前统一 runtime 中可以是同一个 durable actor：只有 `worker_run_id + transcript_request_id + organization_id + owning_stage_run_request_id` 四轴完全一致，且 Primary 不属于任何 Hypothesis 时，presentation adapter 才把它规范化为一个可点击 Main，并在 Analysis Task 下显示不可点击的 Main-owned alias。其余重复 transcript 继续 fail closed，不能用泛化去重掩盖冲突。
14. Investigation rail 与 transcript 默认面向人阅读：`passed` 是成功终态并向父 Subtask/Task 收敛；Hypothesis rail 使用 frozen predicate schema 的短标题，完整 canonical predicate 留在详情 disclosure；Agent transcript 按 entry 顺序显示正文、thinking、最新合法 plan 与折叠 tool activity，terminal `submit_result` JSON 不得作为 assistant prose 重复铺开。closed run 不显示不可执行的 Stop/Reset/Fork，未选中的 Hypothesis 不展开整棵 Verification Task。
15. `Bounded read session` 是 Main 的 immutable context/methodology/omission snapshot authority，不是独立 Agent。rail 只把它画成不可点击的 context 节点；它不得进入 selectable actor/transcript identity 集合，也不得因为本地 store没有同名 Agent而显示虚假的 transcript unavailable。

## 验收

- terminal session 与 aiSessionId 不同时，route 发出的 IPC 使用 aiSessionId。
- 页面显示 outer call，但 summary/hypothesis/campaign/timeline 读取使用 stable authority request。
- operation/execution/outer request 任一冲突继续 unavailable。
- aiSession 与 pane session 不同时，详情仍能读取 pane 的实时 Agent transcript/refresh；缺少 live Agent 集合时页面稳定显示空态，不发生 `Maximum update depth exceeded`。
- retained Investigation 已完成 Main 为 `investigation_primary/leader:primary` 时，summary 能精确找到唯一 actor；不会因错用 Company Controller 角色而返回 `investigation authority is inconsistent`。
- 已完成的历史 Task 在 Tauri/backend 重启、bridge 已释放后仍能打开只读详情；同形的非终态 Task 和所有 control/mutation 仍返回 forbidden。
- retained Candidate snapshot 使用 canonical 5-member unavailable feed census 时 summary 可读；少一个 obligation 或任一 exact member 仍返回 authority corrupt。
- 已在 authority 有效期内终结的 retained Investigation 即使当前时间已超过 TTL，仍可按 exact terminal event 打开完整 summary/list/detail/timeline；活动 run 和过期后才终结的 run 仍拒绝，页面 authority-time 明确标为 `temporally_stale`。
- retained summary 的 projection `changeSeq=14` 与 run-control `changeSeq=2` 可同时成立；详情正常打开，分页绑定14，Stop绑定2及其 exact run-state head。
- retained compiler projection 的 4 条 Hypothesis 均能在同一 terminal snapshot 列出；点击任一 revision 可读取 lineage、proof refs、verification objective 与 exact actor topology。未知 compiler proof role继续拒绝。
- retained Main/Analysis Primary alias 只出现一个 transcript owner，点击 Main 不再显示 identity conflict；所有 passed Worker 的 Subtask 不再显示 running。
- 默认 rail 不铺开四棵 Verification Task，canonical predicate 与 submit_result JSON 不占据首屏；closed run只显示终态，不显示无效控制按钮。
- focused Vitest、Biome、typecheck、JSON/diff 检查通过。
