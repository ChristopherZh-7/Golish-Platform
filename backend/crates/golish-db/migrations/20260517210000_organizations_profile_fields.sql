-- ============================================================================
-- organizations → 甲方资产情报库
--
-- 把 `organizations` 从「只有一个名字」升级为 HVV 攻击方需要的资产情报库。
-- 18 个新字段：MVP 10 个进 5-tab UI（基础/域名/网络/范围/其他），剩余 8 个
-- 二期补 UI（先打基础避免后续重复 schema 变动）。
--
-- 设计来源：会话 bajie-mcp-agent-2-sukoeliv 中按「中国平安 HVV」场景倒推。
-- 配合 `docs/design/2026-05-17-targets-organization-grouping.md` 补充节。
--
-- 字段语义：
--   aliases         别名/简称/英文 → AI 模糊匹配 target 归属
--   industry        行业 → 选 POC 类型
--   tier            critical/high/medium/low → 优先级
--   credit_code     统一社会信用代码 → 工商查询
--   domains         [{domain,wildcard:bool,note}] → 子域爆破基线 + 自动归属
--   ip_ranges       ["1.2.3.0/24",...] → IP 收敛 + scope 判断
--   asns            ["AS12345",...] → whois/BGP 反查
--   email_domains   ["pingan.com",...] → 钓鱼/凭证泄露查询
--   scope_rules     {in:[],out:[],forbid_time:[],forbid_paths:[]} → 授权边界
--   intel           自由 JSONB → 兜底扩展
--   notes           Markdown 备注
--   ... 二期 8 个均为 JSONB[] 默认 '[]'，UI 后做
--
-- 所有字段 NOT NULL 配 sensible default，避免后端模型读取时遇 NULL 崩溃。
-- ============================================================================

ALTER TABLE organizations
  -- 基础 tab
  ADD COLUMN IF NOT EXISTS aliases          TEXT[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS industry         TEXT   NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS tier             TEXT   NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS credit_code      TEXT   NOT NULL DEFAULT '',
  -- 域名 tab
  ADD COLUMN IF NOT EXISTS domains          JSONB  NOT NULL DEFAULT '[]'::jsonb,
  -- 网络 tab
  ADD COLUMN IF NOT EXISTS ip_ranges        JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS asns             JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS email_domains    JSONB  NOT NULL DEFAULT '[]'::jsonb,
  -- 范围 tab
  ADD COLUMN IF NOT EXISTS scope_rules      JSONB  NOT NULL DEFAULT '{}'::jsonb,
  -- 其他 tab
  ADD COLUMN IF NOT EXISTS intel            JSONB  NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS notes            TEXT   NOT NULL DEFAULT '',
  -- 二期字段（schema 一次到位，UI 后续 PR）
  ADD COLUMN IF NOT EXISTS certificates     JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS subsidiaries     JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS business_systems JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS cloud_assets     JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS github_orgs      JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS social_accounts  JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS historical_vulns JSONB  NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN IF NOT EXISTS contacts         JSONB  NOT NULL DEFAULT '[]'::jsonb;

-- GIN 索引：AI 后续做 target ↔ org 模糊匹配时按 alias / domain / ip_range 查询
-- 用得到，提前建好。
CREATE INDEX IF NOT EXISTS idx_orgs_aliases   ON organizations USING GIN (aliases);
CREATE INDEX IF NOT EXISTS idx_orgs_domains   ON organizations USING GIN (domains jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_orgs_ip_ranges ON organizations USING GIN (ip_ranges jsonb_path_ops);
