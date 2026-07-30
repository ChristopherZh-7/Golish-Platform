# Plan A Tool Truth Coverage Contract Schema Amendment v2 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 修正 Plan A 的 execution identity、租户/范围 authority、Evidence/Business lineage 与数据库防篡改模型，使唯一 migration 在未来获得再次授权后能够用复合外键和 direct-SQL 负向测试证明不存在跨 operation、organization、scope、stage 或 worker attempt 的拼接。

**架构：** `tool_truth_execution_authorities` 作为每个 denominator、policy、receipt 和 reconciliation 的 immutable authority spine；stage-owned execution 与 worker-owned execution 使用互斥的 closed shape。既有 `audit_log` 保持不变，Evidence 与 canonical business rows 分别经 normalized immutable adapter 接入同一 execution authority；所有 exact-set header 使用统一 open→members→seal 状态机，所有 event 使用 append-only trigger。

**技术栈：** PostgreSQL、sqlx、Rust 2021、UUID/BIGINT typed identity、canonical JSON、SHA-256、cargo-nextest、隔离的 embedded PostgreSQL。

---

## 文档地位与当前停止线

- 本文是 `docs/superpowers/plans/2026-07-29-tool-truth-coverage-contract.md` 的规范性 schema 修订；二者冲突时以本文为准。
- 原Plan A Task 2 Step 3的DDL code fence仅保留历史决策背景，禁止复制执行；未来00005必须按本文重写其execution keys、adapter、compound FK、hash、seal与trigger。
- 本文只修订 Plan A。Plan B、Plan C、Plan D、frontend/generated IPC、reporting rollout 与 promotion 均不在范围内。
- Task 1 的提交 `389440d3 feat(tool-truth): add coverage status ontology` 保留，不重写、不回滚。
- Task 2 的 RED tests 保持未提交；它们不能与共享 Application Model 改动一起暂存。
- 本文提交时尚未创建、修改或运行 `backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql`。用户已在2026-07-30明确授权技术负责人按本文与仓库事实完成Plan A唯一migration、repo、trigger、constraint及隔离定向测试；文档提交后直接继续Task 2–11。
- 未来仍只允许使用 `20260729000005_capability_execution_receipts.sql`；不得修改已有 migration，不得新增第二个 migration，不得访问 Test1、生产或其他现存用户数据库。
- deployment default 固定为 `legacy_v1`。Plan A 不提供 setter、promotion、联合 rollout 或 rollout repair 入口。

## 本次裁决替换表

| 旧 Plan A 形状 | Schema Amendment v2 |
|---|---|
| denominator、destination policy、receipt 使用 `TIMESTAMPTZ attempt_epoch` | 删除这些列；stage attempt 的唯一 identity 是 `stage_execution_id UUID` |
| worker 与 host execution 共用模糊 attempt 字段 | `execution_owner_kind='host_stage'` 与 `execution_owner_kind='worker_tool'` 为互斥 closed shape |
| worker attempt 可只凭 epoch 或 receipt 自报 | 必须整体绑定 `worker_run_id + worker_attempt_epoch BIGINT + lease_token + source_tool_call_id` |
| receipt 直接重复 operation/org/stage 字段 | 先绑定 immutable `tool_truth_execution_authorities`，再以 authority id + authority compound tuple 绑定所有下游行 |
| `stage_asset_waves(id)` 单列 FK | 新增 immutable `tool_truth_stage_wave_execution_bindings`，把 wave 固定到 project/scope/org/stage execution/stage kind |
| stage unit authority 不含 scope snapshot | 为 `stage_run_units` 增加包含 `scope_snapshot_id` 的 compound UNIQUE，并由 spine 使用完整 FK |
| 假设 `audit_log` 已有 ownership/hash compound key | 不改 `audit_log`；新增 `tool_truth_evidence_authorities` normalized adapter |
| Evidence 与 business ref 共用自由形 JSON/ref kind | 新增独立 `tool_truth_business_ref_authorities`，使用闭集 kind 与 typed UUID/BIGINT id |
| hash 列混用裸 64 hex 与 `sha256:` 前缀 | 所有 Tool Truth hash 使用 `sha256:<64 lowercase hex>` |
| repo-only immutability/seal | DB trigger 强制 rollout guard、open→members→seal、append-only、NULL-safe shape 与 exact parent binding |

---

## 真实 schema 基线

未来实现必须以这些已存在的 authority 为基线，不得假设额外列已经存在：

- `operation_state(operation_id, project_scope_id)` 已有 compound UNIQUE。
- `operation_org_scope_snapshots` 已有 `(id, operation_id)`，但缺 `(id, operation_id, project_scope_id)`；唯一 migration 必须 additive 增加后者。
- `operation_org_scope_units(snapshot_id, organization_id)` 已有 primary key。
- `stage_runs` 已有 `(id, operation_id, stage_kind)` compound UNIQUE。
- `stage_asset_waves` 只有 `id` 及 `(operation_id, organization_id, stage_kind, wave_index)`；它没有 `stage_execution_id` 或 `scope_snapshot_id`。
- `stage_run_units` 已有 `(id, operation_id, stage_execution_id, organization_id, stage_kind)`，但现有 compound key 不含 `scope_snapshot_id`。
- `stage_worker_runs.attempt_epoch` 与 `tool_calls.attempt_epoch` 均为 `BIGINT`；`tool_calls` 已冻结 worker、lease 与 stage/unit context。
- `audit_log.id` 是 `BIGINT`，`run_id` 是 nullable UUID，project identity 是 `project_path TEXT`，organization 仅存在于受约束的 `detail` payload；表中没有 project_scope UUID、organization UUID 或 evidence hash compound key。
- canonical business id 类型为：`target_assets.id UUID`、`dns_records.id BIGINT`、`web_origin_observations.id UUID`、`network_endpoints.id UUID`、`enumeration_endpoint_observations.id UUID`。

这些事实决定本文新增 normalized adapter，而不是扩充通用 `audit_log`。

---

## 统一 digest 与 canonical serialization

所有本文新增或修订的 Tool Truth hash 列统一使用：

```text
Sha256DigestV1 := "sha256:" + 64 个 lowercase hexadecimal 字符
PostgreSQL CHECK := value ~ '^sha256:[0-9a-f]{64}$'
```

