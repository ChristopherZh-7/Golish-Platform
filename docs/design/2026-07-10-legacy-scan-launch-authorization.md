# Legacy GUI 扫描启动授权与 guarded landing 设计

## 背景

`scan_whatweb`、`scan_nuclei_targeted`、`scan_feroxbuster` 原先把 IPC caller 提供的 `target_id`、`project_path` 与 `target_url` 直接交给进程 runner。指纹→PoC 匹配即使读取了 current-owner fingerprint，也不能约束随后真正的网络启动；caller 可以用任意 target id 对任意 URL 发请求，target 在异步准备期间移 scope/org/workspace 后也仍可能被扫描。三个 runner 的结果写入同样只使用 mutable id/project 参数。

## 安全目标

1. 任何 tool lookup、参数准备或网络进程启动前，IPC 必须捕获一个 current `scope=in`、non-null project 的 `TargetWriteGuard`。
2. caller `project_path` 必须与 guard 中 DB 原值精确相等。
3. `target_url` 的 exact Web Origin 只能来自 target `name` / `value`，或 `state=open` 的 `ports[].url`。
4. 异步准备结束、guarded audit start 前复核 guard；每次 command spawn 紧前再次复核。复核失败时 command 调用次数为 0。
5. 所有 target-bound business row 与 scan timeline 使用同一个 launch guard，在各自短事务内 `SELECT ... FOR UPDATE` 后写入。
6. child output 的 URL 必须仍属于 launch exact origin；非零退出、exit 0 runtime/network failure 或畸形 JSONL 不能被表示为 clean success/empty。

## 架构

`golish-scan-runner::authorization` 提供唯一入口：

```rust
pub async fn authorize_scan_target(
    pool: &PgPool,
    target_id: Uuid,
    caller_project_path: Option<&str>,
    requested_url: &str,
) -> ScanRunnerResult<AuthorizedScanTarget>;
```

`AuthorizedScanTarget` 持有原始 `TargetWriteGuard`、请求 URL 与 canonical `scheme://host:effective_port`。三个 public runner 不再接受裸 `target_id/project_path`。

```text
IPC caller
  -> load current TargetWriteGuard
  -> exact project + exact origin authorize
  -> async local preparation
  -> validate guard
  -> guarded audit started
  -> validate guard
  -> spawn child process
  -> validate output origin/process semantics
  -> guarded business landing
  -> guarded completed/failed audit
```

DB 写路径：

- WhatWeb：构造 `FingerprintWrite[]`，一次 `fingerprints::upsert_batch_guarded`。
- Nuclei：每个 hit 在同一事务内锁 guard，并原子 upsert `findings` + `passive_scan_logs`；旧 project 的同 id conflict 返回 0 row 并回滚。
- Ferox：directory entry 经 `ScanStorage` 传 guard 到 `insert_entry_guarded`；敏感 finding 单独 guarded transaction。
- Audit：`PentestAudit::{started,completed,failed}_guarded` 同事务锁 guard、校验 parent lineage owner/project 并写 audit row。

## Caller 配置面

- WhatWeb：保留 aggression/plugins/user-agent；拒绝 caller proxy 与 `extra_args`，固定 `--follow-redirect=never --max-redirects=0`，在发请求层阻止 30x 跨 exact origin。
- Nuclei：要求 1..=512 个 exact template id；拒绝 path/wildcard id、positive tags、template path、proxy 与 `extra_args`；固定 `-dr -ni -dut`。
- Ferox：absolute 与 network-path base URL 必须保持 exact origin；自定义 wordlist 只允许 canonical `workspace/1.txt` 或 `workspace/.golish/wordlists/**` regular file，symlink/traversal escape 拒绝。

## 失败语义

- guard/owner drift：返回 error，0 spawn；已写的旧 project 历史行不迁移。
- child non-zero 或 fatal stderr：不解析/落业务 truth。
- child exit 0 但 stderr 出现 request timeout、DNS/network failure、runtime exception：失败。
- JSON/JSONL malformed：结果 `success=false`，不得变成“检查为空”。
- 输出 foreign origin：丢弃该 item 并记录 error；不绑定到 launch target。

## 验证

```bash
cd backend
cargo test -p golish-scan-runner --all-targets
cargo test -p golish-db --lib repo::audit -- --nocapture
cargo check -p golish-db -p golish-scan-runner -p golish-recon-app
cargo clippy -p golish-db -p golish-scan-runner -p golish-recon-app --all-targets -- -D warnings
```

2026-07-10 focused 结果：scan-runner 13/13、DB audit 8/8，check 与 clippy 零错误/零 warning。
