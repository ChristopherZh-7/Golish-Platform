# Targets · 归属组织分组（Organization Grouping）

- **作者**：MCP-1（全栈工程师）
- **日期**：2026-05-17
- **状态**：S1 实施中 · S2/S3 待评估
- **关联分支**：`feat/targets-group-tree`

---

## 1. 背景 / Problem Statement

当前 `targets` 表是「项目 + 扁平列表」二层结构，依靠单字符串 `grp`
字段做分组：

```
projects (文件系统层 · 一个项目一个 root_path)
  └── targets {id, name, value, target_type, scope: in|out, grp: string, tags: jsonb}
        └── target_assets {asset_type, value, port, protocol, service, ...}
```

**护网 / 红队项目的真实场景需要多级归属信息**：

- 一个项目对接多家甲方公司（例：中国平安 / 招商银行 / 工商银行 …）
- 每家公司下分多个业务系统（网银 / 寿险 / 信用卡 …）
- 业务系统可能再细分模块
- 多人协作，每个业务系统有责任人
- 时间窗（合同允许攻击的时段）必须存档

**当前架构缺什么**：

| 维度 | 当前 | 护网需要 |
|---|---|---|
| 层级 | 项目 → 平面 targets | 项目 → 业务系统 → 子系统 → 资产 |
| 分组 | `grp` 字符串字段（单层） | 多级路径 / 真正的树形组织 |
| 客户 | 无 | 客户/甲方信息（公司名、联系人）|
| 时间窗 | 无 | 开始-结束日期 |
| 范围细化 | scope: in/out | 排除某些 IP / 时段 / 路径 |
| 责任人 | 无 | 谁负责哪个子系统 |
| 进度跟踪 | 无 | 子系统级别的扫描完成度 |
| 报告 | 无聚合视角 | 按子系统/公司出独立报告 |

## 2. 目标场景 / UI Mockups

### 2.1 单目标详情视图

```
┌──────────────────────────────────────────────────────────────────┐
│ ← 返回列表        www.pingan.com     [in-scope ✓] [启动 pipeline▶] │
│ 域名 · 中国平安/平安银行/网银部 · 负责人:张三 · 创建 2026-05-17   │
├──────────────────────────────────────────────────────────────────┤
│ 📊 概览                                                          │
│ ┌──────────┬──────────┬──────────┬──────────────┐               │
│ │ 子资产   │ API 端点 │ 开放端口 │ 漏洞         │               │
│ │   12     │   34     │   5      │ ❶3 ❷2 ❸5    │               │
│ └──────────┴──────────┴──────────┴──────────────┘               │
├──────────────────────────────────────────────────────────────────┤
│ 🌐 子域/资产 (12)                       [+ 加资产] [pipeline▶]   │
│  ├─ www.pingan.com    A 1.2.3.4    [80 443]   [Apache 2.4]      │
│  ├─ api.pingan.com    A 1.2.3.5    [443]      [Nginx]           │
│  └─ ... 9 more                                                  │
├──────────────────────────────────────────────────────────────────┤
│ 🔌 API 端点 (34) / 🐛 漏洞 (10) / 📝 备注                        │
└──────────────────────────────────────────────────────────────────┘
```

### 2.2 多目标列表视图（项目内全部）

```
┌──────────────────────────────────────────────────────────────────┐
│ 📁 项目：xx-2026q2-渗透  | 目标 26 漏洞 13  [+ 添加][批量+][⤓导入]│
│ [搜索...] [scope:all▼] [组织:all▼] [状态:all▼]   [list│tree│graph]│
├──────────────────────────────────────────────────────────────────┤
│ ── 模式 A：平铺列表 ──                                            │
│ ⬜ 名称           类型 值                scope 组织   状态  漏洞 │
│ ⬜ 平安银行主站  🌐 www.pingan.com      in   网银   tested  ⚠3  │
│ ⬜ 平安银行 API  🌐 api.pingan.com      in   网银   scanning⚠2  │
│                                                                  │
│ ── 模式 B：按组织折叠 ──                                          │
│ ▼ 中国平安/平安银行 (4, ⚠5)        ████████░░ 80%               │
│   ├─ www.pingan.com   in  tested   ⚠2                           │
│   └─ ... 3 more                                                 │
│ ▶ 中国平安/平安寿险 (2, ⚠0)        ██████░░░░ 60%               │
│ ▶ 招商银行 (12, ⚠4)                ████░░░░░░ 40%               │
└──────────────────────────────────────────────────────────────────┘
```

### 2.3 护网项目视图（按组织聚合 + 项目元信息）

