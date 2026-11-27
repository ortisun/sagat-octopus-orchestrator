# Security review of design 015, round 1 (2026-11-26)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: ff5bad03c6cd678e6bc6817d82d2d616550f50c1
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 3, Major 13, Nit 6

Every finding below was checked against the code in the worktree, except two that are partly inference and are marked so. Two limits on what I could check. I have no shell, so I could not confirm the worktree is at `ff5bad03`, and I could not run `git show` on the other branches. ADR-0066 was read from this worktree, where its status is Proposed. Design 047 was read from `(the sibling worktree of docs/windmill-parity-readiness) docs/designs/047-oauth-integration-hub.md`. To confirm that copy matches the branch, run `git -C (a local path) show origin/docs/windmill-parity-readiness:docs/designs/047-oauth-integration-hub.md | diff - (the sibling worktree of docs/windmill-parity-readiness) docs/designs/047-oauth-integration-hub.md`.

1. Blocker — Rewrap changes `key_id`, which the encryption binds to the ciphertext, so every rewrapped secret stops opening
   - **Where:** Technical Plan §5 (lines 313-316) and §6 (lines 321-323); Success criterion 015-002.
   - **What is wrong:** The design stores `bao-<version>` in `key_id`, and the rewrap's `UPDATE` "writes the new wrapped keys and `key_id`". But the associated data authenticated by AES-GCM includes `key_id` (`crates/domain/src/secrets.rs:192-211`). Opening rebuilds that associated data from the row's `key_id` (`crates/application/src/secrets.rs:79-86`). Rewrap does not re-encrypt the value, so a row sealed under `bao-3` and rewritten to `bao-4` can never be opened again.
   - **Why it matters:** CUJ 2 would make 200,000 secrets unreadable. Raising `min_decryption_version` afterwards removes the only way back.
   - **Correction:**
     - For Transit rows, put a fixed authority tag in the associated data (for example `bao`, never the version) and read the version from the `vault:v<N>:` prefix of `wrapped_key`.
     - Alternatively, leave `key_id` unchanged on rewrap and select rewrap candidates by the parsed version of `wrapped_key`.
     - Add a test that runs rewrap and then opens the value; mutate it by putting the version back into the associated data.

2. Blocker — `sign_script(digest)` drops the tenant and language from what is signed
   - **Where:** The `KeyAuthority` port (lines 297-298); Requirement 015-013.
   - **What is wrong:** Today the signed message is `"sagat/script-artifact/v1\n{tenant}\n{digest}\n{language}\n"` (`crates/application/src/script_signing.rs:33-40`). That is what stops one tenant's artifact being run as another tenant's, or the same bytes being run as another language (`docs/architecture/security-and-sandbox.md:100`). The new port signs only a 32-byte digest.
   - **Why it matters:** A build with a stolen runner token (G118) could then have an artifact signed and replayed into another tenant. That is unauthorized script execution and a cross-tenant breach, both release-blocking under Principle V.
   - **Correction:**
     - Change the port to `sign_script(&self, tenant: TenantId, digest: &Digest, language: Language)`.
     - Send `script_signing::message(...)` as the Transit `input` and verify the same bytes in the runner.
     - Add tests under the Transit authority that refuse a signature presented for another tenant or another language.

3. Blocker — The output scan for secrets skips Transit rows during an outage, so a secret can reach the journal
   - **Where:** Requirement 015-017; Success criterion 015-005; §8.
   - **What is wrong:** `secret_in_output` (`crates/application/src/worker/secrets.rs:239-246`) unseals every stored version of every declared secret to look for it in the step output. It fails closed only for `StoredValue::OpenBao`; for any other variant it runs `Err(_) => continue`. The design keeps Transit rows in the same `secret_versions` shape (`key_id`, `nonce`, `ciphertext`, plus `wrapped_key`), so a builder would naturally represent them as `StoredValue::Local`.
   - **Why it matters:** While OpenBao is down, or for older versions that are not cached, the unwrap fails and the scan is silently skipped. A value the worker echoed then goes into the output and the journal. Success criterion 015-005 covers deliveries and seals only.
   - **Correction:**
     - Add a `StoredValue::Transit` variant, or a stated rule, so that `KEY_AUTHORITY_UNAVAILABLE` in the scan refuses the report as retryable.
     - Add this path to Success criterion 015-005 and to the canary test (Task 015-20).

