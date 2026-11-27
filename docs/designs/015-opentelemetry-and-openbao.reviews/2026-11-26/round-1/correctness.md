# Correctness review of design 015, round 1 (2026-11-26)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: cbf7160c4e1c342d6f347d41976064548906f2f0
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 2, Major 14, Nit 8

1. Blocker — An older engine reads a `transit` row as a local row and skips the output scan, so a secret can reach the journal
   - **Where:** §5 (key id `transit`), §10 (lines 421-425: "a binary that meets a `transit` row it cannot open refuses with a typed error"), Risks row "A mixed fleet".
   - **What is wrong:** No binary built before this design can be changed to refuse with a typed error. Its `stored_from` (`crates/infra-postgres/src/secrets.rs:49-61`) turns any row with `provider_operation IS NULL` and a `key_id` into `StoredValue::Local`. The ring does not know `transit`, so opening fails. Two readers then treat that failure as a skip:
     - `secret_in_output` runs `Err(_) => continue` (`crates/application/src/worker/secrets.rs:240-246`).
     - Webhook signing does the same (`crates/scheduler/src/deliveries.rs:548-559`).
     Only the control plane runs migrations (`apps/control-plane/src/lib.rs:151`), so an old execution engine starts against the new schema without complaint. The registry gate is checked only when `migrating` starts, and an old binary started later is never checked by it.
   - **Why it matters:** During a `helm rollback` of the engine, or with one stale replica, a 0.3.0 replica delivers the secret. The worker echoes it, reports to the old replica, and the old replica writes the output to the journal unscanned. That is secret exposure, which is release-blocking under Principle V.
   - **Correction:**
     - Pick a representation that old code refuses: store `transit` rows with `key_id IS NULL` and `provider_operation IS NULL`. Old `stored_from` then returns `corrupt("local secret key")`, and the scan and the delivery fail closed.
     - Put the literal `transit` into the associated data as the custody tag; it does not need to come from the column.
     - State this as the compatibility mechanism.
     - Test it by running the 0.2.x `stored_from` and scan against a `transit` row.

2. Blocker — A rotation not yet captured by a snapshot can lose every secret; the backup rule has no enforcement, and standby regions are not covered
   - **Where:** Requirement 015-020 (lines 169-173), O015-006 (`auto_rotate_period` 90 days, line 257), CUJ 2, Success criterion 015-007, Delivery Constraints row of the Constitution Check.
   - **What is wrong:**
     - A new key version exists only in OpenBao until the next scheduled snapshot, and Transit seals under it at once. Example: an auto-rotation at 02:00, `secrets rewrap` at 10:00 (all 200,000 rows moved to version 4), the last snapshot at 00:00, and the OpenBao cluster lost at 12:00. No PostgreSQL backup taken after 02:00 can be paired. The only way to open the secrets is to restore PostgreSQL to before 02:00, which also loses every run, journal entry and other change made since.
     - "Trimming (`min_available_version`) is refused while a database backup older than the last rewrap is retained" has no mechanism. No platform command trims, and the platform cannot see which backups are retained.
     - The Delivery Constraints allow standby regions (ADR-0067). A rotation or rewrap that the standby's OpenBao does not hold makes every row sealed or rewrapped afterwards unopenable after a failover. The design does not mention regions.
   - **Correction:**
     - Record the sealing version in PostgreSQL (`key_authority_state.seal_version`) and pass `key_version` to `transit/datakey`. Advance it only by a command that first verifies a snapshot, and for a cell with standby regions a standby, holding that key version.
     - Make `secrets rewrap` refuse to target a version that no snapshot holds.
     - Drop `auto_rotate_period`, or keep it only with that gate.
     - Turn the trimming rule into a runbook rule, or into a platform command that is the only holder of the `min_available_version` capability.
     - Extend Success criterion 015-007 so that rotation, rewrap and retire happen between the backup and the snapshot, and add a test that an unpaired restore is detected, not silently half-open.

