# SDK and protocol compatibility

The five SDKs in this repository report SDK version `0.1.0` and currently offer worker protocol
`1.4` by default. The workflow document uses the additive
`sagat-octopus-orchestrator.io/v1` schema. Worker protocol `1.5` is a separate capability for
addressed scoped tasks; the production PostgreSQL worker store does not yet advertise it. A build-only
workflow builder can emit a primitive document even when the server refuses to publish that node.

| Client | SDK version | Default worker offer | Current primitive authoring evidence |
| --- | --- | --- | --- |
| TypeScript | 0.1.0 | 1.4 | Shared collection, loop, approval, continuation and declared-state builders |
| Go | 0.1.0 | 1.4 | Same shared builder scenarios |
| Python | 0.1.0 | 1.4 | Same shared builder scenarios |
| .NET/C# | 0.1.0 | 1.4 | Same shared builder scenarios |
| Java | 0.1.0 | 1.4 | Same shared builder scenarios |

The shared build-only scenarios compare each language's canonical v1 document against reviewed
goldens. They do not establish worker execution, server publication, public Query/Update calls or
continuation waits. The production PostgreSQL worker adapter negotiates at most `1.4`, keeps scoped
tasks out of legacy claims, and rejects scoped reports. An explicit test-only opt-in now covers
scoped inline-activity claim, lease, heartbeat, report, retry and lost-lease recovery on real
PostgreSQL. It still rejects scoped scripts, blob reports and several successor node kinds. The
default remains closed so a mixed old-worker deployment cannot receive work it cannot address.

All five clients also expose a typed approval-decision call with local key/reason validation and a
typed receipt. Its shared runtime scenario remains pending because a normally published workflow
cannot yet open an approval request and expose its ID; client unit tests and matching builder
documents are not a substitute for that server round trip.

| Scenario | Expected current behavior | Evidence gate before enabling |
| --- | --- | --- |
| Existing v1 root workflow and worker 1.4 | Accepted with unchanged root identity | Existing execution and SDK suites |
| Additive v1 builder for a new primitive | Canonical document emitted locally | Five-language shared golden |
| Publish or start collection, loop, approval, declared operations or continuation | Rejected by the production parser | Feature-specific transaction, fault, security and e2e suites |
| Worker offers 1.5 to the production PostgreSQL adapter | Negotiates 1.4 | Complete scoped claim/lease/report, recovery and old-minor upgrade tests |
| Old active run after a future primitive-aware server upgrade | Must retain its old replay semantics | Active-run compatibility test, still pending |

Before changing the default worker minor or opening publication, run WP-02–14 in all five SDKs,
old-active-run and old-worker-minor upgrades, and the PostgreSQL crash/restart/lost-broker matrix.
Retain the exact command and results in
[`validation-report.md`](../specs/014-workflow-primitives/validation-report.md). A failure or a
skipped infrastructure test does not count as compatibility evidence. Keep the old reader and
worker negotiation path during rollback until runs written with the new capability drain.
