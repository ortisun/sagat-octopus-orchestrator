# Architecture review of design 015, round 1 (2026-11-26)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: 0a390c4b04ba3d1211098b82de84e3727aff0900
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 2, Major 14, Nit 10

1. Blocker — The sealing floor only fences inserts, so a re-key or re-seal can land below a retired version
   Where: Requirement 015-012 (lines 137-141, "on every insert"); §5 (410-411); §6 rows `reseal-secrets --to transit`, `secrets rekey` and `trim-key-versions`.
   What: `reseal --to transit` and `rekey` write `transit` rows by UPDATE, using a data key fetched at a `seal_version` read before their transaction. Suppose one fetched at version 3 and commits after `retire-key-versions --below 4` has counted zero. The trigger does not refuse it. `min_decryption_version=4` then makes the row unopenable, and `trim-key-versions` makes that permanent: its guard is the declared backups, and it never recounts.
   Why: secret values are lost. The fence that Success criterion 015-002 relies on misses two of its three writers.
   Correction: make the trigger `BEFORE INSERT OR UPDATE OF wrapped_key, wrapped_key_version`. Have trim recount zero rows below `N`. Add a barrier test that holds a rekey between `datakey` and commit.

2. Blocker — The rollout gate departs from accepted ADR-0082 without a new ADR, and part of it cannot register
   Where: Requirement 015-010 (113-119); §6 `begin-migration`; §10 (493-495); Task 015-18.
   What: "Refuse to start" contradicts ADR-0082 decision 3 (a refused process MUST NOT exit; it stays up with its gate closed). It also contradicts decision 7, which already covers an engine restarted without the configuration after an activation. "Every … of 0.3.0 lists `key-authority-transit-v1`" is a binary capability, which is the option ADR-0082 rejected (option 5). The secret cleanup job cannot register at all: `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:39` allows only `CONTROL_PLANE` and `EXECUTION_ENGINE`. It also opens no secret (§7, line 454).
   Why: Governance requires a new ADR for a departure from an accepted one. As written, the gate cannot be built.
   Correction: follow ADR-0082 (gate closed, capability listed only when the Transit configuration is present). Drop the cleanup job from the gate. Cite ADR-0082 in ADR-0152.

3. Major — The `KeyAuthority` port cannot be held where it is used, and its local adapter would be stubs
   Where: §4 (366-404).
   What: `impl Future` methods are not dyn-compatible. `SecretReader` is a non-generic `Clone + Default` holder of `Arc<dyn …>` (`crates/application/src/secrets.rs:50-54`), shared by worker, scheduler, events and connectors, and the authority is chosen at run time. `SecretProvider` boxes its futures for exactly this reason (`secret_provider.rs:36-56`).
   `LocalKeyAuthority` has no meaning for `new_data_key`, `unwrap`, `rewrap` or `key_info(TransitKey)`. It also cannot bind or verify version 1 from `value_sha256`, because version 1 HMACs the value itself (`managed_secrets.rs:142-147`). It would be stubs (AGENTS rule 11). `seal_with` on `SecretCipher` needs a ring instance, which `transit` forbids. `unwrap` cannot tell the cache whether the version is current. `SealedSecret.key_id` is not optional.
   Correction: use boxed futures. Split the port into `EnvelopeKeys` (Transit only, with a `Cacheable` input), `ScriptSigner` and `IntentBinder`, and keep version 1 in `ManagedSecretService`. Make data-key AES-GCM its own function or port. Name the `StoredValue::Transit` payload.

4. Major — It is not decided which custody a new secret takes when feature 017's KV provider is configured
   Where: Terms (41-43); Requirement 015-011; §5 (412-414); O015-004.
   What: today, whenever OpenBao settings exist, every `put_secret` goes to KV and requires `requestId` and `expectedVersion`. See `apps/control-plane/src/lib.rs:758-773` and `crates/api-http/src/routes/governance.rs:301-334`. A `transit` deployment runs OpenBao, so built as described the `transit` custody is never used, or the API's required fields silently change. "KV for the values that need it" names no selector.
   Correction: state the routing rule (for example, `requiredProvider: openbao` and database credentials go to KV, everything else to `transit`) and the API's field rules in each mode. Test each route.

