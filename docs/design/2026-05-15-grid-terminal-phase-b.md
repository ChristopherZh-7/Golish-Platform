# GridTerminal · 替代 xterm.js（Phase B）

**作者**: fullstack-dev agent
**日期**: 2026-05-15
**状态**: Draft / 待实施（Phase A 已上线，参考 `2026-05-15-warp-style-interaction.md`）
**预计工作量**: 4–6 天

## 1. 背景与目标

### 1.1 痛点

- xterm.js 在 Tauri WebView2（Windows）上 WebGL renderer 会黑屏，Canvas renderer 性能不行
- 项目目前还依赖 xterm 仅仅是为了「TUI 应用（vim / htop / less / nano …）能在 fullterm 模式跑」
- 引入了一大堆隐式技术债：`@xterm/xterm` + `@xterm/addon-webgl` + `@xterm/addon-fit` + `@xterm/addon-serialize` + `@xterm/addon-web-links` + `@xterm/headless`
- 命令普通输出渲染走了两套（CommandBlock 用 ansi-to-react、Terminal/LiveTerminal 用 xterm.js），风格不统一

### 1.2 目标

| 维度 | 现状 | Phase B 之后 |
|---|---|---|
| TUI（vim/htop/less）渲染 | xterm.js + WebGL/Canvas，Windows 黑屏 | Rust 后端 `alacritty_terminal` 维护 grid + 前端 React 渲染网格，跨平台一致 |
| 前端 xterm.js 依赖 | 5 个 npm 包 | 仅保留 `@xterm/headless`（`VirtualTerminal` 用），其他全部移除 |
| Windows 黑屏 | 反复出现 | 根治（无 WebGL，纯 DOM） |
| 命令普通输出 | 静态 CommandBlock + Live LiveTerminal 两套 | 普通命令仍走 CommandBlock；alt-screen TUI 走 GridTerminal —— 两条路径但都不依赖 xterm.js 渲染层 |

### 1.3 非目标
- 不取代普通命令的 CommandBlock（那个 ansi-to-react 文本流方案完全够用且更合适）
- 不重写 PTY 解析器（继续用 `golish-pty` 的 OSC + ANSI 解析；GridTerminal 只在 alt-screen 模式启用）
- 不改 stdin 协议（依旧 `pty_write`）

---

## 2. 架构调查（取舍）

### 2.1 候选

| 方案 | 模型 | 优点 | 劣点 | 体积 |
|---|---|---|---|---|
| **A · `alacritty_terminal`** | Rust 完整虚拟终端，alt-screen / scrollback / 字符属性全管 | 工业级、Warp/Alacritty/iTerm 同款 | API 较重、需要适配自家 grid schema | ~80KB stripped |
| **B · `vt100` / `vt100-ctt`** | Rust 轻量 VT100 解析 + 屏幕状态 | 体积小、API 简洁 | scrollback 实现弱、属性集不全 | ~30KB |
| **C · `@xterm/headless` + serialize** | JS headless 终端 + serialize 出 ANSI 文本 | 项目已经有 VirtualTerminal 雏形 | 仍跑 JS、跨进程开销大、不解决前端依赖 | — |
| **D · 自写 ANSI 解析 + grid 状态机** | 从 0 实现 ECMA-48 / DEC private modes | 0 依赖 | 工作量爆炸、bug 风险高 | — |

### 2.2 决策：方案 A（`alacritty_terminal 0.26`）

理由：
1. **完整性最强**：alt-screen / scrollback / SGR / mouse / bracketed paste / window title 一次性都拿到
2. **生态成熟**：Alacritty / Wezterm / Warp 都基于它
3. **工程上不重**：项目已经是 Rust 后端，加 crate 就是一行 Cargo.toml
4. **属性集匹配前端 CSS 渲染**：fg/bg/bold/italic/underline/inverse —— 都是 CSS 变量能直接表达的

兜底：若发现 `alacritty_terminal` 0.26 的 grid API 太重，降级到 `vt100-ctt 0.17`（与原作者 fork，scrollback 已修补），grid schema 不变。

---

## 3. 数据模型与协议

### 3.1 单元格（Cell）

```rust
// backend/crates/golish-pty/src/grid/cell.rs
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Cell {
    pub ch: char,           // 单字符（CJK 宽字符额外占 1 个 Cell 占位）
    pub fg: ColorCode,      // 0..=255 (256 色) 或 -1 = default
    pub bg: ColorCode,
    pub attrs: CellAttrs,   // bitflags: BOLD | ITALIC | UNDERLINE | INVERSE | DIM | BLINK | STRIKETHROUGH
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Cursor {
    pub x: u16, pub y: u16,
    pub visible: bool,
    pub style: CursorStyle, // Block | Underline | Bar
}
```

### 3.2 增量协议

