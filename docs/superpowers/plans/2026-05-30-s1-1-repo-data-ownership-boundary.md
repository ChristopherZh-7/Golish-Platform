# S1-1 Repo 数据所有权边界守卫 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划，每个任务独立验证 + 独立 commit。

**目标：** 给 `golish-db` 的 ~42 个 repo 建立**服务级数据所有权**，并用一个 CI 守卫脚本（ratchet 模式）阻止命令层产生**新的跨服务直接读表 / 跨层裸 SQL**，为未来 DB-per-service 抽服务打地基。
**架构：** 纯增量、零业务行为变更。不移动任何 repo 文件、不改任何 SQL 语义。新增一个 Python 守卫脚本 `scripts/check_repo_ownership.py`（仿 `scripts/check_dag.py`），把当前已存在的跨服务耦合冻结进 `ALLOWLIST` 作为基线，CI 从此只拦**新增**耦合；每条 allowlist 条目就是后续 S1-2 `*Port` 抽取的清单。
**技术栈：** Python 3.11（守卫，无第三方依赖）、GitHub Actions（`arch-check.yml`）、`just`、Rust workspace（被扫描对象，不改 Rust 代码）。

---

## 背景与依据

- 上层设计：`docs/design/2026-05-30-servitization-readiness.md`（§3 阻碍① 共享单库 / §5 五服务边界 / §6 阶段 1 S1-1）。
- 关键证据（只读体检 2026-05-30）：
  - `golish-db` 一个 `PgPool` 承载 42 repo（`backend/crates/golish-db/src/repo/mod.rs:1-45`），`DbState` 单 pool（`backend/crates/golish/src/state/db.rs:11`）。
  - **`golish-db` 内部已干净**：每个 repo 只 `super::scoped::*` 委托，无 repo 调别的 repo（grep 证据：`repo/findings.rs:84`、`repo/targets.rs:36`、`repo/pipelines.rs:13` 等全部指向 `scoped`）。→ 所以本计划**不动 golish-db 内部**。
  - 真正的跨服务耦合在**命令层** `golish/src`，例：
    - `tools/asset_intel/commands.rs:148,202,281,340,366`（recon）调 `golish_db::repo::organizations`（recon，同服务，OK）；
    - `ai/db_bridge/recon.rs:17,66,91,132,159,189-223`（agent）调 `repo::vuln_intel`(vuln) + `repo::api_endpoints`/`js_analysis`/`fingerprints`/`passive_scans`/`target_assets`(recon) → **跨服务**；
    - `tools/pentest_bridge/vault_ops.rs:167,201`（pentest）调 `repo::vault`(platform)、`js_collect/tool_impl.rs:86`（pentest）调 `repo::targets`(recon) → **跨服务**；
    - `tools/audit.rs:134,165,193,223`（platform）调 `repo::passive_scans`(recon)/`agent_logs`(agent)/… → **跨服务**（审计聚合读，属合理但需登记）。
  - 命令层仍有残余裸 `sqlx::query`（~25 文件，如 `tools/targets/cmds.rs` 14 处、`tools/conversation_store/*` 20 处），与 P0-3「作用域 SQL 下沉」重叠。

### 范围内（In scope）
1. 新增 `scripts/check_repo_ownership.py`：服务级 repo 所有权守卫 + 命令层裸 SQL 次级检查，支持 `--emit-allowlist` 生成基线。
2. 用 `--emit-allowlist` 跑出当前耦合，固化进脚本的 `ALLOWLIST` / `RAW_SQL_ALLOWLIST`（ratchet 基线，CI 即绿）。
3. 接入 CI（`arch-check.yml` 新 job）+ 本地 `just arch` recipe。
4. 文档：`docs/architecture.md` 增「数据所有权」表与演进原则；`agent-progress.md`、`feature_list.json` 收尾。

