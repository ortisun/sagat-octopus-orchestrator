# Five-SDK conformance at `38d7b3f0`

The immutable checkout passed all 64 registered scenarios in all five languages, without blessing
or language/scenario selection. Six scenarios are authoring-only. The other 58 produce 290
complete normalized observations (including publication-only checks), independently compared
by the root agent to their exact committed goldens. Canonical authoring parity is also checked
by the harness; build-only output is not persisted as normalized runtime output.

`command.json` and `suite.txt` are the actual final command/result. `manifest.json` records the
failed initial prerequisite, successful focused database check, toolchain/reuse limitations and
fixed executable hashes. `comparisons.json` records each independent normalized comparison/hash.
The source checkout was still tracked-clean at completion. No expected file changed.

The first full attempt failed because database-runner had not been built. The declared CI
prerequisite build and focused database-connections case then passed. The final full invocation
reused all five SDKs prepared unchanged by the initial attempt. Both the Rust test executable
and database-runner were copied before execution so concurrent builds could not replace them.

For a standalone reproduction, create a detached checkout at `38d7b3f0`, use a private
CARGO_TARGET_DIR and the repository-owned Docker profile, and run without concurrent builds:

```sh
cargo build -p sagat-database-runner --bin database-runner --locked
cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

Unset SAGAT_SDK_SCENARIO, SAGAT_SDK_LANGS, SAGAT_SDK_BLESS, SAGAT_SDK_SOURCE_ROOT,
SAGAT_TEST_DATABASE_URL and SAGAT_SDK_REUSE for that fresh preparation. The accepted invocation
instead used SAGAT_SDK_REUSE=1 after its recorded preparation and SAGAT_DATABASE_RUNNER_BIN
pointing to the fixed private copy; its exact command ran the fixed test executable directly.
Do not expose retained runner config/token files. Temporary checkout/runtime artifacts are
removed after comparison; the normalized expected data remains in the pinned source.

This is not acceptance for later event-candidate/pure-command commits, a native scoped event
receiver, nested capacity, actual released-profile rollout, production ceilings or release 0.3.
