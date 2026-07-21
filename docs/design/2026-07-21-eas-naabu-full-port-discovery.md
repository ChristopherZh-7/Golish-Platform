# EAS Naabu 全端口发现与 Nmap 定向指纹设计

> 本文取代 `2026-07-21-candidate-tls-port-scan-coverage.md` 中仅关于 `full=Nmap TCP 1-65535 XML` 的端口发现与 v1 attestation 设计；Candidate/TLS、quick/standard partial、CIDR policy block 和 duplicate Candidate 设计继续有效。

## 背景与实跑根因

GUI run `pentest-chat-1784598026928-1` 已证明当前 full recipe 存在确定性运行时冲突：

- `EasDiscoverPortsTool` 为 full 声明 `timeout_secs=1900`，底层命令是 Nmap TCP Connect `-p- --max-rate 500 --host-timeout 30m`。
- `PentestRunTool` 对所有工具统一执行 `min(requested, 600s)`。
- 4-host 与 2-host full 调用均在约 600 秒被 `COMMAND_TIMEOUT` 杀死，只留下 XML 头，`parsed=0`、`stored=0`、`outcome_persisted=false`。
- Agent 随后退回 Naabu Top-1000，但 standard 依法只能发布 partial，无法关闭 PORT Gate。

四个 host 共 262140 个 TCP 端口，500/s 的理论调度下限已约 524 秒，尚未计算 Connect timeout、重试和 XML 收尾；因此单纯把 Nmap timeout 放大只能修正“被过早杀死”，不能解决扫描慢和失败后整批结果归零。

## 目标

- `full` 改由 Naabu 非特权 TCP Connect 完整扫描 `1-65535`，后端固定范围、速率、timeout、retry、verify 和 list-file recipe。
- Nmap 不再负责发现全部端口；现有 `eas_fingerprint_services` 只消费数据库中已确认开放端口，继续用定向 `-sV` 识别服务与版本。
- full 的 wrapper deadline 与通用 runner 的 600 秒上限一致，不再声明一个运行时不可能兑现的 1900 秒合同。
- quick/standard 仍是 partial；Naabu full 只有真实 guarded launch、成功退出、完整未截断输出、exact manifest/recipe 和 guarded landing/evidence 全部成立时，才发布 PORT `found|empty`。
- Gate read side 独立验证新的 v2 attestation；旧的、已经落账的 Nmap v1 attestation 继续按原严格 XML accounting 规则读取，避免历史事实失效。
- 不改数据库 schema/migration，不扩大授权 target，不允许模型传 raw scanner 参数，不主动修改或重启当前 GUI run。

## 固定扫描流水线

### 端口发现

| profile | 引擎 | 固定范围 | 速率 | Gate 语义 |
|---|---|---|---|---|
| `quick` | Naabu Connect | Top 100 | 200/s | partial |
| `standard` | Naabu Connect | Top 1000 | 500/s | partial |
| `full` | Naabu Connect | TCP `1-65535` | 1000/s | v2 完整证明后 terminal |

full 的 logical recipe 固定为：

```text
naabu -list {input_file} -iv <4|6> -p 1-65535 -s c -rate 1000 -timeout 1000 -retries 1 -verify -silent -no-stdin
```

- 继续保留每批最多四个 expanded host、IPv4 `/30` 或更窄、IPv6 exact `/128` 的现有边界。
- runner input file 写入排序后的exact expanded IP列表，不把CIDR字符串直接交给scanner；因此network/broadcast样式地址不会因为底层CIDR便利语义被静默跳过，input与attested manifest全等。
- full wrapper timeout 固定 600 秒，与 runner cap 一致。按四 host、1000/s 计算，纯端口调度约 262 秒，保留约两倍余量给 Connect/verify/进程收尾。
- `-silent` stdout 只允许 canonical `IP:port` / `[IPv6]:port` 开放端点；任一无法解析或不在 exact expanded-host manifest 内的非空行使完整证明失败。
- 空 stdout 在 process 成功、未截断、fixed recipe 和 manifest 都通过时表示“完整扫描未发现开放端口”，而不是“未检查”。

### 服务指纹

`eas_fingerprint_services` 保持现有职责：从 exact org/project/current-EAS worklist 读取 confirmed-open ports，按 IP/端口小分片执行 Nmap `-sV`，强/弱 service 与 closed attempt 语义不变。端口发现结果不会携带 `/etc/services` 猜测，也不能直接关闭 SERVICE-FINGERPRINT。