### 范围外（Out of scope，本计划不做）
- **不**物理重排 repo 为子模块（`repo/recon/…`）——那是高 churn 的后续步骤 S1-1b，等 ALLOWLIST 接近清零后再做。
- **不**引入 `*Port` trait 消除耦合——那是 S1-2。
- **不**改任何 Rust 代码 / SQL / repo 文件 / 命令签名（本计划零 Rust diff）。
- **不**清空残余裸 SQL（属 P0-3；本计划只「冻结基线 + 拦新增」）。

---

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `scripts/check_repo_ownership.py` | **新建** | 守卫脚本：解析 `repo/mod.rs` 强制登记；扫描 `golish/src` 的 `golish_db::repo::<x>` 与裸 `sqlx::query`；按 `REPO_OWNER`/`DOMAIN_RULES`/`ALLOWLIST` 判违规；`--emit-allowlist` 生成基线 |
| `.github/workflows/arch-check.yml` | 修改 | 新增 `repo-ownership` job 运行守卫 |
| `justfile` | 修改 | 新增 `arch` recipe（本地一次跑 `check_dag.py` + `check_repo_ownership.py`） |
| `docs/architecture.md` | 修改 | 在「Evolution principles」补「#5 数据所有权」+ 一张 repo→service 所有权表 |
| `agent-progress.md` | 修改 | 本轮会话记录 + 已记录证据 |
| `feature_list.json` | 修改 | 新增 `arch-s1-1-repo-ownership-guard` 条目 |

---

## Task 1 — 新建守卫脚本（空 ALLOWLIST，先看基线 RED）

**文件：** 创建 `scripts/check_repo_ownership.py`

**步骤 1.1：** 写入以下完整脚本（`ALLOWLIST` / `RAW_SQL_ALLOWLIST` 此刻为空，下一任务填）：

