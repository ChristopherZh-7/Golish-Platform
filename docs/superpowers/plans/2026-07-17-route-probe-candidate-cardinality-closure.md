# Route Probe / Candidate Cardinality Closure 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 修复零字节统一响应被路径探测误判为 positive，并把完整目录证据按 exact origin 聚合为可验证且有界的 Candidate observation，使当前 Triage operation 可重放进入 analyst。

**架构：** `route_probe_paths` 在持久化前正确识别同状态、同元数据、同空内容的 candidate/baseline；`golish-db` 在 Candidate seed transaction 内锁定并重验完整 Enumeration 行，再生成一个带完整 set count/hash 和 bounded preview 的 `directory_entry_set_v1`。原始 DB/ledger 事实不删除，100-item policy 只约束聚合后的最终 work items。

**技术栈：** Rust 2021、Tokio/reqwest、sqlx/Postgres、serde_json、cargo-nextest、Clippy。

## 文件结构

- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`：零字节 uniform signature 规则及单测。
- 修改 `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`：directory row-set projection、digest、preview、per-origin observation 及单测。
- 修改 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`：记录零字节 baseline 判定。
- 修改 `docs/modules/backend/golish-db/repo.md`：记录 Candidate directory set 聚合合同。
- 修改 `docs/modules/INDEX.md`：同步两张模块卡状态说明。
- 修改 `feature_list.json`、`agent-progress.md`：唯一 active feature 与验证证据。

## 任务 1：用失败测试锁定两个实体根因

**文件：**
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`
- 修改 `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`

**步骤 1：** 新增 `identical_zero_body_candidate_and_baseline_are_uniform`，构造不同 URL、相同 200/status/content metadata/empty hashes 的两个 `ProbeResponseSignature`，断言 `response_signatures_are_uniform` 为 true。

**步骤 2：** 新增 `directory_rows_over_manifest_limit_collapse_to_one_exact_origin_set`，用同一 exact origin 构造 `MAX_ATTACK_MANIFEST_ITEMS + 1` 个不同目录行，断言 merge 成 `surface_analysis_v1 + directory_entry_set_v1` 两项，set 的 `entry_count` 完整、preview 上限 32、`preview_truncated=true`、support evidence 不漂移。

**步骤 3：** 先执行空间守卫，再运行 RED：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(identical_zero_body_candidate_and_baseline_are_uniform)' --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(directory_rows_over_manifest_limit_collapse_to_one_exact_origin_set)' --status-level fail
```

预期：两个测试因旧实现的 `body_len > 0` 和逐行 work item/100-item rejection 分别失败。

**提交：** 本共享 dirty tree 不自动 commit；完成全部 DoD 后由用户决定提交范围。

## 任务 2：修复 route-probe 零字节 uniform 判定

**文件：**
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`

**步骤 1：** 在 `response_signatures_are_uniform` 增加窄 helper/分支，只在双方 body 长度为零、body/template hash、content length、case-insensitive content type 全部相同时返回 uniform；保留 status mismatch 与非空 body 逻辑。

**步骤 2：** 增加相邻负例，证明 content type、declared length 或 hash 任一漂移时零字节签名不被误判为 uniform。

**步骤 3：** 运行 GREEN：

```bash
cd backend && cargo nextest run -p golish-pentest-app -E 'test(/identical_zero_body_candidate_and_baseline_are_uniform|zero_body_metadata_drift_is_not_uniform/)' --status-level fail
```

预期：2/2 passed。

**提交：** 本共享 dirty tree不自动 commit。

## 任务 3：实现 exact-origin directory set observation

**文件：**
- 修改 `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`

**步骤 1：** 定义 `DIRECTORY_ENTRY_SET_OBSERVATION_SCHEMA` 和 `MAX_DIRECTORY_ENTRY_PREVIEW=32`；将合法目录行先按 canonical origin key 分组，不再逐行 push work item。

**步骤 2：** 对每组构造排序后的完整 row projections，计算 `entry_set_sha256`；preview 使用稳定 security-relevance rank、URL、UUID 顺序取前 32，JSON 写完整 `entry_count`、digest、preview count/truncated 和 sorted source evidence IDs。

**步骤 3：** 生成一个 `SeedAttackObservation`：stable key `directory_entry_set:<origin identity hash>`、origin frozen target snapshot、`WSTG-INFO`、完整 observation hash；support map 同时保留 surface 与 set 的 exact evidence IDs。

**步骤 4：** 更新既有 single-row、invalid-row、lineage-drift 测试的 shape 断言；新增 member content/identity drift 导致 set digest 改变的测试。

**步骤 5：** 运行 GREEN：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(/exact_enumeration_support|enumeration_support|directory_support|directory_rows_over_manifest_limit|directory_entry_set/)' --status-level fail
```

预期：全部 focused tests passed，101+ raw rows只产生一个 set item。

**提交：** 本共享 dirty tree不自动 commit。

## 任务 4：同步模块卡和 active feature 状态

**文件：**
- 修改 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/INDEX.md`
- 修改 `feature_list.json`
- 修改 `agent-progress.md`

**步骤 1：** 在 bridge 卡记录零字节 baseline exact-match 规则；在 DB repo 卡记录 per-origin directory set count/hash/preview、原始行保留与 final policy 顺序；INDEX 两模块状态列同步本轮日期/说明。

**步骤 2：** 将前一 `stage-team-scope-bounded-worker-admission-2026-07-17` 暂停为 `blocked` 并说明代码保留、本轮用户切换；新增本 feature 为唯一 `in_progress`。

**步骤 3：** `agent-progress.md` 记录 live operation、3,735 rows、21+3,735>100、两个 1,866 空响应集合、RED/GREEN run ids 和所有退出码。

**验证：**

```bash
jq empty feature_list.json
jq -e '[.features[] | select(.status == "in_progress")] | length == 1' feature_list.json
git diff --check
```

预期：全部 exit 0。

**提交：** 本共享 dirty tree不自动 commit。

## 任务 5：扩大验证并证明当前实体形状可收敛

**文件：** 不新增生产文件。

**步骤 1：** 运行 affected crates：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -p golish-db -E 'test(route_probe) | test(attack_candidate) | test(directory)' --status-level fail
cd backend && cargo check -p golish-pentest-app -p golish-db
cd backend && cargo clippy -p golish-pentest-app -p golish-db --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

预期：测试/检查 exit 0，Clippy 零 warning。

**步骤 2：** 对本机 embedded Postgres 做只读实体核对，记录 21 个 formulaic surface、3,735 个 exact non-root 2xx directory rows、五个 exact origins；根据实现合同验证 final shape `21 + 5 + 0 scanner leads = 26`。不调用 provider、scanner或外部 API。

**步骤 3：** 运行完整门禁：

```bash
just precommit
jq empty feature_list.json
git diff --check
```

预期：`just precommit` 打印全部检查通过；JSON/diff exit 0。若完整门禁失败，feature 保持 `in_progress` 并记录精确 blocker，不伪标 passing。

**提交：** full DoD 后仍不自动 commit，由用户决定是否提交共享工作树。
