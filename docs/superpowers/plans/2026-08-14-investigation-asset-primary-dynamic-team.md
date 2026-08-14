# Investigation Asset Primary 动态团队实现计划

**目标：** 保留公司、资产的严格持久队列；把资产内部改为一个持续 Primary 在“假设形成”和“假设验证”两个模式中动态调用 0..N subagent，并通过动态 Tool Manager/真实浏览器完成所有 canonical hypothesis 后推进。

**架构：** 数据库只预建 Asset Primary，不预建角色 WorkItem。Primary 生成并冻结每轮动态 TaskPlan；运行时按 TaskPlan 申请/执行资产绑定的 child WorkItem。验证会话复用同一 Asset Primary，但每个 hypothesis 使用新的、动态的 child round。业务 Gate 只读取 terminal hypothesis resolution 和 pending discovery。

## Task 1：删除固定角色权威

- 新增 forward migration，退役 `investigation_asset_primary_schedules` 的四角色列、roster hash、exact-four insert trigger和固定 barrier。
- `ensure_investigation_asset_primary_schedule` 只创建/恢复 Primary WorkItem、WorkerRun和跨资产生命周期 message chain。
- 历史固定角色 WorkItem/receipt只读审计，不参与恢复、barrier或完成判断。
- 更新 portable DTO/app bridge，不再返回 `role_work_items` 或固定 barrier。
- migrated-PG 测试锁住 Primary-only、exact replay、无预建 child、foreign lane/context fail closed。

## Task 2：恢复 Primary-authored动态 Analysis plan

- 删除 host `investigation_asset_fixed_analysis_plan`、exact-four census/order和角色专用绑定。
- Primary 首轮生成 0..8 个当前资产 subtask；role来自 frozen allowed catalog，可重复、可缺席。
- host校验每个 subject ref属于当前 asset；只为 accepted plan创建 WorkItem。
- Refiner可按剩余预算增加、删除、替换、重排和重试未完成任务；失败结果回给Primary，不自动按角色恢复。
- 0 subtask直接进入 Primary synthesis；只允许 canonical proposals 或 typed zero-hypothesis。
- 恢复同一实体时隔离旧fixed-roster行，不把旧Browser失败当新round输出或barrier。

## Task 3：动态 Verification round

- verification session由服务器选择当前 unresolved revision，并绑定同一 Asset Primary。
- 删除固定五actor roster、Adviser必达、exact-four role barrier和per-hypothesis Primary。
- Primary动态创建0..N reasoning/execution child；同role可多次，每child绑定exact asset/revision。
- Tool Manager inventory、browser和0..N invocation继续使用scope/JIT/budget/credential/evidence fence。
- Primary提交`verified|refuted|invalid`；host验证current revision和citations，不要求特定角色。
- child proposal走pending discovery→compiler admission/duplicate dismissal；未消费前资产不关闭。

## Task 4：运行时整合和旧逻辑物理删除

- stage runner在一个Asset Primary循环中切换formation/verification，不建立第二Primary。
- 删除旧Campaign/fixed capability/exact-one-tool和固定角色运行入口，不留新运行fallback。
- queue progression只调用resolution-only backlog/closure authority。
- 更新prompt、actor contract和UI投影用“动态subtasks”，不把角色显示成固定队列。

## Task 5：定向验证和实体闭环

1. 每次Cargo前运行`just space-guard`。
2. 定向运行DB schedule/queue/dynamic verification migrated tests。
3. 定向运行runtime/sub-agent动态plan、actor、tool router和recovery tests。
4. 运行受影响crate scoped Clippy、rustfmt、JSON和diff检查；不主动跑全workspace门禁。
5. 构建CLI，恢复现有operation，不重跑Scoping/Application Understanding。
6. 用`run_tree.py --db`、run.log和DB验证当前资产Primary动态调用、hypothesis入库、真实工具调用、terminal resolution、discovery和next-asset推进。
7. 每个实体断点先写定向回归、最小修复、重建并resume，直到全部公司/资产closure。

## 定向命令

```bash
just space-guard
cd backend
cargo test -p golish-db --test investigation_asset_primary_scheduling -- --nocapture
cargo test -p golish-db --test investigation_dynamic_tool_manager_verification -- --nocapture
cargo nextest run -p golish-db --test investigation_asset_queue --status-level fail
cargo test -p golish-agent-runtime --lib investigation_asset --no-fail-fast
cargo test -p golish-sub-agents --lib asset_verification --no-fail-fast
cargo clippy -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app --lib --no-deps -- -D warnings
cargo fmt --all -- --check
```
