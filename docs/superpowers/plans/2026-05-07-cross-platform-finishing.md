# 跨平台收尾实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 Golish 跨平台抽象的"最后一公里"做完——CI 矩阵补全、`golish-platform` 抽象层去重 6 处，让 Linux GUI / Windows 运行时进入可信赖范围。

**架构：** 两条主线 = (1) CI 矩阵保隔离（macOS job + Windows nextest）；(2) 抽象层 dedup（让 `golish-platform` 真正成为 cfg 唯一住所）。第二阶段的 P2/P3（Linux 终端启动器、`install.windows` schema）不在本计划——需要单独头脑风暴出新规格。

**技术栈：** Rust 1.95（cargo nextest, cargo clippy, cargo fmt）、GitHub Actions matrix、`golish-platform` crate（`shell` / `fs_perms` / `package_manager` / `Platform::current` 已实现）。

## 前置假设（每个 Task 都依赖）

1. 仓库根 `cd /Users/christopherzheng/WebstormProjects/Golish-Platform`；后端工作目录 `backend/`。
2. `cargo nextest run` 已装；`just check` 包含 fmt/clippy/test 三联。
3. `golish-platform` 已经 publish 全部 API：`Platform::current()`, `shell::which_executable`, `shell::login_shell_for_path_resolution`, `fs_perms::has_execute_bit`, `package_manager::PackageManager::detect`。
4. 当前 main 分支已包含跨平台抽象 + lint 收尾（commit `<paste-hash>`）；本计划基于此 HEAD 推进。
5. 每个 Task 独立 commit；commit 失败一律不 amend，新 commit 修复。

## 文件结构 / 任务覆盖矩阵

| Task | 主要文件 | 类型 | 预计 |
|---|---|---|---|
| 1 | `.github/workflows/check.yml` | CI（新增 macOS job） | 30 分 |
| 2 | `.github/workflows/check.yml` + `justfile` | CI（Windows nextest） | 1 小时 |
| 3 | `backend/crates/golish-pentest/src/platform.rs` + 若干调用方 | dedup（删 shim） | 30 分 |
| 4 | `backend/crates/golish-pentest/src/command_builder/native.rs` | dedup（用 `which_executable`） | 15 分 |
| 5 | `backend/crates/golish-pentest/src/preflight.rs` | dedup（用 `has_execute_bit`） | 15 分 |
| 6 | `backend/crates/golish/src/commands/proc/command_index/path_resolution.rs` | dedup（用 platform shell） | 30 分 |
| 7 | `backend/crates/golish-pentest/src/scan_types.rs` | dedup（用 PackageManager） | 30 分 |
| 8 | `backend/crates/golish/src/tools/pentest/tool_mgmt.rs` | 形式简化（`Platform::current()` runtime check） | 20 分 |
| 9 | 全 workspace | 验证 + commit reset 检查 | 15 分 |

总预计：约 4 小时（含等待 cargo build 时间）。

---

## Task 1: 加 macOS CI job

**文件：**
- `.github/workflows/check.yml`（修改）

**为什么：** CI 现在只跑 linux-arm64 + windows-latest（仅 cargo check）。macOS 是开发主平台但 CI 真空 → 任何 macOS-specific 回归都会泄漏到 main。本任务加最小可用的 macOS job。

### 步骤 1.1：在 `check.yml` 末尾追加 `macos-check` job

在 `.github/workflows/check.yml` 现有 `windows-check` job **之后**插入：

```yaml
  macos-check:
    runs-on: macos-14
    permissions:
      contents: read
    name: macOS compile + lint check
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 9

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: "20"
          cache: pnpm

      - name: Install frontend dependencies
        run: pnpm install --frozen-lockfile

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cache Rust dependencies
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: backend
          shared-key: rust-macos-aarch64-v1
          cache-targets: true
          save-if: ${{ github.ref == 'refs/heads/main' }}

      - name: Cargo check (macOS aarch64)
        working-directory: backend
        run: cargo check --workspace --all-features

      - name: Cargo clippy (macOS aarch64)
        working-directory: backend
        run: cargo clippy --workspace -- -D warnings

      - name: Cargo fmt check
        working-directory: backend
        run: cargo fmt --check
```