禁止 bare 64-hex、uppercase、base64、可选前缀或多算法自由字符串。`content_key`、row hash、member hash、set hash、policy hash、authority hash、chain hash、source hash、artifact plaintext hash 与 ciphertext hash 均使用同一文本格式。

hash 输入统一为 canonical JSON UTF-8 bytes：

1. object key 按 UTF-8 byte lexical order 排序；
2. UUID 使用 lowercase hyphenated form；
3. BIGINT 使用十进制字符串且无前导零；
4. timestamp 使用 UTC microsecond precision，仅允许用于 observation/retention 时间，不参与 execution identity；
5. `null` 与字段缺失不可互换；
6. set hash 输入为按 canonical ordinal 排列的 member hash 数组；
7. authority hash 必须覆盖本文定义的完整 closed shape，包括所有 nullable discriminator 字段。

Rust 与 PostgreSQL golden corpus 必须共享至少以下拒绝样本：uppercase digest、bare hex、错误前缀、63/65 hex、重复 JSON key、非 canonical UUID、数字/字符串类型漂移与缺字段。

---

## Authority spine

### 1. 既有表 additive compound keys

唯一 migration 获批后只能 additive 增加：

```sql
ALTER TABLE operation_org_scope_snapshots
    ADD CONSTRAINT operation_scope_snapshot_execution_authority_unique
    UNIQUE(id, operation_id, project_scope_id, project_path_at_freeze);

ALTER TABLE stage_asset_waves
    ADD CONSTRAINT stage_asset_waves_tool_truth_authority_unique
    UNIQUE(id, operation_id, organization_id, stage_kind);

ALTER TABLE stage_run_units
    ADD CONSTRAINT stage_run_units_tool_truth_scope_authority_unique
    UNIQUE(
        id, operation_id, stage_execution_id, scope_snapshot_id,
        organization_id, stage_kind
    );

ALTER TABLE tool_calls
    ADD CONSTRAINT tool_calls_tool_truth_worker_authority_unique
    UNIQUE(
        id, operation_id, stage_execution_id, stage_run_unit_id,
        organization_id, worker_run_id, attempt_epoch, lease_token
    );
```

不得修改创建这些表的历史 migration。

### 2. Immutable stage-wave binding

`tool_truth_stage_wave_execution_bindings` 冻结旧 wave 缺失的 execution/scope 维度：

```sql
CREATE TABLE tool_truth_stage_wave_execution_bindings (
    id UUID PRIMARY KEY,
    stage_asset_wave_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL CHECK (BTRIM(project_path_at_freeze) <> ''),
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (BTRIM(stage_kind) <> ''),
    binding_hash TEXT NOT NULL CHECK (binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(
        id, operation_id, project_scope_id, project_path_at_freeze, scope_snapshot_id,
        organization_id, stage_execution_id, stage_kind, binding_hash
    ),
    FOREIGN KEY(operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id, operation_id, project_scope_id, project_path_at_freeze)
        REFERENCES operation_org_scope_snapshots(
            id, operation_id, project_scope_id, project_path_at_freeze
        )
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_execution_id, operation_id, stage_kind)
        REFERENCES stage_runs(id, operation_id, stage_kind) ON DELETE RESTRICT,
    FOREIGN KEY(stage_asset_wave_id, operation_id, organization_id, stage_kind)
        REFERENCES stage_asset_waves(id, operation_id, organization_id, stage_kind)
        ON DELETE RESTRICT
);
```

该表insert trigger必须`FOR SHARE`重读snapshot并要求`sealed_at IS NOT NULL`；随后拒绝UPDATE/DELETE。一个旧 wave 只能绑定一个 stage execution 和一个 sealed scope snapshot，不能在resume时改绑。

### 3. `tool_truth_execution_authorities`

```sql
CREATE TABLE tool_truth_execution_authorities (
    id UUID PRIMARY KEY,
    stable_authority_request_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL CHECK (BTRIM(project_path_at_freeze) <> ''),
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL CHECK (BTRIM(stage_kind) <> ''),

    execution_source_kind TEXT NOT NULL CHECK (
        execution_source_kind IN ('stage_execution','stage_wave','stage_unit')
    ),
    stage_wave_binding_id UUID,
    stage_wave_binding_hash TEXT CHECK (
        stage_wave_binding_hash IS NULL
        OR stage_wave_binding_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    stage_run_unit_id UUID,

    execution_owner_kind TEXT NOT NULL CHECK (
        execution_owner_kind IN ('host_stage','worker_tool')
    ),
    worker_run_id UUID,
    worker_attempt_epoch BIGINT,
    lease_token UUID,
    source_tool_call_id UUID,

    authority_hash TEXT NOT NULL CHECK (authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),

    UNIQUE(
        id, operation_id, project_scope_id, project_path_at_freeze, scope_snapshot_id,
        organization_id, stage_execution_id, stage_kind, authority_hash
    ),
    UNIQUE(operation_id, stable_authority_request_id),
    FOREIGN KEY(operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id, operation_id, project_scope_id, project_path_at_freeze)
        REFERENCES operation_org_scope_snapshots(
            id, operation_id, project_scope_id, project_path_at_freeze
        )
        ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(stage_execution_id, operation_id, stage_kind)
        REFERENCES stage_runs(id, operation_id, stage_kind) ON DELETE RESTRICT,
    CHECK (
        (execution_source_kind='stage_execution'
            AND stage_wave_binding_id IS NULL
            AND stage_wave_binding_hash IS NULL
            AND stage_run_unit_id IS NULL)
        OR (execution_source_kind='stage_wave'
            AND stage_wave_binding_id IS NOT NULL
            AND stage_wave_binding_hash IS NOT NULL
            AND stage_run_unit_id IS NULL)
        OR (execution_source_kind='stage_unit'
            AND stage_wave_binding_id IS NULL
            AND stage_wave_binding_hash IS NULL
            AND stage_run_unit_id IS NOT NULL)
    ),
    CHECK (
        (execution_owner_kind='host_stage'
            AND worker_run_id IS NULL
            AND worker_attempt_epoch IS NULL
            AND lease_token IS NULL
            AND source_tool_call_id IS NULL)
        OR (execution_owner_kind='worker_tool'
            AND execution_source_kind='stage_unit'
            AND worker_run_id IS NOT NULL
            AND worker_attempt_epoch IS NOT NULL
            AND worker_attempt_epoch >= 0
            AND lease_token IS NOT NULL
            AND source_tool_call_id IS NOT NULL)
    ),
    FOREIGN KEY(
        stage_wave_binding_id, operation_id, project_scope_id, project_path_at_freeze,
        scope_snapshot_id, organization_id, stage_execution_id, stage_kind,
        stage_wave_binding_hash
    ) REFERENCES tool_truth_stage_wave_execution_bindings(
        id, operation_id, project_scope_id, project_path_at_freeze, scope_snapshot_id,
        organization_id, stage_execution_id, stage_kind, binding_hash
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        stage_run_unit_id, operation_id, stage_execution_id, scope_snapshot_id,
        organization_id, stage_kind
    ) REFERENCES stage_run_units(
        id, operation_id, stage_execution_id, scope_snapshot_id,
        organization_id, stage_kind
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        source_tool_call_id, operation_id, stage_execution_id, stage_run_unit_id,
        organization_id, worker_run_id, worker_attempt_epoch, lease_token
    ) REFERENCES tool_calls(
        id, operation_id, stage_execution_id, stage_run_unit_id,
        organization_id, worker_run_id, attempt_epoch, lease_token
    ) ON DELETE RESTRICT
);
```

