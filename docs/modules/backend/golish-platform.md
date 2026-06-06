# golish-platform

> **一句话职责**：跨平台抽象层——全仓库**唯一**允许写 `#[cfg(target_os=…)]` / 硬编码 shell / 库后缀的地方，其它 crate 一律调它而非自己分支。

- **类型**：crate（Layer 1 基础层，与 golish-core 同层）
- **路径**：`backend/crates/golish-platform/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 涉及 OS 差异：shell 调用、进程/端口、路径与目录、可执行权限、打开 URL/文件、包管理器、嵌入式 PostgreSQL、系统代理
- 你正想写 `cfg!(windows)` / `Command::new("sh")` 之类的平台分支时——**停，应该加到这里**

## 职责

集中所有条件编译。设计三原则：① 本 crate 之外禁止 `#[cfg(target_os)]`；② 方法按「调用者要什么」命名（如 `Platform::default_shell`）而非平台名；③ 易 mock（`Platform::current()` 是模块级自由函数的薄 facade）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Platform` / `PlatformKind` / `Arch` | 平台检测与能力入口 |
| `PackageManager` | 包管理器安装提示 |
| 各模块自由函数 | `shell::build_shell_command`、`process::kill_pid`、`fs_perms::set_executable`、`open::open_url`、`postgres::*` 等 |

## 依赖

- **内部**：无（基础层，仅依赖外部 crate）

## 被谁依赖 / 改动影响面

`golish`、`golish-pentest-app`、`golish-core`、`golish-pty`、`golish-db`、`golish-shell-exec`、`golish-pentest`、`golish-mcp`。改 OS 行为会波及 shell/进程/DB 启动等底层。

## 关键文件（均单文件模块，无目录子模块）

| 文件 | 作用 |
|---|---|
| `detect.rs` | `PlatformKind` / `Arch` / `Platform` 检测 |
| `shell.rs` | 默认 shell、构建 shell 命令、`which` |
| `process.rs` | 杀进程、查端口监听、进程组 |
| `paths.rs` | 扩展名常量（EXE/DYLIB）、`dirs::*` 包装 |
| `fs_perms.rs` | 可执行位 set/has |
| `open.rs` | 打开 URL / 在文件管理器中显示 |
| `package_manager.rs` | 包管理器安装提示 |
| `postgres.rs` | 嵌入式 PG / pgvector 平台helper |
| `system_proxy.rs` | 桌面系统代理控制 |

## 注意事项 / 坑

- **铁律**：平台分支只能写在这里。在别处加 `cfg(target_os)` 视为违规。
- 嵌入式 PG 的平台 helper 在此（`postgres.rs`），DB 启动相关问题先看这里。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-platform
```