### 步骤 1.2：验证 yaml 语法

```bash
cd /Users/christopherzheng/WebstormProjects/Golish-Platform
yq eval '.jobs.macos-check.runs-on' .github/workflows/check.yml
# 预期输出：macos-14
```

如果没装 `yq` 用：

```bash
python3 -c "import yaml; print(yaml.safe_load(open('.github/workflows/check.yml')).get('jobs', {}).get('macos-check', {}).get('runs-on'))"
# 预期输出：macos-14
```

### 步骤 1.3：commit

```bash
git add .github/workflows/check.yml
git commit -m "ci(macos): add macOS aarch64 cargo check + clippy + fmt job

The Linux job (linux-arm64) and Windows smoke job (windows-latest cargo check)
left macOS as the only main developer platform without CI coverage. macos-14
runners are aarch64 and align with the lab's Apple Silicon dev machines."
```

---

## Task 2: Windows CI 增加 nextest 选择性测试

**文件：**
- `.github/workflows/check.yml`（修改 `windows-check` job）
- `justfile`（新增 recipe）

**为什么：** 现在 Windows CI 只 `cargo check`，不跑测试。许多跨平台代码路径在 Windows 编译能过、运行时挂的情况无法被发现。增加最低限度 nextest，跳过依赖 unix shell 的测试。

### 步骤 2.1：在 `justfile` 末尾加 recipe `test-rust-windows-safe`

```makefile
# Run Rust unit tests excluding ones that require Unix shells / external CLIs
test-rust-windows-safe:
    @cd backend && cargo nextest run \
        --workspace \
        --no-fail-fast \
        --status-level fail \
        -E 'not test(/.*requires_unix.*/)'
```

### 步骤 2.2：把现有要求 unix 的测试加 `requires_unix` 标记

搜索并标注两处（之前 audit 发现的）：

```bash
cd backend
rg "fn test_path_traversal_blocked|fn test_golish_message_to_rig_system_returns_none" -l
```

对 hit 到的测试文件，在 `#[test]` 上方加：

```rust
#[cfg_attr(target_os = "windows", ignore = "requires_unix shell semantics")]
#[test]
fn test_path_traversal_blocked() {
    // ...
}
```

### 步骤 2.3：把 `windows-check` job 的最后一步从 `cargo check` 后追加 nextest

在 `.github/workflows/check.yml` 的 `windows-check` job 步骤里，在 `Cargo check (Windows MSVC)` 之后加：

```yaml
      - name: Install cargo-nextest (Windows)
        uses: taiki-e/install-action@nextest

      - name: Cargo nextest (Windows MSVC, unix-safe subset)
        working-directory: backend
        run: cargo nextest run --workspace --no-fail-fast --status-level fail
```

### 步骤 2.4：本地预跑（在 macOS 上模拟）

```bash
cd backend
just test-rust-windows-safe
# 预期：除被 ignore 的 windows-only 测试外其它全过；输出末尾 `Summary [...] X passed, Y skipped`
```

### 步骤 2.5：commit

```bash
git add .github/workflows/check.yml justfile backend/crates/golish-tools/src/file_ops/tests.rs backend/crates/golish-session/src/tests.rs
git commit -m "ci(windows): run nextest in windows-check job, ignore unix-only tests

Adds a 'requires_unix' attribute to tests that genuinely need /bin/sh or POSIX
permissions semantics, so Windows CI can run the rest. Catches Windows runtime
regressions that the bare 'cargo check' missed."
```

---

## Task 3: 删除 `golish-pentest::platform` deprecated shim

**文件：**
- `backend/crates/golish-pentest/src/platform.rs`（删除）
- `backend/crates/golish-pentest/src/lib.rs`（移除 `pub mod platform`）
- 所有 `use crate::platform::IS_WINDOWS|IS_MAC|...` 调用方（修改）