```python
#!/usr/bin/env python3
"""Data-ownership boundary guard for the Golish command layer.

golish-db holds ONE Postgres with ~42 repo modules for ALL services
(recon / vuln / pentest / agent / platform). To prepare for future
DB-per-service servitization, this guard enforces that a command-layer
module only reaches repos owned by ITS OWN service. Existing cross-service
coupling is frozen in ALLOWLIST (a ratchet); NEW coupling is blocked.
Each allowlist entry is a future `*Port` extraction candidate
(design: docs/design/2026-05-30-servitization-readiness.md §6 S1-2).

Rules:
- Every `pub mod X;` in golish-db/src/repo/mod.rs must appear in
  REPO_OWNER or SHARED_REPOS (forces new repos onto the ownership map).
- A `golish_db::repo::<name>` usage in golish/src is legal iff:
    name in SHARED_REPOS, OR
    owner(name) == domain(caller_file), OR
    (caller_file, name) in ALLOWLIST.
- (Secondary) raw `sqlx::query*` in golish/src is legal iff caller_file
  in RAW_SQL_ALLOWLIST (steers new DB access through golish-db repos;
  overlaps with P0-3 scoped-SQL-down-to-repo).

Exit code: 0 clean / 1 violations / 2 setup error.
Use `--emit-allowlist` to print copy-pasteable baseline entries.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

# repo (table group) -> owning service. Mirrors design doc §5 five-service map.
# NOTE: starting assignment, reviewable. The first --emit-allowlist run will
# reveal where caller locations disagree; those become allowlist entries or a
# corrected mapping. The mechanism — not perfect classification — is the point.
REPO_OWNER: dict[str, str] = {
    # recon — asset / attack surface
    "targets": "recon",
    "target_assets": "recon",
    "organizations": "recon",
    "api_endpoints": "recon",
    "sitemap_store": "recon",
    "directory_entries": "recon",
    "fingerprints": "recon",
    "js_analysis": "recon",
    "passive_scans": "recon",
    "sensitive_scan": "recon",
    "screenshots": "recon",
    "custom_rules": "recon",
    "endpoint_tests": "recon",
    # vuln — vulnerability intelligence
    "vuln_intel": "vuln",
    "vuln_scan": "vuln",
    "scan_queue": "vuln",
    "wiki_kb": "vuln",
    "kb_research": "vuln",
    # pentest — engine / pipeline / findings
    "findings": "pentest",
    "methodology": "pentest",
    "pipelines": "pentest",
    "stage_runs": "pentest",
    "execution_plans": "pentest",
    "evidence_classifications": "pentest",
    "operation_state": "pentest",
    "subtasks": "pentest",
    "tasks": "pentest",
    "sprint_contracts": "pentest",
    # agent — LLM orchestration / sessions
    "sessions": "agent",
    "conversation_store": "agent",
    "message_chains": "agent",
    "agent_logs": "agent",
    "tool_calls": "agent",
    "sub_agent_dispatches": "agent",
    "memories": "agent",
    "msg_logs": "agent",
    "prompt_templates": "agent",
    "vector_store_logs": "agent",
    "search_logs": "agent",
    # platform — vault / notes / os logs
    "vault": "platform",
    "notes": "platform",
    "terminal_logs": "platform",
}

# Cross-cutting repos any service may use (evidence ledger + generic SQL
# helper). Not owned by a single service.
SHARED_REPOS: frozenset[str] = frozenset({"audit", "scoped"})

# Ordered (first-match-wins) caller-path-prefix -> service domain.
# Paths are relative to backend/crates/golish/src/.
DOMAIN_RULES: list[tuple[str, str]] = [
    ("tools/asset_intel", "recon"),
    ("tools/organizations", "recon"),
    ("tools/targets", "recon"),
    ("tools/custom_rules", "recon"),
    ("tools/sensitive_scan", "recon"),
    ("tools/scan_runner", "recon"),
    ("tools/scan_queue", "recon"),
    ("tools/intel_providers", "recon"),
    ("tools/integrations", "recon"),
    ("tools/pentest_bridge", "pentest"),
    ("tools/pentest_ai", "pentest"),
    ("tools/pentest", "pentest"),
    ("tools/findings", "pentest"),
    ("tools/methodology", "pentest"),
    ("tools/pipeline", "pentest"),
    ("tools/execution_plans", "pentest"),
    ("tools/evidence", "pentest"),
    ("tools/security_analysis", "pentest"),
    ("tools/vuln_intel", "vuln"),
    ("tools/wiki", "vuln"),
    ("tools/conversation_store", "agent"),
    ("ai/", "agent"),
    ("tools/vault", "platform"),
    ("tools/audit", "platform"),
    ("tools/notes", "platform"),
    ("tools/recordings", "platform"),
]

# Baseline coupling frozen as a ratchet — seed via `--emit-allowlist` (Task 2).
# Each tuple = (caller_file_relative_to_src, repo_name). REMOVING an entry
# means you introduced the corresponding *Port (see design doc §6 S1-2).
ALLOWLIST: frozenset[tuple[str, str]] = frozenset(
    {
        # filled in Task 2
    }
)

# Files allowed to keep raw sqlx for now (overlaps with P0-3). Seed in Task 4.
RAW_SQL_ALLOWLIST: frozenset[str] = frozenset(
    {
        # filled in Task 4
    }
)

REPO_USE_RE = re.compile(r"golish_db::repo::([a-z_][a-z0-9_]*)")
RAW_SQL_RE = re.compile(r"\bsqlx::query")
PUB_MOD_RE = re.compile(r"^pub mod ([a-z_][a-z0-9_]*);", re.MULTILINE)

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "backend" / "crates" / "golish" / "src"
REPO_MOD = ROOT / "backend" / "crates" / "golish-db" / "src" / "repo" / "mod.rs"


def domain_of(rel: str) -> str | None:
    for prefix, dom in DOMAIN_RULES:
        if rel.startswith(prefix):
            return dom
    return None


def is_test_file(rel: str) -> bool:
    return rel.endswith("_tests.rs") or rel.endswith("tests.rs") or "/tests/" in rel


def declared_repos() -> set[str]:
    return set(PUB_MOD_RE.findall(REPO_MOD.read_text()))


def scan() -> tuple[list[str], list[str], set[tuple[str, str]], set[str]]:
    own_viol: list[str] = []
    raw_viol: list[str] = []
    emit_own: set[tuple[str, str]] = set()
    emit_raw: set[str] = set()
    for path in sorted(SRC.rglob("*.rs")):
        rel = str(path.relative_to(SRC))
        if is_test_file(rel):
            continue
        text = path.read_text()
        dom = domain_of(rel)
        for m in REPO_USE_RE.finditer(text):
            repo = m.group(1)
            if repo in SHARED_REPOS:
                continue
            owner = REPO_OWNER.get(repo)
            if owner is None:
                own_viol.append(f"{rel}: uses unregistered repo `{repo}` — add to REPO_OWNER")
                continue
            if dom is None:
                own_viol.append(f"{rel}: caller path has no domain — add a DOMAIN_RULES prefix")
                continue
            if owner == dom or (rel, repo) in ALLOWLIST:
                continue
            own_viol.append(f"{rel}: {dom} -> repo::{repo} (owned by {owner})")
            emit_own.add((rel, repo))
        if RAW_SQL_RE.search(text) and rel not in RAW_SQL_ALLOWLIST:
            raw_viol.append(f"{rel}: raw sqlx::query in command layer — route via golish-db repo")
            emit_raw.add(rel)
    return own_viol, raw_viol, emit_own, emit_raw


def main() -> int:
    if not SRC.is_dir() or not REPO_MOD.is_file():
        print(f"[repo-ownership] ERROR: paths not found ({SRC} / {REPO_MOD})", file=sys.stderr)
        return 2

    own, raw, emit_own, emit_raw = scan()
    for r in sorted(declared_repos() - set(REPO_OWNER) - SHARED_REPOS):
        own.append(f"golish-db repo `{r}` unregistered — add to REPO_OWNER or SHARED_REPOS")

    if "--emit-allowlist" in sys.argv:
        print("# --- paste into ALLOWLIST ---")
        for rel, repo in sorted(emit_own):
            print(f'        ("{rel}", "{repo}"),')
        print("# --- paste into RAW_SQL_ALLOWLIST ---")
        for rel in sorted(emit_raw):
            print(f'        "{rel}",')
        return 0

    if not own and not raw:
        print("[repo-ownership] OK clean")
        return 0
    print(
        f"[repo-ownership] FAIL {len(own)} ownership + {len(raw)} raw-sql violation(s):",
        file=sys.stderr,
    )
    for v in own + raw:
        print(f"  - {v}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
```

