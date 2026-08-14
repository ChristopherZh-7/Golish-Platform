# Investigation 详情读取双身份实现计划

## 任务 1：锁定 RED

在 `ToolCallDetailView.investigation.test.tsx` 建立 terminal session、conversation、aiSession 三者不同的 fixture，断言 production route 收到 aiSessionId 与 `stage_run:<execution>` authority，同时 outer request 仍用于展示选择。

## 任务 2：最小实现

在 `ToolCallDetailView.tsx`：

1. 从 `conversationTerminals` 精确解析 pane 所属 conversation 的 aiSessionId；无映射时 fail closed。
2. 把 outer request conflict validation 与 durable authority request 分开。
3. exact execution 使用 `stage_run:<stage_execution_id>`；live-only 继续只显示当前 actor，不做 DB read。
4. `InvestigationWorkspaceRoute` 将后端 authority session 与前端 pane projection session 分开；所有 absent store 集合复用稳定 sentinel，禁止 selector 返回新建数组。

不修改 generated DTO、Tauri command、DB schema、Gate 或 evidence。

先用一个 aiSession/paneSession 不同且 aiSession 下没有 `activeSubAgents` 的 route fixture 锁定 React 19 `getSnapshot should be cached` / maximum-depth RED，再转绿。

## 任务 3：定向验收

运行 focused Vitest、两文件 Biome、`pnpm typecheck`、JSON/唯一 in-progress/diff checks；将命令、退出码与实体 trace写入 `agent-progress.md`。

## 任务 4：闭合 retained Investigation Main authority

1. 用只读 retained DB 核对 operation/execution/scope 下的真实 Main WorkItem 和最新 Worker。
2. 建立 focused RED，禁止 summary query 使用 `company_stage_controller`，并要求统一 Investigation Primary 的四个 exact identity 字段。
3. 只修正 `investigation_projection::summary` 的 Main actor 查询，不改 scope/session 授权、projection 数据或历史 operation。
4. 跑 `golish-db` 精确单测、scoped clippy、rustfmt 与 diff check；等 `just dev` 热重载后复用原 Investigation 卡片验证，不重跑全链。

## 任务 5：重启后的历史只读授权

1. 以新 trace `39598aa0` 和 retained `TaskStatus::Finished` 证明 Main actor 已越过，当前拒绝是重启后 live bridge 不存在。
2. 拆分 read-only 与 control authorizer；共享 principal/session/project/sealed-scope 核验，只对 finished Task + missing bridge 开放历史只读分支。
3. 保留 running/waiting/no-bridge forbidden、live bridge workspace mismatch forbidden，Stop/Prepared Action 继续走 strict live-bridge authorizer。
4. 跑 embedded-PG exact authorization regression、`golish-agent-app` scoped clippy、rustfmt/diff/JSON，再等 dev binary 热重载。

## 任务 6：闭合 sealed unavailable managed-feed read authority

1. 对 retained operation 只读分解 `ensure_registry_authority_exact_on`，逐项核对 generation、Candidate snapshot、root bundle、temporal census、target epoch 与 feed authority，定位唯一 false predicate。
2. 保留 live operation contract/current catalog-head 路径；仅新增 Candidate writer canonical 负向 witness：5 个 denominator members、5 个逐项 exact `unavailable` snapshot members、5 个对应 `feed_refresh` obligations。
3. 用 embedded Postgres 正例证明完整负向 witness可读，用少一个 obligation及既有 root/epoch/live-contract omissions证明仍 fail closed。
4. 对同一 retained DB 直接调用完整 stage-run summary，确认返回真实 Main、generation、hypotheses、source census 与 actor topology；重启 Tauri dev backend，禁止只依赖旧热加载进程。
5. 跑两个 focused DB tests、`golish-db` scoped Clippy、rustfmt/diff/JSON；不跑全仓门禁或外部扫描。

## 任务 7：终态历史快照与 authority TTL

1. 用 retained operation 的 exact head event 与 Candidate authority bundle 只读证明 run 在 TTL 内已 `closed`，当前 bootstrap 仅因 wall clock 晚于 TTL 永久 stale。
2. operation-level/current read 保留严格 expiry；stage-run bootstrap 先读取 exact run head，只在 `closed|abandoned`、admission closed、latest terminal event exact 且 terminal time 不晚于 expiry 时，把 read cutoff 冻结为 terminal event。
3. continuation 必须复用同一个 terminal cutoff，client 不能把历史 cutoff 改成更早时间；authority-time 以 `temporally_stale` 呈现，control/mutation authority 不变。
4. 跑 pure policy 正反例、既有 current-first-page expiry 回归、agent-app historical label 回归、受影响 crate Clippy，并直接对 retained DB 调完整 stage-run summary；不重跑实体扫描。

## 任务 8：分离 projection 与 run-control change sequence

1. 用 retained summary 证明 projection head seq 与 unified run head seq 属于不同表/不同 CAS 域，建立 `10 != 2` 的 Route/View regression。
2. summary exact identity 只核对 operation/execution/durable request；projection continuation继续绑定 envelope seq，control view另保留 run seq。
3. Stop request只提交 control seq + run-state head，不能提交 projection seq；refresh hint继续只和 projection seq比较。
4. 跑 Route/View focused Vitest、受影响文件 Biome、`pnpm typecheck` 与 diff/JSON检查。

## 任务 9：兼容 production Investigation compiler Hypothesis projection

1. 对 retained terminal snapshot 直接调用 summary 后的 `list_investigation_hypotheses_for_stage_run`，保留未映射的 repository 原始错误；只读核对 4 条 entity、exact org 与 frozen body key set。
2. 将完整 read-model body 与 compiler body建为两个 closed typed wire shape；compiler shape只从 frozen semantic key/proposal/proof chunks规范化 read DTO，未知字段、proof role、重复 predicate key fail closed。
3. stage-run list/lineage query显式绑定 exact scope snapshot；operation-level兼容读取保持原行为，不选择 latest snapshot。
4. 用纯解析正反例、retained list 4/4 + first detail实体只读、scoped Clippy、rustfmt/diff/JSON验收。旧 `investigation_read_model` fixture若在进入读取前被 rollout adoption门禁阻断，记录为独立 fixture blocker，不把它当本修复失败。

## 任务 10：收敛 retained Investigation presentation

1. 用 retained summary 的 Main/Analysis Primary 同 worker+transcript 形状建立前端回归；只允许四轴 exact 的非-Hypothesis Primary alias 折叠为 Main-owned Analysis Task，其余重复仍拒绝。
2. 建立 `passed` Worker 的父 Subtask/Task终态回归，统一成功/阻塞/运行状态闭集与颜色。
3. Investigation transcript 复用现有 plan、Agent message 与 Codex-style tool disclosure primitives，按 entries 顺序渲染；terminal machine response只保留在折叠 raw tool data，不重复当正文。
4. rail 用短 Hypothesis 标题、仅展开当前选择的 Verification Task；closed run隐藏不可执行 control，完整 identity/predicate保留在 disclosure。
5. 跑 Route/View focused Vitest、受影响文件 Biome、`pnpm typecheck` 与 JSON/diff检查，不运行全仓门禁。

## 任务 11：区分 read authority 与 Agent transcript

1. 建立回归，证明 `Bounded read session` 仍在组织树中可见，但没有 transcript 按钮且不进入 actor selection。
2. 将 read snapshot 渲染成不可点击 context 节点；Main/Primary/Worker/Nested Worker继续是唯一 transcript actors。
3. 跑 Route/View focused Vitest、受影响文件 Biome、`pnpm typecheck` 与 JSON/diff检查。
