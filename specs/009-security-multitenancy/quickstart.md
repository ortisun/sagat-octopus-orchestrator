# Validation quickstart: Security, isolation and multi-tenant governance

Everything below was run against this repository. The pieces that are **not** built are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and Docker (the suites start throw-away PostgreSQL 16 containers through Testcontainers; no port is published on the host).
- Node 22 (`npx`) only for the contract linters in CI.

## Validation flow

1. **The user stories and the release gate battery**, against real PostgreSQL, a fake OIDC provider and real sockets:

   ```sh
   cargo test -p sagat-tests --test e2e_security_multitenancy            # US01 (worker side), US02, US04 through REST and gRPC
   cargo test -p sagat-tests --test contract_rest_security_multitenancy  # all 18 governance operations vs the OpenAPI; the cross-tenant battery
   cargo test -p sagat-tests --test contract_grpc_security_multitenancy  # TLS and mutual TLS with real handshakes
   cargo test -p sagat-tests --test integration_security_multitenancy    # quotas, bindings, secrets, egress against PostgreSQL
   cargo test -p sagat-tests --test fault_security_multitenancy          # connections killed around quota, secret and binding transactions
   cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-secrets -p sagat-execution-engine -p sagat-control-plane
   ```

2. **The battery of the gate** (spec SC-009-001). One suite per class; a release with any failing test is not shippable:

   | Class | Suite | What it does |
   |---|---|---|
   | Cross-tenant | `contract/009.../rest/battery.rs`, `e2e/009.../us01.rs` | Attacks every implemented REST operation with a valid administrator token of *another* tenant, with valid bodies, naming the victim's real ids; each must answer exactly like a missing resource, leak no identifier, and leave the victim's data byte-for-byte unchanged. Then the same story over gRPC with a **leaked, valid** lease token. The suite is driven by the OpenAPI document, so a new endpoint without an entry fails it. |
   | SSRF | `integration/009.../egress.rs` | The worst policy an administrator could write (every dangerous network named outright) still cannot reach loopback, link-local, the metadata addresses, multicast, reserved space, IPv4 hidden in IPv6, or numbers in disguise; the guarded client built from the *stored* policy refuses a loopback server that the policy names. |
   | Secret leak | `e2e/009.../us02.rs`, `integration/009.../secrets.rs` | A canary value is stored, resolved and echoed by a careless activity; it must appear in no table of the database (the ciphertext included), no log line at any level, no response of any operation, and the echo fails the step. |
   | Quota racing | `integration/009.../quotas.rs`, `fault/009.../faults.rs` | Replicas with their own pools racing for the last places never overshoot, with and without connections being killed. |
   | Webhook replay | not built | Feature 004 (G36). |
   | Sandbox escape | not built | Feature 005 (G35). |

3. **Failure injection.** `fault_security_multitenancy` kills database connections while quotas are judged (exactly the acknowledged runs exist, never more than the limit), while secrets are written (every committed version has exactly one audit record), while secrets are resolved (never a wrong value, and the attempt that was handed values has exactly one audit record however often it asked) and while a binding is replaced (no session survives a binding that does not cover it).

