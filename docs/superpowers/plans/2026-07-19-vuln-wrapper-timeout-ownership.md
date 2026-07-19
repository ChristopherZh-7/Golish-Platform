# Vuln Wrapper 超时所有权实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让守卫的 Nuclei wrapper 完整拥有其 10–600 秒前台执行预算，避免 SubAgent 的固定 300 秒外层超时丢弃 wrapper 并切断结果落库。
**架构：** 保留底层 foreground runner 的 deadline、kill、输出解析与 evidence/outcome landing；只在 SubAgent executor 的 timeout 分类中把两个 Nuclei wrapper 标为 self-bounded。通用工具和最大 120 秒的 anonymous-access wrapper 继续使用现有外层超时。
**技术栈：** Rust 2021、Tokio、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`：扩展 self-bounded bridge 分类并承载单元回归。
- 修改 `docs/modules/backend/golish-sub-agents/executor.md`：记录 Nuclei wrapper 的唯一 deadline/landing 契约。
- 修改 `docs/modules/INDEX.md`：更新模块卡的新鲜度说明。
- 修改 `agent-progress.md`：记录 RED/GREEN 命令、退出码、未执行 CLI 的剩余风险。

## 任务 1：用失败测试锁定 600 秒 wrapper 不得被 300 秒外层截断

**文件：** 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**步骤 1：编写失败测试。** 把现有 long bridge 分类测试改名，并把两个 Nuclei wrapper 加入必须绕过外层 timeout 的表驱动断言；同时保留 generic tool 与 anonymous-access 仍使用外层 timeout 的反例。

```rust
#[test]
fn long_guarded_bridge_tools_bypass_sub_agent_outer_timeout() {
    for tool_name in [
        "vuln_nuclei_general",
        "vuln_nuclei_fingerprint_targeted",
    ] {
        assert!(!use_sub_agent_outer_tool_timeout(tool_name));
    }
    assert!(use_sub_agent_outer_tool_timeout("vuln_probe_anonymous_access"));
    assert!(use_sub_agent_outer_tool_timeout("query_target_data"));
}
```

**步骤 2：验证 RED。** 每次 Cargo 前先运行空间守卫。

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -E 'test(long_guarded_bridge_tools_bypass_sub_agent_outer_timeout)'
```

预期：测试因 `vuln_nuclei_general should keep running...` 断言失败，退出码非 0；失败原因是生产分类尚未包含 wrapper。

**提交：** 本轮不创建 commit；共享工作树包含用户的既有未提交改动，只保留精确 diff 并在 progress 中列明。

## 任务 2：实现最小 timeout 分类修复

**文件：** 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**步骤 1：把两个 Nuclei wrapper 加入现有 `matches!` 豁免列表。**

```rust
"vuln_nuclei_general"
    | "vuln_nuclei_fingerprint_targeted"
    | "browser_collect_js_api"
```

同步注释：这些 wrapper 自己拥有 bounded foreground runner timeout，必须等待 wrapper 完成 landing；不得承诺 timeout 是 terminal coverage。

**步骤 2：验证 GREEN。**

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -E 'test(long_guarded_bridge_tools_bypass_sub_agent_outer_timeout)'
```

预期：目标测试通过，退出码 0。

**提交：** 本轮不创建 commit；原因同任务 1。

## 任务 3：做受影响 crate 的定向回归与文档收尾

**文件：** 修改 `docs/modules/backend/golish-sub-agents/executor.md`、`docs/modules/INDEX.md`、`agent-progress.md`

**步骤 1：运行 executor 相邻 timeout 测试与 scoped Clippy。**

```bash
just space-guard
cd backend && cargo nextest run -p golish-sub-agents -E 'test(long_guarded_bridge_tools_bypass_sub_agent_outer_timeout) | test(background_true_failure_gets_runtime_correction)'
just space-guard
cd backend && cargo clippy -p golish-sub-agents --all-targets -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-sub-agents/src/executor/response_parsing.rs
git diff --check -- backend/crates/golish-sub-agents/src/executor/response_parsing.rs docs/design/2026-07-19-vuln-wrapper-timeout-ownership.md docs/superpowers/plans/2026-07-19-vuln-wrapper-timeout-ownership.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/INDEX.md agent-progress.md
```

预期：全部退出码 0，Clippy 无 warning，格式和 diff 检查干净。

**步骤 2：记录边界。** `agent-progress.md` 必须写明真实 CLI 未运行，因此功能只达到本地定向验证，不标记整链路闭环。

**提交：** 本轮不创建 commit；原因同任务 1。

## 自检

- 规格覆盖：600 秒 Nuclei 调用不再被 300 秒 fallback 截断；generic/anonymous-access 反例保留；Gate 的 partial 语义不变；未执行 CLI 的限制被记录。
- 占位内容扫描：计划中的实现、测试与验证命令均已明确给出。
- 类型一致性：沿用现有 `use_sub_agent_outer_tool_timeout(&str) -> bool`，不新增公共类型或跨 crate 接口。
