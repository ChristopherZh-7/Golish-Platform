# Investigation 工具库补齐与授权装载设计

> **状态**：Approved for implementation planning（用户于 2026-08-07 明确要求：如果现有工具库缺少 Investigation 所需渗透工具，应直接新增 `resources/toolsconfig/<tool-id>.json`，而不是只依赖 prompt 或方法论文本）
>
> **日期**：2026-08-07
>
> **配套计划**：[`../superpowers/plans/2026-08-02-rag-first-unified-investigation-stage.md`](../superpowers/plans/2026-08-02-rag-first-unified-investigation-stage.md)
>
> **授权边界**：本文批准工具缺口分析和工具配置实现规划；不授权当前轮提前激活统一 Investigation、不授权安装或执行新工具、不授权真实目标、外部 OAST/provider、credential、brute force、state-changing API fuzz、schema/migration、generated IPC 或 rollout。

## 1. 结论

Golish 当前有47份 `resources/toolsconfig/*.json`，已经覆盖通用Recon、端口与服务识别、目录枚举、Nuclei、SQLi、XSS、CMS、凭据攻击、AD基础工具和部分post-exploit，但不足以支撑统一Investigation对现代API、token、GraphQL、TLS/SSH、SMB/AD补充枚举、容器/IaC和blind OAST hypothesis的自动验证。

缺口不通过继续扩写prompt解决。实施时直接新增第一批10份Tool Manager JSON，让用户能在工具管理器里看到、安装和手动运行；同时由Investigation stage的host-owned catalog决定它们能否进入自动验证。工具存在不等于Agent获权，安装成功不等于coverage完成，CLI输出更不等于Finding成立。

## 2. 两层配置，职责不能混淆

### 2.1 Tool Manager inventory

`resources/toolsconfig/<tool-id>.json`继续使用现有`ToolConfigFile { tool: ToolConfig }`格式，负责：

- id、名称、描述、executable/runtime；
- macOS/Linux/Windows安装方式；
- UI参数和安全preset；
- 输出格式、produces类型和detector；
- Tool Manager展示、安装、环境探测和手动运行。

这一层不保存operation、organization、scope、credential、JIT、fuel或evidence authority。不得把隐藏授权参数写进工具JSON，也不得把`pentestPhase`扩成不存在的`investigation`值；新配置继续使用现有`enum/vuln_id/exploit/post_exploit/aux`语义标签。

### 2.2 Investigation admission

`resources/harness/stages/investigation/tool_catalog.json`引用Tool Manager id，并冻结：

```json
{
  "contract_version": "investigation_operator_tools_v1",
  "tools": [
    {
      "tool_config_id": "arjun",
      "capability": "http_parameter_discovery",
      "execution_class": "active_bounded",
      "default_availability": "jit_only",
      "target_kinds": ["web_origin", "endpoint"],
      "credential_mode": "none_or_exact_grant",
      "external_service": false,
      "terminal_truth": "typed_adapter_required"
    }
  ]
}
```

这一层只由host读取。Analysis worker没有target I/O工具；Verification specialist只提交typed intent；Prepared Action/JIT通过后，host根据exact operation/org/target/hypothesis/action ordinal编译一次调用，typed Operator才可执行。catalog不能授予scope，也不能替代启动前TargetWriteGuard、fuel、lease或output landing。

## 3. 第一批直接新增的10份工具JSON

| JSON / id | 上游 | 补齐能力 | execution class | 自动默认 |
|---|---|---|---|---|
| `arjun.json` / `arjun` | `s0md3v/Arjun` | 隐藏HTTP参数发现 | `active_bounded` | `jit_only` |
| `kiterunner.json` / `kiterunner` | `assetnote/kiterunner` | API route/wordlist枚举 | `active_bounded` | `jit_only` |
| `schemathesis.json` / `schemathesis` | `schemathesis/schemathesis` | OpenAPI/GraphQL schema驱动测试 | `stateful_fuzz` | `disabled`，明确方法/样例/JIT后启用 |
| `jwt-tool.json` / `jwt-tool` | `ticarpi/jwt_tool` | JWT decode、claim/algorithm/key hypothesis | `local_then_active` | 本地decode可用；tamper/crack为`jit_only` |
| `graphql-cop.json` / `graphql-cop` | `dolevf/graphql-cop` | GraphQL配置与常见弱点检查 | `active_bounded` | `jit_only` |
| `testssl-sh.json` / `testssl-sh` | `testssl/testssl.sh` | TLS协议、cipher和证书配置 | `network_observe` | exact endpoint下可自动 |
| `ssh-audit.json` / `ssh-audit` | `jtesta/ssh-audit` | SSH banner、算法和策略审计 | `network_observe` | exact host:port下可自动 |
| `enum4linux-ng.json` / `enum4linux-ng` | `cddmp/enum4linux-ng` | SMB/NetBIOS/AD枚举补口 | `active_bounded` | `jit_only` |
| `trivy.json` / `trivy` | `aquasecurity/trivy` | 本地filesystem/image/IaC漏洞与错误配置 | `local_or_registry_read` | 本地artifact可用；registry/credential为`jit_only` |
| `interactsh-client.json` / `interactsh-client` | `projectdiscovery/interactsh` | blind SSRF/XXE/RCE/OAST interaction | `external_oast` | `disabled`；PAUSE D + external service/JIT |