```jsonc
// Event channel: "terminal_grid_update"
{
  "session_id": "uuid",
  "rev": 12345,             // monotonic counter, 单调递增
  "cols": 80,
  "rows": 24,
  // Dirty rows only — y 是 0-indexed (top-of-grid)
  "dirty_rows": [
    { "y": 0, "cells": [{"ch":" ","fg":-1,"bg":-1,"attrs":0}, ...] },
    { "y": 12, "cells": [...] }
  ],
  "cursor": {"x": 5, "y": 12, "visible": true, "style": "Block"},
  "alt_screen": true,        // 当前是否处于 alt-screen
  "viewport_top": 0,          // scrollback 滚动位置（仅 alt_screen=false 时有意义）
  "scrollback_lines_added": 0 // 此 rev 新增多少行到 scrollback
}
```

#### Full snapshot 兜底

每 60 帧（约 2 秒）或前端订阅时发一次「全量帧」（所有 rows 都在 `dirty_rows`）。`rev` 仍递增。前端收到 `rev` 不连续时主动请求 full snapshot：

```jsonc
// Command (frontend → backend)
{ "op": "request_grid_snapshot", "session_id": "uuid", "from_rev": 12343 }
```

### 3.3 速率与背压

- 后端 grid 状态变化时入队 dirty row 集合
- 60 ms 窗口 coalesce（一帧～16ms 偏激，60ms 更合适：CPU 友好 + 视觉无感）
- 单次 update 序列化后限制 ≤256KB，超过则拆 N 个 update 分发
- 前端订阅时立刻 push 一次 full snapshot

---

## 4. 后端实现

### 4.1 新模块结构

```
backend/crates/golish-pty/src/grid/
├── mod.rs              # 公共 API: GridManager
├── cell.rs             # Cell / Cursor / Color / CellAttrs
├── terminal.rs         # alacritty_terminal 封装
├── snapshot.rs         # Diff / full snapshot 序列化
└── tests.rs            # 单元测试
```

### 4.2 `GridManager`

```rust
pub struct GridManager {
    // sessionId → terminal 实例
    sessions: Mutex<HashMap<String, GridSession>>,
}

struct GridSession {
    terminal: alacritty_terminal::Term<NoOp>,  // alacritty_terminal::event::EventListener
    parser: ansi::Processor,
    last_rev: u64,
    last_dirty_rows: BTreeSet<u16>,
    last_emit_at: Instant,
}
```

### 4.3 接入点

在 `manager/session_create.rs` 的 emitter 线程里，当当前 session 处于 alt-screen 状态时，额外把 raw bytes 喂给 GridManager：

```rust
if session.alt_screen.load(Ordering::Relaxed) {
    grid_manager.write(&session_id, &raw_bytes);
    grid_manager.flush_if_due(&session_id, &emitter);  // 60ms coalesce
}
```

alt-screen 进入/退出由现有 OSC ?1049h/l 检测分发：进入时初始化 GridSession，退出时 drop 释放内存。

### 4.4 键盘 → ANSI

GridTerminal 不直接负责键盘 → 字节转换（前端做）；后端只接 `pty_write(session_id, bytes)`。但需要一个新命令：

```rust
// 调整 grid 尺寸（前端 ResizeObserver 触发）
#[tauri::command]
async fn pty_resize_grid(session_id: String, rows: u16, cols: u16) -> Result<(), String>;
```

这复用现有的 `ptyResize`，无新接口。

---

## 5. 前端实现

### 5.1 模块结构

```
frontend/components/GridTerminal/
├── GridTerminal.tsx              # 主组件
├── GridRow.tsx                   # 单行 React 组件（memo'd by row content hash）
├── useGridState.ts               # rev 同步 + dirty merge
├── keymap.ts                     # 键盘事件 → ANSI 序列
└── GridTerminal.test.tsx
```

### 5.2 渲染策略

```tsx
function GridTerminal({ sessionId }: { sessionId: string }) {
  const { grid, cursor, rows, cols } = useGridState(sessionId);
  const containerRef = useRef<HTMLDivElement>(null);
  useGridKeyboard(sessionId, containerRef);
  useGridResize(sessionId, containerRef);

  return (
    <div ref={containerRef} className="font-mono leading-[1.2] outline-none" tabIndex={0}>
      {grid.map((row, y) => (
        <GridRow key={y} cells={row} cursorX={cursor.y === y ? cursor.x : -1} />
      ))}
    </div>
  );
}
```

`GridRow` 用 `<span>` per cell 渲染，class 由 `attrs` + `fg` + `bg` 拼出。Memo 比较：row 哈希（cells 内容 + cursor 是否落在本行）。

### 5.3 性能预算

- 80×24 = 1920 个 span
- 单 row 重渲染 ~0.5ms（React 19 + Compiler）
- 60ms coalesce 窗口 = 16 fps 实际渲染（足以应付 vim 滚动）
- 长滚动（cargo build 大输出）走 viewport / virtualized rows，超出可视区不挂载

### 5.4 键盘 → ANSI 序列表（节选）

