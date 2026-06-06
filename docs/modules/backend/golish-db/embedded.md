# golish-db / embedded

> **一句话职责**：嵌入式 PostgreSQL 生命周期——`EmbeddedPg` 用 `pg-embed` 下载/解压/initdb/启动 PG 17，安装 pgvector，含 macOS quarantine 清除 + `pg_ctl` 手动启动兜底。

- **类型**：目录模块（属于 crate [`golish-db`](../golish-db.md)）
- **路径**：`backend/crates/golish-db/src/embedded/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- DB 启动失败、PG 二进制下载/解压/缓存、pgvector 安装、端口占用问题时
- 改嵌入式 PG 生命周期（start/stop）、跨平台二进制处理时
- macOS Gatekeeper 拦截 initdb/pg_ctl、库路径（DYLD/LD_LIBRARY_PATH）问题时

## 职责

`EmbeddedPg` 管理随 app 生命周期的嵌入式 PostgreSQL：首次运行下载 ~30MB PG 17 二进制（缓存复用），initdb、启动、建库、安装 pgvector。pg-embed 的 `start_db()` 在 macOS 偶发失败时回退到手动 `pg_ctl`（显式库路径 + 日志文件）。跨平台细节委托给 `golish-platform::postgres`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `EmbeddedPg`（`start(DbConfig)` / `stop` / `connection_string` / `config`） | 嵌入式 PG 句柄 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `EmbeddedPg` 生命周期 + pgvector 安装 + pg_ctl 兜底 |
| `platform.rs` | `copy_binary` / `find_system_pgvector`（跨平台二进制） |

## 依赖

- `pg-embed`（PG_V17）、`golish_platform::postgres`（fetch tag / quarantine / pgvector 库名）、`tokio`、`anyhow`、`dirs`（缓存目录）

## 注意事项 / 坑

- **macOS**：必须在 `setup()` 前清 quarantine（否则 Gatekeeper 拦截未签名 initdb/pg_ctl）；pgvector 的 `.dylib` 要落 `lib/postgresql/`（$libdir）而非 `lib/`，代码有 relocate 逻辑。
- **pg_ctl 兜底**：pg-embed 在 macOS 不传 DYLD_LIBRARY_PATH 会挂，手动启动显式带库路径 + 日志，改启动逻辑别丢这条回退。
- pgvector 找不到系统库时**降级到应用层向量搜索**（不 fatal）；端口已占用视为已在跑。
- 跨平台分支只能在 `golish-platform`（不变量），别在此写 `cfg(target_os)`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db embedded
```