4. Major — The rewrap pass loops forever on a row that cannot be rewrapped, and its port takes one tenant per batch
   - **Where:** §6 (lines 321-325) and the port (`rewrap(&self, tenant, batch)`).
   - **What is wrong:** "No cursor is needed: the predicate is the cursor" fails on a row that cannot be rewrapped (corrupt, moved to another tenant, or a per-item error from Transit). Its `key_id` never changes, so it is selected again every batch. With 250 such rows the pass makes no progress and never reports zero. Separately, batches are chosen by `key_id` across all tenants, but the port carries a single `TenantId`, while Transit needs a derivation context per item. The existing `reseal` keeps an `after` cursor and a `failed` count for exactly this reason (`crates/application/src/secrets.rs:738-806`).
   - **Correction:**
     - Change the port to `rewrap(&self, batch: &[(TenantId, WrappedKey)]) -> Vec<Result<WrappedKey, _>>`.
     - Order by `id` with an `after` cursor, and count, audit and report failures per item.
     - Make the `min_decryption_version` command refuse while any failure remains.

5. Major — Raising `min_decryption_version` is check-then-act, and it interacts with database backups
   - **Where:** Requirement 015-012; CUJ 2; Success criterion 015-002.
   - **What is wrong:**
     - A `put` fetches a data key at version 3, the operator rotates, the pass counts zero, the operator raises the minimum to 4, and then the `put` commits a `bao-3` row. That row cannot be opened.
     - A PostgreSQL restore taken before the rewrap brings back version-3 rows after the minimum was raised.
     - Trimming (`min_available_version`) would make either case permanent.
   - **Correction:**
     - Have the command count again after a grace period longer than the longest seal: the Transit budget plus the statement timeout.
     - Forbid trimming while backups older than the last rewrap are retained.
     - Add "lower `min_decryption_version`" as a step in the runbook's restore procedure.

6. Major — Losing OpenBao loses every secret permanently, and nothing requires a backup
   - **Where:** §7 (`exportable=false`, `allow_plaintext_backup=false`); Risks; Requirement 015-019.
   - **What is wrong:** Under `openbao`, PostgreSQL alone no longer restores any secret. The keys cannot be exported, and a three-node Raft cluster provides availability, not backup.
   - **Why it matters:** It contradicts the Delivery Constraints' recoverability and leaves Principle VIII with no drill.
   - **Correction:**
     - Add a requirement for scheduled, encrypted Raft snapshots and for custody of the unseal and recovery keys.
     - Add a paired restore test (a PostgreSQL backup plus an OpenBao snapshot) to `integration_key_authority`.

7. Major — The design ignores the OpenBao custody of ordinary secrets that already exists
   - **Where:** The Problem (lines 64-67); Non-Goals (line 191); §5; Owner question 015-004.
   - **What is wrong:**
     - Feature 017 already stores ordinary tenant secrets in OpenBao KV (version 2). `secrets.authority IN ('local','openbao')` and the representation check are in `crates/infra-postgres/migrations/20261122000100_openbao_secrets.up.sql:2-49`. The code also has `StoredValue::OpenBao` and `ManagedSecretService::migrate`, which moves secrets from the key ring to KV.
     - `SecretView.provider` is derived from whether `key_id` is present (`crates/application/src/secrets.rs:130-134`), so a Transit row would be shown as `local`.
     - `SAGAT_KEY_AUTHORITY=openbao` reuses the word that `secrets.authority='openbao'` already gives to KV custody.
     - A key-ring id may legitimately be `bao-3`, and the proposed check does not tie `wrapped_key` to `key_id`, so "a row says which authority opens it" does not hold.
   - **Why it matters:** The builder cannot tell which path a new secret takes under `openbao`, or how the two migrations combine.
   - **Correction:**
     - State how KV custody and Transit custody coexist and how the two migrations relate.
     - Route opening by `wrapped_key IS NOT NULL`, and add `CHECK ((wrapped_key IS NULL) = (key_id IS NULL OR key_id !~ '^bao-'))` inside the representation constraint.
     - Make `KeyRing::parse` refuse ids starting with `bao-`.
     - Rename the setting or the column value so the two meanings cannot be confused.