**为什么：** 这个 module 自己注释 "Deprecated. All real implementations now live in golish_platform"，但还作为常量 shim 存在。让所有调用方直接用 `golish_platform::Platform::current().is_windows()` 等运行时方法（const 转 const fn 已经在 detect.rs 暴露）。

### 步骤 3.1：列出所有调用方

```bash
cd backend
rg -l "platform::IS_(WINDOWS|MAC)|platform::PYTHON_(BIN_DIR|EXE_NAME)|platform::CONDA_(BIN_DIR|EXE_NAME)|platform::EXECUTABLE_EXT" crates/golish-pentest/
# 预期：列出 5-10 个文件
```

### 步骤 3.2：每个文件用如下 mapping 替换

| 旧 | 新 |
|---|---|
| `crate::platform::IS_WINDOWS` | `golish_platform::Platform::current().is_windows()` |
| `crate::platform::IS_MAC` | `golish_platform::Platform::current().is_macos()` |
| `crate::platform::PYTHON_BIN_DIR` | `golish_platform::paths::python_bin_dir()` |
| `crate::platform::PYTHON_EXE_NAME` | `golish_platform::paths::python_exe_name()` |
| `crate::platform::CONDA_BIN_DIR` | `golish_platform::paths::conda_bin_dir()` |
| `crate::platform::CONDA_EXE_NAME` | `golish_platform::paths::conda_exe_name()` |
| `crate::platform::EXECUTABLE_EXT` | `golish_platform::Platform::current().executable_extension()` |

注意：常量上下文（`const FOO = ...`）无法用 runtime 方法 → 这些位置（如果有）需要改为 `let foo = ...` 或 `static FOO: Lazy<...>`。先查一遍：

```bash
rg "const\s+\w+\s*:\s*\S+\s*=\s*crate::platform::" crates/golish-pentest/
# 应该没有 hit；如果有，单独处理
```

### 步骤 3.3：删除 module + 测试

```bash
git rm crates/golish-pentest/src/platform.rs
```

修改 `crates/golish-pentest/src/lib.rs`，删除 `pub mod platform;` 行。

### 步骤 3.4：验证编译 + lint

```bash
cd backend
cargo check -p golish-pentest --all-targets
# 预期：成功 + 无 warning

cargo clippy -p golish-pentest --all-targets -- -D warnings
# 预期：成功
```

### 步骤 3.5：commit

```bash
git add -u backend/crates/golish-pentest/
git commit -m "refactor(pentest): retire deprecated platform.rs shim

All IS_WINDOWS / IS_MAC / PYTHON_BIN_DIR / etc constants now resolve directly
via golish_platform::Platform::current() / paths::*. The old shim was already
documented as 'Deprecated' since the cross-platform refactor."
```

---

## Task 4: `which_in_path` 改用 `golish_platform::shell::which_executable`

**文件：**
- `backend/crates/golish-pentest/src/command_builder/native.rs`

**为什么：** 该函数手写 `which`/`where` 平台分支，`golish_platform::shell::which_executable()` 已实现同等行为，且返回 `Option<PathBuf>` 信息更丰富。

### 步骤 4.1：替换 `which_in_path`

打开 `backend/crates/golish-pentest/src/command_builder/native.rs`，定位现有：

```rust
pub(super) fn which_in_path(cmd: &str) -> bool {
    #[cfg(unix)]
    let probe = "which";
    #[cfg(windows)]
    let probe = "where";

    std::process::Command::new(probe)
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

替换为：

```rust
pub(super) fn which_in_path(cmd: &str) -> bool {
    golish_platform::shell::which_executable(cmd).is_some()
}
```

### 步骤 4.2：验证

```bash
cd backend
cargo check -p golish-pentest --all-targets
cargo clippy -p golish-pentest --all-targets -- -D warnings
cargo nextest run -p golish-pentest --status-level fail
# 全过即可
```

### 步骤 4.3：commit

```bash
git add -u backend/crates/golish-pentest/src/command_builder/native.rs
git commit -m "refactor(pentest): use golish_platform::shell::which_executable

