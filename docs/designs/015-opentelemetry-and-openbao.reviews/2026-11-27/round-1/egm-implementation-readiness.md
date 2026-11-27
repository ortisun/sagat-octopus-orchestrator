# EGM stage 9 (implementation readiness, with codebase details) of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: fad9a21de6b0dd0c01ebf1fc64799433e49e5f8c
Reviewer: egm-implementation-readiness
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added. The EGM review of design 015 ran its lens agents with
the accepted EGM plugin (commit f0ba151b), each as an agent with no conversation history, spawned one at a time, as
for design 040: this stage 9 read first.

Counts: Blocker 3, Major 24, Nit 7

1. Blocker — The mode gate closes every serving process at `finish-migration` and makes `begin-return` impossible without downtime
   Where: docs/designs/015-opentelemetry-and-openbao.md:118-128 (Requirement 015-010), 199-208 (Requirement 015-015), 534-540 (the command table), 627-634 (Rollout); docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md:24-27
   What: Under `transit`, a process that has `SAGAT_SECRETS_KEYS` (or the script key, or the intent key file) set keeps its gate closed. In `migrating` and `returning`, a process without the ring keeps its gate closed. These settings are environment variables, so they only change with a restart. Every process that serves during `migrating` therefore has the ring, and its gate closes the moment `finish-migration` commits `transit`. The way back has the same problem. `begin-return` needs `key-authority-return-v1`, which a process lists only when it has both the ring and Transit. Under `transit`, though, a process with the ring is already gate-closed. So no serving process can meet the precondition. The doc never says when a process checks its configuration against the mode (only at registration, on each heartbeat, or on each read of `key_authority_state`), and it never says how a running process learns that the mode changed.
   Why: Requirement 015-015 and the Complexity Tracking table promise both passages "without downtime". As written, any implementation either has an outage at `finish-migration` and `begin-return`, or quietly drops the gate.
   Correction: Say when the check runs and how processes learn the mode (for example the registry heartbeat of ADR-0082 decision 4). Make the rules cover the passages. One option: a process that started with the ring stays open under `transit` until restarted, but reports a non-blocking warning and refuses to use the ring for new seals. The other: let the ring be present under `transit` for the hand-over window, and add a separate `key-authority-ring-absent-v1` requirement for the operator to record after the rollout. Then write down the exact rollout order for both passages and add a test for each.

2. Blocker — The runtime role cannot read `key_authority_state` `FOR SHARE` with `SELECT` only
   Where: docs/designs/015-opentelemetry-and-openbao.md:491-498 (Technical Plan 5); Task 015-12 (lines 761-771)
   What: The doc says seals and processes read the state row `FOR SHARE`, that `sagat_app` has `SELECT` only, and that the privilege audit proves `sagat_app` cannot write the table. In PostgreSQL, `SELECT ... FOR SHARE` (like every row-locking clause) also requires `UPDATE` privilege on at least one column. A seal running as `sagat_app` would fail with "permission denied".
   Why: The locking read the design relies on cannot be built as specified.
   Correction: Choose one approach and write it down. Either give `sagat_app` column-level `UPDATE` on one column (the `ENABLE ALWAYS` trigger still refuses every change), and state that the privilege audit checks the trigger rather than the grant. Or add a `SECURITY DEFINER` function, executable by `sagat_app`, that returns the row under `FOR SHARE`. Also say which transaction each seal takes this lock in, relative to its Transit `datakey` call.

