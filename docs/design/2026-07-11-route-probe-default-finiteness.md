# Route Probe 默认有限队列设计

## 背景

一次真实 Enumeration 运行在 `route_probe_paths` 使用缺省
`wordlist_recursion_depth=3` 时，生成了约 52 MiB 的 checkpoint，其中仍有
242,416 个 pending candidate，并记录 `candidate_generation_limited=true`。
按照现有终态契约，候选生成被截断时 `queue_completed=false`，因此 DIR 只能保持
`partial`。继续恢复同一 checkpoint 只能消费已经持久化的巨大队列，不能证明候选空间已
完整闭合。

根因不是 checkpoint 恢复失败，而是默认计划本身没有实用的有限性：约 1,882 条去重
字典项在每个 verified directory hit 下再次展开，默认三层会形成高分支树。单 root
队列已经可能触达 250,000 candidate 硬上限，更不适合 50-root batch 和 372-root
stage worklist。

## 决策

1. `wordlist_recursion_depth` 缺省值从 `3` 改为 `0`。
2. 缺省计划仍执行：
   - target-bound browser/JS/API/directory observed paths；
   - observed path 的 parent-prefix curated probes；
   - 本地、workspace 或 built-in wordlist 在 exact-origin root 的一次完整探测。
3. 缺省计划不再因为 root wordlist 的 positive directory hit 自动把整份 wordlist
   展开到子目录。
4. 调用方仍可显式传 `wordlist_recursion_depth=1..6` opt in。显式递归继续受
   `MAX_REQUESTS`、runtime、request budget、same-origin、dangerous-route、baseline
   verification 和 checkpoint 约束。
5. 显式递归若触发 candidate-generation limit，现有语义不变：
   `candidate_generation_limited=true`、`queue_completed=false`、DIR outcome 为
   `partial`，不得发布 `found` 或 checked-empty。

## Checkpoint 与兼容性

递归深度属于 route plan hash。缺省值改变后，旧的缺省-depth-3 checkpoint 与新的
depth-0 plan 不匹配，必须被现有 identity/plan 校验拒绝并清理。这是有意行为：新的
completion-oriented 调用不应继承已知不能闭合的巨大递归队列。

调用方若显式选择旧深度，仍然是在主动选择该 plan；工具不会把该选择静默降为 0。
所有 owner、run、session、operation、stage attempt、exact-origin 和 plan identity
约束保持不变。

## Agent 契约

- Enumerator 的标准 completion flow 省略 `wordlist_recursion_depth`，使用默认 0。
- 只有明确需要子目录字典扩展时才传 1..6，并把它视为可能返回 non-terminal
  `partial` 的有界 opt-in。
- `queue_completed=true` 仍是 DIR terminal 的必要条件；业务发现行本身不关闭 coverage。
- 不引入外部 dir fuzzer，也不改变 route probe 的 GET-only、exact-origin 和 evidence
  ownership 边界。

## 验证

- 单测证明省略参数解析为 0，显式 1..6 不被改写。
- dry-run HTTP 测试证明缺省计划会跑 root wordlist，但不会展开 positive child wordlist。
- 既有递归测试继续证明显式 depth 仍可展开。
- queue/candidate-generation/terminal 回归测试继续证明未闭合队列只能是 `partial`。
- schema、Enumerator prompt、stage methodology/spec 与模块卡使用同一契约措辞。
