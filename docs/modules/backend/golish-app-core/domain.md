# golish-app-core / domain

> **一句话职责**：跨服务共享 domain 契约（servitization S1-3）——remote-ready DTO 与不可跨 IPC 的 opaque server principal，多个 app 服务都需要但谁都不该独占。

- **类型**：目录模块（属于 crate [`golish-app-core`](../golish-app-core.md)）
- **路径**：`backend/crates/golish-app-core/src/domain/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改被多服务共用的 target/scope/directory DTO（且要 ts-rs 同步前端）或 trusted operator identity 时
- 排查为何某 DTO 放在 app-core 而非某个具体服务 crate 时

## 职责

持有 >1 个 app 服务需要、但单个服务不该 own 的契约类型（放这里避免 recon-app↔pentest-app 等 sibling 环）。`targets` 子模块导出 remote-ready recon DTO；`operator` 子模块故意不可 serde，只能由服务端 provider 构造。

## 公开接口

| 符号 | 说明 |
|---|---|
| `targets`（`Target` / `Scope` / `ReconUpdate` / `DirectoryEntry` / …） | 跨服务 target 面 DTO（ts-rs 导出） |
| `operator`（`OperatorId` / `OperatorChannel` / `TrustedOperatorPrincipal` / provider trait） | 字段私有且不实现 serde 的 privileged-action actor；身份来自 DB active principal，不来自 request |
| `targets::rank_attack_surface_seeds` | EAS handoff seed 排序/限流；保留 exact domain/url/vhost 身份，并把显式授权 IP/CIDR 提前，domain/url 承载 LIVENESS/WEB，IP 承载 PORT/SERVICE，CIDR 本行只承载 range LIVENESS/PORT 并由 child IP 下波承载 SERVICE/WEB；函数确定性排除 wildcard pattern，只 handoff concrete child；`real_ip` 关系不得折叠或自动授权 IP |
| `targets::web_root_url` / `targets::rank_enumeration_web_roots` | Enumeration web-root URL 推导与 alive-first 排序 helper；调用方负责决定是否 cap / next-wave backlog，不能把 cap 掉的 root 当作 checked_empty |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub mod operator; pub mod targets` |
| `operator.rs` | opaque trusted operator 与 provider port；compile-fail doctest锁定不可反序列化/不可 struct literal |
| `targets.rs` | target/scope/recon/directory DTO |

## 依赖

- `serde`、`ts-rs`、`async-trait`、`uuid`

## 注意事项 / 坑

- **不变量 I5**：DTO 用 `ts_rs::TS` 导出到 `frontend/lib/generated/`，别手写第二份。
- `TrustedOperatorPrincipal` 是反向例外：严禁 serde/TS；privileged command 必须从 server provider 获取。
- 放这里的判据：**多服务共用 + 谁都不该独占**；单服务私有 DTO 留在该服务 crate，别都往这堆。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core domain
```
