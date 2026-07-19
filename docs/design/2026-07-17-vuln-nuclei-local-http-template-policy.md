# Vuln Nuclei 本地 HTTP 模板加载策略

## 背景

真实 `vuln_triage` 日志证明 Nuclei v3.8.0 的 `-tl -dut` 能列出本地模板，但使用相同
过滤器正式执行时把带 ProjectDiscovery digest 的模板报告为 `unsigned`，随后在发出任何目标
请求前以 `no templates provided for scan` 退出。loopback 最小复现进一步证明：同一模板保留
全部协议、tag、并发与目标参数，仅移除 `-dut` 后能够正常加载并访问 `127.0.0.1`。

用户提供的 `adysec/nuclei_poc` 已按上游推荐稀疏导入本机模板根的
`adysec-nuclei_poc/poc_gold_13`。其中大量 HTTP 模板没有当前 Nuclei 可接受的签名，因此在
`-dut` 下同样不会执行。

## 决策

Vuln 和 Verification 的 Nuclei wrapper 从“签名开关信任”改为“本地目录 + 协议能力 +
业务授权”信任边界：

1. active plan 与离线 proof 同时移除 `-dut`，消除 proof/execute 策略漂移；
2. 模板只能来自后端解析并在 spawn 前重验 canonical identity 的本地模板目录；保留 `-duc`，
   扫描时不下载或更新远端模板；
3. 固定 `-pt http,ssl`，不传 `-code`、`-headless`、`-file`、`-esc` 或 `-egm`；因此导入库中的
   code、JavaScript、Headless、文件和全局 matcher 能力不进入执行面；
4. 保留 `-ni`、`-dr`、exact-origin authorization、foreground execution、响应大小、timeout、
   rate/concurrency bounds；
5. general 模式继续排除 `cve,fuzz,dos,bruteforce,intrusive` 并只接受后端 technique→tag 映射；
6. fingerprint-targeted 与 Verification replay 继续只接受服务器从当前 owner 指纹/PoC authority
   选出的 exact template id；模型不能传模板路径或 raw Nuclei 参数；
7. 精确 `no templates provided for scan` 仍落 terminal `blocked`，未知 stderr/timeout/truncation
   继续 fail closed，不能伪造成 `checked_empty`。

这里允许的是本地 HTTP/SSL DSL 模板，不是任意宿主代码执行。模板仍可能向已授权目标发出
主动 HTTP 请求，因此已有 engagement scope、Vuln stage、exact origin、危险标签和速率限制全部
保持不变。

## 导入边界

- Golish 不 vendor 或修改第三方 GPL 模板仓库；本机模板根通过已有
  `GOLISH_NUCLEI_TEMPLATES_DIR` / Nuclei config / home fallback 解析。
- 当前本机导入固定到 `adysec/nuclei_poc@28e82b100a2bd6215be2c3cb87980aaf6eb1ea7e`，
  sparse path 为 `poc_gold_13`。
- macOS 大小写不敏感文件系统会合并上游仅大小写不同的模板路径；这是本地模板供应链事实，
  不改变后端 exact template-id 与 evidence 合同。

## 验证

- planner 单测证明 active/proof/replay 均不再传 `-dut`，同时保留本地目录、`-duc`、
  `-pt http,ssl`、危险 tag、foreground 和 bounded execution 守卫；
- parser/landing 现有 blocked/error/checked-empty 测试保持通过；
- `cargo nextest` 只跑 `golish-pentest-app` Nuclei 定向测试；
- loopback HTTP 集成用官方 digest 模板与导入模板证明正式执行会加载模板且只访问
  `127.0.0.1`；不扫描真实目标；
- 按用户指令不运行 `init.sh` 或 `just precommit`，因此本轮不宣称完整仓库 DoD。
