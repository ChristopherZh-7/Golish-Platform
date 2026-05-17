# Warp 风格交互改造（Phase A）

**作者**: fullstack-dev agent
**日期**: 2026-05-15
**状态**: Draft / 待实施

## 1. 背景与目标

### 1.1 痛点

当前实现里，命令运行中需要交互（如 `bash select`、`Install-Module` 问 Y/N、`npm init` 一问一答），用户体验割裂：

- **场景一**：命令运行 250 ms 后，timeline 末尾出现高度 384 px 的迷你 xterm 块（`LiveTerminalBlock`），用户需要点击它使之 focus 才能按 Y。
- **场景二**：某些命令命中 `BUILTIN_FULLTERM_COMMANDS` 白名单（claude / codex 等），或后端检测到 alt-screen，整个中间区域被全屏 xterm 接管，把已有 timeline 卡片完全盖住。
- **场景三**：xterm.js 在 Tauri WebView2（Windows）上有兼容性 bug，整个终端会黑屏。

用户对标 Warp：交互永远发生在最底部「输入命令」的位置，交互完成后内容上升变成历史卡片，下面恢复打新命令。

### 1.2 目标

| 场景 | 当前 | Phase A 之后 |
|---|---|---|
| Y/N 交互 | xterm 接管 | 底部 `UnifiedInput` 变身交互输入条，Enter 直接 `ptyWrite`（不创建新命令） |
| Install-Module 问 Y/N | xterm 全屏（误触发） | 同上 |
| npm init 多问 | xterm 全屏（误触发） | 同上，每问一次响应一次 |
| vim / htop | xterm 全屏 | **Phase A 暂保留全屏 xterm**（Phase B 用 GridTerminal 替代） |
| claude / codex 白名单 | 自动 fullterm | 取消自动 fullterm，走 Block UI |
| 命令普通输出 | timeline 卡片 | 不变 |

Phase A 不涉及 xterm.js 替换（那是 Phase B），只解决「交互场景被误升级到全屏」的体验问题。

## 2. 当前架构关键点

### 2.1 全屏 xterm 触发链路（要拆掉的）

`frontend/hooks/useTauriEvents.ts`：

- **L224 `fulltermCommands`**：命令开始时如果 process name 在白名单（claude/cc/codex/cdx/aider/cursor/gemini），立即切 fullterm。
- **L410 `alternate_screen`**：后端 PTY 解析器发出 OSC `?1049h` 时，自动切 fullterm。

### 2.2 迷你 xterm（LiveTerminalBlock）

`frontend/components/LiveTerminalBlock/LiveTerminalBlock.tsx` + `frontend/lib/terminal/LiveTerminalManager.ts`：

- 命令运行 250 ms 后挂在 `UnifiedTimeline` 末尾的 xterm，用于接收键盘输入并通过 `ptyWrite` 转发。
- 命令结束后被 serialize 成静态 ANSI 文本，再拆除。
- 约 1200 行代码（含测试）。

### 2.3 UnifiedInput 现状

`frontend/components/UnifiedInput/UnifiedInput.tsx`：用户敲命令 → Enter → 创建新 PTY 命令。
没有「跟运行中的命令对话」的概念。

## 3. 解决方案：UnifiedInput 双模式

### 3.1 行为表

| 状态 | 触发 | placeholder | 颜色提示 | Enter 行为 |
|---|---|---|---|---|
| **command 模式**（默认） | 默认 | `输入命令...` | 默认 | 创建新 PTY 命令 |
| **interactive 模式** | 后端发 `stdin_wait` 事件 | `回复 $cmd…` | 顶部加一条橙色 banner「正在向 $cmd 交互」 | `ptyWrite(input + '\n')`，不创建新命令 |

退出 interactive 模式的触发：
- `command_end` 事件（命令真正结束）
- 用户按 Esc 主动取消

### 3.2 全屏 xterm 触发裁剪

- **删掉 `fulltermCommands` 白名单触发**（L224 整段）。claude/codex 等命令走默认 Block UI 模式。
- **alt-screen 触发收紧**：只有 process name 在 TUI 黑名单（vim / nvim / htop / btop / less / nano / man / top / tmux / pico）才允许自动切 fullterm；其他即使后端检测到 alt-screen 也忽略（这种情况罕见，且通常是错误命中）。
- 用户手动快捷键切 fullterm 不受影响。

### 3.3 LiveTerminalBlock 删除

整个组件 + 测试 + 管理器 + 在 `UnifiedTimeline.tsx` 的引用 + 在 `actions.ts` 的引用一律删除。

命令运行中的输出**继续**通过 `terminal_output` 事件流到 timeline 里的 `CommandBlock`（已经在做），所以这块视觉上无变化。

## 4. API 契约

### 4.1 后端事件 `stdin_wait`

```
event: "stdin_wait"
payload:
  session_id: string
  command: string | null   // 当前正在运行的命令（如有）
  detector: "idle_prompt" | "powershell_choice" | "yn_pattern"  // 启发式来源
```

### 4.2 后端事件 `command_end`（已有）

复用现有事件，前端接收时把对应 session 的 `interactiveMode` 设回 false。

### 4.3 stdin_wait 启发式探测算法

在 PTY 读取线程中，每次 `terminal_output` 发送后：

1. 维护「最后输出时间戳 + 累积输出尾部 N=256 字节」
2. 如果距离最后输出 ≥ **300 ms** 没新字节：触发以下任一启发式 → 发 `stdin_wait`：
   - **`idle_prompt`**：尾部含 `:` 或 `?` 或 `#` 后跟可选空格然后行尾，且当前不在 prompt_end 状态（即命令还在执行）
   - **`powershell_choice`**：尾部匹配正则 `\[Y(?:es)?\]\s*\[N(?:o)?\]` 或 `[Y/n]` / `[y/N]` / `(yes/no)`
   - **`yn_pattern`**：尾部含 `Continue?` / `Are you sure?` / `Press any key` 等关键词
