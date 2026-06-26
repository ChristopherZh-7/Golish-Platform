# Rule-assisted JS analysis

Use this skill after `browser_collect_js_api` and `js_extract_apis` have run.

## Workflow

1. Treat `rule_matches` as the first-pass regex hit list. These rules are a
   Golish-curated set inspired by proven JS/content signal rules.
2. Do not classify from the matched string alone. For every high-impact or
   uncertain hit, use `source_file` plus `ai_analysis.candidate_files[].line_hints`
   or `rule_matches[].line` to read only a small local range with `read_file`.
3. Classify each reviewed hit as:
   - `real`: likely production secret/API/config/internal surface
   - `test`: placeholder, mock, sample, docs, generated fixture
   - `noise`: library code, comments, unrelated static string
   - `needs_followup`: context is insufficient or requires authenticated review
4. Never turn enumeration hits into findings directly. In enumeration, store
   classification in analysis notes/raw analysis and pass real candidates to the
   next triage stage.
5. Never claim an endpoint is verified from AI inference alone. A verified JS/API
   endpoint must be persisted by deterministic collection/extraction as
   `source=js_analysis` or `source=crawler`.

## Review Priorities

Review in this order:

1. `kind=secret` with confidence >= 0.75
2. `kind=config` internal IP/JDBC/API base/runtime config
3. auth/session storage hits (`Authorization Header`, `User Identity Storage`)
4. route/link hits only when endpoint extraction was empty or low-confidence

## Output Shape

When summarizing for the main agent, use compact structured notes:

```json
{
  "js_candidate_review": [
    {
      "source_file": "static/js/app.js",
      "line": 42,
      "rule": "Authorization Header",
      "classification": "real|test|noise|needs_followup",
      "reason": "short context-based explanation",
      "next_stage": "vuln_triage|ignore|manual_review"
    }
  ]
}
```