**步骤 1.2：** 赋可执行位（与 `check_dag.py` 一致用 `python3` 调用，可执行位非必需但保持习惯）：

```bash
chmod +x scripts/check_repo_ownership.py
```

**验证：** 运行守卫，预期看到一批 RED 违规（基线尚未登记）：

```bash
python3 scripts/check_repo_ownership.py; echo "exit=$?"
```

预期输出形如（数量以实际为准）：

```
[repo-ownership] FAIL N ownership + M raw-sql violation(s):
  - ai/db_bridge/recon.rs: agent -> repo::vuln_intel (owned by vuln)
  - tools/pentest_bridge/vault_ops.rs: pentest -> repo::vault (owned by platform)
  - ...
exit=1
```

> 若出现 `uses unregistered repo` 或 `golish-db repo X unregistered`：说明有新 repo 未登记，把它补进 `REPO_OWNER`（按 §5 服务归属）或 `SHARED_REPOS`，再重跑直到只剩「跨服务」与「raw-sql」两类违规。

**提交：**

```bash
git add scripts/check_repo_ownership.py
git commit -m "feat(arch): add repo data-ownership guard (S1-1, baseline RED)"
```

---

## Task 2 — 用 `--emit-allowlist` 固化基线（GREEN）

**文件：** 修改 `scripts/check_repo_ownership.py`（仅填 `ALLOWLIST`）

