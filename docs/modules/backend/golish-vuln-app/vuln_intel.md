# golish-vuln-app / vuln_intel

> **一句话职责**：vuln-intel 操作的 Tauri 命令薄包装——纯逻辑（feed 摄取、NVD/CISA/RSS 抓取、GitHub PoC 搜索、Nuclei 模板发现）在 `golish-vuln-intel` crate，这里把 `DbState` + `SettingsManager` 适配到库 API。

- **类型**：目录模块（属于 crate [`golish-vuln-app`](../golish-vuln-app.md)）
- **路径**：`backend/crates/golish-vuln-app/src/vuln_intel/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 vuln-intel 的 Tauri 命令（feed CRUD、摄取、搜索、目标匹配、PoC/Nuclei 富化）时

## 职责

vuln-intel 命令面：thin `#[tauri::command]` 把 `golish-vuln-intel` 库 API 接到窄 `DbState`（+ `golish_settings::SettingsManager`）。re-export 库的 wire 类型。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands::*`（Tauri 命令） | feed/摄取/搜索/匹配/富化 |
| re-export `GithubPocResult` / `VulnEntry` / `VulnFeed`（自 golish-vuln-intel） | wire 类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export + 命令 |
| `commands/` | Tauri 命令包装 |

## 依赖

- crate 内 app-core（`DbState`）；`golish-vuln-intel`（纯逻辑）、`golish-settings`

## 注意事项 / 坑

- 纯逻辑在 `golish-vuln-intel`（无 Tauri）；本模块只适配，别把抓取/搜索逻辑搬进来。
- 远程摄取/富化发 HTTP——别放进 DB 事务（I9）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-vuln-app vuln_intel
```
