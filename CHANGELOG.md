# Changelog

## Unreleased

### ⚠ BREAKING CHANGES — Architecture

- **agent stack layout** (A1–A3, 2026-05-02):
  - `golish-prompts` no longer depends on `golish-sub-agents`.
    `SubAgentPromptContributor` moved to
    `golish_sub_agents::prompt_contributor::SubAgentPromptContributor`.
    `create_default_contributors` moved to
    `golish_agent_bridge::contributors::create_default_contributors`.
  - Renamed `golish-agent-loop` → **`golish-agent-kit`** (Layer 4a
    building blocks: tool executors, HITL, planner, tool policy, db
    tracking, llm-client wiring).
  - Renamed `golish-agentic-loop` → **`golish-agent-runtime`** (Layer
    4b high-level streaming loop, eval harness, mocks).
  - **Removed `golish-ai` umbrella crate.** All downstream consumers
    now import directly from the implementation crates
    (`golish-agent-kit`, `golish-agent-runtime`, `golish-agent-bridge`,
    `golish-prompts`, `golish-events`).

  Migration: replace `use golish_ai::X::...` with the matching
  implementation crate. Common remappings:
  - `golish_ai::agentic_loop::*`         → `golish_agent_runtime::agentic_loop::*`
  - `golish_ai::eval_support::*`         → `golish_agent_runtime::eval_support::*`
  - `golish_ai::agent_mode / db_* / execution_mode / hitl / llm_client / loop_detection / memory_* / planner / sidecar_trait / system_hooks / tool_*`
                                          → `golish_agent_kit::*`
  - `golish_ai::agent_bridge / AgentBridge` → `golish_agent_bridge::*`
  - `golish_ai::task_orchestrator::bridge_executor::*` → `golish_agent_bridge::bridge_executor::*`
  - `golish_ai::{codex_prompt, contributors, prompt_registry, summarizer, system_prompt, generate_summary, SUMMARIZER_SYSTEM_PROMPT, SummaryResponse, PromptContributorRegistry, build_summarizer_user_prompt}`
                                          → `golish_prompts::*`
  - `golish_ai::{build_summarizer_input, format_for_summarizer, read_transcript, save_summarizer_input, save_summary, transcript_path, CoordinatorHandle, CoordinatorState, EventCoordinator, TranscriptEvent, TranscriptWriter}`
                                          → `golish_events::*`

## [0.3.0](https://github.com/ChristopherZh-7/Golish-Platform/compare/v0.2.43...v0.3.0) (2026-08-11)


### ⚠ BREAKING CHANGES

* **agent-kit:** remove evidence-projection + confirm-only synthesis fallbacks (Task 5-6)
* **pipeline:** removes pipeline_* Tauri commands, PipelineEvent, and the pipelines table.
* **harness:** remove required_checks fixed-menu; gate_rules is sole gate entry

### Features