3. Major — `retire-key-versions` relies on a 60-second sleep, and the platform's own timeouts allow a longer seal
   - **Where:** Requirement 015-012 (lines 129-131), §6 (lines 377-378), Success criterion 015-002, CUJ 2.
   - **What is wrong:** "Longer than the longest seal" is false. A seal is a Transit `datakey` call (5 s budget), then a pool acquire (3 s, `pool.rs:61`), then `put_version`, which issues about 8 statements, each up to 5 s (`statement_timeout`, `pool.rs:62`). PostgreSQL also allows up to 60 s idle between any two statements (`idle_in_transaction_timeout`, `pool.rs:63`). The gap between the `datakey` reply and `BEGIN` has no bound at all: a starved Tokio task or a paused VM can stretch it. A version-3 row that commits at 61 s lands after `min_decryption_version=4` and will not open. The count, the wait, the recount and the raise are check-then-act, and a test cannot hold a seal open "for longer than 60 s" except by sleeping.
   - **Correction:** Fence in the database, not with time.
     - Retire first raises `key_authority_state.min_seal_version` in a transaction that takes the row `FOR UPDATE`.
     - Every `transit` insert reads that row `FOR SHARE` in its own transaction, and a trigger refuses `wrapped_key_version < min_seal_version`. The put is then retried with a fresh data key.
     - Count after the fence commits, and only then raise Transit's minimum.
     - Test it with a barrier: hold one seal between `datakey` and commit, run retire, release the seal, and assert that the late commit is refused.

4. Major — The exact allow-list forbids operations the design needs, and the port is missing them
   - **Where:** Requirement 015-016 (lines 146-154), §7 table, §4 `KeyAuthority`, §6, Requirement 015-013.
   - **What is wrong:**
     - Retire must write `min_decryption_version`, which needs `update` on `sagat-transit/keys/sagat-secrets/config`. The policy test must "refuse … any `keys/*/config`", so no token can run the command.
     - `secrets rewrap` needs "the newest" version, and `/script-runner/signing-key` must list each version's public key and the minimum. Both need `read` on `sagat-transit/keys/<name>`, which is not in the allow-list.
     - The port has `bind_intent` but no way to verify a binding (the policy grants `verify`). It also has no `public_keys`, `key_info` or `set_min_decryption_version`.
   - **Correction:**
     - Add `read` on `keys/sagat-secrets`, `keys/sagat-scripts` and `keys/sagat-intent` to the control plane's list.
     - Run retire under a separate operator role whose policy grants `update` on `keys/sagat-secrets/config` with `allowed_parameters = {min_decryption_version = []}`. Write that carve-out into the test.
     - Add `verify_intent`, `public_keys`, `key_info` and `set_min_decryption_version` to the port.

5. Major — Re-keying after a compromise has no selector and no guard, and the procedure never revokes the stolen token
   - **Where:** Requirement 015-012 (lines 127-129), §6 (lines 379-380), Risks row "A stolen engine token", Constitution Check I ("guarded by the old version").
   - **What is wrong:**
     - The design does not say which rows `rekey` takes, how it resumes, or what predicate guards its update. If it selects rows below the newest version, every row that `rewrap` has already moved to the newest version is skipped, even though it still carries a compromised data key. Example: the 90-day auto-rotation plus the operator's routine rewrap, or a rewrap run in parallel with the rekey. `retire` then passes.
     - "Rotate, rekey, retire" leaves the stolen token valid. Rotation does not invalidate a token, and the engine policy has `decrypt`, so the attacker unwraps the new data keys as well.
   - **Correction:**
     - Add `data_key_issued_at timestamptz` (or a re-key epoch), set by seal and rekey.
     - `rekey --issued-before <cutoff>` selects on that column with an `id` cursor and updates `WHERE id = $1 AND wrapped_key = $old_wrapped AND nonce = $old_nonce`.
     - Change the runbook order to: revoke the token's accessor and the role's issued tokens (rotate the Kubernetes service account), then rotate, rekey and retire.
     - Test that a row rewrapped before the rekey is still re-keyed.

