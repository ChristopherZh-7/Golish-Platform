# Internal Discovery methodology (C6 / P6b)

## Contract

Internal Discovery consumes only an exact active `foothold/<uuid>` from the same
operation, stable project scope, and organization-at-time. Each observation has
a frozen asset identity hash, typed observation object, observation hash, time,
and exact evidence IDs.

## P6b capability boundary

`post_exploit_record_internal_observation` is the only stage wrapper. It records
bounded typed facts under an exact live worker/tool-call fence and exposes no
raw network discovery, port scanner, tunnel, shell, or lateral-movement CLI.
Asset and observation hashes plus row IDs are server-derived.

## Canonical result

`internal_asset_observations` is the authority. Empty means explicitly checked
empty only when backed by a canonical terminal row/evidence contract; missing
rows mean not checked. Free-form host lists do not close the stage.
