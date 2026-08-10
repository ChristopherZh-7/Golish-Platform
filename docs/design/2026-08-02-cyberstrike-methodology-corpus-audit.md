# CyberStrike Methodology Corpus Audit

> Audit snapshot: `CyberStrikeus/CyberStrike`
> commit `80ee899a4ccb2a152fb505e7ce9e1a7874b1f486` (2026-08-01).
> This is a methodology and supply-chain audit, not authorization to vendor or
> execute the upstream corpus.

## Executive decision

Golish should support the whole corpus as a searchable methodology library, but
must not treat all files as equivalent attack recipes and must never execute raw
Markdown commands or payloads.

The corpus is useful after compilation into separate contracts:

1. methodology metadata and applicability;
2. hypothesis templates;
3. typed verification recipes;
4. deterministic evidence/oracle requirements;
5. safety and authorization policy;
6. report-only remediation and references.

Raw upstream content remains quarantined and is never itself Finding authority.

## Inventory

The upstream Git tree contains exactly 7,656 uppercase `SKILL.md` files. It also
contains 27 lowercase `skill.md` Ubuntu CIS files. Case-insensitive macOS scans
can therefore appear to find 7,683 files, while the documented case-sensitive
loader pattern only addresses the 7,656 uppercase files.

| Corpus | Files | Share | Correct Golish role |
| --- | ---: | ---: | --- |
| CIS Benchmarks | 5,000 | 65.31% | Configuration/compliance audit |
| NIST | 1,606 | 20.98% | Governance and reporting context |
| MITRE ATT&CK | 898 | 11.73% | Technique taxonomy, applicability and chain expansion |
| OWASP WSTG 4.2 | 125 | 1.63% | Web hypothesis and test intent |
| Custom top-level | 27 | 0.35% | Offensive, recon and post-exploitation methods |

CIS plus NIST is 6,606 files, or 86.29% of the advertised corpus. The count is
real, but it is not a count of homogeneous vulnerability-verification playbooks.

The custom set contains 16 web attack methods, seven post-exploitation method
documents, recon/AD/Kerberos documents, and one unrelated `bun-file-io` skill.

## Content usability

### Custom offensive methods

The 16 `attack-*` methods are closest to Golish's desired contract. They contain
explicit `What Constitutes a Finding` and `Evidence Requirements` sections.
They are still prose and raw commands; they require manual compilation into
typed tools, baseline/probe/control/reproduction phases and deterministic
oracles.

### OWASP WSTG

Of 125 files, 120 contain both `What to Check` and `How to Test`; 120 contain a
checklist, 99 contain tool guidance and 47 contain example commands or payloads.
They are strong hypothesis/test-intent sources but do not define a standard
Finding oracle or evidence contract. HTTP status, body length or keyword-only
examples must not be promoted into Golish oracles without a paired control and
reproduction rule.

### MITRE ATT&CK

All 898 files provide technique/tactic/platform metadata; 305 contain code and
307 identify Atomic Red Team tests. The corpus is useful for applicability,
attack-path expansion and detection context. A technique being applicable is
not evidence that a vulnerability exists. Atomic tests require individual
safety and cleanup review before they can become executable recipes.

### CIS

CIS contributes 3,784 `Audit Procedure`, 1,209 `Audit`, 1,849 `Expected Result`
and 4,991 `Remediation` sections. Read-only audit procedures can become
configuration-verification recipes. Remediation must be physically separated:
it contains configuration changes, resource changes and deletion commands that
must never be available to a vulnerability verifier.

### NIST

NIST content is structurally complete but highly templated. SP800-53's 1,196
files share the same test code; the 130 SP800-171 and 61 SSDF files do likewise.
This is useful as governance/reporting knowledge, not asset-level exploit or
vulnerability evidence.

## Schema and catalog quality

- All 7,656 YAML frontmatters parse and all `name` values are globally unique.
- Only 7,641 contain a `description` field; 11 are null and 15 are missing, so
  only 7,630 have a usable required description.
- 220 `name` values do not match their directory name.
- 220 `severity_boost` values are strings even though the guide requires a map.
- 1,458 versions are not strict three-part semver.
- `index.json` covers only 129 entries (1.685%) and omits 7,527 files. It cannot
  be the source of truth for discovery.
- Relationship fields mix namespaces. Resolving only by `name` makes 88.3% of
  `chains_with` and 97.4% of `prerequisites` look dangling. A framework-scoped
  alias resolver using name, parent slug, technique id and control id resolves
  all 7,908 chain references and 1,679 of 1,681 prerequisite references. The two
  remaining references are a spelling mismatch.
