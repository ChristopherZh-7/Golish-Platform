# 2026-06-05 · vuln_triage 技术矩阵（Technique Matrix）：外网 web 攻击阶段「该测什么 + 怎么算测过」的记分层

> **Status**: Discussion Draft（设计 only，不动代码 / 不改 `vuln_triage.json`，落地需用户点头 + 过 `just precommit` + 评审）
>
> **承接**：`docs/design/2026-06-05-coverage-matrix.md`（coverage 矩阵的数据结构 + gate 积木已落地 Phase 1.5）。本文是它的**内容侧补全**——把 `vuln_triage` 的 `expected_techniques` 从当前 4 个 WSTG id 扩成一份完整、可控、可校验的「外网 web 该测什么」清单，并为每一类定义「什么证据算测过」（= evidence 契约）。
>
> **关联文件**：`resources/harness/stages/vuln_triage.json`（`expected_techniques` + `gate_rules`）、`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`（`coverage_complete`）、`resources/harness/evidence_kinds.json`（evidence 时效）、`backend/crates/golish-agent-kit/src/harness/rag_prior.rs`（PoC/经验注入）。

---

## 1. 背景：现在 expected_techniques 太薄，但直接堆 WSTG 又会矩阵爆炸

`vuln_triage.json` 现在只声明 4 类期望技术：

```json
"expected_techniques": ["WSTG-INPV-05", "WSTG-INPV-01", "WSTG-ATHZ-04", "WSTG-INPV-19"]
```

即 SQLi / Reflected XSS / IDOR / SSRF。对真实外网渗透**远远不够**——真实流程至少包含：指纹→打对应 n-day PoC、目录扫描（敏感/未授权目录）、端口/服务口令爆破、接口未授权、逐接口注入测试等。

但**不能反过来把整本 WSTG（~100 个 test id）堆进 `expected_techniques`**。因为 coverage 矩阵是 `资产 × 技术`：技术维度一旦膨胀，矩阵单元格爆炸，`coverage_complete` 永远过不了，agent 被「逐格打勾」淹没（`coverage-matrix` 设计 §8「矩阵爆炸」已点名此风险）。

**本文的解法 = 区分三层（见 §2），让 `expected_techniques` 只承载「记分层」的有限技术类，把无上限的具体探针放进「执行层」。**

---

## 2. 核心模型：三层分离（执行 / 记分 / 证据）

「清单不够」与「矩阵爆炸」的矛盾，本质是把**记分**和**执行**混成了一件事。拆三层即化解：

| 层 | 是什么 | 数量 | 住在哪 |
|---|---|---|---|
| **执行层** | 具体探针：每个 n-day PoC、每条目录字典项、每个 SQLi payload、每个爆破口令 | **无上限** | PoC 库 / 字典 / payload 集 / 工具 |
| **记分层** | 技术**类** = coverage 矩阵的列（`expected_techniques`） | **~15，有限** | 本文定义的技术矩阵 |
| **证据层** | 每次执行层工具调用 → 1 条 evidence | 与执行层同量级 | evidence ledger（`audit_role='evidence'`） |

**不变量**：
- 矩阵（记分层）只回答「这一**类**技术，在这个资产上覆盖到没」，不为每个 payload / CVE 开一列。
- 执行层爱多大多大（PoC 库可上万条），矩阵列数恒定。
- 一个 cell 的终态由其覆盖的所有执行层探针**汇总**得出，并引用证据层的 `evidence_id`。

---

## 3. 技术矩阵（外网 web vuln_triage 的记分层）

15 个技术类。`id` 列：能对到 WSTG 叶子 id 的用叶子；属「家族」的用 WSTG 类别级；WSTG 没有的（n-day）用 `GOLISH-*` 自定义命名空间（`expected_techniques` MVP 是自由字符串，允许；词典化对标见 §7）。

