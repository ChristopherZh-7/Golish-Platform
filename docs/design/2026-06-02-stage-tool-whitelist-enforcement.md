# 2026-06-02 · 阶段工具白名单强制（含子代理 + pentest_run 壳工具解析）

> 把「某一关只能用某些工具」的边界，从「charter 里写一句别 dig」的散文黑名单，升级成 **runtime 在所有执行路径上的确定性强制**——包括子代理路径，以及 `pentest_run` / `run_pty_cmd` 这种「壳工具」里真正跑的 CLI 工具。
>
> 关联：`pre_action_authorizer.rs`（每 tool call 授权）、`task_orchestrator/prompts/mod.rs`（charter）、`golish-sub-agents/executor`（子代理执行路）、`resources/harness/stages/*.json`（阶段 allowed/forbidden）。

---

## 1. 背景与问题（实测根因）

发「评估 example.com 外部攻击面」时，AI 在 **scoping** 关就跑了 `dig`/`httpx`/`nmap`（侦察工具），然后才说要进侦察关。scoping 明确禁止探测（`forbidden_tools: [dns_resolve, http_probe, subdomain_enum_passive, …]`），但越界没被拦。三层原因：

1. **子代理不认「阶段」**：实际跑扫描的是 `pentester` 子代理，它走 `golish-sub-agents::executor::dispatch_tool_calls`，那条路只按子代理**角色**的 `allowed_tools` 放工具（`tool_setup.rs`），**完全没有 harness 阶段检查**。阶段授权（`PreActionAuthorizer`）只在主 agent 路（`agentic_loop/turn/phases/tool_dispatch.rs`）跑。
2. **壳工具藏真名**：`forbidden_tools` 按工具名（`dns_resolve`）匹配，但 AI 跑的是 `pentest_run(tool_name="dig")`——名字看到的是外壳 `pentest_run`，看不到里层 `dig`。`run_pty_cmd("dig …")` 同理。
3. **散文黑名单不可扩展**：临时修法是在 charter 里写「别 dig/nmap/httpx…」，每来一个新工具就得加一句，丑且漏。

---

## 2. 不变量（本设计不改）

- **I-A**：阶段 gate 的判定逻辑（schema/scope/vacuous/freshness/evidence）不变。
- **I-B**：扫描关（external_attack_surface 等）必须能正常跑扫描——**不能因为强制白名单把所有扫描拦死**。
- **I-C**：禁止 charter 散文黑名单作为唯一边界；边界由 runtime 确定性强制。

---

## 3. 目标 / 非目标

**目标**
- G1：阶段的工具边界在**子代理路径**也被强制（不只是主 agent 路）。
- G2：`pentest_run` / `run_pty_cmd` 这种壳工具，按**里层真工具**判边界。
- G3：去掉 charter 里的散文黑名单，改由强制兜底。
- G4：不破坏扫描关（用 **forbidden-only** 强制，而非 allow-confinement，见 §5）。

**非目标**
- 不重构整套「canonical 名 vs 真实工具」命名体系（那是更大的活）；本设计用一张**小而显式的别名表**覆盖出现在 forbidden 列表里的探测工具即可。
- 不动 profile 的 `max_authorization` 授权阶梯（已有，正交）。

---

## 4. 命名调和：canonical_tool 解析器

新增纯函数（无 DB、可单测），把一次工具调用解析成**用于边界判定的规范名**：

```
canonical_tool(tool_name, args) -> String
```

规则：
- `pentest_run` → 取 `args.tool_name`（如 `nmap`/`dig`/`httpx`），再过别名表。
- `run_pty_cmd` → 取 `args.command` 的第一个 token，再过别名表。
- 其它 → 工具名本身（再过别名表，通常原样）。

**别名表**（覆盖出现在各关 forbidden 列表里的探测/危险工具；可维护、可扩展）：

| 真实工具 | canonical |
|---|---|
| dig, nslookup, host, dnsx, dnsrecon | `dns_resolve` |
| httpx, curl, wget, http, nmap, masscan, rustscan | `http_probe` |
| amass, subfinder, assetfinder, findomain, sublist3r | `subdomain_enum_passive` |
| whatweb, wappalyzer | `fingerprint_target` |
| sqlmap | `sqlmap`；metasploit/msfconsole → `metasploit` |

未命中别名表的工具 → 用原名（不会误伤；只有当它正好等于某关 forbidden 名时才拦）。

> 别名表是「设计活」的落点：它只需覆盖**会出现在 forbidden 列表里的**工具，不需要枚举所有 CLI。新增探测工具时，若要让某关禁它，往这表加一行即可。

