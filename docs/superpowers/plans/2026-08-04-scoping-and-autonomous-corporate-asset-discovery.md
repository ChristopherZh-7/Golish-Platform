# Scoping 企业实体确认与自主企业资产发现实施计划

> **设计**：`docs/design/2026-08-04-scoping-and-autonomous-corporate-asset-discovery.md`
>
> **集成功能**：`rag-first-unified-investigation-stage-2026-08-02`

## 1. Scoping 企业实体解析

1. 扩展 company lookup adapter，使 ENScan/企查查类与 0.zone org 查询统一返回 typed candidate、provider terminal status、raw artifact 与 Evidence。
2. 添加受控公开搜索 fallback；只有结构化 provider 不可用/空/冲突时允许调用，浏览结果必须 artifact-first 落账。
3. 增加 operation/org-scoped Company Identity receipt 和 finalizer；只有 confirmed 可 seal，歧义走一次 choice。
4. 更新 Scoping methodology/spec/tool list，保持 no target probing/no target creation。

## 2. Target Intel 自主 Goal 生产化

1. 用 Corporate Asset Discovery methodology 替换旧固定清单 prompt/spec。
2. 将 fixture semantic pivot executor 提升为 production：closed AST、scope authorization、provider capability compiler、receipt_v1、rate/cost policy。
3. 为 native provider 补 Tool Truth transport，不再在 receipt_v1 下无条件 unavailable；HTTP provider 继续 pinned transport。
4. 动态通用 SubAgent 只接收 name/prompt/subject refs，不注册固定 WHOIS/ASN/coverage role。

## 3. Observation、归属、可达与晋升

1. 新增 forward-only additive migration，保存 Company Identity、Asset Observation、attribution、reachability、provider metadata 与 promotion lineage。
2. 改 provider landing：先写 observation，禁止 current-run candidate 直接写 `scope=in` Target。
3. 实现确定性 attribution policy 与 AI claim review；共享/第三方/歧义不得晋升。
4. 将 Controller exact message chain、`update_plan` 轨迹、checkpoint、receipts 和实际落库投影为冻结的 `controller_work_memory`；reviewer 不依赖隐藏推理或 completion prose。
5. 打通 reviewer `REWORK`：完整 findings/residuals 追加到同一 Controller chain，原 WorkerRun 在下一 Goal epoch 继续，重新规划并补跑后再 review；fixed point 转 typed human hold。
4. 实现低影响 reachability typed operator；成功后原子 promote，写 liveness、Target/DNS/service 和完整字段。
5. exact-set/CAS/duplicate tests 覆盖并发与 replay。

## 4. 旧六轴退役

1. 删除 Target Intel spec expected techniques、固定 provider→WHOIS 顺序和 coverage prompt。
2. 删除 Target Intel 六轴 denominator、coverage projection、formulaic repair worklist 与 legacy Gate publication 分支。
3. 保留历史 migration/ledger rows 为审计数据，但 production/runtime 不读取兼容路径。
4. 加代码检索与契约测试，禁止六轴标识重新进入 Target Intel 运行路径。

## 5. 定向验证

在任何 Cargo 构建/测试前运行 `just space-guard`。随后按影响面执行：

1. `golish-recon-app`：company lookup、semantic compiler、provider runtime、observation/promotion/reachability focused tests。
2. `golish-agent-kit`：Goal contract/review/finalizer，无六轴 Gate focused tests。
3. `golish-agent-runtime`：Scoping/Target Intel tool exposure、same-chain REWORK、production semantic dispatch tests。
4. `golish-agent-app` + `golish-db`：migration、receipt、exact-set、handoff/final seal focused tests。
5. 受影响 crate scoped Clippy `-D warnings`；有 IPC/前端变更时跑 focused Vitest/Biome/typecheck。

## 6. 实体闭环

1. 新建本地受控 fixture operation，覆盖归属 owned/shared/ambiguous、reachable/unreachable、raw provider fields、duplicate 和 high-risk residual，跑至 Reporting。
2. 在 `/Users/christopherzheng/golish-platform/Test1` 新建 fresh 默安科技 operation，从 DB 解析准确 organization identity，以 `moresec.cn` 授权根运行完整新拓扑。
3. 用 `scripts/run_tree.py --db --full`、DB exact-set、transcript 和 run.log 证明每个阶段及 Target Intel 新契约。
4. 实体暴露问题继续修复并重新跑；不得复用历史 PASS。

## 7. 收口

1. 更新受影响模块卡与索引。
2. 把命令、退出码、关键 exact-set、session/operation id、transcript/run.log 路径写入 `agent-progress.md` 和 `feature_list.json.evidence`。
3. focused 全绿后执行用户已授权的 `just precommit`。
4. 全链路闭合后只创建一个 commit，不 push。
