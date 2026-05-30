# P0-2 重建 ts-rs 类型同步链（I5）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划；每个任务单独 commit。
> Relates to: `docs/design/2026-05-29-architecture-optimization.md` §4.2 / §5 P0-2；AGENTS.md 不变量 I5。

**目标：** 把跨 IPC（Rust → 前端）的数据类型用 `#[derive(ts_rs::TS)]` 自动生成到 `frontend/lib/generated/`，消除前端手写类型与后端的漂移，落实 AGENTS.md I5。

**架构：** 后端各跨 IPC DTO 加 `#[derive(TS)]` + `#[ts(export, export_to = "...generated/...")]`；通过一个 `#[test]` 触发 ts-rs 导出（ts-rs 的导出在测试期发生），由 `just gen-types` 封装；前端把手写类型的 import 逐步切到 `lib/generated/`；CI 增加「生成后 git diff 必须为空」的漂移校验。

**技术栈：** Rust（`ts-rs` crate）、`cargo nextest` / `cargo test`、TypeScript、`just`、biome、git。

---

## 现状（事实，带证据）

- **ts-rs 当前未真正接入**：全仓库对 `ts-rs` 的引用只有一句注释——`backend/crates/golish-agent-kit/src/harness/types.rs:158`（`// 用 newtype 包 StageKind 是为了 …… ts-rs 友好扩字段`）。`backend/Cargo.toml` 的 `[workspace.dependencies]` 中**没有** `ts-rs`；无任何 `#[derive(ts_rs::TS)]` / `#[ts(export)]`。
- **生成目录不存在**：`frontend/lib/generated/`（glob 0 命中）。
- **前端手写类型有漂移风险**：`frontend/lib/ai/types.ts`（785 行）为手写；其它域类型散落在 `frontend/lib/pentest/*`、`frontend/lib/target-panel/types.ts` 等。
- **裸 invoke 旁路**（顺带记录，归 P0-4，本计划不改）：`frontend/components/PipelinePanel/PipelinePanel.tsx:108` 直接 `invoke("pipeline_list")` 未走 `frontend/lib/api/pipeline.ts:11` 的 `listPipelines`。

> 因此本计划是**从零搭建** ts-rs 链路，而非「修复已有链路」。

---

## 文件结构（创建 / 修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/Cargo.toml` | 修改 | `[workspace.dependencies]` 增 `ts-rs`；`[profile]` 无关 |
| `backend/crates/golish-db/Cargo.toml` | 修改 | 增 `ts-rs`（先以 1 个试点 crate 验证链路） |
| `backend/crates/golish-db/src/models/pentest.rs`（及同目录试点类型） | 修改 | 给试点 DTO 加 `#[derive(TS)] #[ts(export, export_to = "...")]` |
| `backend/crates/golish-db/src/models/ts_export_test.rs` | 新建 | 触发 ts-rs 导出的 `#[test]`（试点） |
| `frontend/lib/generated/.gitkeep` | 新建 | 占位，确保目录入库 |
| `frontend/lib/generated/*.ts` | 生成物 | ts-rs 导出（**禁止手改**，AGENTS.md §2.8 I5） |
| `justfile` | 修改 | 增 `gen-types` recipe + 接入 `check` 的漂移校验 |
| `frontend/lib/<域>/types.ts` 调用点 | 修改 | import 从手写切到 `lib/generated/`（逐域，YAGNI：先试点域） |
| `.github/workflows/*.yml`（或现有 CI 配置） | 修改 | 增「生成后 `git diff --exit-code`」步骤 |

> **DRY / YAGNI**：先用 `golish-db` 的 1～2 个 DTO 打通「derive → 测试导出 → 前端 import → CI 校验」全链路（任务 1-7），再批量铺开其余域（任务 8）。不要一上来给所有 DTO 加 derive。

---

## 任务分解（小步骤）

### 任务 1：加 `ts-rs` workspace 依赖

- **文件：** `backend/Cargo.toml`
- **步骤：**
  1. 在 `[workspace.dependencies]` 增加（版本以动手时 `cargo add ts-rs -p golish-db` 实际解析为准，下方为占位锚点）：

```toml
# Cross-IPC type generation (Rust -> TS). See docs/design/2026-05-29-architecture-optimization.md §4.2
ts-rs = { version = "10", features = ["serde-compat", "uuid-impl", "chrono-impl"] }
```

  2. 在 `backend/crates/golish-db/Cargo.toml` 的 `[dependencies]` 增 `ts-rs = { workspace = true }`。