**步骤 2.1：** 生成基线条目：

```bash
python3 scripts/check_repo_ownership.py --emit-allowlist
```

它会打印两段，形如：

```
# --- paste into ALLOWLIST ---
        ("ai/db_bridge/recon.rs", "vuln_intel"),
        ("ai/db_bridge/recon.rs", "api_endpoints"),
        ("tools/pentest_bridge/vault_ops.rs", "vault"),
        ...
# --- paste into RAW_SQL_ALLOWLIST ---
        "tools/targets/cmds.rs",
        ...
```

**步骤 2.2：** 把「`paste into ALLOWLIST`」那一段的全部 `(...)` 行，原样粘进脚本里 `ALLOWLIST = frozenset({ ... })` 的大括号内（替换掉 `# filled in Task 2` 注释）。**本任务只填 `ALLOWLIST`，不填 `RAW_SQL_ALLOWLIST`**（留给 Task 4）。

**验证：** 重跑（此时 raw-sql 仍 RED、ownership 应为 0）：

```bash
python3 scripts/check_repo_ownership.py 2>&1 | grep -c "owned by"; echo "ownership_violations_above_should_be_0"
```

预期：`grep -c "owned by"` 输出 `0`（所有跨服务读已登记）。

**提交：**

```bash
git add scripts/check_repo_ownership.py
git commit -m "chore(arch): freeze cross-service repo coupling baseline (S1-1 ratchet)"
```

---

## Task 3 — 证明守卫能拦「新增」跨服务耦合（RED→GREEN）

**文件：** 临时改 `backend/crates/golish/src/tools/vault.rs`（platform 域），制造一个新的跨服务读，确认守卫报错，然后撤销。

**步骤 3.1：** 在 `tools/vault.rs` 顶部 `use` 区**临时**加一行（`findings` 属 pentest，vault 属 platform → 必触发）：

```rust
#[allow(unused_imports)]
use golish_db::repo::findings as _probe_findings_should_be_blocked;
```

**步骤 3.2：** 跑守卫，预期 RED：

```bash
python3 scripts/check_repo_ownership.py; echo "exit=$?"
```

预期含：

```
  - tools/vault.rs: platform -> repo::findings (owned by pentest)
exit=1
```

**步骤 3.3：** 撤销临时改动（确认 GREEN）：

```bash
git checkout -- backend/crates/golish/src/tools/vault.rs
python3 scripts/check_repo_ownership.py 2>&1 | grep -c "owned by"
```

预期：输出 `0`。

**提交：** 本任务无代码留存（仅验证守卫行为），不产生 commit。

---

## Task 4 — 命令层裸 SQL 次级基线（GREEN）

**文件：** 修改 `scripts/check_repo_ownership.py`（仅填 `RAW_SQL_ALLOWLIST`）

**步骤 4.1：** 复跑生成器，取第二段：

```bash
python3 scripts/check_repo_ownership.py --emit-allowlist
```

**步骤 4.2：** 把「`paste into RAW_SQL_ALLOWLIST`」那一段的全部 `"..."` 行，粘进脚本里 `RAW_SQL_ALLOWLIST = frozenset({ ... })` 的大括号内（替换 `# filled in Task 4`）。

**步骤 4.3：** 在该 `frozenset` 上方加一行注释，标明与 P0-3 的关系：

```python
# These files still hold raw sqlx (P0-3 作用域 SQL 下沉 will drain them). The
# ratchet blocks NEW raw-sql files; existing ones are tracked here, not fixed
# by S1-1. See docs/design/2026-05-29-architecture-optimization.md P0-3.
RAW_SQL_ALLOWLIST: frozenset[str] = frozenset(
    {
        # paste here
    }
)
```

**验证：** 守卫整体 GREEN：

```bash
python3 scripts/check_repo_ownership.py; echo "exit=$?"
```

预期：

```
[repo-ownership] OK clean
exit=0
```

**提交：**