nullable source FK 使用PostgreSQL默认MATCH SIMPLE；对应的NULL-safe source-shape CHECK负责保证id/hash exact-all-or-none，因此非对应variant跳过FK、对应variant必须完整命中parent。禁止改成会让common non-null authority列造成partial-null失败的MATCH FULL。`authority_hash`始终是更外层完整spine hash，不能与`stage_wave_binding_hash`复用。

`stable_authority_request_id`由host生成并进入authority hash；同operation下exact replay返回既有row，payload drift拒绝。worker-owned authority使用source tool call推导稳定request id，host-stage authority使用stage dispatcher持久化的稳定request id。insert trigger先要求scope snapshot已经sealed，并由server重算完整`authority_hash`；insert后拒绝UPDATE/DELETE。worker row 的 mutable lease 不是长期 FK parent；immutable `tool_calls` row 才是 worker attempt fence。insert trigger 仍须在同一 transaction 中 `FOR SHARE` 重读 exact worker row，证明该 tool call 对应当时的 active lease，随后 authority 依赖 immutable tool-call tuple，worker 释放 lease不会破坏历史 authority。

### 4. Stage attempt 与 worker attempt 语义

- Stage attempt 只由 `stage_execution_id UUID` 标识。denominator、destination policy、receipt、reconciliation、authority seal 与 Gate assessment 不得再出现名为 `attempt_epoch` 的 timestamp。
- 多次 receipt begin 由 `(execution_authority_id, capability, attempt_ordinal)` 区分；`attempt_ordinal` 是 receipt-local positive integer，不是 stage identity。
- Worker attempt 只能出现在 `worker_tool` authority shape，字段名固定为 `worker_attempt_epoch BIGINT`。
- 任何含 worker attempt 的 row 必须通过 execution authority 间接绑定 exact `source_tool_call_id + worker_run_id + worker_attempt_epoch + lease_token`。
- host/stage-owned shape 四个 worker 字段全部为 NULL；worker-owned shape四个字段全部非NULL。PostgreSQL CHECK 必须显式写 `IS NULL/IS NOT NULL`，不能依赖 UNKNOWN 被拒绝。

---

## Evidence normalized adapter

不为 `audit_log` 增加operation/project/org/hash authority列。Plan A receipt evidence也不得先走通用audit writer、事后再认领；新增immutable production binding，并由一个module-private transaction entrypoint同时写terminal audit row、current classification与binding：

```sql
CREATE TABLE tool_truth_evidence_production_bindings (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    evidence_audit_id BIGINT NOT NULL UNIQUE
        REFERENCES audit_log(id) ON DELETE RESTRICT,
    evidence_classification_id BIGINT NOT NULL UNIQUE
        REFERENCES evidence_classifications(id) ON DELETE RESTRICT,
    production_binding_hash TEXT NOT NULL
        CHECK (production_binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id, execution_authority_id),
    FOREIGN KEY(
        execution_authority_id, operation_id, project_scope_id, project_path_at_freeze,
        scope_snapshot_id, organization_id, stage_execution_id,
        stage_kind, execution_authority_hash
    ) REFERENCES tool_truth_execution_authorities(
        id, operation_id, project_scope_id, project_path_at_freeze,
        scope_snapshot_id, organization_id, stage_execution_id,
        stage_kind, authority_hash
    ) ON DELETE RESTRICT
);
```

该entrypoint从execution authority生成audit detail内的closed producer envelope。`worker_tool` envelope必须包含并由DB重读exact `source_tool_call_id + worker_run_id + worker_attempt_epoch + lease_token`；`host_stage` envelope必须完全不含worker字段。它不得接受caller提供的operation/project/org/stage/worker ownership。binding存在后，trigger拒绝对应audit/classification/binding的UPDATE/DELETE。这样同stage/org的Evidence不能从worker A绑定给worker B；legacy通用audit row没有production binding，不能进入receipt-v1 authority。

最终normalized adapter只接受上述binding，不接受裸audit id：

