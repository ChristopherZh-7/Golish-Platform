# Repo ownership baseline-ref ratchet 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 为 Runtime Memory / Candidate Pipeline V2 收口提供基于 checkpoint `13b29628` 的 no-new-violation ownership 门禁，同时保留 full checker 的真实历史失败状态。
**架构：** 将现有扫描器抽成只读 snapshot 输入，worktree 与 git ref 都使用当前脚本的同一规则生成精确违规集合；仅 `current - baseline` 阻断。Git ref 通过 blob/tree 读取，不 checkout、不执行旧脚本。
**技术栈：** Python 3 标准库、`unittest`、Git CLI。

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `scripts/check_repo_ownership.py` | 修改 | snapshot 扫描、exact set 比较、`--baseline-ref` CLI |
| `scripts/tests/test_check_repo_ownership.py` | 修改 | 临时 git 仓库中的 baseline/current/removed 回归 |
| `docs/design/2026-07-14-repo-ownership-baseline-ref-ratchet.md` | 新建 | 语义、安全边界、baseline 选择和非绿声明 |
| `docs/superpowers/plans/2026-07-14-repo-ownership-baseline-ref-ratchet.md` | 新建 | TDD 与验证步骤 |

## Task 1：先冻结 exact-set 语义

**文件：** 修改 `scripts/tests/test_check_repo_ownership.py`

### 步骤 1：写失败测试

在临时 git 仓库提交一个 agent→recon 的已知历史违规，再只在 worktree
新增 raw SQL 文件。测试调用以下公开 seam：

```python
baseline = collect_violations(GitRefSnapshot(root, baseline_ref))
current = collect_violations(WorktreeSnapshot(root))
added, removed = compare_violation_sets(current, baseline)
```

精确断言 `added` 只有新 raw-SQL tuple，历史 ownership tuple 仍在 baseline；
删除历史文件后再断言它只进入 `removed`。

### 步骤 2：运行 RED

```bash
python3 -m unittest scripts.tests.test_check_repo_ownership.BaselineRefRatchetTests -v
```

预期在实现前以缺少 `collect_violations` 失败，不能接受断言本身出错或
fixture 未提交导致的假 RED。

## Task 2：实现共享 snapshot 扫描与 CLI

**文件：** 修改 `scripts/check_repo_ownership.py`

### 步骤 1：实现只读 snapshot

提供以下接口：

```python
class SourceSnapshot(Protocol):
    def read_text(self, relative: str) -> str | None: ...
    def iter_paths(self, prefix: str, suffixes: frozenset[str]) -> list[str]: ...

class WorktreeSnapshot: ...
class GitRefSnapshot: ...
```

`WorktreeSnapshot` 必须包含 untracked 文件。`GitRefSnapshot` 必须先用
`git rev-parse --verify <ref>^{commit}` 固定 commit，再用 `git ls-tree -z`
和 `git show` 读取；全部 `subprocess.run([...], shell=False)`。

### 步骤 2：让现有规则共用 snapshot

将 source-root、repo declaration 和 Finding insert 扫描改为接受
`SourceSnapshot`。保留 `scan()`、`declared_repos()` 与
`scan_finding_insertions(root)` worktree wrapper，避免破坏既有测试和 CLI。

精确集合固定为：

```python
Violation = tuple[str, str]

def compare_violation_sets(current, baseline):
    return current - baseline, baseline - current
```

禁止按文件名粗略忽略、按数量相减或执行 baseline 版本脚本。

### 步骤 3：增加严格 CLI

只接受 no option、`--finding-writes-only`、`--emit-allowlist` 或
`--baseline-ref <git-ref>` 四种互斥形式。新增集合为空返回 0；非空逐条
打印 `[category] message` 并返回 1；ref/路径/用法错误返回 2。成功输出
必须包含 `historical violations not asserted clean`。

### 步骤 4：运行 GREEN

```bash
python3 -m unittest scripts.tests.test_check_repo_ownership -v
python3 -m py_compile scripts/check_repo_ownership.py scripts/tests/test_check_repo_ownership.py
```

预期 2 个测试通过，compile exit 0。

## Task 3：运行真实 checkpoint gate，修复新增违规

**文件：** 由违规所属模块 owner 修改；本 Task 禁止修改 ownership allowlist
来隐藏新增耦合。

### 步骤 1：运行真实 gate

```bash
python3 scripts/check_repo_ownership.py --baseline-ref 13b29628
```

首次 RED 必须记录实际新增集合。当前首次运行得到：

```text
[ownership] golish-agent-app/ai/db_bridge/attack_execution.rs: agent -> repo::operation_state (owned by pentest)
[raw-sql] stage_run/runtime_v2.rs: raw sqlx::query in command layer — route via golish-db repo
```

### 步骤 2：按现有边界消除新增违规

- `attack_execution.rs` 通过既有 pentest port 获取 operation state，不直接
  引用 pentest-owned repo。
- `stage_run/runtime_v2.rs` 将 SQL 下沉到 owning `golish-db` repo，stage-run
  只调用 typed repository API。

每项由模块 owner 补对应 Rust 测试；执行任何 Cargo 命令前先跑
`just space-guard`。不得新增 ALLOWLIST/RAW_SQL_ALLOWLIST 项。

### 步骤 3：复跑 ratchet

```bash
python3 scripts/check_repo_ownership.py --baseline-ref 13b29628
```

预期 exit 0，输出 `OK no new exact violations`，并明确历史违规不被宣称
clean。不要用 `python3 scripts/check_repo_ownership.py` 的现有非零结果作为
本 Task 的绿色证据，也不要把它改写成绿色描述。

## Task 4：静态收尾与提交证据

**文件：** 检查本计划文件结构中的四个文件；进度/功能状态由主会话统一更新。

### 步骤 1：执行无 Cargo 验证

```bash
python3 -m unittest scripts.tests.test_check_repo_ownership -v
python3 -m py_compile scripts/check_repo_ownership.py scripts/tests/test_check_repo_ownership.py
git diff --check -- scripts/check_repo_ownership.py scripts/tests/test_check_repo_ownership.py docs/design/2026-07-14-repo-ownership-baseline-ref-ratchet.md docs/superpowers/plans/2026-07-14-repo-ownership-baseline-ref-ratchet.md
```

预期全部 exit 0。

### 步骤 2：记录两种 truth

在主会话的 `agent-progress.md` / `feature_list.json` 中分别记录：

1. baseline-ref ratchet 的退出码与 exact set；
2. full checker 仍有历史违规，未被宣称为 green。

只有 Task 3 的真实 gate exit 0 后才能把 no-new-violation 项记为通过；该
ratchet 不能替代用户授权 live run、完整 verification matrix 或
`just precommit`。
