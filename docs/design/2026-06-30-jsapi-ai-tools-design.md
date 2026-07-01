# JS/API 收集与提取「工具内建 AI」设计（收集补全 + 提取确认落库）

> Date: 2026-06-30
> Status: draft（待用户审查）
> Related:
> - `docs/design/2026-06-09-js-api-extraction-ai-augmented.md`（AI-B 抽取 hybrid：regex 打底 + LLM 补难 case + 幻觉护栏；**已设计未落地**，本设计将其实现化并内建进工具）
> - `docs/design/2026-06-25-enumeration-js-api-collection.md`、`docs/design/2026-06-26-enumeration-deliverables-and-flow.md`（JS/API 收集与交付）
> - `docs/design/2026-06-29-enumeration-tool-boundary.md`（“AI 在编排层”取向；本设计在不破坏 stage 边界的前提下，把两段 AI 从编排层**下沉进工具**，属对该取向的局部修订）
> - 现状原型脚本：`scripts/js_api_ai_recipe_probe.mjs`（AI-A recipe 原型）、`scripts/js_api_pipeline_test.mjs`（AI-B filter / 全流程原型）、`scripts/browser_collect_js_api.mjs`（确定性收集器）
> - 工具/库：`backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs}`、`backend/crates/golish-js-analyzer`
> Invariants touched: I3（后端独立校验）、I7（证据可追溯）、I8（checked_empty ≠ unchecked）、I9（LLM 不入事务）

---

## 1. Problem / 背景

现状（本会话读码核对）：两支 JS/API 工具都是**纯确定性**，AI 只存在于工具**之外**。

- `browser_collect_js_api`（`pentest_bridge/browser_collect_js_api.rs` + `scripts/browser_collect_js_api.mjs`）：无头浏览器收集 JS + 观察 XHR/fetch，确定性。其 `ai_assist` 字段只是给外层 agent 的 “Collector Hints”，**不是工具内 LLM 调用**。
- `js_extract_apis`（`pentest_bridge/js_extract_apis.rs` + `golish-js-analyzer`）：纯 regex 抽 endpoint/secret/param。其 `ai_analysis` 字段只是给外层 agent 的 “Static Analysis Hints” + `param_hints` 由外层 agent 读文本后**喂回来**，工具内不调 LLM。

两条 AI 逻辑当前只活在**离线脚本**里：

- `js_api_ai_recipe_probe.mjs`：收集 → 若有 `ai_review_refs` → 直接 `fetch` DeepSeek 生 `recipe` → 带 recipe 再收一遍。（= AI-A：收集补全）
- `js_api_pipeline_test.mjs --ai-filter`：收集 → 静态抽取 → 把结果取样喂 DeepSeek 分类 real/test/noise/needs_followup + secret_triage。（= AI-B：提取确认/分流）

痛点（用户原话）：

1. “脚本抓不干净”——确定性收集对 minified / webpack 拼接 / 懒加载闭包不全时会漏；需要 AI **验证是否抓完、决定还要补抓哪些**。
2. 抓完后“获取 API / 敏感信息，不确定的找 AI 分析，没问题就落地”——提取端需要 AI 对**难 case** 补抽与确认。
3. “param 也是对应各个接口，先抓然后分析，AI 到文本中分析”——参数发现需要 AI **读 JS 文本**补 body/form 参数，而不是只靠外层 agent 回喂。
4. 这两段应**包装成两个工具**（工具自带 AI），不靠人工跑脚本、也不强依赖外层大 agent。

注：`2026-06-09-js-api-extraction-ai-augmented.md` 已为痛点 2/3（提取端 AI）做了完整 hybrid 设计（触发闸 + 切片 + 幻觉护栏 + `Endpoint.source`），但**代码未落地**；该设计明确假设“缺口不在抓不抓得到 JS”，故**未覆盖痛点 1（收集端 AI）**。本设计 = 实现 2026-06-09 的 AI-B + 新增 AI-A + 把两者内建进两工具。

## 2. Decision（TL;DR）

把两段自动 AI 焊进两个工具，使其自给自足，同时守住 I7/I8（落库证据始终可追溯到真实 JS/网络观察，AI 不无中生有）。

