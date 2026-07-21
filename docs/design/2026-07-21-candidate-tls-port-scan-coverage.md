# Candidate TLS 与端口扫描覆盖语义设计

> 端口发现中 `full=Nmap TCP 1-65535 XML` 与 `eas_port_scan_attestation_v1` 的新写入策略已由 `2026-07-21-eas-naabu-full-port-discovery.md` 取代；本文的Candidate/TLS、quick/standard partial、CIDR policy block与duplicate Candidate设计继续有效。

## 背景

Test1 的 Candidate 克隆运行证明了两处语义缺口：

1. `nuclei_match_v1` 已经有冻结 template id、matched URL 和 exact replay 执行器，但 Candidate 决策仍允许模型把可安全复现的 TLS 配置问题全部写成 `no_candidate`。因此弱密码套件、过时 TLS、自签名证书与证书身份不匹配没有进入 Verification。
2. `eas_discover_ports` 只接受一个整数 `top_ports`，默认执行 Naabu Top 1000；Nmap 服务识别随后只消费已确认开放端口。wrapper 返回和 evidence outcome 没有表达扫描覆盖档位，局部 Top-N 扫描可能被解释成完整端口结论。

本设计不针对任何单个端口加补丁。目标是让 Candidate 的 AI 判断有证据、有具体理由、有可执行验证方案，同时让 TCP 扫描覆盖成为可重放、可审计、确定性的客观事实。

## 目标

- TLS 是否形成 Candidate 仍由 AI 根据冻结 observation、目标上下文和前序 evidence 判断；程序不替代 AI 下漏洞结论。
- 弱密码套件、过时 TLS、自签名证书与证书身份不匹配默认应提出低优先级验证假设；AI 只有给出 evidence-backed 的具体例外理由时才能不建 Candidate，不能再用笼统的 `observation_not_exploitable` 丢弃。
- 纯 TLS 元数据通常以 `tls_metadata_only` 终结，但如果 AI 能提出有证据且可执行的安全假设，Gate 不硬编码禁止它成为 Candidate。
- 端口发现暴露 `quick`、`standard`、`full` 三种 scan profile，而不是让模型自由拼扫描器参数。
- 非 full profile 的 PORT 结果始终是 `partial`；即使发现开放端口，也只代表局部发现，不能关闭 PORT Gate。
- full profile 完整成功后才允许 PORT 为 `found` 或 `empty`。
- wrapper 返回扫描器、profile、TCP 覆盖范围、完整性和明确的下一步；精确命令继续进入 evidence/outcome，便于复查。
- 不修改数据库 schema/migration，不发起真实目标扫描。

## 非目标

- 不把端口号本身解释为漏洞或堡垒机。
- 不在 Candidate 阶段主动扫描；Candidate 仍是 reasoning-only。
- 不把 `tls-version`、证书颁发者、DNS SAN 或 wildcard 证书元数据提升为安全问题。
- 不在本轮重构既有 route-probe soft-404/cardinality 功能；该功能已有独立 passing feature。

## TLS Candidate 判断与闭环 Gate

### AI 的默认判断策略

当 observation schema 为 `nuclei_match_v1`、technique 为 `WSTG-CRYP-03` 且 template id 属于以下集合时，AI 默认提出 Candidate：

- `weak-cipher-suites`
- `deprecated-tls`
- `self-signed-ssl`
- `mismatched-ssl-certificate`

这些条目是安全相关 observation，不是已确认 Finding。AI 默认应形成“该精确 TLS 配置命中需要重放确认”的低优先级假设；Verification 仍执行已有 `verify.nuclei_template_replay`，最终可 verified、refuted、blocked 或 retryable_failed。

AI 可以判断不形成 Candidate，但必须同时满足：

- 引用该 work item 冻结的 evidence；
- reason code 必须精确属于 closed allowlist：`duplicate_candidate`、`evidence_stale`、`target_out_of_scope`、`replay_not_safe`、`context_refuted` 或 `observation_invalid`；任意同义新字符串都 fail closed；
- rationale 解释前序数据如何支持该例外。

