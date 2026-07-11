# Legacy GUI 扫描启动授权实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 WhatWeb、Nuclei、feroxbuster 的真实网络 launch 与全部 target-bound landing 绑定同一个 current-owner/exact-origin immutable guard。
**架构：** IPC 先生成 `AuthorizedScanTarget`，runner 在异步准备后和每次 spawn 前复核其 `TargetWriteGuard`。输出通过 guarded repo/短事务落库，caller process override 参数 fail-closed。
**技术栈：** Rust 2021、Tokio process、SQLx/Postgres、`golish-db`、`golish-pentest-domain::canonical_web_origin`、Cargo test/clippy。

## 文件结构

- 创建 `backend/crates/golish-scan-runner/src/authorization.rs`：current-owner/exact-origin 授权与 validator-before-launch 调度器。
- 修改 `backend/crates/golish-scan-runner/src/{whatweb.rs,feroxbuster.rs,helpers.rs,storage.rs,lib.rs}`：guarded runner、失败语义、storage contract 与 exports。
- 修改 `backend/crates/golish-scan-runner/src/nuclei/{runner.rs,poc_match.rs}`：真实 launch、模板配置面与 guarded finding landing。
- 修改 `backend/crates/golish-recon-app/src/scan_runner/mod.rs`、`targets/directory.rs`：IPC preauthorization 与 guarded callback adapter。
- 修改 `backend/crates/golish-db/src/repo/{scoped.rs,fingerprints.rs,audit/pentest.rs}`：guard loader/fingerprint batch/lineage audit transactions。
- 修改 `backend/crates/golish-scan-runner/tests/storage_and_progress.rs`：新的 guard-carrying trait contract。
- 更新对应 `docs/modules/` 卡片与主索引。

## 任务 1：先锁定 pre-network 授权合同

**文件：** `backend/crates/golish-scan-runner/src/authorization.rs`

**步骤：**

1. 写失败测试，构造含一个 open origin 与一个 closed origin 的 `TargetWriteGuard`，断言 foreign project、foreign origin、closed-port origin 全部拒绝。
2. 写 counting fake launcher 测试；validator 返回 owner-drift error 时 launch closure 不执行：

```rust
let calls = Arc::new(AtomicUsize::new(0));
let result = after_successful_validation(
    async { Err("target owner drift") },
    || async { calls.fetch_add(1, Ordering::SeqCst); Ok(()) },
).await;
assert!(result.is_err());
assert_eq!(calls.load(Ordering::SeqCst), 0);
```

3. 实现 `authorize_scan_target`、`authorize_scan_target_from_guard`、`url_has_authorized_origin` 与 `after_successful_validation`。

**验证：**

```bash
cd backend && cargo test -p golish-scan-runner --lib authorization::tests
```

预期：2 tests passed。

**提交：** `git commit -m "fix(scan): bind legacy launches to target guard"`

## 任务 2：把三个 command launch 放到 guard 后面

**文件：** `backend/crates/golish-scan-runner/src/whatweb.rs`、`feroxbuster.rs`、`nuclei/runner.rs`、`helpers.rs`

**步骤：**

1. 把三个 runner 签名改为接受 `&AuthorizedScanTarget`，删除裸 target id/project 参数。
2. tool lookup/args/base-path/wordlist 准备完成后调用 `validate_target_write_guard`，再写 guarded started audit；spawn 用 `after_successful_validation` 包裹第二次 DB guard validation。
3. 每个 child 检查 `ExitStatus` 与 fatal/runtime/network stderr markers；JSONL 非空坏行写 error。
4. Nuclei 参数固定加入：

```rust
"-dr", // no redirects
"-ni", // no Interactsh
"-dut", // signed templates only
```

5. Nuclei 要求 exact template id；WhatWeb/Nuclei 拒绝 caller proxy/extra args，Nuclei 额外拒绝 template path/positive tags；WhatWeb 固定 `--follow-redirect=never --max-redirects=0` 并用 fake launcher 断言两个 flag 各只出现一次。
6. Ferox 用 `url::Url::join` 生成 base URL，并逐项调用 `url_has_authorized_origin`；wordlist canonical path 只能位于两个 allowlisted workspace 位置。

**验证：**

```bash
cd backend && cargo test -p golish-scan-runner --all-targets
```

预期：unit 8/8、api contract 2/2、storage/progress 3/3。

**提交：** `git commit -m "fix(scan): revalidate scope before scanner spawn"`

## 任务 3：guarded business 与 audit landing

**文件：** `backend/crates/golish-db/src/repo/fingerprints.rs`、`audit/pentest.rs`、`backend/crates/golish-scan-runner/src/{whatweb.rs,feroxbuster.rs,nuclei/runner.rs,storage.rs}`、`backend/crates/golish-recon-app/src/targets/directory.rs`

**步骤：**

1. WhatWeb 将 parsed plugins 收集成 `FingerprintWrite`，通过一个 `upsert_batch_guarded` transaction 发布。
2. Nuclei 在一个短事务中执行：

```rust
let mut tx = pool.begin().await?;
lock_target_write_guard(&mut tx, &authorization.guard).await?;
// upsert finding WHERE existing.project_path == EXCLUDED.project_path
// upsert passive_scan_log with the same project conflict predicate
tx.commit().await?;
```

3. `ScanStorage::store_directory_entry` 改为接收 `&TargetWriteGuard`；recon adapter 调 `insert_entry_guarded`。
4. Ferox sensitive finding 在独立短事务锁同 guard；输出 URL foreign origin 不写。
5. 添加 `PentestAudit::{started,completed,failed}_guarded`；parent lookup 同时匹配 parent id、target id、project path。

**验证：**

```bash
cd backend
cargo test -p golish-db --lib repo::audit -- --nocapture
cargo check -p golish-db -p golish-scan-runner -p golish-recon-app
```

预期：DB audit 8/8；三个 crate check 成功。

**提交：** `git commit -m "fix(scan): guard scanner result and audit landing"`

## 任务 4：模块卡与零 warning 收口

**文件：** `docs/modules/backend/golish-recon-app/{scan_runner.md,targets.md}`、`docs/modules/backend/golish-scan-runner.md`、`docs/modules/backend/golish-scan-runner/nuclei.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/INDEX.md`

**步骤：** 写清 current-owner/exact-origin precondition、双 revalidation、caller override denylist、guarded output/audit landing 与失败语义；主索引描述更新为 guarded launch。

**验证：**

```bash
cd backend
cargo clippy -p golish-db -p golish-scan-runner -p golish-recon-app --all-targets -- -D warnings
cd .. && git diff --check
```

预期：clippy 零 warning，`git diff --check` 无输出。

**提交：** `git commit -m "docs(scan): record guarded legacy launch contract"`
