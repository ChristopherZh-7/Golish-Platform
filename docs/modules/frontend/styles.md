# frontend / styles

> **一句话职责**：全局/特化 CSS——`ansi-colors.css`（终端 ANSI 调色板）、`grid-terminal.css`（GridTerminal 渲染样式）、`xterm-overrides.css`（xterm.js 覆盖）；通用样式走 Tailwind 4（`index.css`）。

- **类型**：前端子系统
- **路径**：`frontend/styles/`（3 CSS）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改终端 ANSI 配色、GridTerminal 渲染样式、xterm.js 样式覆盖时
- 找全局样式入口时（Tailwind 在 `frontend/index.css`，主题 token 在 `lib/theme`）

## 职责

放无法或不便用 Tailwind utility 表达的特化 CSS（主要是终端渲染相关）。通用样式用 Tailwind 4（`index.css`），主题色板/token 逻辑在 `lib/theme`。

## 关键文件

| 文件 | 作用 |
|---|---|
| `ansi-colors.css` | 终端 ANSI 16/256 色板 |
| `grid-terminal.css` | GridTerminal（alacritty 网格渲染）样式 |
| `xterm-overrides.css` | xterm.js 默认样式覆盖 |

## 依赖

- 被 `components`（GridTerminal/Ansi/终端组件）引用；Tailwind 4 在 `index.css`

## 注意事项 / 坑

- 优先 Tailwind utility；这里只放终端/xterm 这类必须的全局/特化 CSS，别把可 Tailwind 化的样式堆这。
- ANSI/grid 配色与 `lib/theme`（主题 token）+ 后端 grid 渲染（`golish-pty/grid`）相关；改配色注意三处一致。

## 测试入口

```bash
just check-fe   # biome（CSS 不进 vitest）
just dev-fe     # 视觉验证终端渲染
```
