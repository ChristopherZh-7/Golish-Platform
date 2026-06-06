# golish-recon-app / integrations

> **一句话职责**：integrations IPC facade——把 `golish-integrations` 的 `SchemaResolver`/`StorageBackend`/`Tester` 桥成 Tauri 命令（list_schemas / get / set / clear / test），并含浏览器凭据 capture 引擎。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/integrations/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改外部服务凭据管理的 Tauri 命令（schema 列举/读写/清除/测试）时
- 改浏览器凭据 capture（`capture/` + `capture_commands`）时

## 职责

把 `golish-integrations` 的 schema 驱动凭据管理暴露给前端：`integrations_list_schemas`（列 schema 供前端渲染表单）、`integrations_get`（读 group 字段，secret 仅 `has_value`）、`integrations_set`/`integrations_clear`（写/清，经 schema 声明的 backend）、`integrations_test`（跑声明的连通测试）。`capture` 是浏览器数据捕获引擎。设计见 `docs/design/2026-05-21-integrations.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `integrations_list_schemas` / `integrations_get` / `integrations_set` / `integrations_clear` / `integrations_test`（commands） | 凭据管理命令 |
| `capture` / `capture_commands` | 浏览器凭据捕获 |
| `state` | integrations 运行时状态 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `commands.rs` | 5 个凭据管理命令 |
| `capture/` / `capture_commands.rs` | 浏览器捕获引擎 + 命令 |
| `state.rs` | 运行时状态 |

## 依赖

- crate 内；`golish-integrations`（`SchemaResolver`/`StorageBackend`/`Tester`）、`tauri`（webview capture）、`dirs`（浏览器数据目录）

## 注意事项 / 坑

- secret 字段读时只回 `has_value=true` + `value=None`（不回明文）；别改成回明文。
- 浏览器 capture 需全 Tauri webview surface + per-OS 浏览器数据目录（经 `dirs`/`golish-platform`）；跨平台分支别在此写死。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app integrations
```
