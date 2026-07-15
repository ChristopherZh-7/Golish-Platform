# CLI / GUI Operation 语义统一与公司闭环验收

> Status: accepted for implementation
> Date: 2026-07-14
> Scope: GUI Task/Profile、`golish --stage-run`、exact resume、Scoping → Attack Candidate
> Phase-boundary CLI adapter detail: `2026-07-15-cli-phase-boundary-approval-parity.md`

## 1. 一句话契约

对同一个已确认的 `OperationLaunchSpec`，CLI 和 GUI 必须得到同一 session owner、
profile、workspace/project scope、可信 target 快照、DAG、approval 决策、gate、evidence、
handoff 与 Candidate barrier；两端只保留输入采集、进度展示和输出格式差异。

```text
GUI adapter ─┐
             ├─ typed launch spec → shared TaskOperation kernel → DB/gate/evidence
CLI adapter ─┘
```

“广州有创网络科技有限公司”当前只是 engagement subject。公司名不能推导扫描授权，
provider/公开信息也不能自动升级为 domain/IP/CIDR/URL target。

## 2. 当前证据

- Test1 workspace 对该公司是 `0 organizations / 0 targets`；全库另有 4 个无关旧 target。
- 最新 GUI session `pentest-chat-1784017367012-1` 已创建 operation
  `e32e9b19-3ae3-4563-b455-ffb56df8299c`，停在 Scoping；公司查询被应用重载中断，
  没有 scope snapshot、deliverable 或 handoff。
- runtime tool owner 已能写成 exact operation owner，但 GUI/CLI 仍各自组装 session、
  tracker、repo、project scope 和 orchestrator。
- 当 `engagement_org_id=NULL` 时，旧 asset truth 会回退全库。2026-07-14 已先改为
  fail-closed，并修正 `run_tree.py` 通过 `sessions.chat_session_key` 找 operation。

因此当前现场只证明 GUI 已到达 tool dispatch，不能证明 Scoping PASS，更不能证明
Scoping → Candidate 闭环。

## 3. 不等价点

| 语义 | GUI | CLI | 风险 |
|---|---|---|---|
| operation 启动 | heuristic / lead `start_operation` | flag 确定启动 | 同一句目标可能不创建同一种 operation |
| session/tracker | `chat.rs` 自行 upsert/rebind | `stage_run` 两处 upsert/rebind | owner 再漂移 |
| profile | UI 先异步 mutate bridge，失败被吞 | args/env | UI 显示与后端实际 profile 不同 |
| DAG | `run()` 全 profile | `run_stage()` slice | 终点语义不同 |
| scope | Scoping review 后 late bind | `--org/--target` 预写并冻结 | 公司名在 CLI 拥有更强 authority |
| approval | typed GUI response | `--auto-approve` 可返回通用文本 | scope/choice/unit review 不等价 |
| Candidate review | DB-authoritative UI + resume | 无对应 adapter | CLI 会停在 waiting_approval |
| provider bootstrap | shared provider config | CLI 自己 match，未知值 fallback | 相同配置可能连接不同 provider |
| 子公司阈值 | 契约使用 51% | 默认 50% 且 `>=` | 50% 单位只被 CLI 纳入 |

## 4. 统一内核

共享内核位于 `golish-agent-app/src/ai/task_operation.rs`，由 `golish` CLI 复用；不能把
CLI flag、terminal 输出或 Tauri UI 放进内核。

### 4.1 Typed launch

```text
TaskOperationLaunch
  chat_session_key
  session_title
  objective
  profile_id
  entry: FullFromScoping | Slice | Resume
  continuity_adoption
  scope: unconfirmed subject | confirmed organization/targets/snapshot
```

第一片只机械统一 fresh operation context：