5. Major — Contract changes the design causes are neither named nor tasked
   Where: §4, §5, Requirements 015-013 and 015-017, Detailed Implementation Plan.
   What:
   - `Secret.provider` is `[local, openbao]` and `keyId` is documented as "null for OpenBao" (`specs/001-platform-foundation/contracts/openapi.yaml:6050-6051`). `SecretView.provider` is derived from `key_id` (`application/src/secrets.rs:130-134`), so a `transit` row would be shown as `openbao`.
   - `SigningKey` holds a single key (openapi.yaml:7788-7793), but the endpoint is "extended".
   - `KEY_AUTHORITY_UNAVAILABLE` is not in `sdks/error-catalog.json` or its six copies.
   - The new audit actions need design 040's OCSF mapping.
   Correction:
   - Add `transit` to the enum and fix `SecretView`.
   - Extend `SigningKey` additively: keep `keyId`/`publicKey` as the current key and add `keys` and `minimumVersion`.
   - Either reuse `UNAVAILABLE` with a `details.reason`, or add catalogue entries.
   - Add one task with the contract and conformance tests.

6. Major — Rolling back from `transit` has no path, yet the check passes Principle IV on it
   Where: §10 (495-496); Constitution Check IV (523); Complexity Tracking; Gap 015-004.
   What: no §6 command leaves `transit`. The trigger refuses `local` rows unless the mode is `local`. Processes in `transit` refuse `SAGAT_SECRETS_KEYS`. The "documented re-seal" therefore needs hand-written SQL against a protected row and a ring that the processes refuse. Check IV's condition points to a Complexity Tracking row that does not exist.
   Correction: add a guarded, audited `key-authority begin-rollback` and `reseal-secrets --to local`. Otherwise mark rollback unsupported and justify it in Complexity Tracking.

7. Major — `migrating` mode does not define how scripts are signed or intents bound, and older sandbox runners are not gated
   Where: Requirements 015-013 to 015-015; §6 `finish-migration`; §10.
   What: only secret rows have a database fence. The design never says whether artifacts and intents created during `migrating` use the local key or Transit. That makes "zero locally signed artifacts, zero pending version-1 intents" a check-then-act against concurrent builds and writes. Sandbox runners are not in the registry. A 0.2.x runner trusts one key, fetched once (`apps/sandbox-runner/src/lib.rs:776-798`), and fails every Transit-signed artifact as untrusted, permanently.
   Correction: sign and bind only through Transit from `begin-migration` on, with database checks that refuse local-signed artifacts and version-1 intents outside `local`. Require upgraded runners before `begin-migration`, and say how that is checked.

8. Major — Several commands need OpenBao operations that the allow-list forbids or that work differently from what is described
   Where: Requirements 015-012, 015-016 and 015-020; §6; §7 (448).
   What:
   - No role holds `update` on `keys/<name>/rotate`, so `secrets rotate` cannot run.
   - `min_available_version` is set through `keys/<name>/trim`, not `config`, and the test refuses every `trim` path.
   - "A snapshot holds the version" needs `sys/storage/raft/snapshot`, which `sys/` refuses, and the platform cannot read a snapshot without restoring it.
   - Nothing rotates or retires `sagat-scripts` or `sagat-intent`.
   - `datakey` honouring `key_version` is assumed, not shown.
   Correction:
   - Carve out `rotate` on the three keys and `trim` on `sagat-secrets` for the operator role.
   - Define the snapshot gate as a recorded operator attestation or a restore drill.
   - Add rotate and retire commands for the other two keys.
   - Prove `key_version` on `datakey` against OpenBao 2.7.1 at the start of Task 015-11.

