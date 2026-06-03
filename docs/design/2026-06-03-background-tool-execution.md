# 工具调用「超时转后台」执行 设计

> 目的：把当前「工具调用一旦超时就**杀进程 + 报错**」的行为，改成 **Cursor 式的「软超时转后台」**：到达软超时不杀进程，而是把命令**降级为后台作业继续运行**，立即把一个 `job_id` 返回给 AI；AI 可**主动轮询观测**增量输出/状态，作业**跑完后回灌通知** AI。彻底替代「超时即放弃 / 盲目重试」。
>
> 证据来源：本设计 §1 表中每条均为 2026-06-03 本会话亲自读真实代码核对。日期：2026-06-03。
> 关联 feature：`background-tool-execution-2026-06-03`（已加入 `feature_list.json`，状态 `not_started`）。
> 实现交接：本文件由 UI 修复会话（MCP-3）撰写，供后续 **Rust 后端会话**实现；注意 §7 并发警示。

---

## 0. 决策（TL;DR）

- **问题**：所有 AI 发起的 shell/pentest 命令都经过 `run_shell_command_detail`，它用 `tokio::time::timeout(...) + Command::kill_on_drop(true)`。超时 → future 被 drop → **子进程被杀**，返回 `{ timed_out: true, exit_code: 124 }`。长任务（大端口扫描、爆破、长 nuclei 等）只要超过默认超时就被打断，AI 只能放弃或重试，且重试同样会再超时。
- **方向（用户 2026-06-03 选定方案 A：先写设计文档）**：引入**后台作业管理器**，把「软超时」与「硬上限」分离：
  - 软超时（默认 30s，可配）→ **不杀**，转后台 + 返回 `{ status: "backgrounded", job_id, partial_stdout, … }`。
  - 硬上限（默认 30min，可配）→ 才真正杀，防资源泄漏。
- **观测**：新增 `check_job` 工具，AI 可主动轮询拿增量输出 / 退出码 / 状态。
- **回灌**：作业结束 → 发 `tool_background_completed` 事件 → agent-runtime 把结果作为后续 turn 注入（复用 ask_human 同款「异步回灌」机制），AI 自动知道结果。
- **非目标**：不重写 agentic_loop / orchestrator；不改 DB schema（首期，作业状态走内存 + 事件）；不改 Chat/Task 分发逻辑。
- **分期**：P1 后台作业管理器 + 软/硬超时 + `check_job`（**纯工具层，不动 agent loop 热文件**）→ P2 完成事件 + 自动回灌（动 agent-runtime，需待其它会话稳定）→ P3 前端「后台运行中」卡片 + 取消按钮 + evidence ledger 记录。

---

## 1. 现状勘验（本会话亲自核对真实代码）

| 环节 | 现状 | 真实落点（已核） | 缺口 |
|---|---|---|---|
| 命令执行核心 | ⚠️ 超时即杀 | `golish-app-core/src/pty_interactive.rs:58-98 run_shell_command_detail`：`timeout(dur, cmd.output())` + `cmd.current_dir(ws).kill_on_drop(true)`（`:67`） | future drop → 进程被 kill；无后台续跑 |
| 超时返回 | ✅ 有标记 | `pty_interactive.rs:88-97`：`Err(_) =>` 返回 `{ error, exit_code:124, timed_out:true, duration_ms }` | 只是「报错」，无 job 句柄可续 |
| run_pty_cmd 工具 | ⚠️ 短超时 | `pty_interactive.rs:170-194`：参数 `timeout`（默认 10s、上限 120s）→ `run_shell_command_detail` | 默认 10s 极易超时 |
| pentest_run 工具 | ⚠️ | `golish-pentest-app/src/pentest_ai/run.rs:19-20`（默认 120s、上限 600s）、`:212-216`、`:234` → `run_shell_command_detail` | 同上，长扫描会被杀 |
| AI 看到的结果 | ⚠️ | 工具返回的 `{timed_out:true}` 作为普通 `tool_result` 回到 agentic_loop | AI 只能据此放弃/重试；无「后台仍在跑」概念 |
| 异步回灌先例 | ✅ 可复用 | `ask_human` 链路：前端 `respondToToolApproval` 把用户回答作为后续注入（见 `useAiChatEvents.ts ask_human_request`）；后端有对应「等待人类回答」的 decision/event | 证明「工具调用挂起 + 稍后异步回结果」的机制已存在，可作为回灌蓝本 |