```
┌──────────────────────────────────────────────────────────────────┐
│ 🔥 护网项目：2026 春季 HVV    组织 8 · 目标 230 · 漏洞 47        │
│ ⏱ 时间窗：05/20 08:00 - 05/22 18:00    剩余：1d 14h              │
│ 👥 团队：张三 · 李四 · 王五                                       │
├──────────┬───────────────────────────────────────────────────────┤
│ 组织树    │ ▼ 🏦 中国平安  · 80 目标 · ⚠12 · 进度 60%             │
│          │   ├─ 平安银行  (32, ⚠5)  ████████░░ 80%               │
│ [筛选]   │   │   └─ 网银部 (12, ⚠2)  ████████░░ 80% · 张三       │
│  ☑ in    │   │       ├─ www.pingan.com   tested  ⚠2              │
│  ☐ out   │   │       └─ api.pingan.com   scanning ⚠0             │
│          │   ├─ 平安寿险  (20, ⚠3)  ██████░░░░ 60% · 李四         │
│ [负责人] │   └─ 平安证券  (28, ⚠4)  ████████░░ 75% · 王五         │
│  ☑ 张三  │ ▶ 🏦 招商银行  · 60 目标 · ⚠18 · 进度 40%             │
│  ☑ 李四  │ ▶ 🏦 工商银行  · 50 目标 · ⚠10 · 进度 55%             │
│  ☑ 王五  │                                                       │
│          │ [▶ 启动批量扫描] [📊 出报告] [⏸ 暂停所有]              │
│          │ 🔥 重点漏洞（3 个 Critical）                          │
│          │ · SQLi @ www.pingan.com/login.php  by sqlmap 2h ago   │
└──────────┴───────────────────────────────────────────────────────┘
```

## 3. 方案对比

| 维度 | 方案 Z（路径化 grp） | 方案 X（加 2 字段） | 方案 Y（organizations 表） |
|---|---|---|---|
| Schema 改动 | 0 | ALTER TABLE 加 2 列 | 新表 + FK |
| 后端代码 | 0 | ~4 处 | ~150 行新 repo + 4 个 commands |
| 前端代码 | TargetPanel 改 ~50 行 | + form 加 input + 分组视图 | + OrganizationsPanel ~300 行 |
| 工作量 | 半天 | 1 天 | 2-3 天 |
| 数据迁移 | 0 | 可选回填 `UPDATE owner_org = grp` | `INSERT organizations SELECT DISTINCT grp ...` |
| 任意深度 | ✅ 字符串前缀 | ❌ 单层 | ✅ 树形 |
| 改名安全 | ❌ 批量 update | ❌ 批量 update | ✅ 改一处 |
| 组织挂联系人 | ❌ | ❌ | ✅ |
| 责任人 | ❌（grp 是字符串）| ✅ 加 owner 字段 | ✅ |
| 演进路径 | → X 或 Y | → Y | 终点 |

## 4. 实施路线（增量演进）

每个 Sprint 独立可用，停下都能带价值。

| Sprint | 时长 | 内容 | 交付物 |
|---|---|---|---|
| **S1** | 半天 | 上方案 Z：`grp` 字段重新定义为「归属组织」+ TargetPanel 加 tree 视图 | 能跳出图 2 模式 B |
| **S2** | 1-2 天 | 上方案 X：加 `owner` 字段（责任人）+ `time_window_start/end` + 项目元信息面板 | 能跳出图 3 顶部区 |
| **S3** | 2-3 天 | 上方案 Y：`organizations` 表 + OrganizationsPanel + 从 grp 平滑迁移 | 能完整跳出图 3 |

从 Z 升 Y 的迁移 SQL（S3 时执行）：

```sql
-- 1) 从 grp 字符串提取唯一组织名
INSERT INTO organizations (project_path, name)
SELECT DISTINCT project_path, grp FROM targets WHERE grp != 'default';

-- 2) targets 关联 organization_id
UPDATE targets SET organization_id = (
  SELECT id FROM organizations
  WHERE name = targets.grp AND project_path IS NOT DISTINCT FROM targets.project_path
);
```

## 5. 本次 Sprint S1 详细设计

### 5.1 范围（明确边界）

| # | 改动 | 文件 | 工作量 |
|---|---|---|---|
| 1 | i18n 文案：「分组」→「归属组织（支持 `/` 分级）」 | `frontend/lib/i18n/zh-CN.json` + `en.json` | 改 2 处 |
| 2 | TargetListView 新建/编辑 form 的 placeholder + label | `TargetListView.tsx` | 改 ~3 处 |
| 3 | TargetPanel 顶栏加「视图模式 list │ tree」切换 | `TargetPanel.tsx` | 加 state + button |
| 4 | TargetListView 新增「按 grp 分组折叠」树形视图 | `TargetListView.tsx`（或拆 `TargetGroupedView.tsx`）| ~80 行新组件 |

