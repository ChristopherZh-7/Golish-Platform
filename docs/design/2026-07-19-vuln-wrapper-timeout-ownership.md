# Vuln Wrapper 超时所有权修复

## 问题

`vuln_nuclei_general` 与 `vuln_nuclei_fingerprint_targeted` 是后端守卫的前台能力：调用方可给出 `10..=600` 秒的扫描预算，底层 foreground runner 负责在该预算到期时杀掉进程、收集输出，wrapper 再解析结果并把 evidence 与 `technique_outcomes` 写入数据库。

SubAgent executor 还在这两个 wrapper 外套了一层固定工具超时。当前 Vuln Scanner 的 idle/tool fallback 为 300 秒，因此一个合法的 `timeout_secs=600` 调用会在 300 秒被外层 `tokio::time::timeout` 丢弃。被丢弃的 wrapper 不能继续解析或落库；已经写入的 pre-launch marker 保持 `partial`，而底层进程可继续到自己的 deadline。模型只看到通用 300 秒错误，于是重复同一目标，消耗 Worker 的 40 iteration 预算并制造重叠扫描。

最新运行提供了直接证据：同一 Worker 对 `https://129.28.12.57:443` 先以 300 秒、再以 600 秒调用 General wrapper，所有请求都精确在外层 300 秒返回 `Sub-agent tool ... timed out`；同批其他目标在 165–226 秒完成并正常落五个 outcomes。说明阻塞点不是 Gate，而是外层 deadline 覆盖了 wrapper 声明的预算。

## 决策

把两个 Nuclei wrapper 归入“self-bounded guarded bridge”工具：SubAgent executor 不再对它们应用通用外层工具超时。

- Nuclei foreground runner 保持唯一的扫描 deadline，并在到期时 kill/settle child。
- Wrapper 始终有机会解析 complete/partial/error，并按当前 authority 落 evidence/outcome。
- `timeout_secs=600` 不再被 300 秒 fallback 静默缩短。
- `vuln_probe_anonymous_access` 仍保留通用外层保护；它自己的最大预算是 120 秒，不存在同一冲突。
- Gate 语义不变：超时仍是 partial/error，不能伪装为 checked-empty；只有真实完成或后端有证据的 terminal outcome 才能通过。
- 用户 Stop/应用退出后的 outcome-unknown 继续走现有 Worker recovery，不自动重放有副作用的扫描。

## 范围

只修改 `golish-sub-agents` 的外层超时分类和对应回归测试，并同步 executor 模块卡。没有 IPC、schema、migration、前端或 Gate 规则变更。

## 验证

先让回归测试在旧分类下失败，再实现最小分类变更并运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -E 'test(long_guarded_bridge_tools_bypass_sub_agent_outer_timeout)'
just space-guard
cd backend && cargo clippy -p golish-sub-agents --all-targets -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-sub-agents/src/executor/response_parsing.rs
```

本轮按用户最新指令不启动真实 CLI/外部扫描，因此这些定向验证只能证明超时所有权契约已修；整条 CLI 实跑仍是后续闭环门槛。