8. Major — Moving the intent HMAC to Transit is underspecified where it matters
   - **Where:** Requirement 015-014; Requirement 015-010; Task 015-16.
   - **What is wrong:**
     - The binding HMAC covers the plaintext value (`crates/application/src/managed_secrets.rs:142-147`). With `transit/hmac`, every value would be sent to a second OpenBao endpoint and pass through its audit device.
     - `binding` is checked to be exactly 32 bytes and is immutable (`20261122000100_openbao_secrets.up.sql:13,26-38`), so it cannot hold `vault:vN:…` and has no column for the key version.
     - Intents still pending under the key file have no migration; migrating mode covers secret rows only.
     - The chart requires `intentKeySecret` and always sets `SAGAT_OPENBAO_INTENT_KEY_FILE` for the control plane (`infra/helm/templates/_openbao.tpl:15-17,46-48`). Under Requirement 015-010 the control plane would then refuse to start.
   - **Correction:**
     - Send Transit the metadata plus the SHA-256 of the value, under a new binding version.
     - Add a migration for the binding format and its key version.
     - In migrating mode, verify with the file key for the old format and with Transit for the new one, and drain pending intents before the file is removed.
     - Change the chart's validation and environment.

9. Major — The policy test is both too strict and too weak
   - **Where:** Requirement 015-016; §7 (lines 339-343).
   - **What is wrong:**
     - **Too strict:** every existing policy grants `auth/token/{lookup,renew,revoke}-self` (`infra/openbao/policies/*.hcl`), which lie outside "the three mounts". The client calls `lookup-self` before every production request (`crates/infra-secrets/src/openbao.rs:137-159`), and "no default policy" makes the explicit grant necessary.
     - **Too weak:** a check by mount prefix accepts `sagat-transit/*` with `update`. That reaches `keys/sagat-secrets/config` (which can turn on `exportable` and `deletion_allowed`) as well as `export/`, `backup/` and `restore/`.
     - `secret/sagat/credentials` is a path inside the shared `secret` mount, and that mount's name is configurable (`SAGAT_OPENBAO_MOUNT`, `openbao.mount`). Design 047's mount is `SAGAT_OAUTH_OPENBAO_MOUNT`, not `sagat-oauth`.
     - `secret-cleanup` (`cleanup.hcl`) is missing from the table.
   - **Correction:**
     - Use an exact allow-list of (path, capabilities) per process kind, generated from the table plus the three token self-paths.
     - Refuse `*` and `+` in the mount segment, and refuse `sys/` and every other `auth/` path.
     - Substitute and validate the configured mount names in the bootstrap.
     - Add a row for secret cleanup.

10. Major — "Its own token per custody" cannot be built with the chart as described
    - **Where:** §7 (lines 337-339); Requirement 015-016.
    - **What is wrong:**
      - Each pod has one OpenBao Agent with one `auto_auth` and one sink (`infra/helm/templates/openbao-agent.yaml:10-33`). The Agent is defined only for control-plane, execution-engine and secret-cleanup; there is no sandbox runner or database runner template.
      - The control plane needs a KV token and a Transit token.
      - Design 047's broker runs inside the control plane process (047, lines 402-413) and needs a third token, but it has no service account of its own.
    - **Correction:** Decide which of these holds:
      - one Agent sidecar per custody in each pod, each with its own role and sink file; or
      - one token per process kind carrying several policies, stating that custody is then separated by code path only, not against a compromised process.