```sql
CREATE TABLE tool_truth_evidence_authorities (
    id UUID PRIMARY KEY,
    production_binding_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    project_path_at_freeze TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    execution_authority_hash TEXT NOT NULL,
    evidence_audit_id BIGINT NOT NULL UNIQUE REFERENCES audit_log(id) ON DELETE RESTRICT,
    evidence_classification_id BIGINT NOT NULL
        REFERENCES evidence_classifications(id) ON DELETE RESTRICT,
    audit_row_hash TEXT NOT NULL CHECK (audit_row_hash ~ '^sha256:[0-9a-f]{64}$'),
    classification_row_hash TEXT NOT NULL
        CHECK (classification_row_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_chain_hash TEXT NOT NULL
        CHECK (evidence_chain_hash ~ '^sha256:[0-9a-f]{64}$'),
    authority_hash TEXT NOT NULL CHECK (authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id, execution_authority_id),
    UNIQUE(id, execution_authority_id, authority_hash),
    UNIQUE(execution_authority_id, evidence_audit_id),
    FOREIGN KEY(production_binding_id, execution_authority_id)
        REFERENCES tool_truth_evidence_production_bindings(id, execution_authority_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(
        execution_authority_id, operation_id, project_scope_id, project_path_at_freeze,
        scope_snapshot_id,
        organization_id, stage_execution_id, stage_kind, execution_authority_hash
    ) REFERENCES tool_truth_execution_authorities(
        id, operation_id, project_scope_id, project_path_at_freeze, scope_snapshot_id,
        organization_id, stage_execution_id, stage_kind, authority_hash
    ) ON DELETE RESTRICT
);
```

唯一可用的 repo insert 路径必须在短 transaction 中 server-side 重读并验证：

1. production binding中的audit/classification/execution authority必须与adapter exact一致；
2. `audit_log.id=evidence_audit_id` 且 `audit_role='evidence'`；
3. `audit_log.run_id=execution_authority.operation_id`；
4. `audit_log.project_path=operation_org_scope_snapshots.project_path_at_freeze`；不能用可能变化的ambient workspace path替代frozen path；
5. safe JSON decoder读取audit detail organization与producer envelope；缺失、错误类型、malformed UUID/BIGINT或不相等均以SQLSTATE `23514`映射为`tool_truth_evidence_producer_envelope_invalid`，不得泄漏`22P02`；
6. classification 的 `evidence_audit_id`相同、`valid_to IS NULL`、`classification='in_scope'`；
7. `producing_stage_run_id=stage_execution_id`；NULL或其他execution均拒绝；
8. worker-owned binding重读immutable tool-call compound tuple；host-stage binding确认worker envelope字段全缺失；
9. 从audit parent chain根到当前evidence的每一行重读id/parent/run/project/role/status/detail，并计算ordered `evidence_chain_hash`；断链、cycle、跨operation parent或caller hash漂移均拒绝；
10. `audit_row_hash`、`classification_row_hash`、`evidence_chain_hash`与adapter `authority_hash`只由server canonical serializer生成。

adapter row insert 后拒绝 UPDATE/DELETE。Evidence member 只存 `evidence_authority_id`，不得再由caller提交裸 `evidence_id + JSON ownership`。

---

## Business-ref normalized adapter

Evidence 与 canonical business reference 是两条独立 lineage；一个不能冒充另一个。新增：

```sql
CREATE TABLE tool_truth_business_ref_authorities (
    id UUID PRIMARY KEY,
    execution_authority_id UUID NOT NULL,
    evidence_authority_id UUID NOT NULL,
    ref_kind TEXT NOT NULL CHECK (ref_kind IN (
        'target_asset','dns_record','web_origin_observation',
        'network_endpoint','enumeration_endpoint_observation'
    )),
    ref_uuid UUID,
    ref_bigint BIGINT,
    snapshot_contract_version TEXT NOT NULL DEFAULT 'tool_truth_business_ref_snapshot.v1'
        CHECK (snapshot_contract_version='tool_truth_business_ref_snapshot.v1'),
    canonical_snapshot JSONB NOT NULL
        CHECK (jsonb_typeof(canonical_snapshot)='object'),
    source_observed_at TIMESTAMPTZ NOT NULL,
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    authority_hash TEXT NOT NULL CHECK (authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id, execution_authority_id),
    UNIQUE(id, execution_authority_id, authority_hash),
    UNIQUE(execution_authority_id, ref_kind, ref_uuid),
    UNIQUE(execution_authority_id, ref_kind, ref_bigint),
    FOREIGN KEY(evidence_authority_id, execution_authority_id)
        REFERENCES tool_truth_evidence_authorities(id, execution_authority_id)
        ON DELETE RESTRICT,
    CHECK (
        (ref_kind='dns_record'
            AND ref_bigint IS NOT NULL AND ref_bigint > 0 AND ref_uuid IS NULL)
        OR (ref_kind IN (
                'target_asset','web_origin_observation',
                'network_endpoint','enumeration_endpoint_observation'
            ) AND ref_uuid IS NOT NULL AND ref_bigint IS NULL)
    )
);
```

两条 nullable-column UNIQUE 不能单独承担去重。实际 migration 还必须增加两个 partial UNIQUE index：UUID variants 按 `(execution_authority_id,ref_kind,ref_uuid) WHERE ref_uuid IS NOT NULL`，DNS 按 `(execution_authority_id,ref_kind,ref_bigint) WHERE ref_bigint IS NOT NULL`。

server-owned variant validator 按闭集执行：

| `ref_kind` | id列 | canonical row与ownership重读 |
|---|---|---|
| `target_asset` | `ref_uuid` | `target_assets.id`，再经 `targets.id=target_id` 验证 organization 与 project path |
| `dns_record` | `ref_bigint` | `dns_records.id`，再经 `targets.id=target_id` 验证 organization 与 project path |
| `web_origin_observation` | `ref_uuid` | observation 自身 organization/project，且其 web origin、可选 network endpoint、可选 target 不能跨org/project |
| `network_endpoint` | `ref_uuid` | endpoint 必须有非NULL organization，且 organization/project 与 execution authority 一致 |
| `enumeration_endpoint_observation` | `ref_uuid` | row 的 operation/org/project 必须一致，且 target/web-origin/endpoint ownership 全部一致 |

每个variant的`canonical_snapshot`是closed tagged object：只包含上表ownership字段、typed source id、variant identity字段、closed observation字段及该表真实observation timestamp；禁止metadata/raw/banner/body/cookie等自由或敏感字段。`target_assets`与`dns_records`除经target验证外，还必须验证事实行自己的`project_path`等于target project path和frozen path。snapshot、`source_observed_at`与`source_hash`由server在同一insert transaction从live row派生，caller提交值只用于exact comparison，任何漂移均拒绝。