| # | 技术类 | id | 执行层跑什么（举例） | 证据契约：什么算「测过」 | 适用资产 |
|---|---|---|---|---|---|
| 1 | SQL 注入 | `WSTG-INPV-05` | sqlmap / 手工 payload 逐接口逐参数 | found=注入成功的请求/响应或 sqlmap 命中报告；checked_empty=sqlmap 完整运行日志（证明扫过） | 带参 http_service |
| 2 | XSS（反射/存储/DOM） | `WSTG-INPV-01` | payload 注入回显点 + DOM sink 检查 | found=payload 在响应/DOM 执行的截图或响应体；checked_empty=注入点枚举 + 无回显证据 | 回显用户输入的 http_service |
| 3 | 命令/代码注入 | `WSTG-INPV-12` | OS command / eval payload | found=带外/回显命令执行证据；checked_empty=payload 集运行记录 | 调系统命令/eval 的功能 |
| 4 | 模板注入 SSTI | `WSTG-INPV-18` | 模板表达式探测（`{{7*7}}` 类） | found=表达式求值回显；checked_empty=探测响应记录 | 模板渲染用户输入处 |
| 5 | SSRF | `WSTG-INPV-19` | 受控 URL 指向探测器/内网 | found=探测器收到回连或内网响应；checked_empty=尝试记录 | 有外连/URL 抓取功能 |
| 6 | 越权 IDOR/BOLA | `WSTG-ATHZ-04` | 跨用户/跨对象替换标识符 | found=越权访问成功的响应对比；checked_empty=替换尝试的请求/响应对 | 带对象标识符参数的接口 |
| 7 | 路径穿越/LFI | `WSTG-ATHZ-01` | `../` 序列 / 文件包含 payload | found=任意文件读取响应；checked_empty=payload 运行记录 | 文件/路径参数 |
| 8 | 认证绕过 | `WSTG-ATHN-04` | 鉴权 schema 绕过 / 逻辑缺陷 | found=未认证访问受保护资源证据；checked_empty=绕过尝试记录 | 登录/鉴权入口 |
| 9 | 默认/弱口令（含爆破） | `WSTG-ATHN-02` | hydra/medusa 跑服务/后台口令 | found=命中凭据（脱敏）+ 登录成功证据；checked_empty=爆破尝试日志 | 任何带认证的服务/端口 |
| 10 | 会话与 CSRF | `WSTG-SESS-02` | cookie 属性检查 + CSRF PoC（SESS-05） | found=缺 HttpOnly/Secure 或 CSRF 成功 PoC；checked_empty=会话/请求检查记录 | 有会话/状态变更 |
| 11 | 敏感暴露/配置 | `WSTG-CONF-05` | 目录扫描（敏感/未授权/危险目录）、备份文件（CONF-04）、HTTP 方法（CONF-06）、安全头 | found=命中文件/目录/危险方法的 HTTP 响应；checked_empty=dirscan 完整结果 | http_service |
| 12 | 传输与敏感数据 | `WSTG-CRYP-03` | TLS 配置 + 明文传输检查 | found=明文敏感数据/弱 TLS 证据；checked_empty=TLS 扫描结果 | 对外服务/TLS |
| 13 | 业务逻辑滥用 | `WSTG-BUSL` | 跨接口替换、流程绕过、参数篡改（经验库套路注入此处） | found=逻辑被绕过的请求序列；checked_empty=尝试的逻辑路径记录 | 多接口/多步流程 |
| 14 | 信息泄露/版本 | `WSTG-INFO` | 报错信息、版本指纹、注释/元数据、调试端点 | found=泄露内容的响应；checked_empty=info-gathering 结果 | http_service |
| 15 | n-day / 已知 CVE | `GOLISH-NDAY` | 按指纹从 PoC 库挑对应 CVE PoC 全跑 | found=某 PoC 命中的请求/响应/判定；checked_empty=适用 PoC 全集运行记录 | 任何带指纹的资产 |

> 现有 4 个 id（INPV-05/01、ATHZ-04、INPV-19）是本矩阵的子集，迁移即「扩充」，不破坏当前行为。