6. Major — Rewrap, re-key and re-seal make concurrent deliveries and step reports fail, contradicting CUJ 2 and Success criterion 015-004
   - **Where:** CUJ 2 ("no delivery failed"), Success criterion 015-004 ("zero failed resolutions"), §6.
   - **What is wrong:** Delivery opens the values outside a transaction, then re-reads and refuses on any difference: `if current != inputs { return Err(AppError::Unavailable) }` (`worker/secrets.rs:200`). The report path does the same: `before != tx.secret_versions(..)` (`worker/secrets.rs:289-291`). `StoredSecret` equality covers the stored representation (nonce, ciphertext, and with this design `wrapped_key`). Rewrap, re-seal and rekey change those bytes without changing the value. With a secret resolved every 10 ms, a re-seal of its row that commits inside the window between the two reads returns `Unavailable`. That window is lengthened by the cold Transit unwrap the re-seal itself causes. The test is flaky or fails, and in production such deliveries fail.
   - **Correction:** Compare the logical identity, the version row id and the version number (and the custody, if needed), never the sealed bytes. Or state that the criteria count retried resolutions. Add a barrier test: hold a delivery between the read and the recheck, commit a rewrap, and expect success.

7. Major — Version-2 intent bindings break a legitimate replay after any rotation of `sagat-intent`
   - **Where:** Requirement 015-014 (lines 138-141), §5 (lines 366-368), Task 015-16.
   - **What is wrong:** A replay recomputes the binding before the store call (`managed_secrets.rs:269`). The store then compares bytes: `old.binding != intent.binding → IdempotencyKeyReused` (`infra-postgres/src/managed_secrets.rs:127-129`). Transit HMAC is computed at the newest key version. A client that retries within the 24-hour receipt window across an auto-rotation, or across the switch from version 1 to version 2, is refused as a reused key, which reports a client mistake for a correct retry. Recomputing inside the store would put a Transit call inside a transaction that holds `lock_secrets`. Separately, the immutability trigger lists fixed columns (`20261122000100_openbao_secrets.up.sql:29-30`), so the new `binding_version` and `binding_key_version` would be mutable.
   - **Correction:**
     - On a replay, read the stored row first, outside the lock.
     - Verify with the stored version: the file key for version 1, `transit/verify` or `hmac` with `key_version` for version 2. Then re-enter the transaction.
     - Fix `algorithm=sha2-256`, so the stored binding stays 32 bytes.
     - Extend `sagat_secret_provider_immutable` to the two new columns.
     - Test a replay across a rotation.

8. Major — The script-signing key has no migration and no re-sign, so switching to `transit` or retiring a version fails every stored script for good
   - **Where:** Requirements 015-010 and 015-013, Requirement 015-015 (which covers secret rows only), Task 015-15.
   - **What is wrong:**
     - Stored artifacts carry a signature by `SAGAT_SCRIPT_SIGNING_KEY` and its key id. Under `transit` that variable is refused at start, and the endpoint lists only Transit versions. The runner then fails every existing script step with `Failure::Untrusted`, permanently (`apps/sandbox-runner/src/lib.rs:870-874`).
     - Retiring a `sagat-scripts` version does the same to every artifact signed under it.
     - On a rotation Transit signs with the new version at once. A runner that refreshed on an unknown version within the last minute (the rate limit) refuses the new signatures for up to 60 s, and those steps also fail for good.
   - **Correction:**
     - Public keys are not secrets: let the control plane keep serving the legacy local public key until a `scripts resign` pass has re-signed every artifact. The pass works per artifact and is guarded by the old key id.
     - Make that pass a precondition of retire and of leaving `migrating`.
     - Sign with `key_version` set to a version that was published at least one refresh period (more than 5 minutes) earlier.
     - Make a refusal for an unknown key retryable (`Unavailable`), not `Untrusted`.

