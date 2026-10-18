# P0 guarded execution protocol — C3 bounded lifecycle correction

The source remains exactly the executed C2 source. Its90-second Tokio timeout is cooperative. It does not impose a hard wall-clock bound on synchronous shared server initialization or Testcontainers destruction. No such intrinsic source guarantee is claimed.

Every final P0 acceptance invocation must execute the complete cargo test process under GNU timeout, without --foreground or --preserve-status:

```sh
gtimeout --signal=TERM --kill-after=5s 180s cargo test --locked -p sagat-tests --test integration_workflow_primitives api_reopen:: -- --nocapture
```

The timeout supervisor exists before Rust startup and remains until the cargo/test process exits, covering synchronous setup and teardown as well as all async operations. TERM is followed by KILL after5s if required. Exit0 and exactly2passing/0ignored tests are mandatory;124/125/126/127/137 or any other nonzero status fails the gate. No timeout is accepted as a successful P0 result. The non-foreground mode includes ordinary cargo/test children; this fixture creates no detached process/session. Docker/provider lifecycle remains external infrastructure, and an unresponsive daemon may require operator cleanup after a killed test; no cleanup-success guarantee is inferred from forced termination.

Root uses the already-owned external PostgreSQL specified through SAGAT_TEST_DATABASE_URL for these acceptance runs, so no Testcontainers PostgreSQL removal occurs in the actual profile. A retained held server spans same-database reconstruction. This does not change source, shared infrastructure, grants, migrations or production. It corrects the execution contract and replaces the earlier unsupported claim of an intrinsic90s whole-scenario bound. The inner90s timeout and30s supplied session settings remain useful cooperative/database guards.

Run the same selection in separate default and explicit SAGAT_TEST_AS_RUNTIME_ROLE=1 invocations, retain exact command/exit/name/count/source seals, and require the same supplied-PID, payload, session-hash, canonical receipt/replay and unarmed cleanup oracles. Both actual API pools are sagat_app; these invocations are not an owner/runtime privilege matrix. Existing C2 default2/2 is preliminary positive evidence only until the corrected guarded protocol passes. No scoped refusal, rollback, actual COMMIT loss, OS restart or release claim follows.
