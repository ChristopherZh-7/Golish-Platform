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
