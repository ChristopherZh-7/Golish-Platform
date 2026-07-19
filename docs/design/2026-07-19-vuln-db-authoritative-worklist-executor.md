# Vuln DB-authoritative Worklist Executor

## 问题

Vuln Triage 的 coverage denominator 已经由数据库确定为 final-sealed Enumeration exact Web Origin × 10 个 technique cells，Gate 也只接受 operation-scoped `technique_outcomes`。但实际执行仍把公式化 fan-out 交给 Company Controller 的 LLM 循环：Controller 可以创建空 `subject_refs` 的 “ALL remaining” WorkItem，child 又在一个长对话里自行分页、调用 wrapper、重试和总结。

这造成四类失配：

1. DB worklist 是精确 cell，执行任务却可能是宽泛自然语言，无法证明一个 Worker 只拥有一个 exact origin/surface-class shard。
2. Nuclei 的扫描预算与 SubAgent/Worker iteration 混在一起；大 shard 超时后，模型可能原样重试并耗尽迭代。
3. target-side transport failure、scanner/runtime failure 和 operator outcome-unknown 都显示成相似的红色失败，但终态语义不同。
4. Stage Team UI 只显示 Worker/Barrier 数量，不能直接展示 Gate 真正关心的 terminal/total cells；最新现场因此看不到 `340/360` 与剩余 `20`。

## 决策

在现有 Company Controller Stage Team 内加入 Vuln 专用、服务端拥有的 worklist executor。它不新增表，也不改变 Gate：

1. 每次 claim 同一个 Company Controller leader 后，服务端先读取该 operation/org 的 `stage_asset_coverage_for_operation`。
2. 服务端只选择 `pending`、`error`、`partial` cells，并按 exact origin + capability family 生成 deterministic shard：
   - `vuln_nuclei_general` parameter DAST：`WSTG-INPV-05/01/12`；
   - `vuln_nuclei_general` baseline：`WSTG-ATHN-02`、`WSTG-SESS-02`、`WSTG-CONF-05`、`WSTG-CRYP-03`、`WSTG-INFO`；
   - `vuln_nuclei_fingerprint_targeted`：`GOLISH-NDAY`；
   - `vuln_probe_anonymous_access`：`WSTG-ATHN-04`。
3. `pending` cells 可在同一 exact origin/capability 内合并；已有 `partial/error` 的 shard 自动缩到单 technique。每个请求必须携带一个 DB-authorized Target canonical fact ref，objective 中固定 exact `target_id`、`target_url`、tool 与 techniques；空 subject、跨 origin 和 “ALL remaining” 不再是可生成形状。
4. 服务端把 shard 作为现有 durable Stage Worker Request/WorkItem 持久化，随后 park Controller、drain children。Controller 不再参与 Vuln fan-out；当 worklist 无 unfinished cells 时，服务端关闭 request epoch 并让同一个 Controller 只做最终 DB/Gate submission。
5. 初始合并 shard 只允许一次 Worker attempt；若 wrapper 落下 `partial/error`，下一轮 worklist 自动生成更小的单-technique recovery shard。最小 shard 只有有限 attempt，耗尽后不生成同形无限 retry；coverage 保持 partial/error，StageRun 明确 BLOCK。
6. 两个 Nuclei shard 由 StageRun 的 host executor直接调用 guarded wrapper，不创建一个负责规划/分页/重试的 Vuln Scanner LLM 对话。anonymous-access shard 仍可使用有界 endpoint review specialist，但 exact origin、capability、subject、attempt fuel 与 durable WorkItem 都由服务端固定。
7. replay 同一个 stable shard 时，`queued/retry_pending` 与 `claimed/running/waiting_dependency` 分开统计；后者表示 DB 中已有在飞执行，不能误报为 `VULN_WORKLIST_EXECUTION_EXHAUSTED`。`recovery_required` 始终走 operator recovery，不自动重放。

## Nuclei timeout 与 cancellation 所有权

两个 Nuclei wrapper 是 deadline、cancellation 和 landing 的唯一所有者：

- SubAgent 不再用通用 300 秒 deadline 截断 wrapper。
- 每个 active wrapper 通过 task-local tool cancellation token 接收用户/Worker cancellation。
- foreground runner 收到 cancellation 后 signal kill，并等待 background reaper 完成 `child.wait()` 与 stdout/stderr drain，之后才把 cancelled/error result交回 wrapper。
- wrapper 仍执行 authority revalidation、evidence append 和 `technique_outcomes=partial` landing；只有 landing/fenced tool lifecycle 完成后，SubAgent dispatch 才返回 cancelled。
- 所以 UI/operator recovery 不再面对“进程还在跑但 wrapper future 已丢失”的人为 outcome-unknown；真正的进程/应用崩溃仍走现有 manual recovery，禁止自动重放。

## 稳定 transport breaker

不新增计数表。每个 Nuclei evidence raw payload 记录 `attempt_generation`、`failure_owner` 和规范化 `failure_class`。下一次 wrapper 只读取同 operation/org/target/exact-origin/tool 的 evidence 尾序列：

- 仅明确的 target-side DNS/connect/refused/reset/TLS 类参与 breaker；
- wrapper deadline、进程 exit、缺工具、template proof、截断、解析/DB/authority 故障均归 `scanner_runtime`，永远保持 `partial`；
- 三次连续、同 class、不同 attempt generation 的 target-side failure 才产生新的 evidence-backed `blocked` outcome；中间出现成功、不同 failure class 或 runtime failure会重置连续序列；
- `blocked` 仍由 wrapper evidence + outcome 写入，不由 Worker prose 或 deliverable 填造。

## UI 语义

`StageTeamRunView` 对 Vuln operation 使用现有 coverage command 的 operation-aware读取，按所有公司 Unit 聚合 cells：

- 显示 `terminal / total` 和 `remaining`，例如 `340/360 · 剩余 20 cells`；
- Worker 行分为“历史 attempt 失败”“当前 retry”“operator recovery”；
- coverage loading/error/empty 分开显示；
- 历史失败不会把一个正在运行的新 retry 画成整个阶段当前失败，manual recovery 也不会与普通有限 retry 混用。

## 安全与兼容边界

- 不改 schema/migration，不写 `frontend/lib/generated/`，不改 deterministic Gate 的 terminal 集合。
- `found/checked_empty/blocked/not_applicable` 仍必须来自当前 operation、当前 owner、exact origin evidence；scanner/runtime/cancelled 永远不能变成 checked-empty。
- 当前 300 秒 stopgap 保留，作为 wrapper self-bounded 的底层防截断契约。
- 本轮按用户指令不启动真实 CLI、provider 或外部扫描；实现只能以 focused tests/静态检查验证，不能宣称真实阶段已闭环。
