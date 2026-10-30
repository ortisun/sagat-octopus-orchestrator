# inline-script

A script node runs a TypeScript (or Python) function in the platform's sandbox. The document pins the script by the
digest of the artifact the platform built, so first build the script, then paste its digest.

```sh
sagat script test enrich.ts --input '{"orderId":"o-1","items":2}'      # run it in the sandbox
sagat script publish enrich.ts                                         # build it: prints artifactDigest sha256:...
# put that digest in workflow.json ("artifactDigest"), then:
sagat workflow validate workflow.json
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
sagat workflow run enrich-order --input '{"orderId":"o-1","items":2}' --wait
```

Until the digest is replaced, `sagat workflow publish` stops with `UNKNOWN_SCRIPT`: the platform has no script with
the placeholder digest. The sandbox needs a runner (`docs/runbooks/scripts.md`).

<!-- EGM directory contract -->

## Interface

- `enrich` is the default TypeScript export, accepting `orderId` and `items` and returning those fields with `express`. [enrich.ts:enrich]
- The example workflow starts at script node `enrich`, then calls activity `orders.store`; `mocks.yaml` supplies its development echo response. [workflow.json:entrypoint] [workflow.json:orders.store] [mocks.yaml:orders.store]

## Invariants

- `express` is true exactly when `items <= 3`; the function copies `orderId` and `items` unchanged. [enrich.ts:enrich]
- The all-zero `artifactDigest` must be replaced with a published artifact digest before publication succeeds. Sandbox setup is described in the [script runbook](../../docs/runbooks/scripts.md). [workflow.json:artifactDigest]

## Side Effects

- None in `enrich`; it computes and returns an object. Publishing/running the surrounding workflow invokes the platform and worker through the documented CLI steps. [enrich.ts:enrich] [workflow.json:nodes]

## Verification

Run from the repository root:

```sh
cargo test -p sagat-tests --test integration_inline_code_sandbox --locked
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