1. `sessions.upsert_by_chat_key` 失败即停；
2. tracker 在任何 tool dispatch 前 rebind 到 DB session UUID；
3. 从同一个 top-level request lease 创建 `BridgeAgentExecutor`；
4. 只创建一次 DB repo provider 与 trait-object views；
5. 只从 canonical workspace 注册 project scope；
6. 使用同一构造器配置 `TaskOrchestrator`；
7. typed entry 只决定 `run` / `run_stage` / `resume`；
8. 返回 operation selector 和 adapter 所需结果。

任何现有 CLI-only 类型若位于 `golish` crate，必须先提升为中立的 app/kit 类型；
禁止为了复用让 `golish-agent-app` 反向依赖 `golish`。

### 4.2 Trusted scope intake

后续将两端收敛到同一个 typed intake：

- `SubjectLabel`：公司名/关键词，只允许 registry lookup 和组织确认；
- `ConfirmedOrganization`：用户确认的法律主体及子公司 policy；
- `ConfirmedTarget`：用户明确给出的 exact domain/IP/CIDR/URL/wildcard；
- `FrozenScopeSnapshot`：服务端从已确认行生成，不能由 model prose 代替。

CLI `--org` 不再把裸公司名直接升级为已授权组织；`--target` 与 GUI target review 必须
产生同一 canonical row/source/snapshot。provider discovery 永远只是 observation。

### 4.3 Approval 与 Candidate review

- 两端共享 typed `ApprovalPort`；`scope_review`、`choice`、`unit_review` 不能使用通用
  `"auto-approved"` 文本。
- CLI 自动模式只能对显式、可机器验证的 decision schema 生效；未知 request fail closed。
- Candidate Gate 后两端读取同一 DB review barrier。CLI 需提供 list/review/resume adapter，
  不能绕过 barrier，也不能把 waiting_approval 当完成。
- provider/model/endpoint 统一由同一 factory 解析；未知 provider 必须报错，不 fallback。

## 5. 公司名-only 的确定性结果

只有公司名、没有可信 target 时：

1. Scoping 可形成组织级空 target snapshot；
2. 在主动 EAS 前进入明确的 `awaiting_concrete_target`/typed approval，或按 profile 转
   reporting；
3. 不执行 liveness、port、service、web、enumeration、vuln 或 Candidate worker；
4. 不允许用空分母一路 PASS 到 Candidate；
5. 不产生 scan evidence、Candidate manifest 或 Candidate row。

真实 Candidate 正向验收另用一个用户明确授权的 exact target；本地自动化只使用 loopback
fixture 和 scripted executor，不请求外部目标。

## 6. Parity 验证

- 同一输入经 GUI/CLI adapter 生成相同 normalized launch spec；
- session UUID、task/operation owner、project scope 和 tracker rebind 顺序相同；
- full/slice 仅入口/终点不同，其余 orchestrator 配置相同；
- typed approvals 和 Candidate waiting/review/resume 状态相同；
- scripted executor 下，忽略 UUID/时间戳后比较 event 序列与 DB rows；
- 50%/51% 边界两端结果相同；
- company-only fixture 在两端都停在具体 target barrier；
- positive loopback fixture 在两端产生相同 handoff/manifest hash。

## 7. 非目标与安全边界

- 本设计不要求 CLI 与 GUI 文案、动画、terminal/JSON 输出一致。
- 本轮不改 DB schema/migration 或 generated IPC type。
- 未收到 exact target、子公司 policy、主动扫描边界和外部 provider 许可前，不对指定公司
  启动真实扫描或付费/外部请求。
- shared kernel 不放松任何 gate、ownership、evidence、Candidate 或 approval fence。

## 8. 完成定义

代码闭环需要 focused parity tests、module cards 和进度证据。feature 只有在同一授权 fixture
经 CLI/GUI 两个 adapter 得到相同 DB/gate/Candidate 结果，并且指定公司获得明确 target 后
完成真实 transcript + `run.log` + `run_tree.py --full --db` + DB truth 核对，才可标记
`passing`。
