# golish-vuln-intel

> **一句话职责**：漏洞情报引擎——vuln feed 摄取（NVD / CISA KEV / RSS）、GitHub PoC 搜索、Nuclei 模板发现/导入；无 Tauri 依赖。

- **类型**：crate（Layer 3 领域）
- **路径**：`backend/crates/golish-vuln-intel/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 vuln feed 抓取/合并/富化、GitHub PoC 搜索、Nuclei 模板搜索/批量导入时
- CVE/CVSS、KEV 相关时

## 职责

owns 全部漏洞情报摄取。应用层用薄命令包装它（从 settings 建 `reqwest::Client`、传入 `VulnIntelStore`，通常是 `PgVulnIntelStore`）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `fetch_cisa_kev` / `fetch_nvd` / `fetch_rss` / `merge_and_enrich` / `enrich_missing_cvss` | feed 抓取/富化 |
| `search_github_poc` / `GithubPocResult` / `build_github_client` | GitHub PoC |
| `search_nuclei_templates` / `batch_search_nuclei_templates` / `discover_all_nuclei` | Nuclei 模板 |
| `VulnIntelStore`(trait) / `PgVulnIntelStore` | 存储 |
| `VulnFeed` / `VulnEntry` / `EntryRow` / `FeedRow` | 类型（部分 re-export 自 domain） |

## 依赖

- **内部**：`golish-core`、`golish-db`、`golish-vuln-intel-domain`

## 被谁依赖 / 改动影响面

`golish`、`golish-app-core`、`golish-vuln-app`。

## 关键文件（无目录子模块）

`fetch.rs`、`github_poc.rs`、`github_client.rs`、`nuclei_search.rs`、`nuclei_discover.rs`、`pg_adapter.rs`、`store_trait.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- 无 Tauri 依赖：HTTP client 与 store 由应用层注入，别在此硬编码。
- 相关：`docs/superpowers/plans/2026-06-05-vuln-triage-technique-matrix.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-vuln-intel
```
