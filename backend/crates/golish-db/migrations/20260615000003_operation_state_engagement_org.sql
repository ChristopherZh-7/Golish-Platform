-- ============================================================================
-- operation_state 加 engagement_org_id 列（engagement-org isolation, 设计
-- 2026-06-15-engagement-org-isolation）
--
-- 锚定本次 operation 的 engagement = 哪个 root organization（scoping 确认的）。
-- 后续阶段的 fan-out / in-scope 读据此约束到该 org 子树（root + 子公司），杜绝
-- 同一个 workspace 里其它 engagement（如历史测过的公司）的 org 串入本次范围
-- （"测 example 串成平安" bug 的根上一环）。
--
-- 这不是恢复 2026-05-17 删掉的 engagements 元信息表：hvv_name / 时间窗 / 团队 /
-- 客户那些"项目元数据"仍走 organizations.profile + targets。这里只加一个 root org
-- 指针（nullable，向后兼容：旧行 NULL = 未绑定 = legacy whole-DB 行为）。
-- ============================================================================

ALTER TABLE operation_state
  ADD COLUMN IF NOT EXISTS engagement_org_id UUID;