`observation_not_exploitable`、`low_severity`、`informational` 这类仅凭标签、没有处理验证价值的笼统理由不能关闭上述 TLS security observation。

### 只作为元数据终结

以下 template id 通常以 `no_candidate` 和 reason code `tls_metadata_only` 终结：

- `tls-version`
- `ssl-issuer`
- `ssl-dns-names`
- `wildcard-tls`

如果这些元数据与其它冻结 evidence 共同形成了具体安全假设，AI 仍可创建 Candidate，且必须通过现有 classifier 生成 exact replay plan。其它 Nuclei template 保留现有逐 observation 决策。

### 确定性 Gate 的职责

Gate 不决定“有没有漏洞”，只确定性检查 AI 的输出是否闭环：

- 每个 frozen work item 恰好一个 decision；
- evidence refs 都来自该 work item；
- Candidate 有 hypothesis、rationale、允许的 technique 和服务器派生的 executable plan；
- no-candidate 有稳定 reason code 和 evidence-backed rationale；
- TLS security observation 不能用通用标签理由跳过。

### 大清单压缩与 Candidate 语义去重

真实克隆运行的冻结清单达到数十项，若要求模型复制每个 hash key，既浪费上下文，也容易在同一 Nuclei template 的多个 matcher 上生成重复 Candidate。`candidate_decision_groups` 因此增加 trusted `nuclei_template_ids` selector：它只会在服务器冻结 manifest 中精确展开已有 template id，并为每个展开项补回其自身 evidence。Candidate group 必须只选一个 template id，使 hypothesis 保持 template-specific；仅 metadata `no_candidate` group 可以合并多个 template。

压缩选择器不会放宽一项一决定 Gate。服务器在预检和最终 acceptance 使用同一语义身份：normalized target identity + target + technique + hypothesis。若两个 work item 会生成同一 Candidate identity，提交立即返回 `ATTACK_CANDIDATE_DUPLICATE_IDENTITY`，点名两条 exact work-item key；AI 必须保留一个 Candidate 并把其余项关闭为 `no_candidate/duplicate_candidate`，或提供真正不同且有证据支持的 hypothesis。这样重复问题在 durable submission/final seal 前可修复，不再依赖数据库唯一约束兜底。

### 风险与优先级

`WSTG-CRYP-03` 当前 verifier recipe 是只读、确定性安全重放，因此 risk class 保持 `deterministic_safe`，Candidate priority 保持 `low`。Verification 确认后的 Finding severity 由实际 template、目标身份和重放结果决定；Candidate priority 不等于最终 Finding severity。

## 端口扫描策略

### Profile

| profile | 固定引擎与端口范围 | 完整 PORT coverage |
|---|---|---|
| `quick` | Naabu Top 100 | 否 |
| `standard` | Naabu Top 1000 | 否 |
| `full` | Nmap TCP `1-65535` XML | 仅完整证明通过后 |

- wrapper 默认 `standard`，避免旧调用突然扩大为全端口主动扫描。
- 完成 EAS Gate 的 concrete IP worklist 必须显式使用 `full`。
- `full` 接受最多四个展开地址的 IPv4 `/30` 或更窄 CIDR，以及 exact IPv6 `/128`。
- 更宽的既有 CIDR 不发起网络请求；wrapper 以 exact target guard 写入 `eas_port_scan_policy_blocked_v1`，把 LIVENESS/PORT 收口为 evidence-backed `blocked`。需要真正全扫时，由 operator 先缩小授权范围。
- 后端只接受 `targets/scan_profile/background`。旧 `scanner/top_ports/ports/rate/timeout_secs` 即使绕过模型 schema 直接调用，也会在网络启动前拒绝；不存在 Masscan/自定义参数旁路。

### Outcome 语义

设 `open_count` 为当前 exact target 在本次工具输出中的开放端口数：

