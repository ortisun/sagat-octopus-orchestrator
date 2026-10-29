# Genuine released reconciliation Applied report, 2026-10-27

With the pinned toolchain, Docker reachable and an owned absolute external build target:

```sh
CARGO_TARGET_DIR=<owned-private-target> \
  python3 scripts/test-released-v02-profile.py --scenario applied-lookup
```

Final runner passes119.27s; old/current builds53.90s/55.25s; runtime command7.63s. Coordinator
1/0/0 passes6.37s, containing actual old report phase1/0/0 in2.38s and post-final old reader
1/0/0 in0.21s. Nested phases are not added into an aggregate. Both are the same copied old
binary whose production source matches the immutable actual v0.2.0 tag. Exact source, mode,
coordinator, overlay and executable identities are in build.json/current-source.json.

The old worker genuinely reports OUTCOME_UNKNOWN, then leases charge~r and reports/replays
Applied against current services across ordinary migration. The current proof checks original
forward uncertainty/fence/provider key, live lookup capability/expiry, both exact outcomes,
lookup completion without payload, forward Applied payload/output, one exact fingerprinted
receipt and one logical forward-output charge before scheduler aggregation. Two current
scheduler ticks finish the run. A fresh actual old process then verifies REST/native terminal
replay and the original immutable prefix. Capabilities never leave old process memory; only
framed anonymous IPC transports refreshed subject auth and non-capability metadata.

Separate retained invocations:

- First weaker baseline1/0/0 in14.52s (126.52s runner, old phase9.19s).
- A review-driven compile was intentionally interrupted(-2,29.36s) before runtime after root
  spotted an accidentally contradictory fixture predicate. It is no test pass or behavior RED.
- Corrected stronger preflight1/0/0 in6.19s (84.68s runner, old phase2.57s), before the final
  typed mode, exact ACK binding and post-final actual-old reader extension.
- FORWARD regression1/0/0 in6.55s (15.77s runner), including the new post-final old reader.
- Active-data regression1/0/0 in6.75s (11.23s runner), with separate old producer/reader phases.
- Strict integration Clippy passes19.57s; eight Python verifier tests pass separately.

Three bounded reviews resolve event semantics, output/fence/uncertainty/receipt/accounting,
mode and prefix-check timing gaps; final review finds none and performs no builds/tests/edits.
Transcript normalization removes whitespace only, with raw/normalized checksums recorded.
This is a released-source compatibility checkpoint, not a release asset or exactly-once
provider-effect guarantee. Quota-refusal/drain/original-forward compensation, expired reports,
mixed-dispatcher activation, five SDKs, nesting/capacity and complete release gates stay open.
