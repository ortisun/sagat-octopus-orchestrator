# crates/infra-sandbox/shims

## Overview

Container-resident TypeScript/Python build/runtime shims embedded by the Rust sandbox and connected through JSON-line frames.

## Interface

- TypeScript build strips source types, checks syntax and emits main.mjs; Python build compiles source and requires a top-level run. [build.mjs:reject] [build.mjs:say] [build.py:say] [build.py:defines_run]
- Runtime APIs provide fetch, secret, log and idempotency keys through frozen `octopus` or `_Octopus`; Python accepts synchronous/awaitable results. [runtime.mjs:octopus] [runtime.mjs:call] [runtime.mjs:start] [runtime.py:_Octopus] [runtime.py:_Response] [runtime.py:_call] [runtime.py:_main]

## Invariants

- Runtime filenames are plain 1–64-character names and entry paths are fixed main.mjs/main.py. TypeScript requires default/run export and Python a top-level run definition. [runtime.mjs:start] [runtime.py:_main] [build.mjs:exported] [build.py:defines_run]
- Fetch/secret operations use supervisor channel requests; acceptance and provider deduplication are outside the shim. Passing an idempotency key is not an exactly-once guarantee. [runtime.mjs:octopus] [runtime.py:_Octopus]

## Side Effects

- TypeScript build writes /tmp/main.mjs and spawns node --check; Python compiles in process. Runtimes write supplied files under /tmp/script, load/execute user entry points, write stdout protocol/stderr logs and terminate. [build.mjs:writeFileSync] [build.mjs:spawnSync] [build.py:compile] [runtime.mjs:start] [runtime.py:_main] [runtime.py:_write]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-sandbox --locked
```

These commands were not run in this documentation-only pass.
