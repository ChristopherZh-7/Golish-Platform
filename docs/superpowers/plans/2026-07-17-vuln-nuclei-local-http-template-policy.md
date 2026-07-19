# Vuln Nuclei 本地 HTTP 模板加载策略实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 修通 Nuclei proof 与正式执行的模板加载链路，并让本机导入的 `poc_gold_13` 在受限 HTTP/SSL 执行面中可用。

**架构：** active、proof 和 exact replay 共享同一本地模板加载策略，移除在当前 Nuclei 安装上拒绝全部模板的 `-dut`。安全边界由 canonical 本地目录、HTTP/SSL-only 协议、后端 technique/exact-id 选择、危险标签排除、exact-origin 授权和有界 foreground runner 共同承担。

**技术栈：** Rust 2021、Nuclei v3 CLI、`golish-pentest-app` adapter tests、loopback HTTP server。

## 文件结构

- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`：统一 active/proof/replay 模板加载参数。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`：固定 planner 安全合同回归。
- 修改 `docs/modules/backend/golish-pentest-app/pentest_bridge.md` 与 `docs/modules/INDEX.md`：同步公开安全边界。
- 修改 `agent-progress.md`：记录复现、验证命令和未运行 full gate 的事实。

## 任务 1：先固定失败合同

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`
- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`

**步骤 1：** 将 planner 断言改为要求 active、proof、exact replay 都不含 `-dut`，并逐项要求以下守卫：

```rust
assert!(!args.contains("-dut"));
assert!(args.contains("-duc"));
assert!(args.contains("-pt http,ssl"));
assert!(!args.contains("-code"));
assert!(!args.contains("-headless"));
assert!(!args.contains("-file"));
```

**步骤 2：** 运行 RED：

```bash
cd backend && cargo nextest run -p golish-pentest-app -E 'test(/plans_are_foreground_server_owned_and_do_not_accept_raw_arguments|every_nuclei_plan_binds_the_same_shell_quoted_local_template_tree|nuclei_template_proof_plans_are_offline_foreground_and_server_owned/)' --status-level fail
```

预期：旧 planner 仍含 `-dut`，新增断言失败。

## 任务 2：实现统一的本地 HTTP/SSL 模板策略

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`

**步骤 1：** 从 general、targeted、exact replay 及两类 proof 参数中移除 `-dut`；保留：

```text
-t <canonical-local-directory> -duc -pt http,ssl
```

general 继续使用：

```text
-ni -dr -etags cve,fuzz,dos,bruteforce,intrusive
```

targeted/replay 继续使用服务器拥有的 `-template-id`，且不接受模型路径或 raw args。

**步骤 2：** 在 planner 附近添加注释，说明 `-dut` 在当前安装上使 `-tl` 与正式执行产生策略漂移；HTTP/SSL-only 能力边界禁止宿主代码型模板进入执行面。

**步骤 3：** 重跑任务 1 的 focused tests，预期全部通过。

## 任务 3：定向集成验证

**文件：** 不新增生产文件。

**步骤 1：** 先运行 Rust 定向测试：

```bash
cd backend && cargo nextest run -p golish-pentest-app -E 'test(/nuclei|vuln_adapter/)' --status-level fail
```

预期：全部通过，无 scanner 或外部网络调用。

**步骤 2：** 在 `127.0.0.1` 启动临时 HTTP server，使用 planner 同形参数分别执行一个官方 HTTP 模板和一个 `adysec-nuclei_poc/poc_gold_13` HTTP 模板。预期不再出现 `no templates provided for scan`，Nuclei exit 0；server 日志只出现 loopback 请求。

**步骤 3：** 运行静态检查：

```bash
cargo fmt --all -- --check
git diff --check
```

预期均 exit 0。不得运行 `init.sh` 或 `just precommit`。

## 任务 4：同步文档和证据

**文件：**

- 修改：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`agent-progress.md`

**步骤 1：** 在模块卡记录本地 unsigned HTTP/SSL template policy、第三方模板边界以及禁止的协议能力；INDEX 只更新该模块说明/日期，不改其他功能状态。

**步骤 2：** 在 progress 记录 loopback RED/GREEN、focused nextest、fmt/diff-check 的命令、退出码和关键输出；明确没有扫描真实目标，未运行 `init.sh`/`precommit`，完整 DoD 未满足。

**步骤 3：** 不切换共享工作树现有唯一 `in_progress` feature；本改动是已 passing 的 `vuln-observation-candidate-closure-2026-07-14` 的运行时缺陷修复，完整门禁受用户指令约束。