- **工具一 `browser_collect_js_api` + AI-A（收集补全）**：确定性收集照跑；若结果“不干净”（`closure_complete=false` / `recursive_queue_remaining>0` / `ai_review_refs_total>0` / 低产出 / `status=*_partial`），工具**内部**调一次受约束 LLM 生 bounded `recipe`（复用现有 `recipe` 入参契约），带 recipe 再收一遍，有界循环至干净或触顶。LLM **只建议抓哪些同源 URL/manifest/route**；实际抓取与落库仍由确定性 `.mjs` 完成，并再验 same-origin/limit。
- **工具二 `js_extract_apis` + AI-B（提取确认落库 + param）**：落地 2026-06-09 hybrid——regex 打底不变；对触发闸命中的难 case 文件 beautify+切片喂 LLM 补抽 `{method,path,auth}`，过**幻觉护栏**（path 最长字面子串须实锚 JS 源码）才采信；**param** 从“外层回喂 `param_hints`”升级为“工具内 AI 读相关文本补 body/form 参数”。AI 确认/补出的候选**才落库**，带 provenance。
- **两工具共性**：
  - 复用 app 既有 LLM provider（与 sub_agents 同一接入路径），**不写死 key、不在确定性 `.mjs` 里 raw fetch**。
  - AI 角色严格限定为“**对真实出现过的候选**做确认/补全/分流”，绝不发明 endpoint/param/secret。
  - 落库每行记来源：`source`（crawler / js_analysis）+ `ai_confirmed` / `ai_added`（布尔），可追溯。
  - 发 audit + `ai_call_trace` 一类事件。
  - **无 provider / key → 自动退化为纯确定性**（不报错、不阻断），与 2026-06-09 退化路径一致。
  - 确定性核心**永远先跑**；AI 是叠加层，可用 `ai`(默认随 provider 可用而开) 参数显式关闭。
- **不改 stage 边界**（守 2026-06-29）：收集仍只持久化同源 XHR/fetch；提取仍只读 captures 下的 JS；两者都不做 EAS liveness/port/service-fingerprint；第三方 URL 仍不自动入 scope。

## 3. 两工具设计

### 3.1 工具一：`browser_collect_js_api` + AI-A（收集补全）

LLM 编排放在 **Rust bridge 层**（`browser_collect_js_api.rs`），不放进 `.mjs`（保持收集器脚本确定性、可离线）：

```
execute():
  1. 跑 .mjs 第一遍（确定性，现状不变）→ result
  2. 若 ai 开 且 provider 可用 且 needs_more(result):
       compact = 精简信号(result.ai_assist.context: ai_review_refs / script_observations / recursive_errors)
       recipe  = LLM(系统prompt=“只建议补抓的同源 URL/manifest/route/chunk_pairs，
                     不发明、不重复已抓/已失败；最小集合；可判 needs_second_pass=false”)
       sanitize(recipe)  // 同源 + 长度/数量上限（复用 .mjs 既有 recipe 校验）
       若 needs_second_pass 且 recipe 有料 且 未超轮/预算:
           跑 .mjs 第二遍(--recipe-json recipe) → result2（合并进 captures）
           （有界循环，最多 N 轮 / 总时长上限）
  3. 落 api_endpoints（同现状，确定性 source='crawler'）；记 ai_recipe_rounds / ai_recipe_rationale 到 audit
```

`needs_more(result)` 触发条件（任一）：`closure_complete=false`、`recursive_queue_remaining>0`、`ai_review_refs_total>0`、`status ∈ {closure_partial,timeout_partial}`、或脚本数/产出过低。

护栏：

- LLM 只产 `recipe`（manifest_paths/script_urls/routes/click_texts/public_path/chunk_pairs），`.mjs` 已对 recipe 做同源与数量上限校验（见 `scripts/browser_collect_js_api.mjs` `safeStringArray/safeChunkPairs/resolveSameOriginUrl`）→ AI 不能借 recipe 越权抓站外。
- 轮次/时间/脚本三上限（参照 `js_api_pipeline_test.mjs` 的 `max_closure_rounds=8 / max_total_scripts / max_total_ms`）。
- I7：落库的 endpoint/script 仍来自**真实 fetch 响应**；AI 只影响“去抓哪些”，不影响“抓到什么算数”。

### 3.2 工具二：`js_extract_apis` + AI-B（提取确认落库 + param）

落地 2026-06-09 hybrid（其护栏设计原样采用），AI 调用放 Rust bridge（`js_extract_apis.rs`），`golish-js-analyzer` 仅扩 `Endpoint.source`：

