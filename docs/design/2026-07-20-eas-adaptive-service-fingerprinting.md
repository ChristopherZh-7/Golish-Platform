# EAS 自适应服务指纹调度设计

## 1. 问题与现场证据

当前 `eas_fingerprint_services` 把模型给出的目标按相同端口集合分组后顺序执行
`nmap -sV -T3`。一次真实 EAS run 中，9 个 IP 被拆成 6 批；三个拥有 57–58 个
开放端口的 IP 分别占满 300 秒后超时，后续正常批次只能排队。Agent 再从 evidence
反推三个缺口，把同一批目标和全部端口原样重试为 600 秒；其中一个目标约 493 秒才
完成。

对同一已授权目标的直接诊断证明端口状态本身并不慢：58 个端口的 `-sT` 约 1 秒，
`-sV --version-intensity 0` 约 49 秒，`--version-intensity 2` 约 71 秒，而当前完整
`-sV -T3` 约 493 秒。尾延迟主要来自默认版本探针库，不是开放端口确认。

代码还存在四个确定性缺口：

1. 多批次用串行 `for + await`，形成队头阻塞。
2. `timeout_secs` 由模型决定，只能机械地 300→600。
3. coverage 已给出 `missing_open_ports`，wrapper 却用全部 DB 开放端口覆盖请求子集。
4. Nmap 文本 SERVICE 列无法区分 `method=table` 的端口表猜测与真实 probe，`smtp?`
   一类弱结果可能被当作强指纹；命令超时则整个 stdout 被拒绝落地。

## 2. 目标

- Agent 一次提交 concrete IP targets；后端独占分片、并发、deadline、降级和有限恢复。
- 小目标不会被慢目标阻塞；每个成功分片立即按原有 target guard 落业务行。
- 只扫描当前 DB 中真正未完成的 confirmed-open ports；调用者只能收窄，不能扩标。
- 每个端口至少获得一次有界版本探测尝试；强指纹和弱尝试严格区分。
- timeout/cancel/runtime failure 保持 `partial/error`，绝不伪装 `checked_empty`。
- 返回每目标 `completed_ports`、`strong_ports`、`weak_ports`、`remaining_ports`、attempt
  与 recovery 状态，让模型只刷新 coverage，不再猜超时或原样重放。

非目标：本次不改 schema/migration，不改变 Target/Organization scope，不新增主动扫描
能力，不把 EAS 通用化成新的全仓 executor，也不修改 Gate 的最终确定性权限。

## 3. 权威端口计划

`golish-db::coverage_truth` 新增只读 `service_fingerprint_port_plan_for_assets`：

- `confirmed_ports` 来自当前 in-scope target 的 open、非 DNS/53 `ports[]`。
- `pending_ports` 是 confirmed ports 减去本次 active EAS epoch 内、同 target/project、
  exact port 的 fresh Nmap fingerprint/attempt marker。`targets.ports[]` 内嵌 service
  没有 per-port observation timestamp，不能单独用来跳过本轮探测，否则 fresh port merge
  可能把上一轮旧 service 带进本轮并与 Gate freshness 漂移。
- organization、project_path、scope 与 target value 都必须精确匹配。
- 调用者传 `ports` 时只做 `pending ∩ requested`；不得把 DB 端口重新扩大，也不得把
  caller 端口扩进 confirmed set。
- 没有 caller ports 时使用全部 pending；pending 为空的 target 直接返回
  `already_complete`，不启动网络进程。

## 4. 服务器所有的执行策略

### 4.1 分片与并发

- 初始按单 target、最多 16 ports 分片；不再按相同端口集合合并多个 target。
- 全局固定最多 3 个 Nmap 子进程，按端口数、target、首端口稳定排序，小任务优先；
  并发只跨 target，同一 target 的 chunks 顺序执行，避免 sibling landing 互相使
  TargetWriteGuard 的 ports witness 失效。