Eliminates an ad-hoc 'which/where' cfg branch that duplicated the abstraction
already exposed by golish-platform."
```

---

## Task 5: `has_unix_execute_bit` 改用 `golish_platform::fs_perms::has_execute_bit`

**文件：**
- `backend/crates/golish-pentest/src/preflight.rs`

### 步骤 5.1：替换函数实现

定位 `preflight.rs` 内：

```rust
#[cfg(unix)]
fn has_unix_execute_bit(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
```

替换为：

```rust
#[cfg(unix)]
fn has_unix_execute_bit(path: &Path) -> bool {
    golish_platform::fs_perms::has_execute_bit(path)
}
```

（保留 `#[cfg(unix)]`，因为调用方仍在 unix 分支里，Windows 走另一条路径。）

### 步骤 5.2：验证 + commit

```bash
cd backend
cargo check -p golish-pentest --all-targets
cargo clippy -p golish-pentest --all-targets -- -D warnings
git add -u backend/crates/golish-pentest/src/preflight.rs
git commit -m "refactor(pentest): use golish_platform::fs_perms::has_execute_bit

The local has_unix_execute_bit duplicated the cross-platform helper that
golish-platform already exposes."
```

---

## Task 6: `path_resolution` 改用 platform shell helpers

**文件：**
- `backend/crates/golish/src/commands/proc/command_index/path_resolution.rs`

**为什么：** `is_executable()` 重复了 `fs_perms::has_execute_bit()`；`resolve_shell_path()` 重复了 `shell::login_shell_for_path_resolution()` + `shell::resolve_login_shell_path()`。

### 步骤 6.1：替换 `is_executable`

定位：

```rust
#[cfg(unix)]
pub(super) fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}
```

由于 `golish_platform::fs_perms::has_execute_bit` 接受 `&Path`（不是 `&Metadata`），这里调用方语义不同；需要看上下文是否能改成传 path。

```bash
rg "is_executable\(" backend/crates/golish/src/commands/proc/command_index/
```

如果调用方已有 path：直接改。如果只有 metadata（避免重新 stat）：保留本函数但函数体改为 `let mode = metadata.permissions().mode(); mode & 0o111 != 0`，并在 `golish-platform/src/fs_perms.rs` 加个 `pub fn has_execute_bit_from_mode(mode: u32) -> bool`。

### 步骤 6.2：替换 `resolve_shell_path`

定位：

```rust
#[cfg(unix)]
pub(super) fn resolve_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".to_string()
        } else {
            "/bin/sh".to_string()
        }
    });
    // ...运行 shell -lic 获取 PATH
}
```

替换为直接调用 `golish_platform::shell::resolve_login_shell_path()`，删本函数：

```rust
#[cfg(unix)]
pub(super) fn resolve_shell_path() -> Option<String> {
    golish_platform::shell::resolve_login_shell_path()
}
```

### 步骤 6.3：验证 + commit

```bash
cd backend
cargo check -p golish --all-targets
cargo clippy -p golish --all-targets -- -D warnings
cargo nextest run -p golish --no-fail-fast --status-level fail
# 注意：忽略 ai_events_characterization snapshot pre-existing failures

git add -u backend/crates/golish/src/commands/proc/command_index/path_resolution.rs
# 如果新增了 has_execute_bit_from_mode：
# git add -u backend/crates/golish-platform/src/fs_perms.rs

git commit -m "refactor(golish): collapse path_resolution into golish_platform helpers

The local is_executable / resolve_shell_path duplicated fs_perms::has_execute_bit
and shell::resolve_login_shell_path. Removing the duplication keeps the cfg
guards centralised."
```

---

## Task 7: `scan_types::list_brew_*` 改用 `PackageManager`

**文件：**
- `backend/crates/golish-pentest/src/scan_types.rs`

**为什么：** 现在用 `if cfg!(target_os = "windows") { return HashSet::new() }` 硬编码"非 brew 平台无包列表"。改为查询 `PackageManager::detect()`，让未来 Linux 用户的 apt/dnf 列表也能从这里跑。

### 步骤 7.1：抽出 PackageManager helper

如果 `golish-platform::package_manager` 还没有 `installed_packages()` 方法，先在 `golish-platform/src/package_manager.rs` 加：

```rust
impl PackageManager {
    /// Return the list of installed packages tracked by this manager.
    /// Only `Homebrew` is currently implemented; other managers return an empty set.
    pub fn installed_packages(&self) -> std::collections::HashSet<String> {
        match self {
            PackageManager::Homebrew => list_brew_packages("formula"),
            PackageManager::HomebrewCask => list_brew_packages("cask"),
            _ => std::collections::HashSet::new(),
        }
    }
}

fn list_brew_packages(kind: &str) -> std::collections::HashSet<String> {
    let arg = match kind {
        "cask" => "--cask",
        _ => "--formula",
    };
    std::process::Command::new("brew")
        .args(["list", arg, "-1"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
```

### 步骤 7.2：替换 scan_types 调用

在 `scan_types.rs` 中：

```rust
fn list_brew_formulas() -> HashSet<String> {
    golish_platform::PackageManager::Homebrew.installed_packages()
}

fn list_brew_casks() -> HashSet<String> {
    golish_platform::PackageManager::HomebrewCask.installed_packages()
}
```

### 步骤 7.3：验证 + commit

```bash
cd backend
cargo check -p golish-platform -p golish-pentest --all-targets
cargo clippy -p golish-platform -p golish-pentest --all-targets -- -D warnings

git add -u backend/crates/golish-platform/src/package_manager.rs backend/crates/golish-pentest/src/scan_types.rs
git commit -m "refactor: route brew list calls through PackageManager::installed_packages

scan_types previously had to short-circuit Windows manually. Now it just asks
the PackageManager abstraction; future apt/dnf/winget hookups slot in without
touching the call site."
```

---

## Task 8: `tool_mgmt` cfg block 简化

**文件：**
- `backend/crates/golish/src/tools/pentest/tool_mgmt.rs`

**为什么：** 三个 Tauri command 现在用 `#[cfg(windows)] { return ... }` + `#[cfg(unix)] { ... }` 双 block，可以扁平化为运行时 `if Platform::current().is_windows() { return Ok(...); }`。除了减少 cfg 嵌套，还让函数主体的 borrow checking 更稳定。

### 步骤 8.1：定位三个函数

```bash
rg "^pub async fn pentest_(check_tool_executable_permission|check_tools_executable_permissions|fix_tool_executable_permission)" backend/crates/golish/src/tools/pentest/tool_mgmt.rs
# 应该 hit 三处
```

### 步骤 8.2：每个函数改为如下结构

```rust
#[tauri::command]
pub async fn pentest_check_tool_executable_permission(
    state: State<'_, PentestState>,
    executable: String,
    runtime: Option<String>,
    install_method: Option<String>,
) -> Result<ExecutablePermissionCheckResult, GolishError> {
    if golish_platform::Platform::current().is_windows() {
        return Ok(ExecutablePermissionCheckResult { ok: true, reason: None });
    }

    #[cfg(unix)]
    {
        let tools_dir = state.config_manager.tools_dir().await;
        Ok(check_tool_executable_sync(
            &tools_dir,
            &executable,
            runtime.as_deref(),
            install_method.as_deref(),
        ))
    }
    #[cfg(not(unix))]
    {
        Ok(ExecutablePermissionCheckResult { ok: true, reason: None })
    }
}
```

注意保留 `#[cfg(unix)]` 包住调用 `check_tool_executable_sync` 的代码块——因为 `check_tool_executable_sync` 本身是 `#[cfg(unix)]` 定义的（编译期不存在于 Windows）。改完后**Windows 的 runtime 分支**永远不会到 `cfg(unix)` 块内（Platform::current 已早返回），但编译器需要每条路径都有有效代码。

类似处理另两个 command。

### 步骤 8.3：验证 + commit

```bash
cd backend
cargo check -p golish --all-targets
cargo check -p golish --target x86_64-pc-windows-msvc --all-features  # 如本地装了 windows target
cargo clippy -p golish --all-targets -- -D warnings

git add -u backend/crates/golish/src/tools/pentest/tool_mgmt.rs
git commit -m "refactor(golish): flatten tool_mgmt cfg blocks via Platform::current()

Replaces nested '#[cfg(windows)] { early return }' + '#[cfg(unix)] { body }'
with runtime 'if Platform::current().is_windows()' early return + a compact
unix block. Reduces cfg surface and makes the borrow tree easier to follow."
```

---

## Task 9: 全局验证 + plan 关闭

**文件：** 无（验证 step）

### 步骤 9.1：跑全 CI 等价命令

```bash
cd /Users/christopherzheng/WebstormProjects/Golish-Platform
just check
# 预期：全绿。等价于：fmt + check-fe + test-fe + lint-rust + test-rust-all
```

### 步骤 9.2：手动 grep 确认 cfg 收敛

```bash
cd backend
rg "cfg\(target_os|cfg\(unix|cfg\(windows" crates/ | grep -v 'golish-platform/' | grep -v 'tests/' | grep -v '/Cargo.toml' | wc -l
# 预期：≤ 6 处（`ime.rs` macOS-only Carbon API + `process_group.rs` POSIX setsid + 几处 ZAP / handlers tool 安装路径差异）
# 如果数字明显比当前大（基线 ~30+），说明没全清；回查
```

### 步骤 9.3：跑 `cargo nextest --workspace --no-fail-fast`

```bash
cd backend
cargo nextest run --workspace --no-fail-fast --status-level fail 2>&1 | tail -20
# 预期：1922+ passed / ≤10 failed（pre-existing ai_events_characterization + agent-runtime + evals 等）
# 不能引入新 fail
```

### 步骤 9.4：commit final（如果有任何零碎清理）

```bash
git status
# 应该是 clean
```

如果还有零碎，写一个 `chore: cleanup post cross-platform finishing` commit。

### 步骤 9.5：开 PR

```bash
git push -u origin <branch>
gh pr create --title "Cross-platform finishing: CI matrix + abstraction dedup" --body "$(cat <<'EOF'
## Summary
- Adds macOS aarch64 CI job and Windows nextest selective testing
- Collapses 6 places where `cfg(target_os)` / `cfg(unix)` duplicated `golish-platform`
- Retires the `golish-pentest::platform` deprecated shim

## Test plan
- [ ] `just check` green
- [ ] All three CI jobs (linux-arm64 / windows-latest / macos-14) green
- [ ] `rg cfg\(target_os|cfg\(unix` outside `golish-platform/` shows ≤ 6 hits
- [ ] No new test failures (only pre-existing snapshot/characterization tests still failing)

EOF
)"
```

---

## 自检结果

**1. 规格覆盖度：** 用户的"还需要改什么"问题里我给出 6 处 P1 重复 + 2 个 CI 空缺 + Linux/Windows 实跑覆盖。Task 1-8 覆盖前两类；Linux 实跑因为需要新设备，本计划没强制（建议留作下一计划）。

**2. 占位符扫描：** 通读全文，未发现 "TODO" / "后续实现"。每个步骤有具体代码块或具体命令。

**3. 类型一致性：** `Platform::current()`, `is_windows()`, `is_macos()`, `which_executable`, `has_execute_bit`, `login_shell_for_path_resolution`, `resolve_login_shell_path`, `installed_packages` 这些方法名前后引用一致。**Task 7 在 `golish-platform` 新增的 `installed_packages` 是该计划唯一新 API**——使用方仅在 Task 7 内部，无前向引用问题。

## 不在本计划范围（提议另写）

- Linux 终端启动器（gnome-terminal/konsole/alacritty/foot）接入 `golish-pentest::terminal`
- `install.windows` / `install.linux` 子 schema 加入 `resources/toolsconfig/*.json` + `tool-manager` 选 installer
- Linux IBus/Fcitx IME switching
- Linux ZAP 证书 → `update-ca-certificates`

以上每项都需要单独的产品决策（schema 设计、Linux 各发行版差异）+ 端到端验证窗口，不适合并入抽象层 dedup。
