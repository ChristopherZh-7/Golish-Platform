# golish-recon-app

> **一句话职责**：**recon 服务**的 per-domain Tauri command crate（crate-per-service M2）——targets、asset intel、organizations、scan runner/queue、sensitive scan、intel providers、integrations 浏览器 capture、agent_tools、wordlists、custom_rules。

- **类型**：crate（Layer 5+ · per-domain app）
- **路径**：`backend/crates/golish-recon-app/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改侦察/攻击面域 Tauri command（targets、organizations、资产情报、扫描队列/执行）时
- 改 intel providers 适配、integrations 浏览器数据 capture、敏感信息扫描、wordlists、custom_rules 时
- 涉及 `targets` / `target_assets` / `organizations` / `directory_entries` / `custom_rules` / `sensitive_scan` 表时

## 职责

侦察 / 攻击面域的命令面（从 `golish/src/tools/` 在 M2 抽出）。owns targets / 资产面相关表。命令取窄 `golish_app_core::DbState`（+ `golish-scan-runner`/`golish-intel-providers`/`golish-integrations` 等域 crate），不取巨石 `golish::AppState`。

## 公开接口 / 关键类型

| 模块 | 说明 |
|---|---|
| `targets` / `organizations` / `organization_recon` | 目标 / 组织 / 组织级侦察命令 |
| `asset_intel`（`runtime/` `service/`） | 资产情报采集 |
| `scan_runner` / `scan_queue` | 扫描执行 + 队列命令 |
| `intel_providers` / `integrations`（`capture/`） | 情报源适配 / 浏览器数据 capture |
| `sensitive_scan` / `wordlists` / `custom_rules` / `agent_tools` | 敏感扫描 / 字典 / 自定义规则 / agent 工具检查 |

## 依赖

- **内部**：`golish-app-core`、`golish-db`、`golish-core`、`golish-settings`、`golish-pentest`、`golish-scan-runner`、`golish-intel-providers`、`golish-integrations`、`golish-projects`
- **外部**：`tauri`、`ts-rs`、`sqlx`、`reqwest`、`zip`、`url`、`dirs`

## 被谁依赖 / 改动影响面

仅 `golish`（通过 `commands_facade` 聚合）。是 recon 命令面的唯一宿主；其 `ports::recon` 被 pentest-app 跨服务消费。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `targets/` | 目标 & directory-entry 数据层 | [→](golish-recon-app/targets.md) |
| `asset_intel/` | 资产情报服务（候选/ENScan） | [→](golish-recon-app/asset_intel.md) |
| `integrations/` | 凭据 IPC + 浏览器 capture | [→](golish-recon-app/integrations.md) |
| `organizations/` | 组织（甲方资产情报库） | [→](golish-recon-app/organizations.md) |
| `organization_recon/` | 组织级 recon 编排 | [→](golish-recon-app/organization_recon.md) |
| `scan_runner/` | 扫描命令包装 | [→](golish-recon-app/scan_runner.md) |
| `agent_tools/` | harness target_intel AI 工具 | [→](golish-recon-app/agent_tools.md) |

## 关键文件

`intel_providers.rs`、`scan_queue.rs`、`sensitive_scan.rs`、`wordlists.rs`、`custom_rules.rs`（单文件模块）。

## 注意事项 / 坑

- **不变量 I2**：targets/organizations 等 CRUD 验资源所有权（IDOR）。
- **不变量 I4/I5**：命令命名 `<domain>_<verb>_<object>`；DTO 走 ts-rs。
- `integrations` 浏览器 capture 需要全 Tauri webview surface，且解析 per-OS 浏览器数据目录（`dirs`）——跨平台分支注意走 `golish-platform`。
- `asset_intel` 输出写到 `golish-projects` 文件存储（L1），非直接 DB。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app
```