- Median skill size is 3,426 bytes/96 lines; total source is about 29.3 MB.
  Metadata-first retrieval plus bounded lazy loading is required.

## Signing claim audit

The README says the corpus is Ed25519-signed. At the audited commit, none of the
7,656 `SKILL.md` frontmatters contains `sha256`, `signature`, or `signed_by`, and
there are no companion signature/public-key files. `index.json` labels 129
entries `verified: official`, but that label is not a cryptographic result.

The current CyberStrike runtime explicitly admits unsigned skills:

- missing `sha256` becomes `unverified`;
- only `tampered` is rejected during discovery;
- `unverified` and `community` skills remain loadable and can be injected into
  an agent prompt;
- the CLI verify summary counts those states as passed.

The supplied bulk signer scans only one directory level, so it can address 27
top-level files while 7,629 skills are nested. Its signature covers the text of
one `SKILL.md`, not directory identity, registry metadata, auxiliary scripts or
release context. The remote registry/index and archives are not signed as a
package.

Therefore Golish must not consume `author: cyberstrike-official` or
`verified: official` as trust authority. If upstream packages are ever allowed,
Golish needs its own signed manifest, trust store and verification record.

## License boundary

The repository declares `AGPL-3.0-only`, and the skill tree does not provide a
uniform independent per-skill license/SPDX manifest. CIS, NIST and MITRE-derived
material also requires upstream content-rights review. Golish may learn from
the abstract document shape, but must not copy or vendor prose, payloads, code or
scripts without an explicit legal/source-provenance review.

## Golish methodology model

Every accepted source is normalized into one of these method families:

- `offensive_test`
- `adversary_technique`
- `compliance_audit`
- `governance_control`
- `recon_workflow`

Every executable derivative also receives one execution class:

- `passive`
- `safe_active`
- `mutating`
- `credential_access`
- `defense_evasion`
- `destructive`
- `manual_review_only`

Upstream `category` is never used as an execution-stage or safety decision.

## Required ingestion layers

1. **RawQuarantine** — source, commit/path/URL, raw hash, license, observed
   signature state and retrieval time. Raw text never reaches an executor.
2. **CanonicalMetadata** — stable method id/version, framework-scoped ids,
   aliases, tags, platform/technology, CWE and relationship edges.
3. **ApplicabilityPredicate** — asset/service/application/identity/business
   signals and exact prerequisite evidence.
4. **HypothesisTemplate** — a falsifiable target claim, never a Finding.
5. **VerificationRecipe** — typed `baseline -> probe -> negative control ->
   compare -> reproduce -> cleanup` actions with limits and expected effects.
6. **EvidenceContract** — required raw request/response, identity context,
   comparison, provenance, receipts and ledger references.
7. **SafetyPolicy** — execution class, scope, authorization, budget, concurrency,
   callback/egress, secret access, cleanup and hard-stop rules.
8. **ReportOnly** — risk, remediation and references, inaccessible to the action
   executor.

## Admission order

| Priority | Material | Admission |
| --- | --- | --- |
| A | 16 custom offensive methods | Manually compile hypothesis, recipe and oracle |
| B | 120 actionable WSTG methods | Extract intent; independently write deterministic oracle |
| C | CIS | Read-only audit only; quarantine remediation |
| D | MITRE | Metadata/detection/chains first; review each Atomic test separately |
| E | NIST | Governance/report-only context |

The remaining content is still searchable, but it cannot increase action
authority. Post-exploitation, credential access, defense evasion, DoS, brute
force, write operations, external callbacks and all remediation commands are
quarantined by default.

## Supply-chain requirements

Any future method package accepted by Golish must provide a signed canonical
manifest containing schema version, method id/version, source, license, every
path/size/digest and a package root. The verifier must support `key_id`, multiple
trust roots, rotation grace, revocation and validity windows. Registry indexes
are signed; downloads are verified before safe extraction; path traversal,
symlinks, archive bombs and low-trust name shadowing are rejected. Evidence
records the exact package digest and verification status used by every
hypothesis and action.

## Consequence for Candidate and Verification

The corpus supports a unified Hypothesis Validation Loop, but does not collapse
authority boundaries. Methodology can propose a hypothesis and a recipe;
application evidence decides applicability; Prepared Action controls execution;
only a typed oracle over landed evidence can support or refute the hypothesis.
No skill, model confidence, risk table or checklist may create a Finding.
