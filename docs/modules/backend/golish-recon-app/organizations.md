# golish-recon-app / organizations

> **一句话职责**：组织（甲方资产情报库）Tauri 命令——多级树形（`parent_id` 自引用）+ 8 基础 + 18 profile 字段（域名/网络/范围/证书/子公司/业务系统/云资产/GitHub/社交/历史漏洞/联系人…），子模块 types/candidates/validation。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/organizations/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改组织 CRUD、树形结构（parent_id）、profile 字段（5-tab UI 对应）、engagement 候选时
- 改 profile patch 校验（`validation`）或候选读写（`candidates`）时

## 职责

组织 = 甲方资产情报库。命令面支持多级树（`parent_id`）+ 26 字段（8 基础 + 18 profile，对应前端 5-tab）。`types` 是 ts-rs wire 类型源，包含 stable `OrganizationCandidate` / `UnitReviewDecisionRow` / `UnitReviewSubmission`；`candidates` 负责 engagement 候选读写与 existing-child stable projection；`validation` 负责 profile patch 校验。

## 公开接口

| 符号 | 说明 |
|---|---|
| 组织 CRUD Tauri 命令 | 增删改查 + profile patch |
| `types`（wire 类型） | 组织 DTO + ts-rs generated candidate/unit-review contracts |
| `candidates`（`upsert_organization_candidates_for_org` / `OrganizationCandidates`） | engagement 候选 |
| `validation` | profile patch 校验 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 组织命令 |
| `artifact_cleanup.rs` | 只消费 committed deletion-job frozen snapshot，幂等清理 target-bound 本地 artifact / sitemap；不在 request transaction 做文件 I/O |
| `types.rs` / `candidates.rs` / `validation.rs` | wire 类型 / 候选 / 校验 |

## 依赖

- crate 内 app-core、`golish-db`（repo::organizations / targets / sitemap_store / operation_state）；`serde` / `url`

## 注意事项 / 坑

- `grp` 字符串分级（§S1）兼容保留作回退；新 target 直接关联 `organization_id`。
- 树形 `parent_id` 自引用：删/移组织注意级联与环检测。
- `organization_delete` 只经 Cleanup `OrganizationDeletionPort` 提交 active workspace witness 与 DB precheck/invalidation job；DB 从 server-owned project scope 冻结 canonical root，不信 target snapshot/caller path。不再直接删文件或 live rows。`DbBackedOrganizationArtifactCleaner` 对 canonical root/namespace/host dir 做 symlink-escape 检查，只消费 committed frozen snapshot；sitemap prune 使用 project-scoped JSON CAS 防并发丢数据，hard delete 由 DB-global Cleanup worker 的后续事务完成。
- Cleanup 返回 active stage-fork conflict 时，`organization_delete` 必须映射成 `GolishError::OrganizationDeletionActiveStageFork`；其它 Cleanup 错误仍保持内部错误边界，不能靠解析错误文本分支。
- **不变量 I2**：组织 CRUD 验所有权（IDOR）。
- `OrganizationCandidate.id` 是 wire 必填；已有 direct child 会在 `organization_candidates_list` 中合成为 `existing-org:<uuid>`，并显式携带 `organization_id`。editable name 不能用于恢复或重算身份。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app organizations
```
