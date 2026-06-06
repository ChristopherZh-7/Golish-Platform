# golish-pty / shell

> **一句话职责**：多 shell 检测与配置——从路径/设置/环境探测 shell 类型（zsh/bash/fish），并自动注入 shell 集成脚本（发 OSC 133）而无需用户改 rc 文件。

- **类型**：目录模块（属于 crate [`golish-pty`](../golish-pty.md)）
- **路径**：`backend/crates/golish-pty/src/shell/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 shell 探测优先级（settings override → `$SHELL` → 回退）时
- 改 OSC 133 shell 集成自动注入（嵌入脚本、ZDOTDIR wrapper、env/args 计算）时

## 职责

`detect_shell` 按「`settings.terminal.shell` 用户覆盖 → `$SHELL`（Unix）→ `/bin/sh` 或 `powershell.exe` 回退」探测；`ShellIntegration` 把嵌入的 zsh/bash 集成脚本装到磁盘并算出注入所需 env/args（发 OSC 133，无需用户编辑 rc）。底层探测类型复用 `golish_platform::shell`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `detect_shell(settings, shell_env) -> ShellInfo` | shell 探测（带优先级） |
| `ShellInfo` / `ShellType`（re-export 自 `golish_platform::shell`） | shell 信息/类型 |
| `ShellIntegration` | 集成脚本安装 + env/shell args 计算 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `detect_shell` + 类型 re-export |
| `integration.rs` | `ShellIntegration`（脚本落盘 + 注入参数） |
| `scripts.rs` | 嵌入的 zsh/bash 集成脚本 + ZDOTDIR wrapper `.zshrc` |

## 依赖

- `golish_platform::shell`（跨平台探测 + 类型）、`golish_settings::schema::TerminalSettings`

## 注意事项 / 坑

- 跨平台 shell 探测/类型在 `golish-platform`（不变量：唯一可写 `cfg(target_os)` 处）；本模块只做编排，别在此写平台分支。
- OSC 133 注入是"无侵入"的（不改用户 rc）；`parser/` 负责解析回来的 OSC 133，两者配对。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-pty shell
```