放置：`golish-agent-kit/src/harness/tool_capability.rs`（harness 子模块，纯函数 + 单测）。

---

## 5. 强制策略：forbidden-only（关键安全取舍）

**为什么不用 allow-confinement（只准白名单）**：阶段 `allowed_tools` 用的是 canonical 名（`dns_resolve`），但扫描关里 agent 真跑的是 `pentest_run`（外壳，不在任何 allowed 列表）。若强制「不在 allowed 就拦」，会把**所有扫描全拦死**。

**采用 forbidden-only**：解析出 canonical 后，**只拦命中本关 `forbidden_tools` 的**。这恰好满足需求且安全：
- scoping 的 forbidden 含 `dns_resolve/http_probe/subdomain_enum_passive` → `pentest_run(dig)`→`dns_resolve`→**拦**；`pentest_run(nmap)`→`http_probe`→**拦**。scoping 越界被堵死。
- 扫描关（eas）forbidden 只含 `metasploit/sqlmap/credential_attack/…` → `pentest_run(dig)`→`dns_resolve`→**放行**（不在 eas forbidden）；`pentest_run(sqlmap)`→`sqlmap`→**拦**（危险工具被堵）。扫描不受影响。
- 非探测工具（manage_targets/record_finding/…）→ 原名 → 不在 forbidden → 放行。

> 即「白名单」在用户语义上 = 「这关不该干的事干不了」。用 forbidden-only + canonical 解析达成，且零破坏扫描。allow-confinement 留作未来命名体系统一后的增强。

---

## 6. 集成点（文件级落点）

| # | 改动 | 位置 |
|---|---|---|
| 6.1 | `canonical_tool` + 别名表 + 单测 | 新增 `golish-agent-kit/src/harness/tool_capability.rs` |
| 6.2 | 主 agent 路：授权前先 `canonical_tool` 解析（壳工具里层也被 forbidden/authz 查） | `agentic_loop/turn/phases/tool_dispatch.rs::gate_tool_call_for_dispatch` |
| 6.3 | 子代理路：执行前 forbidden-only 拦截 | `golish-sub-agents/executor/response_parsing.rs::dispatch_tool_calls` |
| 6.4 | 阶段策略传入子代理（避免 crate 依赖环：用 plain data / 闭包） | `SubAgentExecutorContext` 加 `stage_tool_guard: Option<Arc<dyn Fn(&str,&Value)->Result<(),String>>>`，由 `golish-agent-runtime` 在派子代理时按当前 stage 构造 |
| 6.5 | 去掉 charter 散文黑名单 | `task_orchestrator/prompts/mod.rs::stage_charter`（删 `stage_specific` 里那段「别 dig…」，保留「scoping 无需证据」语义） |

依赖方向：`golish-sub-agents` 不依赖 `golish-agent-kit`（避免环）→ 子代理路只持有一个**闭包**（plain `Fn`），解析+判定逻辑在 `golish-agent-runtime` 侧用 `canonical_tool` 构造闭包后注入。

---

## 7. 边界与风险

- **别名表不全**：未覆盖的探测工具在 scoping 不会被拦。缓解：表覆盖常见 recon 工具；未来可补。属「渐进收紧」，不破坏现有。
- **run_pty_cmd 命令解析**：只取第一个 token，复杂管道命令可能绕过。缓解：先覆盖直接调用；管道场景留 follow-up。
- **forbidden-only 不阻止「这关没列为 forbidden 的越界」**：如某关忘了把某探测加进 forbidden。缓解：scoping 等关 forbidden 已较全；后续可补「无探测关默认禁全部网络 canonical」开关。
- **风险等级**：中。动子代理执行路（核心）+ 主路授权。需单测 + 真跑验证扫描关不被误伤。

---

## 8. 验证计划（证据优先）

1. **单测**：`canonical_tool` 各分支（pentest_run/run_pty_cmd/别名/原名）；forbidden-only 判定（scoping 拦 dig、eas 放 dig、eas 拦 sqlmap）。
2. **子代理路单测**：dispatch 前 guard 拦截命中 forbidden 的调用，放行其它。
3. **真机**：开 dev，跑 example.com：scoping 不再出现 dig/httpx/nmap（被 runtime 拦或 AI 不再尝试）；扫描关 dig/httpx 正常跑。
4. `just test-harness` + `cargo clippy` 全绿。

---

## 9. 与已加临时修法的关系

- charter 散文「别 dig」(2026-06-02 stopgap) = 临时；本设计落地后**删除**，由 §5 强制兜底。
- scoping 的 gate 放宽（enforce_evidence_existence + scope_check 对 scoping 豁免）= 保留（正交，scoping 本就无证据）。