11. Major — The sandbox runner gets an OpenBao credential and an undefined trust-refresh model
    - **Where:** Requirement 015-013; Task 015-15.
    - **What is wrong:**
      - The runner is the host that holds the container engine. The design gives it a token and a network path to the key service, while `docs/architecture/security-and-sandbox.md:26` says the runner holds no scheduler credentials.
      - It ignores the existing trust path: `SAGAT_RUNNER_TRUSTED_KEYS`, plus trust on first use through `/script-runner/signing-key` (G102, `apps/sandbox-runner/src/lib.rs:780-790`).
      - "Non-retired" is not defined; it should mean at or above `min_decryption_version`, since reading the key lists older versions too.
      - No refresh interval is given, so a retired or compromised version stays trusted until restart.
    - **Correction:**
      - Serve the public keys and the minimum version through the existing authenticated control-plane endpoint instead of from OpenBao.
      - Define retirement, and refresh on an unknown key id (rate-limited) and at least every N minutes.
      - Add a test that a retired version is refused without a restart.

12. Major — A cache that expires from insertion contradicts CUJ 3 and Success criterion 015-005
    - **Where:** §8; CUJ 3; Success criterion 015-005.
    - **What is wrong:** Entries expire 300 seconds after insertion, so for hot keys the expiry times are spread evenly. During a 40-second outage about 40/300, roughly 13 percent, of the hot entries expire, and those deliveries fail. "Deliveries continue" and "succeed for up to 300 seconds" are therefore false as written. One LRU shared by all tenants also lets one tenant evict the others' keys.
    - **Correction:** Either:
      - unwrap an entry again in the background once it is older than 240 seconds while OpenBao is healthy (the revocation bound stays at 300 seconds); or
      - correct CUJ 3 and Success criterion 015-005 to "up to 300 seconds since each key's last unwrap".
      
      In either case, bound each tenant's share of the cache.

13. Major — Rotation plus rewrap is presented as a response to compromise, but it keeps the data keys
    - **Where:** Requirement 015-012; User story 015-02; CUJ 2.
    - **What is wrong:** `transit/rewrap` re-encrypts the same data key. An attacker who used a stolen engine token and a database read to unwrap data keys can still read every value after rotation and rewrap. No path issues fresh data keys for rows already under Transit.
    - **Correction:**
      - Add `secrets rekey`: re-seal Transit rows with new data keys, one row per transaction, audited.
      - Write a runbook entry for token compromise.
      - State in Requirement 015-012 that rewrap protects only against exposure of an old key-encryption-key version.

14. Major — The Collector's bearer token can be sent in clear, and its custody is not defined
    - **Where:** Requirements 015-004 and 015-006; Task 015-3.
    - **What is wrong:**
      - `OtlpConfig::from_lookup` accepts `http://` (`crates/telemetry/src/otlp.rs:928`) and attaches headers without marking them sensitive (`otlp.rs:528`).
      - A misconfigured endpoint sends the token in plaintext before the TLS-only Collector refuses the connection.
      - "Delivered by the OpenBao Agent sidecar" requires an Agent template that reads a KV path. That path is in no custody row or policy, and the policy test in finding 9 would refuse it.
    - **Correction:**
      - Refuse a configured authentication header unless the endpoint is `https` or loopback, and call `set_sensitive(true)` on it.
      - Name the KV path (for example `secret/data/sagat/telemetry/collector`) as a row in the table and the allow-list.
      - Read the file again when it changes or after a `401`.

15. Major — The redaction backstop and the Collector filter do not cover the new material
    - **Where:** Requirement 015-004; §3; Non-functional requirement 015-004; Risks.
    - **What is wrong:**
      - `SENSITIVE_PARTS` (`crates/telemetry/src/redact.rs:19-34`) does not match `wrapped_key`, `data_key`, `plaintext`, `ciphertext` or `hmac`.
      - `scrub` recognizes neither `vault:v<N>:` nor OpenBao token shapes (`hvs.`, `hvb.`, `hvr.`).
      - The Collector filter, "generated from that file", inherits these gaps. An `attributes` processor also never touches the log body.
    - **Why it matters:** The last defense the design claims for wrapped keys and tokens does not exist.
    - **Correction:**
      - Add these names and shapes to `redact.rs` before generating the filter.
      - State that the body is protected only by the in-process scrub.
      - Have the canary test log marked material under unlucky field names.