> **核心洞察**：执行核心是**单一收口点** `run_shell_command_detail`，三个工具（run_pty_cmd / pentest_run / run_command）都走它 → 改一处即可影响全部。且「异步回灌」已有 ask_human 先例，P2 不必从零造。

### 1.1 复现链（长任务 → 被杀）

```
pentest_run { tool_name:"nmap", args:"-p- -sV 10.0.0.0/24" }
 → run.rs execute_background → run_shell_command_detail(cmd, ws, 120_000)
 → tokio::time::timeout(120s, cmd.output())   ── 120s 到 ──> Err(Elapsed)
 → future drop → kill_on_drop 杀掉 nmap（扫到一半）
 → 返回 { timed_out:true, exit_code:124 }
 → AI：要么放弃，要么重跑（重跑还会再被杀）
```

---

## 2. 目标 / 非目标

**目标**
1. 软超时不杀：命令到达软超时仍在跑 → 转后台继续，立即返回 `job_id` + 已有的 `partial_stdout`。
2. 可观测：AI 用 `check_job(job_id)` 主动拉增量输出 / 状态 / 退出码。
3. 跑完通知：作业结束自动把结果回灌给 AI（P2），无需 AI 死等。
4. 不泄漏：硬上限到了仍未结束才杀；会话结束 / app 退出时清理所有后台作业。
5. 单点改造：核心只动 `run_shell_command_detail` 收口点 + 新增作业管理器，不散落改每个工具。

**非目标**
- 不重写 `agentic_loop` / `task_orchestrator` / sub-agent。
- 不动 Chat/Task 模式分发、不动意图分类器。
- 首期不引入 DB schema 变更（作业状态走内存 + 事件；持久化留 P3 如需要）。
- 不改 run_pty_cmd 在用户可见终端里的「交互式」语义（本设计只覆盖「tool detail 后台命令」这条 `run_shell_command_detail` 路径）。

---

## 3. 设计方案

### 3.1 后台作业管理器（新模块）

建议落在 `golish-app-core`（与 `pty_interactive` 同 crate，避免新增依赖环），新增 `background_jobs.rs`：

```rust
pub struct BackgroundJob {
    pub id: String,              // job_<uuid8>
    pub command: String,
    pub started_at: Instant,
    pub status: JobStatus,       // Running | Done | Failed | Killed
    pub exit_code: Option<i32>,
    // 滚动缓冲（上限 ~512KB，超出保留尾部，复用 session-helpers 的策略）
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
}

pub struct BackgroundJobManager {
    jobs: DashMap<String, Arc<BackgroundJob>>,   // 进程级单例（OnceCell / AppState 持有）
}

impl BackgroundJobManager {
    // 直接以「后台模式」spawn：不 kill_on_drop，child 交给独立 task 读到 EOF。
    pub fn spawn(&self, command: &str, workspace: &Path, hard_limit: Duration) -> String;
    pub fn snapshot(&self, job_id: &str) -> Option<JobSnapshot>;   // 状态 + 增量/全量输出
    pub fn kill(&self, job_id: &str) -> bool;                       // 取消
    pub fn reap_finished(&self);                                    // 周期清理
}
```

要点：
- spawn 出的 child **不设 `kill_on_drop`**；用独立 tokio task `child.wait_with_output()`（或边读边写缓冲）直到结束，更新 status/exit_code。
- 硬上限用一个 watchdog：到点仍 Running → kill + 标 `Killed`。
- 会话结束 / app 退出 → `kill all`（防孤儿进程）。

### 3.2 改造收口点 `run_shell_command_detail`

把单一 `timeout(cmd.output())` 改为「软超时竞速」：

```text
soft = min(请求超时, 软超时上限)        // 默认 30s，可由参数/配置调
hard = 请求超时（即旧的 timeout_ms）     // 作为后台硬上限

select! {
  res = cmd.output()  => 正常返回（同今天）
  _   = sleep(soft)   => {
     // 不杀！把「已启动的 child」移交后台管理器（或：软超时前就用管理器 spawn，
     //  到 soft 仍 Running 就 detach）。返回：
     { status:"backgrounded", job_id, partial_stdout, partial_stderr,
       hint:"用 check_job 轮询；或等完成通知" }
  }
}
```