9. Major — The runner's artifact cache keeps running artifacts signed under a retired version
   - **Where:** Requirement 015-13 ("refuse a retired version without a restart"), Success criterion 015-006, Task 015-15.
   - **What is wrong:** `load` returns a cached `Loaded`, keyed by digest only, without verifying it again (`sandbox-runner/src/lib.rs:726-729`). Entries are cleared only when the cache reaches 128 (`lib.rs:756-759`), and `Loaded` does not record which key verified it. After a retire and a trust refresh, an artifact that was already cached keeps running. The planned test, a fresh artifact after the retire, passes while the defect remains.
   - **Correction:**
     - Store the `key_id` in `Loaded`, and on each refresh evict the entries whose key is no longer trusted. Alternatively, check the key id against the current verifier on every cache hit.
     - Test: run digest D, retire, run D again, expect a refusal.
     - Also drive the cross-tenant case of Success criterion 015-006 end to end through the runner. `load` verifies against the artifact record's tenant and caches by digest; whether that is enough could not be confirmed from this code.

10. Major — Ending the migration is check-then-act, and its precondition is incomplete
    - **Where:** Requirement 015-015 (lines 142-145), §6 (lines 381-383), Requirement 015-010, §10.
    - **What is wrong:**
      - The mode is a per-process environment variable. Any process still in `local` mode keeps sealing local rows after `reseal-secrets --to transit` has reported zero: a pod not yet restarted, or one rolled back. Switching to `transit` then removes the ring, and those rows no longer open.
      - Nothing stops the switch either way.
      - Leaving `migrating` also requires that no version-1 intent is pending (the file key is refused under `transit`) and that no artifact is signed by the local key (finding 8). Neither condition is stated as part of the switch.
    - **Correction:**
      - Hold the mode in PostgreSQL (`key_authority_state.mode`).
      - Seals read it `FOR SHARE`, and a trigger refuses local inserts once the mode is `migrating` or later.
      - A command `key-authority finish-migration` sets `transit` in one transaction that checks zero `local` rows, zero pending version-1 intents and zero locally signed artifacts.
      - Processes refuse to start when their configuration disagrees with the stored mode.

11. Major — Custody is recorded per secret, but every transition works per version row
    - **Where:** Terms (lines 41-43), §5 (lines 359-365), §6 (re-seal "one row per transaction"), O015-004.
    - **What is wrong:**
      - `secrets.authority` is a per-secret column (`20261122000100_openbao_secrets.up.sql:2`). The representation constraint is on `secret_versions` and cannot read it.
      - During a re-seal, one secret holds `local` and `transit` versions together. The design does not say when `secrets.authority` changes, or which column reading routes by.
      - `put_version` inserts `secrets` without an authority (the default is `local`, `infra-postgres/src/secrets.rs:394`), so a new `transit` secret would show as `provider: local` once "derived from `secrets.authority`".
      - `prepare_provider_intent` refuses any authority other than `openbao` unless the source is `local` (`managed_secrets.rs:146-160`, `103`), and `migrate` needs the ring. A `transit` secret can therefore never move to KV custody.
    - **Correction:**
      - Route opening by the row's columns (`wrapped_key IS NOT NULL`).
      - Write the constraint on columns only: `(wrapped_key IS NULL) = (wrapped_key_version IS NULL)`, with the custody tag consistent.
      - Define `secrets.authority` as the custody of new versions: it changes when the first `transit` version commits, and `put_version` sets it.
      - Decide whether `transit` to KV migration is offered. If it is, add the source `transit` to `legacy_provider_source`.

12. Major — Only one error is defined, so a permanent refusal is retried for ever and transient failures are reported as permanent elsewhere
    - **Where:** Requirements 015-017 and 015-018, Success criterion 015-003, §4 ("every reader that matches on it (delivery, webhook signing, the output scan, cleanup)").
    - **What is wrong:**
      - A row that can never open, whether moved across tenants, below `min_decryption_version`, or corrupt, makes the scan "refuse the report as retryable (`KEY_AUTHORITY_UNAVAILABLE`)". Every report of the step is then refused for ever, and the run is stuck without a terminal signal.
      - In the other direction, the connector credential reader maps every failure to `CredentialsError::Unusable` (`crates/infra-connectors/src/credentials.rs:120-123`). That reader is missing from the design's list, so an OpenBao outage would be reported as wrong credentials.
      - Delivery maps a failure to open to `AppError::Internal` (`secrets.rs:86`), which is a 500.
      - A 403 from a token rotated by the Agent is not classified.
    - **Correction:**
      - Define the `KeyAuthorityError` variants: `Unavailable` (retryable), `Refused` (integrity, not retryable, audited, alerted), and `Denied`.
      - Map each variant on every reader, including the connector credentials and the broker credentials in `deliveries.rs:586-596`.
      - For a scan that meets `Refused`, hold the report with a typed, non-retryable error that operators see.
      - Test each reader under the outage.