4. **The benchmark** (cost of exact counting and of the new worker paths):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/security-multitenancy.md \
     cargo test -p sagat-tests --test perf_security_multitenancy --release -- --ignored --nocapture
   ```

5. **Operating it:** `docs/runbooks/security-governance.md`.

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Prevent cross-tenant access | Tenant A with a valid token tries tenant B's run: reading, manipulation and discovery denied | `battery::every_implemented_operation_resists_an_administrator_of_another_tenant` (39 operations; mutation-checked), `us01::a_leaked_lease_token_of_another_tenant_opens_nothing`, `bindings::each_binding_operation_audits_a_probe...`, `secrets::each_secret_operation_audits_a_probe...` |
| US02 Grant a capability to a script | An author with access to a staging secret publishes; the worker receives only a temporary capability and the authorized secrets | `us02::an_author_with_access_publishes_and_the_worker_receives_only_the_authorized_secret` (no list entry: refused; on the list: published; the worker gets only the declared secret, only while it holds the lease), `secrets::losing_access_after_publishing_stops_the_next_resolution...`. The *script* half (an untrusted runner) is feature 005. |
| US03 Block unauthorized egress | A script tries a forbidden endpoint: the network is blocked and the incident audited | Platform side: `egress::*` (default-deny, no policy opens the forbidden addresses, refusals audited with category and without path or query). The sandbox that runs the script is feature 005 (G35). |
| US04 Enforce quotas per namespace | A namespace exhausts its limit: new requests get quota exceeded without affecting other tenants | `us04::a_full_namespace_answers_quota_exceeded_and_other_tenants_carry_on`, `quotas::the_quota_of_one_tenant_never_affects_another`, `quotas::replicas_racing_for_the_last_places_never_overshoot_the_quota`. Builds and tasks as resources: 005 (G37). |

## Edge cases of the specification

| Edge case | Where it is handled |
|---|---|
| Tenant token spoofing | The tenant is taken from the verified token and the database, never from a header or a body (feature 001); the battery sends a valid token of another tenant against the victim's ids |
| Secret in a stack trace | No value in any error or `Debug`; failures carry codes; `us02::the_secret_reaches_no_response_log_row_or_error_on_the_way` captures every log line; a worker's own logs are outside the platform (G41) |
| Container with undue privileges | Feature 005 |
| Confused deputy across namespaces | A secret name resolves only in the run's own namespace; a binding is per namespace; the publisher's standing is checked at resolution |
| ACL race when renewing a token | The publisher is re-checked at every resolution and the binding at every report (D43, D46); `bindings::a_binding_that_disappears_stops_a_lease_holder...`, `secrets::losing_access_after_publishing...` |
| Quota with concurrent replicas | Exact count under an advisory lock in the creating transaction (D42); `quotas::replicas_racing...`, `fault_security_multitenancy` |

## Security review of the implementation (T009-035)

A read-only review of the whole feature diff (a separate agent that had not written the code) found no query without a tenant filter and no secret in a log, error or table, and eight groups of findings. Every one was checked against the code before anything was changed; all were real. Fixed ones have a test that fails without the fix (each guard was weakened in turn: about forty single-line changes, three of which survived the first test and led to a stronger one).

| # | Severity | Finding | Verdict and fix | Evidence |
|---|---|---|---|---|
| 1 | High | A tenant's administrator holds `quota:manage` and could set any limit up to 1 000 000 000, removing the cap meant to protect the other tenants (US04, FR-009-006) | Real. A tenant's limits are bounded by a **ceiling** the platform grants (default, or granted by an operator with `control-plane quota-ceiling`); enforcement is `min(setting, ceiling)`; the API cannot raise it (D52) | `quotas::a_tenant_administrator_cannot_raise_any_limit_above_the_ceiling...`, `an_operator_grants_a_ceiling...`, `withdrawing_or_lowering_a_ceiling...`, `the_ceiling_command_...`, `us04` (422 above the ceiling), the `conformance` scenarios, `cli.rs` |
| 2 | High | Any client could fill the 2048 connection places: sockets that never speak, a TLS handshake that stalls, peers that answer pings and make no call | Real (tonic 0.14 has no handshake timeout unless set). First-byte and idle deadlines, a TLS handshake timeout, a maximum connection age and a per-address cap (D55) | `grpc/connections.rs` (5 tests over real sockets), `tls::a_handshake_that_starts_and_stalls...`, `limits::tests`, `execution-engine/tests/config.rs` |
| 3 | Medium | `register` checked the binding and then stored the session outside the revocation lock, so a revocation in between left a session the binding no longer covered | Real. `create_session` takes the binding lock and re-reads the binding in its own transaction (D53) | `bindings::the_store_refuses_a_session_that_the_binding_no_longer_covers`, `registering_while_a_binding_is_narrowed_never_leaves...` (concurrent) |
| 4 | Medium | The scheduler failed a child start refused by a *quota* with the code written for the *tree* limit | Real. `CHILD_QUOTA_EXCEEDED` is its own failure; waiting was rejected because it would starve other tenants (D54) | `quotas::child_runs_count_against_the_quota...`, `children::a_fan_out_bomb_stops_at_the_tree_budget_*` |
| 5 | Medium | A secret's access list decides who may *reference* it, not which workers *receive* it | Real design gap, **not fixed**: delivery is decided by bindings. Documented with the guidance to bind fleets narrowly (G48) | Open |
| 6 | Medium | Every `ResolveSecrets` call added an audit row, so a compromised worker could grow the append-only table at its rate budget; `egress.denied` keys on an attacker-chosen host | Real for secrets: written once per run, node and attempt (D56). `egress.denied` has no consumer yet; **pending decision** for 004/005 (G50) | `secrets::asking_again_within_an_attempt_is_answered_but_written_down_once...` |
| 7 | Low-medium | The output scan compared only the current version of a secret (a rotation mid-attempt hid the value handed out), and `fail` stored its error code unscanned (a credential fits a code) | Real. Every kept version is searched, and so is the failure code (D57) | `secrets::a_value_handed_out_before_a_rotation_is_still_recognised...`, `a_secret_reported_as_the_reason_for_failing...` |
| 8 | Low | Smaller points | See below | See below |

The smaller points of finding 8:

| Point | Verdict and fix | Evidence |
|---|---|---|
| The control plane receives secret values over plaintext HTTP with no equivalent of the engine's guard | Real. Outside dev it needs `SAGAT_API_ALLOW_PLAINTEXT_SECRETS=true` to say TLS is in front | `control-plane/tests/config.rs` |
| An expired lease that was not yet reclaimed still resolved secrets | Real. The resolution needs an unexpired lease (reports still accept a late completion) | `secrets::the_capability_ends_with_the_lease...` |
| An administrator holds `worker:bind` and `worker:execute`, so can read any declared secret by binding itself | Real, **accepted**: a tenant administrator is trusted with the tenant's secrets, and every step is audited (G49) | Open |
| `replace_binding` updated `steps.leased_by` with no index, while holding locks | Real. Partial index `steps_leased_by_idx` | `bindings::releasing_the_leases_of_closed_sessions_is_an_index_lookup...` |
| G46 said namespaces of one tenant do not wait for each other; they share the tenant lock | Real. G46, `data-model.md` and the benchmark were corrected, and the benchmark has a two-tenant case | `tests/performance/reports/security-multitenancy.md` |
| `reseal` stopped at the first version no key opens, for every tenant | Real. It steps over it, moves the rest and fails at the end, saying how many | `secrets::one_version_that_no_key_opens...`, `the_cursor_steps_over...` |
| Any developer could list every secret name, which made "a publisher cannot probe which names exist" moot | Real. Anyone but an administrator lists only the secrets whose access list names them | `secrets::a_developer_lists_only_the_secrets_they_may_use...` |
| The publish check costs a lookup per declared name, bounded only by the document | Real. A definition declares at most 128 distinct secrets | `crates/domain/tests/workflow.rs::declared_secrets` |

## Gates

- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `python3 scripts/check-architecture.py`, `cargo test --workspace --locked`, `cargo audit`, the OpenAPI lint and AsyncAPI validation, the protobuf compatibility baseline (`cargo test -p sagat-api-grpc --test compatibility`; `worker.proto` only grew), the compose smoke test.
- During development each guard of this feature was weakened in turn (a lock removed, a check inverted, an audit write skipped, a counter deleted) and a test had to fail; survivors became new tests. This was done with a throw-away script that is **not** part of the repository, and no tally is kept, so it is evidence of method, not a gate that CI repeats.
- Tasks are ticked only with a test or artifact named next to them (`tasks.md`).

## Not built (so nobody assumes it)

The sandbox and its isolation (cgroups, namespaces, seccomp, pools) (G35); inbound webhook signature and replay controls (G36); quotas for builds, storage, logs, retention and event rate (G37); signing and SBOM of customer artifacts (G38); custom roles (G39); a KMS or HSM behind the key ring (G43); certificate-bound tokens (G44); a Helm chart with NetworkPolicy and pod security (G45); product-decided quota values per tier (G42); restricting a secret to the activities or fleets that may receive it (G48); separating the power to bind from the power to read (G49, needs custom roles); bounding `egress.denied` records when something calls the guarded client (G50).