实现取舍（二选一，实现 agent 定）：
- **(a) 一开始就走管理器 spawn**：`run_shell_command_detail` 内部总是 `manager.spawn(...)`，然后 `select(wait_done(soft), sleep(soft))`；soft 内完成→像今天一样返回完整结果并从管理器移除；soft 未完成→返回 backgrounded。**推荐**（child 句柄从头由管理器持有，detach 无需「移交正在 await 的进程」这种麻烦）。
- (b) 先 `cmd.output()` 带 soft 超时，超时后再用管理器重启——**不可取**（重复执行有副作用，pentest 命令尤其不能重跑）。

→ 选 (a)。

### 3.3 观测工具 `check_job`（P1）

新增 `golish_core::Tool`：
```
check_job { job_id: string, tail_bytes?: int }
 → { status, exit_code?, stdout(尾部/增量), stderr, duration_ms, running: bool }
```
注册走 `commands_facade`（如涉及 Tauri command）或直接作为 agent 工具注册（参考 pentest_ai 工具注册方式 `create_pentest_ai_tools`）。命名遵循 `<domain>_<verb>_<object>`：建议 agent 工具名 `check_job`（或 `job_check`，按现有工具命名习惯定）。

### 3.4 完成回灌（P2，动 agent-runtime）

- 作业结束 → 管理器发 `tool_background_completed { job_id, exit_code, stdout_tail }` 事件（走现有 AI event 通道，前端 `useAiChatEvents` 加 case）。
- 后端：在 agent-runtime 注入一条「后台作业 X 已完成，结果：…」作为后续 turn 的输入（**复用 ask_human 那套「异步把外部结果喂回会话」的机制**，不要新造一条注入链路）。
- 注意：这一步触碰 agent-runtime 热文件（§7），**务必等其它会话稳定后再做**，或与其协调。

### 3.5 前端（P3）

- `ToolCallDetailView` / `ToolExecutionCard`：识别 `{status:"backgrounded", job_id}` → 渲染「⏳ 后台运行中（job X）」+「查看输出/取消」；收到 `tool_background_completed` → 变 ✓ 并展示最终输出。
- 复用本会话已落地的 ANSI 渲染（`<Ansi>` + `stripOscSequences`）与 `\n\t` parse 逻辑。

---

## 4. 分期

| 阶段 | 内容 | 触碰文件 | 风险 |
|---|---|---|---|
| **P1** | 作业管理器 + 软/硬超时（改 `run_shell_command_detail`）+ `check_job` 工具 | `golish-app-core/pty_interactive.rs`、新增 `background_jobs.rs`、工具注册 | 低（**不动 agent loop 热文件**） |
| **P2** | 完成事件 + 自动回灌 | agent-runtime（**热文件**）、前端 `useAiChatEvents` | 中高（并发冲突） |
| **P3** | 前端后台卡片 + 取消 + evidence ledger 记录 | 前端工具卡、audit | 低-中 |

> P1 已能消除「超时即杀」并让 AI 轮询，交付大部分价值；P2/P3 增量优化。

---

## 5. 风险 / 边界 / 回滚

- **资源泄漏**：必须有硬上限 watchdog + 会话/app 退出清理；缓冲上限防内存爆。
- **副作用命令重跑**：严禁方案 (b)；只用 (a) 单次执行 + detach。
- **并发冲突**：P2 触碰 agent-runtime/bridge，正被其它会话改（§7）。
- **安全/scope**：后台作业仍是 AI 发起的扫描 → 仍受 stage 白名单（`tool_taxonomy.rs`）约束；backgrounded 不绕过授权门禁。evidence ledger 要记后台作业的最终产物（AGENTS.md I7、I8：「已检查为空」≠「未检查」，后台跑完才算 checked）。
- **回滚**：P1 加 feature-flag（如 `GOLISH_TOOL_BACKGROUNDING=off`）→ 关掉即回到「软超时=硬超时、超时即杀」的现行为；`check_job` 工具不注册即可。

