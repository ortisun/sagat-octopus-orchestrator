# Public collection event waits — v0.3 checkpoint

Refs: T014-014–017/040/043/044, FR-014-004/005/014/015/017. Existing accepted feature-014 scope;
the eight canonical tasks remain open for nesting/capacity and final resource/release acceptance.

The public collection body guard accepts `waitEvent` among its one-to-eight executable nodes,
including an event entry. Existing graph closure, finite budgets, schema, namespace and worker
rules remain in force. The same captured item/index correlation, canonical event output, exact
instance receipt, lookback, native deadline and stop/drain transactions used internally now run
through ordinary public publication. No migration, grant, error code, wire field or worker minor
is introduced. Root/chain signal addressing does not become scoped signal addressing.

`collection_events` has six actual runtime-role PostgreSQL tests: out-of-order event ingress with
ordered aggregation and exact retained values; activity→event lookback after inbox dispatch;
handled deadlines without worker tasks; cancellation and late-event refusal; unhandled timeout
with both original activities compensated; and control→event→control→activity with each worker
seeing only its own event value. All end in native replay and released iteration slots. The
final six-case invocation passed6/0/0 in16.20s (67.66s including compilation). The preceding
five-case result is separate, not an aggregate11-case claim.

The first three public tests failed at the expected unsupported-body publication guard. An
intermediate correction still rejected them because `timeoutMs` also populates Node.timeout;
the event-kind guard now uses the already validated event deadline rather than requiring its
absence. The three-case correction passed, then the expanded five- and six-case gates passed.
No failure assertion was removed.

## Actual released dispatcher cutover

The source-verified v0.2.0 fixture performs a real root-event delivery through its actual old
EventDispatcher under a distinct inherited least-privilege database login. The current
coordinator proves that login and an established connection work before revocation, executes
NOLOGIN and terminates its actual sessions, then proves both an old established connection and
a new authentication attempt fail. It receives the explicit stop acknowledgement and joins
the real released process before normal migration/current startup/publication. The same database
then admits and completes two new scoped waits using current sagat_app services; old and new
native replay pass. Source/overlay/coordinator and copied executable identities are bound by
the existing runner. Its first preflight reached publication and failed at the closed guard;
the completed profile passed1/0/0 in5.36s (31.13s runner), including the separate nested old
phase1/0/0 in1.39s. These nested counts are not added.

This proves one explicit cold-cutover sequence, not automatic fencing or mixed-version safety.
Operators must revoke/terminate every distinct old deployment login and stop/join all old
control-plane/engine instances. A shared unchanged login supplies no generation fence. Old SDK
workers use their separately supported protocol path; this does not authorize old execution
binaries against new definitions. Rollback requires a compatible current reader while new
unsupported records remain.

## Common SDK contract

All five existing SDKs execute `collection-events`, authoring identical canonical IR and producing
identical inspected observations: duplicate event admission, event-entry completion, activity
lookback, handled timeout, qualified worker calls, ordered scope results and persisted server
state. The no-bless final invocation passed1aggregate/0failed in28.12s. Rust must join this new
case after integration with feature034; that all-six result is not claimed here.

The first observation exposed two fixture assumptions: journals imposed an order on independent
concurrent iterations, and the deadline case reused a key with a retained matching event. The
shared case now uses concurrency1 and distinct deadline keys; native tests retain concurrency2.
The next run had five equal observations and failed only for missing reference files. After
reading the canonical definitions, complete journals, two worker calls, ordered results and
persisted projections, references were recorded and rerun without blessing. No journal field
or server assertion was weakened.

Commands and immutable receipts are retained under
[evidence/public-scoped-events-20261027](./evidence/public-scoped-events-20261027/README.md).
The execution ledger records subsequent static/contract/old-profile regressions separately.
General loops, approvals/children/nesting/capacity, full fault/security/resource matrices and
integrated six-SDK release acceptance remain independent work.
