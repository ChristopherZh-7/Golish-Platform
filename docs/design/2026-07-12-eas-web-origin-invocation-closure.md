# EAS Web-Origin Invocation Closure

Status: Implemented with focused verification; fresh compiled EAS acceptance remains pending.

## Problem

The live run `pentest-chat-1783829178322-1` returned authoritative
`details.missing_origins` such as `https://113.240.117.106:443`, but the Prober
later reconstructed the URL as `http://113.240.117.106:443`. The
`eas_fingerprint_web_stack` launch guard correctly rejected that request before
WhatWeb started because exact Web Origin identity includes scheme, host, and
effective port.

The same run exposed three independent seams:

1. `stage_worklist_next` dropped the coverage cell's `details`, so repair mode
   no longer carried the authoritative origins already computed from DB truth.
2. The Prober prompt only required an absolute HTTP(S) URL and did not require
   copying `details.missing_origins` unchanged.
3. A caller typo of `http://IP:443` aborted the whole batch even when the DB had
   exactly one authorized `https://IP:443` origin and the remaining entries were
   valid.

This is separate from the service-fingerprint 300-second timeout closure.

## Decision

Keep the generic exact-origin resolver strict. Browser, JS, route, liveness,
Enumeration preflight, and other producers must never have their scheme or port
silently changed.

Apply a narrow, wrapper-local reconciliation only for
`eas_fingerprint_web_stack`:

- Exact authorized origin always wins unchanged.
- Only a caller-supplied `http://<IP>:<port>` may be reconciled.
- Reconciliation may select only a DB-derived, confirmed, authorized
  `https://<same IP>:<same effective port>` origin.
- The selected origin must be unique after current workspace, current
  organization, and `scope=in` filtering (or unique within a valid explicit
  `target_id`).
- The origin must come from `confirmed_target_web_origins`; numeric port 443 is
  never itself evidence for HTTPS.
- The corrected HTTPS origin is passed back through the existing strict launch
  authorization before spawn. WhatWeb input, launch witness, structured
  landing, evidence, and technique outcome all use that same effective origin.
- No HTTPS-to-HTTP downgrade, host change, port change, DNS alias adoption,
  cross-org/workspace fallback, or ambiguous selection is allowed.
- Reconciliation is returned as audit metadata; it is not hidden.

The data-flow defense is also tightened:

- EAS WEB coverage exposes `recommended_args.target_urls` built from
  `{target_id, details.missing_origins[]}`.
- `stage_worklist_next` preserves `details` and `recommended_args`.
- Prober instructions require copying those arguments unchanged and prohibit
  rebuilding a scheme from a port number.
- Coverage-gap repair accepts the already-supported
  `{target_id,target_url}` objects instead of rejecting them before the wrapper.

## Security invariants

- Authorization remains fail-closed before network launch.
- A missing, out-of-scope, cross-org, cross-workspace, wrong-host, wrong-port,
  non-HTTP, or ambiguous origin still produces zero network requests.
- A DB authorization lookup failure never becomes a correction opportunity.
- Effective-origin duplicates created by reconciliation still reject the whole
  batch before launch.
- A target ownership/origin drift between reconciliation and spawn is caught by
  the existing strict revalidation and guarded landing path.
- Valid sibling inputs are retained when another sibling only needs the allowed
  deterministic HTTP-to-HTTPS correction; genuinely invalid siblings still
  fail the batch.

## Verification

Use TDD regressions for:

- unique same-IP/same-port HTTP-to-HTTPS reconciliation;
- exact HTTP/HTTPS inputs remaining unchanged;
- missing confirmation, wrong host, wrong port, wrong org/workspace/scope, and
  ambiguous owners remaining denied;
- WhatWeb execution inputs and active authorization both using the effective
  HTTPS origin while retaining valid siblings;
- EAS worklist preservation of exact origin details/recommended arguments;
- repair-mode acceptance of `{target_id,target_url}` entries; and
- Prober prompt language requiring exact argument copying.

No schema, migration, generated IPC type, or raw scanner surface changes are
required.
