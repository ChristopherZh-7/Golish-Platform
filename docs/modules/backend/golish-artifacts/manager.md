# golish-artifacts / manager

> **一句话职责**：`ArtifactManager`——管理一个会话的文档产物提案：在 `{session}/artifacts/{pending,applied}/` 下创建/列举/预览/丢弃/应用（apply 时写目标文件 + `git add` + 移到 applied），并能基于已应用补丁再生成 README/CLAUDE.md。

- **类型**：目录模块（属于 crate [`golish-artifacts`](../golish-artifacts.md)）
- **路径**：`backend/crates/golish-artifacts/src/manager/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改产物提案的 pending/applied 生命周期、apply（写文件 + git add + 移动）逻辑时
- 改 diff 预览（`preview_artifact`）或基于补丁再生成文档（`regenerate_from_patches`）时

## 职责

`ArtifactManager` 把会话产生的文档更新提案以「pending → applied」两阶段管理：`create_artifact` 落 pending；`apply_artifact` 写目标文件、`git add`、移到 applied；`preview_artifact` 出 diff；`regenerate_from_patches` 基于已应用补丁 + 会话上下文，经 `synthesis`（LLM）或 `generators`（模板回退）重算 README.md / CLAUDE.md。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ArtifactManager::new(session_dir)` | 构造（绑定会话目录） |
| `pending_dir` / `applied_dir` / `ensure_dirs` | 目录解析/创建 |
| `create_artifact` / `list_pending` / `list_applied` / `get_pending` | 创建/列举/取 pending |
| `discard_artifact` / `apply_artifact` / `apply_all_artifacts` | 丢弃 / 应用（含 git add）/ 批量应用 |
| `preview_artifact` | 对目标文件出 diff |
| `regenerate_from_patches(_with_config)` | L2→L3：基于补丁再生成 README/CLAUDE.md |
| `ArtifactFile` / `ArtifactMeta`（来自 `types`） | 产物文件 + 元数据 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `ArtifactManager` 全部方法 |
| `types.rs` | `ArtifactFile` / `ArtifactMeta` |
| `diff.rs` | `generate_simple_diff` / `continue_or_error`（`pub(crate)`） |

## 依赖

- crate 内 `generators`（模板回退）/ `synthesis`（LLM 合成）；`tokio::fs`、`git`（`tokio::process`）、`anyhow`

## 注意事项 / 坑

- `apply_artifact` 会真写目标文件并 `git add`——属对仓库可见副作用；`apply_all` 失败会 bail 并报已应用数（部分应用风险）。
- 再生成优先 LLM 合成，失败按 `uses_llm()` 回退模板；纯模板失败才报错（`continue_or_error`）。
- 本 crate 当前**未集成**进主流程（见 crate 卡）；改它先确认调用方。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-artifacts manager
```
