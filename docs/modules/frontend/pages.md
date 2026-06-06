# frontend / pages

> **一句话职责**：独立页面——目前仅 `ComponentTestbed.tsx`（组件测试床/预览页）；主应用 shell 在 `App/` + `App.tsx`，不在此。

- **类型**：前端子系统
- **路径**：`frontend/pages/`（1 tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改独立页面（如组件测试床）时
- 找主应用入口时（注意：主 shell 在 `App.tsx`/`App/`，不在 `pages/`）

## 职责

放不属于主应用 shell 的独立页面。当前只有 `ComponentTestbed`——用于隔离预览/调试单个组件的测试床页。

## 关键文件

| 文件 | 作用 |
|---|---|
| `ComponentTestbed.tsx` | 组件测试床 / 预览页 |

## 依赖

- `react`；按需 import `components`

## 注意事项 / 坑

- **主应用入口不在这**：GUI shell 是 `frontend/App.tsx` + `frontend/App/`，`main.tsx` 是 Vite 入口；`pages/` 仅放独立辅助页。
- 目录很小（1 文件）；新增独立页才放这。

## 测试入口

```bash
just check-fe
just test-fe
```
