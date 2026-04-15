# Scan Workflow Implementation

Full-stack implementation of the PoC-first penetration testing workflow: WhatWeb fingerprinting, targeted Nuclei scanning, and feroxbuster directory brute-forcing.

## Architecture Overview

```
Target URL
  │
  ├─ 1. WhatWeb → Fingerprints (DB: fingerprints table)
  │
  ├─ 2. PoC Matcher → Matches fingerprints against vuln_kb_pocs
  │     └─ Extracts Nuclei template IDs from matched PoCs
  │
  ├─ 3. Nuclei Targeted → Runs ONLY matched templates
  │     └─ Stores confirmed vulns (DB: findings + passive_scan_logs)
  │
  └─ 4. feroxbuster → Recursive directory scan
        ├─ Uses ZAP-discovered paths as seed URLs
        └─ Detects sensitive files (DB: directory_entries + findings)
```

## Backend Implementation

### New File: `backend/crates/golish/src/tools/scan_runner.rs`

Core module containing all scan tool integrations (~856 lines).

#### Shared Types

| Type | Purpose |
|------|---------|
| `ScanProgress` | Real-time progress events emitted via `app.emit("scan-progress", ...)` |
| `ScanResult` | Unified result type for all scan tools (items found/stored, errors, duration) |
| `PocMatch` | A matched PoC from the fingerprint→PoC matching engine |

#### 1. WhatWeb Scanner (`scan_whatweb`)

- **Tauri command**: `scan_whatweb(target_url, target_id, project_path?)`
- **What it does**: Runs WhatWeb with JSON output, parses results, stores fingerprints
- **Key functions**:
  - `parse_whatweb_and_store()` — Parses WhatWeb JSON array, extracts technology entries
  - `infer_whatweb_category()` — Maps plugin names to categories (CMS, Framework, Server, Language, etc.)
  - `extract_whatweb_version_confidence()` — Extracts version strings and confidence percentages
- **DB writes**: `fingerprints::upsert()` for each detected technology
- **Events**: Emits `scan-progress` during execution and parsing

#### 2. Fingerprint → PoC Matching Engine (`match_pocs_for_target`)

- **Tauri command**: `match_pocs_for_target(target_id)`
- **What it does**: Queries fingerprints for a target, then searches `vuln_kb_pocs` for matching entries
- **Matching logic**:
  - Builds search terms from fingerprint name + version combinations
  - Uses PostgreSQL `ILIKE` for fuzzy matching across `poc_name`, `description`, and `tags`
  - Extracts Nuclei template IDs from PoC sources (e.g., `nuclei:CVE-2021-44228` → `CVE-2021-44228`)
  - Ranks results by severity (critical > high > medium > low > info)
- **Key function**: `build_search_terms()` — Generates multiple search variations per fingerprint
- **Returns**: `Vec<PocMatch>` with matched PoC details and optional Nuclei template IDs

#### 3. Nuclei Targeted Scan (`scan_nuclei_targeted`)

- **Tauri command**: `scan_nuclei_targeted(target_url, target_id, template_ids, project_path?, severity_filter?)`
- **What it does**: Runs Nuclei with only the matched template IDs (not a full scan)
- **Arguments**:
  - `-template-id <ids>` — Comma-separated template IDs from PoC matching
  - `-jsonl` — JSON Lines output for streaming parse
  - `-severity <filter>` — Optional severity filter
- **Output parsing**: Parses JSONL, extracts CVE IDs from template names and tags
- **DB writes**:
  - `findings` table — Confirmed vulnerabilities with full metadata
  - `passive_scan_logs` — Detailed scan log entries for each finding
- **Key structs**: `NucleiJsonResult`, `NucleiInfo` — Serde models for Nuclei JSONL output
- **CVE extraction**: `extract_cve_from_template()` and `extract_cve_from_tags()` attempt multiple patterns

#### 4. feroxbuster Directory Scanner (`scan_feroxbuster`)

- **Tauri command**: `scan_feroxbuster(target_url, target_id, base_paths, project_path?, options?)`
- **What it does**: Runs feroxbuster recursively on ZAP-discovered paths
- **Options** (`FeroxScanOptions`):
  - `depth` — Recursion depth (default: 3)
  - `threads` — Concurrent threads (default: 50)
  - `wordlist` — Custom wordlist path
  - `extensions` — File extensions to scan
  - `status_codes` — HTTP status codes to match
  - `timeout` — Request timeout in seconds (default: 10)
- **Base paths logic**: If `base_paths` is empty, scans target root; otherwise prepends target URL to each path
- **Output parsing**: Parses JSON output, extracts URL/status/content info
- **Sensitive path detection**:
  - `is_sensitive_path()` — Pattern matching for admin panels, config files, backup files, version control, etc.
  - `classify_sensitive_severity()` — Maps sensitive path types to severity levels
  - Auto-creates findings for sensitive paths
- **DB writes**:
  - `directory_entries` — All discovered paths
  - `findings` — Sensitive path discoveries

#### 5. ZAP Path Discovery (`get_zap_discovered_paths`)

- **Tauri command**: `get_zap_discovered_paths(target_host)`
- **What it does**: Extracts unique API paths from ZAP's topology_scans data
- **SQL**: Queries `topology_scans` for ZAP sitemap entries matching the target host
- **Returns**: Deduplicated path list (e.g., `/api/v1/users`, `/admin/login`)
- **Usage**: These paths become the seed URLs for feroxbuster