每个 business ref 必须引用同 execution authority 下的 Evidence adapter，借此绑定 exact stage/worker attempt。`source_hash`由 DB/repo 根据variant的canonical allowlist snapshot生成；caller不能传任意 JSON 或自行选择hash字段。adapter insert trigger重算expected snapshot/hash，不相等时拒绝。adapter insert后拒绝UPDATE/DELETE。

这五类legacy canonical table均可能upsert或随target清理。adapter保存typed capture id、canonical allowlist snapshot与source hash，不以长期`ON DELETE RESTRICT`阻塞既有清理；validator在insert transaction内重读live row。后续reconciliation再次读取live row：缺失或hash漂移转为orphaned/revalidation，历史adapter仍append-only保留。`audit_log` evidence仍使用RESTRICT，因为它本身就是审计账本。

---

## Downstream compound-parent contract

所有Plan A持久化对象都必须携带 `execution_authority_id`；header同时保存以下不可变authority tuple：

```text
execution_authority_id
operation_id
project_scope_id
project_path_at_freeze
scope_snapshot_id
organization_id
stage_execution_id
stage_kind
execution_authority_hash
```

header 使用完整 compound FK 指向 `tool_truth_execution_authorities`。member 不重复信任 caller tuple，而使用下列 exact compound parent：

| child family | 必须引用的parent UNIQUE/FK |
|---|---|
| denominator item | `(denominator_id, execution_authority_id, denominator_hash)` |
| destination-policy member/hop | `(policy_id, execution_authority_id, policy_hash)`；hop还绑定同receipt |
| receipt input、raw witness、typed source、parser/temporal/budget census | `(receipt_id, execution_authority_id, receipt_authority_hash)` |
| input Evidence member | `(receipt_id, denominator_item_id, execution_authority_id)` + `(evidence_authority_id,execution_authority_id)` |
| input Business member | 同receipt/input parent + `(business_ref_authority_id,execution_authority_id)` |
| reconciliation及其lineage members | `(receipt_id,execution_authority_id)`，lineage只接normalized adapters |
| freshness attestation | `(receipt_id,reconciliation_id,semantic_version,semantic_hash,execution_authority_id)` |
| discovered-child manifest/member/closure | parent receipt+denominator item+execution authority整体；derived denominator继续使用同root authority或新server-derived child authority，不允许caller替换 |
| authority set/bundle与Gate assessment | root denominator、member receipt、reconciliation、freshness与execution authority整体绑定 |
| revalidation obligation/replacement | source与replacement各自完整authority，禁止用相同org/stage文字跨scope嫁接 |

`coverage_denominators`、`capability_execution_destination_policies` 与 `capability_execution_receipts` 删除 `TIMESTAMPTZ attempt_epoch`。新的execution keys为：

```text
root denominator: UNIQUE(execution_authority_id, stable_seal_request_id)
destination policy: UNIQUE(denominator_id, capability, policy_hash)
receipt: UNIQUE(denominator_id, execution_authority_id, capability, attempt_ordinal)
```

任何只含单列 `operation_id`、`organization_id`、`stage_execution_id`、`receipt_id` 或 `policy_id` 的 FK 都不能作为authority证明；单列FK可保留做存在性，但必须同时存在上述compound parent FK。

---

## DB-enforced lifecycle

### Rollout direct-mutation guard

`tool_truth_rollout` 必须先seed唯一 `(TRUE,'legacy_v1')`，再安装 trigger：

- UPDATE → SQLSTATE `23514`，message `tool_truth_rollout_direct_mutation_forbidden`；
- DELETE → 同一错误；
- seed后任何INSERT → `tool_truth_rollout_singleton_already_seeded`；
- `operation_state` INSERT的`tool_truth_contract`必须等于singleton当前值；Plan A singleton固定legacy，因此direct SQL不能创建shadow/receipt operation；不相等时拒绝为`tool_truth_operation_contract_not_deployed`；
- migration内不创建setter、promotion function、admin bypass或production test seam。

operation row的 `tool_truth_contract` 仍有独立 immutable trigger；same-operation resume/fork继承已有contract。

所有本文trigger invariant统一使用SQLSTATE `23514`和稳定lower-snake-case message；compound FK使用SQLSTATE `23503`并断言具名constraint；uniqueness/exact-one使用SQLSTATE `23505`并断言具名constraint。测试不得依赖PostgreSQL自由文本。

### Open→members→seal

下列header统一使用 `sealed_at TIMESTAMPTZ NULL`，并具有 `member_count BIGINT`、`member_set_hash Sha256DigestV1`：denominator、destination policy、temporal policy、temporal census、parser census、discovered-child manifest、overflow manifest、receipt-input lineage、reconciliation lineage、authority set、authority bundle。

每类header/member安装同一语义的专用trigger：

1. header insert只能`sealed_at IS NULL`；
2. member insert只允许parent `sealed_at IS NULL`；
3. member UPDATE/DELETE永远拒绝；
4. 唯一允许的header UPDATE是`sealed_at NULL → statement_timestamp()`；
5. seal trigger按canonical ordinal重算exact member count/set hash与identity，不信任caller；
6. zero-member只允许schema明确支持`sealed_empty=TRUE`的family；
7. seal后UPDATE/DELETE、第二次seal、hash/count变化、late member insert均拒绝；
8. deferred constraint trigger在transaction结束前拒绝unsealed header被receipt、Gate、consumer或其他sealed authority引用。

### Append-only families

target-state epoch events、revalidation dispatch events、revalidation obligation events、raw-witness access/retention events、network-hop receipts、freshness attestations与discovery-budget ledger在INSERT后拒绝UPDATE/DELETE。predecessor trigger验证同authority、ordinal/epoch严格`previous + 1`，并拒绝self/cycle与head跳跃。

### NULL-safe shape checks

所有OR-shape的required列必须同时检查`IS NOT NULL`与内容。例如：

```sql
CHECK (
    policy_decision <> 'blocked'
    OR (reason_code IS NOT NULL AND BTRIM(reason_code) <> '')
);

CHECK (
    closure_kind NOT IN ('blocked','external_dependency','out_of_scope')
    OR (residual IS NOT NULL AND jsonb_typeof(residual)='object')
);

CHECK (
    disposition <> 'ignored_versioned'
    OR (
        ignore_reason_code IS NOT NULL AND BTRIM(ignore_reason_code) <> ''
        AND ignore_rule_version IS NOT NULL AND BTRIM(ignore_rule_version) <> ''
    )
);
```

