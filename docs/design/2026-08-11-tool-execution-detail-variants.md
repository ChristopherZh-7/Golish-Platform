# 工具执行详情变体设计

## 背景

首版 Codex 式 Tool Activity 只把顶层 `command/stdout/stderr/job_id/hint` 适配为终端详情。真实的 `vuln_probe_anonymous_access` 结果没有这些字段，因此展开后只剩 Raw Data；另一方面，`vuln_nuclei_general` 与 `vuln_nuclei_fingerprint_targeted` 虽然通过 `PentestRunTool` 启动 Nuclei，facade 返回结果却丢弃了 runner 的 `command`，前端同样无法展示。

这两个缺口不是同一种执行：匿名访问探测由 Rust `reqwest` 在进程内直接发 GET/HEAD，不存在 shell command；Nuclei wrapper 则存在真实 active scan command。展示层必须按事实来源区分，不能为了统一视觉伪造 `$ curl`，也不能从 wrapper args 重建 Nuclei 命令。

## 用户体验

### 进程内 HTTP

展开 `vuln_probe_anonymous_access` 后显示：

```text
HTTP requests                                      In process
Origin  https://app.example.test:443

GET  /admin/tasks                    200  Suspicious
HEAD /profile            request timeout  Inconclusive
```

继续展开单条 observation 后显示服务端已经记录的 Query overrides、response fingerprint、redirect/error 等审计字段。`selected_count=0` 或全部未发送时明确显示 `No HTTP requests were sent`，而不是空白。

### CLI wrapper

Nuclei active scan 真正完成 runner launch 后，外层 ToolResult 的白名单 `runner_execution` 携带 runner 返回的 exact `command`：

```text
$ nuclei -u https://app.example.test:443/ ...
```

preflight、template proof 或 authorization 在 active scan 前失败时不添加 command；前端继续诚实地不显示终端命令。wrapper 自己的业务 `exit_code` 不改义，底层进程退出码只以 `wrapped_exit_code` 暴露。

## 数据合同

### Frontend presentation

保留现有 command presentation 字段，并增加只读 HTTP variant：

```ts
export interface HttpExecutionPresentation {
  kind: "http";
  origin: string | null;
  selectedCount: number | null;
  networkAttempted: boolean | null;
  completionState: string | null;
  requests: HttpRequestPresentation[];
}

export interface ToolActivityPresentation {
  // existing action/runner/command/output fields
  execution: HttpExecutionPresentation | null;
}
```

只有 `vuln_probe_anonymous_access` 可以从顶层 result 构造此 variant。result 仍只允许 native plain object 或一层 JSON-string object；不递归搜索 nested payload。

每个 request 只接受 exact `method/path/network_attempted/status_code/verdict/error_class/response/query_bindings`。`path` 不含 persisted query，`query_bindings` 只代表安全 replacement，因此 UI 分开显示 Origin、Method+Path 与 `Query overrides`，绝不拼成声称完整的 URL。

### Nuclei facade result

active runner 返回后，facade 只把受信 runner result 的执行事实复制进 nested `runner_execution`：

- `command`：非空 exact string；
- `exit_code` 与 `duration_ms`：runner 的 exact process metadata；
- `stdout_truncated` / `stderr_truncated`：存在时的 exact boolean，用于说明报告输入是否截断。
- `stdout_original_bytes` / `stderr_original_bytes`：存在时的 exact bounded-output metadata。

outer `exit_code` 继续表示 wrapper/business completion，绝不被进程退出码覆盖。本次不把完整 stdout/stderr、input file 或 error 再复制进 facade result：结构化 `report` 已经是 Nuclei 的用户结果，重复大输出会放大 transcript/DOM并扩大敏感结果暴露。现有 parser 仍直接消费 runner stdout/stderr，业务/evidence 语义不变。前端只读取 exact `result.runner_execution.command` 路径，不做任意 nested command 搜索。

## 安全与 truth 边界

- 不生成 `$ curl`；如果未来提供“等价 curl”，必须显式标为 equivalent、不能进入 executed command 字段。
- 不从 `runner_args`、template ids、tool name、report 或 error prose 重建命令；`runner_execution.command` 缺失时整个 execution summary 缺失。
- Nuclei 只透传 active scan runner 的 command；离线 `nuclei -tl` template proof 不冒充 active scan。
- HTTP 状态、verdict 和 response fingerprint 只展示 producer 已记录的数据，不推导 Gate、coverage、Finding 或 evidence 成功。
- Query override 继续服从 producer 的敏感名与安全 scalar 校验；前端不承担唯一脱敏责任。
- Raw Tool Data 保持最深层 fallback，presentation 不删除或改写原事件。

## 影响范围

- Frontend：`toolActivityPresentation`、`ToolActivityDisclosure` 与 focused tests。
- Backend：`golish-pentest-app::pentest_bridge::vuln_capabilities` 的 Nuclei facade execution annotation 与纯单测。
- 文档：frontend components、`golish-pentest-app/pentest_bridge` 模块卡和模块索引。
- 不改 IPC、ts-rs、数据库 schema/migration、authorization、network policy、evidence/Gate 合同。
