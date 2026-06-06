# golish-artifacts

> **一句话职责**：项目产物（L3）——基于会话活动自动维护项目文档（README.md / CLAUDE.md），生成「待用户审阅」的更新提案。

- **类型**：crate（Layer 3）
- **路径**：`backend/crates/golish-artifacts/`
- **状态**：✅ 已写卡（⚠️ 实现完成但**尚未集成**）

---

## 何时该读这张卡（给 AI 的触发提示）

- 自动维护项目文档（README/CLAUDE）、生成文档更新提案时
- 与 sidecar 的 artifacts 流程联动时

## 职责

根据会话活动综合生成项目文档更新提案，交用户 review 后应用。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `generators::*` | 各类产物生成器 |
| `manager::*` | 产物管理（提案/应用） |
| `prompts::*` | 生成用 prompt |
| `synthesis::*` | LLM 综合 |

## 依赖

- **内部**：`golish-settings`

## 被谁依赖 / 改动影响面

`golish`、`golish-sidecar`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `manager/` | 产物提案/应用管理 | [→](golish-artifacts/manager.md) |

## 关键文件

`generators.rs`、`prompts.rs`、`synthesis.rs`。

## 注意事项 / 坑

- lib 顶部 `#![allow(dead_code)]` 注明「artifact 系统已实现但尚未集成」——动它前确认是否仍未接入主流程。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-artifacts
```