Rust reducer拒绝的terminal tuple必须同步为DB CHECK，不能只约束`coverage_extent='complete'`。

### Raw witness exact receipt binding

`capability_raw_witness_artifacts` 增加：

```text
UNIQUE(id, receipt_id, execution_authority_id)
FOREIGN KEY(receipt_id, execution_authority_id, receipt_authority_hash)
    -> capability_execution_receipts(id, execution_authority_id, receipt_authority_hash)
```

receipt 的反向引用必须使用：

```text
FOREIGN KEY(raw_witness_artifact_id, id, execution_authority_id)
    -> capability_raw_witness_artifacts(id, receipt_id, execution_authority_id)
```

两个方向的FK均为`DEFERRABLE INITIALLY DEFERRED`，并安装deferred reciprocal-binding constraint trigger。唯一合法事务顺序是：先创建raw pointer为NULL的running receipt，再插入exact-one artifact，最后以CAS执行receipt pointer `NULL → artifact_id`；commit时两个方向必须exact一致。receipt pointer第二次转换、artifact没有reciprocal pointer、pointer没有artifact、同receipt第二个artifact及cross-authority artifact全部拒绝并整体回滚。

这样receipt A不能引用receipt B的artifact。`coverage_extent='complete'`要求exact-one raw witness、sealed parser census、sealed temporal census及consistent reconciliation全部属于同receipt和同execution authority。

---

## Direct-SQL negative test matrix

所有测试放在未来唯一测试文件 `backend/crates/golish-db/tests/capability_execution_receipts.rs`，只使用隔离临时 PostgreSQL。每个测试断言SQLSTATE及稳定message code，不能只断言“某种数据库错误”。

| test name | 直接SQL伪造 | 必须拒绝为 |
|---|---|---|
| `execution_authority_rejects_cross_project_scope` | operation正确但project scope或frozen path不同 | `tool_truth_authority_project_mismatch` |
| `execution_authority_rejects_cross_organization_scope_member` | authority org不属于snapshot | `tool_truth_authority_scope_org_mismatch` |
| `execution_authority_rejects_cross_scope_snapshot` | operation/project正确但换foreign snapshot | `tool_truth_authority_scope_snapshot_mismatch` |
| `execution_authority_rejects_cross_stage_execution` | stage id来自另一operation或stage kind | `tool_truth_authority_stage_mismatch` |
| `wave_binding_rejects_cross_stage_or_scope` | 旧wave嫁接另一stage execution/snapshot | `tool_truth_wave_binding_mismatch` |
| `wave_binding_rejects_unsealed_scope_snapshot` | wave绑定仍open的scope snapshot | `tool_truth_scope_snapshot_unsealed` |
| `stage_unit_authority_rejects_foreign_scope_snapshot` | unit id配另一个sealed snapshot | `tool_truth_stage_unit_scope_mismatch` |
| `wave_binding_rejects_update_or_delete` | 改绑或删除immutable binding | `tool_truth_wave_binding_immutable` |
| `host_execution_rejects_any_worker_field` | host shape带worker id/epoch/lease/tool | `tool_truth_execution_owner_shape_invalid` |
| `worker_execution_requires_complete_worker_fence` | worker shape漏四字段之一 | `tool_truth_execution_owner_shape_invalid` |
| `execution_source_shape_rejects_partial_or_mixed_variant` | stage/wave/unit的id/hash为partial NULL或同时出现wave+unit | `tool_truth_execution_source_shape_invalid` |
| `worker_execution_rejects_same_epoch_from_other_worker` | epoch相同但worker_run/tool call不同 | `tool_truth_worker_fence_mismatch` |
| `worker_execution_rejects_old_epoch_with_new_lease` | old epoch拼new lease/tool call | `tool_truth_worker_fence_mismatch` |
| `worker_execution_rejects_foreign_source_tool_call` | tool call来自另一unit/org/stage | `tool_truth_worker_tool_call_mismatch` |
| `evidence_adapter_rejects_non_evidence_audit_role` | action/approval row冒充evidence | `tool_truth_evidence_role_invalid` |
| `evidence_adapter_rejects_legacy_audit_without_production_binding` | 事后认领普通audit evidence | `tool_truth_evidence_production_binding_missing` |
| `evidence_production_binding_rejects_cross_worker_same_stage` | 同stage/org把worker A evidence绑给worker B | `tool_truth_evidence_worker_fence_mismatch` |
| `evidence_adapter_rejects_cross_operation` | audit `run_id`不同 | `tool_truth_evidence_operation_mismatch` |
| `evidence_adapter_rejects_cross_project` | audit project path不等于project scope | `tool_truth_evidence_project_mismatch` |
| `evidence_adapter_rejects_missing_or_cross_org` | detail无org、非UUID或foreign org | `tool_truth_evidence_organization_mismatch` |
| `evidence_adapter_rejects_cross_stage_classification` | classification producer不是stage execution | `tool_truth_evidence_stage_mismatch` |
| `evidence_adapter_rejects_stale_or_out_of_scope_classification` | valid_to非NULL或非in_scope | `tool_truth_evidence_classification_invalid` |
| `evidence_adapter_rejects_classification_for_other_audit` | classification id属于另一evidence row | `tool_truth_evidence_classification_mismatch` |
| `evidence_adapter_rejects_malformed_org_without_cast_leak` | detail organization不是UUID | SQLSTATE `23514` + `tool_truth_evidence_producer_envelope_invalid` |
| `evidence_adapter_rejects_broken_or_forged_chain_hash` | parent断链/cycle/foreign parent/错误hash | `tool_truth_evidence_chain_invalid` |
| `evidence_adapter_rejects_forged_row_classification_or_authority_hash` | 任一server-derived hash漂移 | `tool_truth_evidence_hash_mismatch` |
| `evidence_binding_and_adapter_reject_update_or_delete` | 改删immutable production/adapter row | `tool_truth_evidence_authority_immutable` |
| `business_ref_rejects_dns_uuid_shape` | `dns_record`把id写入UUID列 | `tool_truth_business_ref_id_shape_invalid` |
| `business_ref_rejects_uuid_kind_bigint_shape` | TargetAsset等把id写入BIGINT列 | `tool_truth_business_ref_id_shape_invalid` |
| `business_ref_rejects_unknown_kind` | 非闭集ref kind | `tool_truth_business_ref_kind_invalid` |
| `business_ref_rejects_nonexistent_typed_id` | 闭集kind使用不存在的UUID/BIGINT | 对应`tool_truth_<variant>_ref_invalid` |
| `business_ref_rejects_foreign_authority_evidence` | business authority和Evidence authority不同 | 具名compound FK，SQLSTATE `23503` |
| `business_ref_rejects_cross_org_or_project` | canonical row owner与authority不同 | `tool_truth_business_ref_owner_mismatch` |
| `business_ref_rejects_cross_operation_enumeration_row` | Enumeration row operation不同 | `tool_truth_business_ref_operation_mismatch` |
| `business_ref_rejects_caller_source_hash` | caller hash与server重算不同 | `tool_truth_business_ref_source_hash_mismatch` |
| `business_ref_rejects_forged_snapshot_or_observed_time` | caller snapshot/time与live canonical row不同 | `tool_truth_business_ref_snapshot_mismatch` |
| `business_ref_rejects_update_or_delete` | 改删immutable adapter | `tool_truth_business_ref_immutable` |
| `receipt_rejects_cross_authority_denominator` | denominator与receipt authority不同 | `tool_truth_receipt_authority_mismatch` |
| `destination_policy_rejects_cross_authority_member` | policy member/hop来自foreign policy/authority | `tool_truth_destination_authority_mismatch` |
| `reconciliation_rejects_cross_receipt_or_worker_authority` | 同stage下另一worker authority的lineage | `tool_truth_reconciliation_authority_mismatch` |
| `raw_witness_rejects_cross_receipt_back_reference` | receipt A指向artifact B | `tool_truth_raw_witness_receipt_mismatch` |
| `raw_witness_rejects_cross_authority_or_missing_reciprocal_pointer` | artifact authority不同或只写单向pointer | `tool_truth_raw_witness_authority_mismatch` / `tool_truth_raw_witness_reciprocal_binding_missing` |
| `raw_witness_rejects_second_artifact_or_second_pointer_transition` | 同receipt第二artifact或再次改pointer | 具名UNIQUE `23505` / `tool_truth_receipt_transition_invalid` |
| `sealed_header_rejects_late_member_insert` | seal后追加member | `tool_truth_sealed_parent_immutable` |
| `sealed_header_rejects_reseal_or_hash_update` | 二次seal/改count/hash | `tool_truth_sealed_parent_immutable` |
| `sealed_member_rejects_update_or_delete` | 改删已插member | `tool_truth_member_append_only` |
| `unsealed_header_cannot_be_consumed` | receipt/Gate引用open header | `tool_truth_unsealed_authority` |
| `rollout_rejects_direct_update` | UPDATE legacy→shadow/receipt | `tool_truth_rollout_direct_mutation_forbidden` |
| `rollout_rejects_direct_delete_or_second_insert` | DELETE或第二singleton | `tool_truth_rollout_direct_mutation_forbidden` / `tool_truth_rollout_singleton_already_seeded` |
| `rollout_rejects_false_singleton` | INSERT singleton=FALSE | `tool_truth_rollout_singleton_check` |
| `operation_insert_rejects_contract_not_equal_to_frozen_rollout` | direct INSERT shadow/receipt operation | `tool_truth_operation_contract_not_deployed` |
| `operation_contract_rejects_update_or_unknown_value` | 改已冻结contract或写future enum | `operation_tool_truth_contract_immutable` / 具名CHECK `23514` |
| `shape_checks_reject_null_required_reason_or_residual` | blocked/ignored/closure用NULL绕过 | `tool_truth_shape_required_field_missing` |
| `hash_columns_reject_noncanonical_digest` | bare/uppercase/错误长度digest | constraint name以`_sha256_v1_check`结尾，SQLSTATE `23514` |
| `canonical_serializer_rejects_duplicate_key_type_drift_or_missing_field` | 重复key、数字/字符串漂移、缺字段、非canonical UUID | `tool_truth_canonical_payload_invalid` |
| `execution_authority_rejects_forged_server_hash` | shape正确但authority hash错误 | `tool_truth_authority_hash_mismatch` |