```
execute():
  1. regex 抽取（现状不变）→ endpoints(source 内部标 regex) + signals(secrets/configs/rules)
  2. 若 ai 开 且 provider 可用:
       a) 难 case 补抽: 对触发闸命中的文件 beautify+关键词窗口切片 → LLM 吐 [{method,path,auth}]
          → 幻觉护栏(path 去 ${} 后最长字面子串≥6 须实锚该文件源码) → endpoints(ai_added=true, confidence×0.9)
       b) 不确定候选确认: 对低 confidence / 模板化 endpoint、待分类 secret/rule，
          AI 读 suggested_read_file_ranges 的源码片段 → 标 real/test/noise/needs_followup（ai_triage）
       c) param 补全: 对每个 endpoint，AI 读其调用点文本 → 补 body/form 参数名（不发明，须见于文本）
  3. 合并去重 regex∪ai by (method,path)（tie 取 regex）→ 仅“regex 命中”或“AI 确认/补出且锚回源码”的落库
       → api_endpoints(source='js_analysis', ai_confirmed/ai_added, params 合并)
       → js_analysis_results（带 source/triage 元数据）
```

触发闸（控成本，2026-06-09 §3.2）：低产出（endpoints/KB < 0.2）、高度 minified（平均行长 > 2000）、suspect、run 预算 `MAX_AI_FILES=8 / MAX_AI_BYTES=256KB / MAX_CHUNKS=12`；`force_ai` 全开。

param 变更：现状 `param_hints` 由外层 agent 回喂（`js_extract_apis.rs` L100-122/L631-670）。AI-B 把这步**内化**：工具内 AI 读文本补参数；保留 `param_hints` 入参作为外层显式覆盖/兼容。落 `GOLISH-ENUM-PARAM` outcome 的口径不变（只有真补到参数才算 found）。

### 3.3 共用：LLM vehicle / provenance / 开关 / 退化

- **vehicle（实现前必核，见 §9-1）**：复用 sub_agents 的 provider 接入路径，把 provider/handle 注入两个 Tool（构造期或 agent 上下文）。**绝不**在确定性 `.mjs` 内 raw fetch DeepSeek。配置沿用 app 现有 AI 设置（不是脚本各自读 `~/.golish/settings.toml`）。
- **provenance**：新增/复用字段 `source`、`ai_confirmed`、`ai_added`、`ai_triage`（real/test/noise/needs_followup）、`ai_rationale`（短）；`Endpoint` 加 `#[serde(default)] source`（2026-06-09 §3.5，向后兼容）。
- **开关**：`ai`(bool，默认 true) 工具参数；provider 不可用时强制走确定性。
- **退化**：无 provider/超时/JSON 解析失败/锚不回 → 跳过该 AI 结果，确定性结果照常落库（宁漏不编）。

## 4. Data Flow

```
                ┌─────────────────────────── 工具一 browser_collect_js_api ───────────────────────────┐
target_url ──▶ │ .mjs 确定性收集(第1遍) ─▶ needs_more? ─是▶ LLM 生 recipe(同源/最小) ─▶ .mjs(带 recipe) │ ─┐
                │                              └─否─────────────────────────────────────────────▶ done │  │
                └───────────────────────────────────────────────────────────────────────────────────────┘  │
                                                                                                             ▼
                                                                          captures/{host}/{port}/js/*.js（真实抓到的 JS）
                                                                                                             │
                ┌─────────────────────────── 工具二 js_extract_apis ──────────────────────────────────────┐│
                │ regex 抽取(打底) ─▶ 触发闸? ─是▶ 切片→LLM 补抽/确认/补param ─▶ 幻觉护栏(锚回源码) ──┐      ││
                │                     └─否────────────────────────────────────────────────────────┐ │      ││
                │ 合并去重(regex∪ai, tie取regex) ◀───────────────────────────────────────────────┘─┘      ││
                └──────────────────────────────────────────────────┬──────────────────────────────────────┘│
                                                                    ▼                                        │
                          api_endpoints(source, ai_confirmed/ai_added, params) + js_analysis_results(triage) ◀┘
                                                                    │
                                                                    ▼  下游 auth_probe / enumeration coverage（契约不变）
```

## 5. I7 / I8 / I9 对齐

- **I7（证据可追溯）**：落库行始终来自真实 JS/网络观察；AI 仅“选择去抓哪些 + 确认/补全/分流真实出现过的候选”。每行带 `source` + `ai_*` provenance；AI-B 端点过硬锚校验（path 实锚源码）。
- **I8（checked_empty ≠ unchecked）**：AI 退化/跳过不得把“未检查”写成“已检查为空”；technique outcome（JSAPI/PARAM）口径不变，仍由确定性结果驱动 found/empty/error，AI 仅加 `ai_triage` 注记，不改 outcome 真值。
- **I9（事务）**：所有 LLM 调用在 DB 事务**之外**（抽取/收集与落库分离，沿用现有时序）。
- **I3**：不信 LLM 自报，AI 端点/param 必须锚回源码或来自真实响应。
- **stage 边界（2026-06-29）**：不新增越权工具行为；收集仍同源、提取仍只读 captures；不做 EAS 指纹；第三方 URL 不自动入 scope。

