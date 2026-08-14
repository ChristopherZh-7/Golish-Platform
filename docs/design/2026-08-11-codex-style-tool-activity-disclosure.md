# Codex 式工具活动与命令披露设计

> Superseded by `docs/design/2026-08-11-tool-execution-detail-variants.md`，后者保留本设计的 disclosure 层级，并补齐进程内 HTTP 执行与 CLI wrapper 真实命令透传。

## 背景

Company Controller 的 selected-Agent transcript 当前把普通 `SubAgentToolCall` 的 `streamingOutput`、`result` 或 `args` 直接 `safeStringify` 后放进 `<pre>`。对 `eas_discover_ports` 这类由 AI Tool 包裹的命令行能力，默认视图因此被 `completion_state`、`capability`、`generic_evidence_disabled`、`hint` 等机器字段占满，而用户真正关心的底层命令被埋在 JSON 中。

本设计把工具调用呈现为可逐层展开的活动记录，同时保留 Golish 的 evidence-first 边界：展示层可以说明“执行了什么”，但不能从命令、stdout 或自然语言推导 Gate、coverage 或 evidence 成功。

## 用户体验

selected-Agent transcript 中连续的普通工具调用按一次活动组呈现：

```text
🔧 扫描了端口，探测了 Web 服务                         ▾
```

展开活动组后显示每个工具：

```text
◉ 使用 Naabu 扫描 4 个目标                         后台运行  ▾
✓ 使用 HTTPX 探测 Web 服务                         已完成    ▾
```

继续展开单个工具后，优先显示后端返回的真实命令和实际输出：

```text
$ naabu -list '/…/pentest-input.txt' -iv 4 -top-ports 1000 …

Job c41a9755 仍在后台运行，等待输出。
```

最深层的 `Raw Tool Data` 才显示完整 Input/Result。命令行是产品证据入口，不是被 Raw JSON 取代的调试附录。

## 展示层级

1. **活动摘要**：确定性的人类动词，合并同一 transcript burst 内连续的普通工具调用。
2. **工具列表**：工具/runner、目标或输入摘要、执行状态。
3. **命令与输出**：只显示后端实际返回的 `command`；stdout/stderr/partial output 使用终端样式。
4. **Raw Tool Data**：完整 args/result 的结构化 fallback，默认折叠。

`update_plan` 继续由 `AgentPlanCard` 独立呈现；`stage_team_dispatch_workers` 和 `sub_agent_*` 继续使用现有 SubAgent 派发卡。它们会打断普通工具活动组，不被合并成命令活动。

## 确定性 presentation adapter

新增纯前端 adapter，把 `SubAgentToolCall` 转为稳定的 view model：

```ts
export interface ToolActivityPresentation {
  action: string;
  completedAction: string;
  runner: string | null;
  subject: string | null;
  command: string | null;
  commandProvenance: "executed" | "requested" | null;
  stdout: string | null;
  stderr: string | null;
  hint: string | null;
  jobId: string | null;
}
```

- `eas_discover_ports` 使用“扫描端口”，runner 为 Naabu。
- 已有 `getToolActionLabel` / `getToolPrimaryArg` 作为通用 label/subject fallback。
- `command` 优先只读解析后 result 的 exact `command` 执行事实。只有 `run_command` / `run_pty_cmd` / `shell` 尚未返回该字段时，才展示 args 的 exact `command`，并明确标为 `requested`；EAS / `pentest_run` 绝不根据 flags、hint、tool name 或固定 recipe 重建命令。
- 命令展示与复制保留 raw string，不复用会把字面 `\\n` / `\\t` 改写成换行/空格的 display formatter。
- result 可以是 object 或 JSON string；最多只做一次 JSON object 归一化。
- output 只读取 exact `stdout` / `partial_stdout` / `output` 与 `stderr` / `partial_stderr`；不从任意 JSON prose 推断终端输出。
- `streamingOutput` 是 live output 的第一优先级；因为当前 event stream 已混合 stdout/stderr，存在时不再并排追加 partial stderr，避免重复展示。
- Raw Input/Result 始终保留；presentation adapter 不删除原始事件。

## 分组边界

只把相邻的 generic `tool_call` transcript entry 合并。以下事件都会结束当前组：

- text / thinking；
- 最新有效 `update_plan`；
- `stage_team_dispatch_workers`；
- `sub_agent_*`；
- 缺失对应 `SubAgentToolCall` 的 transcript entry。

组标题按首次出现顺序去重动作，最多显示两个动作；更多动作显示“以及 N 项操作”。单工具也沿用相同 disclosure，保证交互一致。

## 状态与安全边界

- running/backgrounded 使用 spinner；completed 使用 success；error/interrupted 使用 warning/error。
- 执行状态只描述进程调用，不等价于 coverage、Gate 或 evidence 状态。
- 只有权威结构化字段存在时才能额外显示 coverage/evidence badge；本次不新增这些推导。
- 命令展示层不显示或重建凭据。当前 result 中若未来可能含秘密，后端必须在写入 transcript 前完成脱敏；前端不得尝试用不完整正则承担唯一脱敏责任。
- 非命令型工具显示结构化 input/result，不伪造 `$ command`。

## 范围

本次只改 frontend presentation、focused tests 与模块卡；不改 Rust、IPC、数据库 schema、工具执行或 evidence/Gate 合同。
