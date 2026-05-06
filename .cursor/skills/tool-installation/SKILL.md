---
name: tool-installation
description: "Use when installing, configuring, or validating penetration testing tools. Handles package managers, Python environments, compiled tools, and PATH configuration."
---

# Tool Installation & Environment Configuration

> **CRITICAL**: All AI agents MUST install tools through the Golish ToolManager pipeline (`pentest_install_tool` / ToolManager UI) — never invoke `brew install`, `pip install`, `gem install`, `go install`, `cargo install`, or `git clone` of a tool repo directly via `run_pty_cmd`. Bypassing the pipeline corrupts the registry, skips runtime checks, and leaves the tool invisible to ToolManager.
>
> **关键约束**：所有 AI agent 必须通过 Golish ToolManager 管线安装工具（`pentest_install_tool` / ToolManager UI），**禁止**用 `run_pty_cmd` 直接跑 `brew install` / `pip install` / `gem install` / `go install` / `cargo install` 或 `git clone` 工具仓库。绕过管线会污染注册表、跳过运行时检查、让工具对 ToolManager 不可见。

---

## English

Install, configure, and validate security tools following the project's ToolManager conventions.

### Pre-flight Checks

Before installing any tool:

1. **Check existing installation**: query ToolManager (`pentest_list_tools`) — if `installed === true`, skip.
2. **Verify config exists**: a tool must have a JSON config under `<tools_dir>/<tool>/<id>.json` declaring `install.method` + `install.source`. If absent, the user must register the tool via ToolManager UI (Add Tool / Import from GitHub).
3. **Verify OS compatibility**: determine the best installation method for the current platform (macOS / Linux).

### Installation Methods (by priority)

| Priority | Method | When to use |
|----------|--------|-------------|
| 1 | ToolManager API | **Always preferred** — `pentest_install_tool(toolId)` runs the platform's homebrew / git / gem / pip pipeline with proxy, retries, and progress tracking |
| 2 | ToolManager UI | When the tool's config does not yet exist — register it first, then install |
| 3 | Manual `run_pty_cmd` | **Forbidden for tool installation.** Reserved for ad-hoc shell tasks that have nothing to do with installing a managed tool |

### Workflow

```
1. pentest_list_tools  →  if installed, skip
2. If config missing  →  ask user to register via ToolManager UI; do not invent configs
3. pentest_install_tool(toolId)  →  pipeline handles homebrew / github / gem / pip
4. Re-list to confirm installed: true and executable resolves
5. Run --help or --version through pentest_run to validate
6. Report version + install path
```

### Constraints

- **Never install as root** unless absolutely necessary (e.g., nmap raw socket mode)
- **Always use virtual environments** for Python tools — Golish manages `python<ver>_env` per project; never pollute global site-packages
- **Handle dependency conflicts** gracefully — report clearly, don't force-overwrite
- **Respect ToolManager state** — after install, re-check via `pentest_list_tools`; do not edit JSON configs by hand
- **Clear error messages** on failure — include the exact command that failed and its stderr

### Validation Checklist

After installation, verify:

- [ ] `pentest_list_tools` shows `installed: true`
- [ ] `pentest_run <tool> --version` (or `--help`) succeeds
- [ ] Required runtime dependencies are present (Python venv, Java major, Go modules)
- [ ] Executable path is valid and points inside `<tools_dir>`

---

## 中文

按照项目 ToolManager 规范安装、配置和验证安全工具。

### 安装前检查

安装任何工具之前：

1. **检查现有安装**：调 `pentest_list_tools`，若 `installed === true` 则跳过
2. **确认配置存在**：每个工具必须有 `<tools_dir>/<tool>/<id>.json` 配置，声明 `install.method` + `install.source`。若不存在，必须让用户通过 ToolManager UI（Add Tool / Import from GitHub）先注册
3. **验证系统兼容性**：确定当前平台（macOS / Linux）的最佳安装方式

### 安装方式（按优先级）

| 优先级 | 方式 | 使用场景 |
|--------|------|---------|
| 1 | ToolManager API | **始终优先**——`pentest_install_tool(toolId)` 走平台的 homebrew / git / gem / pip 管线，自带代理、重试、进度跟踪 |
| 2 | ToolManager UI | 工具配置尚未存在时——先注册再安装 |
| 3 | 手动 `run_pty_cmd` | **禁止用于工具安装**。仅保留给跟「装受管工具」无关的临时 shell 任务 |

### 工作流

```
1. pentest_list_tools  →  已装则跳过
2. 配置缺失  →  让用户通过 ToolManager UI 注册；禁止凭空伪造配置
3. pentest_install_tool(toolId)  →  管线处理 homebrew / github / gem / pip
4. 复查 installed: true 且 executable 可定位
5. 通过 pentest_run 运行 --help 或 --version 验证
6. 报告版本号与安装路径
```

### 约束

- **禁止以 root 安装**，除非绝对必要（如 nmap 原始套接字模式）
- **Python 工具必须用虚拟环境**——Golish 按项目维护 `python<版本>_env`，禁止污染全局 site-packages
- **妥善处理依赖冲突**——清晰报告，不强制覆盖
- **尊重 ToolManager 状态**——安装后通过 `pentest_list_tools` 复查，禁止手编 JSON 配置
- **失败时给出清晰错误信息**——包含失败的具体命令及其 stderr

### 验证清单

安装完成后确认：

- [ ] `pentest_list_tools` 显示 `installed: true`
- [ ] `pentest_run <tool> --version` 或 `--help` 运行成功
- [ ] 必需的运行时依赖已就位（Python venv / Java 主版本 / Go modules）
- [ ] 可执行文件路径有效且位于 `<tools_dir>` 之内

---

## Package Manager Expertise / 包管理器能力

- **System / 系统级**: brew, apt, dnf, pacman
- **Language / 语言级**: pip, gem, go install, cargo, npm
- **Python envs / Python 环境**: venv, pyenv, pip dependency resolution
- **Compiled / 编译类**: Go builds, Rust compilation, C/C++ make
- **Container / 容器类**: Docker image pull and management

> Even when invoking these package managers, do so **through ToolManager's `install.method`** rather than `run_pty_cmd` shell calls. The pipeline already abstracts homebrew / pip / gem / cargo correctly.
>
> 即使要使用这些包管理器，也**必须**走 ToolManager `install.method` 而非 `run_pty_cmd` shell 调用。管线已经正确抽象了 homebrew / pip / gem / cargo 等方式。
