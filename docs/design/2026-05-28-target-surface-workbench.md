# Target Surface Workbench

- **Date**: 2026-05-28
- **Status**: Draft / user-approved mock direction
- **Scope**: `frontend/components/TargetPanel`, target security-analysis APIs, recon pipeline launch surface
- **Mock**: `.codex-screenshots/target-surface-workbench-mock.svg`

## 1. Decision

After removing OWASP ZAP and the old SecurityView shell, Golish should not rebuild a generic "security tools" area. The next UI should be a target-centric ASM / recon workspace:

```text
Organization -> Target -> Surface -> Evidence -> Vulnerability Candidates
```

The approved direction is:

- Keep `Target Manager` as the primary navigation entry.
- Keep the organization tree as the left spine.
- Promote target details from an inline tree expansion into a formal **Target Surface Workbench**.
- Treat org-level pages as ownership/scope/intel workspaces.
- Treat target-level pages as recon/surface/evidence workspaces.

## 2. Current Problems

The current UI has useful pieces but the information architecture is muddy:

1. **Org and target concerns are mixed**
   - Organization profile, scope, subsidiaries, targets, candidate review, services, JS files, and operation logs all compete in one tree view.

2. **Target recon data is too small**
   - `TargetDetailView` already loads `targetAssetsList`, `apiEndpointsList`, `fingerprintsList`, `jsAnalysisList`, and `oplogListByTarget`.
   - But the data is rendered as a small inline expansion under a target row, so it does not feel like the user's next working area.

3. **Copy and flow still reference old shapes**
   - Empty state says "Network view" / "List", while the current UI has `Org Tree` / `Graph`.
   - Discover Assets still says orchestration is not wired, which conflicts with the desired org -> target -> recon loop.

4. **Actions are not staged**
   - Users need clear next actions: import targets, review scope, run baseline recon, collect JS, check sensitive exposure, match vulnerabilities.
   - These should map to deterministic evidence-producing steps, not vague "scan" buttons.

## 3. Target Mental Model

### Organization

An organization answers:

- Who owns the asset?
- What is in scope?
- Which subsidiaries/domains/IP ranges are candidates?
- Which evidence supports this profile?

Organization tabs:

- `Profile`: legal/company intel, DNS/MX, leakage intel, apps, aliases.
- `Scope`: authorization, in/out rules, customer-provided scope.
- `Targets`: confirmed in-scope target list.
- `Candidates`: discovered orgs/targets waiting for promotion.
- `Runs`: asset-intel and recon activity.
- `Evidence`: provider/tool evidence and raw provenance.

### Target

A target answers:

- What exact object can be tested?
- What services and HTTP origins exist?
- What JS/API/sitemap/sensitive signals were found?
- Which evidence proves each claim?
- What is the next allowed action?

Target tabs:

- `Identity`: host/IP/source/scope/CDN/ASN/DNS resolution.
- `Surface`: ports, services, HTTP probes, fingerprints.
- `Sitemap`: robots, sitemap.xml, crawler paths, interesting paths.
- `JS / API`: JS files, source maps, extracted API endpoints.
- `Sensitive`: confirmed/candidate secret exposure and source-map leaks.
- `Evidence`: replayable tool runs, imports, timestamps, raw pointers.

## 4. Approved Mock Layout

```text
┌──────────┬────────────────────┬──────────────────────────────────────────────┐
│ App Rail │ Organization Tree  │ Target Surface Workbench                     │
│          │                    │                                              │
│          │ Xiaomi Corp        │ portal.mi.com                         IN    │
│          │ ├─ 小米通讯         │ source/customer evidence/last recon          │
│          │ ├─ mi.com          │ [Run baseline recon] [Collect JS] [Match]    │
│          │ └─ portal.mi.com   │                                              │
│          │                    │ Identity | Surface | Sitemap | JS/API | ...  │
│          │                    │                                              │
│          │                    │ ports/services + JS/API + evidence trail     │
└──────────┴────────────────────┴──────────────────────────────────────────────┘
```

The mock intentionally keeps the operational SaaS feel: dense, scan-friendly, no hero layout, no decorative cards, and controls close to the data they affect.

## 5. Data Model Expectations

The UI should reuse existing APIs first:

- `targetAssetsList(targetId)`
- `apiEndpointsList(targetId)`
- `fingerprintsList(targetId)`
- `jsAnalysisList(targetId)`
- `passiveScansList(targetId)`
- `targetTimeline(targetId)`
- `oplogListByTarget(targetId)`
- existing target `ports`, `real_ip`, `cdn_waf`, `os_info`, `source`, `scope`

If new APIs are needed later, they should aggregate rather than duplicate:

- `target_surface_summary(target_id)`
- `target_recon_run_start(target_id, profile)`
- `target_recon_run_status(run_id)`

The first UI pass can compose existing calls client-side.

## 6. Stage Actions

Avoid a generic "Scan" command. Use staged actions:

- Organization:
  - `Import targets`
  - `Discover subsidiaries`
  - `Enrich profile`
  - `Review candidates`
- Target:
  - `Run baseline recon`
  - `Probe services`
  - `Collect JS`
  - `Extract APIs`
  - `Check sensitive exposure`
  - `Match vuln templates`

Every action must produce evidence or show why it did not run.

## 7. Non-Goals

- Do not bring back ZAP.
- Do not recreate the deleted SecurityView shell.
- Do not add exploit/active validation flows in this UI pass.
- Do not change DB schema in the first pass unless a specific missing field blocks the work.
- Do not treat LLM prose as completion evidence.

## 8. First Implementation Slice

The first code slice should be frontend-heavy:

1. Fix stale empty-state and discovery copy.
2. Split target selection state from inline target row expansion.
3. Add a right-side `TargetSurfaceWorkbench` component.
4. Move the existing `TargetDetailView` data fetching into a reusable hook.
5. Render `Identity`, `Surface`, `JS / API`, and `Evidence` tabs from current data.
6. Leave `Sitemap` and `Sensitive` tabs as honest empty/loading states if no backend data is available yet.

This gives users a coherent next step without requiring a risky backend rewrite.

