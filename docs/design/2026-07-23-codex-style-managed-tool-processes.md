> Superseded by `docs/design/2026-07-23-codex-same-session-process-yield.md` for the global AI-tool process contract. This document remains the implementation history for the first EAS-scoped pass.

# Codex 式受管工具进程与 EAS 后台完成设计

## 背景

最新 EAS run `pentest-chat-1784774068531-1` 没有进入 Gate；Naabu Connect 端口发现被 EAS wrapper 的 600 秒 foreground deadline 连续杀死，失败结果又不能发布 terminal PORT truth，模型因此从同一个 pending worklist 不断缩批重试。

Golish 的通用 shell/pentest 路径原本已经支持“短暂前台等待后返回 `job_id`，进程继续运行”，但仍有三处与目标行为冲突：

1. `BackgroundJobManager` 还有普通 30 分钟、AXFR 15 秒 hard watchdog，会按 elapsed time 自动 kill；
2. guarded EAS wrapper 强制 `ForegroundOnly`，达到 wrapper deadline 会 kill，无法把决定权交给 AI/operator；
3. `submit_stage_deliverable` 默认在一次调用里等待约 30 分钟，AI拿不到及时的存活/输出反馈，也就不能决定继续等或 kill。

Codex 当前公开产品语义把长命令保留为后台 terminal：agent/user可查看后台 terminal 及最近输出，并显式停止；`background_terminal_max_timeout` 约束的是一次空轮询等待窗口，不是进程寿命。Golish 采用同一控制模型，但仍保留安全 producer 自身的连接 timeout、请求预算、scope 撤销与会话取消。这些是单次 I/O/授权边界，不是按总 elapsed time 自动杀整个进程。

## 目标

- 普通 shell/pentest 进程不再因 soft/hard wall-clock deadline 被自动 kill。
- inline wait 只决定何时把仍存活的进程交还为 `job_id`；它不改变子进程寿命。
- AI可以读取 `running`、elapsed、stdout/stderr累计字节、最近输出距今时长和保留 tail，再选择 bounded wait、检查、缩小任务或显式 `kill_job`。
- 进程自然退出后完整输出仍可供 server-side parser/hash/landing 使用；UI/model只接收有界 tail。
- EAS full port discovery 可以后台运行，但必须携带不可由模型构造的 typed continuation；完成后继续使用原 target guard、structured landing、evidence/outcome 和 Gate attestation，绝不回退到 legacy generic background parser。
- 历史 Nmap v1 / Naabu v2 terminal evidence继续严格可读；新受管进程只生产 v3 attestation。
- 不修改 `golish-db`、schema 或 migration；app重启会终止进程内 supervisor，重启后的 pending worklist安全重跑。本期不承诺跨 app crash 恢复 OS child。

## 非目标

- 不取消 scanner/browser/helper 内部的单连接、单请求或有界 producer budget。
- 不允许 Candidate verifier、cleanup side effect 或其它明确要求同步 receipt 的动作自行逃逸成 generic background job。
- 不把“长时间无 stdout”自动解释为挂死。Naabu `-silent` 在没有开放端口时本来就可能长期安静；AI必须结合进程存活、recipe规模、elapsed slow-SLO和累计字节判断。
- 不把 cancelled、killed、非零退出、输出不完整或 guard drift 改写成 checked-empty。

## 进程生命周期合同

### 状态与终止原因

后台 manager 只接受以下终止来源：

- `exited`：子进程自然退出；只有 exit 0 且输出完整时，安全 producer才可能发布 terminal truth；
- `operator_cancelled`：用户或 AI 显式 `kill_job`；
- `session_cancelled`：用户停止/关闭 AI session；
- `spawn_failed` / `wait_failed` / `output_incomplete`：基础设施失败。

不存在 `deadline_expired`。任何基于 elapsed time 的自动 kill 分支和 `COMMAND_TIMEOUT` 合成都从通用 process manager 移除。

### inline handoff

