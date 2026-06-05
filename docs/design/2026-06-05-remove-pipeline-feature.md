# 移除 Pipeline 功能 · 决策记录（Design）

> Status: Accepted / Implementing（用户确认「按默认全删并开工」）。执行期偏差：**D2 = 保留 `pipeline_only`**——读真实代码确认其为「不暴露为可委派 `sub_agent_*` 工具」的内部编排标志（reflector/refiner/orchestrator），非 DAG pipeline，强删会致 orchestrator 自我委派回归。详见 plan Phase 5 + agent-progress 2026-06-05。
> Date: 2026-06-05
> 配套实现计划：`docs/superpowers/plans/2026-06-05-remove-pipeline-feature.md`
> 触发：用户要求「把 pipeline 的整体逻辑全部删掉，无论前端还是后端」（方案 A 全删）。

---

## 1. 背景：今天的 pipeline 是什么

"pipeline" 在仓库里其实是 **三个共用名字但相对独立的子系统**，删除前必须区分清楚：

| # | 子系统 | 实质 | 入口 |
|---|--------|------|------|
| **S1** | **DAG 引擎 + 编辑器**（真正的 Pipeline 功能） | 把渗透工具串成 DAG、解析输出、写库 | `golish-pipeline` crate、`PipelinePanel`、`pipeline_*` 命令、`pipelines` 表、`run_pipeline`/`flow_compose` AI 工具 |
| **S2** | **`pipeline_progress` 时间线可视化** | 把一次 pipeline 执行渲染成时间线块 | `PipelineProgressBlock`、`store/slices/workflow/pipeline.ts`、`store/types/pipeline.ts`、`syncPlanToPipeline` |
| **S3** | **名字带 pipeline 的相邻物** | 与 DAG 引擎无直接关系，仅共用词 | `ProjectOverview/PipelineProgressBar`（AI recon 进度条）、sub-agent `pipeline_only` 标志、`run_recon_pipeline`（已废弃 stub） |

### 现状信号（说明该功能已在被淘汰）
- AI 的 `run_pipeline` / `flow_compose` **默认未注册给 agent**（`pentest_bridge/mod.rs:36-43` 注释 + `execution_mode/policy.rs` 全 false + 多处测试断言其缺席）。
- `run_recon_pipeline` 已是 **返回错误的 stub**（`golish-agent-app/.../workflow.rs:42-44`）。
- `PipelineProgressBlock` / `pipeline_progress` 块 **当前无任何生产渲染器**（`UnifiedBlock`/`VirtualizedTimeline` 只渲染 `command` 块，`pipeline_progress` → null）；`syncPlanToPipeline` 仅被 `mocks/showcase.ts` 调用。

> 结论：S1 是仍可用的真功能；S2 实际是**死代码**；S3 里 `run_recon_pipeline` 是死 stub、`PipelineProgressBar` 是仍可见的 AI recon 进度 UI、`pipeline_only` 是 sub-agent 能力标志。

---

## 2. 决策

**全量移除 S1 + S2**（DAG 引擎、编辑器、命令、DB 表、AI 工具、时间线可视化及其持久化/恢复/锚点配套），前后端一并删除。这符合用户「全部删掉」的意图，且 S2 是死代码、移除零可见回归。

---

## 3. 需用户拍板的三个决策点（写进本文档供 review）

> 默认值均按「全部删掉」取激进方案；如有不同请在确认时指出。

- **D1 · `ProjectOverview/PipelineProgressBar` + `useReconFeed`（S3）**：这是**仍可见**的 AI recon 进度条（消费 ai-event 的 `pipeline_start/done/error`），与 DAG 引擎无关，只是名字带 pipeline。
  - 默认（A 全删）：**一并删除**进度条 + `useReconFeed` + `ProjectOverview/types.ts` 的 `pipeline_*` feed 种类。
  - 备选：**保留但重命名**为 `ReconProgressBar`，与 pipeline 解耦（不丢可见功能）。
  - ⏳ 推荐：**保留并重命名**（它不是 DAG pipeline，删了会丢一个能用的 recon 进度 UI）。请确认走「删」还是「重命名」。

