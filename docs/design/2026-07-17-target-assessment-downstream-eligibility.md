# EAS Gate 修复、Target 检测状态与下游资格

Status: Approved by the user on 2026-07-17 and implemented subject to final repository gates.

## 现场问题

夜间 Test1 run `pentest-chat-1784214296000-1` 的 Nmap service fingerprint 已完成。
最终 Gate BLOCK 来自九个 exact Web Origin 的 WhatWeb `connection_reset`：当时每格只有
attempt 1 `error`，既不是 `checked_empty`，也还没有达到 producer 的三次同类失败终态。
Gate 因此正确拒绝把这九格当作已覆盖。

旧 continuation 还有一个独立问题：同一 request 内允许一次 Controller Gate repair，但后续
Turn 的恢复额度错误地与这次 repair 共用 fuel。Controller 即使拿到新 Turn，也可能只重提旧
deliverable，然后以 `stage_team_controller_turn_resume_fuel_exhausted` 停住，无法执行 attempt
2/3。

## 决策

### Gate 仍然只有 PASS / BLOCK

不增加模糊的 Gate pass 状态。对单个 asset × technique cell，`found`、`checked_empty`、
`blocked` 和有证据的 `not_applicable` 是不同的 terminal coverage disposition；`error` 仍是
nonterminal。Gate 只判断所有必需格是否已经得到合法 terminal disposition。

### Controller repair 只保留三层必要机制

1. Gate BLOCK checkpoint 持久化完整 server-authored `gap_manifest`，而不只保存 hash。
2. 下一个 coordination round 从 checkpoint 读取该 data-only manifest，先更新 repair plan，
   只调用 manifest 指定的原 producer；禁止把 `error` 改写为 pass-like 状态，也禁止在该轮直接
   重提旧 deliverable。
3. same-Turn repair 与 successor-Turn continuation 使用独立预算：一次 same-Turn repair，加最多
   两个 successor Turns。两个 successor 恰好覆盖 attempt 2 和 attempt 3；第三个 successor
   仍 fail closed。

向前 migration 只替换 continuation contract trigger：额度读取兼容缺失/坏 policy 并硬上限为
2；authority 必须把 source gap 的完整 JSON 原样带进 resumed Worker checkpoint，不能只对 hash。

### 下游只剔除 exact origin

WhatWeb attempt 3 只说明该 producer 已停止。Enumeration 真正排除一个 origin 时，仍由后端
`list_eas_web_transport_blocked_origins` 交叉验证 operation/org/target/origin、producer blocked
outcome、独立 direct/proxy transport evidence 和 freshness。排除单位只能是 canonical
`scheme://host:port`；Target、IP、open-port fact 和 sibling origins 全部保留。

当前实体 CLI 只运行 EAS slice，因此九个 origin 已满足“下游可排除”的 marker 条件，但 operation
尚未进入 Enumeration，不能在 UI 上提前声称“已排除”。

### Target UI 只展示 evidence-ledger producer 状态

Target Surface 不改 `targets.status`。`oplog_list_by_target` 暴露已有 audit ledger 的 typed
`audit_role`、`evidence_technique`、`evidence_outcome`、`evidence_asset` 字段；前端按 exact origin
选择最新 evidence，展示：

| 状态 | 含义 |
|---|---|
| `Not assessed` | 没有权威 WhatWeb evidence，不推断健康 |
| `Fingerprint found` | ledger outcome 为 `found` |
| `Checked empty` | ledger outcome 明确为 `checked_empty` |
| `Retry pending N/3` | 结构化 `error`，仍需 producer 重试 |
| `WhatWeb stopped` | 结构化 attempt 3 `blocked`；不代表 whole Target 或下游已排除 |
| `Evidence error` | typed outcome 与 structured payload 不一致，fail visible |

状态只在 Web Origins 表保留一个列。删除整 Target 的“最坏严重度”汇总、Overview/详情重复
badge，以及根据 audit JSON 猜测 `Excluded downstream` 的逻辑。Evidence 卡仍可解释独立探针，
但明确写成“Enumeration 将重新验证”，不提前授权下游路由。

## 实体验收

使用 fresh `backend/target/debug/golish` 精确续跑原 operation
`c14e6e10-4343-4b9e-9642-2617bfb56754`，并固定 session/task/operation/org/stage identity。
续跑跨过原 fuel blocker，在原 Controller WorkerRun
`42f51251-5a9a-4e12-a341-f4426a9d84c1` 与 message chain
`d1a9b2fd-3094-4f3f-810f-5baee1f156dc` 上执行两轮真实 WhatWeb。

最终九个 exact origin 均形成连续三条 evidence id、terminal `blocked` technique outcome 和
`independently_confirmed=true` marker；EAS deterministic Gate PASS，CLI exit 0。三条 IP Target
仍为 in-scope，`targets.ports` 中 8000/8088/8443 open facts 完整保留，无扫描进程残留。

## 不变量

- 未检查、checked empty、retryable error、producer blocked、downstream excluded 必须分开。
- 前端状态不授权 Gate 或 routing。
- 不能把 exact-origin transport failure 扩大为删除 Target、IP、port 或 sibling origin。
- successor continuation 必须复用原 plan/unit/leader/WorkerRun/message chain。
- migration additive/forward-only；历史 gap/evidence 不重写。