- `background:true`：保留短启动确认窗口；立即失败仍内联返回，仍在运行则返回 handle。
- 普通 auto 模式：最多内联等待 `min(requested_wait, GOLISH_TOOL_INLINE_WAIT_MS)`，到点只把 job promote 到当前 session并返回handle；旧 `GOLISH_TOOL_SOFT_TIMEOUT_MS` 仅作为兼容fallback读取。
- foreground-policy 模式：不按 wall clock kill；只响应当前 tool/session cancellation。它适用于不能安全 detached 的同步动作。
- 只有 promote 后的 job进入 session reconciliation barrier和 completion listener。内联完成的 job不会同时触发 generic background evidence，避免同步/后台双落库。

### 可观测性

`JobSnapshot` / `check_job` 至少返回：

- `status` / `running` / `termination_reason`；
- `duration_ms`；
- `stdout_total_bytes` / `stderr_total_bytes`；
- `last_output_age_ms`（尚无输出时为 null）；
- 有界 stdout/stderr tail及其 retained/truncated metadata；
- `automatic_kill=false`。

`wait_for_background_jobs` 继续是有界观察调用：timeout/idle只结束这一次等待，绝不终止进程。Stage submit只做短 grace/snapshot，发现 outstanding job就立即把相同活动信息交还 AI，不再内部等待几十分钟。

### 完整输出 spool

每个 job在 server-owned `.golish/background-jobs/` 下写 stdout/stderr spool；内存仍只保留 512 KiB tail，completion事件仍只带小 tail。spool使用每流固定安全上限并记录总字节与是否截断；进程自然退出且两个 pipe 都 EOF 后才标 terminal。

- `check_job` 读取内存 tail，不把大输出灌进模型上下文；
- typed reconciler读取完整 spool做解析和 hash；
- spool截断、写失败、pipe drain失败均是 `output_incomplete`，安全 producer不得发布 terminal truth；
- terminal job在 manager保留期内仍可读取；reconciled job被正常 prune时一并清理spool。

## typed background reconciler

`golish-app-core` 定义与业务无关的 async `BackgroundJobReconciler`：manager在 child exit、pipe drain、spool finalize之后、completion broadcast之前恰好调用一次。trait只接收 server-created terminal output descriptor，返回：

- 完整 typed tool result（供内联完成或后台 terminal UI）；
- model-facing completion note；
- evidence ids（供 HarnessTrace）；
- `skip_generic_persistence=true`。

manager不依赖 EAS 类型。`golish-pentest-app` 实现 EAS reconciler，闭包持有：

- exact session/tool/operation/stage/org identity；
- `Vec<ActiveTargetAuthorization>` / `TargetWriteGuard`；
- `PortScanPlan`、manifest、workspace和 scanner recipe；
- DB pool。

EAS runner新增 crate-internal guarded-managed入口；模型不能用 JSON `background:true` 为任意 guarded command安装 reconciler。launch前仍在最后 async seam重验全部 target guards。EAS wrapper无论在inline窗口内还是后台自然退出，都只由同一个reconciler执行 `land_authorized_output → persist_guarded_eas_evidence_and_outcomes → apply_wrapper_completion_semantics`，不会同步/后台各写一次。

reconciler失败、显式kill、非零退出、spool不完整、target owner/scope/ports漂移、parser/receipt/hash不一致时返回partial/error，`skip_generic_persistence`仍保持true；legacy background hook绝不能接管失败的EAS typed job。

## EAS Naabu v3 recipe

移除的是 wrapper/process wall-clock kill，不是每次 TCP Connect 的 socket timeout。Naabu 2.6.1 的 `-rate` 只是发送速率上限，Connect worker默认仅25；因此v3固定显式worker和网络参数：

```text
quick:    naabu -list {input_file} -iv <4|6> -top-ports 100  -s c -Pn -c 128 -rate 200  -timeout 500 -retries 1 -verify -warm-up-time 0 -silent -nc -duc -no-stdin
standard: naabu -list {input_file} -iv <4|6> -top-ports 1000 -s c -Pn -c 128 -rate 500  -timeout 500 -retries 1 -verify -warm-up-time 0 -silent -nc -duc -no-stdin
full:     naabu -list {input_file} -iv <4|6> -p 1-65535      -s c -Pn -c 128 -rate 1000 -timeout 500 -retries 1 -verify -warm-up-time 0 -silent -nc -duc -no-stdin
```