```bash
git add scripts/check_repo_ownership.py
git commit -m "chore(arch): freeze command-layer raw-sql baseline (S1-1)"
```

---

## Task 5 — 接入 CI 与本地 recipe

**文件：** 修改 `.github/workflows/arch-check.yml` 与 `justfile`

**步骤 5.1：** 在 `.github/workflows/arch-check.yml` 的 `jobs:` 下、`dag:` job 之后新增一个 job（缩进与现有 `dag:` 对齐）：

```yaml
  repo-ownership:
    name: Repo data-ownership boundary
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup Python
        uses: actions/setup-python@v5
        with:
          python-version: "3.11"

      - name: Run repo-ownership guard
        run: python3 scripts/check_repo_ownership.py
```

**步骤 5.2：** 在 `justfile` 末尾新增一个本地汇总 recipe（一次跑两个架构守卫）：

```makefile
# Run architecture guards locally (DAG + repo data-ownership). CI runs these
# in .github/workflows/arch-check.yml; this is the local mirror.
arch:
    @python3 scripts/check_dag.py
    @python3 scripts/check_repo_ownership.py
```

**验证：**

```bash
just arch; echo "exit=$?"
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/arch-check.yml')); print('yaml ok')"
```

预期：`just arch` 两个守卫都 OK、`exit=0`；YAML 解析 `yaml ok`。

**提交：**

```bash
git add .github/workflows/arch-check.yml justfile
git commit -m "ci(arch): enforce repo data-ownership guard + add `just arch`"
```

---

## Task 6 — 文档与收尾

**文件：** 修改 `docs/architecture.md`、`agent-progress.md`、`feature_list.json`

**步骤 6.1：** 在 `docs/architecture.md` 的「## Evolution principles」列表末尾追加一条：

```markdown
5. **Data ownership** — every `golish-db` repo belongs to exactly one
   service (recon / vuln / pentest / agent / platform). Command-layer
   modules may only touch repos their own service owns; cross-service
   reads must go through a port (S1-2) and are frozen as a ratchet in
   `scripts/check_repo_ownership.py`. This prepares DB-per-service.
   See `docs/design/2026-05-30-servitization-readiness.md`.
```

**步骤 6.2：** 在 `docs/architecture.md` 的「### Crate catalog」之后新增一节「### Repo data ownership (servitization S1-1)」，放服务→repo 映射表（内容取自脚本 `REPO_OWNER`，5 行一服务）：

```markdown
### Repo data ownership (servitization S1-1)

每个 `golish-db` repo 归属唯一 service；守卫 `scripts/check_repo_ownership.py` 强制。

| Service | Repos |
|---|---|
| recon | targets, target_assets, organizations, api_endpoints, sitemap_store, directory_entries, fingerprints, js_analysis, passive_scans, sensitive_scan, screenshots, custom_rules, endpoint_tests |
| vuln | vuln_intel, vuln_scan, scan_queue, wiki_kb, kb_research |
| pentest | findings, methodology, pipelines, stage_runs, execution_plans, evidence_classifications, operation_state, subtasks, tasks, sprint_contracts |
| agent | sessions, conversation_store, message_chains, agent_logs, tool_calls, sub_agent_dispatches, memories, msg_logs, prompt_templates, vector_store_logs, search_logs |
| platform | vault, notes, terminal_logs |
| shared | audit, scoped |
```

**步骤 6.3：** 在 `feature_list.json` 的 `features` 数组追加一条（把当前 `in_progress` 的其它条目先确认无冲突；本条 `status` 初始 `in_progress`）：