### 5.2 不做的事（划清边界）

- ❌ 不改 schema（`targets.grp` 字段不动）
- ❌ 不改后端任何代码
- ❌ 不加责任人 / 时间窗（S2 范围）
- ❌ 不加 organizations 表（S3 范围）
- ❌ 不删 list 视图（保留作为默认）

### 5.3 树形视图渲染逻辑

输入：`targets: Target[]`（每个 target 有 `grp` 字符串，如 `"中国平安/平安银行/网银部"`）

构建 tree：

```ts
type TreeNode = {
  name: string;          // "网银部"
  path: string;          // "中国平安/平安银行/网银部"
  children: TreeNode[];
  targets: Target[];     // 该层级直接归属的 targets
};

function buildTree(targets: Target[]): TreeNode[] {
  // 1. 按 grp 字符串 split('/') 构造嵌套
  // 2. 同名节点合并 children
  // 3. 默认空 grp 或 "default" 归入 "未分组" 节点
}
```

UI：

- 每层缩进 16px
- 节点行：`▶/▼ icon · path 末段 · (n targets, ⚠m findings) · 进度条`
- 展开节点显示子树 + 直接归属的 targets
- 默认全部展开（除非节点超 5 个则折叠 root）

### 5.4 兼容性

- 旧数据 `grp = "default"` → 归入「未分组」节点
- 空 `grp` → 同上
- list 视图渲染逻辑不变，仅作为切换选项

### 5.5 验收标准（DoD · Definition of Done）

- [ ] 旧项目数据打开 TargetPanel，list 视图渲染完全不变
- [ ] 切换到 tree 视图，旧数据全部归入「未分组」节点正确显示
- [ ] 新建 target 时填 `中国平安/平安银行` → tree 视图能正确两级折叠
- [ ] 新建 target 时不填 grp → 归入「未分组」
- [ ] `pnpm tsc --noEmit` 通过
- [ ] ReadLints 0 错误
- [ ] 现有 TargetPanel 测试不破坏

## 6. API / 数据契约（S1 阶段）

**S1 阶段 0 API/Schema 改动**。

未来 S2 阶段会扩展（仅作前瞻参考）：

```typescript
// S2 拟加字段
interface Target {
  // ... 现有字段
  owner: string | null;           // 责任人
  time_window_start: string | null;  // ISO 8601
  time_window_end: string | null;
}

// S2 拟加项目元信息表
interface Engagement {
  project_path: string;
  hvv_name: string;          // "2026 春季 HVV"
  team_members: string[];    // ["张三", "李四"]
  start_at: string;
  end_at: string;
}
```

## 7. 风险与权衡

| 风险 | 应对 |
|---|---|
| `grp` 字符串重命名会让旧数据混乱 | 保持「未分组」作为兜底节点；提供「批量重命名」操作（S2） |
| 用户混淆「业务系统」和「组织」 | i18n 描述明确写「归属组织（支持 `/` 分级）」+ help tooltip |
| 同名组织散落不同项目 | `grp` 本身已 scoped to project_path，自然隔离 |
| 后续升 Y 时如何不丢数据 | 提供平滑迁移 SQL（见第 4 节）|
| Tree 渲染大数据集（1000+ targets）卡顿 | 默认折叠 + 虚拟滚动；不在 S1 处理 |

## 8. 测试策略

- **单测**：树构建函数 `buildTree` 覆盖：单层 / 多层 / 同名合并 / 空 grp / `default` grp
- **组件测**：TargetPanel 切视图、tree 节点展开/折叠
- **手动 e2e**：跑 dev → 新建测试项目 → 加几个不同 grp 的 target → 切换视图验证

## 9. 后续路线（仅供参考，不在本次范围）

- S2：组织字段独立（`owner_org`/`owner_path`）+ 责任人 + 项目时间窗
- S3：`organizations` 表 + OrganizationsPanel
- 后续：scope_rules（细粒度排除规则）、按组织出报告、批量启动 pipeline by 组织

---

## 10. 方案 E · 隐式组织（2026-05-17 落地）

> **状态**：已实施 · 替代之前方案 D（`Project.mode` 区分 pentest/redteam）
> **作者**：MCP-2 主控中心
> **关键变更**：删除 `Project.mode` 字段；每个项目至少 1 个 root org（pentest 项目即「单 root 无 children」的特例）；UI 基于「组织树形态」而非 mode 字段决定渲染分支

