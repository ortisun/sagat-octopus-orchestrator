# ADR-0152: OpenBao Transit holds the platform's keys, secret values are sealed in PostgreSQL under tenant-derived wrapped data keys, and each custody has its own mount, token and policy

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with design 015, accepted by the agent preparing design 015 for build under the owner's
delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-26-119. The owner may overrule it.
**Refines / supersedes:** refines ADR-0006 (secrets are references resolved at execution time), ADR-0033 (the wrapped
key is classified `secret`), ADR-0035 (feature 017's OpenBao mount is one custody among several) and ADR-0066 decision 1
(the platform's policies may grant no path of a mount that holds feature 031's client keys); supersedes none.
**Enforced by:** the cross-tenant unwrap test, the rotation and rewrap test, the migration test under load and the
outage test on a real OpenBao (Task 015-19); the policy test that refuses a grant outside the platform's mounts (Task
015-17); the canary test (Task 015-20); the start-up refusals of Requirement 015-010 (Task 015-18). Until they exist:
review only.

## Context

Today a key ring in an environment variable seals every secret, an Ed25519 key in another signs scripts, and a 32-byte
file authenticates feature 017's receipts with no rotation; OpenBao holds only feature 017's database credentials
(design 015, The Problem).

## Decision

1. A deployment MUST choose a key authority, `local` or `openbao`; both are supported. Under `openbao` no platform key
   MAY exist outside OpenBao, and a process MUST refuse to start if a local key is also configured, except in the
   `migrating` mode.
2. Under `openbao` a secret version MUST be sealed with a fresh data key from Transit key `sagat-secrets`, derived with
   the tenant id as context, and stored with its wrapped data key in PostgreSQL. The value MUST NOT be sent to OpenBao.
3. Rotation MUST need no restart: new seals use the newest key version; a bounded, resumable pass rewraps stored data
   keys with `transit/rewrap`; raising `min_decryption_version` is the operator's step.
4. Scripts MUST be signed with Transit key `sagat-scripts` and verified by runners with its public keys; feature 017's
   receipts MUST be authenticated with Transit key `sagat-intent` and name the key version.
5. Each custody MUST have its own mount, token and policy (`sagat-transit`, `secret/sagat/credentials`, design 047's
   OAuth mount). No platform policy MAY grant a path outside these mounts; the bootstrap refuses one that does.
6. Without OpenBao nothing is sealed or opened by other means. Opening MAY use an unwrapped data key from a cache of at
   most 4,096 keys held at most 300 seconds each.

## Considered options

1. **Envelope encryption with Transit-wrapped data keys, values in PostgreSQL** — one Transit call per seal and per cold
   open, the value commits with its record.
2. **All values in OpenBao KV** — every write and every restore spans two systems.
3. **Transit `encrypt` of each value** — the value crosses the network on every read.
4. **A key per tenant in Transit** — rotation and policies per tenant; derivation gives the same separation.

## Consequences

OpenBao becomes a dependency of every secret seal and every cold open under `openbao`; its availability is the
platform's for those paths (a three-node Raft cluster in the runbook). A binary older than 0.3.0 cannot open a
Transit-sealed row, so `openbao` is gated by the instance registry. Rolling back is a documented re-seal.

## Evidence

Design 015 (`docs/designs/015-opentelemetry-and-openbao.md`); `crates/infra-secrets/src/lib.rs` (`KeyRing`),
`crates/infra-secrets/src/openbao.rs` (the bounded client reused), `crates/domain/src/secrets.rs` (`SecretCipher`),
`infra/openbao/` and `infra/helm/templates/openbao-agent.yaml`.
