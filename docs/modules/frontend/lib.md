# frontend / lib

> **一句话职责**：前端的非-UI 基础设施层——`api`（统一 Tauri 客户端，唯一允许 `invoke` 处）、`generated`（ts-rs 后端类型，**禁手改**）、`events`、`ai`、`pentest`、`models`、`settings`、`theme`、`timeline`、`terminal`、`i18n`、`ui-state` 等。

- **类型**：前端子系统
- **路径**：`frontend/lib/`（~260 ts/tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 调后端命令（必须经 `lib/api/<domain>.ts`，**禁裸 `invoke()`**）时
- 用跨 IPC 类型（从 `lib/generated/` import，ts-rs 生成）时
- 改前端 AI 逻辑、pentest 视图模型、主题、i18n、timeline、终端逻辑时

## 职责

承载前端所有非-UI 逻辑与后端边界。`api/` 是唯一允许 `invoke` 的地方（37 个域 wrapper + `client.ts` + `error-codes.ts`）；`generated/` 是 ts-rs 从后端生成的 wire 类型（62 文件，手写文件**禁改**）；其余子目录是领域逻辑/工具。

## 关键子目录

| 子目录 | 说明 |
|---|---|
| `api/` | 统一 Tauri 客户端：`api.<domain>.<verb>`，37 域 wrapper + `client.ts`（持 `invoke`）+ `error-codes.ts` |
| `generated/` | **ts-rs 生成**的后端 wire 类型（62 文件，禁手改） |
| `events/` / `ai/` | AI 事件类型 / 前端 AI 逻辑 |
| `pentest/` / `target-panel/` / `timeline/` | pentest 视图模型 / 目标面板 / 时间线 |
| `models/` / `settings/` / `theme/` / `i18n/` / `terminal/` / `ui-state/` / `serde_json/` | 模型 / 设置 / 主题 / 国际化 / 终端 / UI 状态 / JSON 工具 |

## 依赖

- `@tauri-apps/api`（仅 `lib/api/client.ts`）；被 `components`/`hooks`/`store`/`services` 广泛消费

## 注意事项 / 坑

- **不变量 I5**：跨 IPC 类型从 `lib/generated/` import（ts-rs 生成），**不要手写第二份**；`generated/` 下手写文件禁改（AGENTS.md §2.8）。
- **不变量（AGENTS.md §2.3）**：组件调后端走 `lib/api/<domain>.ts`，**禁裸 `invoke()`**；`invoke` 只在 `api/client.ts`。
- **不变量 I1**：错误按 `error-codes.ts` 的 `code` 翻译，不靠 HTTP status 做业务判断。
- 加新后端域：加 `lib/api/<domain>.ts` wrapper + 在 `api/index.ts` 注册。

## 测试入口

```bash
just check-fe   # biome + typecheck（含 ts-rs 绑定漂移检查）
just test-fe    # vitest
```