9. Major — Regions: only the rotation waits for the standby
   Where: Requirement 015-012; §6; Delivery Constraints (Regions).
   What: the constitution says rotations and revocations count as durable only once the standby holds them.
   - Retire raises `min_decryption_version` without checking that the standby PostgreSQL has replayed the rewrap.
   - The compromise procedure (revoke, rekey) has no standby check.
   - A standby's OpenBao is only ever restored from snapshots (feature 032, Gap 1560), so "the standby holds the version" has no mechanism.
   - Feature 032's key inventory (`specs/032-multi-region-failover/research.md:256-258`) fingerprints the three keys as blocking checks, which a key held in Transit cannot pass.
   Correction: gate retire and the compromise procedure on the standby's replay position. Define the standby check. Record the change to 032's inventory as a dependency.

10. Major — Two readers of secret values are missing from Requirement 015-017
    Where: Requirement 015-017; §4 (400-403); Task 015-10.
    What: inbound webhook verification skips any non-OpenBao failure and then answers `Refusal::NoSecret` (`crates/application/src/events.rs:1009-1023`). Under `transit`, an OpenBao outage therefore becomes a non-retryable refusal to the sender, and events are lost when the sender does not retry a 4xx. The managed-database reader (`crates/application/src/worker/database.rs:79,223`) is also unlisted.
    Correction: list both readers. Map `Unavailable` to a retryable 503. Cover both in the outage test of Success criterion 015-005.

11. Major — The output scan puts OpenBao on every step report, without a bound
    Where: §8 (471-472); CUJ 3; Non-functional requirement 015-002.
    What: a step report scans every kept version of every declared secret, up to 16 × 5 = 80 opens (`crates/application/src/worker/secrets.rs:239`). Only the 16 current versions can be cached, so up to 64 are single cold Transit calls (2-second attempts, 5-second budget). During an outage, every report of such a step is refused, which CUJ 3 does not say. Nothing bounds Transit concurrency, so one tenant can saturate the shared OpenBao.
    Correction: use one `batch_input` decrypt per report. Scan only the versions handed out to that attempt. Add a global and a per-tenant concurrency limit with a metric. Extend CUJ 3 to cover reports.

12. Major — The key-authority commands are not audited, and the database role allowed to run them is not named
    Where: §6; Constitution Check VI (525).
    What: `begin-migration`, `finish-migration`, `rotate`, `retire-key-versions`, `trim-key-versions` and `scripts resign` change policy for every secret but write no audit record. The database role allowed to update `key_authority_state` is not named. If `sagat_app` can, a compromised API process can lower `min_seal_version` or advance `seal_version` to a version no snapshot holds.
    Correction: write an audit record in each command's transaction. Allow changes to the row only through owner-role functions, as ADR-0082 decision 1 does, with a row in the privilege audit.

13. Major — The central rule has no fitness function
    Where: ADR-0152 "Enforced by"; `docs/engineering/architecture-principles.md`.
    What: the defect both reviews found, a reader that turns a resolve error into a skip, already exists in five places: `worker/secrets.rs:245`, `scheduler/src/deliveries.rs:559` and `:596`, `events.rs:1016`, and `infra-connectors/src/credentials.rs:123`. The plan only tests named readers, so a sixth caller would pass. No principles row is added.
    Correction: add a `scripts/check-architecture.py` rule that refuses `.resolve(` followed by `Err(_) => continue`, `.ok()` or a discarding `map_err`, unless allow-listed with a reason, plus negative fixtures. Add principles rows for it and for the `transit` trigger.

