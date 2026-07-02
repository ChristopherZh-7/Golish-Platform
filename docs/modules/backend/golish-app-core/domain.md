# golish-app-core / domain

> **一句话职责**：跨服务共享 domain DTO（servitization S1-3）——remote-ready 契约类型，多个 app 服务都需要但谁都不该独占（搬这里打破 sibling-crate 环）；当前是 recon `targets` 面（`Target`/`Scope`/`ReconUpdate`/`DirectoryEntry`），被 recon/pentest/agent 共用。

- **类型**：目录模块（属于 crate [`golish-app-core`](../golish-app-core.md)）
- **路径**：`backend/crates/golish-app-core/src/domain/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改被多服务共用的 target/scope/directory DTO（且要 ts-rs 同步前端）时
- 排查为何某 DTO 放在 app-core 而非某个具体服务 crate 时

## 职责

持有 >1 个 app 服务需要、但单个服务不该 own 的 remote-ready 契约类型（放这里避免 recon-app↔pentest-app 等 sibling 环）。当前 `targets` 子模块导出 recon targets 面 DTO，供 recon/pentest/agent 服务消费，并 ts-rs 导出给前端。

## 公开接口

| 符号 | 说明 |
|---|---|
| `targets`（`Target` / `Scope` / `ReconUpdate` / `DirectoryEntry` / …） | 跨服务 target 面 DTO（ts-rs 导出） |
| `targets::rank_attack_surface_seeds` | EAS handoff seed 排序/限流；已有 direct IP target 时折叠解析到该 IP 的 domain/url/端口 URL 别名，避免 Prober 主扫 worklist 被别名和端口端点撑大；未折叠的 domain/url 只承载 LIVENESS，PORT/SERVICE 属于 IP/CIDR host |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub mod targets` |
| `targets.rs` | target/scope/recon/directory DTO |

## 依赖

- `serde`、`ts-rs`

## 注意事项 / 坑

- **不变量 I5**：DTO 用 `ts_rs::TS` 导出到 `frontend/lib/generated/`，别手写第二份。
- 放这里的判据：**多服务共用 + 谁都不该独占**；单服务私有 DTO 留在该服务 crate，别都往这堆。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core domain
```