- PORT + incomplete profile：`partial(open_count)`。
- PORT + full：`found(open_count)` 或 `empty(0)`。
- LIVENESS + incomplete profile：发现 host/open port 时可 `found`；未发现时为 `partial`，不能声称主机不活跃。
- LIVENESS + full：沿用完整扫描的 `found/empty` 终态。
- runner、landing、authorization 或 evidence 持久化失败：仍是 `error/partial`，不得因 profile 覆盖而提升。

EAS Gate 已经在重加 terminal cell 前移除 raw business-table EAS facts，并只接受 fresh `technique_outcomes` 的 `found|empty`。因此让 incomplete PORT outcome 留在 `partial` 即可阻止 Gate 被 `targets.ports` 的局部命中旁路，不需要新增数据库列。

### 返回契约

`eas_discover_ports` 在原返回之外增加：

```json
{
  "scan_coverage": {
    "schema": "eas_port_scan_coverage_v1",
    "profile": "standard",
    "scanner": "naabu",
    "protocol": "tcp",
    "port_scope": "top-1000",
    "complete": false,
    "complete_for_gate": false,
    "target_manifest_sha256": "sha256:...",
    "expanded_host_count": 1
  },
  "per_target": [{
    "target_id": "...",
    "input": "192.0.2.10",
    "state": "partial",
    "ports_per_host": 1000,
    "open_port_count": 2,
    "open_endpoints": [
      {"ip": "192.0.2.10", "port": 80, "protocol": "tcp"},
      {"ip": "192.0.2.10", "port": 443, "protocol": "tcp"}
    ],
    "evidence_ids": [101, 102]
  }],
  "next_action": "Run eas_discover_ports with scan_profile=full for the remaining concrete IP PORT cells before Gate submission."
}
```

`full` 只有真实 process launch、exit 0、未截断输出、Nmap runstats success、每个 manifest host 的 open/closed/filtered accounting 合计 65535、landing 和 guarded evidence 全部成功时 `complete=true`。`-Pn reason=user-set` 只作为扫描 manifest，不落 `host_up`，也不创建 CIDR child。

每个端口 evidence 的 raw payload 改为版本化 `eas_port_scan_attestation_v1`，冻结 profile/version、canonical expanded-host manifest、fixed recipe、exit/truncation、目标身份和 observed counts，并内嵌原 scanner stdout/stderr。Gate 端从 `audit_log.detail.raw_output` 独立重算 manifest hash并重解析 XML；生产者自报 `complete=true` 不能单独过 Gate。后续 quick/standard 的 partial publication使用 monotonic guarded upsert，不能降级已有 full terminal truth。

## 安全边界

- Agent 仍不能传 raw Nmap/Naabu CLI args。
- full profile 只扩展已冻结、已授权的 concrete IP，不从扫描结果自行扩大 scope。
- service fingerprint 仍只消费数据库中确认开放的端口，不能扩标。
- 超预算 CIDR full scan 在 wrapper 层、网络启动前写 evidence-backed policy block；不新增大网段审批旁路，也不静默缩小用户授权范围。
- TLS Candidate 的 model text 不能改变 target、template、matched URL、technique、budget 或 executor contract；AI 只拥有假设和有证据的决策权。

## 验证

- `golish-agent-kit` 纯函数测试覆盖 actionable TLS 的 Candidate、具体例外 no-candidate、通用理由拒绝、metadata-only 和普通 Nuclei 决策。
- `golish-pentest-app` 纯函数测试覆盖三个 profile 的命令、CIDR/full 拒绝、partial/full outcome 与返回字段。
- 更新 Prober 方法学与模块卡，确保模型用 profile 而不是 `top_ports` 完成 Gate。
- 只运行受影响 crate/测试的定向 nextest、scoped Clippy、rustfmt 与 diff/JSON 检查；不运行 `init.sh`、全仓测试或 `precommit`。
- 在 `golish_gatefix_20260720_d` 克隆库用 CLI immutable-source fork 实跑 Candidate：确认大清单 selector、typed duplicate repair、最终 Gate PASS 与 proposed-only review 边界；不批准 Candidate、不创建 Attempt、不访问目标。