append-only family必须数据驱动逐表运行以下negative tests，不允许只抽测一种event：

```text
target_state_epoch_event
revalidation_dispatch_event
revalidation_obligation_event
raw_witness_access_event
raw_witness_retention_event
network_hop_receipt
freshness_attestation
discovery_budget_ledger_entry
```

| parameterized test | 直接SQL伪造 | 必须拒绝为 |
|---|---|---|
| `append_only_<family>_rejects_update` | 修改任一事实列 | `tool_truth_append_only` |
| `append_only_<family>_rejects_delete` | DELETE历史event | `tool_truth_append_only` |
| `append_only_<family>_rejects_foreign_predecessor` | predecessor来自另一authority/head | 对应具名compound FK，SQLSTATE `23503` |
| `append_only_<family>_rejects_ordinal_gap_or_fork` | 跳ordinal或同predecessor建两个successor | `tool_truth_event_ordinal_invalid` / 对应UNIQUE `23505` |
| `append_only_<family>_rejects_self_or_predecessor_cycle` | self predecessor或闭环 | `tool_truth_event_cycle_invalid` |
| `append_only_<family>_head_rejects_nonadjacent_or_stale_cas` | head跳代或旧row_version写入 | `tool_truth_head_nonadjacent` / `tool_truth_head_stale` |

open→members→seal family同样必须逐表参数化覆盖，而不是只测denominator：

```text
coverage_denominator
destination_policy
temporal_validity_policy
temporal_census
parser_census
discovered_child_manifest
discovery_overflow_manifest
receipt_input_lineage
semantic_reconciliation
authority_set
authority_bundle
```