- **验证：** `cd backend && cargo metadata --format-version 1 >/dev/null`（依赖图可解析，不报 ts-rs 缺失）。
- **提交：** `chore(types): add ts-rs workspace dependency`

### 任务 2：给试点 DTO 加 `#[derive(TS)]`

- **文件：** `backend/crates/golish-db/src/models/pentest.rs`（试点选 `Target` 或 `Finding` 这类前端高频用类型）
- **步骤：** 在试点 struct 上叠加 derive 与导出属性（保留既有 `FromRow`/`Serialize`）：

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Target {
    // ……既有字段不变……
}
```

  > `export_to` 是相对 crate 根的路径，指向 `frontend/lib/generated/`；动手时按该 crate 实际深度校准 `../` 层数（见任务 4 验证）。
- **验证：** `cd backend && cargo check -p golish-db`（编译过，derive 宏展开无误）。
- **提交：** `feat(types): derive ts-rs TS on golish-db pilot DTOs`

### 任务 3：写「导出触发」测试

- **文件：** `backend/crates/golish-db/src/models/ts_export_test.rs`（并在 `models/mod.rs` 加 `#[cfg(test)] mod ts_export_test;`）
- **步骤：**

```rust
//! ts-rs exports types as a side effect of `TS::export_all_to`/`export`.
//! Running this test (re)writes frontend/lib/generated/*.ts.
#[cfg(test)]
mod tests {
    use ts_rs::TS;

    #[test]
    fn export_bindings() {
        crate::models::pentest::Target::export_all().expect("export Target bindings");
    }
}
```

- **验证：** `cd backend && cargo test -p golish-db export_bindings`；预期：测试通过，且 `frontend/lib/generated/Target.ts` 被生成。
- **提交：** `test(types): add ts-rs export trigger test`

### 任务 4：固化生成目录与路径

- **文件：** `frontend/lib/generated/.gitkeep`（新建）
- **步骤：**
  1. 创建 `.gitkeep` 占位。
  2. 运行任务 3 的测试，确认生成文件落在 `frontend/lib/generated/`；若路径偏移，回任务 2 调 `export_to` 的 `../` 层数。
- **验证：** `ls frontend/lib/generated/` 含生成的 `.ts`；`git status` 显示新增生成文件。
- **提交：** `chore(types): seed frontend/lib/generated directory`

### 任务 5：加 `just gen-types` recipe

- **文件：** `justfile`
- **步骤：** 增 recipe（封装导出测试），并在 `check` 末尾接入漂移校验：

```just
# Regenerate cross-IPC TypeScript bindings from Rust (ts-rs)
gen-types:
    cd backend && cargo test --workspace export_bindings

# Fail if generated bindings are stale (run after gen-types in CI)
check-types: gen-types
    git diff --exit-code -- frontend/lib/generated/
```

- **验证：** `just gen-types` 成功；改一个被导出 struct 字段后 `just check-types` 应**失败**（证明能抓漂移），还原后应**通过**。
- **提交：** `build(types): add gen-types / check-types just recipes`

### 任务 6：前端切试点 import 到 generated

- **文件：** 试点域的前端调用点（如 `frontend/lib/pentest/*.ts` 中 `Target` 的手写定义处）
- **步骤：**
  1. 删除试点类型的手写定义；
  2. 改为 `import type { Target } from "@/lib/generated/Target";`（按现有 tsconfig alias 写法）。
- **验证：** `just check-fe`（biome + tsc 通过，无类型缺失）。
- **提交：** `refactor(types): import pilot Target type from generated bindings`

### 任务 7：CI 增加漂移校验

- **文件：** 现有 CI（如 `.github/workflows/ci.yml`；动手前先 `ls .github/workflows/` 确认文件名）
- **步骤：** 在 Rust job 后增步骤：

```yaml
- name: Check generated TS bindings are up to date
  run: just check-types
```

- **验证：** 本地 `just check-types` 通过即代表 CI 步骤可行；故意制造漂移看是否红。
- **提交：** `ci(types): enforce generated bindings drift check`

### 任务 8：批量铺开其余跨 IPC DTO（试点链路验证后）