```json
{
  "id": "arch-s1-1-repo-ownership-guard",
  "priority": 1,
  "area": "scripts + .github/workflows + justfile + docs/architecture.md",
  "title": "S1-1 repo data-ownership boundary guard (servitization groundwork)",
  "user_visible_behavior": "无用户可见行为变化。新增 CI 守卫，命令层不得新增跨服务直接读 golish-db repo / 不得新增裸 sqlx；现有耦合冻结为 ratchet 基线。为未来 DB-per-service 抽服务打地基。",
  "status": "in_progress",
  "verification": [
    "python3 scripts/check_repo_ownership.py → exit 0 / 'OK clean'",
    "just arch → exit 0 (check_dag + check_repo_ownership 均绿)",
    "临时加一处跨服务 repo 调用 → 守卫 exit 1 并报该行（Task 3 已演示）"
  ],
  "evidence": {},
  "notes": "依据 docs/design/2026-05-30-servitization-readiness.md §6 S1-1。物理重排 repo 子模块(S1-1b)与 *Port 抽取(S1-2)为后续。"
}
```

**步骤 6.4：** 在 `agent-progress.md` 顶部新增本轮会话记录（目标 / 已完成 / 验证证据 / 下一步），把 Task 1-5 实际跑过的命令与退出码贴进「已记录证据」。完成后把 `feature_list.json` 本条 `status` 改 `passing` 并回填 `evidence`。

**验证（完成定义，AGENTS.md §3）：**

```bash
python3 scripts/check_repo_ownership.py; echo "guard_exit=$?"
just arch; echo "arch_exit=$?"
python3 -c "import json; json.load(open('feature_list.json')); print('feature_list json ok')"
```

预期：两个 exit 均 0；`feature_list json ok`。

> 说明：本计划**零 Rust diff**，无需 `cargo` 编译验证；但合并前仍按 AGENTS.md §2.6 跑一次 `just precommit` 确认未误伤（应与基线一致，因为没动 Rust/TS）。

**提交：**

```bash
git add docs/architecture.md agent-progress.md feature_list.json
git commit -m "docs(arch): record repo data-ownership boundary (S1-1) + feature_list"
```

---

## 自检（writing-plans 收尾）

**1. 规格覆盖度（对照设计文档 §6 S1-1 三项）：**
- 「`golish-db` repo 按域归组」→ 本计划用**逻辑所有权表 + 守卫**实现（Task 1 `REPO_OWNER` + Task 6.2 文档表）；**物理归组显式延后**（范围外，标注为 S1-1b）。覆盖（以更低风险的方式）。
- 「域 repo 隔离 CI 守卫」→ Task 1/2/3/5 完整覆盖。
- 「命令层禁跨域裸 SQL」→ Task 4（次级规则 + 基线）覆盖，并标注与 P0-3 的边界。

**2. 占位符扫描：** 无「TODO / 待定 / 后续实现」。`ALLOWLIST` / `RAW_SQL_ALLOWLIST` 的具体条目由 `--emit-allowlist` 在执行时生成（机制全量给出，数据来自真实扫描而非猜测——这是正确做法，非占位符）。

**3. 类型/标识一致性：** 脚本内 `REPO_OWNER` / `SHARED_REPOS` / `DOMAIN_RULES` / `ALLOWLIST` / `RAW_SQL_ALLOWLIST` / `domain_of` / `scan` / `declared_repos` 命名在 Task 1 定义后，Task 2/4 仅向同名容器粘贴条目，Task 5 命令与函数名一致；`just arch`、`check_repo_ownership.py` 路径全程一致。

**4. 风险与回滚：**
- 全部为**新增文件 + 新增 CI job + 文档**，零 Rust/TS 改动；回滚 = revert 对应 commit，主链路不受影响。
- 守卫**起点即绿**（基线 allowlist 冻结现状），不会阻断现有 PR；只拦**新增**耦合。
- 若 `REPO_OWNER` 初始归属有争议（如 `scan_queue`/`execution_plans`/`operation_state`），不阻塞：要么调整该 repo 的 owner，要么留在 allowlist；二者皆为单行数据改动。

**验证命令汇总：**
```bash
python3 scripts/check_repo_ownership.py            # 守卫主入口
python3 scripts/check_repo_ownership.py --emit-allowlist  # 生成基线
just arch                                          # 本地两守卫
just precommit                                     # 合并前总门禁（应与基线一致）
```
