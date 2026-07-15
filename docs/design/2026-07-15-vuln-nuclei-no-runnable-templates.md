# Vuln Nuclei 无可运行受信模板的终态设计

## 背景

“广州有创网络科技有限公司”真实 `Scoping → Attack Candidate` CLI 验收在
`vuln_triage` 暴露出确定性死循环：`vuln_nuclei_general` 的离线 `-tl` proof 能列出
按 tag 匹配的模板，但 active Nuclei 在固定 `-dut` 安全策略下返回：

```text
Could not run nuclei: no templates provided for scan
```

当前 parser 把该结果统一写成 `partial`。worklist 因而持续建议同一个 wrapper，worker
反复执行相同扫描，最终耗尽 retry budget；gate 始终看到 8 个非终态 WSTG cell。

## 语义判断

该 fatal 与普通 scanner error 不同：

- 必须同时满足 non-zero exit、空 stdout、未截断输出，以及 stderr 含 Nuclei 的精确
  `no templates provided for scan` fatal；
- 它表示 `-dut` 下没有任何可运行的受信模板，Nuclei 在发出目标请求前停止；
- 不能移除 `-dut`，也不能把结果伪造成 `checked_empty`；
- 这是当前本地受信 scanner capability 对该 technique 的 evidence-backed blocker，应成为
  terminal `blocked`。

未知 stderr、timeout、truncation、shape drift、其他 exit 1 仍保持 `partial/error`，不得被
这个窄规则吞掉。

## 设计

1. `NucleiCompletion` 增加 `Blocked`。
2. parser 只识别上述精确 fatal，并保留可审计的 bounded error reason。
3. landing 将 `Blocked` 写为 technique outcome `blocked`；不写 Finding，不写
   `checked_empty`。
4. facade 把已成功 guarded landing 的 `Blocked` 视为 wrapper `complete`，防止 agent
   重试；响应和 evidence 的 `network_attempted=false`，因为该 fatal 在 Nuclei target
   request 前产生。
5. 其他 parser/landing 语义不变；generation CAS、authorization、operation epoch、exact
   origin 与 evidence guard 全部复用原路径。

## 验证

- RED：精确 fatal 当前解析为 `Error`。
- GREEN：精确 fatal 解析为 `Blocked`；相邻未知 fatal 仍为 `Error`。
- focused `golish-pentest-app` vuln tests。
- 重新跑真实 CLI slice，确认 Vuln worklist 不再重复，terminal blocked cells 可被 gate
  识别，并继续进入 Attack Candidate；仍不得进入 Verification。