- 每个分片继续走 `PentestRunTool::execute_guarded`，在最后 spawn seam 重验 target
  witness；落地时继续走 exact authorization refresh。

### 4.2 Nmap 模式

初始 fast pass：

```text
-n -Pn -sV --version-intensity 2 -T4 --max-retries 1
--host-timeout <server-budget>s -oX - -iL {input_file} -p <chunk>
```

外层 foreground deadline 比 Nmap host deadline 多一个很小的 kill/reap 余量。模型可见
schema 不再暴露 `timeout_secs`；兼容传入值也不参与执行预算。

若 fast chunk 没产出完整端口记录，后端最多进行一次 recovery：把该 chunk 缩为最多
4 ports，改用 `--version-intensity 0` 和更短 deadline。没有第三次自动重放。

若 fast 已覆盖全部端口但只有少量弱结果，最多对 8 个弱端口做一次 bounded deep
enrichment；大量统一弱响应不做全端口深扫，避免 CDN/Edge 或 tarpitting surface 再次
制造分钟级尾延迟。fast 弱尝试本身仍是合法 terminal attempt，不发明产品/版本。

### 4.3 增量落地

- 每个格式完整、未截断的 XML chunk 独立解析并立即写 target ports、network endpoint 和
  fingerprint rows。
- 命令非零/timeout 时，只有能完整解析的 host/port records 才允许落地；不完整 XML、
  truncated stdout 或无法精确归属的记录一律不终态化。
- 分片不单独覆盖 asset-level technique outcome；全部分片结束后按 target 聚合一次 guarded
  evidence/outcome，防止一个 weak chunk 把另一个 found chunk降级。
- `remaining_ports` 非空时 aggregate outcome 为 `partial`；全部端口已尝试且至少一个强
  指纹为 `found`；全部只得到弱尝试时为 evidence-backed `empty`，Gate 仍从 per-port
  Nmap attempt marker核对完整覆盖。

## 5. 结构化 Nmap 合同

`golish-pentest::output_parser` 在 trusted nmap 输出以 XML 开头时解析：

- host address/status；port protocol/state；
- service `name/product/version/extrainfo/tunnel/method/conf`；
- 每条 port record 继承 exact IP host。

强指纹要求 open port 且满足以下之一：

- `service_method=probed` 且 service 不是 pseudo value；或
- XML 有非空 product/version/extrainfo。

`method=table`、文本服务名尾随 `?`、`tcpwrapped/unknown/open/...` 均为弱尝试，只写
`service_attempt`，不写 `service` fingerprint。evidence 保留 method/conf 和 raw fields。
现有文本解析继续兼容普通 raw nmap/历史输出，但同样把尾随 `?` 视为弱值。

## 6. 模型与 coverage 合同

- Prober prompt / methodology / StageRefiner 改为“一次 wrapper 调用，后端拥有分片和恢复”。
- 删除“尽量少 wrapper batch、相同端口集合由模型分组、partial 后模型调大 timeout”的
  指令。
- coverage 继续返回 exact `missing_open_ports`，但推荐参数无需要求模型计算 timeout。
- wrapper 结果明确给出 `network_jobs` 和 per-target 状态；模型在 partial 后刷新 worklist，
  只在服务端仍返回 remaining ports 时发起新的用户级调用。

## 7. 安全与失败语义

- I2/I7：每个 chunk 的 launch/landing/evidence 都绑定现有 immutable TargetWriteGuard。
- I8：缺少输出、超时、取消、截断和 parser failure 只能是 partial/error。
- 不在事务中运行 Nmap；DB 事务仍仅做短 guard/landing/upsert。
- 固定并发和有限 recovery 防止进程/端口耗尽；Stop 继续由 task-local cancellation
  kill+await child。
- 本设计不需要 DB migration；若后续要做 durable跨进程 job状态，另立设计并重新取得
  schema授权。