13. Major — The cache sizing and the outage claims ignore eviction, the multi-version scan and the refresh period
    - **Where:** §8, O015-007, CUJ 3, Success criterion 015-005, Task 015-13.
    - **What is wrong:**
      - The scan opens every kept version: `KEPT_VERSIONS = 5` (`infra-postgres/src/secrets.rs:27`, `worker/secrets.rs:239`). With 16 secrets per activity, one step touches 16 × 5 = 80 entries.
      - A tenant whose scanned secrets have 103 × 5 = 515 versions exceeds its 512-entry share, so LRU evicts current versions that deliveries need. CUJ 3's 500 hot secrets in one tenant already take 500 of the 512 slots.
      - "Unwrapped less than 300 seconds before succeed" ignores eviction.
      - The refresh period T is not stated. The guaranteed survival is 60 − T seconds, not 60 s.
      - A test of a 300-second TTL and a 240-second refresh needs an injected clock, and none is planned.
    - **Correction:**
      - Do not promote scan-only opens of non-current versions in the LRU, or size the per-tenant share for them.
      - Fix T (for example 10 s, giving at least 50 s) and state the guarantee as "for entries still cached".
      - Drive the cache from the platform's `Clock`, and test the 239/241 and 299/301 second boundaries.

14. Major — With no reader configured the output scan is skipped, and under `transit` that reader depends on different settings
    - **Where:** Requirement 015-017 ("never skips the scan"), §4, Task 015-10.
    - **What is wrong:** `checked_secret_report` returns `Ready(tx, None)` when `self.vault.is_none()` (`worker/secrets.rs:275-277`). `secret_in_output` does the same (`233-235`). Today that reader exists only when the ring is configured (`apps/execution-engine/src/settings.rs:141-143`). Under `transit` there is no ring. One replica started without the Transit settings would accept the reports of steps that declare secrets without scanning them, while other replicas deliver those secrets.
    - **Correction:**
      - A step that declares secrets and reaches a replica with no reader must have its report refused as retryable, never accepted.
      - A process in `transit` or `migrating` mode refuses to start without its key authority.
      - Test it with a replica that has no reader configured.

15. Major — Rewrap is audited once per run, outside the transaction of the change, and the text contradicts itself
    - **Where:** Requirement 015-012 ("auditing every row it could not rewrap") against §6 ("one record per run"), Constitution Check VI.
    - **What is wrong:** A crash after N batches leaves N × 250 rows changed with no audit record, while Principle VI asks for evidence written in the same transaction as the decision. "Resumable from its cursor" names no durable cursor. A whole-call Transit outage returns `Err` for every item, and that would be counted and audited as per-row refusals.
    - **Correction:**
      - Write an audit record per batch, with the counts and the ids of refused rows, in the transaction that updates the batch.
      - Persist the cursor, or say that a restart begins again from the start (which is idempotent).
      - Count an outage of the whole call separately from per-item refusals.
      - Parse per-item `batch_results` whatever the HTTP status.

16. Major — Under `transit`, a secret is opened on paths the plan neither lists nor tests
    - **Where:** §4 reader list; Tasks 015-10 and 015-19.
    - **What is wrong:**
      - Webhook signing with several versions (`deliveries.rs:547-559`) is named in §4, but no test covers it during an outage. Signing with only the cached older version sends a signature that a receiver which finished rotating rejects.
      - The broker credentials (`deliveries.rs:596`, `.ok()`) and the connector supervisor (`infra-connectors`) are not named at all.
    - **Correction:** List every `SecretReader::resolve` caller in §4. Add outage tests for webhook signing (no partial signature: the delivery waits) and for broker and connector credentials.