14. Major — The new header rule breaks existing deployments, and the database runner has no route to the Collector credential
    Where: Requirement 015-006; Task 015-3; §7 (450).
    What: `from_lookup` accepts `OTEL_EXPORTER_OTLP_HEADERS` with any `http://` endpoint today (`crates/telemetry/src/otlp.rs:928-973`). The new refusal would stop, on upgrade, every process that sends to an in-cluster Collector behind a mesh. The database runner uses the shared service account and has no Agent (`infra/helm/templates/database-runner.yaml:73`). Granting it "read" means a new service account, role, sidecar and egress to OpenBao, which is the exposure the design refuses the sandbox runner.
    Correction: apply the refusal only to the new header file, or add an opt-in like `grpc.allowPlaintextSecrets`. Give the runner its header through a Kubernetes Secret.

15. Major — Parts of features 031, 040 and 047 are built here without an approved dependency
    Where: Requirement 015-021; §7 rows for design 040 and design 047; Tasks 015-17 and 015-23.
    What: the design cites ADR-0066 decision 1 as accepted, but ADR-0066 is Proposed in this tree (`docs/adr/ADR-0066-client-side-payload-encryption.md:3`). The `sagat/audit/` policy (design 040, Draft) and the broker policy (design 047, absent from this tree) are installed by this feature's bootstrap and frozen by its exact allow-list test. The constitution requires an `Owner-gate:` line for this.
    Correction: add Owner-gate lines that cite the owner rows, or make these rows extension points that the other features add themselves.

16. Major — Two security residual risks are marked accepted under the delegation
    Where: `specs/015-opentelemetry-and-openbao/research.md:44,46` (Gaps 015-003 and 015-005).
    What: Gap 015-003 (cached keys usable for 300 seconds after a revocation) and Gap 015-005 (one process holds every custody token) are marked "Accepted". The constitution keeps every security exception with the owner, outside Owner decision 2026-11-24-4.
    Correction: mark both "Open; owner" and ask the owner, or record an owner row.

17. Nit — research.md:59 still records O015-006 as "90 days automatic", which contradicts Owner decision 2026-11-26-115. Append a superseding line.

18. Nit — §1 gives the log drop counter two names: `telemetry_dropped_total{signal}` (332) and `otel_logs_total{outcome}` (338). Keep the second.

19. Nit — The ADRs' "Refines" lines cite the wrong records: ADR-0006 is about scripts, ADR-0008 is the journal projection, and ADR-0017 (decisions 4 and 5) is the record ADR-0151 actually extends. ADR-0152 also omits ADR-0035 decision 8.

20. Nit — ADR-0152's title and decision 5 say each custody has "its own mount", while Requirement 015-016 and §7 use path prefixes on the shared KV mount.

21. Nit — `secret_versions_by_custody` is "sampled by the rewrap command", but a one-shot command is never scraped. Move it to the control plane's metrics upkeep. Also, the alert "on any Refused" has no matching outcome in the set of Requirement 015-018.

22. Nit — The workspace sets `unsafe_code = "forbid"` (`Cargo.toml:81`), and `PR_SET_DUMPABLE` exists only on Linux. Task 015-25 should name a safe wrapper (`rustix` or `nix`) and the behaviour on other platforms. Its Refs cite the wrong requirement.

23. Nit — The migration's `down` must refuse while any `transit` row, `secrets.authority = 'transit'` or a non-`local` mode exists, because the old representation constraint cannot hold them.

24. Nit — In a logs-only configuration no `OtlpLayer` is installed, so log records carry no span ids. Say so. Also state that any `OTEL_LOGS_EXPORTER` value other than `otlp` or `none` is a configuration error (ADR-0017 decision 4), and that the exporter's own log targets are kept out of the log queue.

25. Nit — The two lanes are not independent: Task 015-7 (lane A) needs the per-custody Agent and the `sagat/telemetry/` policy from Tasks 015-17 and 015-23 (lane B). Add this as an order point.

26. Nit — A migration cannot read `SAGAT_SECRETS_KEYS`. "Refuses to run when a ring key is named `transit`" can only mean refusing existing rows with `key_id = 'transit'`; say that.
