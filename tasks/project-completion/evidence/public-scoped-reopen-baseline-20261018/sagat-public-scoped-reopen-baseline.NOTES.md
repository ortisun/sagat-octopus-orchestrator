# Public scoped reopen baseline draft

- Source repository: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`
- Pinned source commit: `f81a1285a4d26bb1897ef4f178bafbbb65c5ad5d`
- Shared worktree HEAD after sealing: `8b8046e53dc6deb1925d3fb058469cdd5e0506b7`; the owned `main.rs` blob is unchanged from the pinned source (`0cd3b30dadf91ee81634d75ee334da5b77adcbe0`).
- Accepted prerequisite baseline referenced by the assignment: `c7f62b4a`
- Patch: `/tmp/sagat-public-scoped-reopen-baseline.patch`
- Sealed mirrors: `/tmp/sagat-public-scoped-reopen-baseline-draft/tests/integration/workflow-primitives/`
- Exact new test: `public_refusal_commit_loss::ordinary_public_scoped_refusal_reopens_on_supplied_runtime_pool`

## Owned source paths

1. New `tests/integration/workflow-primitives/public_refusal_commit_loss.rs`
2. Existing `tests/integration/workflow-primitives/main.rs`, one preserved module registration

No repository file was edited. The pre-existing legacy fixture working diff and all evidence directories were left untouched.

## Review units

- Lines 1–310: imports, exact workflow/output builders, durable witnesses, full snapshot, canonical scoped receipt scope and completion fingerprint.
- Lines 311–520: public 17-member setup, real Keeper/Placement scheduling, original leased request retention, authority/source premise, clean Keeper departure, bounded old-pool close, same-database supplied-pool reopen.
- Lines 521–748: first public refusal assertions, full receipt/audit/head checks, exact public replay, fresh direct PgRunStore journal/path reads, bounded pool cleanup and unarmed proxy abort guard.
- `main.rs`: one-line module registration.

## What the draft proves if the intended gates pass

The public API creates a concurrency-one 17-member collection with `reserve`/`release` compensation and literal-one Return. Sixteen public 250,000-byte outputs are genuinely completed and checkpointed. The seventeenth reserve is genuinely leased, and the exact 250,000-byte alternate completion command is retained privately.

The setup scheduler owns shards through a real Keeper/Placement and leaves them cleanly. After setup API references and pools close, the test attaches the exact owned database without migration or provisioning, starts the existing relay unarmed, and supplies its exact max-one `sagat_app` pool to `start_on_db_with_pool`. It reconnects the original worker/session and does not register or poll again.

The first public CompleteStep must return newly applied `SUCCEEDED/CANCELLING`. Durable checks require one `FAILED/RUN_DATA_LIMIT` stop before one scoped success, unchanged 4,000,000-byte logical/existing output premises, OPEN scope and held member, the original reported fence, no candidate payload/event payload/blob, no Return/End/checkpoint/saga, unchanged source/authority/version state, zero report outbox growth, and one fixed truthful refusal audit.

The receipt oracle independently derives the current production lease-qualified scope and the SHA-256 length-prefixed `complete + attempt(u32 BE) + generation(u64 BE) + exact output` fingerprint. It retains tenant, scope, key, request hash, resource, and created-at. Exact public replay must return the original ACK and leave the whole stored execution/payload/quota/audit/outbox/source/authority snapshot and receipt unchanged. Fresh direct PgRunStore calls verify the full journal and exact scoped path with fixed non-disclosing failures.

## Static validation performed

- `git apply --check /tmp/sagat-public-scoped-reopen-baseline.patch`: passed after the shared worktree advanced; the two owned baselines remain identical to pinned source `f81a1285`.
- No build, test, formatter, SQL session, container, commit, or repository write was run, per assignment.
- The runtime session expiry is deliberately omitted from byte-equality snapshots because every authenticated worker command renews it. The snapshot includes the durable session identity, subject, namespace, capabilities, protocol, creation time, and exact token hash; explicit checks separately preserve the original task lease row before reporting.

## Intended root gates

Before building, run `df -h`, select a private `CARGO_TARGET_DIR` outside the repository, and keep at most two heavy builds active.

1. Apply the patch and inspect the two-path diff.
2. Run `cargo fmt --check` and strict integration-target Clippy.
3. Under the accepted GNU process guard (`TERM` at 180 seconds, `KILL` 5 seconds later, without a foreground wrapper), run:
   - `cargo test -p sagat-tests --test integration_workflow_primitives --locked public_refusal_commit_loss::ordinary_public_scoped_refusal_reopens_on_supplied_runtime_pool -- --exact --nocapture`
   - the same exact selection with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
4. Re-run the accepted supplied-pool reopen prerequisite:
   - `api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen`
   - `api_reopen::current_api_reopens_owned_database_and_resumes_original_public_worker`
5. Run the relevant existing ordinary refusal characterization:
   - `byte_ceilings::a_known_successful_seventeenth_member_is_preserved_without_its_refused_output`
6. Run `python3 scripts/check-architecture.py` and `python3 scripts/check-agent-config.py`.

## Boundary and limitations

This is P0b characterization only. The relay is never armed and `proxy.close()` is never called. It does not inject reply loss, select a COMMIT, exercise deferred abort, claim an unknown outcome, test Applied reconciliation, run post-ACK scheduler discovery/drain/compensation, simulate OS death, exercise an old producer, or close any P1 acceptance item. The 90-second Tokio timeout is cooperative; hard process containment remains the root GNU timeout gate.

