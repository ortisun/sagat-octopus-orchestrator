# Actual released callback publisher cold cutover

Refs: T033-007, FR-033-004/005/009, SC-033-004. This test reuses the approved 014 released-source
harness after local integration of features 033/034. It proves one explicit operator sequence,
not mixed-version safety, an automatic deployment fence or external exactly-once delivery.

The immutable v0.2.0 publisher must deliver an actual HTTP callback under a dedicated runtime
login. Its native reader decodes the retained subscription and delivery. Pause that fixture
subscription through the native audited store so it creates no unrelated future obligation.
Keep encryption keys in the old process; pass only identifiers and source/receiver digests.

The current coordinator proves the dedicated login and established connections work, revokes
login, terminates only those sessions, proves refusal of existing and fresh access, obtains stop
acknowledgement and joins the actual old process. Only then perform normal current migrations,
start current runtime-role APIs with Redis activation, create a public Redis subscription and
finish a new real gRPC run. The actual delivery worker must XADD once, settle DELIVERED and retain
stable delivery/event/body identity. Exact old subscription/delivery/attempt metadata must remain
unchanged after migration and new delivery. All containers belong to this fixture. Loopback
networking uses the existing test-only sender/standing broker adapter; production egress policy
and credentials have their separate real refusal/TLS tests. No final release checkbox closes
until this profile and combined release gates actually pass.


## Executed combined-tree evidence — 2026-10-27

The real released publisher profile passes after integration with 014 and 034. The final
coordinator passes 1/0/0 in 5.37 s (68.92 s complete command, including compilation); its actual
old phase passes separately in 1.59 s. Both schema boundaries are exact:
20261220000000 → 20261224000000. Current APIs use the runtime role, not the owner pool.
The original old callback settings, delivery and attempts remain byte-equivalent through their
metadata digest; the new stream entry matches the persisted delivery id, event id and exact body.
The second current delivery tick sends nothing. This is fixture acceptance of a cold sequence,
not exactly-once delivery or Redis failover persistence.

All six source-bound released profiles pass independently on one frozen combined source manifest:

| Scenario | Complete runner | Selected phase process |
| --- | ---: | ---: |
| active-data | 215.99 s | 9.46 s |
| applied-lookup | 14.67 s | 12.06 s |
| applied-output-quota | 18.31 s | 15.56 s |
| inflight-worker | 9.02 s | 6.47 s |
| redis-callback-cutover | 68.92 s | 6.68 s |
| scoped-event-cutover | 10.64 s | 6.93 s |

The active-data command includes waiting for the concurrent Clippy build lock; these command
timings are not performance measurements. The common current-source manifest SHA256 is
`9e7753896fe49cc910c4d20ea783c62f84f411c7c50220248052826396bfbe66`.
[Artifacts and provenance](evidence/combined-v03-20261027/artifacts.json) retain each command,
phase transcript, result identity, build/executable hashes and compressed complete source manifest.

Combined regressions: 111 existing messaging tests plus 28 Redis tests pass as the runtime role
(515.09 s complete command); all 109 native Rust SDK tests/doctests pass (56.73 s). The five new
common scenarios — collection-events, collection-timers, collection-control-entry,
collection-control-routing and collection-local-recovery — each pass all six SDKs without blessing:
30 scenario/language combinations, unchanged reviewed goldens. This is focused five-scenario
acceptance, not a new full 65-scenario aggregate claim. Strict workspace/all-target Clippy passes
(239.75 s), alongside format, architecture (24 crates), agent configuration, 16 Python profile
checks, vendor provenance and workflow lint. Studio API drift, typecheck and lint also pass.
Primitive contracts, SDK registry and developer reference checks pass; the separate full SDK
aggregate is deliberately ignored in that contract-only command and is not counted as a pass.

The first combined active-data run failed because its exact last-migration expectation still
named the pre-Redis schema. Both old-data/report coordinators now require the exact Redis migration
instead. An intermediate compile of the new schema observation failed because a JSON comparison
inferred i32; the check now decodes i64 explicitly. Both original failures and their source/build
identities are preserved under failures/. Neither is described as a passing runtime scenario.
One bounded review found a missing Unix module gate, now fixed; its formatting claim did not
reproduce (`cargo fmt --all -- --check` passed before that fix). No Windows runtime is claimed.

Reproduce with the standard private target and owned Docker fixture environment:
`python3 scripts/test-release-upgrade-profile.py --scenario redis-callback-cutover`.
The final integrated release CI, remaining 014 nested-capacity/fault/resource work and publication
remain open. T033-007 stays unchecked for those release gates; its cold-cutover prerequisite is
now evidenced rather than inferred from the default-off flag.
