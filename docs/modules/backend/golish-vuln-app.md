# golish-vuln-app

> **一句话职责**：**vuln-intel 服务**的 per-domain Tauri command crate（crate-per-service M1 首叶）——漏洞情报 feed CRUD、NVD/CISA/RSS 摄取、本地+远程搜索、目标匹配、per-CVE GitHub PoC + Nuclei 富化，以及文件系统 wiki 页面 CRUD。

- **类型**：crate（Layer 5+ · per-domain app · DAG 叶子）
- **路径**：`backend/crates/golish-vuln-app/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改漏洞情报域 Tauri command（feed、搜索、目标匹配、PoC/Nuclei 富化）时
- 改 wiki 页面文件系统 CRUD（`wiki_dir` 下 markdown 页）时

## 职责

漏洞情报域的命令面（从 `golish/src/tools/vuln_intel/` 抽出，是 crate-per-service 拆分的首个叶子，out-degree 0）。命令取窄 `golish_app_core::DbState`（+ `golish-settings` 的 `SettingsManager`），不取巨石 `golish::AppState`。

## 公开接口 / 关键类型

| 模块 | 说明 |
|---|---|
| `vuln_intel`（`commands/`） | feed CRUD、摄取、本地/远程搜索、目标匹配、PoC/Nuclei 富化命令 |
| `wiki`（`pages/`） | 文件系统支持的 wiki 页面 CRUD（`golish_core::paths::wiki_dir`） |

## 依赖

- **内部**：`golish-app-core`、`golish-vuln-intel`、`golish-db`、`golish-settings`、`golish-core`
- **外部**：`tauri`、`sqlx`、`reqwest`、`tokio`（`tokio::fs` 异步文件 I/O）、`url`

## 被谁依赖 / 改动影响面

仅 `golish`（通过 `commands_facade::vuln_intel` 聚合）。是 vuln 命令面的唯一宿主，本身不依赖任何 sibling app crate（DAG 叶子）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `vuln_intel/` | 漏洞情报命令面（thin over lib） | [→](golish-vuln-app/vuln_intel.md) |
| `wiki/` | wiki/KB 命令（pages/search/links/research/dashboard） | [→](golish-vuln-app/wiki.md) |

## 关键文件

`lib.rs`（仅 `vuln_intel` + `wiki` 两个子模块声明）。

## 注意事项 / 坑

- **不变量 I2**：feed/匹配等 CRUD 验资源所有权（IDOR）。
- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`。
- `wiki` 是文件系统 CRUD（M1b 从别处搬入），走 `golish_core::paths::wiki_dir` 解析磁盘根，用 `tokio::fs`，**不是** DB 表。
- 远程搜索/富化发 HTTP（`reqwest`），注意不要放进 DB 事务（I9）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-vuln-app
```