3. Blocker — `scripts resign` must update a table that is immutable by trigger
   Where: docs/designs/015-opentelemetry-and-openbao.md:180-184, 536 (`scripts resign`, guarded by `WHERE key_id = $old`), 517-519; crates/infra-postgres/migrations/20261014000000_scripts.up.sql:150-164 (`script_artifacts_reject_mutation`, `BEFORE UPDATE OR DELETE`, `ENABLE ALWAYS`), 303 (`sagat_app` has `SELECT, INSERT` only), 87 and 141-148 (`script_versions.key_id`, frozen by `script_versions_immutable`)
   What: Re-signing replaces `signature` and `key_id` on existing `script_artifacts` rows. An `ENABLE ALWAYS` trigger refuses that update for every role, the schema owner included. `script_versions.key_id` also records the signing key and is frozen too. No task changes these triggers. Task 015-12 only adds "the signing-key trigger on script artifacts".
   Why: `scripts resign` cannot be built. `finish-migration` and `finish-return` both need it to report zero, so neither passage can complete. Relaxing artifact immutability is a security decision (the trust argument in `crates/application/src/script_signing.rs:1-11`) that the doc leaves to the builder.
   Correction: State the storage change in Technical Plan 5 and Task 015-12. One option: a new table `script_artifact_signatures(tenant_id, digest, key_id, key_version, signature, signed_at)` with insert-only rows, the runner using the newest trusted one, and the artifact bytes left immutable. The other: a narrowed immutability trigger that allows only `signature`/`key_id` to change, only under the schema-owning role, and only through an audited function. Say what happens to `script_versions.key_id`.

4. Major — The format, precedence and reload of `OTEL_EXPORTER_OTLP_HEADERS_FILE` are undefined
   Where: docs/designs/015-opentelemetry-and-openbao.md:102-111 (Requirement 015-006), 391-397, 584-585; Task 015-3 (722-725); Task 015-7 (735-737); crates/telemetry/src/otlp.rs:961-973 (per-signal `*_HEADERS` variables), 332-337 (headers fixed in `Arc<OtlpConfig>`)
   What: The doc never says what the file contains (`name=value` pairs as in `OTEL_EXPORTER_OTLP_HEADERS`, or a bare token turned into `Authorization: Bearer`). It never says how the file interacts with the per-signal `OTEL_EXPORTER_OTLP_{TRACES,LOGS,METRICS}_HEADERS` and the common variable when both are set, or how "when it changes" is detected (polling interval, modification time, content digest). `OtlpClient` builds its headers once at start, so reloading needs a mutable header holder, which no task names. The OpenBao Agent template (Task 015-7) and the database runner's Kubernetes Secret both have to write the file, so they depend on the format.
   Why: The Agent template, the Secret layout and the parser could each end up with a different format.
   Correction: Define the file format, the precedence order, the reload trigger (for example re-read when the content digest changes, checked at most every 30 seconds, and once after a `401`), and the size limit. Name the type that holds the reloadable headers in `crates/telemetry`.

5. Major — The logs-only configuration and the log record shape need type changes that no task names
   Where: docs/designs/015-opentelemetry-and-openbao.md:386-399 (Technical Plan 1); Task 015-2, Task 015-3; crates/telemetry/src/otlp.rs:238-265 (`endpoint: String` is required), 126-136 (`LogData` has non-optional `trace_id`/`span_id` and `error: bool`), 409-414 (severity only `ERROR`/`INFO`), 917-927 and 956-960 (logs endpoint only from the common endpoint), 662-673 (`Open` is private); apps/cli/src/telemetry.rs:74 (reads `config.endpoint`)
   What: A logs-only configuration needs `OtlpConfig.endpoint` to become optional. Log records "with no span ids" need optional ids in `LogData`. "The severity" and `SAGAT_OTEL_LOGS_LEVEL` need more than a boolean. `LogLayer` needs read access to `OtlpLayer`'s private `Open` extension to find the current span. The doc says none of this, and the CLI would break when `endpoint` changes type.
   Why: A builder has to redesign public types of `sagat-telemetry` that the CLI uses.
   Correction: List the type changes in Technical Plan 1 and Task 015-2/015-3: `endpoint: Option<String>` (or a separate `logs_only` form), `LogData.trace_id/span_id: Option<…>`, a severity enum mapped to OpenTelemetry severity numbers, and `Open` made crate-visible. Add the CLI to the files changed.

