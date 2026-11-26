# ADR-0152: OpenBao Transit holds the platform's keys, secret values are sealed in PostgreSQL under tenant-derived wrapped data keys, and each custody has its own mount, token and policy

**Status:** Proposed (2026-11-26). Returned from Accepted the same day: design 015's first review round found decisions to revise (its architecture findings are open); it is accepted with design 015's approval
**Decided by:** agent proposal with design 015, accepted by the agent preparing design 015 for build under the owner's
delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-26-119. The owner may overrule it.
**Refines / supersedes:** refines ADR-0006 (secrets are references resolved at execution time), ADR-0033 (the wrapped
key is classified `secret`), ADR-0035 (feature 017's OpenBao mount is one custody among several) and applies ADR-0066 decision 1 (feature
031's customer keys live outside the platform's OpenBao); supersedes none.
**Enforced by:** the cross-tenant unwrap test, the rotation and rewrap test, the migration test under load, the
outage test and the paired restore test on a real OpenBao (Tasks 015-19, 015-24); the signature-binding tests (Task 015-15); the exact allow-list policy test (Task
015-17); the canary test (Task 015-20); the start-up refusals of Requirement 015-010 (Task 015-18). Until they exist:
review only.

## Context

Today a key ring in an environment variable seals the secrets stored in PostgreSQL, an Ed25519 key in another signs
scripts, and a 32-byte file binds feature 017's provisioning intents with no rotation; OpenBao's KV engine holds the
values of feature 017's `openbao` custody (design 015, The Problem).

## Decision

1. A deployment MUST choose a key authority, `local` or `transit`; both are supported. Under `transit` no platform key
   MAY exist outside OpenBao, and a process MUST refuse to start if a local key is also configured, except in the
   `migrating` mode.
2. Under `transit` a secret version of `transit` custody, which carries no key id so that older readers fail closed,
   MUST be sealed with a fresh data key from Transit key
   `sagat-secrets`, derived with the tenant's context, and stored with its wrapped data key and the key's version in
   PostgreSQL. The value MUST NOT be sent to OpenBao. The associated data MUST name the custody and never the Transit
   key's version. Feature 017's KV custody stays as it is.
3. Rotation MUST need no restart and MUST NOT outrun the backups: new seals use the sealing version recorded in
   PostgreSQL, advanced only once a snapshot holds it; a bounded, resumable pass rewraps stored data
   keys with `transit/rewrap` and reports every row it could not rewrap; raising `min_decryption_version` first raises a
   sealing floor that the database enforces, then counts. A re-key command issues new data keys after
   a compromise of a token.
4. Scripts MUST be signed with Transit key `sagat-scripts` over the message that binds the tenant, the digest and the
   language; runners MUST get the public keys from the control plane, never from OpenBao. Feature 017's intents MUST be
   bound with Transit key `sagat-intent` over their metadata and the digest of the value, never the value.
5. Each custody MUST have its own mount and policy, and each process kind its own token per custody it uses. Every
   policy MUST be an exact allow-list; the bootstrap and a test refuse any other grant. Feature 031's customer keys MUST
   live outside the platform's OpenBao.
6. Without OpenBao nothing is sealed or opened by other means, on every read path including the scan of a step's
   output. Opening MAY use an unwrapped data key from a cache of at most 4,096 keys, 512 per tenant, each at most 300
   seconds after its unwrap.
7. Under `transit`, OpenBao MUST be backed up by scheduled encrypted snapshots, and a restore pairs a PostgreSQL backup
   with a later snapshot.

## Considered options

1. **Envelope encryption with Transit-wrapped data keys, values in PostgreSQL** — one Transit call per seal and per cold
   open, the value commits with its record.
2. **All values in OpenBao KV** — every write and every restore spans two systems.
3. **Transit `encrypt` of each value** — the value crosses the network on every read.
4. **A key per tenant in Transit** — rotation and policies per tenant; derivation gives the same separation.

## Consequences

OpenBao becomes a dependency of every secret seal and every cold open under `transit`; its availability is the
platform's for those paths (a three-node Raft cluster in the runbook). A binary older than 0.3.0 cannot open a
Transit-sealed row, so `transit` is gated by the instance registry. Rolling back is a documented re-seal.

## Evidence

Design 015 (`docs/designs/015-opentelemetry-and-openbao.md`); `crates/infra-secrets/src/lib.rs` (`KeyRing`),
`crates/infra-secrets/src/openbao.rs` (the bounded client's practices, applied to a client per mount), `crates/domain/src/secrets.rs` (`SecretCipher`),
`infra/openbao/` and `infra/helm/templates/openbao-agent.yaml`.
