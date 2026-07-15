# Vuln 匿名访问 outcome 的 read-model / final-gate 对齐

## 背景

“广州有创网络科技有限公司”真实 CLI 链路在 `vuln_triage` 已生成并持久化：

- `vuln_probe_anonymous_access` evidence；
- `WSTG-ATHN-04 not_applicable` technique outcome；
- `stage_worklist_status = 10/10 done, ready_to_submit=true`。

但同一 operation 的 per-org final gate 仍报告 `WSTG-ATHN-04 never attempted`，导致 worker
重复提交和重复 producer 调用，最后由 stall guard 中止。

## 根因

`golish-agent-app` 的 guarded outcome projection 已正确要求：

- `WSTG-ATHN-04` source = `vuln_probe_anonymous_access`；
- evidence kind = `vuln.anonymous_access_observation`；
- current operation / organization / target / exact origin / positive evidence id 全部匹配。

但 `golish-agent-kit::harness::org_gate::vuln_outcome_source_is_trusted` 把除 N-day 外的所有
Vuln technique 都错误归到 `vuln_nuclei_general`。因此 final gate 在第二层 source 校验时
丢弃了合法的匿名访问 outcome，而 worklist read model 没有丢弃，两个确定性入口产生矛盾。

## 设计

1. final gate 的 producer/source matrix 明确拆分三类：
   - `GOLISH-NDAY` → `vuln_nuclei_fingerprint_targeted`；
   - `WSTG-ATHN-04` → `vuln_probe_anonymous_access`；
   - 其余八个公式化 WSTG technique → `vuln_nuclei_general`。
2. 不接受模型 coverage、自报 N/A 或其他 source；现有正 evidence id、freshness、current-owner
   和 exact-origin 校验保持不变。
3. `not_applicable` 继续通过 server-owned `GateContext.not_applicable_coverage` 关闭 cell，
   不伪装为 checked-empty。
4. focused gate fixture 使用真实生产者分工和 `coverage=[]`，确保 worklist 与 final gate 对同一
   DB truth 给出相同终态。

## 验证

- RED：dedicated wrapper 的 `WSTG-ATHN-04 not_applicable` 被 final gate helper 丢弃。
- GREEN：只接受 `vuln_probe_anonymous_access`，拒绝 `vuln_nuclei_general` 冒充该 technique。
- 运行 `golish-agent-kit` 的 `vuln_triage_` focused suite。
- 用新二进制恢复/重跑真实 operation，确认 Vuln gate PASS 并进入 Attack Candidate。