3. 同一命令只发一次 `stdin_wait`，新一轮 `terminal_output` 到达后再次重置 idle 计时器

误报兜底：
- `stdin_wait` 发出后如果 1.5 s 内没有用户响应也没新输出，前端**不**主动重发，但后端会在下一次 idle 触发时重发
- 用户主动 Esc 退出 interactive 模式后，**短暂屏蔽** 同一命令 1 s 内的新 `stdin_wait`（避免立刻又跳起来）

### 4.4 前端 store 新增字段

```ts
type SessionState = {
  // ... 现有字段 ...
  interactiveMode: {
    active: boolean;
    command: string | null;
    enteredAt: number;  // 时间戳
  } | null;
};

setInteractiveMode(sessionId: string, mode: SessionState["interactiveMode"]): void
```

## 5. 实现拆解

### 5.1 后端

1. `backend/crates/golish-pty/src/manager/emitter.rs`：新增 `emit_stdin_wait` 方法。
2. `backend/crates/golish-pty/src/manager/session_create.rs` 的 reader 线程：维护尾部 buffer + idle timer，触发启发式探测后调 `emitter.emit_stdin_wait`。
3. `backend/crates/golish-pty/src/manager/emitter.rs`：定义 `StdinWaitEvent` 结构。
4. `backend-tauri/src/events.rs`（或类似文件）：把事件桥接到 Tauri。

### 5.2 前端

1. `frontend/lib/api/tauri-events.ts`（或 `tauri-event-types.ts`）：新增 `stdin_wait` 事件类型。
2. `frontend/store/types/session.ts`：加 `interactiveMode` 字段。
3. `frontend/store/slices/session-core.ts`：加 `setInteractiveMode` action。
4. `frontend/hooks/useTauriEvents.ts`：
   - 删除 L224 `fulltermCommands` 自动 fullterm 块
   - 改 L410 alt-screen 处理，只在 TUI 黑名单进程名时才切 fullterm
   - 新增 `stdin_wait` 监听 → `setInteractiveMode(sid, { active: true, command, enteredAt: now })`
   - 在 `command_end` 处理里追加 `setInteractiveMode(sid, null)`
5. `frontend/components/UnifiedInput/UnifiedInput.tsx`（或其 hook `useUnifiedInputState.ts`）：
   - 读 `interactiveMode`，true 时改 placeholder、颜色、Enter 行为
   - 顶部加一条 banner 显示「正在与 `$cmd` 交互」
6. `frontend/components/UnifiedTimeline/UnifiedTimeline.tsx`：移除对 `LiveTerminalBlock` 的渲染。
7. 删除：
   - `frontend/components/LiveTerminalBlock/`（整目录）
   - `frontend/lib/terminal/LiveTerminalManager.ts`
   - `frontend/lib/terminal/LiveTerminalManager.test.ts`
   - `frontend/lib/terminal/index.ts` 中的 `liveTerminalManager` 导出
   - 其他文件中所有 `liveTerminalManager` / `LiveTerminalBlock` 引用

### 5.3 测试

| 测试 | 类型 |
|---|---|
| `stdin_wait` 启发式：尾部含 `[Y/n]` 时触发 | Rust unit |
| `stdin_wait` 启发式：纯 stdout 行尾非 prompt 时不触发 | Rust unit |
| `stdin_wait` 启发式：300 ms idle 阈值正确 | Rust unit + tokio test |
| useTauriEvents 收到 `stdin_wait` 调 setInteractiveMode | Vitest |
| UnifiedInput interactiveMode true 时 Enter 走 ptyWrite，不走 createCommand | Vitest |
| UnifiedInput 显示 banner、placeholder 切换 | Vitest |
| TUI 黑名单：vim 仍能切 fullterm | Vitest |
| 非 TUI alt-screen：不切 fullterm | Vitest |

### 5.4 手动验证场景

- bash: `select yn in "Yes" "No"; do echo $yn; break; done` → 输入框变身，按 1 回车
- bash: `read -p "Continue? [Y/n] " c` → 输入框变身，按 Y 回车
- npm init → 多轮交互
- PowerShell（Windows）: `Install-Module Pester` → 弹 Untrusted repository 问 Y → 输入框变身
- vim foo.txt → 仍切 fullterm，:wq 退出回 Block UI
- claude → 走 Block UI（不再自动 fullterm，看视觉效果是否可接受）

## 6. 风险与回滚

### 6.1 风险

- **stdin_wait 误报**：启发式可能在命令正常输出 `Loading…` 等带 `:` 的字符时误判。缓解：300 ms idle 阈值 + 正则严格。
- **stdin_wait 漏报**：奇怪的 prompt（如自定义 reset color 后接 prompt）可能不触发。缓解：用户仍可用 Esc 退出，或在输入框手动切换。
- **claude/codex 走 Block UI 后体验下降**：可能 spinner 动画不流畅。缓解：保留 `fullterm_commands` 配置项，用户可手动加回白名单（Phase B 上来后用 GridTerminal 渲染就完全 OK）。

### 6.2 回滚

- 全部改动集中在 Phase A 的几个文件，git revert 单 PR 即可。
- 配置开关：在 `settings.toml [terminal]` 加 `disable_interactive_mode = false` 默认关闭新逻辑作为兜底。

## 7. 后续 Phase

- **Phase B**：Rust `alacritty_terminal` + 前端 `GridTerminal` 组件，彻底替代 xterm.js。vim/htop 不再依赖 xterm，Windows 黑屏根治。
- **Phase C**：TUI 命令 Block 默认折叠为一行，点开看最后一帧。