---

## 6. 验证 DoD

1. 单测：管理器 spawn/snapshot/kill/硬上限 watchdog；`run_shell_command_detail` 软超时返回 `backgrounded` 且进程仍 Running；硬上限到点被 Killed。
2. 集成：一条 `sleep 60` 命令，软超时 5s → 返回 backgrounded → `check_job` 轮询到最终 exit_code=0。
3. 副作用安全：确认 backgrounded 路径**只执行一次**（用带副作用的临时脚本计数验证）。
4. `cargo nextest -p golish-app-core`（+ 涉及 crate）全绿；`just lint-rust` 0 warning。
5. P2：活体 `just dev` 跑一条长命令，软超时后 AI 收到 backgrounded，作业完成后 AI 自动收到回灌并继续。
6. `just precommit` 全绿后才可合并。

---

## 7. 给实现 agent 的并发警示（重要）

- 本仓 git 工作区当前有**多个会话并行改 agent harness 热文件**：`golish-agent-bridge`、`golish-agent-kit`、`golish-agent-runtime`（见撰写时 git status）。
- **P1 刻意设计为不碰这些热文件**（只在 `golish-app-core` + 工具注册落地），可立即安全开工。
- **P2 必须**先 `git pull`/与相关会话对齐，确认 agent-runtime 事件注入点稳定后再动，避免合并冲突。
- 加 command / 改 IPC 严格走 `docs/development.md` 五步 + `commands_facade`；跨 IPC 类型用 `#[derive(ts_rs::TS)]`（AGENTS.md I4/I5）。

---

## 8. 实现记录（P1 已落地 · 2026-06-03 · MCP-3）

P1 已实现并 scoped 验证全绿，**未碰 agent-runtime/bridge 热文件**（仅 golish-app-core + 工具注册）：

- `golish-app-core/src/background_jobs.rs`（新）：`BackgroundJobManager`（进程级单例 `manager()`）+ `spawn/snapshot/kill/remove/prune`；增量读 stdout/stderr 到 512KB 上限尾部缓冲（字符边界安全）；`select!` 在 child 退出 / 硬上限 sleep / kill Notify 三者间竞速；非 `kill_on_drop`，子进程随后台续跑。
- `golish-app-core/src/pty_interactive.rs`：`run_shell_command_detail` 改为「经管理器 spawn + 软超时轮询」——软超时内完成→返回完整结果（同旧）；超时未完成→返回 `{ status:"backgrounded", job_id, partial_stdout, hint }`（无 `error`/无非零 `exit_code`，agentic loop 视为成功）。旧行为保留为 `run_shell_command_blocking`（`GOLISH_TOOL_BACKGROUNDING=off` 时启用）。新增 `CheckJobTool`（`check_job`）。
- `golish-agent-app/src/ai/commands/bridge_config.rs`：`register_visible_pty_tool` 一并注册 `CheckJobTool`。
- 配置（env）：`GOLISH_TOOL_BACKGROUNDING`（默认 on）、`GOLISH_TOOL_SOFT_TIMEOUT_MS`（默认 30000）、`GOLISH_TOOL_HARD_TIMEOUT_MS`（默认 1_800_000）。soft = min(caller_timeout, soft_cap)；hard = max(caller_timeout, hard_default)，确保后台续跑长于旧超时。

**验证**：`cargo check -p golish-app-core`/`-p golish-agent-app` → 0；`cargo clippy -p golish-app-core --all-targets` → 0 warning；`cargo nextest -p golish-app-core` → 24 passed（含 6 个新 background_jobs 真子进程测试：捕获 stdout/非零退出→failed/长任务保持运行+kill/硬上限 kill/字符边界截断/未知 job）；`cargo fmt -p golish-app-core --check` → clean（workspace fmt 红仅别会话 harness 文件，与本改动无关）。活体 E2E（`just dev` 跑长命令观察 backgrounded + check_job 轮询）未做。

**仍待（P2/P3，给接手 agent）**：完成事件 `tool_background_completed` + 自动回灌（动 agent-runtime 热文件，需先对齐）；前端「后台运行中」卡片 + 取消按钮；evidence ledger 记后台作业最终产物。
