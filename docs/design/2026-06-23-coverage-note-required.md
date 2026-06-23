# T1 · blocked/not_applicable 强校验 note + reason_kind

> 评审 claim #1。父：`docs/superpowers/plans/2026-06-22-...` 评审脉络 + `docs/design/2026-06-23-unified-gate-context-builder.md` §7.3。状态：进行中（2026-06-23）。

## 1. 问题

`CoverageStatus` 文档要求 CheckedEmpty/Blocked/NotApplicable「必须挂 note」(`harness/types.rs:188-193`)，但 gate 的 `other_ok`（`rule_engine.rs:624-627`，经 `cell_status` 闭包）**只看 status、不看 note**——注释自承「自报 + note 的判断态…**不收紧**」。后果：agent 可提交一个 `blocked` 空 note 的格蒙混过关（「我被挡了」却无任何理由/证据），与本平台「证据优先」（I7/I8）相悖。

## 2. 目标 / 非目标

**目标**：
1. **note 强校验**（用户「必须」）：`blocked` / `not_applicable` 终态格要求 `note` 非空，否则不算终态 → 缺口 → BLOCK。**gray-switch**（默认 off，逐字节不变；按 spec 翻开）。
2. **reason_kind**（用户「最好」）：给 `CoverageCell` 加结构化原因类别枚举 `ReasonKind`（provider_missing / credential_missing / rate_limited / tool_missing / out_of_scope / not_applicable），与自由文本 note 互补，便于审计/诊断分类。**可选元数据**（加字段 + 进 submit schema 让模型可填），本 PR **不**在 gate 强制（留作后续 toggle）。

**非目标**：不强制 reason_kind；不动 checked_empty/found 的判定（仍由 authoritative/derive 路径管）。

## 3. 设计

### 3.1 note 强校验（gray-switch = rule 配置字段）
`GateRule::CoverageComplete` 加 `#[serde(default)] require_note_for_other: bool`（与 `authoritative_found`/`derive_from_evidence` 同款 serde-default-false 灰度位）。透传进 `coverage_complete(...)`。`other_ok` 的 cell 命中闭包加 note 子句：

```rust
let cell_other_ok = |want| d.coverage.iter().any(|c|
    canon_asset(&c.asset)==asset_key && c.technique==*tech && c.status==want
    && (!require_note_for_other
        || c.note.as_deref().map(|n| !n.trim().is_empty()).unwrap_or(false)));
```

`require_note_for_other=false`（缺省）⇒ note 子句恒真 ⇒ 与现行 `cell_status` 逐字节一致。`true` ⇒ 空/缺 note 的 blocked/not_applicable 格不算终态 → 缺口 BLOCK。

**为何选 rule 配置而非 env flag**：与既有 coverage 灰度位（`host_aware_coverage` / `freshness_window` / `coverage_anchor_only`）同模式——gate 保持纯函数（DB-free / env-free），按 spec 逐阶段灰度翻开，零全局突变。

### 3.2 reason_kind（加字段）
`harness/types.rs`：新增 `ReasonKind` 枚举（`#[serde(rename_all="snake_case")]`）+ `CoverageCell.reason_kind: Option<ReasonKind>`（`#[serde(default)]`，加性、向后兼容；旧交付物/未填 → None）。`submit_stage_deliverable` 的 `parameters()` coverage cell schema 加 `reason_kind` 可选枚举，让模型在 blocked/not_applicable 时填类别。CoverageCell **非** ts-rs 导出（无前端类型链影响）。

## 4. 验证

- gate 单测：`require_note_for_other=false` → blocked 空 note 仍 Pass（逐字节不变）；`=true` → blocked 空 note BLOCK、blocked 带 note Pass、not_applicable 同理。
- reason_kind serde round-trip（snake_case）+ 缺省 None。
- submit schema：coverage cell 含 reason_kind 枚举（6 变体）。
- `cargo nextest -p golish-agent-kit -p golish-agent-app` 零回归（8 个 CoverageCell 字面量补 `reason_kind: None`）。
- `cargo clippy … -D warnings` 零告警。

## 5. 风险 / 回滚 / 激活

- 默认 off ⇒ 落地即**零行为变化**。
- **激活**（gray-switch 翻开）= 在目标 stage 的 `spec.json` 给 `coverage_complete` 规则加 `"require_note_for_other": true`——属 live gate 收紧（会把「blocked 空 note」的 run 翻 BLOCK），**本 PR 不翻任何 spec**，留给用户决定逐阶段灰度（与 perdim-freshness 翻 spec 同流程）。
- 回滚 = 去掉 spec 里的该字段。

## 6. 后续

- 可选 toggle：require reason_kind（不止 note）；gate 用 reason_kind 做更细的 recovery 提示（如 credential_missing → 提示配置凭证）。属 T1 的增量，非本 PR。