17. Nit — Stale registers and ADR text describe the design the security review found broken
    - Research D015-005 and D015-006 (lines 28-29) still say `key_id` `bao-<version>` and "select by `key_id` … update with the old `key_id` in the predicate", with nothing marking them superseded. D015-011 does not say it replaces them.
    - ADR-0152's Consequences say "under `openbao`" and "`openbao` is gated", and its Evidence says "the bounded client reused", which contradicts §4's per-mount client.
    - Correction: mark D015-005 and D015-006 "superseded by D015-011", and fix ADR-0152's wording.

18. Nit — Redaction shapes cannot be matched as specified
    - `scrub_words` splits on `:` (`redact.rs:62-63`), so `vault:v3:<base64>` reaches the matcher as three words. A prefix check never sees `vault:v`; it needs a two-word lookback like `follows_scheme`.
    - A bare `s.` prefix would scrub ordinary text. Bound it, for example `s\.[A-Za-z0-9]{24}`, `hvs\.` with at least 90 characters.
    - The Collector's `attributes` filter deletes numeric attributes such as `payload_bytes` and `http.request.body.size`, which the in-process rule keeps. The drift test compares names only, not this behaviour.

19. Nit — The telemetry configuration does not match the code it extends
    - `from_lookup` returns `None` when no traces endpoint is set (`otlp.rs:918-927`), so a logs-only configuration exports nothing.
    - `export_log` sends one record per request (`otlp.rs:486-492`), so batches of 512 need a batch encoder.
    - The CLI already exports a gauge over OTLP (`apps/cli/src/telemetry.rs:254`) and has no admin port, so Requirement 015-001's "expose its metrics for the Collector to scrape" cannot apply to it.
    - The drop counter is `otel_spans_total{outcome="dropped"}` today, not `telemetry_dropped_total{signal}`.

20. Nit — `secret_provider_requests_total` gets two label vocabularies
    - Feature 017 emits `outcome` values `ok`, `denied`, `conflict`, `failed` (`openbao.rs:191-197`). §9 adds `refused`, `unavailable`, `timeout` for `provider="transit"`.
    - Alerts and dashboards on one family then need per-provider queries.
    - Unify the set.

21. Nit — The token cache is keyed by time, not by token
    - "Caches its token's `lookup-self` until the token's time to live" (Requirement 015-018). When the Agent writes a new token, for example after re-authenticating, every call fails until the cached time to live ends.
    - Key the cache by a digest of the file's content, read the file on each call, and do the lookup again after a 403.

22. Nit — The integer version can drift from the wrapped key, and the rewrap guard is weaker than it should be
    - Add `CHECK (wrapped_key_version = substring(wrapped_key from '^vault:v([0-9]+):')::int)`.
    - Guard rewrap with `wrapped_key = $old`, not with the version only.
    - The migration should refuse to run when an existing ring key is named `transit`; `KeyRing::parse` refusing it later would stop the upgraded processes at start.

23. Nit — Deployment details missing or contradictory
    - The chart has no sandbox-runner template (`infra/helm/templates/`), yet §7 and Task 015-7 give it an Agent.
    - The Collector credential lives on feature 017's KV mount, against "each custody MUST have its own mount".
    - The secret cleanup job is granted `decrypt`, but it opens no secret (`cleanup.hcl`: "No credential read"; `cleanup.rs` resolves nothing).
    - The Collector scrape needs NetworkPolicy ingress to the admin ports, not only egress.
    - The bearer-token rotation of `bearertokenauth` needs an overlap window; the design states none.

24. Nit — Small inaccuracies
    - Line 62: the associated data binds the version row id, not the version (`domain/src/secrets.rs:192-211`).
    - `script_signing::message` is private (`script_signing.rs:33`).
    - Task 015-25 refers to Requirement 015-011 for core dumps.
    - "Usable through an outage of up to 60 seconds after its last refresh" should read "at least 60 − T seconds after a refresh is due" (see finding 13).

I could not run anything. Each "confirmed" point was checked against the code at the lines cited, in . Three points are plausible but not proven:
- Whether an older engine is actually running next to new ones (finding 1). The skip itself is confirmed.
- Whether the artifact API scopes artifacts by tenant (finding 9).
- Transit's HTTP status for a partial batch failure (finding 15).
