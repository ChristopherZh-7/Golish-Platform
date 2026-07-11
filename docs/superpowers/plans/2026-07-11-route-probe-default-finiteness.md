# Route Probe 默认有限队列实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 将 `route_probe_paths` completion-oriented 缺省递归深度改为 0，同时保留 root wordlist、observed/curated parent probes、显式递归和原有 non-terminal 限制语义。

**架构：** 只改变 route plan 的缺省输入，不改变候选验证、队列、checkpoint 或 outcome 算法。用真实 dry-run HTTP 测试锁定缺省不递归，用现有显式递归路径锁定 opt-in 行为，并同步工具 schema 与 Enumerator 阶段契约。

**技术栈：** Rust 2021、Tokio、serde_json、cargo nextest、Markdown/JSON stage resources。

## 文件清单

- `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`：缺省值、工具 schema、TDD 回归测试。
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`：Enumerator 系统提示词。
- `backend/crates/golish-sub-agents/src/defaults/tests.rs`：提示词契约测试。
- `resources/harness/stages/enumeration/methodology.md`：阶段执行方法。
- `resources/harness/stages/enumeration/spec.json`：gate recovery hint。
- `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：模块单一事实源。
- `docs/design/2026-07-11-route-probe-default-finiteness.md`：决策与兼容边界。

## 任务 1：先写 route 默认行为红灯测试

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`

**步骤：**

1. 扩展 schema 测试，要求 `wordlist_recursion_depth` 声明 `default=0`、范围
   `0..6`，且说明显式 1..6 才开启 positive-hit recursion。
2. 新增 dry-run HTTP 测试，省略 `wordlist_recursion_depth`，返回一个 root
   positive 和一个只可能由递归访问的 child positive；断言 root 被检查、
   `wordlist_recursion_depth=0`、`recursive_expansions=0` 且 child 不出现。
3. 在既有显式 depth=1 测试中断言结果仍回显 1 且 child 被发现。
4. 新增 candidate-generation/queue outcome 回归断言：生成受限意味着
   `queue_completed=false`，`route_probe_outcome` 与 `route_probe_status` 保持 partial。

**验证：**

```bash
cd backend && cargo nextest run -p golish-pentest-app route_probe_schema_bounds_and_requires_batch_entries route_probe_default_plan_runs_root_wordlist_without_recursive_expansion
```

预期：实现前至少缺省值/缺省递归断言失败，失败值为当前默认 3。

**提交：** 本协作任务按上级要求不创建 commit。
## 任务 2：最小实现缺省值与 schema

**文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`

**步骤：**

1. 将常量改为：

```rust
const DEFAULT_WORDLIST_RECURSION_DEPTH: usize = 0;
```

2. 更新工具总描述，明确默认 root wordlist、observed/curated parent probes 和
   opt-in positive-hit recursion。
3. 为参数 schema 增加：

```json
{"default": 0, "minimum": 0, "maximum": 6}
```

4. 保留现有 `bounded_usize_arg(..., 0, 6)`，确保显式值不被默认覆盖。

**验证：**

```bash
cd backend && cargo nextest run -p golish-pentest-app route_probe
```

预期：新增红灯转绿，现有 checkpoint、outcome、soft-404、explicit recursion 测试全绿。

**提交：** 本协作任务按上级要求不创建 commit。

## 任务 3：同步 Enumerator 与阶段契约

**文件：**

- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- `backend/crates/golish-sub-agents/src/defaults/tests.rs`
- `resources/harness/stages/enumeration/methodology.md`
- `resources/harness/stages/enumeration/spec.json`
- `docs/modules/backend/golish-pentest-app/pentest_bridge.md`

**步骤：**

1. 将“默认递归队列”改为“root wordlist + observed/curated parent probes；默认不对子目录重复整份 wordlist”。
2. 明确 completion flow 省略该参数，显式 1..6 是 opt-in；candidate-generation
   limit 继续保持 partial。
3. 在 Enumerator prompt 测试中断言上述关键短语，防止文档与 runtime 漂移。
4. 不改 gate terminal ownership、checkpoint identity 或外部工具 allow-list。

**验证：**

```bash
cd backend && cargo nextest run -p golish-sub-agents test_enumerator_prompt_is_content_enum
python3 -m json.tool resources/harness/stages/enumeration/spec.json >/dev/null
```

预期：prompt 测试与 JSON 解析全绿。

**提交：** 本协作任务按上级要求不创建 commit。

## 任务 4：格式与回归验证

**文件：** 以上全部修改文件。

**步骤：**

1. 运行 `cargo fmt --check`。
2. 运行两个受影响 crate 的完整 nextest。
3. 运行两个受影响 crate 的 clippy，拒绝 warning。
4. 检查 diff，确认没有覆盖并行的 checkpoint epoch 修改，且未修改
   `feature_list.json`、`agent-progress.md`、DB 或 migration。

**验证：**

```bash
cd backend && cargo fmt --check
cd backend && cargo nextest run -p golish-pentest-app -p golish-sub-agents --status-level fail
cd backend && cargo clippy -p golish-pentest-app -p golish-sub-agents --all-targets -- -D warnings
git diff --check
```

预期：所有命令退出码 0。

**提交：** 本协作任务按上级要求不创建 commit。