- **D2 · sub-agent `pipeline_only` 标志（S3）**：标记「只在 pipeline 上下文可用」的子 agent（3 个 builtin agent 用了 `as_pipeline_only()`）。pipeline 删除后该语义失效。
  - 默认：**移除 `pipeline_only` 字段**及其 builder/loader/executor/filter 逻辑；被标记的 3 个 agent 转为普通可委派 agent（或一并下线，见计划 Phase 5 备注）。
  - ⏳ 推荐：移除字段，3 个 agent 默认转普通（计划里逐个列出，执行时确认是否要彻底下线某个）。

- **D3 · `pipelines` DB 表**：表里可能存有用户保存的 pipeline JSON。
  - 默认（随 A）：新增 **forward-only drop migration** `DROP TABLE IF EXISTS pipelines;`（遵守 I10「先扩后清」语义——这是清理步）。**会丢失已保存数据**。
  - 备选：保留表与数据，仅删代码（表变成孤儿，后续可清）。
  - ⏳ 推荐：新增 drop migration（既然功能全删，留表无意义）；执行前再确认一次「确认丢弃 pipelines 表数据」。

---

## 4. 关键风险与处置

| 风险 | 处置 |
|------|------|
| **`StoreStats` 跨 crate 依赖**：`golish-pipeline/src/parser.rs` 定义的 `StoreStats` 被 `golish-pentest-app/src/output_parser.rs:296` re-export，供**通用** `output_parse_and_store` 命令使用（非 pipeline 专属） | **删 crate 前先迁移** `StoreStats` 到非 pipeline 归宿（计划 Phase 0：在 `golish-pentest-app` 本地定义同字段结构，更新消费方），否则删 crate 会编译失败 |
| **错误码契约（I1/I5）**：`GolishError::Pipeline` 变体 + `"PIPELINE"` code 与前端 `error-codes.ts:40` 镜像 | 后端删变体 **同时** 删前端 `PIPELINE` 映射，保持两侧同步 |
| **AI 系统提示**：`pentestSystemPrompt.ts` 仍把 `run_pipeline` 写成主 recon 路径 | 重写该提示，移除 `run_pipeline` 引导，改述为直接调用各 recon 工具 |
| **时间线持久化**：`pipeline_progress` 块写进了 conversation DB sync / terminal-restore / workspace-storage | 一并移除块类型与读写分支；旧会话里残留的该类型块在恢复时安全忽略（计划 Phase 7 含 sanitize 兜底） |
| **`run_recon_pipeline` 前端仍在调**：`lib/ai/session.ts:150 triggerAutoRecon → runReconPipeline` | 删命令 **同时** 删前端 `runReconPipeline` 与 `triggerAutoRecon` 调用点 |
| **ts-rs 生成类型** | 经核查 `frontend/lib/generated/` 无 `*Pipeline*` 类型，无需处理生成物 |

---

## 5. 触及的项目不变量（AGENTS.md §5）

- **I1**（错误码契约）：删 `Pipeline` 变体两侧同步。
- **I5**（ts-rs 同步）：无 pipeline 生成类型，不涉及。
- **I10**（迁移向后兼容）：drop 走 forward-only migration，作为「清理旧字段」步，符合「先扩后清」末步。
- **§2.7 高风险**：删 crate / 删大量代码 / drop 表均已先出文档 + 待确认。

---

## 6. 范围之外（保留，不动）

- 不相关的 "pipeline" 词义：`golish-pentest/evidence_sanitizer.rs`、`golish-agent-kit/task_orchestrator/prompts/pipeline.rs`（task-mode 阶段提示，非 DAG）、`golish-pty` parser pipeline、`resources/toolsconfig/*.json` 的 `json-pipeline` 标志、`resources/skills/*` 文档等。
- 仅断言 pipeline 缺席的测试（删除后仍应通过）：`golish-prompts/.../tests.rs`、`golish-sub-agents/.../defaults/tests.rs`、`golish-agent-runtime/.../prompt_render_tests.rs`、`modes/chat.rs` inline test。
- `check_recon_tools_cmd`（工具可用性检查，非 DAG）保留。
- 历史文档/计划里的 pipeline 描述不回改（保留决策史，I6）。

---

## 7. 回滚

- 全程在 `chore/remove-pipeline` 分支进行，按 Phase 独立 commit；任一 Phase 出问题可 `git revert` 该 commit 或回退分支。
- drop migration（D3）是唯一不可逆数据动作——执行前二次确认。
