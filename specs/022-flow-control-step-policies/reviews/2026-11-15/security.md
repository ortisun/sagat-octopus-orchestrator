# Review: feature 022, 0.3.0 slice foundation — security

- **Change:** `feature/022-throughput-control`, commits `4d044013..b5d071b3` (the foundation: declarations, events, codec, gates, migrations, keyed digests, refusals, telemetry, activation command)
- **Reviewer:** `security-reviewer` (independent read-only agent, given the changed files and the spec, not the author's conclusions), 2026-11-15
- **Recorded by:** the author, from the reviewer's report; every finding below is the reviewer's, the answer column is the author's.

| Finding | Severity | Answer |
|---|---|---|
| The runtime role could set `holders`, `waiters`, `live_grants`, `grants_total` (and `max_period_ms`) when it inserts a semaphore or window row (`GRANT INSERT` covered every column), making a key look full forever | Major | Fixed: column-limited `INSERT` grants without the counters; `max_period_ms` defaults and is kept by the trigger. Test `the_runtime_role_cannot_set_a_counter_when_it_inserts_a_key` (red first: the insert succeeded) |
| A continuation successor checked only the run-level gate; a stored definition with a node-level gate and `continueAsNew` would run its step ungated | Minor | Fixed: `refuse_gated_successor` uses `declares_gates()`. The successor test gained a node-level tamper case (red first: the hand-off advanced) |
| The key parser accepted `+` signs (`u8::from_str_radix`); key material not zeroized | Nit | Fixed the parser (every character a hex digit), test case added (red under mutation). Zeroizing the material is left: `ring::hmac::Key` holds its own copy |
| A `RETIRED` key version could be made active again | Nit | Fixed: `flow_control_key_activate` answers `RETIRED` and changes nothing. Test `a_key_row_is_written_only_under_the_active_version_and_a_retired_one_stays_retired` |
| (Suspicion) a deciding transaction that read version 1 before a rotation could file a key under it after the rotation, splitting one key into two rows | Minor (latent) | Fixed before any writer exists: `flow_control_active_key_version()` reads the active version `FOR SHARE` (the rotation takes the table exclusively), and a `BEFORE INSERT` trigger refuses a key row under any version but the active one. Same test as above (red first: the stale insert succeeded) |

Checked with no finding (reviewer's words, summarised): tenant boundary of every FK and of the HMAC input; `SECURITY DEFINER` functions pin `search_path`, qualify names, are not executable by `PUBLIC`; the runtime role cannot write key versions or requirements; no key value reaches the journal, a metric, a log or an error; the only `INSERT INTO runs` sites refuse a gated definition; no injection.

Counts: B0 M1 m2 N2 — all addressed in this change.