| 键 | 输出 |
|---|---|
| ArrowUp | `\x1b[A` (normal) / `\x1bOA` (application) |
| Enter | `\r` |
| Backspace | `\x7f` |
| Tab | `\t` |
| Ctrl-C | `\x03` |
| Ctrl-W | `\x17` |
| F1–F12 | `\x1bOP` … |
| 普通字符 | UTF-8 bytes |
| Paste | bracketed `\x1b[200~...\x1b[201~` 包裹 |

`keymap.ts` 是一张完整对照表，参考 alacritty/xterm.js 默认绑定，便于 vim / tmux 等成熟 TUI 直接可用。

---

## 6. 替换 `Terminal.tsx`

### 6.1 步骤

1. 新增 `GridTerminal` 组件并 mount 在 `Terminal.tsx` 同一位置作为 dual-render：
   - feature flag `terminal.use_grid_renderer`（默认 `false`）
   - flag true 时渲染 GridTerminal，否则保留 xterm.js（灰度过渡）
2. 内部 dogfood 验证 vim / htop / less / nano / nvim / tmux 5+ 场景
3. 翻 flag 默认 `true`
4. 删除 `Terminal.tsx` 旧路径 + 移除 `@xterm/xterm` / `@xterm/addon-*` 依赖
5. `VirtualTerminal.ts`（headless）继续保留 —— 它只用于 ANSI 文本预处理，不渲染

### 6.2 待删 npm 依赖

```
@xterm/xterm
@xterm/addon-fit
@xterm/addon-webgl
@xterm/addon-web-links
@xterm/addon-serialize  # (在 LiveTerminalManager 删除后已不需要；Phase B 一并清理)
```

只保留：
```
@xterm/headless  # VirtualTerminal.ts 用
```

---

## 7. 测试计划

### 7.1 单元

- Rust: `alacritty_terminal` + grid serialize 端到端测试（写入已知 ANSI 序列 → 比较输出 Cell 矩阵）
- Frontend: `useGridState` 增量合并、`keymap` ANSI 序列正确性

### 7.2 集成

- Playwright e2e：
  - `vim foo.txt` → 输入 i → 输入 hello → Esc → :wq → 文件正确写入
  - `htop` → 按 F10 退出
  - `less LICENSE` → space → q 退出
  - `tmux new` → 在 tmux 内开 split → exit
  - 中文/CJK 宽字符正确渲染（`echo 你好 hello 世界`）

### 7.3 性能

- benchmark：`cargo build` 内一次性输出 50KB ANSI → 测前端帧率
- benchmark：vim 内连续滚动 1000 行 → 测后端 grid 序列化耗时

### 7.4 跨平台

- macOS / Linux / Windows 三平台手动跑前述 e2e 一遍
- Windows 重点验证 vim 不黑屏

---

## 8. 实施里程碑

| Day | 交付 |
|---|---|
| **D1** | `golish-pty/src/grid/` 骨架 + `alacritty_terminal` 集成 + 单元测试 |
| **D2** | grid 序列化 + `terminal_grid_update` 事件 + 60 ms coalesce |
| **D3** | 前端 `GridTerminal` 组件 + `useGridState` + 基础渲染 |
| **D4** | `keymap.ts` 全键位 + 调整尺寸 + 焦点处理 |
| **D5** | dual-render flag 接入 + dogfood vim/htop/less |
| **D6** | 移除 xterm.js 依赖 + e2e 测试 + Windows 平台验证 + 文档更新 |

---

## 9. 风险与回滚

### 9.1 风险

- **alacritty_terminal API 变动**：0.26 是相对新版本；锁定 minor 版本 + 包一层抽象层防止后续破坏
- **CJK 宽字符位置**：alacritty 用 `wcwidth`-based 处理，要验证常见 emoji / 中文标点不错位
- **键盘 IME 输入**：composition events 需要单独处理，否则中文输入断字
- **focus / blur**：Tauri WebView2 焦点切换有时丢失，需要主动 textareaRef.focus()

### 9.2 回滚

- 全程通过 `terminal.use_grid_renderer` flag 控制；任何时刻可关回 xterm.js
- 若发现某场景 GridTerminal 不行，**单 session 级**降级：检测到该 session 在用 vim 之外的更复杂 TUI 时退回 xterm（极少数）
- npm 依赖移除推迟到 D6 才做，方便回滚

---

## 10. 与 Phase A / Phase C 的衔接

- Phase A 已让大多数交互场景不依赖 xterm —— Phase B 是把残留的 TUI 场景也搬走
- Phase C 的「TUI 退出后折叠为一行卡片」与 Phase B 解耦：GridTerminal 退出后由 CommandBlock 接管展示，折叠逻辑写在 CommandBlock 里
- Phase B 落地后 `frontend/styles/xterm-overrides.css` 可以删除

---

## 11. 待办（与本文档脱离的小事）

- `frontend/components/Terminal/RecordingsPanel.tsx` 依赖 xterm.js 重放录制 —— Phase B 完成后改成 GridReplayer 或直接删除该功能
- `e2e/terminal-portal-architecture.spec.ts` 需要重写为 GridTerminal portal 架构
- `docs/architecture.md` 「终端渲染」一节重写
