# Route Probe Generation 与恢复持久性设计

## 问题

Enumeration 的 DIR producer 会跨多个 invocation 消费同一 exact-origin 队列。
旧实现只保存 pending network candidate，存在四类会破坏闭环的状态：

1. verified positive 已得到但 `directory_entries` 写失败，cursor 却把 URL 当完成；
2. 新 attempt 已替换 generation 后，旧 HTTP 响应仍可补写业务行或覆盖/清理新 cursor；
3. terminal evidence/outcome 写失败前先清 cursor，下一轮只能重新发网络；
4. 稳定的 network、business-write 或 terminal-publication 失败可无限自动重试。

这些情况都可能把已知 positive 变成后续 `empty/blocked`，或让 stage 永久停在
无界 retry。

## 决策

- checkpoint schema v8 保存 pending network queue、pending verified business writes、
  compact terminal-publication cursor，以及三类 bounded failure state。
- checkpoint identity 绑定 run/session/operation/stage start/engagement org/target owner/
  exact origin/plan；旧 v1-v7 cursor 不恢复。
- checkpoint store/clear 先锁 operation epoch，再用新 statement 锁并验证 current DIR
  generation，避免 READ COMMITTED 在等待旧 snapshot 后误写。
- `directory_entries` 写在短事务中同时锁 target guard、operation epoch、engagement
  subtree 与 DIR generation；旧 attempt 返回 `Superseded`，不写业务行。
- terminal `found|empty|blocked` 与对应 operation-state slot 删除在同一事务完成。
  崩溃只能留下 `partial + cursor` 或 `terminal + no cursor`。
- authorization lookup unavailable 与 confirmed owner drift 分开：前者保留 cursor，
  后者必须按 current operation/generation 清理刚写入的 terminal cursor；若 generation
  已变化则只标记 superseded，不得删除新 cursor，也不得恢复旧结果。
- 初始候选和 batch target 都按 canonical exact origin/absolute URL 去重；同 URL 的
  wordlist witness 优先，保留显式 recursion 语义。
- network candidate 稳定 2 次或累计 3 次失败，可在完整队列闭合后形成 evidence-backed
  DIR `blocked`。business-write 与 terminal-publication 稳定失败只保持 `partial`，
  `automatic_retry_allowed=false`，修复后由对应 `retry_exhausted_*` flag 单 root 重试。
- completion 的单 root runtime 默认/上限 30 分钟；未显式设置 batch ceiling 时不再
  施加批次级 start gate，所有有限 per-root task 都会被调度。显式
  `batch_max_runtime_ms` 仅表示“超过该时间不再启动新 root”，不会取消已启动 root；
  checkpoint count/slot/namespace 仍有硬上限。
  capacity overflow 不普通 resume，返回 manual reason，要求增加 runtime/rate 或收窄
  显式 seeds/wordlist。

## 模型可见合同

single/batch compaction 必须保留：

- `automatic_retry_allowed` 与完整 `retry.reason_codes`；
- pending candidate/business-write/terminal cursor；
- `authorization_unavailable`、`attempt_superseded`；
- business-write/terminal-publication breaker、counter、last failure kind/preview、manual
  reason 与可直接执行的 `recovery_action`。

当 `automatic_retry_allowed=false` 时，compactor 不得再以 `partial` 或 queue 未完成为由
推荐普通 retry；next action 必须说明停止自动重试和精确的人工恢复动作。

## 不变量

- 不在事务内发 HTTP 或追加外部工作。
- persistence failure 不能升级成 DIR `blocked` 或 checked-empty。
- superseded attempt 不能写业务事实、checkpoint 或 terminal outcome。
- terminal outcome 没有真实 evidence id 时不能发布。
- batch 中一个 exact origin 只能出现一次，即使 target_id 不同。