* add deepseek provider support ([01342af](https://github.com/ChristopherZh-7/Golish-Platform/commit/01342af0f6bcf68e51dbfd0a44476f53115d7d3b))
* **agent-kit:** remove evidence-projection + confirm-only synthesis fallbacks (Task 5-6) ([3e1563f](https://github.com/ChristopherZh-7/Golish-Platform/commit/3e1563f31e1b969e775451bea9dcfc6ee0c1d82e))
* **agent-kit:** unified refiner skeleton — deterministic classifier + per-class templates (Task 1-2, design 2026-06-12-unified-refiner) ([31f570d](https://github.com/ChristopherZh-7/Golish-Platform/commit/31f570d346c30865e91f225efe31ad4ccc5cf7e0))
* **agent-kit:** wire both gate sites through the unified refiner (Task 3-4,7) ([be59f1d](https://github.com/ChristopherZh-7/Golish-Platform/commit/be59f1d9d3a51b5765e1cb694a496efd26776cb3))
* **agent-runtime:** record sub-agent dispatch lifecycle in direct path ([30a1a86](https://github.com/ChristopherZh-7/Golish-Platform/commit/30a1a86c57e2b2a7f75e5457b8f6df767398a160))
* **agent-tools:** background tool result feedback (P2) + live card / cancel / evidence (P3) ([20c36a0](https://github.com/ChristopherZh-7/Golish-Platform/commit/20c36a06d70ff7f12ba8acbc3ddab777fd36500b))
* **agent-tools:** expose list_in_scope_targets to the LLM ([3303ad5](https://github.com/ChristopherZh-7/Golish-Platform/commit/3303ad5a11e4faacce211bc474c0eca3a10a1a36))
* **agent-tools:** soft-timeout → background execution for AI shell/pentest commands (P1) ([da9c086](https://github.com/ChristopherZh-7/Golish-Platform/commit/da9c086f217edf3917296b98d3cd96a54563503b))
* **agent:** add kill_job tool + cap DNS AXFR background timeout (A+C+D) ([6c8742e](https://github.com/ChristopherZh-7/Golish-Platform/commit/6c8742e97fa008ee3cd772eded15e27311b712c0))
* **agent:** gate execution mentor modes ([dac6fd8](https://github.com/ChristopherZh-7/Golish-Platform/commit/dac6fd8bc812c0322d54efcb3ca2096a1f3a4c40))
* **agent:** persist resume repair directives ([37b04f4](https://github.com/ChristopherZh-7/Golish-Platform/commit/37b04f44a97fac643fa4b88fe93518d8f99e0dce))
* **agent:** trace execution mentor advice ([c549238](https://github.com/ChristopherZh-7/Golish-Platform/commit/c5492386d7c6ad1e50d5af13c80c16ab7bb0fa29))
* **ai-chat:** add model settings popover, agent status indicator, and LLM quirks/overrides ([b3fae8e](https://github.com/ChristopherZh-7/Golish-Platform/commit/b3fae8e1deb4c4f5c0b23a10f5cf0ac5fa12e121))
* **ai-chat:** emit ContextWarning on history restore + bolder usage ring ([33917a9](https://github.com/ChristopherZh-7/Golish-Platform/commit/33917a90d6dc550f89f777cfe79609f9d9f8d2d2))
* **ai-chat:** lead agent thinks then decides whether to plan ([392b4c5](https://github.com/ChristopherZh-7/Golish-Platform/commit/392b4c500ddfe3396e8d161bbac2bd324dd1456c))
* **ai-chat:** persistent tool-approval switch + per-tool always-allow ([baaaa30](https://github.com/ChristopherZh-7/Golish-Platform/commit/baaaa30e770d89c409afd6e07d47786950727c0e))
* **ai-chat:** pin reasoning pane to latest line while streaming ([5d4e83f](https://github.com/ChristopherZh-7/Golish-Platform/commit/5d4e83ff9a680beb1cb84635f4f759f2d982d560))
* **ai-chat:** surface passive-intel/OSINT providers on recon tool cards ([b5c1fd7](https://github.com/ChristopherZh-7/Golish-Platform/commit/b5c1fd7a900d4837655add755b62ad70fa14c129))
* **ai-events:** surface sub-agent background jobs ([e529a75](https://github.com/ChristopherZh-7/Golish-Platform/commit/e529a7542bebdc1ddced13208d183d90bc668d55))
* **arch:** add repo data-ownership boundary guard (S1-1) ([dc9ad0f](https://github.com/ChristopherZh-7/Golish-Platform/commit/dc9ad0f823b0abdd0e7700e4404a1094ab111616))
* **arch:** add VaultReadPort + PgVaultAdapter (S1-2a ports skeleton) ([6abaec8](https://github.com/ChristopherZh-7/Golish-Platform/commit/6abaec8fec72c9235d86faefde6c4a2f810b6d5d))
* **arch:** complete S1-2 horizontal-coupling port-ization (S1-2c–f) ([4246739](https://github.com/ChristopherZh-7/Golish-Platform/commit/424673909aa5fb1719abfdc45fabd6aeb992aee1))
* **arch:** crate-per-service split (M0–M5) + ReconPort layer-B (S1-2b) ([45f4bb2](https://github.com/ChristopherZh-7/Golish-Platform/commit/45f4bb225f90e8eb37cb4c5c78f9f876503e10d7))
* **arch:** S1-3 pentest tool factory port + legacy AI bridge cleanup ([936350a](https://github.com/ChristopherZh-7/Golish-Platform/commit/936350abb181c0699d178559a2487db304a44c79))
* **ask-human:** steer unit_review context to {organization_id} (T4) ([d7e49f9](https://github.com/ChristopherZh-7/Golish-Platform/commit/d7e49f93610e10e1586f805d23264a256543709c))
* **ask-human:** unit_review sources candidates from DB by org_id (T2+T3) ([a1fdec1](https://github.com/ChristopherZh-7/Golish-Platform/commit/a1fdec1ece5910bcfedbcf790b4f1e891f0aa19e))
* **asset-intel:** 0.zone expand email/code/member + Leakage/DNS UI groups ([398c83a](https://github.com/ChristopherZh-7/Golish-Platform/commit/398c83ae494f54a303656e01cc976e97a9670030))
* **asset-intel:** close the passive-intel pairing -&gt; landing loop ([8312365](https://github.com/ChristopherZh-7/Golish-Platform/commit/8312365dedcece24ab0b9eb98c65456e522e0072))
* **asset-intel:** native_provider runtime + dedicated intel-providers dir ([27352e1](https://github.com/ChristopherZh-7/Golish-Platform/commit/27352e165c828a9d811d34ba60d6dc47445d36d5))
* **asset-intel:** scaffold provider abstraction + flat schema + two-phase hydrate (WIP) ([527d988](https://github.com/ChristopherZh-7/Golish-Platform/commit/527d988164b1c1d4c7b7360fba04d22d93918b28))
* **candidate:** define readonly analysis team ([babaec4](https://github.com/ChristopherZh-7/Golish-Platform/commit/babaec427b428dc03d3ffafb012222be67ab1cde))
* **candidate:** route registry-authoritative stage runs ([316e562](https://github.com/ChristopherZh-7/Golish-Platform/commit/316e562a03b9d539b8e88bb473b56950c90128cd))
* **candidate:** run bounded hypothesis analysis team ([fc9ea65](https://github.com/ChristopherZh-7/Golish-Platform/commit/fc9ea65572ea4f82f3a0402026f005d22747e319))
* **capture:** broaden credential auto-capture rules ([4705a90](https://github.com/ChristopherZh-7/Golish-Platform/commit/4705a9010ea5c04a432585f04c2b4f6ae7f49c55))
* **capture:** Phase 2 CaptureEngine — scaffold + state machine + webview + extraction + TTL watcher + wiring (T2.1-T2.6) ([423b103](https://github.com/ChristopherZh-7/Golish-Platform/commit/423b103f446a7e23967566298269a030fa55b00c))
* **capture:** Phase 3 backend — 3 Tauri commands (T3.1) ([02da74d](https://github.com/ChristopherZh-7/Golish-Platform/commit/02da74d223319afbb7fb3a81d730cda20c6ad94a))
* **capture:** Phase 3 frontend — captureStart/Status/Cancel IPC wrappers (T3.2) ([7be9eca](https://github.com/ChristopherZh-7/Golish-Platform/commit/7be9eca44f64919158d6b933dbf6ce7edf687019))
* **capture:** Phase 4 frontend UX — i18n + useCaptureSession hook + 3 components + IntegrationGroup integration (T4.1-T4.5) ([9517a73](https://github.com/ChristopherZh-7/Golish-Platform/commit/9517a73f47d4570ee30401d7aefd5908c296cddb))
* **capture:** Phase 5 T5.1 — ENScan AQC capture recipe + fixture-load smoke test ([2dbd369](https://github.com/ChristopherZh-7/Golish-Platform/commit/2dbd36934a04b111ba5958f712a704296a10abf3))
* checkpoint durable stage team runtime ([5af4f31](https://github.com/ChristopherZh-7/Golish-Platform/commit/5af4f31a02c7e4a78277460909201692611c1520))
* close candidate memory and runtime recovery ([f435077](https://github.com/ChristopherZh-7/Golish-Platform/commit/f4350776ffff49f111e0e815695e99070ec20a11))
* **commands:** list_running_sub_agent_dispatches Tauri command ([2fda447](https://github.com/ChristopherZh-7/Golish-Platform/commit/2fda447fcf6245bbdc4782176b0e52e964c9d39a))
* **core/plan:** add FailureKind enum + PlanStep.failure_kind (P0-2 stage 1) ([524a02d](https://github.com/ChristopherZh-7/Golish-Platform/commit/524a02d8c673d230bb61f125636bb865b0283e50))
* **db-traits:** add dispatch lifecycle methods + DispatchStatus enum ([44bfb12](https://github.com/ChristopherZh-7/Golish-Platform/commit/44bfb12f8782fd26a8583a0b066f772ff967d0f9))
* **db,bridge:** sqlx impl for sub_agent_dispatches lifecycle ([3332cc9](https://github.com/ChristopherZh-7/Golish-Platform/commit/3332cc9becbc30a8c5d42be2b1087fb6b772b258))
* **db:** add evidence ledger + sprint_contracts schema (Task 1a.1) ([1792885](https://github.com/ChristopherZh-7/Golish-Platform/commit/179288566246ffb9d577cad5aa1d76cfff1b4797))
* **db:** add hypothesis registry schema ([502b485](https://github.com/ChristopherZh-7/Golish-Platform/commit/502b4855cad08e75bb8d169678256cdc36bb5d05))
* **db:** add nullable evidence_technique/evidence_outcome to audit_log (PR2, additive/I10-safe) ([990c3b8](https://github.com/ChristopherZh-7/Golish-Platform/commit/990c3b80c027cb807662ae32173491c6fed9037e))
* **db:** add sub_agent_dispatches table for resumable dispatch tracking ([c71f77c](https://github.com/ChristopherZh-7/Golish-Platform/commit/c71f77c7494ed681e031d0eca186787530da4dca))
* **db:** anchor chat session to one DB session via chat_session_key ([67a5984](https://github.com/ChristopherZh-7/Golish-Platform/commit/67a5984c3eb4fb77afa9a4b34608903f0484bc61))
* **db:** host-aware coverage 2c-2 — type-aware truth projection ([fae164e](https://github.com/ChristopherZh-7/Golish-Platform/commit/fae164e12db992be4e0fcc7f99c858b676d5d675))
* **db:** host-aware coverage 2c-3a storage + truth (RDNS + IP-WHOIS) ([f140df0](https://github.com/ChristopherZh-7/Golish-Platform/commit/f140df06f7d8ba07a2dbe24f6128df9d5d9e94d9))
* **db:** in_scope_target_types override powers harness expected_techniques (P3 Phase B) ([5460d3f](https://github.com/ChristopherZh-7/Golish-Platform/commit/5460d3f8612f9f328073772cc305a90b3d422d69))
* **db:** project-scoped repo helpers for residual scoped SQL sink (P0-3b) ([65e0292](https://github.com/ChristopherZh-7/Golish-Platform/commit/65e0292fb912b045d6bd64de41e8200e6f551312))
* **db:** resumable-task lookup + reaper keeps checkpointed tasks resumable ([f22bff3](https://github.com/ChristopherZh-7/Golish-Platform/commit/f22bff3d7a77bd1665b50aea998a7095731f83b4))
* **dispatch/ui:** in-flight sub-agent dispatch monitor in Advanced Settings ([7fd344c](https://github.com/ChristopherZh-7/Golish-Platform/commit/7fd344c6764bd61a02ff9d958a0a48f4c17b05ba))
* **dispatch:** reap stale running dispatches on list query (P0-4.c) ([48528f8](https://github.com/ChristopherZh-7/Golish-Platform/commit/48528f80d7b8206d1f949ac5b9c51032fe7fdf81))
* **engagement:** backend fleet driver + engagement_run_fleet command + frontend switch (fleet Phase B) ([a6b3464](https://github.com/ChristopherZh-7/Golish-Platform/commit/a6b34641ba8a211b9d4c4d7348999a9843ba890e))
* **engagement:** scoping fanout scaffolding + scoping HITL wiring ([db744e9](https://github.com/ChristopherZh-7/Golish-Platform/commit/db744e90fa34b2da7d99c9852ec7af169081ebad))
* **engagement:** stage-run fan-out — inline engagement card + stage-run detail view + design spec ([1f5ce7b](https://github.com/ChristopherZh-7/Golish-Platform/commit/1f5ce7b93166168eb0781f7d8a07f8718ad52936))
* Enhance organization recon functionality and add new tools ([0891f60](https://github.com/ChristopherZh-7/Golish-Platform/commit/0891f6026364a8bf460513cf35b53e5d4f491e08))
* **error:** end-to-end error-code contract (P0-1, I1) ([92522a7](https://github.com/ChristopherZh-7/Golish-Platform/commit/92522a78653fd69d8c2fdb56cf010f21a35d978c))
* **evidence:** add evidence_kinds.json static config (Task 1a.4) ([af60bc3](https://github.com/ChristopherZh-7/Golish-Platform/commit/af60bc35896ce1d70ff4b3dce6557fc9b6ad4904))
* **evidence:** add EvidenceLedger + ScopeService + repo functions (Task 1a.2) ([e5eb552](https://github.com/ChristopherZh-7/Golish-Platform/commit/e5eb552c190a45d18c8c933c3d392f0089e9a9c6))
* **evidence:** add EvidenceSanitizer for prompt injection defense (Task 1b.1) ([aa7e6bf](https://github.com/ChristopherZh-7/Golish-Platform/commit/aa7e6bfec87ab753b483bca81311643867747879))
* **evidence:** add read_evidence Tauri command with sanitize layer (Task 1b.2) ([b215046](https://github.com/ChristopherZh-7/Golish-Platform/commit/b2150469d3553025eca0a7728cfcdd334f427ac5))
* **evidence:** stamp passive-intel evidence with (technique, asset, outcome) facts (PR2) ([49c538d](https://github.com/ChristopherZh-7/Golish-Platform/commit/49c538dc20586dd72345074aeca0c995b304e508))
* **evidence:** startup reclaim abandoned audit rows (Task 1a.3) ([03f24fa](https://github.com/ChristopherZh-7/Golish-Platform/commit/03f24fa0329cc4f80dfb75f4888c919f328f19f8))
* **fe:** IP-centric target panel — domains in the host's Surface tab ([521a39a](https://github.com/ChristopherZh-7/Golish-Platform/commit/521a39a0a0be6decade7906390515d67236fa97a))
* **frontend:** add hypothesis registry audit panel ([df0a94a](https://github.com/ChristopherZh-7/Golish-Platform/commit/df0a94a64b92a6b71443b948a08dea42c66cfd1d))
* **frontend:** add useAsyncQuery + AsyncView async/tri-state primitives ([c1423af](https://github.com/ChristopherZh-7/Golish-Platform/commit/c1423af6aa7a45776ac6f12a0538d102aa9bf8b5))
* **frontend:** generate model ID constants from JSON at build time ([00292e6](https://github.com/ChristopherZh-7/Golish-Platform/commit/00292e6070e63748aae2d25f971308597b552626))
* **golish-db:** DB-truth coverage query layer (PR-A/B) ([f3a7337](https://github.com/ChristopherZh-7/Golish-Platform/commit/f3a7337b7029c96eda4854a2ae70b1d472839eed))
* **golish-db:** extract embedded Postgres with the zip crate (Windows, no unzip CLI) ([da4db80](https://github.com/ChristopherZh-7/Golish-Platform/commit/da4db802de1ac010e547bd32bc2166a7ea63ac51))
* **golish-pentest:** dns_record_add db_action persists dig output to dns_records (PR-B) ([bc7328d](https://github.com/ChristopherZh-7/Golish-Platform/commit/bc7328dd61e25285a4335c5e782ffcac90145fcb))
* harden candidate EAS and runtime recovery ([fbc7cd0](https://github.com/ChristopherZh-7/Golish-Platform/commit/fbc7cd08f813b0340ca9670ecb908f4d975f302b))
* **harness:** activate dynamic expected_techniques seam (P3 Phase A) ([ab4ac35](https://github.com/ChristopherZh-7/Golish-Platform/commit/ab4ac3517aff9e571f9fff154110f4f49d6f79cc))
* **harness:** add assessment profile + external_attack_surface stage spec (Task 1c.1) ([163f04e](https://github.com/ChristopherZh-7/Golish-Platform/commit/163f04e02b1093adb486f48505ad31c24a624ee8))
* **harness:** add deterministic IntentClassifier (Task 1c.3) ([bb98f3e](https://github.com/ChristopherZh-7/Golish-Platform/commit/bb98f3e7927b500abfd86222b578eef7d5124082))
* **harness:** add enumerator specialist for enumeration stage ([d6413ee](https://github.com/ChristopherZh-7/Golish-Platform/commit/d6413ee4ad55f0937dc7dfffdff3c8b736c6cbec))
* **harness:** add gate check full impl (Task 1c.5) ([52f70d4](https://github.com/ChristopherZh-7/Golish-Platform/commit/52f70d48433d52304ed43b303382752a0a5c5827))
* **harness:** add optional technique field to StageClaim/HarnessFinding (P5 Task 1) ([d16b3da](https://github.com/ChristopherZh-7/Golish-Platform/commit/d16b3dadc1486d3fd1f252bf2f789ab05d83282c))
* **harness:** add prober specialist + host-aware 2b coverage for external_attack_surface ([f1e6770](https://github.com/ChristopherZh-7/Golish-Platform/commit/f1e67708cca057adb72bf488738e350b01535867))
* **harness:** add ScopingPolicy per-profile scoping config ([e8bedc9](https://github.com/ChristopherZh-7/Golish-Platform/commit/e8bedc9e229b6a9d75d46233a52db675ad557f99))
* **harness:** add source_query_log layer ([#5](https://github.com/ChristopherZh-7/Golish-Platform/issues/5)) — per-source passive-intel query log ([cec6af0](https://github.com/ChristopherZh-7/Golish-Platform/commit/cec6af0f09781d731642aa02b5ccdd2fdf17b73e))
* **harness:** add Sprint Contract generator with cross-vendor LLM (Task 1c.4) ([1bcdc52](https://github.com/ChristopherZh-7/Golish-Platform/commit/1bcdc52b29c83b967cba6018ec61d952b45970c2))
* **harness:** add stage harness module skeleton (Task 1c.2) ([559416f](https://github.com/ChristopherZh-7/Golish-Platform/commit/559416fe829a93705cbb0a029ae7c47543c3f041))
* **harness:** agent-driven stage body with self-managed todos ([1fcb9ef](https://github.com/ChristopherZh-7/Golish-Platform/commit/1fcb9ef2c182f5892296376ae4a602069189363c))
* **harness:** align target_intel prompts with DB-truth coverage (PR3) ([2b3d047](https://github.com/ChristopherZh-7/Golish-Platform/commit/2b3d047d3edbe1c34f19efb54f4848d69ed1485a))
* **harness:** authoritative found — coverage gate only trusts DB/ledger truth (redteam Phase 0) ([c20b1d5](https://github.com/ChristopherZh-7/Golish-Platform/commit/c20b1d53d2e1204ece2d768e9176a1a2236fe8b5))
* **harness:** auto-bind engagement org from scoping on the chat path (core 0 A2) ([c1489eb](https://github.com/ChristopherZh-7/Golish-Platform/commit/c1489eb744e62fece257aa2a41ac78adfa728614))
* **harness:** branch scoping prompt by scoping_policy ([ebafbe7](https://github.com/ChristopherZh-7/Golish-Platform/commit/ebafbe77c4e985fcc9d3eb6b8226eda952c5cbf2))
* **harness:** category-based per-stage tool whitelist (deny-by-default); retire forbidden-tool blacklist ([eba2a74](https://github.com/ChristopherZh-7/Golish-Platform/commit/eba2a745d2a1b6abc6290949aeeb5365cfdb8291))
* **harness:** checkpoint deterministic stage closure and V2 design ([ab7b0c4](https://github.com/ChristopherZh-7/Golish-Platform/commit/ab7b0c4a6712c55a002e120c669435e9f7d48eed))
* **harness:** checkpoint runtime memory and candidate pipeline v2 ([13b2962](https://github.com/ChristopherZh-7/Golish-Platform/commit/13b29628f2954b56b918329bfe3217132fe6eb56))
* **harness:** close enumeration and harden durable execution ([f75764c](https://github.com/ChristopherZh-7/Golish-Platform/commit/f75764ca157ea72255ccf5c72a0f882ace11d9f9))
* **harness:** close target_intel provider evidence loop ([395643d](https://github.com/ChristopherZh-7/Golish-Platform/commit/395643df9e3b13f4359b44989df6557f06261f9a))
* **harness:** closeout reconciliation barrier + in-flight workspace batch ([f07ef48](https://github.com/ChristopherZh-7/Golish-Platform/commit/f07ef48e972c93397ba966261489a5eaf26b0f09))
* **harness:** complete Phase C runtime wiring (C1-C6) + multi-profile + scoping entry ([634a6dc](https://github.com/ChristopherZh-7/Golish-Platform/commit/634a6dcb26f28a3340f9d63d1640c3d605708428))
* **harness:** configure per-mode scoping_policy in 6 profiles ([68d8419](https://github.com/ChristopherZh-7/Golish-Platform/commit/68d8419dbf679993d36ba55ee44d3ea68c9ab387))
* **harness:** confine list_in_scope_targets to the engagement org subtree (core 3) ([46b9377](https://github.com/ChristopherZh-7/Golish-Platform/commit/46b9377625481facaac8c36092f827e3a0e835da))
* **harness:** confine stage_run fan-out to the engagement org subtree (core 2) ([362237c](https://github.com/ChristopherZh-7/Golish-Platform/commit/362237c2e32d1a545e903854032da64f33f60e01))
* **harness:** consume recon in-scope assets (coverage gate + stage injection) ([f95998f](https://github.com/ChristopherZh-7/Golish-Platform/commit/f95998f9946baee8a0c64b0b4f6f0c8ecea95ec0))
* **harness:** coverage matrix Phase 1 — data model + gate building blocks ([ca86a5e](https://github.com/ChristopherZh-7/Golish-Platform/commit/ca86a5ecb8e080ed3e9158a9f339a83bab56d95d))
* **harness:** coverage matrix Phase 1.5 — coverage_complete + WSTG sample + checked_empty evidence ([7b7e77c](https://github.com/ChristopherZh-7/Golish-Platform/commit/7b7e77cca48aa676285242065bf10dca375f7482))
* **harness:** coverage_complete derive_from_evidence projection, target_intel gray rollout (PR3, I8-safe) ([b5abf46](https://github.com/ChristopherZh-7/Golish-Platform/commit/b5abf46b7e4511a4b080d80755861142d0bd8875))
* **harness:** data-driven gate_rules engine + real evidence ids on block ([d02dbb4](https://github.com/ChristopherZh-7/Golish-Platform/commit/d02dbb463b70e637f5a500e4954a4a74e8efa06a))
* **harness:** dead-asset liveness_state + EAS report-noise fixes ([6cfdeaa](https://github.com/ChristopherZh-7/Golish-Platform/commit/6cfdeaa249d1779f1aabcabfbbab776561dd8bd9))
* **harness:** default stage_mode ON with explicit kill switch ([aacd425](https://github.com/ChristopherZh-7/Golish-Platform/commit/aacd4257d2e9ee631f539e346a93f2b31eee22fe))
* **harness:** document technique tagging in submit_stage_deliverable schema + pin capture tests (P5 Task 7) ([f88cbc2](https://github.com/ChristopherZh-7/Golish-Platform/commit/f88cbc283942751dc88a20892d5f2d53813557bd))
* **harness:** drop target_intel hard tool floors; enrich is primary ([976836c](https://github.com/ChristopherZh-7/Golish-Platform/commit/976836cd136c326b5aad44b59bb3a30ca13d6749))
* **harness:** EAS + enumeration per-asset technique matrices (host-aware coverage 2b core) ([e12a763](https://github.com/ChristopherZh-7/Golish-Platform/commit/e12a76389a631a3c2ab2e7976b6df516059a2b80))
* **harness:** extend [#4](https://github.com/ChristopherZh-7/Golish-Platform/issues/4) technique_outcomes wiring — submit-preview read + recon/enrich write points ([d842eb1](https://github.com/ChristopherZh-7/Golish-Platform/commit/d842eb115fff52fb05b4e361ca3ff70b89fd0f8e))
* **harness:** facts-only stages satisfy vacuous from DB truth (PR2) ([c6c39ae](https://github.com/ChristopherZh-7/Golish-Platform/commit/c6c39ae07891ebc2c5253920568fa5bd6cd5a5d2))
* **harness:** feature flag harness.stage_mode_enabled (default off) (Task 1c.7) ([10dd927](https://github.com/ChristopherZh-7/Golish-Platform/commit/10dd927c74036c87178dff67a4c13d04c5c32a0b))
* **harness:** finish runtime memory candidate v2 implementation ([6acabe4](https://github.com/ChristopherZh-7/Golish-Platform/commit/6acabe45e841a060af87e6a143ca2531caf0f88f))
* **harness:** full stage reset that purges discovered facts + rewind-only stage picker ([08fab8c](https://github.com/ChristopherZh-7/Golish-Platform/commit/08fab8c6f51b9a5c00d5fa3b8426f356cfe82123))
* **harness:** gate-review follow-ups — GateContextBuilder + note/reason_kind + failure≠empty + technique_outcomes provenance ([48d1429](https://github.com/ChristopherZh-7/Golish-Platform/commit/48d1429089974bbc2484a532fd24ef630bfc81a4))
* **harness:** headless single-stage runner (golish --stage-run) ([22e1cba](https://github.com/ChristopherZh-7/Golish-Platform/commit/22e1cba37301e46297c13bd21ce6f3648d3fae4e))
* **harness:** host-aware coverage 2c-1 — authoritative asset type axis ([a409d73](https://github.com/ChristopherZh-7/Golish-Platform/commit/a409d732b1c8b590cf84586a0000cd82c5d72f36))
* **harness:** host-aware coverage 2c-3b — require RDNS + IP-WHOIS for IP assets ([a3bb618](https://github.com/ChristopherZh-7/Golish-Platform/commit/a3bb618c5cb7daff014e838668df6259ba590ba2))
* **harness:** host-aware coverage type-axis, stage tool listing, recon tooling ([653f998](https://github.com/ChristopherZh-7/Golish-Platform/commit/653f998a6bc3c4e21c1da10877c4a2910e38f865))
* **harness:** inject human-scope-approval gate for scoping per profile ([2854689](https://github.com/ChristopherZh-7/Golish-Platform/commit/2854689c654a0a60911c9a3fd680ae53f7602085))
* **harness:** live-wire eval/guardrail/rag_prior into the runtime ([e7ed484](https://github.com/ChristopherZh-7/Golish-Platform/commit/e7ed484084c9ff688d4690e6b29ac40c115d9bad))
* **harness:** make external_attack_surface target-touching only; inherit subdomains from target_intel (task 2/5) ([e546fac](https://github.com/ChristopherZh-7/Golish-Platform/commit/e546fac59911b9d95bfff64f51ed277a96f7c831))
* **harness:** make passive subdomain enum a target_intel min-invocation floor (task 3/5) ([9a7f4ef](https://github.com/ChristopherZh-7/Golish-Platform/commit/9a7f4ef465cd6a4c6ca3eacb72a8e6557755fa98))
* **harness:** make red_team scoping reliable on Xiaomi MiMo end-to-end ([1e2c374](https://github.com/ChristopherZh-7/Golish-Platform/commit/1e2c374ccba67c7a1fd8415777b14cd9aa844fbb))
* **harness:** merge DB-truth facts into coverage gate + diagnostic reflector (PR-A/C) ([df69fde](https://github.com/ChristopherZh-7/Golish-Platform/commit/df69fde0f3970485f8992e01beee65966ae629c6))
* **harness:** Operation DAG layer-2 engine + gate-driven stage cursor ([6b97d57](https://github.com/ChristopherZh-7/Golish-Platform/commit/6b97d57c9dce08fa2d43c38144b3d4d2841b74af))
* **harness:** P1 vendor metalcraft graph engine + checkpoint/resume ([84077a9](https://github.com/ChristopherZh-7/Golish-Platform/commit/84077a93d5ec7e9f12645d485b7820253c0d61fb))
* **harness:** P2 config-driven verification gate (a/b) ([0f19e55](https://github.com/ChristopherZh-7/Golish-Platform/commit/0f19e55c394191b42f2328181112032815542a94))
* **harness:** P2 increment 4a — StageRunner abstraction for executor-driven flow ([95ff8a7](https://github.com/ChristopherZh-7/Golish-Platform/commit/95ff8a7544462ff4aabf8b66eb28790087206efc))
* **harness:** P2 increment C-1 — group subtasks by stage (executor-driven run foundation) ([cc9bf69](https://github.com/ChristopherZh-7/Golish-Platform/commit/cc9bf69bc769a74bc450ca5bbab38114b78f1a7b))
* **harness:** P2 metalcraft graph-flow module + conditional routing (increment 1) ([2b57616](https://github.com/ChristopherZh-7/Golish-Platform/commit/2b5761657ea684d5926ef57d732d8105dd2335fe))
* **harness:** P2 wire flag-gated graph-flow routing into live transitions (increment 2) ([8f7c063](https://github.com/ChristopherZh-7/Golish-Platform/commit/8f7c06348ab3a859f53ac2d6cfe27f323d35a0ed))
* **harness:** P2-c doer-quality eval (rule-based, borrow Heartbit) ([b97c616](https://github.com/ChristopherZh-7/Golish-Platform/commit/b97c616f396c4dd3dd349ab0edfbce794d75ec7a))
* **harness:** P2-d tool I/O guardrails (rule-based, borrow AutoAgents/OpenFang) ([de5fd4d](https://github.com/ChristopherZh-7/Golish-Platform/commit/de5fd4d09cfe1d87cedd6fbb9e43624ca43b3c1d))
* **harness:** P3 RAG prior + knowledge-graph prior + continuous feedback ([95e98df](https://github.com/ChristopherZh-7/Golish-Platform/commit/95e98df6a582a07b9312113ba24255abcf56078a))
* **harness:** per-dimension intel/EAS coverage freshness + slim deliverable ([649fef1](https://github.com/ChristopherZh-7/Golish-Platform/commit/649fef1e6dde9ac286c96cf3193c00eb1221d66c))
* **harness:** per-org authoritative stage gate + fan-out pass_token closure ([15f88c3](https://github.com/ChristopherZh-7/Golish-Platform/commit/15f88c3a163c4763556cc740d5645c0a4bb56823))
* **harness:** per-run AI traceability — sub-agent prose, gate gap matrix, run_tree tool ([a1f2b06](https://github.com/ChristopherZh-7/Golish-Platform/commit/a1f2b06d45ae0bbdc050b724e0e041ef2b6e4f2a))
* **harness:** persist engagement org binding in operation_state (core 0) ([3b6f188](https://github.com/ChristopherZh-7/Golish-Platform/commit/3b6f1880ef33ce106b854682f3fb0ae73f79a11e))
* **harness:** profile-driven resilience, task-mode lead-agent triage, lazy-plan design ([4046360](https://github.com/ChristopherZh-7/Golish-Platform/commit/404636005448a44823df5b539dbad6e1ce6e9200))
* **harness:** re-anchor external_attack_surface stage to Target Surface Workbench ([7a6ef85](https://github.com/ChristopherZh-7/Golish-Platform/commit/7a6ef85f5d4704bf1405448dfbe873f7cb6b312e))
* **harness:** redteam Phase 1+2 — DB-truth landing points + subsidiary scoping gate ([3155e23](https://github.com/ChristopherZh-7/Golish-Platform/commit/3155e230481a73f7e2a926e0ab293f7a7388d446))
* **harness:** resume operation from checkpoint; interrupted marked Waiting ([0915859](https://github.com/ChristopherZh-7/Golish-Platform/commit/0915859b6df22257dde8877545849df10dca9114))
* **harness:** select operation profile from chat-panel mode picker ([1300216](https://github.com/ChristopherZh-7/Golish-Platform/commit/13002167b3f5b21c30f20b855f6208b4793a5af6))
* **harness:** technique-aware gate ops — schema validation, derive, corroborate (P5 Tasks 2-5) ([548fc2a](https://github.com/ChristopherZh-7/Golish-Platform/commit/548fc2a207da4de5e061206bd624ca3a42ac3089))
* **harness:** thread engagement org id through to the agentic loop (core 1) ([f92d535](https://github.com/ChristopherZh-7/Golish-Platform/commit/f92d535f95c4a3ff38159bbcb62d4edda57aba8a))
* **harness:** two-level phase model (Phase × Stage) — phase-flow + phase-boundary approval (flag-gated) ([1fc9bd4](https://github.com/ChristopherZh-7/Golish-Platform/commit/1fc9bd49da148404ac0014584e41f68aacb90e22))
* **harness:** unified AI + harness observability (P1 + additive P2 events) ([8624117](https://github.com/ChristopherZh-7/Golish-Platform/commit/8624117532efbde305da50253fa97b3d8cb463e2))
* **harness:** vuln_triage technique matrix + denominator coverage + Phase 2 taxonomy & gate-context seam ([3201635](https://github.com/ChristopherZh-7/Golish-Platform/commit/320163548f3415cee9b068d895adb52b2d1fceac))
* **harness:** weak-model submit channel — targeted repair + tool_choice lock ([104d02d](https://github.com/ChristopherZh-7/Golish-Platform/commit/104d02d742daf9cfceff2d03693cb04156380d86))
* **harness:** wire ctfr (CT) and asnmap (ASN) as target_intel coverage producers ([89e6e7f](https://github.com/ChristopherZh-7/Golish-Platform/commit/89e6e7ffd1d48823f2f34e7022e5cae31d9b23f3))
* **harness:** wire P0 evidence ledger write loop + anti-fabrication gate ([b2247e7](https://github.com/ChristopherZh-7/Golish-Platform/commit/b2247e7eb53d059b9ac6197f136b4b7b3dfca13b))
* **harness:** wire PreActionAuthorizer into per-tool dispatch (C3) ([3a06265](https://github.com/ChristopherZh-7/Golish-Platform/commit/3a062650b2931dceed675f0d6cd16c3cc00a8cd9))
* **harness:** wire technique derive+corroborate into target_intel + teach it in charter (P5 Tasks 6,8) ([bcbed37](https://github.com/ChristopherZh-7/Golish-Platform/commit/bcbed3730ae95f60d1e8c7acef60429aab1d008b))
* **hypothesis:** add atomic registry repositories ([1fcdff2](https://github.com/ChristopherZh-7/Golish-Platform/commit/1fcdff2b817ec6ab6770dadcd0f982927f8610d6))
* **hypothesis:** add deterministic identity reducer ([653c540](https://github.com/ChristopherZh-7/Golish-Platform/commit/653c5408bbb1b0db14b3f08809b08176ef4e2a31))
* **hypothesis:** enforce candidate analysis gate ([2fabb09](https://github.com/ChristopherZh-7/Golish-Platform/commit/2fabb09473a10a47a4057d2a2d6b0820aacfe096))
* **integrations:** add CaptureRecipe + CaptureRule schema (T1.1) ([a7eb697](https://github.com/ChristopherZh-7/Golish-Platform/commit/a7eb69759f8652f9315f5b2e17190b5d7dbcc02c))
* **integrations:** finish Phase 1 backend — capture types/errors/cross-validation (T1.2-T1.4) ([54cd141](https://github.com/ChristopherZh-7/Golish-Platform/commit/54cd1411ee2c34a05c3f095bf49368d4a08fa2a9))
* **integrations:** mirror capture types to frontend (T1.5) ([70332d5](https://github.com/ChristopherZh-7/Golish-Platform/commit/70332d5820f5c8a81f6de50d36af7ab89664ad15))
* **integrations:** schema-driven external-service credential management ([29e488b](https://github.com/ChristopherZh-7/Golish-Platform/commit/29e488b21368fb37773672eff788f20ceba85bbc))
* **integrations:** wire real exec resolver + builtin dispatcher ([541f19c](https://github.com/ChristopherZh-7/Golish-Platform/commit/541f19ce2b2dcefed6dd45678046443782d724c0))
* **intel-providers:** ASM platform integration with 0.zone first impl ([8a0c7a6](https://github.com/ChristopherZh-7/Golish-Platform/commit/8a0c7a62c24730b76c8b8494520edcac831549ab))
* **intel-providers:** FOFA + 360 Quake full impl with API docs alignment ([17fff5e](https://github.com/ChristopherZh-7/Golish-Platform/commit/17fff5e9898fc0f03db5984e88ae53c5cced828a))
* **intel-providers:** Hunter + Shodan full impl with API docs alignment ([fe70ece](https://github.com/ChristopherZh-7/Golish-Platform/commit/fe70ecef5df721d73137456773e6a90ce1de2133))
* **investigation:** define frozen rollout contract ([1ed13b3](https://github.com/ChristopherZh-7/Golish-Platform/commit/1ed13b38e6800572e5b3245171b416e812fa16f3))
* **investigation:** freeze operation rollout mode ([4f55525](https://github.com/ChristopherZh-7/Golish-Platform/commit/4f555250fa7ed17d84eaa093291fc1d7f7a9af26))
* **investigation:** recover and close unified full chain ([6aeec33](https://github.com/ChristopherZh-7/Golish-Platform/commit/6aeec339cdeb83e5a502cb48139564531d839bd4))
* **investigation:** seal Plan B production runtime ([86998eb](https://github.com/ChristopherZh-7/Golish-Platform/commit/86998eb8ff109ca5056c6d4a1b3df36c25e2f1af))
* IP-centric asset model (Phase 0/1) + host-aware coverage (2a) ([342eb54](https://github.com/ChristopherZh-7/Golish-Platform/commit/342eb54a378946afb36410a7c7ce4f9693f51e66))
* **kg/frontend:** typed SDK for kg_list_entities / kg_search / kg_neighbors ([70d5949](https://github.com/ChristopherZh-7/Golish-Platform/commit/70d59491332a1eace16781b4b81898e1476b5fb1))
* **kg/ui:** Knowledge Graph snapshot card in Advanced Settings ([60242af](https://github.com/ChristopherZh-7/Golish-Platform/commit/60242afaba73fc35c824c664b8a91097f4e76185))
* **kg:** also auto-extract entities from run_pty_cmd stdout ([613619f](https://github.com/ChristopherZh-7/Golish-Platform/commit/613619f443e1320316712862318e7cedce0dbdeb))
* **kg:** derive deterministic co-occurrence edges on auto entity extraction ([d6bb46e](https://github.com/ChristopherZh-7/Golish-Platform/commit/d6bb46ed9a6fa6dea1de500f4557c4faf3aec185))
* **kg:** inject KG context into sub-agent briefings + frontend query commands ([fa3e457](https://github.com/ChristopherZh-7/Golish-Platform/commit/fa3e4579b86d1e1ddfe4e0f8a00b09ab1e213223))
* **kg:** regex auto-extract IP/CVE/URL from sub-agent responses ([76a2a2d](https://github.com/ChristopherZh-7/Golish-Platform/commit/76a2a2d9f1fea53cc620b8cff210a0654eed3171))
* make EAS service fingerprinting adaptive ([72afe22](https://github.com/ChristopherZh-7/Golish-Platform/commit/72afe22ddc986bab130a94b83d471a320e2ef31b))
* **orchestrator:** wire stage harness gate into execute_single_subtask (Task 1c.6) ([1b0a23e](https://github.com/ChristopherZh-7/Golish-Platform/commit/1b0a23ef402b2e3e882edfcc5aabac6ca1fb3669))
* **pentest-domain:** canonical_asset_key + migrate AssetClass (E1 PR-A) ([39bd87d](https://github.com/ChristopherZh-7/Golish-Platform/commit/39bd87d68037e271957b74b35d277385b0b2ebd7))
* **pentest:** add manage_organizations agent tool ([fa0925e](https://github.com/ChristopherZh-7/Golish-Platform/commit/fa0925e923672d5555b9f151de978753767da9f7))
* **pentest:** java/node/python runtime install & version management ([9a77bdb](https://github.com/ChristopherZh-7/Golish-Platform/commit/9a77bdb1217775e0fc9c9e16d2ec02a53ca70fc6))
* **pentest:** manage_targets supports scope/organization_id + set_scope ([114ad19](https://github.com/ChristopherZh-7/Golish-Platform/commit/114ad193052e2b2711865056a471893f7bdc871a))
* **pentest:** tag tools by pentest phase + drop burpsuite-community ([6f482e6](https://github.com/ChristopherZh-7/Golish-Platform/commit/6f482e620d1a5c5a9379f83876de2e29c030ff6c))
* **pentest:** unify install proxy takeover (platform proxy = single source of truth) ([5e5581d](https://github.com/ChristopherZh-7/Golish-Platform/commit/5e5581dd70bdae5f2eb6e30f6e2945c451afe234))
* **plan-ui:** per-stage plan cards + operation roadmap ([679d9fd](https://github.com/ChristopherZh-7/Golish-Platform/commit/679d9fd9a9b6e3b4a245fba907330056fcb6130b))
* **plan-ui:** per-stage plan roadmap UX, persistence, and harness wiring ([352f205](https://github.com/ChristopherZh-7/Golish-Platform/commit/352f205da1e2f87b851ab56b07a4600d26d16795))
* **planner:** add PlanEventEmitter trait for broadcasting plan changes ([4220567](https://github.com/ChristopherZh-7/Golish-Platform/commit/42205677ccd691f04a765eee175538fb0b98061d))
* **planner:** apply_patch_ops + PlanPatchOp variants (P0-2 stage 2) ([65b02de](https://github.com/ChristopherZh-7/Golish-Platform/commit/65b02decb153e7ee3821dff69258f8510cccf2bf))
* **planner:** emit PlanUpdated event after load_from_db restore ([622f4e0](https://github.com/ChristopherZh-7/Golish-Platform/commit/622f4e09d0b8d1053dc9bcaf4db22af39bc6291c))
* **planner:** expose update_plan_patch tool to the LLM (P0-2 stage 3) ([cc93bb2](https://github.com/ChristopherZh-7/Golish-Platform/commit/cc93bb26fea8a5a95f6540033a428ad8873b2e81))
* **platform:** sync final branch updates ([41845f8](https://github.com/ChristopherZh-7/Golish-Platform/commit/41845f83e23081248f2ed0964fc88c58b5a6533d))
* **recon:** 0.zone asset-intel partial-success/retry + WeChat official-account mapping ([3ea3466](https://github.com/ChristopherZh-7/Golish-Platform/commit/3ea3466d99903d3279052e363cccc28970f182e4))
* **recon:** host-aware coverage 2c-3a collectors (reverse-DNS + IP-WHOIS) ([83c45c0](https://github.com/ChristopherZh-7/Golish-Platform/commit/83c45c0df0038cc26af1f86ddc9f8c544aa5faef))
* **recon:** implement organization recon closed-loop design and orchestration ([741fdd1](https://github.com/ChristopherZh-7/Golish-Platform/commit/741fdd1710eebf0177ec9dcfe175cb4ea151abe0))
* **recon:** land passive CNAME/MX/TXT DNS records in target_intel ([5f168e3](https://github.com/ChristopherZh-7/Golish-Platform/commit/5f168e3014e94561d5e89f96c39ef0fcf3003dc1))
* **recon:** land passive per-host port/service into target_assets ([993eab5](https://github.com/ChristopherZh-7/Golish-Platform/commit/993eab542fb7828869ce8107ce2c82acbb55d44b))
* **recon:** land target_intel coverage on the agent enrich path (PR1) ([d99f248](https://github.com/ChristopherZh-7/Golish-Platform/commit/d99f248d0c88ee121ace8dbbda7b3c044deb0db7))
* **recon:** OSINT and dork tool configs + brave_search dork strategy ([8f47a0f](https://github.com/ChristopherZh-7/Golish-Platform/commit/8f47a0f62df6145b11e869070c583fb564588a8e))
* **recon:** split enrich into recon_map_assets + recon_lookup_whois tools ([f314fa9](https://github.com/ChristopherZh-7/Golish-Platform/commit/f314fa96ec71878d699e64f1f534773e8ca9d47c))
* retire legacy company stage runtime ([a56a7cd](https://github.com/ChristopherZh-7/Golish-Platform/commit/a56a7cd87d934987f0b6da64e46ce9b54b4e117f))
* **runtime:** expose manage_organizations to task specialists ([e72e602](https://github.com/ChristopherZh-7/Golish-Platform/commit/e72e602401a6dbcc8ea9f8f34251e0de38b392cb))
* **scope-review:** candidatesToUnitRows maps DB candidates to review rows (T1) ([3d0bd02](https://github.com/ChristopherZh-7/Golish-Platform/commit/3d0bd02667076b4d195853a9273fd3dbced16440))
* **scoping:** subsidiary unit_review HITL — countdown, DB fallback, threshold filter ([53c3d25](https://github.com/ChristopherZh-7/Golish-Platform/commit/53c3d2561a1d3b5ff6909fb47a5d6f94116668ad))
* **stage_run:** re-run specialist with gate feedback until org passes (Phase 2 闸1) ([2a5fc94](https://github.com/ChristopherZh-7/Golish-Platform/commit/2a5fc9489db37222007f3152b39bb9086add7af8))
* **stage-run:** backend engine — per-org specialist fan-out + progress events + frontend wiring ([d613cc1](https://github.com/ChristopherZh-7/Golish-Platform/commit/d613cc19cd8e99fe315cc5379fc2e796d31d52c0))
* **stage-run:** per-subsidiary multi-org runs (redteam Phase 3) + subsidiary CLI + session-id unification + deepseek arm ([3717bda](https://github.com/ChristopherZh-7/Golish-Platform/commit/3717bdab53e6258894c411b4ae22385316f25b09))
* **stage-run:** per-subsidiary progress eprintln in fleet fan-out ([cecab77](https://github.com/ChristopherZh-7/Golish-Platform/commit/cecab77fc3c551f14f8e3c996231e2a51f3fd372))
* **stage-run:** persist live stage_run fan-out snapshot across restart ([fbf49af](https://github.com/ChristopherZh-7/Golish-Platform/commit/fbf49afb26e8331846f18da3277bfdc39ddc7b33))
* **stream-retry:** rate-limit evidence_read tool calls (Task 1b.3) ([ffee39a](https://github.com/ChristopherZh-7/Golish-Platform/commit/ffee39ac5be83ec8c7ed25b505fa91e65c5190cf))
* **target-panel:** delete all targets in a group ([cb8d5d3](https://github.com/ChristopherZh-7/Golish-Platform/commit/cb8d5d3dc0e380cecdbd34b2e152f5a5fdb9e52c))
* **targets:** add engagement workspace flow ([dc2ddf0](https://github.com/ChristopherZh-7/Golish-Platform/commit/dc2ddf0e00fed1da8aaba4e4d65a003c4b931099))
* **targets:** add Target Surface Workbench + redesign topology graph ([95ad45e](https://github.com/ChristopherZh-7/Golish-Platform/commit/95ad45e54bd33c31a0980ea917379dcfa2217082))
* **targets:** S1 organization grouping with tree view + grp end-to-end ([943a36d](https://github.com/ChristopherZh-7/Golish-Platform/commit/943a36d512c779bde789187c5c622e7934bd5685))
* **targets:** S2 owner + time_window + ProjectInfoPanel + engagements table ([5d3957a](https://github.com/ChristopherZh-7/Golish-Platform/commit/5d3957abdd2c1c8722293d7f082f027b1def36f6))
* **targets:** S3 organizations tree table + OrganizationsPanel + grp migration ([9d309da](https://github.com/ChristopherZh-7/Golish-Platform/commit/9d309da8a0a4f4388f9e62bb60d5f05e6e454cc9))
* **task-plan:** fallback fetch on session activate so restored plan shows up ([d769118](https://github.com/ChristopherZh-7/Golish-Platform/commit/d769118a5ee958bbe489836a113c443252dcf098))
* **task-plan:** surface failure_kind via category badge in InlinePlanCard ([c265872](https://github.com/ChristopherZh-7/Golish-Platform/commit/c2658724ddebcd4a53e0f50a4c9fe12f87ea18d0))
* **telemetry:** per-run run.log debug layer (one readable file per run) ([e7b62d1](https://github.com/ChristopherZh-7/Golish-Platform/commit/e7b62d12e14acf57e4b06f1afe8cc39cea402061))
* **terminal:** merge GridTerminal recovery stack ([819c3e4](https://github.com/ChristopherZh-7/Golish-Platform/commit/819c3e43474f96d5ab9e3990441bbb0bcaa77d49))
* **tool-manager:** queue installs, auto-pick single entrypoint, live refresh ([eda6eae](https://github.com/ChristopherZh-7/Golish-Platform/commit/eda6eae55db703072d641da6f107fb62f3f46cfa))
* **tool-truth:** add coverage status ontology ([389440d](https://github.com/ChristopherZh-7/Golish-Platform/commit/389440d3e9653668f3a1a44c07d0ff01769c7a2f))
* **tool-truth:** add producer receipt lifecycle ([3dd673c](https://github.com/ChristopherZh-7/Golish-Platform/commit/3dd673c6587d3753af3f49e7291500fed7b0f03a))
* **tool-truth:** freeze receipt contract per operation ([234639d](https://github.com/ChristopherZh-7/Golish-Platform/commit/234639d989ee7b1717bb647e677d1d40c1fa90cc))
* **tool-truth:** persist execution receipts and reconciliation ([1705f51](https://github.com/ChristopherZh-7/Golish-Platform/commit/1705f51716b839bcf901faa74af6bc95d1b7bd97))
* **tool-truth:** revalidate stale evidence with bounded obligations ([383ffa8](https://github.com/ChristopherZh-7/Golish-Platform/commit/383ffa877693720d3672f934ca6d2c8495158332))
* **tool-truth:** seal coverage and shadow gate grades ([cf6041c](https://github.com/ChristopherZh-7/Golish-Platform/commit/cf6041cc90454db52d372a3a89986bb19f79b81d))
* **toolsconfig:** github-source layout for ctfr/asnmap + asnmap output parsing ([88303f5](https://github.com/ChristopherZh-7/Golish-Platform/commit/88303f5c138f4fbb76b49f7b961167dbbf044d37))
* **topology:** add lineage focus/isolate + cursor-anchored zoom ([555de7c](https://github.com/ChristopherZh-7/Golish-Platform/commit/555de7c9bbed79b24fce03e20828ad1535bb3c47))
* **types:** finish ts-rs cross-IPC type generation (P0-2) ([98beea9](https://github.com/ChristopherZh-7/Golish-Platform/commit/98beea9d88c1284f9409b6a85e209683a973a88d))
* **types:** wrap up ts-rs cross-IPC sync long-tail (A+B batch) ([e91bdb9](https://github.com/ChristopherZh-7/Golish-Platform/commit/e91bdb90cb5414f1371a4120739180e276af6ed2))
* **ui:** editable scope_review/unit_review confirmation table ([4922bc4](https://github.com/ChristopherZh-7/Golish-Platform/commit/4922bc41fcbdde0c4d211dd980c2f41cb583320e))
* **ui:** in-app window controls for Windows/Linux custom titlebar ([d77d2a2](https://github.com/ChristopherZh-7/Golish-Platform/commit/d77d2a2ef9dc40c219527fe3788f9a69c8e183ec))


### Bug Fixes

* **agent-runtime:** drop dead target_registered usage after pipeline-gate removal ([fea837e](https://github.com/ChristopherZh-7/Golish-Platform/commit/fea837e530252c71c078cbc7faba3ec64d6fb8ce))
* **agent-runtime:** harden the submit-only lock against tool-call escapes ([0ace3e4](https://github.com/ChristopherZh-7/Golish-Platform/commit/0ace3e4a00d36e1c55eb6ee2eda46bdbed241001))
* **agent:** coerce tool-call args to object so MiMo history replay stops 500ing ([e5afb4f](https://github.com/ChristopherZh-7/Golish-Platform/commit/e5afb4ff0a4454bd38a9b9b6b6aba004cb8eadb9))
* **agent:** enable hard mentor by default ([13b5bc6](https://github.com/ChristopherZh-7/Golish-Platform/commit/13b5bc680b60eb0559fa1bdc4004d49fd6b95353))
* **agent:** land harness and provider bug fixes ([34eb478](https://github.com/ChristopherZh-7/Golish-Platform/commit/34eb47801f9aa770ad811135783c8b24cbf80ca6))
* **agent:** observe sub-agent tool stalls ([7d6cdac](https://github.com/ChristopherZh-7/Golish-Platform/commit/7d6cdacc35886e2b2b0da157350446e134280cc4))
* **agent:** preserve targeted stage repair ([77868b4](https://github.com/ChristopherZh-7/Golish-Platform/commit/77868b469c7e500347aca0bc200fd35c0f0be874))
* **agent:** stop textual tool-call markup leaks and parse streamed string args ([fbc4538](https://github.com/ChristopherZh-7/Golish-Platform/commit/fbc4538cf3c378e2bf12e2e76fd6274ce3aa1194))
* **ai-chat:** hide duplicate footer during sub-agent delegation ([58176f3](https://github.com/ChristopherZh-7/Golish-Platform/commit/58176f304678128dd3159b8ad5c2daf4446477ec))
* **ai-chat:** keep active conversation tab in view on switch/add ([141accd](https://github.com/ChristopherZh-7/Golish-Platform/commit/141accdcf05ba50864f6c6e51b795b57e7ba160b))
* **ai-chat:** per-tool approval switch + strip DeepSeek tool-call leak ([01edd1e](https://github.com/ChristopherZh-7/Golish-Platform/commit/01edd1ebc35a1574915682742ef0ade4591f7b91))
* **ai-chat:** plan card, stuck spinner, and planning indicator ([0877192](https://github.com/ChristopherZh-7/Golish-Platform/commit/08771922f9fa6dfec983aabdf4adc4124a21f1c3))
* **ai:** align managed process yielding with codex ([16771f4](https://github.com/ChristopherZh-7/Golish-Platform/commit/16771f4b167b6e843212083943c3dc8ecc9cb674))
* **arch:** place golish-graphiti at L2 in DAG guard ([b0811ea](https://github.com/ChristopherZh-7/Golish-Platform/commit/b0811eabfd66210fa1b5532649fada83bd83603b))
* **asset-intel:** back off + retry HTTP 429 instead of dropping the request ([e7dc540](https://github.com/ChristopherZh-7/Golish-Platform/commit/e7dc54069d81307355a463fe6f91a24c47f2b7a9))
* **chat:** route AI command output to tool detail ([cc7890b](https://github.com/ChristopherZh-7/Golish-Platform/commit/cc7890b7bb474281ed2281c76446604c73246e92))
* **chat:** tighten sub-agent panel rendering ([b2a90f1](https://github.com/ChristopherZh-7/Golish-Platform/commit/b2a90f17e0575fd87e1191d380806d087ac7f528))
* **cleanup:** preserve retained truth during organization deletion ([c886d3f](https://github.com/ChristopherZh-7/Golish-Platform/commit/c886d3f61d4380415d42fddf0d757341285f8a03))
* close EAS CLI resume loop ([9c273ef](https://github.com/ChristopherZh-7/Golish-Platform/commit/9c273ef6a10137e6303453daa24849ba4ec0e067))
* **core:** treat terminal status/flags as failure in is_tool_result_success ([e67b3aa](https://github.com/ChristopherZh-7/Golish-Platform/commit/e67b3aab1828a1534c5a2214dc1311c7137ec532))
* **db-truth:** guard jsonb_array_length against non-array JSONB in coverage gate ([e25da49](https://github.com/ChristopherZh-7/Golish-Platform/commit/e25da493f8c35e0b6ecdb7c7ed6d22f68008cb51))
* **db:** repair_migrations also drops phantom _sqlx_migrations records ([51c6129](https://github.com/ChristopherZh-7/Golish-Platform/commit/51c61292c12caa5775f74b06a27abbd0d4951219))
* **dispatch:** non-UUID session ids return empty list instead of erroring ([6417404](https://github.com/ChristopherZh-7/Golish-Platform/commit/6417404f85796c3d23db44b49de72587b1bf147d))
* **eas:** align service fingerprint gate timing ([cd4a1f9](https://github.com/ChristopherZh-7/Golish-Platform/commit/cd4a1f96a36997d05c65dae6106b5fe18edb8234))
* **eas:** clarify service fingerprint tool routing ([ad8bb44](https://github.com/ChristopherZh-7/Golish-Platform/commit/ad8bb444083b6c19238ed36afc9783a3a0edd8f2))
* **eas:** require per-input proof for empty output ([cc34590](https://github.com/ChristopherZh-7/Golish-Platform/commit/cc34590fc2fbeef162cc1bb456db9be84562ca38))
* **enumeration:** keep preflight failures nonterminal ([1f541d8](https://github.com/ChristopherZh-7/Golish-Platform/commit/1f541d85b3945c35476bd8a428f69dab2d5464b5))
* **frontend:** AI chat panel UX polish + tool-output rendering ([f175afd](https://github.com/ChristopherZh-7/Golish-Platform/commit/f175afdd670b4dd80e4b0ccbaa5b999b5dd8d715))
* **frontend:** batch streaming detail output ([1fb24e5](https://github.com/ChristopherZh-7/Golish-Platform/commit/1fb24e5897c76872f7f7ee924560f45240464730))
* **frontend:** bound tool-result serialization to prevent OOM ([1dd49bc](https://github.com/ChristopherZh-7/Golish-Platform/commit/1dd49bcdd6c6ed5c6deac5fa8dfc2eb98751dc82))
* **frontend:** drop biome assist on ts-rs generated dir; sort pentest types ([1c523e2](https://github.com/ChristopherZh-7/Golish-Platform/commit/1c523e2b99dcae37b3d82d1dc166222bad6aafad))
* **frontend:** interleave thinking by content offset + Cursor-style ask_human options ([0cf168c](https://github.com/ChristopherZh-7/Golish-Platform/commit/0cf168c18eab9a0b6313edc0aaa0b3cfacb81ca0))
* **frontend:** interleave thinking, beautify tool output, surface task-mode stage boundaries ([5fe447d](https://github.com/ChristopherZh-7/Golish-Platform/commit/5fe447d51bd951cde29bc95aacf9f56f2c539b2e))
* **frontend:** prevent dev white-screen flash on cold load ([2d722f5](https://github.com/ChristopherZh-7/Golish-Platform/commit/2d722f5fac1188246fa7ac1a01bcf015ce0a31f3))
* **frontend:** task-mode preparing indicator + chat/tool rendering & i18n tweaks ([54a5e6a](https://github.com/ChristopherZh-7/Golish-Platform/commit/54a5e6a37294883fca9c59cb6e7b2efc11b90295))
* **harness:** align stage charter + EAS subtask prompt with touches-target boundary (task 4/5) ([64c53f8](https://github.com/ChristopherZh-7/Golish-Platform/commit/64c53f828d551c7e54bd0aa49eee9f4a150ccefc))
* **harness:** allow zero-touch whois in target_intel (recon/whois tool type) ([75641e3](https://github.com/ChristopherZh-7/Golish-Platform/commit/75641e3c1e1a36f064ed7190e2d21be7c332e82c))
* **harness:** backfill engagement org on manage_targets add ([a5756a9](https://github.com/ChristopherZh-7/Golish-Platform/commit/a5756a9ff73c98c861166d6833cf5535fae3ecce))
* **harness:** block empty coverage matrix on stages that declare expected_techniques ([6a688cb](https://github.com/ChristopherZh-7/Golish-Platform/commit/6a688cbf080c15d10ffe0d16b01736761c6daa17))
* **harness:** break sub-agent loop on repeated identical stage BLOCK ([004b4a8](https://github.com/ChristopherZh-7/Golish-Platform/commit/004b4a877f4369f5a36d1c500302de9e9935c161))
* **harness:** canonicalize asset identity in coverage gate join (PR-B B0/B1a/B2) ([abb5bec](https://github.com/ChristopherZh-7/Golish-Platform/commit/abb5bec1f6d8d6518dd222e86b5d7baa90dd4852))
* **harness:** classify passive OSINT tools + add allowed_tool_names lookup ([2b9b137](https://github.com/ChristopherZh-7/Golish-Platform/commit/2b9b13784913c2f8a784ecf9adc2a0b2dd354117))
* **harness:** credit ledger evidence-facts in submit-gate preview ([cfe73c3](https://github.com/ChristopherZh-7/Golish-Platform/commit/cfe73c3818f4cffd4d7058eb9feb4953390aaad2))
* **harness:** get_plan returns empty plan for uninitialized session + monotonic legacy final plan version (P2) ([78ef07f](https://github.com/ChristopherZh-7/Golish-Platform/commit/78ef07f2e7ace320d643116209e6239bb58931ab))
* **harness:** land EAS active probe results ([b089d4e](https://github.com/ChristopherZh-7/Golish-Platform/commit/b089d4e0e52e4c388430934898b14f1eb530bb65))
* **harness:** make background scan waits explicit ([86f5028](https://github.com/ChristopherZh-7/Golish-Platform/commit/86f502866d66e45771ccd78e31b195f6f9befb6d))
* **harness:** make phase-approval gate clickable + rework on rejection ([ad6019f](https://github.com/ChristopherZh-7/Golish-Platform/commit/ad6019f34ea9389b22c07a55576b4988607fa46e))
* **harness:** make submit-tool side-channel authoritative at stage close (PR1) ([90df7ab](https://github.com/ChristopherZh-7/Golish-Platform/commit/90df7abff1986a09405fbd3868f9fe6d343c5e81))
* **harness:** parse quoted tool path in passive-intel attribution ([76d8897](https://github.com/ChristopherZh-7/Golish-Platform/commit/76d8897e993537769ee705f0e0214e08bf005b2c))
* **harness:** persist HarnessTrace events to the transcript ([ad95043](https://github.com/ChristopherZh-7/Golish-Platform/commit/ad950431b7bc3f7fa61eb66c913424a171edac75))
* **harness:** resolve transcript base from workspace for reads ([5254e25](https://github.com/ChristopherZh-7/Golish-Platform/commit/5254e25143802595ba945ce0b987b2100758c27c))
* **harness:** stabilize EAS worker status and evidence handling ([e7d1ece](https://github.com/ChristopherZh-7/Golish-Platform/commit/e7d1ece9378942a99c4f3f9b30bc8aa90a3bea9a))
* **harness:** stop scoping over-run, collapse confirm stages, hide offensive sub-agents ([11d47ad](https://github.com/ChristopherZh-7/Golish-Platform/commit/11d47ad8aa4623265d187888260dc90d3e23c85d))
* **harness:** stop target_intel coverage gate dead-loop ([bb25a3f](https://github.com/ChristopherZh-7/Golish-Platform/commit/bb25a3f96921bc17351e9e001d1c7fc3ffbdc8c9))
* **harness:** target_intel resume-skip + OSINT-required prompts (run review [#3](https://github.com/ChristopherZh-7/Golish-Platform/issues/3), [#6](https://github.com/ChristopherZh-7/Golish-Platform/issues/6)) ([3299c59](https://github.com/ChristopherZh-7/Golish-Platform/commit/3299c59857026480913f9b557ecc16883889d947))
* **harness:** writeback subtask/plan progress + reap zombie tasks on executor-driven run (P0+P1) ([a055f96](https://github.com/ChristopherZh-7/Golish-Platform/commit/a055f96387e992dcf1ef3a5d603d318288f8ce8e))
* **integrations:** align ENScan schema + harvest full baidu Cookie header ([40a0e18](https://github.com/ChristopherZh-7/Golish-Platform/commit/40a0e18b939e815cf4b919d11db8bb2298a6a154))
* **integrations:** resolve clippy doc_lazy_continuation in test module docs ([d023386](https://github.com/ChristopherZh-7/Golish-Platform/commit/d02338695d610fdb867af4f9dea0980f5e17cbbd))
* **intel-providers:** expose Intel Providers section in Settings dialog ([57de7e9](https://github.com/ChristopherZh-7/Golish-Platform/commit/57de7e9bb4ab8b1c4a5976ece9de59c7974da59a))
* **llm:** bump deepseek-v4 context to 1M / output to 64K ([da74bd1](https://github.com/ChristopherZh-7/Golish-Platform/commit/da74bd16886b736bd316a14ecbbe628bd3935278))
* **merge:** drop leftover conflict markers in direct.rs ([bc7bafe](https://github.com/ChristopherZh-7/Golish-Platform/commit/bc7bafe28844ab20d92c79505f69d55b34418bb5))
* **nuclei:** treat scanner no-match as inconclusive ([ada624d](https://github.com/ChristopherZh-7/Golish-Platform/commit/ada624d37dbb01481334e6ea1cd3de24976b315b))
* **nvidia-nim:** prune 15 undeployed model IDs and rewrite Go-default-404 error ([7606b21](https://github.com/ChristopherZh-7/Golish-Platform/commit/7606b21848c783506be47fc63a2c211a6515b10a))
* **observability:** surface harness::* gate/eval logs via a harness log directive ([85fafad](https://github.com/ChristopherZh-7/Golish-Platform/commit/85fafad4dfba64c23307ceb8bff2e88107571d5a))
* **organizations:** accept snake_case keys in profile patch (I3) ([f329be5](https://github.com/ChristopherZh-7/Golish-Platform/commit/f329be5cf20afb2ebd9e0266bf21e54b75e7f116))
* **planner:** persist apply_patch_ops snapshots to DB ([0182b01](https://github.com/ChristopherZh-7/Golish-Platform/commit/0182b01804d9e781b8e6ccbec85f920ab20c010f))
* **provider-config:** add model_override to Deepseek variant ([6f6293b](https://github.com/ChristopherZh-7/Golish-Platform/commit/6f6293b482c582757c799dcf674cd8e517a157e1))
* **recon:** drive target_intel via asset_intel engine on MiMo sub-agents ([8c578f8](https://github.com/ChristopherZh-7/Golish-Platform/commit/8c578f8af48ba0739809ee6394fc36254bca0845))
* **recon:** land in-scope subdomain targets into target_assets ([0399e3c](https://github.com/ChristopherZh-7/Golish-Platform/commit/0399e3cda34b9f867be607372e0b69d28c672928))
* **recon:** land target_intel intel into gate-read tables (SUBDOMAIN/DNS/CT/WHOIS) ([b927a05](https://github.com/ChristopherZh-7/Golish-Platform/commit/b927a0592169300aa03f5ac96f4eded05b59e72b))
* **recon:** provider direct-landing to gate tables + Quake CT extraction ([edee750](https://github.com/ChristopherZh-7/Golish-Platform/commit/edee75065f64159d54ddbf96a4de85b5f7c5484b))
* **recordings:** scope recording commands to the active project (IDOR) ([a831631](https://github.com/ChristopherZh-7/Golish-Platform/commit/a8316318a708b0b766be671334e59333ac40453b))
* **scoping:** get-or-create in manage_organizations(action="create") ([fefe796](https://github.com/ChristopherZh-7/Golish-Platform/commit/fefe7964e29a1cae63e5706805596e95804310ca))
* **sensitive-scan:** scope ai_verdict update by project_path (I2) ([1255470](https://github.com/ChristopherZh-7/Golish-Platform/commit/1255470a1acef6465361365598b68e28854b8728))
* stabilize EAS restart recovery ([bc5534d](https://github.com/ChristopherZh-7/Golish-Platform/commit/bc5534dc10f100453356059c3b44ce2fda083fd1))
* **stage_run:** co-locate per-run run.log with transcript.json ([24ae0ae](https://github.com/ChristopherZh-7/Golish-Platform/commit/24ae0ae58ae6d98869a63b9e2f4a6500fd9f382b))
* **sub-agent:** route knowledge-base tools through ToolProvider ([fea60aa](https://github.com/ChristopherZh-7/Golish-Platform/commit/fea60aac2f1e0239c756620b1e4251f31f665d00))
* **sub-agents:** preserve model overrides across the DB-template reload race ([7f26ce2](https://github.com/ChristopherZh-7/Golish-Platform/commit/7f26ce2b56786fa7aa7888ae72b544b461ee29b6))
* **targets:** backfill grp field plumbing + 3 db_target_add call sites ([6842cee](https://github.com/ChristopherZh-7/Golish-Platform/commit/6842cee3b1b5be8054a635e8e09aeef8aa03caa2))
* **targets:** drop NOW() from engagements partial index (PG IMMUTABLE rule) ([c64edd8](https://github.com/ChristopherZh-7/Golish-Platform/commit/c64edd82fc6d1f73e4b60a34ccd3587e67b4591a))
* **task-mode:** resume-aware entry, upsert session by chat key ([3d9e3d1](https://github.com/ChristopherZh-7/Golish-Platform/commit/3d9e3d16591f1c43c9bdefb4a07c07ea9f8b3af8))
* **terminal:** answer Windows PowerShell DSR query + Format-Table ANSI rendering ([641e3f8](https://github.com/ChristopherZh-7/Golish-Platform/commit/641e3f8998eadf432f84f9017cc52abe9fd51cac))
* **tool-manager:** clone source when a github tool has no releases ([fcef331](https://github.com/ChristopherZh-7/Golish-Platform/commit/fcef3310c53b9803c55055634d6a7f9aab74e112))
* **tool-truth:** isolate target intel receipts by attempt ([ed044ea](https://github.com/ChristopherZh-7/Golish-Platform/commit/ed044ea7cc43b83af1de343f463850dda01cdc8f))
* **tool-truth:** keep positive signals separate from coverage ([fefbaec](https://github.com/ChristopherZh-7/Golish-Platform/commit/fefbaec3d6c41610a918c1efda5d4c0a1ff60127))
* **tool-truth:** seal multi-root authority bundles ([aa59782](https://github.com/ChristopherZh-7/Golish-Platform/commit/aa597823341654056e6b850d0845230d291ef0c8))


### Refactoring

* **agent-kit:** re-export AssetClass from pentest-domain (E1 PR-A) ([af06f91](https://github.com/ChristopherZh-7/Golish-Platform/commit/af06f91b60131e5d3494e0a638aa6d64fab27dc7))
* **agent-kit:** split planner/manager.rs into persistence/mutations ([616472b](https://github.com/ChristopherZh-7/Golish-Platform/commit/616472bd8c7ba3ae55ae4a192ac0324b7dcccc94))
* **agent-runtime:** split stream_processor into chunks/tests ([2c91682](https://github.com/ChristopherZh-7/Golish-Platform/commit/2c91682b63af0f1a82cb97a2b78c7beaaa4d0a41))
* **agent-runtime:** split tool_execution/direct.rs into sub_agent_call ([9a3272a](https://github.com/ChristopherZh-7/Golish-Platform/commit/9a3272a5c5d0dc18a3bb920bd0e0766f71c51726))
* **ai:** split ai/commands into bridge_config submodule ([77ac579](https://github.com/ChristopherZh-7/Golish-Platform/commit/77ac579137c522e53a33d3f729e658e693fed4a5))
* **ai:** split db_bridge DbRepoProvider impl by domain ([26c1553](https://github.com/ChristopherZh-7/Golish-Platform/commit/26c1553c00639c5b3205c677dee375e975b7b2c8))
* **ai:** split tracking_bridge by domain ([3508a5e](https://github.com/ChristopherZh-7/Golish-Platform/commit/3508a5e3dc21ddd3a59dc815c6ceac1df9b8857f))
* **arch:** inject PgVaultAdapter into vault/auth tools (S1-2a) ([1149ddb](https://github.com/ChristopherZh-7/Golish-Platform/commit/1149ddb39c23fc08f8b0c9a83dc8175e672fdeec))
* **arch:** route AuthProbeTool through VaultReadPort (S1-2a) ([1a7018b](https://github.com/ChristopherZh-7/Golish-Platform/commit/1a7018bfa5a191f9d8cd30943c3a1ebe4f9b9e33))
* **arch:** route VaultTool through VaultReadPort (S1-2a) ([1e162de](https://github.com/ChristopherZh-7/Golish-Platform/commit/1e162de8cd2afe17a9b66428db02f43c984ac9f1))
* **asset-intel:** split monolith into 9 submodules (logic + DTOs) ([1c409f0](https://github.com/ChristopherZh-7/Golish-Platform/commit/1c409f09b187f37abc132f961702e438799eaef6))
* **asset-intel:** split monolith into runtime/service/commands layers ([1ff31bc](https://github.com/ChristopherZh-7/Golish-Platform/commit/1ff31bcc99e04b299acbf8f178b7fa62b80bcc01))
* **core:** consolidate time/path/string helpers into golish-core ([6fa4cc3](https://github.com/ChristopherZh-7/Golish-Platform/commit/6fa4cc315b26ee73f852445ad3e4b7ff8eee6d8b))
* **db:** project-scoped CRUD helper + enforce IDOR scope (P1-1, P0-3, I2) ([30cb5e1](https://github.com/ChristopherZh-7/Golish-Platform/commit/30cb5e167cccf8905a48801e966d6032a696f3bb))
* **db:** split repo/audit.rs into pentest/queries/timeline submodules ([ead8b76](https://github.com/ChristopherZh-7/Golish-Platform/commit/ead8b76aa99e1218d5da45a0ae48c0774b1491c8))
* **db:** typed DbError + explicit golish-graphiti -&gt; golish-db dependency ([49cc135](https://github.com/ChristopherZh-7/Golish-Platform/commit/49cc135766add5137a768b76908a7656c75f967a))
* dedup AiProvider + vuln-intel types to single sources ([63d8903](https://github.com/ChristopherZh-7/Golish-Platform/commit/63d8903c33ac324d6b50cd77d4d093463767f7a1))
* **frontend:** add formatClockTime to lib/time and reuse in surfaceModel ([432ad09](https://github.com/ChristopherZh-7/Golish-Platform/commit/432ad09086de693eb5b43d2ff8667748e3edace1))
* **frontend:** consolidate duration/relative-time formatters into lib/time ([f2827a3](https://github.com/ChristopherZh-7/Golish-Platform/commit/f2827a357cce0e279c3ffaed21684759040ad61a))
* **frontend:** extract mock fixtures into mocks/fixtures.ts ([ed7f75c](https://github.com/ChristopherZh-7/Golish-Platform/commit/ed7f75cb30fe6f0cb188187360693e8951e24d9b))
* **frontend:** merge duplicate vuln research/links API wrappers ([e84f508](https://github.com/ChristopherZh-7/Golish-Platform/commit/e84f5081cd57bce77dfb7022e1a765f02281520a))
* **frontend:** route through typed API layer + tri-state (P0-4, M2) ([6065658](https://github.com/ChristopherZh-7/Golish-Platform/commit/60656586e1512890010ea919097dfff5f9414fdc))
* **frontend:** split mocks.ts into mocks/ modules (P2, partial) ([83a105c](https://github.com/ChristopherZh-7/Golish-Platform/commit/83a105cf3157b47c2f7e44cd353e5f773c1928b6))
* **harness:** drop harness feature flags, hardwire new paths ([80495e6](https://github.com/ChristopherZh-7/Golish-Platform/commit/80495e63f6aa1b6513fb15553f9841c5137dfa0b))
* **harness:** drop legacy subtask-loop path, make graph-flow the sole driver ([c48eb9b](https://github.com/ChristopherZh-7/Golish-Platform/commit/c48eb9b7a91c4a8f03756794d802e60279c31169))
* **harness:** remove required_checks fixed-menu; gate_rules is sole gate entry ([8453473](https://github.com/ChristopherZh-7/Golish-Platform/commit/845347365206388a9fdde58896b320abd94ecd4f))
* **harness:** reorganize stage resources into per-stage subdirs ([64da3ba](https://github.com/ChristopherZh-7/Golish-Platform/commit/64da3ba5c0f2516f70b5dc97e4ca6143f6d055d0))
* **integrations:** extract external_file inline tests to sibling file ([a70ab9e](https://github.com/ChristopherZh-7/Golish-Platform/commit/a70ab9eb7eb5ced40617b2ee052d5d9f9672e167))
* **integrations:** extract resolver inline tests to sibling file ([df6fbc0](https://github.com/ChristopherZh-7/Golish-Platform/commit/df6fbc01a11e1fec6e50ef5e65fc9163d52c9c21))
* **integrations:** split schema.rs into storage/test_kind/capture submodules ([9e83dfa](https://github.com/ChristopherZh-7/Golish-Platform/commit/9e83dfac99b1ef5b951e39aa58eb3f4d6a395393))
* **intel-providers:** extract shared HTTP client + decode helper ([3123c74](https://github.com/ChristopherZh-7/Golish-Platform/commit/3123c74cb0ae2a7a835281ac978a35a873cb6e2d))
* **models:** migrate all 12 LLM providers to JSON-driven registry ([2971ab6](https://github.com/ChristopherZh-7/Golish-Platform/commit/2971ab6f333846723939c1a09aa1c7b99050c5e0))
* **pentest-domain:** split models.rs into submodules (P2) ([a71319b](https://github.com/ChristopherZh-7/Golish-Platform/commit/a71319b765e62a6b6caab6e64f70023f29329c4d))
* **pentest:** dedup golish-pentest models/search into golish-pentest-domain ([4a0bde6](https://github.com/ChristopherZh-7/Golish-Platform/commit/4a0bde6238621633173b6ae90a41320a8eba303f))
* **pentest:** split output_store/organizations.rs into writers/tests ([230c53c](https://github.com/ChristopherZh-7/Golish-Platform/commit/230c53c9200e4029ad9ef53bd19b37ca608f7d22))
* **pipeline:** move execute_pipeline_inner into orchestrator/run.rs ([116fbf3](https://github.com/ChristopherZh-7/Golish-Platform/commit/116fbf3bfff054da10f2aee1204bc5a94e76569d))
* **pipeline:** remove DAG pipeline feature end-to-end ([0d3bd66](https://github.com/ChristopherZh-7/Golish-Platform/commit/0d3bd66f439956e5f77f06f4facf2c46436eb3d0))
* **pipeline:** split engine/steps/single.rs into ai_tool/exec ([7c3d4e5](https://github.com/ChristopherZh-7/Golish-Platform/commit/7c3d4e55d5c0c6b1be5d3af98a46e445f12e1961))
* **pty:** split manager/session_create.rs into util/reader/emitter_loop ([2f2d079](https://github.com/ChristopherZh-7/Golish-Platform/commit/2f2d079de26cd854d2c8dc5b43cf75d9e0dc4de8))
* **settings:** split schema/llm.rs into provider submodules ([929ec2e](https://github.com/ChristopherZh-7/Golish-Platform/commit/929ec2e1e741225abbb82483bd43cbe116e55c94))
* **stage-run:** converge CLI subsidiary fan-out onto run_fleet_scheduler ([b26375a](https://github.com/ChristopherZh-7/Golish-Platform/commit/b26375ade43c81689375811756f04b1add407de5))
* **stage-run:** drop engagement layer, converge on stage_run ([284979a](https://github.com/ChristopherZh-7/Golish-Platform/commit/284979a6cad092e1c53623f80a954879b28d0901))
* **target-panel:** extract org/target subcomponents from TargetGroupedView (P1-4) ([b03a51f](https://github.com/ChristopherZh-7/Golish-Platform/commit/b03a51f736d57b5b3f91e7a8ebafdb04448dcace))
* **target-surface:** split TargetSurfaceWorkbench into surface/ modules ([93f380b](https://github.com/ChristopherZh-7/Golish-Platform/commit/93f380b311c3b5f245f78655aace00cc3b486eeb))
* **targets:** unify panel + organization asset-intel profile ([96be059](https://github.com/ChristopherZh-7/Golish-Platform/commit/96be059815aa31af815395096e381c990d8dd3d6))
* **tests:** extract inline #[cfg(test)] modules into sibling _tests.rs ([8049196](https://github.com/ChristopherZh-7/Golish-Platform/commit/8049196f029514b18b9511505dd1a472ebd724dd))
* **tool-install:** extract pure install logic from useToolInstall ([a1c8127](https://github.com/ChristopherZh-7/Golish-Platform/commit/a1c812744868149a1b0ed51b9928f7028a47a677))
* **tools:** route residual scoped SQL through golish-db repo (P0-3b) ([06af27a](https://github.com/ChristopherZh-7/Golish-Platform/commit/06af27a99eb6bb716c92567f254bb8a1c1cbc690))
* **tools:** split asset_intel/runtime/cli.rs into stream submodule ([252b838](https://github.com/ChristopherZh-7/Golish-Platform/commit/252b838c66b565e46b516ca6bb39e9322a68bdbe))
* **tools:** split integrations/capture/engine.rs into submodules (P2) ([63c196e](https://github.com/ChristopherZh-7/Golish-Platform/commit/63c196e1f15324ab21b4a2af684a8cd096ba33f5))
* **tools:** split methodology into types/templates submodules ([7995e93](https://github.com/ChristopherZh-7/Golish-Platform/commit/7995e934f09cd3e30c7a5b70530a0b5c5a878168))
* **tools:** split pentest_bridge/js_collect.rs into submodules (P2) ([03871db](https://github.com/ChristopherZh-7/Golish-Platform/commit/03871dbfb68037efbef72995cec7959ac6552403))
* **tools:** split tools/organizations.rs into types/candidates/validation ([348346a](https://github.com/ChristopherZh-7/Golish-Platform/commit/348346a7d0453b989935d6d7a664190af8afcbfa))
* **types:** converge cross-IPC/cross-crate types to ts-rs single source (I5/I8) ([449a99a](https://github.com/ChristopherZh-7/Golish-Platform/commit/449a99a224699df77b1652bd328e5e59976ea32f))

## [0.2.43](https://github.com/golish-ai/golish/compare/v0.2.42...v0.2.43) (2026-03-13)


### Features

* **home:** add remove recent dirs and delete project actions ([111d1d7](https://github.com/golish-ai/golish/commit/111d1d79d5f25141984e4aea51fc88ff94d4b674))
* **home:** add remove recent dirs and delete project actions ([31689b8](https://github.com/golish-ai/golish/commit/31689b8c71f92c8d60734e38bf7c35f4b65af0f4))

## [0.2.42](https://github.com/golish-ai/golish/compare/v0.2.41...v0.2.42) (2026-03-06)


### Features

* **models:** add GPT-5.4 model with reasoning effort variants ([7570671](https://github.com/golish-ai/golish/commit/757067193bacb5f2dd59c7f67f85b55b09ea89c8))
* **models:** add GPT-5.4 model with reasoning effort variants ([3a038d4](https://github.com/golish-ai/golish/commit/3a038d48fdb7cd4a440005322d416c9ae3f0b90c))


### Bug Fixes

* **ai:** preserve partial output and add resilient stream-start retries ([ba7688d](https://github.com/golish-ai/golish/commit/ba7688dbbea332e1a464913ab71f28c5298e64c0))
* **ai:** preserve partial output and harden stream-start retries ([32603a8](https://github.com/golish-ai/golish/commit/32603a8538595169d8b06899e8b86ce028cba0dd))
* resolve lint, formatting, and flaky test issues ([48a632e](https://github.com/golish-ai/golish/commit/48a632ed5bbdc6e2222a805fc0f48c1533ee69e2))
* **tab-bar:** hide pending command indicator during fullterm sessions ([2390813](https://github.com/golish-ai/golish/commit/239081381f573f1a920d144808da272367cc10f0))
* **terminal:** suppress fullterm output from timeline ([96f24eb](https://github.com/golish-ai/golish/commit/96f24ebd794e40b5b7d7251ab7c32284a55f1eef))
* **terminal:** suppress fullterm output from timeline ([329ec4b](https://github.com/golish-ai/golish/commit/329ec4bfda92d22a9c78d5f2ba07fd08cdfe3fba))

## [0.2.41](https://github.com/golish-ai/golish/compare/v0.2.40...v0.2.41) (2026-03-02)


### Bug Fixes

* **frontend:** resolve build errors from missing deps and test syntax ([8da1cec](https://github.com/golish-ai/golish/commit/8da1cec0ec5abb426d91926edf08d23da17cab3d))
* **frontend:** resolve build errors from missing deps and test syntax ([37224f8](https://github.com/golish-ai/golish/commit/37224f88fff3f669d27dddf764698ea2c98ad93b))

## [0.2.40](https://github.com/golish-ai/golish/compare/v0.2.39...v0.2.40) (2026-03-02)


### Features

* **providers:** add extra_high reasoning effort level for OpenAI ([566fcd8](https://github.com/golish-ai/golish/commit/566fcd8adea2176b5231782af17d9f78cfe6d76b))
* **providers:** add extra_high reasoning effort level for OpenAI models ([4391882](https://github.com/golish-ai/golish/commit/43918829166443beff54c4ad87f1bbd520394ed5))
* **providers:** add extra_high reasoning effort level for OpenAI models ([4018ca4](https://github.com/golish-ai/golish/commit/4018ca48f822834523248ddd18c61ab75dd02cde))
* **providers:** add GPT 5.3 Codex model with all reasoning effort levels ([f651fea](https://github.com/golish-ai/golish/commit/f651fea53e0aeccfdd1c74279fee65104b942b01))


### Bug Fixes

* **agentic_loop:** extend Responses API reasoning-sequencing guard to openai_reasoning ([966a295](https://github.com/golish-ai/golish/commit/966a295a1d887beefae31128a44f62074b971491))
* **completion:** ensure Detailed reasoning summary and param overrides ([fc18d02](https://github.com/golish-ai/golish/commit/fc18d021747453bb876736d99f88801a4c0f3633))
* **openai-responses:** enable stateless multi-turn with reasoning models ([1a01b15](https://github.com/golish-ai/golish/commit/1a01b1540a264ccd1b359a83942bf8c63dd90dbe))
* **openai-responses:** enable stateless multi-turn with reasoning models ([b48a605](https://github.com/golish-ai/golish/commit/b48a60592bbeac439081a8070a5f92b44573e19c))
* **openai-responses:** remove status field from input items sent to Responses API ([8919228](https://github.com/golish-ai/golish/commit/8919228534d278a32f2774b9e79af2505a9e454b))
* remove unreachable catch-all in sub-agent stream match and sort imports ([2409650](https://github.com/golish-ai/golish/commit/2409650b37e14bcfec7f7d91fbbb86ddf2497485))
* **store:** default tab type and harden pane moves ([11247b8](https://github.com/golish-ai/golish/commit/11247b87826463cf9a0928084cb114de71ee6990))
* **tab-bar:** derive tab numbers excluding home tab ([c6a2166](https://github.com/golish-ai/golish/commit/c6a2166267af6f287a02651d0b65f2747bdca7ef))
* **terminal:** park live xterm in offscreen lot on React unmount to avoid renderer races ([1c1d15a](https://github.com/golish-ai/golish/commit/1c1d15a4b86df3b2a68e27ed92303ec9b385e5bb))
* **tracing:** restore Langfuse span data for openai_reasoning provider (GPT-5.2/Codex) ([f02eee2](https://github.com/golish-ai/golish/commit/f02eee27bf6fd501b442af6a8edd47f518c51e0c))
* **tracing:** restore missing Langfuse span data for openai_reasoning provider ([0e0b2c9](https://github.com/golish-ai/golish/commit/0e0b2c94c875e5c7182c7f7dc51b7bf871d0f4d9))

## [0.2.39](https://github.com/golish-ai/golish/compare/v0.2.38...v0.2.39) (2026-03-01)


### Features

* add glm-5 model to Z.AI provider ([eb1bef0](https://github.com/golish-ai/golish/commit/eb1bef05481cf7e7455a071611f05cde16825689))
* add glm-5 model to Z.AI provider ([2555e26](https://github.com/golish-ai/golish/commit/2555e266409c9bd095c7206a7174abaa5977f288))


### Bug Fixes

* **openai:** correct call_id propagation for streaming tool calls ([cdc8a0a](https://github.com/golish-ai/golish/commit/cdc8a0a088b614a45d83571619c0a3fd340c4755))
* **openai:** correct streaming tool call_id propagation and silence keepalive errors ([7ab6667](https://github.com/golish-ai/golish/commit/7ab6667b939c28955ad625e55a75835564505292))
* **openai:** silence keepalive deserialization errors in Responses API stream ([b252f94](https://github.com/golish-ai/golish/commit/b252f94ef4030b3708947d0e4b70a2c5656f2f85))


### Refactoring

* **sub-agents:** streamline explorer prompt ([9315c1b](https://github.com/golish-ai/golish/commit/9315c1b719168c244bc41fd4d0ecfc8e4890fa12))
* **sub-agents:** streamline explorer prompt and fix test ([a8b6dc8](https://github.com/golish-ai/golish/commit/a8b6dc84217eef199b2cd752e44a180261ba6da1))

## [0.2.38](https://github.com/golish-ai/golish/compare/v0.2.37...v0.2.38) (2026-02-24)


### Features

* **terminal:** add customizable block caret for input area ([44494a1](https://github.com/golish-ai/golish/commit/44494a107836a9e3db4222b815eb31f851f1e747))
* **terminal:** add customizable block caret for input area ([f2c0619](https://github.com/golish-ai/golish/commit/f2c061982f0383e215e1c26249d91981cab01e0c))

## [0.2.37](https://github.com/golish-ai/golish/compare/v0.2.36...v0.2.37) (2026-02-22)


### Features

* **vertex:** enable 1M context window beta for Sonnet 4.6 ([c53b71a](https://github.com/golish-ai/golish/commit/c53b71a54234099540154cd6ce3b5f52303964f3))
* **vertex:** enable 1M context window beta for Sonnet 4.6 ([72d617b](https://github.com/golish-ai/golish/commit/72d617ba274b28bd42378a8253127be25eda4f48))

## [0.2.36](https://github.com/golish-ai/golish/compare/v0.2.35...v0.2.36) (2026-02-19)


### Bug Fixes

* **pty:** fix rendering for TUI applications ([a3193cc](https://github.com/golish-ai/golish/commit/a3193ccd057c50a225c9ff2d38cdc09f1452b82b))
* **pty:** pass all CSI and ESC sequences through in Output region ([f3975a3](https://github.com/golish-ai/golish/commit/f3975a3a89df08a512a1469332860721773094d0))
* **terminal:** simplify fullterm transition and home cursor for non-alternate-screen apps ([15b3b2a](https://github.com/golish-ai/golish/commit/15b3b2a3b923e77ed38820896218487a39e6b426))


### Performance

* **pty:** coalesce PTY output in dedicated emitter thread ([31fe079](https://github.com/golish-ai/golish/commit/31fe079c7767d65179ba4dac43ba8ad77556ee3d))

## [0.2.35](https://github.com/golish-ai/golish/compare/v0.2.34...v0.2.35) (2026-02-18)


### Features

* add auto input mode with command classification ([62f6a91](https://github.com/golish-ai/golish/commit/62f6a91801d759121ccfd32c44a2d7051a32c144))
* **backend:** add command index for auto input mode classification ([488eb3e](https://github.com/golish-ai/golish/commit/488eb3ec03797924a561865765a2c831c6e0ff49))
* **frontend:** add auto input mode type and classify_input binding ([c4843b8](https://github.com/golish-ai/golish/commit/c4843b898fd0914fecbfc6c078ece2a492bb5560))
* **frontend:** implement auto input mode in UnifiedInput ([c112e32](https://github.com/golish-ai/golish/commit/c112e320ca33e6e171c67a73729b41247ceaa44d))
* **models:** add Claude Sonnet 4.6 support ([09cfb78](https://github.com/golish-ai/golish/commit/09cfb786dfb6116c4d65d095a3963b826c074c3b))
* **models:** add Claude Sonnet 4.6 support and bump to v0.2.29 ([7e0f3df](https://github.com/golish-ai/golish/commit/7e0f3dfa6e1e26ba769517728409e68ff7e2db06))


### Bug Fixes

* **backend:** follow symlinks when scanning PATH for executables ([32d9146](https://github.com/golish-ai/golish/commit/32d91468a0c29ee160edc1393fa5626cb6a8ecef))
* **backend:** resolve full shell PATH for command index on macOS ([0abbe58](https://github.com/golish-ai/golish/commit/0abbe58bf34fdf323c7904f5785335e045fc84f3))

## [0.2.34](https://github.com/golish-ai/golish/compare/v0.2.33...v0.2.34) (2026-02-13)


### Features

* **frontend:** display prompt generation status in sub-agent cards ([ea47c60](https://github.com/golish-ai/golish/commit/ea47c60b814734ee43dbcf98ccadea221bd5533f))
* **sub-agents:** add worker agent with LLM-powered prompt generation ([c828e50](https://github.com/golish-ai/golish/commit/c828e50c3beeeae7195fa198fe30a44aec38529c))


### Bug Fixes

* **tools:** downgrade tool registry lock from write to read for execution ([bc18456](https://github.com/golish-ai/golish/commit/bc184566ef9fdd16eabee60a8f57d09ed89b9b2b))


### Refactoring

* **ai:** extract execute_single_tool_call and make LoopCaptureContext Send+Sync ([199aaf9](https://github.com/golish-ai/golish/commit/199aaf92a8276314347c345195b387d795e578e4))

## [0.2.33](https://github.com/golish-ai/golish/compare/v0.2.32...v0.2.33) (2026-02-10)


### Bug Fixes

* remove unused sessionId parameter from mcp.listTools() call ([882a431](https://github.com/golish-ai/golish/commit/882a431fe2b90825d88030cced19cade7650b6a7))
* remove unused sessionId parameter from mcp.listTools() call ([a4f7e84](https://github.com/golish-ai/golish/commit/a4f7e84528a9389d9ab4efd1aea91e2e3e8ef09f))

## [0.2.32](https://github.com/golish-ai/golish/compare/v0.2.31...v0.2.32) (2026-02-10)


### Features

* **panes:** add right-click context menu with pane management actions ([54f40a9](https://github.com/golish-ai/golish/commit/54f40a99ebbc73cbd21149aa284d167ffdd2c938))
* **panes:** right-click context menu with pane management ([7c26931](https://github.com/golish-ai/golish/commit/7c26931b2c82023623a6a51de32522b249496c5a))

## [0.2.31](https://github.com/golish-ai/golish/compare/v0.2.30...v0.2.31) (2026-02-10)


### Features

* **sub-agents:** make final toolless LLM call on max iterations ([c25d2eb](https://github.com/golish-ai/golish/commit/c25d2eb029f14eb5de26be9675f4e6a536779793))
* **tabs:** add tab reordering, context menu, and convert-to-pane modal ([ca01cf1](https://github.com/golish-ai/golish/commit/ca01cf13ce82759ead27cf98dd157ec9bbe786c2))
* **tabs:** add tab reordering, context menu, and convert-to-pane modal ([76c66f3](https://github.com/golish-ai/golish/commit/76c66f3b0b56677a47db7326ab920ffb1f10f6c9))
* **ui:** add MCP servers badge to footer status row ([1a85627](https://github.com/golish-ai/golish/commit/1a85627d0922c8bf04005725aa8463c5c8ecccc7))
* **ui:** add MCP servers badge to footer status row ([8c6eddd](https://github.com/golish-ai/golish/commit/8c6edddeaa03624126f894ecce06c8c2640f8313))


### Bug Fixes

* **sub-agents:** return summary response when max tool calls reached ([457c4df](https://github.com/golish-ai/golish/commit/457c4df39d066d290d534e625bbe8aa4750a341d))

## [0.2.30](https://github.com/golish-ai/golish/compare/v0.2.29...v0.2.30) (2026-02-09)


### Features

* integrate tokenx-rs for proactive token counting before LLM calls ([b2e84b5](https://github.com/golish-ai/golish/commit/b2e84b5622bdc5c5c3af49346f644a53a3728882))
* proactive token counting with tokenx-rs for better compaction timing ([59a82e3](https://github.com/golish-ai/golish/commit/59a82e3069f8fb1f395be6b98c4fba086ea54cc1))


### Performance

* optimize app startup by deferring non-critical initialization ([be33e14](https://github.com/golish-ai/golish/commit/be33e144670aec8f9f41dec742f37c9e838637cf))


### Refactoring

* global MCP client shared across agent sessions with background init ([f9ac217](https://github.com/golish-ai/golish/commit/f9ac21764195426d35252249f04d4f1b9d24e3d0))

## [0.2.29](https://github.com/golish-ai/golish/compare/v0.2.28...v0.2.29) (2026-02-08)


### Features

* add reusable React profiling analysis scripts ([45d4b9f](https://github.com/golish-ai/golish/commit/45d4b9f78d8340f4a3136ae4b41e02d188352b57))


### Bug Fixes

* **test:** add missing setQuickOpenDialogOpen mock ([68da5ec](https://github.com/golish-ai/golish/commit/68da5ec37bb24aec6d1e831238ad9793f40c6a0a))
* **test:** add missing setQuickOpenDialogOpen mock to keyboard handler test ([30569d2](https://github.com/golish-ai/golish/commit/30569d23407b69852fe368cdd84d268a7eee1d84))


### Performance

* eliminate cascading re-renders and add profiling tooling ([1be76d4](https://github.com/golish-ai/golish/commit/1be76d49fb710100d9f63b1302ef022c642123c9))
* eliminate cascading re-renders with architectural optimizations ([ab4f157](https://github.com/golish-ai/golish/commit/ab4f157fb5acbeb941d14983ca50009ab2ccae62))
* eliminate cascading re-renders with architectural optimizations ([1826283](https://github.com/golish-ai/golish/commit/18262834716667d0883d6cf10f5272e50449b8a9))

## [0.2.28](https://github.com/golish-ai/golish/compare/v0.2.27...v0.2.28) (2026-02-06)


### Features

* **editor:** detect external file changes and auto-reload open tabs ([5393136](https://github.com/golish-ai/golish/commit/5393136d8aafedd5b3843704ef081719a7fcce36))
* **editor:** detect external file changes and auto-reload open tabs ([94ffd6a](https://github.com/golish-ai/golish/commit/94ffd6a2febe63c946a885c0c52c1281377ff5bb))
* **file-browser:** add hidden files toggle, editable path bar, and quick open ([e9b33cc](https://github.com/golish-ai/golish/commit/e9b33cced82957148ef0a476cb5c83ac88e55297))
* **file-browser:** add hidden files toggle, editable path bar, and quick open ([dac7fca](https://github.com/golish-ai/golish/commit/dac7fca8f5c7ed3f7440330262f21c7b6c6434c1))
* **sub-agents:** add overall and idle timeout to prevent stuck sub-agents ([e5329c3](https://github.com/golish-ai/golish/commit/e5329c3ad87e23f8e829fa28e3c6a174855c1162))
* **sub-agents:** add overall and idle timeout to prevent stuck sub-agents ([5054eca](https://github.com/golish-ai/golish/commit/5054ecaf3aea9b382ac87b986485f44f0150c262))


### Bug Fixes

* **scroll:** auto-scroll reliably when streaming tool calls update ([c429fbe](https://github.com/golish-ai/golish/commit/c429fbe6cda8e926fcefe8d5a65462aad4a89f37))
* **scroll:** auto-scroll reliably when streaming tool calls update ([620a5c5](https://github.com/golish-ai/golish/commit/620a5c52fecb3e238c590b50f90a63af95c4efb2))
* snapshot workingDirectory on agent messages for stable file links ([74db6e6](https://github.com/golish-ai/golish/commit/74db6e6c14df9793830e55798279310b9ac86b44))
* snapshot workingDirectory on agent messages for stable file links ([192aea5](https://github.com/golish-ai/golish/commit/192aea58f50b852c3394c42e77bc1c454e345339))
* **thinking:** render markdown headers on separate lines ([c944a5f](https://github.com/golish-ai/golish/commit/c944a5f423f86e169c08e229a93d1f3a2ef152ec))
* **thinking:** render markdown headers on separate lines ([cdbb4a8](https://github.com/golish-ai/golish/commit/cdbb4a857626c14a28b29a26d721551d02f00d52))

## [0.2.27](https://github.com/golish-ai/golish/compare/v0.2.26...v0.2.27) (2026-02-06)


### Features

* **compaction:** improve truncation, add retry, and expose summarizer I/O ([#269](https://github.com/golish-ai/golish/issues/269)) ([034648b](https://github.com/golish-ai/golish/commit/034648b7aa0b9d70a7b4b07c7b0688560a7aef56))
* **tabs:** add right-click context menu with duplicate tab ([#268](https://github.com/golish-ai/golish/issues/268)) ([b4ae7b5](https://github.com/golish-ai/golish/commit/b4ae7b5b26cbe50607023abe0beaeab0a93afb10))


### Bug Fixes

* **markdown:** use full ReactMarkdown renderer during streaming ([#271](https://github.com/golish-ai/golish/issues/271)) ([aeb4f30](https://github.com/golish-ai/golish/commit/aeb4f3076759f8684453cc3322a9437ce71f5c7f))
* **pty:** prevent zsh recursion when nested Golish inherits ZDOTDIR ([#267](https://github.com/golish-ai/golish/issues/267)) ([1294e75](https://github.com/golish-ai/golish/commit/1294e756dd1565067a7d650bf4d0c03917b90a6f))
* **rig-anthropic-vertex:** handle citations_delta stream events from Claude web search ([#266](https://github.com/golish-ai/golish/issues/266)) ([07358be](https://github.com/golish-ai/golish/commit/07358be7c954fd891b91634283c83111de2e7998))

## [0.2.26](https://github.com/golish-ai/golish/compare/v0.2.25...v0.2.26) (2026-02-06)


### Features

* **mcp:** add Model Context Protocol support for external tools ([fead2de](https://github.com/golish-ai/golish/commit/fead2dea4efcf4dadd637a20d4121a54dc6e16fc))
* **mcp:** add Model Context Protocol support with OAuth 2.1 authentication ([#260](https://github.com/golish-ai/golish/issues/260)) ([5fb59ca](https://github.com/golish-ai/golish/commit/5fb59ca677417737a6ede08ebca84a5d8197c6a0))


### Refactoring

* **backend:** consolidate 11 single-purpose crates into parent crates ([#265](https://github.com/golish-ai/golish/issues/265)) ([bed3a92](https://github.com/golish-ai/golish/commit/bed3a928fd9cf22ca7a5bc45e678542f3db83f02))
* **backend:** remove vtcode-core dependency and related features ([#264](https://github.com/golish-ai/golish/issues/264)) ([97dea1b](https://github.com/golish-ai/golish/commit/97dea1b5ea9f9a7f0e87cd80c83c333801780182))
* **transcript:** switch to JSONL format and fix coordinator writer propagation ([#263](https://github.com/golish-ai/golish/issues/263)) ([f548bf6](https://github.com/golish-ai/golish/commit/f548bf60d9f20fb055064948d071bbbc065f2fc1))

## [0.2.25](https://github.com/golish-ai/golish/compare/v0.2.24...v0.2.25) (2026-02-05)


### Features

* **models:** add Claude Opus 4.6 support on Vertex AI ([#259](https://github.com/golish-ai/golish/issues/259)) ([19574d3](https://github.com/golish-ai/golish/commit/19574d35040df6b98b5f6668425ab0d5f5bd9aee))

## [0.2.24](https://github.com/golish-ai/golish/compare/v0.2.23...v0.2.24) (2026-02-04)


### Bug Fixes

* add missing [@codemirror](https://github.com/codemirror) peer dependencies ([#257](https://github.com/golish-ai/golish/issues/257)) ([5dc4420](https://github.com/golish-ai/golish/commit/5dc44200a6b043c68b3d1c9c792fe6d47e0a1e7b))

## [0.2.23](https://github.com/golish-ai/golish/compare/v0.2.22...v0.2.23) (2026-02-04)


### Features

* empty commit to trigger release please ([#255](https://github.com/golish-ai/golish/issues/255)) ([73c7438](https://github.com/golish-ai/golish/commit/73c7438f92c6d84edac0a292a5c9420f68aa9bb0))

## [0.2.22](https://github.com/golish-ai/golish/compare/v0.2.21...v0.2.22) (2026-02-04)


### Performance

* **frontend:** comprehensive performance optimizations ([#252](https://github.com/golish-ai/golish/issues/252)) ([c1448a1](https://github.com/golish-ai/golish/commit/c1448a1bb4ebf18aca90e90e6ca8db78bca36771))

## [0.2.21](https://github.com/golish-ai/golish/compare/v0.2.20...v0.2.21) (2026-02-03)


### Features

* cleanup ([#249](https://github.com/golish-ai/golish/issues/249)) ([afc75bc](https://github.com/golish-ai/golish/commit/afc75bc4ba39a3048e37a48e9e6a1988acac5563))


### Bug Fixes

* **models:** update context limits and add Codex support ([#251](https://github.com/golish-ai/golish/issues/251)) ([0f7b716](https://github.com/golish-ai/golish/commit/0f7b716ee89bb8e2d134645236d4fb6126662d1c))

## [0.2.20](https://github.com/golish-ai/golish/compare/v0.2.19...v0.2.20) (2026-02-02)


### Bug Fixes

* **rig-openai-responses:** use structured types for tool calls and results ([#244](https://github.com/golish-ai/golish/issues/244)) ([31acdcd](https://github.com/golish-ai/golish/commit/31acdcd14b079343398058900c2721cb5cc36f52))
* **ui:** limit thinking blocks to one at a time in timeline ([#240](https://github.com/golish-ai/golish/issues/240)) ([b8b493f](https://github.com/golish-ai/golish/commit/b8b493fbe8c2ba4cab800f65d6f405b8b63c7f04))


### Refactoring

* **sub-agents:** optimize explorer prompt for speed over thoroughness ([#243](https://github.com/golish-ai/golish/issues/243)) ([241c76d](https://github.com/golish-ai/golish/commit/241c76d3174d164cf82e75a1fc192c8e84e4aadc))
* **ui:** remove left accent color from ToolGroup component ([#241](https://github.com/golish-ai/golish/issues/241)) ([fb0d2f0](https://github.com/golish-ai/golish/commit/fb0d2f0dd76e9a6ccbbedde80a93e7f072f8a604))

## [0.2.19](https://github.com/golish-ai/golish/compare/v0.2.18...v0.2.19) (2026-02-02)


### Features

* **ui:** add langfuse tracing badge to footer bar ([#238](https://github.com/golish-ai/golish/issues/238)) ([2c76c8d](https://github.com/golish-ai/golish/commit/2c76c8df740c294c5f0b275b9c523085c276eed1))


### Bug Fixes

* **ui:** improve tab activity indicator rendering and add pulse animation ([#237](https://github.com/golish-ai/golish/issues/237)) ([b348215](https://github.com/golish-ai/golish/commit/b348215ac01a9f48c6cdb2fc6b7339186e5be055))

## [0.2.18](https://github.com/golish-ai/golish/compare/v0.2.17...v0.2.18) (2026-02-01)


### Features

* **ai:** add openai_reasoning provider to Codex-style prompt routing ([#234](https://github.com/golish-ai/golish/issues/234)) ([29976f1](https://github.com/golish-ai/golish/commit/29976f1da0df4dfd441a9ccf08c12a37b98b033f))
* **ai:** add provider-specific system prompts for OpenAI models ([#232](https://github.com/golish-ai/golish/issues/232)) ([446470b](https://github.com/golish-ai/golish/commit/446470b1124f00fe34407439f3d6eef2b3b7f022))
* **notifications:** add native OS notifications via tauri-plugin-notification ([#235](https://github.com/golish-ai/golish/issues/235)) ([32431c6](https://github.com/golish-ai/golish/commit/32431c6b2fdc0ab0ff5a6585e985d17f2988b29e))
* **settings:** add version display to Advanced settings ([#230](https://github.com/golish-ai/golish/issues/230)) ([ad32087](https://github.com/golish-ai/golish/commit/ad32087fda8f9e34b67d8894f13826027d464575))
* **ui:** show busy and new-activity indicators on tabs ([#236](https://github.com/golish-ai/golish/issues/236)) ([19de44b](https://github.com/golish-ai/golish/commit/19de44b76dbd2bacda36a2623039f847e430bf56))

## [0.2.17](https://github.com/golish-ai/golish/compare/v0.2.16...v0.2.17) (2026-01-30)


### Features

* add editor settings and codemirror extensions ([#223](https://github.com/golish-ai/golish/issues/223)) ([7329aec](https://github.com/golish-ai/golish/commit/7329aec12fca98a8ca044b3fc4889a5fd049cd0f))
* **ai:** add JSON repair for malformed LLM tool call arguments ([#227](https://github.com/golish-ai/golish/issues/227)) ([be6f226](https://github.com/golish-ai/golish/commit/be6f22615ff9b924e467936eac8787c173e16d62))


### Bug Fixes

* **notification-widget:** render panel via portal ([#228](https://github.com/golish-ai/golish/issues/228)) ([20c139b](https://github.com/golish-ai/golish/commit/20c139bbc56235a89fda994e2b42fd2182a2e275))
* prevent duplicate tool blocks causing output to update wrong component ([#224](https://github.com/golish-ai/golish/issues/224)) ([5b18eab](https://github.com/golish-ai/golish/commit/5b18eab5747e77918a8630a658668b9a9fa321e1))

## [0.2.16](https://github.com/golish-ai/golish/compare/v0.2.15...v0.2.16) (2026-01-29)


### Features

* enhance thinking block rendering and tool grouping ([#220](https://github.com/golish-ai/golish/issues/220)) ([693d3ea](https://github.com/golish-ai/golish/commit/693d3ea859a5752f2d0cfa8b88e903dfc66e32ea))
* **home:** add worktree deletion from context menu ([#222](https://github.com/golish-ai/golish/issues/222)) ([1bb91fe](https://github.com/golish-ai/golish/commit/1bb91fe659c8f076d6ef894deb94de62ce2b31c1))

## [0.2.15](https://github.com/golish-ai/golish/compare/v0.2.14...v0.2.15) (2026-01-29)


### Features

* add image attachment functionality to AgentMessage and UnifiedInput ([#214](https://github.com/golish-ai/golish/issues/214)) ([a2b15ae](https://github.com/golish-ai/golish/commit/a2b15ae8dadd35e10887b56fbfb981a2db5a1a39))
* **home:** add Home View with projects and worktree management ([#215](https://github.com/golish-ai/golish/issues/215)) ([13b6b0f](https://github.com/golish-ai/golish/commit/13b6b0fa621b16a73d55182b205e714ecdef9810))


### Bug Fixes

* **e2e:** update tests for Home tab and add missing mock handlers ([#219](https://github.com/golish-ai/golish/issues/219)) ([e7f69a0](https://github.com/golish-ai/golish/commit/e7f69a0036f3f979fdabc9494b52b854bcee2091))
* **ui:** notification z-index and hide run_command from timeline ([#218](https://github.com/golish-ai/golish/issues/218)) ([6f023ba](https://github.com/golish-ai/golish/commit/6f023ba70e9de59e79240fea1f53b47cd72f7beb))
* **vision:** add vertex_gemini to vision-capable providers ([#217](https://github.com/golish-ai/golish/issues/217)) ([fe0cefc](https://github.com/golish-ai/golish/commit/fe0cefc9b25af76f92ae4004daa406a301e01309))

## [0.2.14](https://github.com/golish-ai/golish/compare/v0.2.13...v0.2.14) (2026-01-28)


### Features

* **history:** add persistent command and prompt history system ([#210](https://github.com/golish-ai/golish/issues/210)) ([860a6a6](https://github.com/golish-ai/golish/commit/860a6a620ba529f1d717e40383efbaa50753d91b))
* **llm-providers:** add Gemini on Vertex AI provider ([#206](https://github.com/golish-ai/golish/issues/206)) ([f0c4c8d](https://github.com/golish-ai/golish/commit/f0c4c8d20d0b024032211f6d081f651fbf197b18))
* **rig-openai-responses:** add image support for user messages ([#207](https://github.com/golish-ai/golish/issues/207)) ([bff0596](https://github.com/golish-ai/golish/commit/bff0596dff04cf62084fa03647215ab80133c5c4))
* **shell:** add streaming output for run_command tool ([#211](https://github.com/golish-ai/golish/issues/211)) ([36d9003](https://github.com/golish-ai/golish/commit/36d90033e498924bc6467e202c698b5c5d08271f))


### Bug Fixes

* **biome:** resolve worktree config conflicts ([#213](https://github.com/golish-ai/golish/issues/213)) ([b83a2b2](https://github.com/golish-ai/golish/commit/b83a2b2a57e642c669b3ef91c1a03c6e774c3eb3))


### Refactoring

* **file-editor:** use single shared instance across all tabs ([#208](https://github.com/golish-ai/golish/issues/208)) ([5c7e935](https://github.com/golish-ai/golish/commit/5c7e935ee3a8bb144276dfb92f362855f4ac3169))
* **workflows:** simplify environment variable usage in evals and update-homebrew workflows ([ce58bb6](https://github.com/golish-ai/golish/commit/ce58bb6f67c8200d4315513ac53f841ae13a36cc))

## [0.2.13](https://github.com/golish-ai/golish/compare/v0.2.12...v0.2.13) (2026-01-26)


### Refactoring

* **input:** remove placeholder text and use data attributes for E2E tests ([#202](https://github.com/golish-ai/golish/issues/202)) ([92a0206](https://github.com/golish-ai/golish/commit/92a02062900f1ee10109608381787760efb4a7b6))

## [0.2.12](https://github.com/golish-ai/golish/compare/v0.2.11...v0.2.12) (2026-01-26)


### Refactoring

* provider-model config consolidation (Phases 1-4) ([#199](https://github.com/golish-ai/golish/issues/199)) ([5b2e570](https://github.com/golish-ai/golish/commit/5b2e5706bd55500b6ab85808e52487d05c392fc3))

## [0.2.11](https://github.com/golish-ai/golish/compare/v0.2.10...v0.2.11) (2026-01-26)


### Features

* **path-completion:** enhance tab completion with fuzzy matching and file type icons ([#198](https://github.com/golish-ai/golish/issues/198)) ([4e8d069](https://github.com/golish-ai/golish/commit/4e8d0693b242650d7dcda3dd364433b7563219e0))
* **timeline:** implement Phase 2 performance improvements and reliability fixes ([#194](https://github.com/golish-ai/golish/issues/194)) ([1e4452c](https://github.com/golish-ai/golish/commit/1e4452c917c11ca68dcc2cb68af25e43aac6b69b))


### Refactoring

* **store:** implement single source of truth for timeline data ([#197](https://github.com/golish-ai/golish/issues/197)) ([dddc37c](https://github.com/golish-ai/golish/commit/dddc37ca848d7786aff36f56e562a6c2a3560ba6))

## [0.2.10](https://github.com/golish-ai/golish/compare/v0.2.9...v0.2.10) (2026-01-25)


### Features

* **ai:** implement EventCoordinator for deadlock-free event management ([#185](https://github.com/golish-ai/golish/issues/185)) ([cbe18c7](https://github.com/golish-ai/golish/commit/cbe18c7a3a5cddeab408ac2a4d9a73ba77f66eb6))
* **ai:** replace Z.AI providers with unified rig-zai-sdk ([#191](https://github.com/golish-ai/golish/issues/191)) ([7e54a77](https://github.com/golish-ai/golish/commit/7e54a77caaa99e5053fb8dc5e64dd895b4784d9f))
* **evals:** SWE-bench Lite integration for agent benchmarking ([#181](https://github.com/golish-ai/golish/issues/181)) ([0b544ba](https://github.com/golish-ai/golish/commit/0b544baedc6c3774d356b1d12f1f859d76a73bf4))
* **openai:** add reasoning effort support and xhigh level for GPT models ([#177](https://github.com/golish-ai/golish/issues/177)) ([a495bd3](https://github.com/golish-ai/golish/commit/a495bd31ee241813c977ddac0a483f0a6d42aecb))
* **swebench:** integrate official SWE-bench harness for test evaluation ([22d180c](https://github.com/golish-ai/golish/commit/22d180c2ff20cb07dea044de1fd0127374190e6f))


### Bug Fixes

* **ai:** improve event reliability and prevent directory change deadlock ([#184](https://github.com/golish-ai/golish/issues/184)) ([0cc9bec](https://github.com/golish-ai/golish/commit/0cc9becdfe9782a44002606e3c96ea40b5e96d81))
* **ai:** OpenAI temperature regression, UTF-8 panic, and rig-core upgrade ([#187](https://github.com/golish-ai/golish/issues/187)) ([928de26](https://github.com/golish-ai/golish/commit/928de26e92954b00ccfcf26bcea4cf97ac21ec32))
* **ai:** resolve deadlock in release builds when switching models ([c270f5c](https://github.com/golish-ai/golish/commit/c270f5c57dee9cb580f6447ca5c332312e18530d))
* **ai:** resolve deadlock in release builds when switching models ([#179](https://github.com/golish-ai/golish/issues/179)) ([fba5466](https://github.com/golish-ai/golish/commit/fba5466e18fce03b9cfed7067619d2e8c5905135))
* **openai:** fix reasoning display and history for Responses API ([#180](https://github.com/golish-ai/golish/issues/180)) ([66165a0](https://github.com/golish-ai/golish/commit/66165a0abc67621671b21e7ff866a8d403e9abd1))
* remount race conditions ([#190](https://github.com/golish-ai/golish/issues/190)) ([a957551](https://github.com/golish-ai/golish/commit/a95755185fffb2bb484191fadc6cbd2f25b2584d))


### Refactoring

* **settings:** remove obsolete Z.AI and Z.AI (Anthropic) providers in favor of ZaiSdk ([#192](https://github.com/golish-ai/golish/issues/192)) ([8f2c059](https://github.com/golish-ai/golish/commit/8f2c05950a15a3fa8cc263fca7653951a9385611))

## [0.2.9](https://github.com/golish-ai/golish/compare/v0.2.8...v0.2.9) (2026-01-18)


### Features

* **events:** add reasoning field to completion events and reduce log noise ([#168](https://github.com/golish-ai/golish/issues/168)) ([90e71d1](https://github.com/golish-ai/golish/commit/90e71d1fec3cb684d5138f786fd64cbba4d3703a))
* **skills:** add Agent Skills support with agentskills.io spec ([#174](https://github.com/golish-ai/golish/issues/174)) ([4edc0f8](https://github.com/golish-ai/golish/commit/4edc0f832a686fa9792bafde455051a2fc3c18b1))
* **system-hooks:** add logging, OTel events, and plan completion reminder ([#172](https://github.com/golish-ai/golish/issues/172)) ([bfdbb6e](https://github.com/golish-ai/golish/commit/bfdbb6ebc2360d2860363ea6aa8cc30afdc7050e))
* **vertex:** enable prompt caching for Anthropic Vertex AI provider ([#171](https://github.com/golish-ai/golish/issues/171)) ([6619ad9](https://github.com/golish-ai/golish/commit/6619ad9fe1c7b3bc2f5321616c150575dbf6ee26))


### Bug Fixes

* **agentic-loop:** ensure assistant messages are added to history before loop exit ([#170](https://github.com/golish-ai/golish/issues/170)) ([1217423](https://github.com/golish-ai/golish/commit/12174237e1de9c4830797f9122e1466bab339982))
* e2e tests ([8abd766](https://github.com/golish-ai/golish/commit/8abd76606041c2f4cec36856f81e1f8392d2ce7d))
* settings functionality ([b536455](https://github.com/golish-ai/golish/commit/b5364555c6f659bd4174c4eda62d33fef6eb8e40))
* settings theme saving ([4dde840](https://github.com/golish-ai/golish/commit/4dde8402a8b4768f6f87e772e70f36ed938a2378))
* **settings:** settings bugs ([9d32607](https://github.com/golish-ai/golish/commit/9d326070ab84fdac562acc92cbd61ad01067c193))

## [0.2.8](https://github.com/golish-ai/golish/compare/v0.2.7...v0.2.8) (2026-01-14)


### Features

* add LLM API request/response logging ([#148](https://github.com/golish-ai/golish/issues/148)) ([2a2174a](https://github.com/golish-ai/golish/commit/2a2174a2a022573d0f12cbc82787413d47d19b38))
* Add TavilyToolsContributor for system prompt integration ([#142](https://github.com/golish-ai/golish/issues/142)) ([18cd66b](https://github.com/golish-ai/golish/commit/18cd66bc4c041980cf592c5a0cb0f7bbc6ea0287))
* add transcript recording and context compaction trigger ([#158](https://github.com/golish-ai/golish/issues/158)) ([817d867](https://github.com/golish-ai/golish/commit/817d867fa13f33f5c8b4c32f5041be35991821d9))
* **ai:** add Z.AI Anthropic-compatible provider ([#149](https://github.com/golish-ai/golish/issues/149)) ([a70fbdb](https://github.com/golish-ai/golish/commit/a70fbdbf1cb52831ed40cd3a9f02f5782fe68732))
* **context-compaction:** add frontend UI for compaction events ([#165](https://github.com/golish-ai/golish/issues/165)) ([d5f0ad5](https://github.com/golish-ai/golish/commit/d5f0ad505d09413d586bf1e9bd5636fce0c01089))
* **context-compaction:** implement hard reset mechanism (step 5) ([#163](https://github.com/golish-ai/golish/issues/163)) ([0b739e9](https://github.com/golish-ai/golish/commit/0b739e9c2b880c561ba7902c2502432880f87810))
* **context-compaction:** implement summarizer agent and compaction trigger ([#159](https://github.com/golish-ai/golish/issues/159)) ([51ebaa6](https://github.com/golish-ai/golish/commit/51ebaa6888a8916a5e6f31a88c858fad62249636))
* **context-compaction:** implement summarizer input builder ([#161](https://github.com/golish-ai/golish/issues/161)) ([ec7b5b9](https://github.com/golish-ai/golish/commit/ec7b5b93ea8c5ce0382d793ce8ef94880a3de810))
* **context:** add compaction trigger and multi-model token limits ([#160](https://github.com/golish-ai/golish/issues/160)) ([040fd41](https://github.com/golish-ai/golish/commit/040fd413d275888fc8a42efd717669955fc444b7))
* **git:** add periodic status polling for status bar badge ([bd5dd23](https://github.com/golish-ai/golish/commit/bd5dd235d1639db00fa886f2be3f9efef75e8f59))
* **pty:** initial bash shell integration ([#155](https://github.com/golish-ai/golish/issues/155)) ([74ce062](https://github.com/golish-ai/golish/commit/74ce0621bbdbd2b88e3d592672330841c9ca4ef4))
* telemetry filtering, API logging, indexer deduplication, and UserMessage fix ([#157](https://github.com/golish-ai/golish/issues/157)) ([bc39762](https://github.com/golish-ai/golish/commit/bc39762595755ea2c0b3817b53c8f9b3883e7736))
* **vertex-ai:** support application default credentials ([#145](https://github.com/golish-ai/golish/issues/145)) ([976b7cf](https://github.com/golish-ai/golish/commit/976b7cf749de71b3d1103a26b65743608be45fa2))


### Bug Fixes

* add UTF-8 safe string truncation to prevent panics ([#162](https://github.com/golish-ai/golish/issues/162)) ([5546552](https://github.com/golish-ai/golish/commit/5546552a8436bd1095e0d731d419a36b16b2c406))
* **ai:** emit error notifications and fix context pruning event ([#141](https://github.com/golish-ai/golish/issues/141)) ([bbc5ab7](https://github.com/golish-ai/golish/commit/bbc5ab7f71af545b6851b698381f36119023e7d2))
* **context-compaction:** improve trigger timing and timeline display ([#166](https://github.com/golish-ai/golish/issues/166)) ([0221220](https://github.com/golish-ai/golish/commit/022122096b7493b3fd1af44a21dd7d0bb9b96b91))
* **executor:** ensure sub-agent spans are parented correctly ([#154](https://github.com/golish-ai/golish/issues/154)) ([1bb39ad](https://github.com/golish-ai/golish/commit/1bb39ad261eeec21f0bedbaf11930b20923d5378))
* **frontend:** add global error handling and fix runtime errors ([#156](https://github.com/golish-ai/golish/issues/156)) ([891d95a](https://github.com/golish-ai/golish/commit/891d95a0baa023896fda8281bfabc1701b5cef39))
* **git:** show diff for untracked files in GitPanel ([#150](https://github.com/golish-ai/golish/issues/150)) ([5e14742](https://github.com/golish-ai/golish/commit/5e1474273c5d6154e0612436503a8703b6e8ef1c))
* **pty:** revert parser changes causing terminal visibility issues ([#167](https://github.com/golish-ai/golish/issues/167)) ([839d0f8](https://github.com/golish-ai/golish/commit/839d0f8a89f2395fa711fe1a41169b002f013877))
* **sub-agents:** include thinking blocks in conversation history ([#151](https://github.com/golish-ai/golish/issues/151)) ([5f9ed65](https://github.com/golish-ai/golish/commit/5f9ed654d636c37b4298e7def3e2095ac73c4ef3))
* **telemetry:** properly instrument main agentic loop spans ([#139](https://github.com/golish-ai/golish/issues/139)) ([1be19b3](https://github.com/golish-ai/golish/commit/1be19b3638543cc01b4f21e5ce2aa5daeb0c39e4))
* **ui:** improve AgentMessage layout and copy button positioning ([73dd3f1](https://github.com/golish-ai/golish/commit/73dd3f1caba76f2565c1406ac50658317e26ca40))
* update e2e test regex and add auto-approve safeguards ([#152](https://github.com/golish-ai/golish/issues/152)) ([ce4452a](https://github.com/golish-ai/golish/commit/ce4452aa080bef9c458effaa8f9164d42cec0a8d))


### Refactoring

* **ai:** simplify system prompt structure ([#147](https://github.com/golish-ai/golish/issues/147)) ([65cb9ec](https://github.com/golish-ai/golish/commit/65cb9ecaabd6fc5c945f899ea2068e4097575d89))
* **context:** remove legacy pruning system ([#164](https://github.com/golish-ai/golish/issues/164)) ([d35bd82](https://github.com/golish-ai/golish/commit/d35bd821014804a1378bded643b2fd44313737f5))

## [0.2.7](https://github.com/golish-ai/golish/compare/v0.2.6...v0.2.7) (2026-01-10)


### Features

* **ci:** add Linux x86_64 build to release workflow ([135b93a](https://github.com/golish-ai/golish/commit/135b93a427e7b5cde4d65e1c53dc9fe23d350cd1))
* **ci:** add Linux x86_64 build to release workflow ([1a1b715](https://github.com/golish-ai/golish/commit/1a1b71587039a0578a9cddd5439a377b58de19b6))
* **telemetry:** improve Langfuse tracing for sub-agents and LLM spans ([#136](https://github.com/golish-ai/golish/issues/136)) ([b1fc749](https://github.com/golish-ai/golish/commit/b1fc749c6d1b59b3bba143925c47d905c760d1fe))


### Bug Fixes

* **telemetry:** improve log readability and span nesting ([#138](https://github.com/golish-ai/golish/issues/138)) ([149ec5c](https://github.com/golish-ai/golish/commit/149ec5cfef7761987af3a9fdb66512ee9a5f466f))

## [0.2.6](https://github.com/golish-ai/golish/compare/v0.2.5...v0.2.6) (2026-01-10)


### Features

* **editor:** add vim commands and improve file path detection ([#129](https://github.com/golish-ai/golish/issues/129)) ([826721a](https://github.com/golish-ai/golish/commit/826721a0e7d42790789247a275c43fc42b47ee93))
* **settings:** render settings as tab instead of modal dialog ([#110](https://github.com/golish-ai/golish/issues/110)) ([73aa78c](https://github.com/golish-ai/golish/commit/73aa78cc8c876f36cad520a0183e68b2068d63da))
* **sub-agents:** add parent_request_id to correlate sub-agent events ([#125](https://github.com/golish-ai/golish/issues/125)) ([1ee748c](https://github.com/golish-ai/golish/commit/1ee748cce83001b98eaa36159182c9fff69ab8a9))
* **tools:** add tool group details modal and mixed tool grouping ([#133](https://github.com/golish-ai/golish/issues/133)) ([f489979](https://github.com/golish-ai/golish/commit/f489979dc8f2cf6b9210539d7f61875cf0a1025f))
* **ui:** add clickable file path links in markdown and terminal ([#128](https://github.com/golish-ai/golish/issues/128)) ([a869f44](https://github.com/golish-ai/golish/commit/a869f44334f10d7562bf24c745c91f4997c4c02a))


### Bug Fixes

* **agent:** auto-approve mode now bypasses tool policy checks ([#127](https://github.com/golish-ai/golish/issues/127)) ([4f2777b](https://github.com/golish-ai/golish/commit/4f2777b04a8e3581b6555ce121cd192638aa3018))
* **agent:** resolve tab close and multi-agent initialization issues ([#126](https://github.com/golish-ai/golish/issues/126)) ([736518a](https://github.com/golish-ai/golish/commit/736518a596ba1624c91e8bf26cb46d998ce2b228))
* **git:** Auto-refresh branch/status after checkout commands ([#124](https://github.com/golish-ai/golish/issues/124)) ([65542a4](https://github.com/golish-ai/golish/commit/65542a4e4da8eb9cb5c0c90aa7befc730e62db52))
* **session:** fix restore initialization order and add agent_mode persistence ([#130](https://github.com/golish-ai/golish/issues/130)) ([0bcb2f7](https://github.com/golish-ai/golish/commit/0bcb2f77b4204e6f9f3498612dec5b065e188340))
* **session:** use current default provider when restoring sessions ([#131](https://github.com/golish-ai/golish/issues/131)) ([d4d030f](https://github.com/golish-ai/golish/commit/d4d030f84df101df76e354ee722d1b34ed73ca15))
* **terminal:** align path completion with standard shell behavior ([#132](https://github.com/golish-ai/golish/issues/132)) ([8153206](https://github.com/golish-ai/golish/commit/8153206548478826f55d531f63585e873d4cfe81))

## [0.2.5](https://github.com/golish-ai/golish/compare/v0.2.4...v0.2.5) (2026-01-08)


### Features

* **ai:** add per-sub-agent model overrides ([#112](https://github.com/golish-ai/golish/issues/112)) ([7dd3911](https://github.com/golish-ai/golish/commit/7dd3911a80f74722272e395747f92d1305eb4c2e))
* **input:** add argument support for slash commands ([#121](https://github.com/golish-ai/golish/issues/121)) ([8677cd1](https://github.com/golish-ai/golish/commit/8677cd1db21e54e8c678b42e504e2808020f9808))
* **input:** add multi-modal image input via drag-drop and paste ([#104](https://github.com/golish-ai/golish/issues/104)) ([5bff13f](https://github.com/golish-ai/golish/commit/5bff13f333e0ba1ffb3a184e4705703ca912fc29))
* **logging:** add persistent file logging and reduce verbosity ([#106](https://github.com/golish-ai/golish/issues/106)) ([7b727b4](https://github.com/golish-ai/golish/commit/7b727b44325bda76ba779a9cde4f69f502106172))
* **settings:** add per-project AI settings persistence ([#115](https://github.com/golish-ai/golish/issues/115)) ([fe4a32a](https://github.com/golish-ai/golish/commit/fe4a32ac6f4d75ee86c3af28e354710ae6f0e931))
* **terminal:** replace ANSI text output with embedded xterm.js terminals ([#111](https://github.com/golish-ai/golish/issues/111)) ([3f1911d](https://github.com/golish-ai/golish/commit/3f1911d1fa8a98bd2f29b3253ec90532bb76430e))
* **ui:** add copy buttons to user messages and command blocks ([1067f2b](https://github.com/golish-ai/golish/commit/1067f2b160284323879944d574070577324cae14))
* **ui:** add copy buttons to user messages and command blocks ([75159ec](https://github.com/golish-ai/golish/commit/75159eca5d13d2c27c93fba6001434026841fd63))
* **ui:** add details modal for sub-agent cards in timeline ([#116](https://github.com/golish-ai/golish/issues/116)) ([70983b5](https://github.com/golish-ai/golish/commit/70983b5aef26b5448f0f68d4c259589cdcbfc702))


### Bug Fixes

* close tab button not working with active agent/running command ([#118](https://github.com/golish-ai/golish/issues/118)) ([378b7c6](https://github.com/golish-ai/golish/commit/378b7c61ce8410a3c997676cde71073b1baf0d47))
* **e2e:** use globally exposed mocks for timeline scroll tests ([#107](https://github.com/golish-ai/golish/issues/107)) ([9086695](https://github.com/golish-ai/golish/commit/9086695dc20fc2a651fdd446cce8a084fac3066f))
* **input:** improve arrow key history navigation and command block handling ([#114](https://github.com/golish-ai/golish/issues/114)) ([623fc83](https://github.com/golish-ai/golish/commit/623fc8386acc6a6b62b53967c4faadc1dc9e2cde))
* **session:** sync session workspace path when cwd changes ([#122](https://github.com/golish-ai/golish/issues/122)) ([7053340](https://github.com/golish-ai/golish/commit/70533406f00ccf3ae92d76c3f82748c21f95d63f))
* **ui:** apply agent mode to backend when loading project settings ([#119](https://github.com/golish-ai/golish/issues/119)) ([4d2b516](https://github.com/golish-ai/golish/commit/4d2b5166299c775aaf904a17e2210009de312839))
* **ui:** remove git loading spinner and improve streaming auto-scroll ([#117](https://github.com/golish-ai/golish/issues/117)) ([f385d64](https://github.com/golish-ai/golish/commit/f385d64656ea0d065d2b0bdee82c40a04fa4abef))


### Refactoring

* **golish:** use if let instead of match for single variant ([d5d3470](https://github.com/golish-ai/golish/commit/d5d347099b34588d621862b41e979a85af5b24af))
* **sub-agents:** use natural language output for analyzer and explorer ([#120](https://github.com/golish-ai/golish/issues/120)) ([3751bf0](https://github.com/golish-ai/golish/commit/3751bf09680c7e68a3cba8d486cb7854b18b9131))
* **window:** move window state persistence from frontend to Rust backend ([e772fc0](https://github.com/golish-ai/golish/commit/e772fc0157a8bb484886dbd0d948a0fad4472e6f))

## [0.2.4](https://github.com/golish-ai/golish/compare/v0.2.3...v0.2.4) (2026-01-06)


### Features

* **ai:** add dynamic prompt composition system ([6215cdc](https://github.com/golish-ai/golish/commit/6215cdccbf0ac49e9e8a424214372633b5fd04fe))
* **ai:** add multi-modal image attachment support ([#101](https://github.com/golish-ai/golish/issues/101)) ([bd1b836](https://github.com/golish-ai/golish/commit/bd1b83681074943c22fe9d33d3705296b5f7c205))
* **ai:** add OpenAI native web search integration ([b6525d5](https://github.com/golish-ai/golish/commit/b6525d56f3e6f63033ba1dd6ba8f66931d50e125))
* **capabilities:** enhance Z.AI support with preserved thinking mode and reasoning continuity ([3c823fe](https://github.com/golish-ai/golish/commit/3c823fe6d6c51fe77af480fc68ef9c5d28dae5d5))
* **evals:** add metric pass threshold logic for providers ([0433627](https://github.com/golish-ai/golish/commit/0433627d8634b39120d2066a399bca44b971b46e))
* **evals:** add OpenAI model scenarios and connectivity test framework ([4253ec7](https://github.com/golish-ai/golish/commit/4253ec7e10f2fb623e41b206ccab1ae1537fd9da))
* **evals:** add OpenAI provider and upgrade rig-core to 0.27.0 ([#82](https://github.com/golish-ai/golish/issues/82)) ([6adee68](https://github.com/golish-ai/golish/commit/6adee68c58d8da93c8f0285c6c5e450a991e2078))
* **evals:** add Z.AI GLM-4.7 provider support ([#75](https://github.com/golish-ai/golish/issues/75)) ([cb8c722](https://github.com/golish-ai/golish/commit/cb8c72210a0cfce7136be757ab6c3352081818a8))
* **evals:** align eval system prompts with production agent ([#95](https://github.com/golish-ai/golish/issues/95)) ([5fbc8c5](https://github.com/golish-ai/golish/commit/5fbc8c5c96bc5b520848b122623e59dfb7829425))
* **sub-agents:** add sub-agent support with timeline integration and E2E tests ([0f3a768](https://github.com/golish-ai/golish/commit/0f3a768eee38ddf015ed71cd3e048030508b5d0c))
* **terminal:** add portal-based rendering for Terminal state persistence ([89bc8bf](https://github.com/golish-ai/golish/commit/89bc8bff71803b2895b7aba6e5345b1c8b8be32d))
* **terminal:** add React portal architecture for Terminal persistence ([bdd0d5d](https://github.com/golish-ai/golish/commit/bdd0d5dc434d9a41194893e6aa40ef04f6c8fcfa))
* **terminal:** add TerminalInstanceManager for cross-remount persistence ([2b300ce](https://github.com/golish-ai/golish/commit/2b300ce3a6c981cbbab454d33c09260061c6ac74))
* **terminal:** integrate portal system and preserve tab state ([de3e40e](https://github.com/golish-ai/golish/commit/de3e40e58bc97e3a6ad1292925d2bd3abb817992))
* **tools:** add ast-grep tools for structural code search and replace ([#94](https://github.com/golish-ai/golish/issues/94)) ([ab15841](https://github.com/golish-ai/golish/commit/ab158416578852015a8d7f41cb83707edc58a70b))
* **ui:** add 3-level nested model selector with temperature support ([b729dbf](https://github.com/golish-ai/golish/commit/b729dbf33db1c70a2d35775cb029a2974f120ae8))
* **ui:** add comprehensive OpenAI model support ([#83](https://github.com/golish-ai/golish/issues/83)) ([281135a](https://github.com/golish-ai/golish/commit/281135a3c01f29cf4e79bb9b0c7e7bfc25ca6939))
* **web-tools:** add native web search and web fetch support for Claude ([35044cf](https://github.com/golish-ai/golish/commit/35044cfed47984ac08520d8e45a117fac7f3cfce))
* **workflows:** implement new workflows with structured schemas ([f85aafb](https://github.com/golish-ai/golish/commit/f85aafbb598f2ab677bf95c9f00dee203f41e7f7))


### Bug Fixes

* **ai:** preserve OpenAI Responses API reasoning IDs across turns ([#92](https://github.com/golish-ai/golish/issues/92)) ([1793e66](https://github.com/golish-ai/golish/commit/1793e66fe02d64cd73713c595938039fda35f15f))
* **ci:** enable ad-hoc code signing for macOS builds ([86003bf](https://github.com/golish-ai/golish/commit/86003bfdca87d475098e8114505d39d31e5c9d28))
* **ci:** enable ad-hoc code signing for macOS builds ([dd63a26](https://github.com/golish-ai/golish/commit/dd63a2611488584270f82657883a1c3d7a1ddb72))
* **ci:** only run release build when release is created ([1ecb190](https://github.com/golish-ai/golish/commit/1ecb1901b5c1fca4c4761518344d8111863153b0))
* **ci:** only run release build when release is created ([402ba93](https://github.com/golish-ai/golish/commit/402ba93e587263219504c5607cbc54b2635f9ffe))
* **e2e:** exclude xterm helper textarea from selectors ([9572fd9](https://github.com/golish-ai/golish/commit/9572fd94d3bce3c35e14559d02430a8f0462c6b5))
* **e2e:** replace non-null assertions with proper null checks ([5610bb6](https://github.com/golish-ai/golish/commit/5610bb6fd03fd26e137df6c05a773e6dc1f35f8b))
* **e2e:** update OpenAI model tests for nested dropdown menus ([e79454b](https://github.com/golish-ai/golish/commit/e79454b682ca53a538ea408a1e7c1ce735f82d5e))
* **evals:** add ast-grep tools to eval system prompt and fix LLM score parsing ([#99](https://github.com/golish-ai/golish/issues/99)) ([54625a9](https://github.com/golish-ai/golish/commit/54625a9394afcb9e95732dea10ef56d1efd965ad))
* **evals:** improve eval reliability and build performance ([#85](https://github.com/golish-ai/golish/issues/85)) ([718fd2f](https://github.com/golish-ai/golish/commit/718fd2fa09e6f57a0da80d60f862944382a04567))
* **evals:** improve LLM judge reliability and prompt composition tests ([#80](https://github.com/golish-ai/golish/issues/80)) ([4322ca9](https://github.com/golish-ai/golish/commit/4322ca9793dc3c7788d41b66be8338f9ff214d34))
* file editor dirty/clean indicator now correctly reflects undo state ([#102](https://github.com/golish-ai/golish/issues/102)) ([e8f84a3](https://github.com/golish-ai/golish/commit/e8f84a32145535b4fa8853962d7d3af2053f567c))
* **keybinds:** separate Ctrl+D close from Cmd+D split on macOS ([54405ba](https://github.com/golish-ai/golish/commit/54405bae74cde07099c249ef4ee5943ccc046254))
* **pty:** fall back to home directory when cwd is root ([683465d](https://github.com/golish-ai/golish/commit/683465de16e79ae76002825377b48309377baef6))
* **pty:** fall back to home directory when cwd is root ([234d94c](https://github.com/golish-ai/golish/commit/234d94c14df6be20b1bf7f13c9d4a8097da0c0e2))
* **shell:** load PATH from shell rc files in run_command tool ([407c686](https://github.com/golish-ai/golish/commit/407c686d22ae66198bb64d1d2ea4eed56febc065))
* **shell:** load PATH from shell rc files in run_command tool ([#96](https://github.com/golish-ai/golish/issues/96)) ([f401404](https://github.com/golish-ai/golish/commit/f40140460ca565b0a4bc65a93a69cb53386b0b54))
* **terminal:** improve initialization and fullterm mode transitions ([e6da594](https://github.com/golish-ai/golish/commit/e6da5949b6f111fc794e67ce152d485a286add23))
* **terminal:** improve resize debouncing and pane focus handling ([4e8d48c](https://github.com/golish-ai/golish/commit/4e8d48c7f22d25e411e8ca24f35cc537ce349468))


### Refactoring

* **ai:** consolidate agentic loop implementations ([#87](https://github.com/golish-ai/golish/issues/87)) ([c1c20eb](https://github.com/golish-ai/golish/commit/c1c20eb7d3e82c4e96cd2ed68d31e0ea919abc3c))
* **ai:** redesign system prompts with structured XML format ([#89](https://github.com/golish-ai/golish/issues/89)) ([432ee8e](https://github.com/golish-ai/golish/commit/432ee8ebf2e3c3f7c96a0c09685681b393ca54d8))
* **build:** improve test and check scripts with silent outputs and clearer messaging ([4f329de](https://github.com/golish-ai/golish/commit/4f329de994e263d3d0d89dcf15f66bd1abf0cf45))
* **build:** improve test and check scripts with silent outputs and clearer messaging ([8e1b5ba](https://github.com/golish-ai/golish/commit/8e1b5ba0afcaec08bdc84a4e89ac3f72b9f0d2ef))
* **evals:** use &Path instead of &PathBuf in LLM judge helpers ([5bc6801](https://github.com/golish-ai/golish/commit/5bc68012366d23aa6515b73891d4fdc22a803f9a))

## [0.2.3](https://github.com/golish-ai/golish/compare/v0.2.2...v0.2.3) (2025-12-31)


### Bug Fixes

* **ci:** build golish-cli sidecar for release bundling ([#72](https://github.com/golish-ai/golish/issues/72)) ([b6cc102](https://github.com/golish-ai/golish/commit/b6cc1025409becb394e32bd099d397d5eaa3555f))

## [0.2.2](https://github.com/golish-ai/golish/compare/v0.2.1...v0.2.2) (2025-12-31)


### Bug Fixes

* **ci:** configure Tauri action project path for release builds ([#70](https://github.com/golish-ai/golish/issues/70)) ([d63c464](https://github.com/golish-ai/golish/commit/d63c464dc913cf07d53754cea3ad3f69373d70be))

## [0.2.1](https://github.com/golish-ai/golish/compare/v0.2.0...v0.2.1) (2025-12-30)


### Features

* **ai:** add OpenAI Responses API support and standardize temperature ([#67](https://github.com/golish-ai/golish/issues/67)) ([debae67](https://github.com/golish-ai/golish/commit/debae67f41bd41ee52d445c500e515b171e51815))
* **ui:** add multi-pane support for split terminal layouts ([#65](https://github.com/golish-ai/golish/issues/65)) ([0d3d306](https://github.com/golish-ai/golish/commit/0d3d306577bcb77935d3bcfaa6986a8055856225))


### Bug Fixes

* **build:** specify mainBinaryName to fix macOS release bundling ([#68](https://github.com/golish-ai/golish/issues/68)) ([c43ccd0](https://github.com/golish-ai/golish/commit/c43ccd05e29734ae3f8cb65ea32a741af5730877))

## [0.2.0](https://github.com/golish-ai/golish/compare/v0.1.0...v0.2.0) (2025-12-29)


### ⚠ BREAKING CHANGES

* **sidecar:** Sidecar API completely rewritten.

### Features

* add conversation-level token usage tracking ([#49](https://github.com/golish-ai/golish/issues/49)) ([ac21420](https://github.com/golish-ai/golish/commit/ac214209d6e846540b835485f579fba50deab170))
* add event mocking support to Tauri IPC mocks ([6cccd44](https://github.com/golish-ai/golish/commit/6cccd44e21e9718acf058b2f37208eae13816b31))
* add MockDevTools panel for browser-mode development ([926edc3](https://github.com/golish-ai/golish/commit/926edc31718bb09e2313a6b2dbb4a2c970ea6a10))
* add OpenRouter model support ([397d79b](https://github.com/golish-ai/golish/commit/397d79b27bf326086326c4da158cbe1e445ddca4))
* add path completion commands and React hook for use in Tauri terminals ([3715bb0](https://github.com/golish-ai/golish/commit/3715bb0d2a59b4654e2e71081ed2d07b22a8e29a))
* add preset scenarios to MockDevTools ([d76370c](https://github.com/golish-ai/golish/commit/d76370c189ad31e24a3e8094f8c2e0e20c8241a8))
* add Tauri IPC mock adapter for browser-only development ([74b3fe3](https://github.com/golish-ai/golish/commit/74b3fe3cc5c683b798075e30d0feb101410f8b24))
* add theme settings to settings ([6b0bc95](https://github.com/golish-ai/golish/commit/6b0bc95a3194d08eb5e67ae77bad0486086f419e))
* Add theme system with background image support ([26367bf](https://github.com/golish-ai/golish/commit/26367bf559b358398c904451c95fb5cf4f5cb629))
* **ai:** add agent mode support for flexible tool approval behavior ([7420204](https://github.com/golish-ai/golish/commit/742020469dce253708a0c147ed9aae4cbf2b6929))
* **ai:** add dynamic memory file lookup from settings ([e1ffaec](https://github.com/golish-ai/golish/commit/e1ffaec2434a9b8a825d932ca1a6c2e9c4668f4e))
* **ai:** add extended thinking mode and UI for reasoning content ([d95dd0d](https://github.com/golish-ai/golish/commit/d95dd0dfddbad2f96ce7230ab941943d9eca6de6))
* **ai:** add multi-provider support for Anthropic, Ollama, Gemini, Groq, and xAI ([708b804](https://github.com/golish-ai/golish/commit/708b804499a43dd32ec20f8168255a872d57e0f2))
* **ai:** add multi-provider support for Anthropic, Ollama, Gemini, Groq, and xAI ([2f86e9c](https://github.com/golish-ai/golish/commit/2f86e9cfc536687fb52c2969621923ce7db2ffe7))
* **ai:** add OpenAI provider support ([9fba524](https://github.com/golish-ai/golish/commit/9fba524a231574ea1a6db27843ebac7d2f7f87b3))
* **ai:** add OpenAI provider support to Rust backend ([9591ffc](https://github.com/golish-ai/golish/commit/9591ffcdc934641e9af3495b41467dea5d484fd2))
* **ai:** add OpenRouter provider support for arbitrary model IDs ([35eeeb7](https://github.com/golish-ai/golish/commit/35eeeb76e5a7c15b25a2557ccc36e97cbbec7375))
* **ai:** add rig-zai crate for Z.AI thinking/reasoning support ([a302916](https://github.com/golish-ai/golish/commit/a3029164bc20cf9f1ac00036b509735768b9c1f5))
* **ai:** add web search tools via Tavily integration ([fcd2d73](https://github.com/golish-ai/golish/commit/fcd2d730e07f486c9835ca015e8bb7064e225a3b))
* **ai:** add web search tools via Tavily integration ([51f1ba7](https://github.com/golish-ai/golish/commit/51f1ba7fd21f180cf8ab5b864fd372101d236e50))
* **ai:** add Z.AI GLM provider support ([5694751](https://github.com/golish-ai/golish/commit/5694751772bbece4b108b2605ceea8812c69d8ca))
* **ai:** add Z.AI GLM provider support ([#47](https://github.com/golish-ai/golish/issues/47)) ([b5e94f5](https://github.com/golish-ai/golish/commit/b5e94f57a3646c87f3c0e1f7e8abc572d45bbedf))
* **ai:** enable all tools for main agent and fix HITL session bug ([9978404](https://github.com/golish-ai/golish/commit/99784047a60907f28e400f6c78df2c85ad9b5997))
* **ai:** enable all tools for main agent and fix HITL session bug ([7755f25](https://github.com/golish-ai/golish/commit/7755f2543811830ce95d2dd4b761072bf7da2bf3))
* **ai:** enhance reasoning processing and UI for extended thinking mode ([401cd49](https://github.com/golish-ai/golish/commit/401cd499ae356d899caf51fd962f31fa3cb7ea0e))
* **ai:** extend AgentBridge and LLM client with Z.AI provider integration ([9543634](https://github.com/golish-ai/golish/commit/9543634e59ff041d180a4b86c902b2d76e586421))
* **ai:** implement udiff editing sub-agent ([499a1ea](https://github.com/golish-ai/golish/commit/499a1ead8904e544a69f8aaed9b3de08b90f811c))
* **ai:** integrate Z.AI GLM provider with full backend and frontend support ([228272a](https://github.com/golish-ai/golish/commit/228272ad64db9063a83f969c257c08ad6d2c2777))
* **ai:** Introduce modular sub-agent execution framework ([df6dc8f](https://github.com/golish-ai/golish/commit/df6dc8fdeff9d99ea5dc56d88c9b2f93a67537db))
* **ai:** introduce task planning and management system ([9d7ded8](https://github.com/golish-ai/golish/commit/9d7ded8c9ebaf5b284fdf583d0429475432dc85a))
* **ai:** unify provider initialization and enhance multi-provider support ([61d13b6](https://github.com/golish-ai/golish/commit/61d13b62183b643e280b097052fc046e89167a34))
* **ai:** wire memory file setting to agent system prompt ([57dbd57](https://github.com/golish-ai/golish/commit/57dbd5767901cfd2dc18bec200d0268697fd7283))
* **cli:** implement interactive REPL mode and enhance terminal and JSON output ([e3be1f9](https://github.com/golish-ai/golish/commit/e3be1f9a28f7389179c8dbe19b4b0ced568d13a9))
* **context-panel:** add context panel and backend support for enhanced session management ([50fe5f0](https://github.com/golish-ai/golish/commit/50fe5f0a99fad85ff1a98862408604f1549b1a58))
* **context:** implement context compaction with end-to-end wiring ([8dd93b8](https://github.com/golish-ai/golish/commit/8dd93b8c882bf2059787aacfa78447c345a04b53))
* **evals:** add custom sidecar scorers, utilities, and integration tests ([ee3d256](https://github.com/golish-ai/golish/commit/ee3d25612accd9ea972c7cf171b1f92a3dd30f2c))
* **evals:** add DeepEval-based evaluation framework for golish-cli ([0167435](https://github.com/golish-ai/golish/commit/016743535a83f1c5802463e12d757488b74dc00b))
* **evals:** add Rust-native evaluation framework with rig ([b1886cb](https://github.com/golish-ai/golish/commit/b1886cb85d01787899c766afe46a3732b77bfa32))
* **evals:** add Rust-native evaluation framework with rig ([c3b443e](https://github.com/golish-ai/golish/commit/c3b443eb49ed0b561deb8d23480eaa08c48d2451))
* **evals:** enhance memory recall scenarios and CLI testing framework ([f2fce75](https://github.com/golish-ai/golish/commit/f2fce75142aa49f79b119849849a39a667225392))
* **evals:** introduce Layer 1 session state support with scorers, utilities, and API types ([cd8ee8f](https://github.com/golish-ai/golish/commit/cd8ee8f80b14bb162017b7e8aa3531e2995b9ca0))
* **evals:** Rust-native evaluation framework with rig ([4fd37f2](https://github.com/golish-ai/golish/commit/4fd37f2bbd9af061b5d9c1f30ec9709477d3771c))
* **frontend:** add migrateCodebaseIndex wrapper ([3cb6903](https://github.com/golish-ai/golish/commit/3cb690337914639b35c48281cad5f9f063f1eb31))
* **indexer:** add codebase management commands ([7f328ff](https://github.com/golish-ai/golish/commit/7f328ff6849eca7fcc487cbfb388907c26182872))
* **indexer:** add configurable global index storage location ([a647877](https://github.com/golish-ai/golish/commit/a647877550b80eb65dac2a0a1111af8b5044ecd3))
* **indexer:** add paths module for index directory resolution ([6988a0b](https://github.com/golish-ai/golish/commit/6988a0bf2885a8d347e785557c83664520f30806))
* **indexer:** integrate configurable storage location ([10d2bfb](https://github.com/golish-ai/golish/commit/10d2bfbbad175066ce13a4366721e18e1fdfb5ac))
* **input:** add @ file reference commands for agent mode ([b0a7648](https://github.com/golish-ai/golish/commit/b0a7648cdf9eec3cf3549d88b84dd4fd94369c9f))
* **input:** improve path completion with final selection handling ([bb031eb](https://github.com/golish-ai/golish/commit/bb031eb1d835f87fea600351ef73dc685e980769))
* **input:** integrate @ file commands into UnifiedInput ([9477071](https://github.com/golish-ai/golish/commit/94770712c2cf2f1bc4380d9fbe36bd1bcb23cfc9))
* **mock-devtools:** implement incremental diffs, baselines, and context improvements ([7e2e500](https://github.com/golish-ai/golish/commit/7e2e500dfab98ccf6bc5899d1a058e97591731b7))
* **mock-devtools:** implement incremental diffs, baselines, and context improvements ([696f93b](https://github.com/golish-ai/golish/commit/696f93b0a81d3fe6f269884abfd8f8c11d199305))
* **models:** update Gemini and Groq model lists ([30aa25a](https://github.com/golish-ai/golish/commit/30aa25a04a42a46fa73a74383a3de74eb3e00e89))
* **models:** update model lists and defaults for Gemini, Groq, and xAI ([bca43aa](https://github.com/golish-ai/golish/commit/bca43aac3ff2462efab6ecebd94eca453f3c6628))
* per-session AI agent isolation ([69a3bc5](https://github.com/golish-ai/golish/commit/69a3bc5510b9ef75396d59aaaffbb209bc705ef9))
* **pty:** detect alternate screen buffer via ANSI CSI sequences ([29e77b4](https://github.com/golish-ai/golish/commit/29e77b40478c69d5abc7866b9cfc21e711043883))
* register workflow commands in Tauri app ([04679b8](https://github.com/golish-ai/golish/commit/04679b8d208b55c648ef0925db3fa05b8c5e3305))
* **rig-zai:** add custom streaming with reasoning_content support ([5abf6e5](https://github.com/golish-ai/golish/commit/5abf6e5f8ff5a3815ca8e6c2c42607481169d3dd))
* **rig-zai:** enable thinking mode for GLM-4.7 ([4a3984f](https://github.com/golish-ai/golish/commit/4a3984f0b62c9eb926bffe0121c5d46f36f73727))
* **runtime:** abstract event emission with runtime and CLI support ([afd5b51](https://github.com/golish-ai/golish/commit/afd5b5105ef3574104c685b6816e968a9761937d))
* **runtime:** enable event emission support with Tauri integration and enhanced Layer 1 logging ([4bdd872](https://github.com/golish-ai/golish/commit/4bdd872a0765417b3b96560e3ad454d2a75e1a50))
* **server:** add HTTP/SSE server support for CLI and evaluation framework ([c399149](https://github.com/golish-ai/golish/commit/c399149f051f265e1363176304d04922e6c5e3c8))
* **settings:** add CodebaseConfig schema for codebase management ([d742755](https://github.com/golish-ai/golish/commit/d74275588f0ae330dc716cb33ea9fc34ba83e091))
* **settings:** add fullterm_commands setting for custom TUI apps ([336c09f](https://github.com/golish-ai/golish/commit/336c09f52094231d042bc613b1cbd63af75e98ac))
* **settings:** add IndexLocation enum for configurable index storage ([2aa7b35](https://github.com/golish-ai/golish/commit/2aa7b354624dc69b2c8401d314f4c1c02971bd84))
* **settings:** add provider visibility toggle for model selector ([de402f6](https://github.com/golish-ai/golish/commit/de402f6d033b87578e9bfc90fd5726f54a471f1c))
* **settings:** add provider visibility toggle UI ([15fe28b](https://github.com/golish-ai/golish/commit/15fe28b95c32e403c9f0666081d07133cf49f903))
* **settings:** add settings system with UI and settings.toml ([8edfed7](https://github.com/golish-ai/golish/commit/8edfed7ed21a675cd6b8b153d34548a2533e4f30))
* **settings:** add show_in_selector field to AI provider settings ([b441114](https://github.com/golish-ai/golish/commit/b441114a05a87169107b1a29a6fc6545ee60eddb))
* **shell:** add multi-shell support for bash and fish ([e629585](https://github.com/golish-ai/golish/commit/e629585828679e75cf69ce23c34f550c9b769370))
* **shell:** add venv reporting to shell integration scripts ([03ace38](https://github.com/golish-ai/golish/commit/03ace38ac0d4b25b8eed9080a6c6c4714446c656))
* **sidecar:** add context capture system for session tracking ([07cfa37](https://github.com/golish-ai/golish/commit/07cfa37615020f1fe3f27d7420c3adb5b56ff807))
* **sidecar:** add context capture system for session tracking ([b5105bb](https://github.com/golish-ai/golish/commit/b5105bba3bac855a32c23586900f593d66bd9c92))
* **sidecar:** add optional `local-llm` feature for mistral.rs integration ([4078d03](https://github.com/golish-ai/golish/commit/4078d03041b14701fe9cdfd2dafcc15d68c4355b))
* **sidecar:** add session resume and matching functionality to enhance context restoration ([591d9ab](https://github.com/golish-ai/golish/commit/591d9abe5c4d7c2ecfacc6dafeb598ea10208597))
* **sidecar:** add session resume and matching functionality to enhance context restoration ([18cac0a](https://github.com/golish-ai/golish/commit/18cac0ad03d99464d9e27bde85d063fb4acead84))
* **sidecar:** enhance context panel with patches and artifacts integration ([2f62e07](https://github.com/golish-ai/golish/commit/2f62e07e417e0a7b6073833d6e5d7907828d70e7))
* **sidecar:** enhance LLM-based state management and context panel UI ([ffb8aca](https://github.com/golish-ai/golish/commit/ffb8acaf07a319c53e0a7ab82744beace85289f2))
* **sidecar:** enhance synthesis metadata, context panel, and settings ([863daf8](https://github.com/golish-ai/golish/commit/863daf8db67b5f288799d9bfdac3fc0e0bf7c1d7))
* **sidecar:** expand session diagnostics and enhance GCP token handling ([9cdbc83](https://github.com/golish-ai/golish/commit/9cdbc83e75915461c409c057c1882897254a50d9))
* **sidecar:** implement LLM-based commit message generation ([bc58b2e](https://github.com/golish-ai/golish/commit/bc58b2ecf6f4bc669aa253f97d5aaaf4b9dc1311))
* **sidecar:** introduce schema verification tests and embeddings support ([c72a362](https://github.com/golish-ai/golish/commit/c72a36256be34e0a64b9071077dbe4351b41c666))
* **sidecar:** remove session_start events and extend event schema ([45678df](https://github.com/golish-ai/golish/commit/45678df2b88b49d46e3b912591bd7038c35ac77f))
* **statusbar:** filter model selector based on provider visibility ([c677c81](https://github.com/golish-ai/golish/commit/c677c8107503fab76f3131e48e9daa74161ca3a0))
* **store:** add renderMode state for terminal display modes ([d0d2b18](https://github.com/golish-ai/golish/commit/d0d2b18d0e8ee0881175448f16fab45752301a8b))
* **tabs:** customizable tab names and process display ([1be50ec](https://github.com/golish-ai/golish/commit/1be50ecf81b615b0178c64382d594842fb479aed))
* **tabs:** customizable tab names and process display ([259f337](https://github.com/golish-ai/golish/commit/259f33702a4e8f0f1628dfbcb6449b35a46828c8))
* **terminal:** add DEC 2026 synchronized output and improve TUI compatibility ([#48](https://github.com/golish-ai/golish/issues/48)) ([21b3cfd](https://github.com/golish-ai/golish/commit/21b3cfd743962d051bf07016e60ef2f0cd0cd550))
* **terminal:** add fullterm mode for interactive CLI apps ([016b1d7](https://github.com/golish-ai/golish/commit/016b1d724772e8d67f69c8fa8ebd8903e5796fa8))
* **terminal:** add fullterm mode with auto-switch for interactive commands ([5b735dc](https://github.com/golish-ai/golish/commit/5b735dc5355ed7e7f99783dbc756554a907f0c01))
* **terminal:** add virtual environment detection and display ([6dc5292](https://github.com/golish-ai/golish/commit/6dc5292c787702725c32d3ae4109fc1c45c013aa))
* **terminal:** add VirtualTerminal for ANSI sequence processing ([a87f16a](https://github.com/golish-ai/golish/commit/a87f16a70c9e31c617f0ae9652c9ebee53f43ccc))
* **terminal:** add VirtualTerminalManager and useProcessedOutput hook ([f8bc50e](https://github.com/golish-ai/golish/commit/f8bc50ed8e5fe1b4c289e31d6fbdffd74cc4654e))
* **terminal:** integrate VirtualTerminal for pending command output ([58b760f](https://github.com/golish-ai/golish/commit/58b760f0ba49eea9e12d8e2301392a0331931d14))
* **themify-ui:** extend theme tokens to more ui components ([37cddc8](https://github.com/golish-ai/golish/commit/37cddc8f90a3c8e531ca919c3287f85fef8b2992))
* **themify-ui:** extend theme tokens to more ui components ([fdf97d2](https://github.com/golish-ai/golish/commit/fdf97d262afab3aa2fdf8cd04cf183f645ea574c))
* **theming:** add theme support ([bcce8bc](https://github.com/golish-ai/golish/commit/bcce8bce7cb49aaefc95dd7ccd046713c58ec58f))
* **ui:** add accessibility labels to input mode toggle buttons and implement input focus e2e tests ([3c3b878](https://github.com/golish-ai/golish/commit/3c3b878e83ba22daa82265f1e4fc5b37e8490ad2))
* **ui:** add Codebases settings tab for managing indexed repos ([a1cddfd](https://github.com/golish-ai/golish/commit/a1cddfd01dd88bea911fe9276c2ee991409ed77c))
* **ui:** add Codebases settings tab for managing indexed repositories ([81d30e0](https://github.com/golish-ai/golish/commit/81d30e0c4b775737222cb3e2b039a8c01143c138))
* **ui:** add copy button to markdown code blocks ([862d27f](https://github.com/golish-ai/golish/commit/862d27f1c4922c163f5b7ede469ff84cb24c8021))
* **ui:** add ctrl+R reverse history search ([96427b9](https://github.com/golish-ai/golish/commit/96427b97eb6494daf025c211581f7134a9a7ce84))
* **ui:** add diff view for edit_file tool results ([999044e](https://github.com/golish-ai/golish/commit/999044e8f204b60ad727e50efc0fec940d488d88))
* **ui:** add fullterm mode toggle and status indicator ([7dfb978](https://github.com/golish-ai/golish/commit/7dfb9786b75f7c665534511cb708558855fa7d32))
* **ui:** add OpenAI provider to frontend ([205e8bc](https://github.com/golish-ai/golish/commit/205e8bc7ee50e71d72657a1ed5811f235275b6b8))
* **ui:** add OpenRouter model selection to StatusBar and Settings ([a19fc35](https://github.com/golish-ai/golish/commit/a19fc35f61fae17e2964fa745f9a2fdedfe0e945))
* **ui:** add slash commands for user-defined prompts ([e730573](https://github.com/golish-ai/golish/commit/e730573eb9aeb0982f8061b31ee178c081f5d031))
* **ui:** add slash commands for user-defined prompts ([ae55346](https://github.com/golish-ai/golish/commit/ae5534694af8885adb81a9a41474529bcac4daa2))
* **ui:** add sub-agent tool call details display ([92121f5](https://github.com/golish-ai/golish/commit/92121f5e1a7e4e9f1d62472ad27c2c94c612a559))
* **ui:** add task planner panel and status bar integration ([0285ba8](https://github.com/golish-ai/golish/commit/0285ba8d8864d5b5ac436e043f22af9fc501f80d))
* **ui:** add terminal mode indicator to status bar ([15835b0](https://github.com/golish-ai/golish/commit/15835b0152b19db1d532dc7247000c65b63e30f7))
* **ui:** add terminal mode indicator to status bar ([20f9fe8](https://github.com/golish-ai/golish/commit/20f9fe89cc6bd9673d94adce5559f6327817d719))
* **ui:** add tool call details modal ([260312a](https://github.com/golish-ai/golish/commit/260312a325a91d486ba0c0eb1a5b03d93fbabf13))
* **ui:** add workflow UI components ([b0f26ed](https://github.com/golish-ai/golish/commit/b0f26ed6bed8428adcea3a0c15c321113c49a9c4))
* **ui:** add WorkflowTree component for hierarchical display ([0c2cf3e](https://github.com/golish-ai/golish/commit/0c2cf3e2ab73e7d02429ea0cd2e809f70a088c86))
* **ui:** display git branch in status bar ([#55](https://github.com/golish-ai/golish/issues/55)) ([a7c1c52](https://github.com/golish-ai/golish/commit/a7c1c526989a39f5beee3b839d6503c0c94ae44d))
* **ui:** enhance tool group and AI workflow integration ([72817fa](https://github.com/golish-ai/golish/commit/72817fa83c8c7b3b8e2f29215f891fa3c095825a))
* **ui:** implement native macOS titlebar with draggable region ([194fb6f](https://github.com/golish-ai/golish/commit/194fb6f917b20b68ff97939806b00d0e4a4a685b))
* **ui:** implement native macOS titlebar with draggable region ([05f88d5](https://github.com/golish-ai/golish/commit/05f88d5eee6da7cf9af9b4c48a34af3322e3677a))
* **ui:** integrate workflow system into application ([915c18a](https://github.com/golish-ai/golish/commit/915c18aa8912bf828f5e7a014921b5db72202566))
* **workflow:** add core workflow infrastructure ([2ed0f01](https://github.com/golish-ai/golish/commit/2ed0f01f4854aea0b0618fab1f85c5c8f094d54b))
* **workflow:** add git commit workflow agents ([ce31a55](https://github.com/golish-ai/golish/commit/ce31a55e22f1cd16ace30d99bd90ed6731abc226))
* **workflow:** add Tauri workflow commands ([8cab27c](https://github.com/golish-ai/golish/commit/8cab27cb171c3d53d674baf1120b7dca32d3c4f8))
* **workflow:** integrate workflow system with AI module ([d28833a](https://github.com/golish-ai/golish/commit/d28833a6f9129fa9a432e218054ebc9efe3bfa78))


### Bug Fixes

* add packages field to pnpm-workspace.yaml ([3e64c3b](https://github.com/golish-ai/golish/commit/3e64c3b7e342330d11a65d5badda9ea8cdf0c09c))
* **ai:** use camelCase for Tauri invoke parameters in session-specific commands ([7296b5d](https://github.com/golish-ai/golish/commit/7296b5d50a1a92014576dcec9e521adad128bd96))
* allow dead_code for unused HunkApplyError variant ([4e6d39d](https://github.com/golish-ai/golish/commit/4e6d39d2b48096aec10ac41d335ebc7c5d57c2d2))
* **app:** use function call for browser mode detection ([55cdbdb](https://github.com/golish-ai/golish/commit/55cdbdbe24515159374a756533d777a5bc8610f4))
* **ci:** make sccache gracefully fallback when unavailable ([0da1e88](https://github.com/golish-ai/golish/commit/0da1e88bc733e3880c05263c11a36a33aed8eeda))
* **ci:** remove pnpm caching to fix store path error ([28691b1](https://github.com/golish-ai/golish/commit/28691b1096ed6b39cbcae294b554cff40c0a3ad1))
* **ci:** resolve illegal path in release-please config ([#59](https://github.com/golish-ai/golish/issues/59)) ([d361bcc](https://github.com/golish-ai/golish/commit/d361bcca71546cbad9638d21670c78d08ba02159))
* **ci:** simplify release-please config for monorepo ([aaa1e3e](https://github.com/golish-ai/golish/commit/aaa1e3e1087350676a672a52ed1541ef6e312242))
* **ci:** simplify release-please config for monorepo ([e9ed088](https://github.com/golish-ai/golish/commit/e9ed08809dff5abc0ba79dab51cd5972115634f7))
* **ci:** update evals workflow for Rust evals framework ([f3442be](https://github.com/golish-ai/golish/commit/f3442bee571325c9317cc3e83d63c64b4ffddea4))
* **ci:** use built-in pnpm caching in setup-node ([d3dabde](https://github.com/golish-ai/golish/commit/d3dabde6b8e8ddff0c37ae93eb6f4123ffe2d6fb))
* correct command_block event format for terminal output ([0385546](https://github.com/golish-ai/golish/commit/03855466571b355ab8b54befa1d5a87965eaef31))
* **deps:** remove unused lancedb and vector DB dependencies ([e3acd83](https://github.com/golish-ai/golish/commit/e3acd83e8d0262b3a373a7ad07f33517164bfe11))
* displaying shell and ai responses ([04d5f62](https://github.com/golish-ai/golish/commit/04d5f62dc495a16885c1176f7db0b9192d405841))
* displaying shell and ai responses ([645007c](https://github.com/golish-ai/golish/commit/645007ce9fb5813efed846ec92a2579cae2cb185))
* **e2e:** add Z.AI provider to mock settings ([e6f3aa8](https://github.com/golish-ai/golish/commit/e6f3aa8495ec04f7b4ae9d295b8290f2ec12985b))
* **e2e:** clear notifications during test setup ([b55ed6e](https://github.com/golish-ai/golish/commit/b55ed6e8baeebecede782e69afc37bc7fdfce353))
* **e2e:** fix test locators and accessibility issues ([884c88f](https://github.com/golish-ai/golish/commit/884c88f24212965c625afc31aacc069edb89cf82))
* **e2e:** improve test reliability by waiting for app readiness ([9660326](https://github.com/golish-ai/golish/commit/966032696ec83de37f223fcf0f6c033ebe7757d5))
* **e2e:** replace waitForTimeout with auto-retrying assertions ([ffac557](https://github.com/golish-ai/golish/commit/ffac557b2776c9f38b2d3966557324da9f6fa4cf))
* **e2e:** use role-based dialog selector to avoid strict mode violation ([b5e8968](https://github.com/golish-ai/golish/commit/b5e89682fda5fa64dff01534e60c0980a1aba251))
* **frontend:** add Z.AI provider to StatusBar model selector ([49f36c7](https://github.com/golish-ai/golish/commit/49f36c7e274376df98216a04879ac1f55468aad4))
* **frontend:** use session working directory for AI agent initialization ([948bd21](https://github.com/golish-ai/golish/commit/948bd212923e366e881659ed5dcf083b4a7218de))
* handle plugin:event IPC commands in mocks ([2cd86ad](https://github.com/golish-ai/golish/commit/2cd86adcd15f3969ef137b5c48bc1df62c6ee155))
* implement proper event dispatching for mock system ([ac2fccb](https://github.com/golish-ai/golish/commit/ac2fccb053974d07605799ad06055389748c9bff))
* make mock event system work with ES module restrictions ([0b769d1](https://github.com/golish-ai/golish/commit/0b769d12dc3cf1c10fb1d7a5040030f667ae494b))
* **mocks:** return valid mock credentials for Vertex AI config ([9ae6115](https://github.com/golish-ai/golish/commit/9ae6115b411b331506f9799b72faf1bedc0e2bb7))
* **models:** update Anthropic models to Claude 4.5 and use constants ([0dbf35b](https://github.com/golish-ai/golish/commit/0dbf35bb2afeef8594f9620cb6f586587a4d4bb2))
* resolve clippy warnings for CI ([ce5f5fb](https://github.com/golish-ai/golish/commit/ce5f5fbb7958369f2f63f5bc8e7d4d4e5e6687cb))
* resolve IPv6 localhost issue for Playwright tests ([aa4e5a4](https://github.com/golish-ai/golish/commit/aa4e5a45fd4ac61a375de62a40beb2fb09de5ea8))
* resolve lint errors for CI checks ([816a183](https://github.com/golish-ai/golish/commit/816a18352c0553209d04bc3f2b72d4eeb7d33878))
* resolve test failures after sub-agent merge ([7efb6ee](https://github.com/golish-ai/golish/commit/7efb6ee0e1e177be79d9d5cb0177499c8bab478b))
* resolve test failures and improve test stability ([84110d3](https://github.com/golish-ai/golish/commit/84110d369ee61f5e070cada9c01385d80c2a9808))
* **rig-zai:** add budget_tokens and debug logging for thinking mode ([9a0fac2](https://github.com/golish-ai/golish/commit/9a0fac25e05d271d2c416bd9bba35ac86a8c4c66))
* **settings:** preserve codebase configs when saving settings ([21495d5](https://github.com/golish-ai/golish/commit/21495d5c90fabf2731102e149f3c3c92eb7474fe))
* **settings:** resolve fullscreen dialog layout and overflow issues ([d44a513](https://github.com/golish-ai/golish/commit/d44a513fa666d74599ee7c87dcd2d0d2d1ec14c2))
* **settings:** resolve fullscreen dialog layout and overflow issues ([c0d63fa](https://github.com/golish-ai/golish/commit/c0d63fa9ab069cadb924fae2f1cd66db883db690))
* **store:** refine Vertex AI provider validation and enhance TypeScript checks ([3ddac7e](https://github.com/golish-ai/golish/commit/3ddac7e138ccd45d377ba1ddf65878099e8a8210))
* **store:** skip command block creation in fullterm mode ([5310df8](https://github.com/golish-ai/golish/commit/5310df849ea6ea118695de25e8e0788c4a5b0e9e))
* **tabs:** allow closing the last tab ([022cf91](https://github.com/golish-ai/golish/commit/022cf91e9894ea880fd38e9c8bd4f0d439f4e6c5))
* terminal input focus ([6e89d95](https://github.com/golish-ai/golish/commit/6e89d95f2300a5f4a14e40b29b8364cf54f40ba0))
* **tools:** improve error messages for file path resolution ([e45d2c2](https://github.com/golish-ai/golish/commit/e45d2c245a9ac2dbb5e87acd56e9f720c35da393))
* **ui:** add min-h-0 to ContextPanel flex containers for proper scrolling ([863d0ba](https://github.com/golish-ai/golish/commit/863d0ba6283002a268304e37108d3f4c95d2a8de))
* **ui:** align streaming and completed agent response font styles ([ddde5b4](https://github.com/golish-ai/golish/commit/ddde5b4138b65bead3d10e783deec33169ee8e67))
* **ui:** align streaming and completed agent response font styles ([29ce21c](https://github.com/golish-ai/golish/commit/29ce21c365680de710292a798a46787883e883ba))
* **ui:** extend tool cards to full width like thinking cards ([39ee63f](https://github.com/golish-ai/golish/commit/39ee63f26fc612367e6563da9d490acd53545e92))
* **ui:** extend tool cards to full width like thinking cards ([3fe7b11](https://github.com/golish-ai/golish/commit/3fe7b110e1edaba361a8c1e4b864e19056e410a7))
* **ui:** reset input submission state when switching sessions ([586ef39](https://github.com/golish-ai/golish/commit/586ef399686802b98455c3c02f756f464395609c))
* **ui:** reset input submission state when switching sessions ([143aa2e](https://github.com/golish-ai/golish/commit/143aa2ef418b8cc58a6af7c353db9b07bbac7f94))
* **ui:** terminal input focus ([2fbbf44](https://github.com/golish-ai/golish/commit/2fbbf4458e324a204c5dab8aa7e5021f708d45d2))
* update CLI bootstrap for new sidecar API and add sidecar evals ([2a97aa7](https://github.com/golish-ai/golish/commit/2a97aa774fc11923301b092d819769166e2c99bb))


### Performance

* **ci:** add sccache and improve cargo caching for evals ([7f73747](https://github.com/golish-ai/golish/commit/7f73747d1324c00665df84277d72578a592dbbe3))
* **ci:** add sccache to check workflow ([8b3d3f0](https://github.com/golish-ai/golish/commit/8b3d3f046a7b39d8ba901b64567198f67e0ca504))
* **ci:** use debug build for evals (faster compile, network-bound runtime) ([8501459](https://github.com/golish-ai/golish/commit/8501459a32055dd46c7a06bedd37b7a76888bfa8))


### Refactoring

* add `#[allow(dead_code)]` for test-only functions and metadata ([dce7fa2](https://github.com/golish-ai/golish/commit/dce7fa2ed9340a9a9d71251ab32615c4f54267e9))
* add `#[allow(dead_code)]` to public API functions and structs ([c39b8e2](https://github.com/golish-ai/golish/commit/c39b8e2a70d9b0e33db87201eed3c2baa72dbe62))
* **agent-chat:** separate sub-agent and content blocks for improved rendering ([f9179d8](https://github.com/golish-ai/golish/commit/f9179d83dafe227be14729c45edf2f15f89c9b98))
* **ai, ui:** enhance Markdown rendering, sub-agent management, and streaming handling ([b1ac064](https://github.com/golish-ai/golish/commit/b1ac064a09b2e1047990862986e5ec4a4586fcfa))
* **ai:** Adjust defaults and improve error handling in agentic loop ([1d9183e](https://github.com/golish-ai/golish/commit/1d9183ee4a378794af8f0f80218130c1088f85ab))
* **ai:** improve code structure and reuse across modules ([c89f70f](https://github.com/golish-ai/golish/commit/c89f70f340ce6d07652ab3242761ae59135926f1))
* **ai:** remove PromptContext and simplify prompt handling ([d6697f4](https://github.com/golish-ai/golish/commit/d6697f41745fa4b80c7850463a130cef937ed4bc))
* **ai:** remove unused is_default method from AgentMode ([5271b1e](https://github.com/golish-ai/golish/commit/5271b1edb1d1bd99cdaf4f77197dfcf054704e93))
* **ai:** remove unused methods and tests, simplify handling across modules ([de323ab](https://github.com/golish-ai/golish/commit/de323abaf3f88428c2d79ae42b6fc839ff20149c))
* **ai:** reorganize commands module into logical submodules ([861fa7e](https://github.com/golish-ai/golish/commit/861fa7e49337124e56af062c874fd4e0c12165a8))
* **cli:** remove indexer initialization from CLI bootstrap ([ac55ca8](https://github.com/golish-ai/golish/commit/ac55ca8c08f0ded59afa7b32abc49b6411bac68f))
* **cli:** simplify `CliRuntime::new` invocation and remove redundant newline in `session.rs` ([8aea922](https://github.com/golish-ai/golish/commit/8aea922989b66995308dd30942a52ed98edb9ad8))
* **CommandPalette, UnifiedInput:** Simplify mode handling with toggle logic ([df6dc8f](https://github.com/golish-ai/golish/commit/df6dc8fdeff9d99ea5dc56d88c9b2f93a67537db))
* **dependencies:** reorder imports in golish modules for consistency ([63fc1fc](https://github.com/golish-ai/golish/commit/63fc1fce4c3bc4f1723b614c6b964e122ffb7632))
* **eval:** simplify server handling and allow configurable workspace via env variable ([b38f5fc](https://github.com/golish-ai/golish/commit/b38f5fc108b5fb0ac576a53bdf984d75e81d1e16))
* Extract Rust backend into modular workspace crates ([#50](https://github.com/golish-ai/golish/issues/50)) ([37bffd1](https://github.com/golish-ai/golish/commit/37bffd184d26a7057976c8a060a010c5fa55d547))
* **frontend:** improve ANSI fallback and simplify UI ([5149c91](https://github.com/golish-ai/golish/commit/5149c9111502d76f2d3a29d7800e45acf052eacd))
* **frontend:** remove auto-indexing from app initialization ([1c39361](https://github.com/golish-ai/golish/commit/1c39361313a63e9088555105b5ff46b9946a4505))
* **frontend:** use ANSI-based fullterm mode detection ([4cc635e](https://github.com/golish-ai/golish/commit/4cc635ec8622167e5e61d153f456024dbc7451fc))
* Improve code readability, formatting, and AI workspace syncing ([96f489d](https://github.com/golish-ai/golish/commit/96f489d6c0750b0596187e6dcd0f2994ca4473e6))
* **logging:** enhance tracing for tool execution, session management, and PTY operations ([3c8d6eb](https://github.com/golish-ai/golish/commit/3c8d6ebe0c8cbf94e2b77aa55affed68d261cdc8))
* **mocks:** simplify `validateRequiredParams` function signature for cleaner readability ([25e0651](https://github.com/golish-ai/golish/commit/25e0651a00ddd4327187c0ee67256485c5a89be1))
* **models:** consolidate model definitions and simplify accessors ([ca9ea66](https://github.com/golish-ai/golish/commit/ca9ea66c9366019309bfb2905425d1c5d3dc2f75))
* optimize imports, formatting, and minor logic updates ([1281a9a](https://github.com/golish-ai/golish/commit/1281a9a86a1cea0557006e4b2d9c86100b6f10a9))
* **pty/manager:** prioritize `QBIT_WORKSPACE` for working directory resolution ([bc58b2e](https://github.com/golish-ai/golish/commit/bc58b2ecf6f4bc669aa253f97d5aaaf4b9dc1311))
* remove deprecated code and streamline API across core, cli, and ui ([8295346](https://github.com/golish-ai/golish/commit/8295346168d415abbb763d4200062a19f9e5c194))
* remove old monolithic workflow module ([70877b4](https://github.com/golish-ai/golish/commit/70877b4019191bfb87c5f478fda6c7986471d633))
* remove unused code and improve modularization across components ([8592008](https://github.com/golish-ai/golish/commit/85920085c338204ef0d84bcf6c0306a9daa40850))
* remove unused test cases and obsolete functions ([0e3bf7d](https://github.com/golish-ai/golish/commit/0e3bf7d548bba0923955db54efb24d3c0e634676))
* rename project directories for clarity (src-tauri→backend, src→frontend) ([97c01e9](https://github.com/golish-ai/golish/commit/97c01e9ca886e65fbaccc1dd00b24b694680ed54))
* rename src-tauri to backend and src to frontend ([83ed990](https://github.com/golish-ai/golish/commit/83ed990ae410b4dbdab1926365fef62132f65b89))
* **rig-zai:** simplify tool call handling and improve OpenAI compatibility ([ededc4e](https://github.com/golish-ai/golish/commit/ededc4e516be272973df9064e718e9d505d106a2))
* **rig-zai:** simplify tool call handling and improve OpenAI compatibility ([e7352b2](https://github.com/golish-ai/golish/commit/e7352b24d58c11f543acf488becfd5d0de4b19f4))
* **rust:** implement high-impact simplifications from rust-simplifier review ([f70e7e6](https://github.com/golish-ai/golish/commit/f70e7e6a93276f05c5b16f65636af518580a20a6))
* **sidecar:** make session management atomic and add idempotency tests ([9d8459d](https://github.com/golish-ai/golish/commit/9d8459ddfaee23f4611a3a56fcccac6d3bacc0b2))
* **sidecar:** replace LanceDB architecture with markdown-based sessions ([6813ab6](https://github.com/golish-ai/golish/commit/6813ab6da4850cde8ded20735a06b03e8ad66044))
* **sidecar:** replace LanceDB with markdown-based sessions ([b7f1a62](https://github.com/golish-ai/golish/commit/b7f1a62f8ebd33556d043892f0f6cceb56377d7c))
* **sidecar:** simplify session architecture and improve patch handling ([46e24e6](https://github.com/golish-ai/golish/commit/46e24e6af9afe28fbef66b9f59ee5195454f504b))
* **sidecar:** simplify session architecture and improve patch handling ([a7aa130](https://github.com/golish-ai/golish/commit/a7aa130a21083c737c4c2fea8a651de0efa7f0d0))
* simplify and reorganize agent evaluation tests ([5411847](https://github.com/golish-ai/golish/commit/5411847ad9626c426b08a01ae973fd4153e0e72f))
* **terminal:** add barrel export for Terminal component ([9713d87](https://github.com/golish-ai/golish/commit/9713d875244473b4859fde3b9abb0bf55df4f28c))
* **tests, workspace:** overhaul session and file operation tests; cleanup unused fixtures ([ed0769f](https://github.com/golish-ai/golish/commit/ed0769f180b1eca9a6175622f8e2a0592558d99f))
* **tests:** enhance batch prompt execution logging and verbose mode handling ([d26198b](https://github.com/golish-ai/golish/commit/d26198ba37ac91b23754e16cda0038684af93fed))
* **tests:** remove unused `test_events_jsonl_created` function from `test_sidecar.py` ([f20090e](https://github.com/golish-ai/golish/commit/f20090e7edb2f6eec24b511478fe4bd924835ae0))
* **tests:** replace `networkidle` with `domcontentloaded` in page load waits for e2e tests ([c618b1e](https://github.com/golish-ai/golish/commit/c618b1e820b03b4bfc3cf33e3c6abd3364a7fe82))
* **tests:** replace `networkidle` with `domcontentloaded` in page load waits for e2e tests ([a2f4fef](https://github.com/golish-ai/golish/commit/a2f4fef25195be5985ad075ff7f24e265dfce781))
* **theme:** replace hardcoded colors with CSS variables and improve component styles ([dce8f67](https://github.com/golish-ai/golish/commit/dce8f673d8088ab3401f226fa6add98da4395ae0))
* **tool-display:** replace inline expansion with modal details view ([dc00e77](https://github.com/golish-ai/golish/commit/dc00e777e46759da40b4d09953b9af4c3d0b33b6))
* **tool-display:** replace inline expansion with modal details view ([de86e26](https://github.com/golish-ai/golish/commit/de86e269cd891a2116427c45b526d0b47125cf09))
* **tools:** migrate from vtcode-core to golish-tools ([f8a3c9e](https://github.com/golish-ai/golish/commit/f8a3c9ec871ae0072f57e0072a9eab80c811ef7e))
* UI overhaul with shadcn components, added ComponentTestbed, and updated dependencies for improved modularity. ([840eac6](https://github.com/golish-ai/golish/commit/840eac6cef54cf507506081fb098d27e73b873ed))
* **ui, ai:** improve code sharing and clean up deprecated components ([c297c74](https://github.com/golish-ai/golish/commit/c297c74528d7b4805d312db58125f7f7b7e592d3))
* **ui:** adjust left margin and border styles for improved layout consistency ([7ec7876](https://github.com/golish-ai/golish/commit/7ec7876264d97ef3d7bf5e98767fa563e1d75a22))
* **ui:** enhance styles and improve component readability ([5165822](https://github.com/golish-ai/golish/commit/51658223dd0d2867fae32b3a9c8f44f3a1b1c417))
* **ui:** simplify `CommandBlock` styles and remove unused components ([f25d3d1](https://github.com/golish-ai/golish/commit/f25d3d1e832146149086925156a2c823de8badec))
* **ui:** simplify `WelcomeScreen` by removing unused sub-agent and workflow capabilities logic ([70f4df5](https://github.com/golish-ai/golish/commit/70f4df5132f1b57fec67770cf14c9b0b804dc893))
* vtcode migration part 1 - dead code cleanup and modularization ([a8919f9](https://github.com/golish-ai/golish/commit/a8919f9edf7bc72c80d20ba2c7aef593ea75c9e1))