---

## 4. 一个 cell 端到端：怎么测 + 怎么交证据

以 `(asset = api.example.com, technique = WSTG-INPV-05 SQLi)` 为例：

1. **测**：inner loop（PentAGI 式 subtask/refine）对该资产所有带参接口跑 sqlmap/payload —— N 次工具调用（执行层）。
2. **交证据**：每次工具调用 → 归一化成一条 evidence 入 ledger（`audit_role='evidence'`），得 `evidence_id`。**agent 引用 `evidence_id`，不把原始输出当自然语言贴进 deliverable**（防幻觉 + 防 prompt 注入，对齐 evidence-ledger 设计）。
3. **定终态**（写入 `CoverageCell.status`）：
   - 任一接口注入成功 → `found`，`evidence_refs = [成功那次的 id]`
   - 全跑了无果 → `checked_empty`，`evidence_refs = [证明确实扫过的 id，如 sqlmap 完整日志]`（I8：已检查为空 ≠ 未检查）
   - WAF 全挡 → `blocked` + note
   - 该资产无带参接口 → `not_applicable` + note
4. **gate 校验**（已落地）：`coverage_complete`（每资产 × 每期望技术都有终态，零 `not_attempted`）+ `for_all over coverage where status∈{found,checked_empty} require non_empty evidence_refs`。

n-day（`GOLISH-NDAY`）同构：指纹=Struts2.5 → 从 PoC 库拉适用 CVE PoC 全跑；命中=`found`，跑完无中=`checked_empty`，evidence=各 PoC run 记录。

> ⚠ **但「跑完无中 → checked_empty」必须对着分母才成立**——只「工具跑过」不等于「面测全」。对一个有 5000 个接口的资产，跑了 3 个接口就标 `checked_empty` 是**假全面**。这个洞与修复见 §5。

---

## 5. 面覆盖 / 分母：cell 内部的完整性（2026-06-05 补 · 修正一个真漏洞）

> **动机**：§4 的 cell 终态（found/checked_empty）只回答「这技术在这资产上**跑没跑**」，把**资产内部的「面」塌缩没了**。对一个有 5000 个接口的资产，跑了 sqlmap 3 个接口就标 `checked_empty` = **假全面**——正是 I8「已检查为空 ≠ 未检查」在更细粒度上重演。
>
> 注意：现有 `gate/surface_coverage_check.rs` **救不了**这个——它管的是 recon 阶段的**类别覆盖**（Surface/JsApi/Sitemap 有无证据，见 `surface_mapping.rs`），不是 vuln 阶段的**分母覆盖**（M 个接口测了几个）。

### 5.1 缺的维度：分母

完整性必须对着**一个已知的「面清单」**量，而不是对着「工具跑过没」量。

- **分母 M** = enumeration/active_recon 阶段爬出的**可测单元全集**（接口 / 参数 / 路径 / 服务），作为证据存着。
- 一个 `(资产 × 技术)` cell 的完整性 = `tested N / total M`，不是裸 found/checked_empty。

### 5.2 cell 加「覆盖账」+ partial 语义

扩 `CoverageCell`（**这是对 coverage-matrix 数据模型的真实扩展，非纯文档**）：

```rust
pub struct CoverageCell {
    // ... 原有 asset / technique / status / evidence_refs / note
    pub tested_units: u32,                    // N：实际测过的单元数
    pub total_units: u32,                     // M：该(资产×技术)的可测单元分母（来自 enumeration）
    pub sampling_rationale: Option<String>,   // 抽样时必填
}
```

终态合法性收紧：
- `checked_empty` 只在 `N ≥ 阈值 × M` 且无发现时合法；
- `found` 仍合法（找到即有价值），但应记 `N/M` 反映还覆盖了多少面；
- `N` 不足阈值且未声明抽样 → 视为 `partial`（没测完）→ gate **Block**。

### 5.3 gate 按分母校验（扩 coverage_complete）