第一批不新增`feroxbuster`，因为现有`route_probe_paths + ffuf + gobuster`已覆盖目录枚举；不新增`paramspider`，因为现有`gau + waybackurls`覆盖被动URL来源，主动隐藏参数先由Arjun承担；不新增`x8`，避免第一版同时存在两个相同authority的主动参数scanner。出现可证明的coverage或可靠性缺口后，后两者进入后续exact-set评审。

Cloud/Kubernetes不是所有operation的默认工具面。`prowler/scout-suite/kube-bench/kube-hunter/kubescape/pacu/kerbrute`保留为profile-specific后续批次；没有cloud account/cluster/AD exact scope、credential authority与独立JIT时不进入通用Investigation。

## 4. 每份JSON的最低合同

每份新增配置必须满足：

1. `id`与文件名、catalog引用、executable探测名一致，全库唯一；
2. `pentestPhase`只使用`VALID_PENTEST_PHASES`，不得写`investigation`；
3. parent install method与每个平台override都指向官方upstream/package；不使用任意镜像、curl pipe shell或未记录fork；
4. `params`必须显式列出required target/schema/token/file，不能靠一个自由文本`args`绕过command builder；
5. preset只包含version、本地analysis或bounded只读/低速检查；不得预置brute force、password spray、destructive method、persistence、evasion、payload delivery或默认external callback；
6. 输出优先JSON/JSONL/SARIF；保存tool id/version/exit/args hash/raw artifact ref/parser result；secret/token/cookie只能作为credential ref，不能进入JSON preset、日志或evidence正文；
7. 第一版不写`db_action: finding_add`。没有typed adapter与Tool Truth七轴前，输出只能是raw observation/evidence candidate，不能直接成为Finding或checked-empty；
8. 安装验证与执行验证分开：JSON parse/normalize/validate通过不代表二进制可运行，必须有`--version`或等价本地smoke；
9. 对目标发包的preset即使存在于Tool Manager，也不自动进入Agent工具集；stage catalog、Prepared Action/JIT和typed Operator缺一不可；
10. license、upstream URL、审计revision/version和配置SHA-256写入实现证据。CyberStrike只作为coverage/methodology参考，不能把其AGPL代码或配置逐字复制进Golish。

## 5. Tool Truth与结果落点

每个新工具必须映射至少一个closed capability和technique outcome。一次执行分别记录：

```text
request authority
→ launch guard
→ process outcome
→ raw observation/artifact
→ parser/typed landing
→ evidence append
→ coverage disposition
```

以下情况都不能发布`checked_empty`：工具未安装、版本不兼容、参数/wordlist/schema缺失、scope guard拒绝、credential缺失、timeout、输出截断、parser失败、外部OAST不可用、只完成本地decode但没有执行active hypothesis。positive observation也只关闭与exact target member和technique相符的obligation，不能传播到兄弟asset或其它organization。

## 6. CyberStrike的使用边界

[CyberStrike](https://github.com/CyberStrikeus/CyberStrike)可用来核对能力覆盖：它明确覆盖JWT、SSRF、SSTI、race、request smuggling、cache poisoning、CORS、GraphQL、XXE、WebSocket、cloud/Kubernetes和AD等方法论，也强调按发现动态编排工具。Golish采用这些coverage categories和方法论检索思路，但保留自己的Tool Manager JSON、Tool Truth、evidence ledger、scope/JIT和typed Operator合同。

CyberStrike仓库按其AGPL-3.0许可和固定revision进入Methodology Corpus provenance；本文不授权复制其实现、内置payload、post-exploit脚本或签名skill正文。

## 7. 完成标准

- 10份JSON均通过production `ToolConfigFile` parse/normalize/validate，id/file/catalog exact set一致；
- Tool Manager可展示全部10项，缺runtime/executable时显示明确not-ready而不是假installed；
- 每项至少一个本地无目标smoke可重放；主动工具的target test只用本地fixture；
- Investigation cognitive roles仍看不到raw CLI、shell、HTTP或credential；
- 只有`network_observe` exact-target action可以在profile允许时自动，`active_bounded/local_then_active/stateful_fuzz/external_oast`按catalog要求JIT或disabled；
- 没有typed adapter/Tool Truth终态合同的工具不能产生Finding或terminal coverage；
- focused验证、exit code、upstream/license/version/hash和任何平台缺口写入`agent-progress.md`及feature evidence。

任一项缺失时，相关tool catalog member保持`contract_pending`或`disabled`；工具JSON可以存在于Tool Manager，但不得被统一Investigation自动调度。