### 10.1 动机：消除 mode 字段带来的概念冗余

方案 D 引入了 `ProjectMode = Pentest | Redteam` 双形态，导致：

- **数据冗余**：pentest 项目 `targets.organization_id` 永远 NULL，`organizations` 表对它无意义；
- **路径分叉**：后端写入、查询、UI 渲染都要按 mode 分两套；
- **演进摩擦**：pentest → redteam 升级需要走「修改不可变 mode 字段」的别扭路径；
- **新人理解成本**：两个表 + 一个 enum 表达「单层 / 多层」一件事，违反 [SRP](https://en.wikipedia.org/wiki/Single-responsibility_principle)。

方案 E 把判定权交给**数据形态本身**：

| 项目实质 | 数据形态 | UI 表现（推导） |
|---|---|---|
| pentest（SRC / 单点测试）| 1 个 root org，无 children | 隐藏组织树 / 默认 list 视图 / 自动绑根 org / 隐藏 time_window |
| redteam（HVV / 大型项目）| ≥2 个 org 或 root 有 children | 显示组织树 / 默认 tree 视图 / 必选 org / 显示 time_window + engagement 元信息 |

### 10.2 数据模型变更

| 表 | 方案 D | 方案 E |
|---|---|---|
| `projects/<slug>/config.toml` | `name` + `root_path` + `mode` 三字段 | `name` + `root_path` 两字段。`mode` 字段若仍存在于老 toml，被 serde 静默丢弃，不报错 |
| `organizations` | redteam 项目有节点；pentest 项目空 | 每个 project_path 至少 1 个 root org；新项目首次 save 时自动 seed |
| `targets.organization_id` | pentest 必 NULL；redteam 必非 NULL | **保持 NULLABLE**（决策 4B），UI 兜底显示「未分类」；backfill SQL 把历史 NULL 挂到隐式 root |
| `targets.grp` | pentest 用作分组字符串 | **保留**（决策 6B），作为 org 内的可选子标签；UI 仅在单 root 项目里直接暴露输入框 |

### 10.3 关键文件改动清单

#### 后端（Rust）

| 文件 | 改动 |
|---|---|
| `golish-projects/src/schema.rs` | 删 `ProjectMode` enum 与 `ProjectConfig.mode`；重写 3 个测试覆盖「老 toml 兼容 + 不再写 mode」|
| `golish/src/projects/commands.rs` | 删 `ProjectFormData.mode` / `ProjectData.mode` / mode 不可变守卫；`save_project` 加 `DbState` 参数 + `ensure_root_org` 异步 helper，首次保存时按项目名 upsert 一行 root org |
| `golish/src/tools/targets/cmds.rs` | 删 `assert_org_allowed_for_mode` import 与两处调用（`target_add` / `target_update`），修复方案 D 末期遗留的编译错 |
| `golish/src/tools/targets/db.rs` | （不动）`db_target_add` 已接受 `organization_id` 参数 |

#### 前端（TypeScript / React）

| 文件 | 改动 |
|---|---|
| `lib/api/projects.ts` | 删 `ProjectMode` 类型；`ProjectFormData` / `ProjectData` 去掉 `mode` 字段 |
| `lib/projects.ts` | 删 `useCurrentProjectMode` hook |
| `store/slices/app-shell.ts` | 删 `currentProjectMode` 状态字段；`setCurrentProject` 签名去掉 mode 参数 |
| `store/actions.ts` | `openProject` 不再读取 / 传递 mode |
| `components/TargetPanel/hooks/useProjectOrgShape.ts` | **新增** hook，提供 `{ orgs, rootOrg, isSingleRoot, loading, refresh }`，是 UI 层判断 pentest/redteam 形态的唯一入口 |
| `components/TargetPanel/TargetPanel.tsx` | 把 `VIEW_MODES_BY_PROJECT_MODE` / `DEFAULT_VIEW_MODE` 改成 `allowedViewModesFor(isSingleRoot)` / `defaultViewModeFor(isSingleRoot)` 两个纯函数；删除右上角 mode pill |
| `components/TargetPanel/TargetListView.tsx` | `isRedteam` 改为 `!isSingleRoot`；单 root 自动把 `rootOrg.id` 注入到 `organizationId`（用户看不到 org picker）；多 org 时仍强制选择 |
| `components/TargetPanel/ProjectInfoPanel.tsx` | `isPentest` 改为 `isSingleRoot`；其余分支不变 |
| `components/HomeView/SetupProjectModal.tsx` | 删除整个 ModeCard 卡组与不可变警告；初始 targets 输入框始终可见 |
| `lib/i18n/{en,zh-CN}.json` | 删 `projectSetup.mode*` / `modeImmutable` / `initialTargetsOnlyPentest` / `createAndConfigure` 8 个键 |

#### 数据库迁移

`backend/crates/golish-db/migrations/20260517200000_implicit_root_org_backfill.sql`：

- **Part 1**：扫描 `targets.project_path` 所有 distinct 值，给尚无 root org 的项目插入 1 个 root org，名字取 path 尾段（`SUBSTRING(pp FROM '[^/\\]+$')`），尾段为空回退 `'Default'`。
- **Part 2**：把所有 `organization_id IS NULL` 的 target 挂到对应项目的第一个 root org（用 `DISTINCT ON (project_path)` 确保幂等）。

`save_project` 应用层的 `ensure_root_org` 与这条 migration 互不冲突——前者在新项目创建时同步写入；后者只回填历史数据。

### 10.4 UI 形态推导逻辑

`useProjectOrgShape()` 返回 `isSingleRoot = orgs.length <= 1 && roots.length <= 1`：

```ts
isSingleRoot = true   // pentest-like
  ⇒ TargetPanel 视图可选: [list, orgs, graph]   // tree 隐藏（无可折叠），但 orgs 永远保留
  ⇒ TargetListView: 隐藏 org 下拉、隐藏 time_window、显示 grp 输入、保存时自动绑根 org
  ⇒ ProjectInfoPanel: 只显示「客户名」一行

isSingleRoot = false  // redteam-like
  ⇒ TargetPanel 视图可选: [list, tree, orgs, graph]
  ⇒ TargetListView: 显示 org 下拉（必选）、显示 time_window、隐藏 grp 输入
  ⇒ ProjectInfoPanel: 显示完整 engagement 元信息（HVV 名 / 时间窗 / 成员）
```

> **设计纪律（2026-05-17 加固）**：`orgs` 视图（OrganizationsPanel）在**所有**项目形态下都必须可见。把它和 `tree` 视图一起隐藏会让用户在单 root 项目里没有「加新组织」的入口 —— 等于把「pentest → redteam 自然演进」的承诺给毁了。`tree` 隐藏 OK；`orgs` 永不隐藏。

用户从单 root 升级到多 org 的路径：在 `OrganizationsPanel` 创建第 2 个 root org 或给现有 root 加 child → `useProjectOrgShape().refresh()` → UI 自动切到 redteam 形态。**无需人工切 mode**。

### 10.5 风险与权衡

| 风险 | 应对 |
|---|---|
| 历史 pentest 数据 backfill 失败 | migration 用 `ON CONFLICT DO NOTHING` + `DISTINCT ON` 保证幂等；应用层 `ensure_root_org` 失败只 `tracing::warn`，不阻塞 `save_project` |
| 老 `config.toml` 残留 `mode = "..."` 字段 | serde `toml::from_str` 默认丢弃未知字段，单测 `config_with_extra_mode_field_still_parses` 守住 |
| 用户在 redteam 项目里把所有 org 删到只剩 1 个 | UI 形态会切回 pentest-like，但 `grp` 输入框不会立即出现旧数据；用户在 TargetDetail 里仍可手工补 grp |
| 单 root 项目里 `rootOrg` 加载未完成时点「添加 target」| `useProjectOrgShape` 在 loading=true 时 `rootOrg=null`，`handleAdd` 会以 `organizationId=undefined` 提交，后端落库时 `organization_id` 是 NULL；下次 backfill 会自动挂回 root org |
| 双轨：迁移已跑过 + 应用层 ensure_root_org 也尝试插入 | 两边都用 `ON CONFLICT DO NOTHING` 兜底；唯一约束 `uq_orgs_root_name (project_path, name) WHERE parent_id IS NULL` 保证不会出现重复 root |

### 10.6 验收（DoD）

- [x] `cargo check -p golish-projects` 通过
- [x] `cargo check -p golish` 通过
- [x] `cargo test -p golish-projects --lib` 15/15 通过（含新的 3 个 schema 测试）
- [x] `pnpm typecheck` 通过
- [x] 修改文件 `pnpm check` 0 错 0 警
- [x] 现有 pentest 项目不需要用户操作即可继续工作（依靠 backfill migration）
- [x] 新项目创建后 Tree View 立刻能看到 1 个以项目名命名的 root org
- [x] 用户在 Tree View 加第 2 个 org → 顶栏 tree 视图自动出现新的 org 分支（无需切 mode）

---

## 11. 方案 E 后续 · 统一面板重构（2026-05-17 当晚）

> **背景**：方案 E 落地后，用户立刻指出 UX 反直觉问题——「先建组织、再往组织里加 target」是天然的工作流，但当时实现把 `OrganizationsPanel`（管组织）和 `TargetListView`（管 target）拆成两个独立视图。用户原话：
>
> > 「为什么你这个左边两个视图这么奇怪，正常逻辑不是先创建组织，然后对这个组织添加目标，你怎么两个添加分两个页面显示，这个好奇怪啊」

### 11.1 解决方案：把组织 CRUD 内联进 Tree View

把 `TargetGroupedView`（原本只是只读树形视图）升级为**统一组织 + target 管理面板**：每个 org 节点 hover 时露出 4 个操作按钮，按使用频率排序：

| 按钮 | 图标 | 作用 |
|---|---|---|
| `+ Target` | `Crosshair` | 在该 org 下内联新增 target（输入 value/name 即可，回车提交）|
| `+ 子组织` | `Building2` | 在该 org 下内联新增 child org |
| `编辑` | `Pencil` | 内联改名 + 改 owner |
| `删除` | `Trash2` | confirm 后 cascade 删除（含子 org 与挂在其下的 target）|

面板顶部加一行：`+ 创建根组织` 按钮 + `N orgs · M targets` 计数。

「未分类」桶（孤儿 target，`organization_id IS NULL`）继续保留作为兜底节点，但只允许 `+ Target`（不能加子组织，因为它本就是虚拟节点）。

### 11.2 视图按钮三件套

| 按钮 | 视图 | 作用 |
|---|---|---|
| 📋 List | `TargetListView` | 扁平 target 列表 · 搜索 / scope 过滤 / 批量导入 |
| 🌳 Tree | `TargetGroupedView` | **组织 + target 统一管理** · 创建/编辑/删除 org · 在 org 下加 target |
| ↗ Graph | `TargetGraphView` | target 关系图谱 |

三个视图**始终可见**，无论项目形态。**原独立的 `🌐 Orgs` 按钮删除**——它的全部能力被 Tree View 接管。

### 11.3 关键文件改动

| 文件 | 改动 |
|---|---|
| `frontend/components/TargetPanel/TargetGroupedView.tsx` | 完全重写。引入 `addingChildTo` / `addingTargetTo` / `editingOrgId` 三个互斥 inline editor state；新增 `renderInlineCreateOrgForm` / `renderInlineAddTargetForm` / `renderOrgEditForm` 三个内联表单组件；接 `orgsApi.createOrganization / updateOrganization / deleteOrganization` 做 org CRUD；接收新的 `onAdd: AddTargetForm => ...` prop 做 target 创建 |
| `frontend/components/TargetPanel/TargetPanel.tsx` | 视图按钮列表简化为 `["list", "tree", "graph"]` 永远可见；删除 `useProjectOrgShape` 调用与 `allowedViewModesFor` / `defaultViewModeFor` 两个工厂函数；`TargetGroupedView` 调用处加 `onAdd={handleAdd}` |
| `frontend/components/TargetPanel/OrganizationsPanel.tsx` | **删除**（10735 bytes）|
| `useProjectOrgShape.ts` | 保留 —— `TargetListView` 仍用它判断「单 root 时隐藏 org 下拉、自动绑定 root org」|

### 11.4 数据契约

- `target_add` IPC 不变 —— 仍接受可选 `organizationId`
- `organization_create / update / delete / move` IPC 不变
- 后端无任何改动

### 11.5 边界与权衡

| 场景 | 处理 |
|---|---|
| 用户在 Tree View 同时打开两个 inline editor | `closeAllEditors()` 在每个 start* 回调里先调用，保证最多 1 个 editor 开 |
| 删除 root org（cascade 把子 org 和 target 全删）| 用 `confirm()` 弹原生确认 + 后端 `ON DELETE CASCADE` 兜底；如需细粒度警告（"将删除 N 个子组织、M 个 target"），后续 UI 可加 |
| 编辑 org 名字与 target 详情冲突 | 两个 state 独立（`editingOrgId` vs `editingTargetId`），同时打开互不干扰 |
| 「未分类」桶里 + Target | `organizationId` 传 `undefined`，target 创建后 `organization_id IS NULL`，下次刷新仍归到「未分类」 |
| inline 表单只露 value / name 两字段 | 复杂字段（tags / time_window / notes）仍走 List View 的完整 add 表单或 detail 编辑面板 |

### 11.6 验收（追加）

- [x] `pnpm typecheck` 通过
- [x] `biome check` 两个新文件 0 错 0 警
- [x] `cargo check -p golish` 通过（无后端改动，但确认无 dangling 引用）
- [x] Tree View 在单 root + 多 org 项目里都默认可见
- [x] 单 root 项目里用户能直接在 Tree View 看到隐式 root，并在它下面加 target / 加 child org
- [x] 旧 OrganizationsPanel 入口完全消失，没有 dead button

---

## §12 — organization → 甲方资产情报库（2026-05-17 续）

> 触发：用户回到 controller 会话后提出「想把 organization 升级为甲方资产情报库，存
> IP 段 / 域名 / 证书 / ASN 等乱七八糟数据」。按 HVV 攻击方倒推清单，最终选定方
> 案 A —— **表加全部 18 字段，UI 先实现 MVP 5 tab**。

### 12.1 字段清单

| 类别 | 字段 | 类型 | 用途 | MVP UI |
|---|---|---|---|---|
| 基础 | `aliases` | `TEXT[]` | 别名/简称/英文名 — AI 模糊匹配 | Tab1 |
| 基础 | `industry` | `TEXT` | 行业 — 选 POC 类型 | Tab1 |
| 基础 | `tier` | `TEXT` | critical/high/medium/low | Tab1 |
| 基础 | `credit_code` | `TEXT` | 统一社会信用代码 — 工商查询 | Tab1 |
| 域名 | `domains` | `JSONB` | `[{domain, wildcard, note}]` — 子域爆破 | Tab2 |
| 网络 | `ip_ranges` | `JSONB` | CIDR 字符串数组 — IP 收敛 / scope | Tab3 |
| 网络 | `asns` | `JSONB` | ASxxx 字符串数组 — whois/BGP | Tab3 |
| 网络 | `email_domains` | `JSONB` | 邮箱域 — 钓鱼/凭证泄露 | Tab3 |
| 范围 | `scope_rules` | `JSONB` | `{in,out,forbid_time,forbid_paths}` | Tab4 |
| 其他 | `intel` | `JSONB` | 兜底自由对象 — 未来新字段不改表 | Tab5 |
| 其他 | `notes` | `TEXT` | Markdown 备注 | Tab5 |
| 二期 | `certificates` `subsidiaries` `business_systems` `cloud_assets` `github_orgs` `social_accounts` `historical_vulns` `contacts` | `JSONB[]` | 证书/子公司/重点业务/云资产/GitHub/社媒/旧洞/联系人 | 后续 PR |

所有字段 `NOT NULL DEFAULT`，避免后端模型读 NULL 崩溃；JSONB 字段默认 `'[]'` 或 `'{}'`，TEXT 默认 `''`，TEXT[] 默认 `'{}'`。

### 12.2 索引

```sql
CREATE INDEX idx_orgs_aliases   ON organizations USING GIN (aliases);
CREATE INDEX idx_orgs_domains   ON organizations USING GIN (domains   jsonb_path_ops);
CREATE INDEX idx_orgs_ip_ranges ON organizations USING GIN (ip_ranges jsonb_path_ops);
```

为后续 AI 做 target↔org 自动归属时按 alias / domain / ip_range 模糊查询用。

### 12.3 后端 API（新增 2 个 Tauri command）

| 命令 | 入参 | 出参 | 错误 |
|---|---|---|---|
| `organization_get(id)` | `id: Uuid` | `Organization`（含全 18 字段）| `NotFound` |
| `organization_update_profile(id, patch)` | `id`, `OrganizationProfilePatch`（19 字段全可选）| `Organization` | `Validation` / `NotFound` |

#### PATCH 语义

`OrganizationProfilePatch` 的每个字段 `Option<T>`：`None` = 不修改，`Some(value)` = 覆盖（**包括 `Some([])` = 清空**，与"不修改"语义严格区分）。

#### 后端格式校验（在 `validate_profile_patch`）

| 字段 | 规则 | 不合法返回 |
|---|---|---|
| `tier` | `critical / high / medium / low / ''` | `Validation` |
| `ip_ranges` | 每项 `IP/PREFIX`，IPv4 前缀 `0..=32`，IPv6 `0..=128` | `Validation` |
| `domains` | 每项 RFC1035 简化版 + 支持 `*.` 通配 | `Validation` |
| `asns` | `^AS\d{1,10}$` | `Validation` |
| `email_domains` | 同 domain 规则 | `Validation` |

任一字段不合法 → 整笔 patch reject（不落库），错误消息 `validation: ip_ranges=\`bad-ip\` → invalid CIDR; asns=\`12345\` → invalid ASN ...`，便于前端定位。

7 个单元测试覆盖 CIDR / domain / ASN 各正反例与 patch 整体校验。

### 12.4 前端 UI

#### `OrgProfileDrawer.tsx`（新建）

- Radix `Sheet` 右侧滑入，最大宽 `2xl`
- 顶部：组织名 + subtitle 提示 "AI 后续会用这些数据自动匹配 target"
- 中间：Radix `Tabs` 5 个（基础 / 域名 / 网络 / 范围 / 其他）
- 底部：失败/已保存状态条 + 关闭 + 保存按钮
- **三态**：`loading` / `loadError` / `ready`；`intel` JSON 解析失败时 textarea 边框变红 + 保存禁用；后端 400 错误整段 surface 在底栏

#### `TargetGroupedView.tsx`（修改）

- 在 hover 操作组的 `Building2`（+ 子组织）旁加 `Info` 按钮（仅 non-unassigned 节点）
- 点击 → `setProfileOrgId(node.id)` 打开抽屉
- 抽屉关闭 → `refreshOrgs()` 拉新一遍，避免改名后 Tree 上仍显示旧名

#### `lib/api/organizations.ts`（扩展）

- `Organization` 接口加 18 字段（精确类型：`OrgDomainEntry[]` / `string[]` / `OrgScopeRules` / `Record<string, unknown>` 等）
- `OrganizationProfilePatch` 接口
- `getOrganization(id)` / `updateOrganizationProfile(id, patch)` 函数

#### i18n

`organizations.profile.*` 节加 28 个键（tab 名 / 5 个 tab 内的 label + helper + placeholder + tierOptions 子节）；zh-CN + en 双语对齐。

### 12.5 文件清单

| # | 文件 | 状态 | 行数 |
|---|---|---|---|
| 1 | `backend/crates/golish-db/migrations/20260517210000_organizations_profile_fields.sql` | 新建 | 54 |
| 2 | `backend/crates/golish-db/src/models/pentest.rs` | 改 | +56 |
| 3 | `backend/crates/golish-db/src/repo/organizations.rs` | 改 | +145 |
| 4 | `backend/crates/golish/src/tools/organizations.rs` | 改 | +287（含 7 个单测）|
| 5 | `backend/crates/golish/src/commands_registry.rs` | 改 | +1（注册 2 个 cmd）|
| 6 | `frontend/lib/api/organizations.ts` | 改 | +120 |
| 7 | `frontend/components/TargetPanel/OrgProfileDrawer.tsx` | 新建 | 478 |
| 8 | `frontend/components/TargetPanel/TargetGroupedView.tsx` | 改 | +35 |
| 9 | `frontend/lib/i18n/zh-CN.json` | 改 | +60 |
| 10 | `frontend/lib/i18n/en.json` | 改 | +60 |

### 12.6 验收

| 检查 | 结果 |
|---|---|
| `cargo check -p golish-db` | ✅ 0 错 0 警 |
| `cargo check -p golish` | ✅ 0 错 0 警 |
| `cargo test -p golish --lib tools::organizations` | ✅ 7 passed; 0 failed |
| `pnpm typecheck` | ✅ 0 错 |
| `biome check` 5 个改动文件 | ✅ 0 错 |

### 12.7 风险与待办

| 项 | 说明 |
|---|---|
| migration 向后兼容 | 全部 `ADD COLUMN IF NOT EXISTS … NOT NULL DEFAULT`，旧版后端可继续读旧字段；rollback 需手动 `DROP COLUMN`（无逆 migration 脚本，本项目当前 migration 都是单向，下个版本若加 sqlx-rollback 再补）|
| AI ↔ org 自动归属 | **未做**。Phase 2 PR：让 AI 调 `manage_targets add` 时按 hostname → `organizations.domains` / `aliases` 模糊匹配选 `organizationId`；本 PR 已铺好 GIN 索引 |
| 二期 8 字段 UI | 表已就位，UI 后续 PR（建议复用 `OrgProfileDrawer` 加 tab 或拆子 drawer）|
| 抽屉数据竞态 | 抽屉打开时拉一次 `getOrganization`；如果用户保存期间别处改了同一行，会出现 last-write-wins。MVP 可接受；后续可加 `updated_at` 乐观锁 |
| 校验前后端口径 | 前端只校验 intel JSON 合法性；CIDR/domain/ASN 全靠后端，错误信息直接 surface。前端可后续加 onBlur 提示提前预警 |