6. Major — The sources of `service.instance.id` and `deployment.environment.name` are undefined, and Technical Plan 3 contradicts Requirement 015-005
   Where: docs/designs/015-opentelemetry-and-openbao.md:99-101 (Requirement 015-005: "every log record and span carries"), 421 (the Collector's `resource` processor "adds `deployment.environment.name`"); Task 015-4 (726-728); crates/telemetry/src/otlp.rs:378-383 (the resource has only name and version)
   What: The doc does not say where a process gets its instance id (the instance registry id, the pod name, a random identifier at start) or its environment name (`SAGAT_ENV`, `OTEL_RESOURCE_ATTRIBUTES`, a chart value). If only the production Collector adds the environment, the development Collector does not, and Requirement 015-005 fails there. The sandbox runner and the database runner are not in the instance registry.
   Why: A builder would pick sources, and the resource-attribute test could not be written against a defined expectation.
   Correction: Name the source of each attribute for each of the five processes. Say whether `OTEL_RESOURCE_ATTRIBUTES` is read and how it interacts with the tenant/namespace ban. Either set the environment name in the process or drop "every record carries" in favour of the Collector adding it in both configurations.

7. Major — The production Collector configuration as described cannot pass `otelcol-contrib validate`, and the test location is wrong
   Where: docs/designs/015-opentelemetry-and-openbao.md:423-425 ("exporters left as commented examples"; "a test in `infra/observability/tests/`"), 90-92 (Requirement 015-003: the shipped configuration "MUST export them over OTLP"); Task 015-6 (732-734); .github/workflows/ci.yml:700-704 (the existing `validate` step); infra/observability/tests/*_test.yaml (promtool rule tests only)
   What: A Collector pipeline with no exporter fails validation, and Requirement 015-003 requires an OTLP exporter. `validate` today runs as a step of the continuous integration workflow, not as a test in `infra/observability/tests/`, which holds promtool rule files. The doc also does not say how the file is parameterized for the operator (endpoint, Kubernetes service discovery namespace and labels, certificate and token paths).
   Why: Task 015-6 cannot pass as written.
   Correction: Ship one active `otlphttp` exporter whose endpoint and headers come from `${env:…}` placeholders, keep the other exporters commented, and list the placeholders. Extend the existing workflow step to validate both files (or name the new test harness), with the placeholder environment it sets.

8. Major — The attribute filter "on string attributes only" cannot be built with the processor named, and it contradicts Requirement 015-004
   Where: docs/designs/015-opentelemetry-and-openbao.md:95-97 (Requirement 015-004: "deletes any attribute whose name matches"), 421-423 (Technical Plan 3: "an `attributes` processor … from string attributes only"); Task 015-5
   What: The `delete` action of the Collector's `attributes` processor matches on the key pattern, whatever the value type, so it cannot keep numeric values such as `payload_bytes`. The requirement and the plan also disagree on whether numeric attributes survive.
   Why: The builder must choose a different processor (for example `transform` with OpenTelemetry Transformation Language conditions), and the drift test has no defined target.
   Correction: Decide the behaviour (delete regardless of type, or redact only string values). Name the processor and the exact statement form the generator writes, and make Requirement 015-004 and Technical Plan 3 say the same thing.

9. Major — The development stack would double-count metrics and has no log backend, so Success criterion 015-001 cannot be checked
   Where: docs/designs/015-opentelemetry-and-openbao.md:410-413 (Prometheus "may still scrape directly"; alert rules unchanged), 417-418 (development logs and metrics go to the debug exporter and a Prometheus remote-write), 274-275 (Success criterion 015-001: log lines "found in the backend"); Task 015-8 (739-741); infra/docker/compose.observability.yaml:65-78 (Prometheus scrapes directly; no `--web.enable-remote-write-receiver`; no log backend); infra/observability/prometheus.yml:12-21
   What: If the Collector scrapes the processes and remote-writes to the same Prometheus that also scrapes them, every series arrives twice with different `job`/`instance` labels, and the `sum`-based alert rules count double. The Prometheus in compose does not accept remote writes. Logs only reach the debug exporter, so no backend exists in which to find a log line and follow it to its span.
   Why: Task 015-8 cannot show its success criterion, and the shipped development stack would fire wrong alerts.
   Correction: For compose, pick one metrics path (the Collector scrapes and Prometheus stops scraping, or the other way round), add the remote-write flag if it is used, and add a pinned log backend (for example Loki, with a Grafana data source) or Jaeger's log support. Name these compose changes in Task 015-6 or 015-8.

10. Major — The claim that only database connection forms send `requiredProvider: openbao` is wrong: the Studio sends it on every secret write
    Where: docs/designs/015-opentelemetry-and-openbao.md:136-144 (Requirement 015-011 and its table); frontend/apps/studio/src/features/secrets/api.ts:9 (every `put` sends `requiredProvider: 'openbao'`); frontend/apps/studio/src/features/secrets/SecretForm.tsx:30 (throws unless `provider === 'openbao'`)
    What: Under `migrating` and `transit`, every secret written from the Studio routes to KV, never to `transit` custody. If KV is not configured, the table has no cell for that case. The Studio would also reject a `transit` response. Task 015-26 only regenerates the Studio types.
    Why: The main user path would never use the new custody, and the table misses a case.
    Correction: Add a Studio task. The general Secrets form stops sending `requiredProvider` (or sends a mode-aware value) and accepts `provider: transit`, while the connection forms keep `openbao`. Add the table cell "`migrating`/`transit`, `requiredProvider: openbao`, no KV provider configured", with its status code.

11. Major — How the three error kinds map to errors on each surface is not specified
    Where: docs/designs/015-opentelemetry-and-openbao.md:223-233 (Requirement 015-017; "holds it with a typed, non-retryable error that operators see"), 830-832 (Task 015-26); crates/application/src/error.rs (no key-authority variant in `AppError`); crates/application/src/managed_secrets.rs:437-446 (today's provider mapping); sdks/error-catalog.json:13 (each entry needs `origin`, `http`, `grpc`, `retryable`, `remediation`)
    What: `SecretReader::resolve` returns `AppError`, but the doc defines no `AppError` variants for `Unavailable`, `Refused` and `Denied`. It gives no HTTP status, gRPC code or origin for `KEY_AUTHORITY_REFUSED`, and no code at all for `Denied`. It does not say what a worker receives over gRPC. "Holds" the report is undefined: is the report refused non-retryably, is the step failed, or is it parked for review?
    Why: Each of the eight readers named in Requirement 015-017 would map differently. The catalog entries and the conformance tests cannot be written.
    Correction: Add a mapping table: `KeyAuthorityError` kind, then `AppError` variant, then REST status and code, then gRPC status, then `retryable`, then audit and alert. Give the full catalog entries. Define what "holds" does to the attempt and the step.

12. Major — The Transit configuration, the operator token and the application wiring have no names and no task
    Where: docs/designs/015-opentelemetry-and-openbao.md:40 (`SAGAT_KEY_AUTHORITY`), 123-126 ("its Transit configuration"), 212-218 and 466-467 (the operator's token), 467-470 (one client per mount), 814-817 (Task 015-22: "the reference of the new variables")
    What: Apart from `SAGAT_KEY_AUTHORITY`, the doc never names the variables for the Transit mount, the token file for each custody, the CA, or the operator token used by the commands. Commands such as `advance-seal-version` and `trim-key-versions` need `read` on keys, which the operator policy lacks, so they need two tokens; the doc does not say which. No task wires `TransitEnvelopeKeys` and the signers into the control plane and the engine (`SecretService`, `SecretReader`, `WorkerService` vault, scheduler deliveries, events, connectors). Today these are built only from the ring.
    Why: The documentation-first rule needs the names before the code. A builder would invent them, and the chart, the compose files and the gate would drift apart.
    Correction: Add a configuration table (variable, process kinds, default, validation) for the authority mode, the Transit address, the mount, each token file, the operator token file and the cache bounds. Add a task for the wiring in `apps/control-plane` and `apps/execution-engine`, with its tests.

13. Major — The `secret-resolve-never-discards` rule cannot tell secret resolves apart from other `.resolve(` calls
    Where: docs/designs/015-opentelemetry-and-openbao.md:482-485; Task 015-10; crates/infra-connectors/src/guard.rs:64 (`self.resolve(...).await.is_err()`), crates/application/src/scripts.rs:965, crates/infra-lake/src/relay.rs:786, crates/infra-database/src/executor.rs:107
    What: `scripts/check-architecture.py` works on text. Many unrelated `.resolve(` calls exist, and some of them discard errors on purpose. The doc does not say how the rule finds a `SecretReader::resolve` call: by receiver name, by file list, by renaming the method to something unique, or by type information.
    Why: The rule either fires on unrelated code or misses real sites.
    Correction: Rename the method to a unique name (for example `resolve_secret`), or state the receiver names and files the rule scans. List the forbidden patterns exactly, `.is_err()` included.

14. Major — Limiting the output scan to the versions an attempt could receive, with one batch unwrap per report, has no task, and the store changes are not named
    Where: docs/designs/015-opentelemetry-and-openbao.md:594-599 (Technical Plan 8); crates/application/src/worker/secrets.rs:230-263 (`secret_in_output` opens every stored version one by one), 282; crates/infra-postgres/src/execution.rs:4713 and crates/infra-postgres/src/execution/path.rs:170 (`secret_versions` returns all versions)
    What: Filtering by "the version current when the attempt's lease began and every version created since" needs the lease start time and a new store query in both implementations of `secret_versions`. The batch unwrap needs a new `SecretReader` operation, since `resolve` takes one secret. No task (Task 015-10, 015-11 or 015-13) includes this, and no test checks it.
    Why: A requirement with no task or test. CUJ 3 and the one-round-trip promise depend on it.
    Correction: Add it to Task 015-10 or a new task. Name the lease-start source, the new store method, the `SecretReader` batch method, and the tests (a report with five secrets makes one Transit call; a version older than the lease start is not opened).

15. Major — The move from `transit` to `openbao` custody is promised but has no task, and the existing SQL refuses it
    Where: docs/designs/015-opentelemetry-and-openbao.md:523; crates/infra-postgres/src/managed_secrets.rs:103 (`legacy_provider_source` requires `s.authority='local'`); crates/application/src/managed_secrets.rs:149-152, 195-202 (the migration source opens through a ring-only `SecretReader::local`)
    What: Allowing `transit` as a source of feature 017's `migrate` needs the SQL predicate changed, a Transit-capable reader in `ManagedSecretService`, and tests. No task names them.
    Why: A requirement with no task.
    Correction: Add it to Task 015-27 (or a new task), with the code changes and a test that moves a `transit` secret to KV.

16. Major — Re-sealing does not say what happens to `secrets.authority`, which blocks the `down` migration and mislabels custody
    Where: docs/designs/015-opentelemetry-and-openbao.md:504-506 (`authority` set by `put_version`), 514-516 (`down` refuses while any `secrets.authority = 'transit'`), 537-540 (finish guards count rows only), 535 and 539 (`reseal-secrets --to …`)
    What: The re-seal passes replace envelopes through `replace_sealed`, not `put_version`, so `secrets.authority` keeps its old value. After a full return to `local`, every secret that was never rewritten still says `transit`. The `down` migration then refuses forever, and the "custody of the secret's new versions" is wrong.
    Why: A defect a builder is likely to build.
    Correction: State that each re-seal sets `secrets.authority` to the target custody in the same transaction, when every retained version of that secret is in the target custody. Add `secrets.authority` to the guards of `finish-migration` and `finish-return`.

17. Major — The "key-list form" of the signing-key request is undefined, and the runner gate misses 0.2.x runners
    Where: docs/designs/015-opentelemetry-and-openbao.md:184-189, 534; Task 015-15 (783-790); specs/001-platform-foundation/contracts/openapi.yaml:3999-4013 (a bare `GET` with no parameters); apps/sandbox-runner/src/api.rs:500; apps/sandbox-runner/src/lib.rs:776-796 (a 0.2.x runner fetches once, on first use)
    What: The doc does not say how a new runner marks its request as the key-list form (a query parameter, a header, an `Accept` value), or what "runner identity" is. Task 015-26 changes only the response. The gate looks back only 15 minutes, and the fetch table is new. A 0.2.x runner that fetched its key before the upgrade, or more than 15 minutes earlier, never fetches again and is never seen. The gate passes, and that runner then refuses every Transit signature.
    Why: Both the wire contract and the safety of `begin-migration` are left to guesswork.
    Correction: Define the request form in the contract (for example `GET /script-runner/signing-key?form=keys`) and the identity recorded (the principal subject). Close the blind spot: for example, require the operator to attest that every runner was restarted after the upgrade, or have new runners register on start and count runners by identity over the 24-hour retention.

18. Major — The key id and version of a Transit script signature are undefined, so the signing trigger and the runner's lookup cannot be written
    Where: docs/designs/015-opentelemetry-and-openbao.md:181-184 (triggers refuse rows "signed by the local key" or "Transit-signed"), 829-830 (`keys` in `SigningKey`); crates/application/src/script_signing.rs:42-50, 114-123 (key id is the first 8 bytes of the public key's SHA-256); crates/infra-postgres/migrations/20261014000000_scripts.up.sql:55, 63
    What: The database cannot tell a local key id from a Transit one unless a format or a column says which signed the row. The doc defines neither, nor how the Transit version is recorded. The runner's `Verifier` looks keys up by `key_id_of(public_key)`.
    Why: The trigger, `scripts retire-key-versions --below N` ("signed below N") and the runner's verification all need it.
    Correction: Define the stored key id for Transit signatures (for example `transit:sagat-scripts:v<N>`, or `key_id_of` of the version's public key plus a `key_version`/`signer` column). Write the trigger predicate and the runner's lookup rule in terms of it.

19. Major — How the Transit keys are created is incomplete: derivation and key types are not stated
    Where: docs/designs/015-opentelemetry-and-openbao.md:581-583 (Technical Plan 7), 241-244 (Requirement 015-019); infra/openbao/bootstrap-kubernetes.py and infra/openbao/bootstrap-development.py (no Transit today)
    What: Per-tenant derivation needs `sagat-secrets` created with `derived=true`. The doc lists only `exportable`, `allow_plaintext_backup`, `deletion_allowed` and `auto_rotate_period`. It gives no key types (`aes256-gcm96`, `ed25519`, and which type for `sagat-intent`, for example `hmac`). It does not note that `deletion_allowed` is set through `keys/<name>/config` after creation, not at creation.
    Why: Without derivation the context is not applied and tenant separation fails. The bootstrap author has to guess the types.
    Correction: Give the exact creation and configuration parameters for each key, and add a bootstrap check that refuses an existing key whose type or derivation differs.

20. Major — The standby waits depend on feature 032's standby list, which does not exist, with no dependency recorded
    Where: docs/designs/015-opentelemetry-and-openbao.md:151-152, 164-166, 542-545, 552-555; specs/032-multi-region-failover/tasks.md (one task ticked); no `standby` code under `crates/`
    What: `advance-seal-version --standby-restored-at`, `rekey` and `retire-key-versions` wait for "every standby PostgreSQL of the cell" from feature 032's list. That list is not built. The doc does not say what the commands do when there is no list or no standby, and no task carries an `Owner-gate:` line for the dependency the constitution requires.
    Why: Task 015-14 cannot be built or tested as written.
    Correction: State the behaviour for a cell with no standby (skip the wait and record that in the audit). Either define where the standby list comes from in this design (for example a command flag or configuration) or gate the standby part on feature 032 with an `Owner-gate:` line.

21. Major — `retire-key-versions --below N` can raise the sealing floor above the sealing version and stop every seal
    Where: docs/designs/015-opentelemetry-and-openbao.md:160-166, 545
    What: Nothing requires `N <= seal_version`. If `N` is higher, every seal fails at its commit and "is retried with a fresh data key" at the same `seal_version`, forever. The doc also does not say whether the floor stays raised when the count afterwards is not zero, or how many retries a seal makes.
    Why: A defect a builder is likely to build, and an outage of all secret writes.
    Correction: Add the guard `N <= seal_version` to the command table. State that a non-zero count leaves the floor raised (or lowers it in the same command), and give the seal retry bound and the error after it.

22. Major — No storage registry matches "the classification registry gains `wrapped_key` as `secret`"
    Where: docs/designs/015-opentelemetry-and-openbao.md:522; Task 015-12; crates/infra-postgres/storage-classification.json:10-15 (classes `content`, `log`, `step_log`, `protected_system`; every variable-width column listed); docs/adr/ADR-0033-payload-values-are-classified-per-field.md (it classifies payload fields, not columns)
    What: The phrase "classification registry" appears nowhere else in the repository. The storage registry has no `secret` class. It also lists every variable-width column, so the new text and bytea columns (`wrapped_key`, the columns of `script_runner_trust_fetches`) each need an entry.
    Why: The builder must guess which file and which class.
    Correction: Name the file and the class for each new column, or add the `secret` class to `storage-classification.json` with its rule, and list every new variable-width column.

23. Major — Commands that change Transit do not define the order of the Transit call and the audit commit
    Where: docs/designs/015-opentelemetry-and-openbao.md:527-530 ("writes its audit record … in the transaction that commits its change"), 541-548; Requirement 015-012 (148-152)
    What: `secrets rotate`, `retire-key-versions`, `trim-key-versions` and the rotate and retire commands for scripts and intents change OpenBao, which is not part of any PostgreSQL transaction. The doc does not say whether the audit and `key_authority_record_rotation` commit before or after the Transit call, or how a crash in between is recovered. It also does not say whether `advance-seal-version` compares the snapshot time with the recorded rotation time or with Transit's own creation time for version `N`.
    Why: A crash between the two leaves a rotation unrecorded or an audit record that claims an unapplied change, and the snapshot guard may compare against the wrong time.
    Correction: For each command, specify the order (for example: commit an intent audit record, call Transit, commit the outcome), what a rerun does, and say that the guard uses Transit's `key_info` creation time for version `N`.

24. Major — The Raft snapshot schedule required of the chart needs a `sys/` grant that the policy test must refuse
    Where: docs/designs/015-opentelemetry-and-openbao.md:153 ("its policies grant no `sys/` path"), 216-218 (the test refuses any `sys/` path), 245-247 (Requirement 015-020), 819-821 (Task 015-23); infra/helm/values.yaml:47-49 (OpenBao is operated by the operator, not deployed by the chart)
    What: A scheduled snapshot calls `sys/storage/raft/snapshot`, which needs a token whose policy grants a `sys/` path. The doc does not say who holds that token, where snapshots are written, or how they are encrypted. The chart does not deploy OpenBao.
    Why: Task 015-23 and the policy test of Task 015-17 contradict each other.
    Correction: Either move the snapshot schedule to the operator's OpenBao installation (`infra/openbao/production-values.yaml` and the runbook) under an operator-only policy that is outside the platform's allow-list test and named as such, or specify the chart CronJob, its token source, its destination and its encryption, and the narrow exception in the test.

25. Major — Requirements on the chart have no task or no defined values
    Where: docs/designs/015-opentelemetry-and-openbao.md:126 ("the Helm chart's validation follows"), 254-255 (Requirement 015-021, the NetworkPolicy), 354 and 735 (`telemetry.otlpEndpoint`); infra/helm/templates/_helpers.tpl:208-212 (ring always set from `secrets.existingSecret`); infra/helm/templates/_openbao.tpl:15-17; infra/helm/values.yaml:88-90 (`databaseRunner.telemetry.tracesEndpoint`)
    What: No task adds a chart value for the key authority or the validation of Requirement 015-010 (no ring under `transit`; ring and Transit together in the passages). Only Task 015-16 covers the intent key. Kubernetes NetworkPolicy has no deny rule, and the doc does not say how "customer key services" are identified (address blocks to exclude, a values list the chart refuses as peers). It also does not say how the new `telemetry.otlpEndpoint` relates to the existing `databaseRunner.telemetry.tracesEndpoint` and `egress`.
    Why: The builder would invent chart values and semantics.
    Correction: Name the values (for example `keyAuthority.mode`), the validation failures and the rendered NetworkPolicy shape, for example `ipBlock.except` built from `networkPolicy.customerKeyServices`, plus a refusal when an egress peer overlaps them. Say whether `databaseRunner.telemetry.*` is kept, replaced or derived.

26. Major — The alert rules and the change to the shared outcome set have no task
    Where: docs/designs/015-opentelemetry-and-openbao.md:235-238 (one outcome set "for every provider"), 612-617 (two alert rules, `secret_versions_by_custody` sampling); crates/infra-secrets/src/openbao.rs:191-196 (today's outcomes `ok`, `denied`, `conflict`, `failed`)
    What: Using one outcome set for every provider changes the existing KV client's labels. The alerts on `unavailable` and `refused` need entries in `infra/observability/alerts.yaml` and promtool tests. The control plane needs an upkeep task for the custody gauge. No task names any of these.
    Why: Principle VI work with no task, and an existing metric changes without a named change.
    Correction: Add them to Task 015-11 and Task 015-13 (or a new task): the `openbao.rs` change, the alert rules with promtool cases, the upkeep sampling, and a dashboard row.

27. Major — The "re-seal inventory" of other features is undefined, and `finish-migration` ignores it
    Where: docs/designs/015-opentelemetry-and-openbao.md:535 ("the entries other features add to its re-seal inventory (design 040's `audit_sinks` columns, by Task 040-26)"), 537; docs/designs/040-ocsf-event-standard.md:555-560, 868-869
    What: The doc does not define the mechanism through which another feature registers sealed columns, or the guard. `finish-migration` checks only `secret_versions`. Ring-sealed `audit_sinks` columns would then become unreadable, because under `transit` no process may hold the ring.
    Why: Switching to `transit` would silently break design 040's sinks.
    Correction: Define the inventory: a trait or list in `crates/application`, each entry with a count of rows still sealed by the ring. Make `finish-migration` and `finish-return` require zero across the whole inventory.

28. Nit — The lanes do not cover every task, and Task 015-25 touches every application
    Where: docs/designs/015-opentelemetry-and-openbao.md:704-714
    Correction: Assign Task 015-26 (contracts, catalogue, Studio) and Task 015-27 (`crates/api-http`) to a lane. List `apps/execution-engine`, `apps/sandbox-runner`, `apps/database-runner` and `apps/cli` among the crates touched, and the order points between Task 015-3 and Task 015-25.

29. Nit — Type names disagree between sections
    Where: docs/designs/015-opentelemetry-and-openbao.md:589 (`TransitKeyAuthority`) and 662 (`KeyAuthority` "in the application") against 440-468 (`EnvelopeKeys`, `TransitEnvelopeKeys`)
    Correction: Use the names of Technical Plan 4 everywhere, and say which adapter owns the cache.

30. Nit — The error catalogue does not have "six copies"
    Where: docs/designs/015-opentelemetry-and-openbao.md:831; sdks/error-catalog.json plus four JSON copies (`sdks/dotnet/...`, `sdks/go/`, `sdks/java/...`, `sdks/rust/src/`), with TypeScript and Python generated by `sdks/typescript/scripts/sync-shared.mjs` and `sdks/python/scripts/sync_shared.py`
    Correction: Name the copies and the sync scripts to run.

31. Nit — The associated data is described with a field it does not contain
    Where: docs/designs/015-opentelemetry-and-openbao.md:63-64 ("binds the tenant, namespace, name, version and key id"); crates/domain/src/secrets.rs:192-211 (tenant, namespace, name, row id, key id)
    Correction: Say "row id" instead of "version".

32. Nit — The new sensitive names catch harmless fields
    Where: docs/designs/015-opentelemetry-and-openbao.md:112-114; crates/telemetry/src/redact.rs:39-42 (matching by substring)
    Correction: Note that `data_key` also hides a textual `data_key_issued_at`, and that `plaintext` hides any field so named. Either accept this in the doc or match the new parts as whole words.

33. Nit — The default of `OTEL_LOGS_EXPORTER` is not stated
    Where: docs/designs/015-opentelemetry-and-openbao.md:391-394; infra/docker/compose.observability.yaml:21-39 (the common endpoint already set)
    Correction: State the default (the OpenTelemetry specification's `otlp`) and that an upgrade with a common endpoint already set starts exporting logs, so an existing Collector without a logs pipeline will answer `404`, counted as failed batches.

34. Nit — The location and language of the exact allow-list policy test are not stated
    Where: docs/designs/015-opentelemetry-and-openbao.md:216-222; Task 015-17 (795-799)
    Correction: Name the test target (for example a Python test beside `infra/openbao/` run by the continuous integration workflow, or a `sagat-tests` target) and the allow-list file format.
