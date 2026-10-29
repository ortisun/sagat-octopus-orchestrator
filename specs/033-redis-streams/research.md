# Decisions and evidence — feature033

D033-001: owner explicitly selects optional Redis internal transport and additional Redis callback
transport for0.3. Existing gRPC/PG semantics, NATS default compatibility and HTTP callback remain.
D033-002: XADD* preserves stable application ids as fields, allowing physical duplicates after an
ambiguous acknowledgement; no application exactly-once effect or failover persistence claim.
D033-003: reuse current delivery transaction/fence/rate/retry/DLQ/redrive/secret/egress paths.
D033-004: outbound broker kinds are independent from inbound connector kinds; Redis ingress is excluded.
D033-005: operator-controlled internal hints may trim under a bounded MAXLEN; callback publishing
never trims customer streams. Redis AOF/replication are operational prerequisites, not PG commit substitutes.

Sources consulted2026-10-27:
- https://redis.io/docs/latest/commands/xadd/ — auto stream ids and field/value acceptance.
- https://redis.io/docs/latest/develop/data-types/streams/ — persistence, replication and delivery limitations.
- https://redis.io/docs/latest/develop/clients/rust/connect/ — async redis-rs and rustls configuration.
- https://docs.rs/redis/1.7.1/redis/ — pinned client APIs/features.

Observed test/command evidence belongs in validation-report.md, not inferred from this design.


D033-006: internal selection is a deployment-wide cold-cutover contract. Pending rows follow the
new selected backend; confirmed rows stay confirmed. Physical cross-broker duplicates are possible
after an ambiguous old attempt. Mixed rolling transports are explicitly unsupported.
D033-007: callback Redis creation requires a separate explicit activation gate after every old
reader/writer has stopped/upgraded. Additive/check/down migration is not mixed-reader safety.
D033-008: one immutable DB/auth/TLS connection lifecycle per attempt; no pools, mutable SELECT,
Cluster/Sentinel client or redirect following. Current policy/secrets are reevaluated each attempt.
D033-009: exact MAXLEN for internal bounds; binary Redis field names/types are in contracts.md.
D033-010: stable redis-rs1.7.1 was verified via cargo info/local downloaded source. Redis image
8.6.8-alpine was verified in the official registry; release8.6.8 contains a security fix:
https://github.com/redis/redis/releases/tag/8.6.8 . No prerelease client is used.

First adversarial review identified mixed transport/old-reader activation, connection-local SELECT,
missing wire/auth/config/retention definitions, dedup wording and multiplexed timeout hazards.
contracts.md specifies the boundaries. Activation/cold rollout still need their named actual tests.


Implementation review reconciliation (2026-10-27):
- Valid/actionable: internal port0/unbounded URL/subsecond constructor deadline; two real RED
  controls in redis-internal-review-red. Fix URL≤16KiB, port>0 and deadline≥1s.
- Valid/actionable: refused Redis replies mapped to unavailability. Preserve the existing
  PublishError codes REJECTED/BROKER_UNAVAILABLE; no new error code is introduced.
- Valid/actionable: Compose omitted the documented timeout variable; pass it through explicitly.
- Valid/actionable: optional callback credentials were contradicted by process boot wiring that
  omitted outbound management/workers without a secret backend. Actual process REDs show404 and
  no attempt; use the existing default fail-closed SecretReader for anonymous deliveries.
- Valid/actionable: refuse a Redis secret reference without TLS during management, require the
  canonical JSON content type, and cap DNS candidate connections to one per attempt.
- Test correction: a superseded attempt's DeliveryRound reports its attempted retry verdict, while
  PostgreSQL fences the durable update. The race test checks the actual delivered row and single
  attempt record instead of inventing a new DeliveryRound meaning.
Single-model review only, following the owner's existing decision to skip external review.


D033-011: a checked Redis resolution returns typed Denied/Network failures and runs once per
attempt; policy/store/DNS errors do not all become policy denials. The owned connection receives
one selected checked address and retains the original hostname for TLS. No parallel socket fan-out
or automatic within-attempt fallback is promised. RED controls and the final28-case gate prove the
selection and classification. The final third callback review found no new actionable issue.
