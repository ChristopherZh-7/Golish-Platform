# Phase B + Phase C 续接文档（Handoff）

**目的**: 让接手者无需对话历史也能继续推进。
**前置阅读**（必读，按顺序）:
1. `docs/design/2026-05-15-warp-style-interaction.md` — Phase A 设计（已完成上线）
2. `docs/design/2026-05-15-grid-terminal-phase-b.md` — Phase B 设计文档（架构 + 协议 + 6 天计划）
3. 本文档 — 接力棒：当前进度 + 剩余工作详细 TODO

---

## 0. TL;DR

- **目标**：用 Rust `alacritty_terminal` + React grid 渲染替换 xterm.js，根治 Windows WebView2 黑屏 bug。
- **进度**：Phase A 全部交付；Phase B 全部交付（D1 + D2 + D3 + D4 + D5 + D6.1/D6.2/D6.3/**D6.4b**）。`use_grid_renderer` 默认 `true`，xterm.js 4 个核心 npm 包已 `pnpm remove`，老渲染层 + portal + RecordingsPanel + TerminalInstanceManager 全部删干净。剩余：Phase C 的性能压测 / 跨平台 QA / 录制功能决策（需手动 dogfood）。
- **当前里程碑**：dogfood 一轮真实 vim / htop / pwsh / 大量 cargo build，确认无回归后即可发版。如果 GridTerminal 在某个场景失败，**没有 xterm.js 兜底**——要么补 GridTerminal，要么临时复活旧渲染层（D6.4b 提交是单 commit，`git revert` 即可恢复）。

---

## 1. 已完成内容 · 代码快照

### Phase A（Warp 风格交互输入·已上线）

| 模块 | 文件 | 摘要 |
|---|---|---|
| 后端探测 | `backend/crates/golish-pty/src/manager/stdin_wait_detector.rs` | 5 种启发式（YnChoice / Password / PowerShellChoice / Continue / GenericPrompt），15 个单测 |
| 后端事件 | `manager/emitter.rs` + `session_create.rs` | `emit_stdin_wait`，idle 300ms 触发 |
| 前端 store | `frontend/store/slices/session-core.ts` `setInteractiveMode` | + 5 单测 |
| UI | `frontend/components/UnifiedInput/{UnifiedInput.tsx,useUnifiedInputState.ts,useInputKeyboard.ts}` | 双模式，Esc 退出 |
| 替代 | `frontend/components/UnifiedTimeline/RunningCommandCard.tsx` | 取代被删的 LiveTerminalBlock |
| 删除 | `LiveTerminalBlock/`、`LiveTerminalManager.ts` | 共 ~1200 行 |
| 收紧 | `frontend/hooks/{useTauriEvents.ts,tauri-event-types.ts}` | alt-screen 只在 `ALT_SCREEN_TUI_PROCESSES` 时切 fullterm |

### Phase B 已完成（D1 + D2 + D3）

#### D1 · 后端 grid 状态机
- `backend/crates/golish-pty/src/grid/` 模块（mod / cell / terminal / snapshot / tests）
- 25 单测全过
- 依赖 `alacritty_terminal=0.26` / `unicode-width=0.2` / `bitflags=2`
- 公共 API: `GridManager`、`GridTerminal::{write,resize,snapshot_full,snapshot_diff,alt_screen,cols,rows,rev}`、`GridUpdate`、`RowUpdate`、`Cell`、`Color`、`CellAttrs`、`Cursor`、`CursorStyle`、`GridDims`

#### D2 · grid 接入 emitter 线程 + Tauri 命令
- `manager/emitter.rs`: `PtyEventEmitter::emit_grid_update` 新方法，`RuntimeEmitter` 把 session_id flatten 进 GridUpdate JSON 后 emit `Custom { name: "terminal_grid_update" }`
- `manager/core.rs`:
  - `ActiveSession` 新字段 `alt_screen: Arc<AtomicBool>`
  - `PtyManager` 新字段 `grid_manager: Arc<GridManager>`
  - 新公共方法 `grid_terminal(sid) -> Option<Arc<Mutex<GridTerminal>>>` 和 `resize_grid(sid, cols, rows)`
- `manager/session_create.rs`:
  - `dispatch_parsed_events` 接 `grid_manager`：alt-screen on → set flag + eager create；off → dispose
  - emitter 线程多两段：alt-screen 上升沿立刻 `snapshot_full` + emit；其余每次写入后 60ms coalesce 发 `snapshot_diff`（`GRID_EMIT_INTERVAL`）
- `backend/crates/golish/src/commands/proc/pty.rs`:
  - 新 Tauri 命令 `pty_request_grid_snapshot(sid) -> Option<GridUpdate>`
  - 新 Tauri 命令 `pty_resize_grid(sid, cols, rows)`
- `commands_registry.rs` 已注册

#### D3 · 前端 GridTerminal 组件
- `frontend/components/GridTerminal/`:
  - `useGridState.ts`: 订阅 `terminal_grid_update`，full/diff merge，rev 跳跳自动 `pty_request_grid_snapshot` 恢复
  - `GridRow.tsx`: 单行 memo 组件，每个 cell 用 `<span>` + className/style
  - `GridTerminal.tsx`: 主组件，「等待首帧」占位 + 网格渲染
  - `index.ts`: 公共导出
  - `useGridState.test.ts`: 6 单测全过
- `frontend/lib/events/payloads.ts`: 加 `TerminalGridUpdatePayload`、`GridCellPayload`、`GridRowPayload`、`GridCursorPayload`、`GridColor`、`GridCursorStyle`；进 `EventPayloadMap`
- `frontend/lib/api/pty.ts`: 加 `ptyRequestGridSnapshot` / `ptyResizeGrid`
- `frontend/styles/grid-terminal.css`: 字体 / 颜色 token / bold / italic / underline / cursor / prefers-reduced-motion

#### D4 · 键盘 + 尺寸 + 焦点
- `frontend/components/GridTerminal/keymap.ts` 全键位映射（printable / Ctrl+x / Alt+x / 箭头 DECCKM / F1-F12 / PageUp 等 + modifier suffix），17 单测
- `useGridKeyboard.ts` keydown / paste / compositionend 监听 container（不监听 window，避免多 pane 冲突）
- `useGridResize.ts` ResizeObserver + 「M」字符 cell metrics 探针 + 100 ms debounce + 同时调 `ptyResize` / `ptyResizeGrid`
- 后端 `GridUpdate` 添 `app_cursor_mode: bool` 字段（前端 keymap 用）
- `GridTerminal.tsx` 集成：`tabIndex=0`、autoFocus、onMouseDown 重取焦点、`data-focused` / `data-app-cursor` 调试标记
- CSS 加 `[data-focused="true"]` inset box-shadow

#### D5 · Settings flag + PaneLeaf 双路渲染
- `golish-settings::TerminalSettings.use_grid_renderer: bool`，默认 `false`
- 前端 `lib/settings/{types,defaults}.ts` 同步
- `frontend/components/PaneContainer/PaneLeaf.tsx` 读 settings + 听 `settings-updated`；fullterm + flag 为 true 时挂 `<GridTerminal>`（lazy），否则保留旧 xterm portal —— 互斥
- `frontend/components/Settings/TerminalSettings.tsx` 加 checkbox 「Use GridTerminal renderer (experimental)」

#### D6 · 保守清理（已完成的部分）
- 后端 schema：`fullterm_commands` 改 `Option<Vec<String>>`（向后兼容老 settings.toml），前端类型同步标 `@deprecated`
- 新增 `e2e/grid-terminal.spec.ts` 5 个 case（empty state / full frame / diff merge / app_cursor_mode / 跨 session 过滤）
- `docs/architecture.md` Terminal 行同步、`CHANGELOG.md` 加 Phase A + B 条目、本 handoff 文档更新

### 验证基线（D6 保守部分完成时）

```bash
cd backend && cargo test -p golish-pty
# 168 passed; 0 failed

cd backend && cargo build --workspace
# Finished `dev` profile (clean)

cd .. && pnpm typecheck
# clean

cd .. && pnpm vitest run frontend
# 965 passed | 42 failed | 12 skipped
# 42 failed 是预存在的 useAiEvents.test.ts + useTauriEvents.test.ts（@tauri-apps/api/event 没 mock，d1-vitest-react19 issue），非本次回归
```

---

## 2. 剩余工作 · 详细任务

### ✅ Phase B · D4 — keymap + 尺寸 + 焦点 — COMPLETED

（详见 §1 「D4 · 键盘 + 尺寸 + 焦点」）

### ✅ Phase B · D5 — Settings flag + PaneLeaf 双路渲染 — COMPLETED

（详见 §1 「D5 · Settings flag + PaneLeaf 双路渲染」）

### ✅ Phase B · D6 (保守部分) — schema 收敛 + e2e + 文档 — COMPLETED

### ✅ Phase B · D6.4b — 删 xterm.js（已完成 2026-05-15）

D6.4b 在用户明确指示下**先删，后 dogfood**（违反原计划的 dogfood-first 推荐），因此当 GridTerminal 在任何场景失败时**没有 xterm.js 兜底**。回滚策略：D6.4b 是单次提交，`git revert <sha>` 即可。

**已落地清单**：

| 类别 | 动作 |
|---|---|
| Settings | `use_grid_renderer` 默认 `true`（backend `schema/ui.rs` + frontend `defaults.ts`）；Settings UI 文案去掉 `experimental` 标 |
| npm | `pnpm remove @xterm/xterm @xterm/addon-fit @xterm/addon-webgl @xterm/addon-web-links`。**保留** `@xterm/headless` + `@xterm/addon-serialize`（`VirtualTerminal.ts` 用，给静态 ANSI 文本预处理；不渲染） |
| 删除文件 | `frontend/components/Terminal/{Terminal,Terminal.test,Terminal.webgl.test,TerminalLayer,RecordingsPanel,TerminalRecordingControls,index}` · `frontend/components/CommandBlock/StaticTerminalOutput.tsx`（无人引用） · `frontend/lib/terminal/{TerminalInstanceManager,TerminalInstanceManager.test,SyncOutputBuffer}.ts` · `frontend/hooks/useTerminalPortal.{tsx,test.tsx}` · `frontend/styles/xterm-overrides.css` · `frontend/lib/theme/ThemeManager.batching.test.ts` · `e2e/terminal-portal-architecture.spec.ts` |
| 重构 | `PaneLeaf.tsx` 去 dual-render，统一 `<GridTerminal>` · `DetachedView.tsx` 改 lazy GridTerminal · `AppShell.tsx` 去 `TerminalPortalProvider` / `TerminalLayer` / `RecordingsPanelView` 渲染槽 · `TabBar.tsx` 去 `TerminalRecordingControls` · `CommandPalette.tsx` 把 "Terminal Recordings" 菜单项守在 `onOpenRecordings` 存在时 · `dialog.ts` 去 `recordingsPanelOpen*` · `lazyRegistry.ts` 去 `RecordingsPanelView` · `lib/terminal/index.ts` 去 `TerminalInstanceManager` / `SyncOutputBuffer` 导出 · `ThemeManager` 去 `applyToTerminal`（GridTerminal 走 CSS 变量） |
| 调用点改 no-op | `store/actions.ts` · `store/slices/{pane,session-core,session-tabs}.ts` · `lib/{conversation-db-sync,terminal-restore}.ts` · `hooks/useCreateTerminalTab.ts` · `components/AIChatPanel/hooks/useChatConversationOps.ts`（GridTerminal 由 server-side `pty_destroy` 接管销毁；scrollback 持久化退化为空字符串） |
| 测试 | `UnifiedTimeline.{test,memo.test,optimized.test}.tsx` + `PaneLeaf.{lazy,memo}.test.tsx` + `AgentMessage.performance.test.tsx` 去 xterm mock；`bundle-optimization.test.ts` 去 `xterm` chunk 期望；`e2e/input-mode-focus.spec.ts` 去 `xterm-helper-textarea` 判断 |
| 配置 | `vite.config.ts` 去 xterm 单独 chunk · `index.css` 注释把 `.xterm-viewport` 改为 `.grid-terminal__scroll` |
| ErrorBoundary | 去 `@xterm/_renderer` race 过滤分支 |

**验证状态**（commit 时跑过）:
- `pnpm typecheck` 干净
- `cargo build --workspace` 干净
- `cargo test -p golish-pty -p golish-settings`: 54/54 pass
- 受影响 vitest 子集: 208/208 pass + 68/68 pass
- 全量 vitest: 940 pass / 42 fail（全部 pre-existing：40 个 `d1-vitest-react19` 已知 flaky + 2 个 `HomeView.memo.test.tsx` 已知 flaky）。stash + 复跑确认数字一致

#### D6.4 剩余收尾（可选）

- `settings.terminal.fullterm_commands` 仍是 `Option<Vec<String>>` 保留向后兼容。下一版本可以整字段删掉。
- `frontend/lib/terminal/VirtualTerminal.ts` 还在用 `@xterm/headless` + `@xterm/addon-serialize`，用于命令静态输出的 ANSI 文本预处理。如果要 0 xterm 依赖，需要自写一个简易 ANSI 状态机替换它（工作量约 200 行 + 单测，跨平台无 native binding）。

#### D6.4 必做跨平台 QA（dogfood，需手动）
| 场景 | macOS | Linux | Windows |
|---|---|---|---|
| vim hello | ☐ | ☐ | ☐ |
| htop | ☐ | ☐ | ☐ |
| less <large-file> | ☐ | ☐ | ☐ |
| tmux | ☐ | ☐ | ☐ |
| CJK / IME 输入 | ☐ | ☐ | ☐ |
| 颜色（cargo build 进度条） | ☐ | ☐ | ☐ |
| 黑屏（Windows 重点） | N/A | N/A | ☐ |
| 大量输出（10MB+ log） | ☐ | ☐ | ☐ |
| Ctrl-C 中断 / Ctrl-D EOF | ☐ | ☐ | ☐ |
| PowerShell `Read-Host` 交互 | ☐ | ☐ | ☐ |

---

### Phase B · D4 详细参考（已完成，留作上下文）

**目标**：让 GridTerminal 真正可交互（接键盘 + 自适应尺寸）。

#### D4.1 keymap.ts — 键盘 → ANSI 序列表

新文件 `frontend/components/GridTerminal/keymap.ts`。导出一个函数：

```ts
export function keyEventToAnsiBytes(
  event: React.KeyboardEvent,
  appCursorMode: boolean
): string | null;
```

需要覆盖的键位（参考 xterm.js 默认 + alacritty 默认）:

| 键 | normal | application cursor mode |
|---|---|---|
| ArrowUp | `\x1b[A` | `\x1bOA` |
| ArrowDown | `\x1b[B` | `\x1bOB` |
| ArrowRight | `\x1b[C` | `\x1bOC` |
| ArrowLeft | `\x1b[D` | `\x1bOD` |
| Home | `\x1b[H` | `\x1bOH` |
| End | `\x1b[F` | `\x1bOF` |
| PageUp | `\x1b[5~` | 同 |
| PageDown | `\x1b[6~` | 同 |
| Insert | `\x1b[2~` | 同 |
| Delete | `\x1b[3~` | 同 |
| F1–F4 | `\x1bOP/Q/R/S` | 同 |
| F5–F12 | `\x1b[15~`/17/18/19/20/21/23/24~ | 同 |
| Enter | `\r` | 同 |
| Backspace | `\x7f` | 同 |
| Tab | `\t`，Shift+Tab `\x1b[Z` | 同 |
| Escape | `\x1b` | 同 |
| Ctrl + a..z | 对应控制字符（`Ctrl-a` = `\x01`） | 同 |
| Alt + char | `\x1b<char>`（ESC prefix） | 同 |
| 普通可打印字符（`event.key.length === 1` 且非控制） | 直接 UTF-8 字节 | 同 |
| Paste | bracketed `\x1b[200~<text>\x1b[201~` | 同 |

注意:
- application cursor mode 来自后端 `TerminalGridUpdatePayload` —— D4 顺手在 payload 加一个布尔字段 `app_cursor_mode` 通过 `term.mode().contains(TermMode::APP_CURSOR)` 暴露
- Composition events（IME 中文输入）：跳过 `event.isComposing === true` 的 keydown，监听 `compositionend` 输出合成串
- 把现有 Phase A 的 IME 处理逻辑（`useUnifiedInputState.handleFocus/handleBlur`）参考一遍，对 GridTerminal 也做 `imeSetSource("com.apple.keylayout.ABC")` 切英文

新 hook `useGridKeyboard.ts`：在 `GridTerminal` 上 `tabIndex={0}` + `onKeyDown` + `onPaste` + `onCompositionEnd`，把序列调 `ptyWrite(sessionId, bytes)`。

单测目标:
- `Ctrl-c` → `\x03`
- `ArrowUp` 在 normal/app 模式下序列不同
- `Tab` / `Shift-Tab`
- IME composition skip
- bracketed paste 包裹

#### D4.2 ResizeObserver → ptyResizeGrid

新 hook `useGridResize.ts`:
- 接收 `containerRef` 和 `sessionId`
- 测量 container clientWidth / clientHeight
- 测量单 cell 大小：拿一个 `<span>测试字符</span>`，`getBoundingClientRect` 得到 ch 宽和行高
- 算出 `cols = floor(width/ch)`、`rows = floor(height/lineHeight)`
- 同时调 `ptyResizeGrid(sid, cols, rows)` 和 `ptyResize(sid, rows, cols)`（PTY 那边也要 SIGWINCH）
- debounce 100ms

集成到 `GridTerminal.tsx` 一个 `<div ref={containerRef}>` 包裹。

#### D4.3 焦点

- GridTerminal 容器加 `tabIndex={0}`，CSS `outline: none`，但 focus 时给 `data-focused` 加细的 border
- 鼠标点击 → focus
- 后端发 alt-screen → 自动 focus（第一次）

---

### Phase B · D5 — dual-render flag 接入 + dogfood（~1 天）

**目标**：让 GridTerminal 真正在产品里跑起来，灰度可关。

#### D5.1 Settings 加 flag

`backend/crates/golish-settings/src/schema/ui.rs` `TerminalSettings`:
```rust
/// Render alt-screen TUI applications (vim, htop, less, …) through
/// the Phase B GridTerminal stack instead of xterm.js. Defaults to
/// false until D6 ships; flipped on once Windows / macOS / Linux all
/// pass the manual verification matrix.
#[serde(default)]
pub use_grid_renderer: bool,
```

`frontend/lib/settings/types.ts` 同步加 `use_grid_renderer: boolean`，`frontend/lib/settings/defaults.ts` 默认 `false`。

#### D5.2 PaneLeaf 切换路径

`frontend/components/PaneContainer/PaneLeaf.tsx`:
- 现状: `renderMode === "fullterm"` 时挂 `<div ref={terminalPortalRef}>` + xterm.js portal
- 改: 读 `settings.terminal.use_grid_renderer`，true 且 alt-screen → 挂 `<GridTerminal sessionId={sessionId} enabled />`；否则保留旧 xterm portal

需要把 `enabled` 从 store 推断：`session.renderMode === "fullterm"` AND `useGridRenderer`。

#### D5.3 dogfood 验证清单

`cargo tauri dev` 跑起来后：
- bash: `vim foo.txt` → i → "hello world" → Esc → :wq → 验证文件
- bash: `htop` → F10 退出
- bash: `less LICENSE` → space space → q
- bash: `tmux new` → Ctrl-b c 开新 pane → exit
- bash: `nano test.txt` → 输入 → Ctrl-X
- macOS: `top` → q
- PowerShell (Windows): `vim` 是否能跑（取决于 git for windows / wsl）
- CJK: `echo "你好 hello 世界"` 在 alt-screen 内对齐
- claude / codex 启动 → 走 Block UI（已是 Phase A 状态）

每个场景记录是否 OK，把 fail 项写到 D6 修复清单。

---

### Phase B · D6 — 删 xterm.js 依赖 + e2e + 跨平台（~1–2 天）

**目标**：清理工程，flag 翻 true，发版准备。

#### D6.1 移除 npm 依赖

```bash
pnpm remove @xterm/xterm @xterm/addon-fit @xterm/addon-webgl @xterm/addon-web-links @xterm/addon-serialize
# 保留 @xterm/headless（VirtualTerminal.ts 用，用于流式 ANSI → 静态文本）
```

文件清理：
- `frontend/components/Terminal/Terminal.tsx` — 整文件删除（GridTerminal 替代）
- `frontend/components/Terminal/Terminal.test.tsx` `Terminal.webgl.test.tsx` — 删
- `frontend/components/Terminal/TerminalLayer.tsx` — 改/删，portal 不需要了
- `frontend/components/Terminal/RecordingsPanel.tsx` — 决策：(a) 删功能 (b) 用 GridReplayer 重写。建议 (a)，因为录制场景小众
- `frontend/components/Terminal/TerminalRecordingControls.tsx` — 跟 (a) 一起删
- `frontend/components/Terminal/index.ts` — 删导出
- `frontend/lib/terminal/TerminalInstanceManager.ts` — 删（GridManager 取代）
- `frontend/lib/terminal/TerminalInstanceManager.test.ts` — 删
- `frontend/lib/terminal/SyncOutputBuffer.ts` — 评估，可能不再需要
- `frontend/styles/xterm-overrides.css` — 删
- `frontend/components/CommandBlock/StaticTerminalOutput.tsx` — 改用 ansi-to-react 输出（已是这样了），删 xterm 相关注释
- `frontend/components/PaneContainer/PaneLeaf.tsx` — 去掉 dual-render flag，统一走 GridTerminal
- `frontend/hooks/useTerminalPortal.ts` `useTerminalPortal.test.tsx` — 删
- `frontend/components/DetachedView/DetachedView.tsx` — 也用到 xterm，需重构成 GridTerminal 或暂时禁用
- `vite.config.ts` — 移除 xterm chunk 配置（grep 一下 `@xterm`）
- `e2e/terminal-portal-architecture.spec.ts` — 重写为 GridTerminal 架构（核心场景）
- `e2e/input-mode-focus.spec.ts` — 可能需要调整选择器

更新 `frontend/test/setup.ts` —— 已经移除 liveTerminalManager mock，确认 virtualTerminalManager mock 仍 OK。

#### D6.2 把 Settings flag 默认翻 true

`use_grid_renderer = true`，删 `backend/crates/golish-settings/src/schema/ui.rs` 里的 `fullterm_commands` 字段（Phase A 已不再使用），从 `frontend/lib/settings/types.ts` 同步删。

#### D6.3 e2e 测试

`e2e/grid-terminal.spec.ts`（新）：
- 启动 vim → 输入 → :wq → 验证（headless tauri 跑得动的话）
- 网格渲染时数据 selector：`[data-testid="grid-terminal"]`，行：`.gt-row`

#### D6.4 跨平台手动 QA 表

| 场景 | macOS | Linux | Windows |
|---|---|---|---|
| vim hello | | | |
| htop | | | |
| less | | | |
| tmux | | | |
| CJK | | | |
| 颜色（cargo build 进度条） | | | |
| 黑屏（Windows 重点） | N/A | N/A | |
| 性能（大输出） | | | |

#### D6.5 文档更新

- `docs/architecture.md` 「终端渲染」一节 → 删 xterm 介绍，加 GridTerminal 架构图
- `docs/windows-support.md` → 标注 Windows 黑屏 bug 已根治
- `CHANGELOG.md` → 添 `feat(terminal): GridTerminal replaces xterm.js for TUI rendering`
- `docs/risks/d1-vitest-react19.md` → 评估是否还有效（很多失败 test 都是 Terminal/xterm 相关，删完应该好很多）

---

### Phase C — TUI 折叠 + 跨平台 QA + 润色（~1–2 天）

#### C.1 TUI 退出后折叠

`frontend/components/CommandBlock/CommandBlock.tsx`：
- 检测 `block.command` 命中 `ALT_SCREEN_TUI_PROCESSES`（用 `extractProcessName` 工具）
- 命中 → 默认 `isCollapsed: true`，渲染只显示 `~ (Xs) vim foo.txt`
- 点击展开 → 显示 GridTerminal 的最后一帧快照

折叠后内容怎么存：D2 emitter 在 alt-screen 退出前先记录 final snapshot 写到 store。需要新加 store 字段 `gridSessionFinalFrames: Record<sessionId, GridUpdate>`，或者把 final frame 序列化进 `CommandBlock.output`。

#### C.2 删 `frontend/styles/xterm-overrides.css`（如果 D6 没删完）

#### C.3 跨平台回归全跑一遍

#### C.4 性能压测

- benchmark: `cargo build` 大输出 → 测前端 fps，看 React 渲染 1920 个 span 是否吃力
- benchmark: vim 内 G（跳末行）+ gg（跳首行）连续滚动 → 测后端 grid serialize 耗时

如果发现瓶颈：
- 前端：用 `react-window` 虚拟化超大网格（>50 行）
- 后端：grid 序列化用更紧凑格式（cell 数组压缩成「同属性连续运行」）

#### C.5 录制功能（可选）

如果决定保留 recordings 功能，写 `frontend/components/Terminal/GridReplayer.tsx`：存储 `GridUpdate[]`，按时间戳回放。

---

## 3. 关键决策上下文（避免接手者走弯路）

### 3.1 Color JSON 形状

是 **struct-variant**，不是 tuple：
```json
// 对的
{"fg": {"kind": "rgb", "value": 16711935}}
// 错的
{"fg": ["rgb", 16711935]}
```
serde 的 `#[serde(tag = "kind", rename_all = "snake_case")]` 配合 `Indexed { value: u8 }` / `Rgb { value: u32 }` 是这个形状的关键。

### 3.2 alacritty_terminal 0.26 API 关键点

- `Term::new(Config, &impl Dimensions, EventListener)`
- 用 `alacritty_terminal::term::test::TermSize::new(cols, rows)` 当 Dimensions
- `EventListener` 用 `VoidListener`（不需要 mouse/clipboard 等事件）
- 写入：`vte::ansi::Processor::new().advance(&mut term, bytes)` —— 注意 vte 必须从 `alacritty_terminal::vte` re-export 拿（版本要匹配），不能直接用 crate `vte=0.13`
- 读取：`term.grid()[Line(y)][Column(x)]` 返回 `&Cell`
- damage: `term.damage()` 返回 `TermDamage::{Full, Partial(iter)}`，然后 `term.reset_damage()` 清掉

### 3.3 GridTerminal 在哪个线程跑

emitter 线程（不是 reader 线程）。原因：
- reader 线程已经在用 `TerminalParser`（Golish 自家的 OSC 解析器），不要混入第二份 vte
- emitter 线程负责 coalesce + 出口 IPC，grid 数据本来就要从这里出
- GridTerminal 在 `Arc<Mutex<>>` 后面，Tauri 命令线程（snapshot / resize）也能访问

### 3.4 alt-screen 信号怎么传

`reader_session.alt_screen: Arc<AtomicBool>` —— reader 线程的 `dispatch_parsed_events` 翻它，emitter 线程在每个 Timeout tick 读它。Release/Acquire 顺序就够（无需 SeqCst）。

### 3.5 60ms coalesce 关键

`GRID_EMIT_INTERVAL = 60ms` 在 `manager/session_create.rs` 顶部。比 PTY 输出的 16ms coalesce 大一档，避免每次输出 burst 都触发 grid 序列化（昂贵 O(rows×cols) JSON）。`grid_pending_emit` 标志保证只有真正写过 grid 才会发。

### 3.6 rev 跳跳恢复

前端检测 `payload.rev !== lastRev + 1` 时调 `pty_request_grid_snapshot` 拉 full。后端 `snapshot_full()` 会同步 `served_full_since_creation = true`，下一次 diff 又是增量。

### 3.7 为什么 stdin_wait 探测放在 emitter 线程而不是 reader

- emitter 线程已经在每 16ms tick 检查超时
- 探测要看「已经发给前端的字节尾部」，所以必须在 coalesce 之后
- reader 线程只跑 vte parser，不该负责业务逻辑

### 3.8 测试基础设施已知坑

- `frontend/hooks/useTauriEvents.test.ts`（42 failed 里大部分） — 这文件没有 `vi.mock("@tauri-apps/api/event")` mock，是历史 d1 issue（参考 `docs/risks/d1-vitest-react19.md`）。D6 时清掉 xterm 后这文件可能也要重写
- 新写测试时记得 mock `@/lib/events.onEvent`，可参考 `useGridState.test.ts` 的 stub pattern

---

## 4. 接手者验证清单

接手 5 分钟内跑一遍确认基线没坏：

```bash
# 1. Rust 后端
cd backend && cargo test -p golish-pty 2>&1 | tail -3
# 期望: 168 passed

# 2. Rust workspace
cargo build --workspace 2>&1 | tail -3
# 期望: Finished `dev` profile

# 3. 前端 typecheck
cd .. && pnpm typecheck 2>&1 | tail -3
# 期望: 无错误

# 4. 前端单测
pnpm vitest run frontend/components/GridTerminal frontend/components/UnifiedInput 2>&1 | tail -7
# 期望: GridTerminal 6/6 + UnifiedInput 65/5 skipped 全过

# 5. 跑一下 dev（验证 D3 渲染层在 Tauri 里是否能 boot）
cd src-tauri && cargo tauri dev
# 期望: 不黑屏，正常 home 视图
```

---

## 5. Git 当前 diff 总览

```
git status --short
```

应该看到（截至 D3 完成时）：
- Phase A 改动：`frontend/{components/{LiveTerminalBlock(del)/,UnifiedInput,UnifiedTimeline,PaneContainer},lib/{events/payloads,api/pty,terminal/index(del LiveTerminalManager)},hooks/{useTauriEvents,tauri-event-types},store/{slices/{session,session-core},types/{session,index},store-types},test/setup}.{ts,tsx}`
- Phase B D1-D3 新增：`backend/crates/golish-pty/src/grid/`（5 文件）、`frontend/components/GridTerminal/`（5 文件）、`frontend/styles/grid-terminal.css`
- Phase B D2 改动：`backend/crates/golish-pty/{Cargo.toml,src/manager/{core,emitter,session_create,mod}.rs,src/lib.rs}`、`backend/crates/golish/src/{commands/proc/pty.rs,commands_registry.rs}`
- 设计文档：`docs/design/2026-05-15-{warp-style-interaction,grid-terminal-phase-b,phase-b-handoff}.md`

未提交 → 接手者可以选择：
- 先 `git add -A && git commit -m "phase A complete + phase B D1-D3"` 把当前进度封一个 checkpoint
- 或者每个 D 一个 commit（推荐）

---

## 6. 上手指令

把这份文档发给新 session 的 AI，配一句话即可：

> 接手 Golish-Platform 的 Phase B 工作。
> 1. 读 `docs/design/2026-05-15-warp-style-interaction.md`（已完成的 Phase A 上下文）
> 2. 读 `docs/design/2026-05-15-grid-terminal-phase-b.md`（Phase B 整体设计）
> 3. 读 `docs/design/2026-05-15-phase-b-handoff.md`（当前进度 + 剩余 TODO）
> 4. 先跑 §4 的 5 个验证命令确认基线
> 5. 启动 Phase B D4（keymap + resize + 焦点）

新 AI 不需要任何对话历史就能继续。
