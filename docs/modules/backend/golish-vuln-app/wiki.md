# golish-vuln-app / wiki

> **一句话职责**：wiki / 漏洞知识库命令——按特性面拆 5 块：pages（文件系统页 CRUD + YAML frontmatter）、search（grep + PG FTS）、vuln_links（漏洞↔wiki/PoC/扫描历史）、kb_research（per-CVE 研究日志）、dashboard（Karpathy 式概览）。

- **类型**：目录模块（属于 crate [`golish-vuln-app`](../golish-vuln-app.md)）
- **路径**：`backend/crates/golish-vuln-app/src/wiki/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 wiki 页 CRUD（`wiki_*`）、搜索（grep / PG FTS）、漏洞链接（`vuln_link_*`/`vuln_poc_*`）、研究日志、dashboard 时

## 职责

wiki/KB 域命令面（从单个 ~1300 行文件按特性拆）。`pages` 文件系统页 CRUD（`wiki_init`/`list`/`read`/`write`/`delete`/`rename`/`create_dir`/`create_cve`/`reindex` + YAML frontmatter 解析）；`search` grep + PG FTS（`wiki_search`/`wiki_search_db`/`wiki_stats`）；`vuln_links` 漏洞↔wiki/PoC/扫描历史 CRUD；`kb_research` per-CVE 研究日志；`dashboard` 分组/changelog/backlinks/孤儿页/统计。

## 公开接口

| 符号 | 说明 |
|---|---|
| `pages`（`wiki_*` 命令） | 文件系统页 CRUD |
| `search`（`wiki_search`/`wiki_search_db`/`wiki_stats`） | grep + PG FTS |
| `vuln_links`（`vuln_link_*`/`vuln_poc_*`） | 漏洞链接 |
| `kb_research` / `dashboard` | 研究日志 / 概览 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 5 子模块声明 |
| `pages.rs` / `search.rs` / `vuln_links.rs` / `kb_research.rs` / `dashboard.rs` | 各特性面 |

## 依赖

- crate 内 app-core；`golish-core::paths`（wiki_dir）、`tokio::fs`、`golish-db`（FTS/链接）

## 注意事项 / 坑

- 命令名**不可变**：经 `commands_facade::wiki` glob 暴露给 `generate_handler!`，每个命令必须停在本模块路径可达处。
- pages 是文件系统 CRUD（YAML frontmatter + markdown），`search_db`/链接才走 PG——别混。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-vuln-app wiki
```
