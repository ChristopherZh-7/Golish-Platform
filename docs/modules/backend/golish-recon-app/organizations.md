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

组织 = 甲方资产情报库。命令面支持多级树（`parent_id`）+ 26 字段（8 基础 + 18 profile，对应前端 5-tab）。`types` wire 类型、`candidates` engagement 候选读写（被 asset_intel 用）、`validation` profile patch 校验。

## 公开接口

| 符号 | 说明 |
|---|---|
| 组织 CRUD Tauri 命令 | 增删改查 + profile patch |
| `types`（wire 类型） | 组织 DTO |
| `candidates`（`upsert_organization_candidates_for_org` / `OrganizationCandidates`） | engagement 候选 |
| `validation` | profile patch 校验 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 组织命令 |
| `artifact_cleanup.rs` | 删除组织前清理 target-bound 本地 artifact / sitemap / operation 绑定 |
| `types.rs` / `candidates.rs` / `validation.rs` | wire 类型 / 候选 / 校验 |

## 依赖

- crate 内 app-core、`golish-db`（repo::organizations / targets / sitemap_store / operation_state）；`serde` / `url`

## 注意事项 / 坑

- `grp` 字符串分级（§S1）兼容保留作回退；新 target 直接关联 `organization_id`。
- 树形 `parent_id` 自引用：删/移组织注意级联与环检测。
- 删除组织会先按 org 子树 target 引用解析 host，清理工作区内 target-bound `.golish/captures/<host>`、`.golish/analysis/<host>`、`sitemap_store` 中对应 entry，并清空指向该 org 子树的 `operation_state.engagement_org_id`；日志 / transcript / audit / finding 历史证据不在这里删除。
- **不变量 I2**：组织 CRUD 验所有权（IDOR）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app organizations
```