- **文件：** `backend/crates/golish-db/src/models/*`、`backend/crates/golish/src/tools/*/types.rs`、`backend/crates/golish-pentest-domain`、`backend/crates/golish-vuln-intel-domain` 等承载跨 IPC 类型的模块
- **步骤：**
  1. 先**盘点清单**：`rg "#\\[tauri::command\\]" -l backend/crates/golish/src` 找命令，回溯其参数/返回类型，列出需导出的 DTO（产出一张「类型 → 文件:行 → 前端使用处」表，附到本计划末尾或 design 文档）。
  2. 对每个 DTO 重复任务 2 的 derive；扩展任务 3 的导出测试（用 `export_all` 汇总）。
  3. 前端逐域替换 import（重复任务 6），每域一个 commit。
  4. 全部替换后删除 `frontend/lib/ai/types.ts`（785 行）中已被生成物覆盖的部分。
- **实施约定（工程效率，见 §验证）：** **批量**给一批 DTO 加 derive 后，**统一**跑一次 `cargo check`，集中批量修错，不要每加一个 derive 就编译一次。
- **验证：** `just check-types` 通过且 `frontend/lib/ai/types.ts` 行数显著下降；`just check-fe` 通过。
- **提交：** 每域一个 `refactor(types): migrate <domain> types to generated bindings`

---

## 影响面

- **后端**：`backend/Cargo.toml`、`golish-db/Cargo.toml`、`golish-db/src/models/*`、后续各域 DTO 模块；新增导出测试（仅测试期副作用，不影响运行时）。
- **前端**：新增 `frontend/lib/generated/`；逐域改 import；最终瘦身 `frontend/lib/ai/types.ts`。
- **构建/CI**：`justfile` 增 2 个 recipe；CI 增 1 个 drift 步骤。
- **不影响**：运行时行为、Tauri 命令签名、DB schema（纯类型生成）。

## 验证

| 命令 | 预期 |
|---|---|
| `just gen-types` | 生成 `frontend/lib/generated/*.ts`，退出码 0 |
| `just check-types` | 生成物与提交一致时通过；漂移时 `git diff --exit-code` 失败 |
| `cd backend && cargo test -p golish-db export_bindings` | 试点导出测试通过 |
| `just check-fe` | biome + tsc 通过，无类型缺失 |
| `just precommit` | 合并前全绿门禁 |

**实施约定（工程效率，来源：用户要求 2026-05-29；见全局记忆 `golish:workflow:backend-build-policy`）：** 后端改动采用「**批量改完 → 统一编译 → 批量修错**」节奏——把一批 DTO 的 derive/属性全部加完后，**只**统一跑一次 `cargo build` / `cargo check`（或 `just check-rust`），集中查看全部编译错误后**批量**修复，再统一编译验证；**不要每改一处就编译一次**。仅在「全部改完」后才进入编译-修错循环。最终合并仍以 `just precommit` 全绿为准。

## 回滚

- 链路是**纯增量**：未迁移的域仍用手写类型，二者并存。
- 回滚顺序：还原前端 import → 删 `frontend/lib/generated/` → 移除 derive 属性 → 删 `ts-rs` 依赖与 recipe。每个任务单 commit，可独立 revert。

## 风险

| 风险 | 缓解 |
|---|---|
| `export_to` 相对路径在不同 crate 深度下算错 | 任务 4 先用试点 crate 校准 `../` 层数，再铺开 |
| ts-rs 对 `chrono`/`uuid`/`serde` 的类型映射与手写不一致 | 任务 6 用 `just check-fe` 兜底，diff 生成与手写差异，必要时用 `#[ts(type = "...")]` 覆写 |
| 生成物被误手改 | CI `check-types` 的 `git diff --exit-code` 拦截；目录头注释标注「禁止手改」 |
| 大批量 derive 一次性引入编译错误 | 遵守上面的批量编译策略，集中批量修 |
| 与正在活跃的 asset_intel/findings 改动冲突 | 试点先避开活跃文件；铺开阶段按域小步提交，频繁 rebase |

---

## 自检

- **规格覆盖**：①盘点（任务 8.1）②derive（任务 2/8）③导出配置（任务 3-5）④前端 import（任务 6/8）⑤CI 校验（任务 5/7）——全覆盖。
- **占位符扫描**：版本号与 CI 文件名标注了「动手时以实际为准」，非 TODO；其余步骤均带具体代码/命令。
- **类型一致性**：试点统一用 `Target`；`export_bindings` 测试名、`gen-types`/`check-types` recipe 名前后一致。