## 6. Files（预计改动，实现期细化）

| File | Change |
|---|---|
| `backend/crates/golish-js-analyzer/src/lib.rs` + `patterns.rs` | `Endpoint` 加 `#[serde(default)] source: EndpointSource{Regex,Ai}`；导出供 bridge 标注 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs` | AI-B pass：触发闸 + 切片 + LLM 补抽/确认 + 幻觉护栏 + param 内化 + 合并去重 + provenance 落库 |
| `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs` | AI-A：Rust 层 needs_more 判定 + LLM 生 recipe + 有界第二遍编排 + audit |
| LLM provider 注入（待定，§9-1） | 两个 Tool 构造/上下文取得 provider handle（复用 sub_agents 路径） |
| `scripts/browser_collect_js_api.mjs` | 不动 LLM（保持确定性）；如需，仅微调 `api_observed` 静态副档名过滤（cosmetic，独立小改） |
| `docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`golish-js-analyzer.md` | 模块卡同步（I6/§2.4） |
| `feature_list.json` / `agent-progress.md` | 新增功能条目 + 进度 |

无 DB schema 破坏性变更优先：`ai_*` provenance 尽量走既有 `api_endpoints.params` / `js_analysis_results.raw` JSON 字段承载；若需新列则按 I10 向后兼容迁移（实现期定）。

## 7. Verification（DoD）

```bash
cd backend && cargo nextest run -p golish-js-analyzer -p golish-pentest-app --status-level fail
cd backend && cargo clippy -p golish-js-analyzer -p golish-pentest-app --all-targets -- -D warnings
node --check scripts/browser_collect_js_api.mjs
# 活体（人工）：真实 minified SPA，hybrid 召回 > 纯 regex；收集 AI-A 把 closure 补到 complete
```

关键单测：

- AI-A：`needs_more` 各触发条件；recipe 同源/上限 sanitize；无 provider 退化纯收集；有界轮次不超限。
- AI-B：触发闸命中/预算耗尽；切片窗口+合并；**幻觉护栏**（编造端点被丢、真锚保留 source=ai）；regex∪ai 去重 tie 取 regex；param 内化（AI 补出的参数须见于文本）；`Endpoint.source` serde 向后兼容。
- I8：AI 退化时 outcome 不被写成 checked_empty。

## 8. 错误处理 / 回滚

- 无 provider/key → 纯确定性（不报错）。
- LLM 超时/报错/JSON 解析失败 → 跳过该 AI 结果，确定性照常。
- 幻觉锚不回 → 丢弃（宁漏不编）。
- 预算耗尽 → 停 AI，summary 标注，剩余走确定性。
- 回滚：`ai=false` 或无 provider 即退回今天的纯确定性行为，零副作用。

## 9. 开放问题 / 实现前必核 / Out of Scope

**实现前必核**

1.（必核·阻塞）**LLM vehicle**：bridge Tool 如何拿到 app 的 provider handle？需读 sub_agents 的 provider 接入路径，确认能注入 `BrowserCollectJsApiTool` / `JsExtractApisTool`（构造期 or `current_agent_*` 上下文）。这是落地前提，未确认前不进实现。

**拍板项**

2. 触发阈值默认值（沿用 2026-06-09：0.2 个/KB、2000 字符/行、MAX_AI_FILES=8、MAX_AI_BYTES=256KB、WINDOW=1500、MAX_CHUNKS=12）；AI-A 轮次/总预算默认值。
3. `ai` 默认 true（provider 可用即自动启用）是否符合预期，或默认 false 由调用方显式开。
4. provenance 落 JSON 字段 vs 新增列（倾向 JSON 承载，避免 schema 迁移）。

**Out of Scope（本期不做）**

- 完整 JS 反混淆 / AST 解释（仅 beautify + 关键词切片）。
- 把 hybrid 推广到 source map 重建 / wasm 端点。
- `auth_probe` 按 source 加权（后续）。
- 不改 `auth_probe` / `js_collect` 消费契约。

> 下一步：用户审查本设计 → 拍板 §9（尤其 9-1 vehicle）→ 用 writing-plans 出实现计划 `docs/superpowers/plans/2026-06-30-jsapi-ai-tools.md` → executing-plans（TDD：AI-A needs_more/recipe、AI-B 触发闸/切片/护栏/param 逐个红绿）。本设计独立新增，不覆盖旧文档（I6）。