每个family必须执行`unsealed_cannot_be_consumed`、`wrong_member_count`、`wrong_member_hash`、`ordinal_gap`、`duplicate_semantic_member`、`illegal_empty_seal`、`member_update`、`member_delete`、`late_member_insert`、`sealed_header_update`、`sealed_header_delete`、`reseed_or_reseal`与`concurrent_second_sealer`；稳定拒绝码依次为`tool_truth_unsealed_authority`、`tool_truth_set_member_count_mismatch`、`tool_truth_set_member_hash_mismatch`、`tool_truth_set_ordinal_invalid`、具名UNIQUE `23505`、`tool_truth_set_empty_invalid`、`tool_truth_member_append_only`、`tool_truth_member_append_only`、`tool_truth_sealed_parent_immutable`、`tool_truth_sealed_parent_immutable`、`tool_truth_sealed_parent_immutable`、`tool_truth_set_seal_conflict`。

NULL-safe shape必须逐tuple测试，不能由一个聚合用例代替：root/derived denominator parent、raw-range bounds、server-control no-range、semantic head version/id/hash triple、input-lineage seal summary quartet、reconciliation open/sealed terminal fields、budget observed/actual pair、allowed/blocked hop destination/reason、network/nonnetwork child closure residual、retention event next-policy、truncated/original/stored byte counts，以及complete receipt的attempt/landing/observation/coverage/gap/reconciliation/destination/raw/parser/temporal/budget/input全轴。

另外保留两个positive control：同一worker/tool fence的exact replay必须返回同一authority id；同一host-stage stable request replay必须返回同一denominator id。它们不允许创建第二份authority或扩大member set。

---

## 唯一 migration 的已授权清单

2026-07-30的Plan A技术负责人授权覆盖下列范围：

1. 在 `operation_state` 增加`tool_truth_contract`、legacy backfill/default/check及immutable trigger。
2. 创建只读且DB层不可直接修改的`tool_truth_rollout` singleton；不创建promotion。
3. 为scope snapshot、stage wave、stage unit、tool call增加本文四个additive compound UNIQUE。
4. 创建`tool_truth_stage_wave_execution_bindings`与immutable trigger。
5. 创建`tool_truth_execution_authorities`、closed source/owner shape、compound FKs与immutable trigger。
6. 创建`tool_truth_evidence_production_bindings`与`tool_truth_evidence_authorities`，安装exact producer fence、audit/classification/chain server validator；不ALTER通用`audit_log` authority列。
7. 创建`tool_truth_business_ref_authorities`、五个closed variant validator、typed id shape、partial UNIQUE与server-derived source hash。
8. 将原Plan A denominator/policy/receipt/reconciliation及所有member表改为execution-authority compound parent模型，并删除三处timestamp attempt epoch。
9. 安装统一Sha256DigestV1 checks、NULL-safe terminal/variant checks、raw witness双向exact receipt FK。
10. 为所有exact-set family安装open→members→seal triggers，为所有event family安装append-only/predecessor/head triggers。
11. 在隔离临时数据库运行本文negative matrix、migration fresh smoke与只包含HEAD+本提交+未来Task2 commit的isolated snapshot验证。
12. migration comment明确：一旦写入Tool Truth audit truth，只允许forward-fix；不得通过down migration删除receipt/evidence/business authority。

该清单不授权修改已有migration、访问现存数据库、promotion、Plan B/C/D、frontend/generated IPC、Reporting或外部请求。

---

## 实施顺序

### Task A：先写完整 RED migration contract

**文件：**

- 创建：`backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql`
- 扩展：`backend/crates/golish-db/tests/capability_execution_receipts.rs`

**步骤：**

1. 先加入本文direct-SQL matrix的authority spine、adapter、seal、rollout与raw binding测试。
2. 运行每组测试，确认失败原因是缺表/缺约束/缺trigger，而不是fixture错误。
3. 记录每个RED命令、exit code与首个稳定失败证据。

**验证：**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts -E 'test(execution_authority_) | test(wave_binding_) | test(stage_unit_authority_) | test(worker_execution_) | test(evidence_adapter_) | test(business_ref_) | test(receipt_rejects_) | test(destination_policy_) | test(reconciliation_) | test(raw_witness_) | test(sealed_) | test(unsealed_) | test(rollout_) | test(shape_checks_) | test(hash_columns_)')
```

Expected：selected tests明确RED，且没有连接Test1或外部数据库。

### Task B：实现唯一 migration 并逐组转GREEN

**文件：**

- 仅创建/修改：`backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql`
- 仅扩展：`backend/crates/golish-db/tests/capability_execution_receipts.rs`

**步骤：**

1. 先实现scope/wave/unit/tool-call compound keys与execution spine，跑authority组GREEN。
2. 再实现Evidence adapter，跑Evidence组GREEN。
3. 再实现Business adapter与五variant validator，跑Business组GREEN。
4. 再接downstream compound parents与raw exact binding，跑cross-authority组GREEN。
5. 最后安装rollout、seal、append-only、NULL-safe与hash triggers，跑immutability组GREEN。
6. 从staged snapshot构造隔离提交树，证明它不依赖共享Application Model未提交hunk或未跟踪migration。

**验证：**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts)
```

Expected：该integration test binary全部通过，exit code 0；只使用临时embedded PostgreSQL。

### Task C：精确提交并暂停

未来Task 2提交只能包含：

```text
backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql
backend/crates/golish-db/src/repo/tool_truth_rollout.rs
backend/crates/golish-db/src/repo/mod.rs 中仅Plan A hunk
backend/crates/golish-db/src/repo/operation_state.rs 中仅Plan A hunk
backend/crates/golish-db/src/repo/runtime_memory_tx.rs 中仅Plan A hunk
backend/crates/golish-db/tests/capability_execution_receipts.rs
```

候选提交信息保持：

```text
feat(tool-truth): freeze receipt contract per operation
```

若isolated staged snapshot不能独立compile/migrate/test，停止，不得把共享Application Model改动、既有untracked migration或第二个migration夹入提交。

---

## 本文档提交边界

当前只允许暂存并提交：

```text
docs/superpowers/plans/2026-07-30-tool-truth-coverage-contract-schema-amendment-v2.md
```

当前文档提交不得包含Task 2 RED tests、`agent-progress.md`、`feature_list.json`、Plan A原文、模块卡、INDEX、migration或任何共享dirty-tree代码。提交后立即继续Task 2，不等待逐步批准。