**默认 = 全覆盖（D6 已定 2026-06-05）**：每个期望 `(资产 × 技术)` 要求 `tested == total` 才算完整。

**抽样 = 显式例外**：仅当单元格填了 `sampling_rationale` 时，才允许 `tested / total ≥ 声明阈值` 通过（给长尾 / 超大面留活路，但必须留痕说明为何抽样）。

两者都不满足（`tested < total` 且无 `sampling_rationale`）= `partial` = Block，reason 附 `N/M` 与缺口数。

### 5.4 两条诚实底线

- **认知极限**：分母只能是「**已发现**的面」。只能对爬到的 M 覆盖；没爬到的接口是 enumeration 的责任 + **永远承认的 gap**，不能假装不存在。完整性永远是「相对已发现面」的完整性。
- **务实现实**：`5000 × 15` 不可能穷尽。真渗透 = **风险优先 + 诚实声明抽样**（高危接口全测、长尾抽样，把「测了哪个子集、抽样率多少」写进证据）。**被禁止的不是「抽样」，是「3/5000 却谎称 checked_empty」**。

### 5.5 待拍板（本节新增）

- **D6 ✅ 已定（2026-06-05）= 默认全覆盖**：每个期望 `(资产×技术)` 默认 `tested == total`。抽样不是默认，而是**显式例外**——单元格须填 `sampling_rationale` 才允许 `tested < total`，否则 `partial` → Block。
- **D7 ✅ 已定（2026-06-05）= 分母从 enumeration**：M（接口/参数清单）来自 enumeration 阶段的爬取产物，vuln 阶段经 `inherits_evidence_from` 继承。推论：enumeration 爬得越全，vuln「全覆盖」越有意义；爬漏的不在 vuln 分母（是 enumeration 的责任）。
- **D8 ✅ 已定（2026-06-05）= MVP 静态**：覆盖率阈值放 `stage_spec`（静态）；Phase 2 改 skeleton 按资产动态。
  - **实现状态（Phase 2 ①③ seam · 2026-06-05）**：①（资产从 DB）+③（skeleton 动态 expected）的 **gate 侧 seam 已预埋**：`rule_engine::GateContext { in_scope_assets, expected_techniques }` + `eval_with_context` + `gate::validate_stage_gate_with_context` + `StageSkeleton.expected_techniques`（均加性、默认 None=旧行为，mock 测覆盖）。`coverage_complete` 资产维度优先取 `ctx.in_scope_assets`、期望技术优先取 `ctx.expected_techniques`（否则 skeleton，否则 spec）。**仅剩活体接线 deferred**：① 阶段收尾外层查 DB 注入 in-scope 资产；③ 扩 `DefaultSprintContractGenerator` 按真实目标/资产产 `skeleton.expected_techniques`——待资产库合入 + DB §2.7，届时只改外层调用方、gate 纯函数零改。

---

## 6. 矩阵爆炸控制（资产 / 技术维度）

矩阵格数 = `资产数 × 期望技术数`。控制策略：

1. **期望技术固定 ~15 类**（本文），不随 payload/CVE 增长。
2. **适用性收窄**：不是每类对每资产都要 `found/checked_empty`。MVP 允许大方使用 `not_applicable + note`（如某资产无外连 → SSRF 标 n_a）。
3. **Phase 2 条件化**（deferred）：按资产指纹 / 类型动态裁剪期望技术（接 `coverage-matrix` 设计 §6.5 的 skeleton 动态生成），让矩阵只列「对该资产真适用」的技术，进一步缩格。
4. **长尾靠 AI 拓展**：矩阵是「地板」（最低必测），不是「天花板」。AI 在 inner loop 里可自由追加矩阵外的 `findings` 与新 `coverage` 格（`technique` 字段自由字符串），gate 不拦——只拦「漏了期望技术」和「无证据」。

---

## 7. 与现有 harness 的对接点