host budget固定为quick=128、standard=32、full=4。`-c 128`兼顾silent-drop吞吐和GUI进程常见FD预算；`-Pn`移除host discovery分支，`-duc/-nc`禁止更新/云上传，exact IP避免DNS，`warm-up-time=0`去掉固定暖机延迟。

保守 slow-SLO（只告警/提示，不kill）按每端口“初次+1 retry+verify”、每步500ms worker service，再叠加rate节流估算：

| profile | 最大 hosts | slow-SLO |
|---|---:|---:|
| quick | 128 | 240 秒 |
| standard | 32 | 480 秒 |
| full | 4 | 3600 秒 |

本期不新增跨调用的manifest去重表或process-wide full并发锁；这些属于独立的调度/资源治理问题。现有stage计划仍负责避免重复调度，同一managed handle由`job_id`持续观察。若后续要做全局去重，需要单独设计可回收的manifest→job索引，不能把它混入本次“取消elapsed自动kill”的生命周期修复。

## v3 attestation

新 full producer写：

- `schema=eas_port_scan_attestation_v3`；
- `profile.schema=eas_port_scan_coverage_v3` / `profile_version=3`；
- coverage中的`inline_wait_secs=30`、profile-specific `operational_slo_secs`和`automatic_deadline=null`；
- execution中的`termination_reason=exited`与`automatic_kill=false`；
- exact v3 recipe、expanded-host manifest、完整spool stdout hash、逐host `1..65535/count=65535/completed` receipt。

manifest hash使用 `eas_port_scan_manifest_v3` domain separator，并纳入profile/version、family、port scope、exact args和排序后的expanded hosts。read side继续原样验证Nmap v1和Naabu v2，再单独严格验证Naabu v3；v2不得接受新command或无deadline语义。

## UI 与 Agent 行为

- Background panel把“Soft timeout”改为“前台等待后转后台”，删除hard-deadline倒计时，显示“不会自动停止；由AI/你决定”。
- panel继续显示last output；stop按钮仍是显式取消。
- Prober默认工具面显式包含`wait_for_background_jobs` / `check_job` / `kill_job`；EAS methodology明确silent scanner可安静，先按slow-SLO/manifest规模、进程存活、累计字节与last-output age观察，不得把idle observation本身当kill理由；调度层仍应避免重复启动同一manifest。
- submit发现live job时返回handle、elapsed、last-output/bytes和下一步，不把它当Gate BLOCK，也不内部长期睡眠。

## 安全与恢复

- target launch guard和每次landing/evidence/outcome guarded write保持不变。
- scope/owner撤销可显式取消job；任何撤销后的输出仍因guard失败而不能落terminal。
- session stop杀该session全部running jobs；这是用户取消，不是timeout。
- app crash后内存callback不存在，不能伪称完成。重启时DB cell仍pending，现有worklist可安全重跑。跨重启supervisor/durable PID不在本期，因其需要新的durable job descriptor和恢复协议。

## 验证策略

不对外部目标发真实扫描。使用短生命周期本地进程、fixture reconciler、临时spool和构造的Naabu输出验证：

- manager不会因elapsed自动kill，explicit kill/session stop仍终止并reap；
- running snapshot准确报告last activity/bytes，完成后spool仍可读；
- typed reconciler在broadcast前exactly-once，且EAS job永不走generic fallback；
- EAS managed handle自然退出才guarded landing；cancel/nonzero/truncation/guard drift均不terminal；
- 超过内存tail的spool仍能完整hash/parse；跨调用manifest去重留给后续调度治理；
- Gate接受严格v3并拒绝workers/timeout/retry/Pn/automatic-kill/termination/hash/receipt篡改，同时历史v2 fixture仍通过；
- frontend不再显示hard deadline，显示manual lifecycle和last output。