16. Major — Holding feature 031's keys in the platform's OpenBao contradicts ADR-0066
    - **Where:** §7 (`customer-*` "by convention"); Requirement 015-016; ADR-0152 "refines ADR-0066 decision 1".
    - **What is wrong:** ADR-0066 decision 1 requires key material to exist only in customer processes and the customer's KMS, and its purpose is "values the platform operator cannot read". A policy-separated mount inside the operator's own OpenBao, whose bootstrap uses an administrator token, does not separate keys from the operator. ADR-0152 also refines an ADR that is only Proposed.
    - **Correction:**
      - Require 031's key providers to sit outside the platform's OpenBao: a separate cluster, or at least a namespace whose administrator is the customer.
      - Treat the bootstrap's refusal as defense in depth only.
      - Add a NetworkPolicy so that no platform pod can reach a customer KMS.

17. Nit — The OpenBao client being reused is specific to KV
    - **Where:** §4 ("the bounded client of `openbao.rs`"); Non-functional requirement 015-002; Requirement 015-018.
    - **What is wrong:**
      - The client reads the token file and calls `lookup-self` on every request, so a cold open takes two round trips, not one.
      - It has one mount and one token file.
      - `measured` wraps the whole retry loop in a single 5-second timeout (`openbao.rs:182-200`), which conflicts with "a 5-second timeout, at most 2 retries".
    - **Correction:** Cache the `lookup-self` result until the token's TTL, use a timeout per attempt within a total budget, and configure the client per mount.

18. Nit — Core dumps cannot be disabled through the Kubernetes `securityContext`, and decrypted keys are not fully wiped
    - **Where:** Risks (line 374); Requirement 015-011.
    - **What is wrong:** The `securityContext` has no control over core dumps. Separately, Transit's base64 `plaintext` would be parsed into an ordinary `String` that is never wiped.
    - **Correction:** Call `prctl(PR_SET_DUMPABLE, 0)` and `setrlimit(RLIMIT_CORE, 0)` at process start, and parse responses into a type that wipes on drop, as `EncodedValue` already does (`openbao.rs:425-429`).

19. Nit — The encoding of the derivation context is not fixed
    - **Where:** Requirement 015-011; Owner question 015-005.
    - **What is wrong:** The context is permanent once used. Its form (UUID text or bytes) and any label separating it from other uses of the key are unstated.
    - **Correction:** Specify `base64("sagat/secret/v1" || tenant uuid bytes)` and test it against a golden value.

20. Nit — The new commands and metrics clash with existing ones
    - **Where:** §6; §9.
    - **What is wrong:**
      - The existing command is `reseal-secrets` (`apps/control-plane/src/cli.rs:80`). It selects with `key_id <> active` (`crates/infra-postgres/src/secrets.rs:691-713`), so under `openbao` it would treat Transit rows as stale.
      - `secret_provider_requests_total{provider="openbao"}` already exists alongside the new `openbao_requests_total`.
    - **Correction:** Say how `reseal-secrets` behaves under each authority, or retire it, and keep one metric family.

21. Nit — Comparing versions as text sorts them wrong
    - **Where:** §6 ("selected by `key_id` below the newest version").
    - **What is wrong:** As text, `bao-10 < bao-9`.
    - **Correction:** Compare the parsed integer, or store the version in an integer column.

22. Nit — The rollout gate is underspecified
    - **Where:** §10.
    - **What is wrong:** "Recorded for both kinds" is undefined, although secret cleanup and webhook signing also open secrets. An older binary started after activation is never seen by the registry (the same gap as ADR-0066 G1537).
    - **Correction:** List the process kinds that must report `key-authority-openbao-v1`, and have those binaries refuse a Transit row they cannot open with a typed error.