| 对接 | 现状 | 本文动作 |
|---|---|---|
| `expected_techniques`（记分层列） | 4 个 id | 扩成 §3 的 15 类 |
| `coverage_complete`（完整性闸） | 已落地（自报资产版） | 直接复用，无需改 Rust |
| found/checked_empty 证据规则 | 已落地 | 直接复用 |
| evidence 契约（§3 末列） | 未显式定义 | 本文给出每类「什么证据算数」；落 `evidence_kinds.json` / charter 提示 |
| n-day ↔ PoC 库 | `rag_prior` 已能检索 wiki/PoC | `GOLISH-NDAY` 类把 PoC 库正式纳入记分 |
| 资产维度 | 自报（Phase 1.5） | 维持自报；Phase 2 从 DB 注入（待资产库 + DB 授权） |
| 面覆盖 / 分母（§5） | **缺**（cell 无分母字段） | 扩 `CoverageCell` + `coverage_complete` 按分母核覆盖率（真代码改动） |

---

## 8. 开放问题（落地前需拍板）

> 完整决策清单 = 本节 D1-D5（技术维度）+ §5.5 的 D6-D8（面覆盖维度）。

- **D1 ✅ 已定（2026-06-05）= MVP 混用 id，Phase 2 词典统一**：叶子 / 类别级 / `GOLISH-*` 自定义混用先跑通；`coverage_complete` 不在乎 id 形态，词典化是过早优化。
  - **实现状态（Phase 2 ② · 2026-06-05）**：词典已落地为 `resources/harness/technique_taxonomy.json`（id→{name,standard} 注册表）+ `golish-agent-kit/src/harness/technique_taxonomy.rs`（`is_recognized` / `lookup`）。`coverage_complete` 仍对 id 形态不敏感（D1 不变）；词典作为**测试期 fail-closed 守卫**：`all_embedded_expected_techniques_are_recognized` 断言每个 stage spec 的 `expected_techniques` 都已登记，堵「拼错 WSTG id 造出永远覆盖不了的矩阵列」。新增技术类先登记再用。
- **D2 ✅ 已定 = `GOLISH-NDAY` + PoC 选取不进契约**：n-day 类 id = `GOLISH-NDAY`；`fingerprint→PoC 选取`逻辑留执行层（agent/工具），契约只管「什么证据算测过」，写死会僵。
- **D3 ✅ 已定 = 两者都落**：evidence 契约同时落 `evidence_kinds.json`（kind 给 gate 机器校验）+ charter 提示（自然语言给 agent 指导）；一个管硬判定、一个管怎么做。
- **D4 ✅ 已定 = BUSL 防口袋**：BUSL 的 `checked_empty` 必须挂「试过哪些逻辑路径」的证据，且鼓励经验库套路驱动，防止「啥都标空」退回假全面。
- **D5 ✅ 已定 = MVP 静态适用性**：静态全列 + 大方 `not_applicable + note`；Phase 2 按资产指纹动态裁剪（待同事扫描/指纹数据合并）。

---

## 9. 落地次序（设计获批后）

1. 本草案评审 → 调整 §3 清单到团队实际 + 拍板 D1-D8。
2. 把 15 类填进 `resources/harness/stages/vuln_triage.json` 的 `expected_techniques`；charter 提示补「每类 found/checked_empty 的证据契约」（小改 + 配 gate 集成测试）。
3. **扩 `CoverageCell`（tested/total + 抽样理由）+ `coverage_complete` 按分母核覆盖率（§5）**——coverage-matrix 数据模型的真实扩展，独立 commit + 测试。
4. evidence 契约落 `evidence_kinds.json`（如需新 kind）。
5. mock 资产集端到端验证 `coverage_complete` + 分母覆盖率（不依赖真实资产库 / DB）。
6. `just precommit` 全绿 + 证据记 `agent-progress.md` + `feature_list.json` 登记。

> 第 2-5 步是 Golish 代码改动，按 `AGENTS.md` 规矩走（测试 + 评审 + precommit）。本草案本身（第 1 步）零代码、零风险。
