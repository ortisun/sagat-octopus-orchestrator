# ADR-0152: OpenBao Transit holds the platform's keys, secret values are sealed in PostgreSQL under tenant-derived wrapped data keys, and each custody has its own mount or path prefix, token and policy

**Status:** Proposed (2026-11-26). Returned from Accepted the same day: design 015's first review round found decisions to revise (its architecture findings are open); it is accepted with design 015's approval
**Decided by:** agent proposal with design 015, accepted by the agent preparing design 015 for build under the owner's
delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-26-119. The owner may overrule it.
**Refines / supersedes:** refines ADR-0035 (feature 017's OpenBao
mount is one custody among several; this is the separate platform-wide proposal its decision 8 leaves the signing-key
and platform migration to), follows ADR-0082 (the gate of a process whose configuration disagrees with the recorded
mode is closed, never an exit; the requirement is a capability listed only with the configuration) and applies ADR-0066
decision 1 (feature 031's customer keys live outside the platform's OpenBao); supersedes none. (An earlier text said it
refined ADR-0033 by classifying the wrapped key `secret`; ADR-0033 classifies payload fields, and the wrapped key's
column is classified in `crates/infra-postgres/storage-classification.json` as `protected_system`, design 015 Technical
Plan 5.)
**Enforced by:** the cross-tenant unwrap test, the rotation and rewrap test, the migration test under load, the
outage test and the paired restore test on a real OpenBao (Tasks 015-19, 015-24); the signature-binding tests (Task 015-15); the exact allow-list policy test (Task
015-17, `scripts/check-openbao-policies.py`); the canary test (Task 015-20); the closed gates of Requirement 015-010
and the two rollout orders under load (Task 015-18); the insert-only signature table (Tasks 015-12, 015-15); the
re-seal inventory (Task 015-34); the rule
`secret-resolve-never-discards` of `scripts/check-architecture.py` and the representation and floor triggers (Tasks
015-10, 015-12); the privilege audit of the state row's functions (Task 015-12). Until they exist: review only.

## Context

Today a key ring in an environment variable seals the secrets stored in PostgreSQL, an Ed25519 key in another signs
scripts, and a 32-byte file binds feature 017's provisioning intents with no rotation; OpenBao's KV engine holds the
values of feature 017's `openbao` custody (design 015, The Problem).

## Decision

1. A deployment MUST choose a key authority, `local` or `transit`; both are supported, and so are the passages between
   them, `migrating` and `returning`. Under `transit` no platform key MAY exist outside OpenBao once the operator has
   recorded the requirement `key-authority-ring-absent-v1`, which a process lists only with no local key: from then on
   a process with a local key configured keeps its gate closed (ADR-0082). Before it is recorded (the hand-over window
   after `finish-migration`, and before `begin-return`) a process with a local key stays open, never uses that key, and
   warns, so that neither passage closes a serving process's gate. A process reads the mode at registration and after
   every registry heartbeat. The mode changes only through functions of the schema-owning role, each change audited in
   its transaction; the triggers that enforce it take the state row's lock through a `SECURITY DEFINER` function the
   runtime role cannot call.
2. Under `transit` a secret version of `transit` custody, which carries no key id so that older readers fail closed,
   MUST be sealed with a fresh data key from Transit key
   `sagat-secrets`, derived with the tenant's context, and stored with its wrapped data key and the key's version in
   PostgreSQL. The value MUST NOT be sent to OpenBao. The associated data MUST name the custody and never the Transit
   key's version. Feature 017's KV custody stays as it is.
3. Rotation MUST need no restart and MUST NOT outrun the backups: new seals use the sealing version recorded in
   PostgreSQL, advanced only once a snapshot holds it; a bounded, resumable pass rewraps stored data
   keys with `transit/rewrap` and reports every row it could not rewrap; raising `min_decryption_version` of any of the
   three keys first raises that key's sealing floor, which the database enforces on every new sealed value, signature
   or binding, then counts across every table of the re-seal inventory; trimming recounts the same way. A re-key command issues new data keys after
   a compromise of a token.
4. Scripts MUST be signed with Transit key `sagat-scripts` over the message that binds the tenant, the digest and the
   language; runners MUST get the public keys from the control plane, never from OpenBao. Artifact bytes stay
   immutable: every signature is a row of an insert-only table, so re-signing adds a row and never updates an
   artifact. Feature 017's intents MUST be bound with Transit key `sagat-intent` over their metadata and the digest of
   the value, never the value.
5. Each custody MUST have its own mount or path prefix and its own policy, and each process kind its own token per
   custody it uses. Every
   policy MUST be an exact allow-list; the bootstrap and a test refuse any other grant. Each allow-list declares the
   custody prefixes it owns, and the test refuses a grant outside them and any overlap of prefixes between features or
   with the platform's own, so that a feature's fragment cannot certify a grant on another custody. Feature 031's customer keys MUST
   live outside the platform's OpenBao.
6. Without OpenBao nothing is sealed or opened by other means, on every read path including the scan of a step's
   output, which opens every retained secret version under `transit` as under `local`. Opening MAY use an unwrapped data key from a cache of at most 4,096 keys, 512 per tenant, each at most 300
   seconds after its unwrap, keyed by what a rewrap does not change (the value's row and its data key's issue time).
7. Under `transit`, OpenBao MUST be backed up by scheduled encrypted snapshots, and a restore pairs a PostgreSQL backup
   with a later snapshot. The schedule belongs to the operator's OpenBao installation, under an operator-only policy
   that no platform role holds and that the platform's allow-list test does not read.

## Considered options

1. **Envelope encryption with Transit-wrapped data keys, values in PostgreSQL** — one Transit call per seal and per cold
   open, the value commits with its record.
2. **All values in OpenBao KV** — every write and every restore spans two systems.
3. **Transit `encrypt` of each value** — the value crosses the network on every read.
4. **A key per tenant in Transit** — rotation and policies per tenant; derivation gives the same separation.
5. **The ring delivered from OpenBao KV into files, with per-tenant keys derived in the process by HKDF** — no keys in
   the environment and rotation without a redeploy, with no cache and no OpenBao call on a cold open; but the master
   keys live in the memory of every control plane and engine and OpenBao audits no use of them, which is what
   regulated work chooses `transit` for (design 015, Complexity Tracking).

## Consequences

OpenBao becomes a dependency of every secret seal and every cold open under `transit`; its availability is the
platform's for those paths (a three-node Raft cluster in the runbook). A binary older than 0.3.0 cannot open a
Transit-sealed row, so `transit` is gated by the instance registry, and a sandbox runner of 0.2.x by the trust header
every newer runner sends on each request. Leaving `transit` is the `returning` passage, gated and audited like the way in.

## Evidence

Design 015 (`docs/designs/015-opentelemetry-and-openbao.md`); `crates/infra-secrets/src/lib.rs` (`KeyRing`),
`crates/infra-secrets/src/openbao.rs` (the bounded client's practices, applied to a client per mount), `crates/domain/src/secrets.rs` (`SecretCipher`),
`infra/openbao/` and `infra/helm/templates/openbao-agent.yaml`.
