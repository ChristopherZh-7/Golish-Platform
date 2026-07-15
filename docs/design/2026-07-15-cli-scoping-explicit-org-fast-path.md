# CLI Scoping 显式组织身份快通补遗

> 本文是 `docs/design/2026-07-14-cli-gui-operation-parity.md` 的窄范围补遗，只修订其中
> CLI Scoping `--org` 的身份 intake 与 company-only launch parity 描述；其余 shared
> kernel、evidence、Candidate、授权与完成门禁继续有效。旧设计保留，不覆盖。

## 1. 用户决策

当前仅支持完成闭环的 profile 固定为 `red_team`。对 fresh headless
`golish --stage-run --from scoping ... --org <公司名>`：

1. CLI 显式 `--org` 本身就是对该公司主体名称的确认；Scoping 不需要再与 GUI prompt
   中的公司候选进行身份对齐。
2. trusted seed 按 exact label 在当前 project 下 get-or-create root organization，并把其
   UUID 交给 typed launch 的 `ConfirmedOrganizationIntake`。
3. 该快通只确认 organization identity。没有本次 `--target` 时，exact target 集合必须
   保持为空；公司名、组织字段、provider discovery 与同名组织历史 target 都不能变成
   domain/IP/CIDR/URL/wildcard 授权。
4. GUI prompt-only 公司名仍使用 `UnconfirmedSubject`。本处明确允许 CLI 与 GUI 在
   Scoping intake 交互上不同，不再要求 company-only authority projection 完全相等。

## 2. Fresh target authority 三态

shared typed launch 向 `TaskOrchestrator` 投影一个 fresh-only 三态：

| 状态 | 来源 | pre-EAS 行为 |
|---|---|---|
| 未指定（`None`） | GUI/unconfirmed lifecycle | 保留既有交互式 Scoping 语义，按 bound org 读取 trusted snapshot |
| 本次明确有 target（`Some(true)`） | `ConfirmedTargetIntake`，且 target 已通过 exact-shape 校验 | 仍必须读取 DB snapshot，只有 canonical `scope=in` row 才可继续 |
| 本次明确无 target（`Some(false)`） | `ConfirmedOrganizationIntake` | 在读取 org 历史 target 之前直接 HOLD，发出 `ACTIVE_RECON_TRUSTED_TARGET_REQUIRED` |

`Some(true)` 不是直接放行位；它只允许 gate 去验证本次 intake 已落的 DB truth。
`Some(false)` 的优先级高于同名组织历史数据，从而消除“复用组织顺带借用旧 target”的越权。
`fresh_launch_authority` 只是 exact resume 可选的已验证提示，不是安全前提。
headless exact resume 见到合法 marker 时恢复其值；marker 缺失时一律收紧为
`Some(false)`，malformed 则拒绝恢复。因此 company-only HOLD 后恢复仍会在历史
target DB read 之前 HOLD；旧的 exact-target operation 如无 marker 也会安全地 HOLD，
需用本次明确 `--target` 新建 fresh run。本补遗不改变 exact resume 的
frozen-operation/source 选择接口，也不让 resume 伪造新的 fresh intake 状态。

## 3. Headless Scoping approval

`--auto-approve` 只能执行可机器验证的 typed decision：

- `scope_review` 只能回放本次显式 `--target` 生成的 exact rows；
- `subsidiary_scope` choice 必须同时满足：context 可解析、
  `decision="subsidiary_scope"`、`organization_id` 精确等于本次 seeded root；然后才按
  fresh CLI 的 `--include-subsidiaries` flag 选择 root-only 或 include option；
- generic `confirmation`（包括 phase crossing）、ordinary/unknown `choice`、`unit_review`、
  `credentials`、`freetext` 一律拒绝。

默认 `--include-subsidiaries=false` 时，上述 typed choice 可确定性选择 root-only，使
red_team Scoping 不必制造空 candidate/unit-review 表。它仍不伪造 tool lifecycle 或
StageDeliverable：agent 必须按现有 charter 调用同-root choice，并提交
`scope_confirmed` + `scope_human_approved` claims；否则 deterministic gate 继续 BLOCK。

## 4. Scoping scope freeze

CLI 在 Scoping create 时只传递 confirmed organization identity，不预先冻结
`CliRuntimeScope`。预冻结会让 Scoping finalizer 在 root Unit 和 trusted submission 尚未产生
时误进 replay 分支。正确顺序是：

1. persisted `ask_human` subsidiary choice 与 trusted StageDeliverable 通过 deterministic gate；
2. V2-writing operation 在发出 `stage_passed` 前调用 `finalize_scoping_scope`；
3. finalizer 同一事务内派生 decision、sealed snapshot、passed root Unit，并绑定该
   trusted submission；
4. snapshot/root Unit UUID 从 deliverable submission UUID 确定性派生，使同一
   submission 的 ambiguous retry 可 exact replay；
5. finalization 失败把 gate 收紧为 BLOCK，不发 `stage_passed`、不进入下一阶段。

只有显式绕过 Scoping 的 post-Scoping direct entry 才可把已解析 CLI scope 与
operation create 同事务预冻结。

## 5. 边界与验收

- company-only Scoping 可以完成组织身份确认并进入被动 Target Intel；Target Intel → EAS
  仍因 exact target 为空而 HOLD。
- HOLD 发生在 generic phase approval、stage transition、executor/tool work 和历史 target
  DB read 之前。
- 正向 loopback 只有本次显式 exact URL/IP/domain/CIDR/wildcard intake 才进入
  `ConfirmedTargetIntake`，并仍由 DB snapshot 校验。
- 所有 focused fixture 使用 `red_team`；不运行真实 provider、LLM、外部目标或付费请求。
- 本补遗不放松 evidence、scope ownership、active scan、Candidate review 或 resume gate，
  也不新增 schema/migration/IPC 类型。