## `eas_port_scan_attestation_v2`

新的 full evidence raw payload 使用 v2：

```json
{
  "schema": "eas_port_scan_attestation_v2",
  "profile": {
    "schema": "eas_port_scan_coverage_v2",
    "profile": "full",
    "profile_version": 2,
    "scanner": "naabu",
    "protocol": "tcp",
    "port_scope": "tcp-1-65535",
    "complete": true,
    "complete_for_gate": true,
    "timeout_secs": 600
  },
  "batch_manifest": {
    "sha256": "sha256:...",
    "expanded_hosts": ["192.0.2.10"]
  },
  "coverage_receipt": [{
    "host": "192.0.2.10",
    "first_port": 1,
    "last_port": 65535,
    "scheduled_port_count": 65535,
    "completed": true
  }],
  "execution": {
    "network_launched": true,
    "command": "naabu ...",
    "exit_code": 0,
    "stdout_truncated": false,
    "stderr_truncated": false,
    "target_manifest_complete": true,
    "scanner_stdout_sha256": "sha256:..."
  },
  "scanner_stdout": "192.0.2.10:443\n"
}
```

`coverage_receipt` 是服务器根据冻结的 expanded-host manifest 和 fixed `1-65535` recipe 生成的执行收据，不是模型文本，也不声称 Naabu会输出 closed/filtered 逐端口行。Gate 独立重算：

1. producer/source/kind 必须为 `eas_discover_ports` / `naabu` / `eas.discover_ports`；
2. schema/profile/version/scanner/range/timeout 必须精确匹配 v2；
3. expanded hosts 非空、同地址族、最多四个，数量字段与 exact receipt host 集合全等；
4. command 必须精确等于 server-owned Naabu recipe，manifest hash 必须按 v2 domain separator 重算一致；
5. guarded network launch、exit 0、无 stdout/stderr truncation、manifest complete 全部为真；
6. stdout hash重算一致，每个非空 stdout 行都能解析为 manifest 内的 TCP endpoint；
7. target id/requested identity 继续受现有 target-bound evidence 与 current owner/scope 校验。

任一漂移都不投影 terminal PORT truth。quick/standard 仍写 partial，不进入 terminal attestation 验证。

## 兼容与失败语义

- 旧 `eas_port_scan_attestation_v1` 只有 `tool=nmap` 且 XML runstats success、逐 host accounting=65535、manifest/hash/target/launch/exit/truncation 全部通过时继续有效。
- 新 terminal evidence 只接受 `tool=naabu + v2`；不能用 Naabu output 伪装 v1，也不能把 Nmap stdout 塞进 v2。
- Naabu缺失、非零退出、超时、取消、stdout/stderr截断、未识别输出、foreign endpoint、landing失败或evidence写失败均保持 partial/error，不生成 terminal outcome。
- full 成功后若开放端口为零，PORT 为 `empty`，LIVENESS沿用现有完整扫描语义；若有开放端口，PORT/LIVENESS为 `found`。
- 后续 quick/standard 的 monotonic guarded upsert 不能降级已有 terminal full truth。

## 安全边界

- 模型只传 `targets` 与 `scan_profile`，不能传端口范围、速率、扫描类型、timeout、retry、raw args 或底层 scanner。
- 使用 `-s c`，不要求 root/raw socket；不切换到更隐蔽或更高权限的 SYN 模式。
- 仍在真实 process spawn 前重验 exact target write guards；CIDR child 只能由现有 containment 规则产生。
- 全批1000/s是server-owned上限；不新增无界并发或大网段旁路。
- Nmap只消费confirmed-open ports，不能根据常见端口号猜服务，也不能扩大为新的全端口扫描。

## 定向验证

- `golish-pentest-app`：三个 profile recipe、full 600 秒合同、Naabu空结果/开放结果/foreign或畸形行/非零退出/截断、per-target endpoint、partial不terminal、CIDR边界。
- `golish-agent-app`：新 v2 attestation接受、recipe/manifest/receipt/hash/stdout/target/producer任一漂移拒绝，并保留严格Nmap v1读取兼容。
- `golish-agent-kit` / `golish-sub-agents`：repair与Prober文案继续要求full完成PORT，并明确服务指纹只针对confirmed-open ports。
- 运行受影响crate的focused nextest、scoped Clippy、rustfmt、JSON与diff检查；未获授权不运行init/precommit/全workspace测试，也不为验收主动扫描外部目标。
