# Sub-agent model override wiped by DB-template reload race

- Date: 2026-06-12
- Status: implemented
- Scope: `golish-sub-agents` (registry), `golish-agent-bridge` (reload wiring)

## Symptom

In `--stage-run` (and any GUI session configuring `[ai.sub_agent_models.*]`), the
9 sub-agents that were configured to use a different model (e.g.
`xiaomi/mimo-v2.5-pro`) silently ran on the **main** model instead. Logs showed:

```
08:00:54.080411  Sub-agent 'pentester' configured to use xiaomi/mimo-v2.5-pro   (x9)
08:00:54.087164  [prompt-registry] Reloaded sub-agents with DB template overrides   <- +7ms
08:02:58        [sub-agent:pentester] Executing with main model (no override): provider=deepseek
```

The intended load-split ("main + refiner on deepseek, sub-agents on xiaomi") never
happened — all sub-agent traffic landed on the main model.

## Root cause (race, not a logic bug in the override setter)

`AgentBridge::set_db_backend` (`golish-agent-bridge/src/agent_bridge/config.rs`)
spawns a **detached** task that:

1. `await`s a DB query (`load_prompt_template_overrides`),
2. rebuilds all sub-agent definitions from defaults via
   `create_default_sub_agents_from_registry` — these defaults carry
   `model_override = None` (and `temperature/max_tokens/top_p = None`),
3. takes the registry write lock and calls `register_multiple`, whose `register`
   does `agents.insert(id, def)` — a **wholesale replacement** of each definition.

Meanwhile `configure_bridge` synchronously reaches
`configure_sub_agents -> apply_sub_agent_model_settings`, which sets those four
override fields from settings.

Because the spawned reload must round-trip the DB first, it lands a few ms **after**
the synchronous override application — and its `register_multiple` overwrites the
just-set overrides with default (None) values. The reload's only legitimate job is
to refresh `system_prompt` from DB templates; it should not reset model routing.

## Fix

Add `SubAgentRegistry::register_preserving_overrides`, used by the reload instead
of `register_multiple`. For each reloaded definition it carries over the four
runtime-override fields (`model_override`, `temperature`, `max_tokens`, `top_p`)
from the existing entry (matched by id) before inserting, so a prompt-template
refresh keeps model routing intact.

Race-safe in both orderings, because the merge reads the current entry under the
same registry write lock that every override writer
(`apply_sub_agent_model_settings`, `set_sub_agent_model`) takes:

- apply-then-reload (observed): reload reads the override that apply set -> kept.
- reload-then-apply: reload finds nothing to carry; apply runs later -> kept.

A brand-new agent id (no prior entry) inherits nothing and stays unoverridden.

## Tests

`golish-sub-agents` `definition::tests`:
- `test_register_preserving_overrides_keeps_runtime_overrides` — overrides survive
  a reload while `system_prompt` is still refreshed; a newly added agent stays
  unoverridden.
- `test_register_preserving_overrides_no_existing_is_noop` — reverse order; a later
  override write still sticks.
