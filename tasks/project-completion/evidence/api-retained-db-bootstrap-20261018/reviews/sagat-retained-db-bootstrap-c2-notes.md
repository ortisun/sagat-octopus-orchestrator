# Retained database API bootstrap C2 draft notes

## Revision purpose

C2 preserves all C1 artifacts and replaces the boolean EXISTS witness that could miss an extra session with a different worker name or an extra binding. The test now captures exact ordered tenant, membership, binding and session identity/security rows before the first API is dropped, asserts one original scoped session, and compares the complete witness after every restart phase that could accidentally recreate or widen authority.

Repository read only: /Users/marcelo/Documents/Projects/sagat-octopus-project-completion

- HEAD observed while drafting: 21b2b79391a2e5ca0fdfe91e5fc1a42f96a371dc
- No repository file was edited.
- Root had not applied C1; tests/support/mod.rs, tests/support/api.rs and tests/integration/workflow-primitives/main.rs still matched the C1 source hashes.

## Read versions and hashes

| Input | SHA-256 |
|---|---|
| .specify/memory/constitution.md | e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e |
| specs/014-workflow-primitives/spec.md | eac73844ed31360ac03293474ac5d6213ab0db7bbfc585d5a3155fdf2d4490df |
| specs/014-workflow-primitives/plan.md | ffaa76a78278faaf575278c1425615f6a042dbb2412afb2ad2a690f3fff12930 |
| specs/014-workflow-primitives/tasks.md | c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e |
| /tmp/q10c-ordinary-inflight-report-fixture-plan.md | fb7fb20ce8bd183e31ab87db069073872dd03e07948acb47fe8cdabef1390b97 |
| tests/support/mod.rs | c0b11c1689f806756aca3bdf127e25f1f31ab921e7fb978b857a500371180a24 |
| tests/support/api.rs | f7a5460f59d7407e3d5cd2d416424402660a1e03239dc31d17b16c87fc6ee719 |
| tests/integration/workflow-primitives/main.rs | f2454ad111ef57b44244b27f3db8f9e0344bf7e3ae675300f4901a295516fc86 |

## C2 artifacts

1. /tmp/sagat-retained-db-bootstrap-red-tests-c2.patch
   - SHA-256: 75b1e97aba8fd577e6905feea3484da143c94dc0e473ac39f327229dc10a7978
   - Applies directly to the unchanged repository snapshot.
   - Adds only api_reopen.rs and its registration.
2. /tmp/sagat-retained-db-bootstrap-green-c2.patch
   - SHA-256: 6e3eff37c322be941b826965dddfcd4655dd17db035907e82f2d601b92f96e56
   - Applies after the C2 RED patch.
   - Adds attach_existing_db and start_on_db, then changes only the second API boot in the test.

Combined source scope remains exactly four files:

- tests/support/mod.rs
- tests/support/api.rs
- tests/integration/workflow-primitives/api_reopen.rs
- tests/integration/workflow-primitives/main.rs

## Exact safe witness

IdentityWitness contains only nonsecret durable identity/security fields:

- exact tenant ID selected with the original tenant ID and slug;
- exact membership rows for the original human and worker, including tenant ID, subject and role;
- every binding row for the original tenant and worker subject, including tenant ID, namespace ID, subject, pattern and updater;
- an exact count of every session for the original tenant and worker subject;
- every matching session ID and its tenant ID, namespace ID, subject, worker name, activities, languages, pools, max inflight and accepted protocol minor.

The original phase asserts:

- one exact tenant identity;
- admin role for alice and worker role for retained-db-public-worker;
- one exact wildcard binding in the original namespace;
- session count exactly one;
- the sole session ID equals the real RegisterWorker result;
- worker name wire-test-worker, activity act.a, language rust, no pools, max inflight 100 and negotiated minor 5.

Because bindings and sessions are fetched without narrowing to one expected pattern or worker name, an extra binding or registration appears in the ordered vectors and fails equality.

The witness deliberately excludes the session token, its stored hash, the lease token, created/updated timestamps and session/lease expiry. Assertion Debug output therefore contains no opaque credential. PollTasks may legitimately renew expires_at without invalidating the security comparison.

## RED behavior

The original real public sequence is unchanged: current API provisioning, public publish/start, first-phase worker membership and binding, gRPC registration 1.5, actual lease, and an exact CompleteStepRequest retained before shutdown. The worker borrow ends and the original TestApi is dropped before the second API starts. hold_server spans both API lifetimes.

The C2 RED second boot uses existing start_with. It creates a different migrated database, constructs Platform with the original tenant IDs, and immediately compares the complete durable witness before public namespace authentication or transport reconnect. The actual witness in that new database is empty while the expected witness has the original tenant, memberships, binding and sole session, so the test fails for the precise retained-database behavioral gap. It has no unresolved helper or compile-failure dependency.

## GREEN behavior

The GREEN helper and API extraction are unchanged from C1:

- attach_existing_db accepts only t_ plus 32 lowercase hexadecimal characters;
- it connects to the exact requested current_database;
- it creates no database, migration, role, grant, identity, status, payload or ledger state;
- it starts no fallback server and never prints the URL or credentials;
- start_with retains its existing provider then fresh migrated database behavior;
- start_on_db wires a caller-owned, already migrated TestDb through the same private API bootstrap.

The restarted current API compares the exact witness:

1. immediately after retained database bootstrap;
2. after Platform::connect and restoration of the original worker ID/session;
3. after PollTasks validates and renews the original stored session;
4. after submission of the exact pre-shutdown CompleteStepRequest.

The current phase never calls tenant, provision, grant, bind_everything, worker, worker_at, register or register_version. It uses a fresh current JWT, resolves the original namespace/run through HTTP, reuses the exact stored session through gRPC polling, and completes the original leased request. Both API instances are observed using sagat_app, while the inspection pools remain owner connected.

## Apply checks

No build, test, formatting, migration, commit, or repository edit was run.

Against the unchanged source snapshot:

    git apply --check /tmp/sagat-retained-db-bootstrap-red-tests-c2.patch
    exit 0

In a fresh disposable /tmp tree:

    git apply /tmp/sagat-retained-db-bootstrap-red-tests-c2.patch
    exit 0
    git apply --check /tmp/sagat-retained-db-bootstrap-green-c2.patch
    exit 0

Root should retain the actual behavioral RED output, then apply GREEN and execute:

    cargo test -p sagat-tests --test integration_workflow_primitives --locked api_reopen::current_api_reopens_owned_database_and_resumes_original_public_worker -- --exact

## Honest limits

- Compilation, runtime behavior, clippy and formatting remain unverified because execution was expressly prohibited.
- The expected RED mismatch follows directly from inspected start_with fresh database allocation; only root execution can record the actual failure.
- This remains a current-current API instance restart in one process with a retained PostgreSQL server. It makes no OS process restart, immutable-old/current, secret/blob key continuity or Q10 report/drain claim.
- It closes only the test harness dependency if its RED/GREEN execution passes. It does not close feature 014 collection tasks or any broader compatibility/fault matrix.