### Modified Files

| File | Changes |
|------|---------|
| `backend/Cargo.toml` | Added `v5` feature to `uuid` dependency |
| `backend/crates/golish/src/tools/mod.rs` | Added `pub mod scan_runner;` |
| `backend/crates/golish/src/lib.rs` | Registered 5 new Tauri commands |

### Registered Tauri Commands

```rust
tools::scan_runner::scan_whatweb,
tools::scan_runner::match_pocs_for_target,
tools::scan_runner::scan_nuclei_targeted,
tools::scan_runner::scan_feroxbuster,
tools::scan_runner::get_zap_discovered_paths,
```

## Frontend Implementation

### New File: `frontend/lib/pentest/scan-runner.ts`

TypeScript API wrappers for all scan runner Tauri commands.

| Function | Parameters | Returns |
|----------|-----------|---------|
| `scanWhatWeb` | `targetUrl, targetId, projectPath?` | `ToolScanResult` |
| `matchPocsForTarget` | `targetId` | `PocMatch[]` |
| `scanNucleiTargeted` | `targetUrl, targetId, templateIds, projectPath?, severityFilter?` | `ToolScanResult` |
| `scanFeroxbuster` | `targetUrl, targetId, basePaths, projectPath?, options?` | `ToolScanResult` |
| `getZapDiscoveredPaths` | `targetHost` | `string[]` |

### New Component: `frontend/components/ScanPanel/ScanPanel.tsx`

Unified scan workflow UI embedded in the TargetDetailView.

#### Features

1. **Collapsible panel** — Expandable "Scan Workflow" section in target detail
2. **Full workflow button** — "Run Full Workflow" executes all 4 steps sequentially
3. **Individual step controls** — Each step can be run independently with play/retry buttons
4. **Real-time progress** — Listens to `scan-progress` events for progress bar updates
5. **Step status dots** — Mini indicators (WW, PoC, Nu, Fx) in the header show overall status
6. **PoC results** — Grouped by severity with color-coded badges (critical/high/medium/low/info)
7. **Directory results** — Expandable list of discovered paths with status codes and content sizes
8. **Fingerprint display** — Shows detected technologies with confidence bars
9. **ZAP path count** — Displays number of ZAP-discovered base paths for feroxbuster

#### Component Props

```typescript
interface ScanPanelProps {
  targetId: string;   // UUID of the target
  targetUrl: string;  // URL/value of the target to scan
}
```

#### Integration Point

The ScanPanel is rendered in `TargetDetailView` (in `TargetPanel.tsx`) for targets of type `url` or `domain`, positioned between the fingerprints section and the ports section.

### Modified Files

| File | Changes |
|------|---------|
| `frontend/lib/pentest/index.ts` | Added `export * from "./scan-runner"` |
| `frontend/components/TargetPanel/TargetPanel.tsx` | Imported and rendered `ScanPanel` component |

## Workflow Logic

### Step-by-Step Flow

```
1. WhatWeb runs on target URL
   → Fingerprints stored in DB (category, name, version, confidence)
   → UI shows detected technologies

2. PoC Matcher queries DB
   → Reads fingerprints for this target
   → Searches vuln_kb_pocs for matching entries
   → Extracts Nuclei template IDs from matches
   → UI shows matched PoCs grouped by severity

3. Nuclei runs with ONLY matched templates
   → Not a full scan — only templates from step 2
   → Confirmed vulns stored as findings
   → UI shows confirmed vulnerability count

4. feroxbuster runs recursively
   → Queries ZAP topology_scans for discovered paths
   → Uses those paths as seed URLs (not blind root scan)
   → Detects sensitive files/directories
   → UI shows discovered path tree with status codes
```

### Full Workflow vs Individual Steps

- **Full Workflow**: Runs steps 1→2→3→4 sequentially, passing data between steps
- **Individual**: Each step can be triggered independently (useful for re-running after adding more PoCs or ZAP data)

## Event System

| Event | Payload | Source |
|-------|---------|--------|
| `scan-progress` | `{ tool, phase, current, total, message }` | All scan commands |

Phases per tool:
- **whatweb**: `running` → `parsing` → `done`
- **nuclei**: `preparing` → `scanning` → `storing` → `done`
- **feroxbuster**: `scanning` (per URL) → `done`

## Database Tables Used

| Table | Used By | Purpose |
|-------|---------|---------|
| `fingerprints` | WhatWeb | Store detected technologies |
| `vuln_kb_pocs` | PoC Matcher | Query for matching PoCs |
| `findings` | Nuclei, feroxbuster | Store confirmed vulnerabilities |
| `passive_scan_logs` | Nuclei | Detailed scan log entries |
| `directory_entries` | feroxbuster | Store discovered paths |
| `topology_scans` | ZAP paths | Query ZAP sitemap for seed URLs |

## External Tool Requirements

| Tool | Installation |
|------|-------------|
| WhatWeb | `brew install whatweb` or `gem install whatweb` |
| Nuclei | `brew install nuclei` or `go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest` |
| feroxbuster | `brew install feroxbuster` or `cargo install feroxbuster` |

The backend uses `which` to locate each tool at runtime. If a tool is not found, a descriptive error is returned with installation instructions.
