# Scoped leaf Finish binding — conditional B1b.2b.1 candidate

Status: candidate for bounded review only. No schema, port, executable consumer or authority.
Refs:T014-014..017/040, A1, B0, B1a, B1b.2a. This does not accept the whole nested protocol.

## Four-sentence contract

One ScopedPath member binds immutably to `(tenant, namespace, originalRun, rootOwnerNode, index)`
under the fixed ordinary-leaf V1 source profile; it never follows a chain head or successor.
Its proposed source is the exact persisted `ORDINARY_LEAF_V1` canonical certificate, validated in
the owning transaction against the original run pin and record structure, never a supplied record
or a terminal-run label. Entry retains applicable chain → original Run → selected G `NOWAIT` →
current scheduler guard/postwait clock, rechecks binding, and returns a matching existing Finish
receipt before creation CAS or active-member checks. First application atomically changes only
selected member/header contribution and appends receipt, truthful audit and scoped reference hint;
this documentary contract permits fixture-created membership only and establishes no execution,
quiescence, descendant settlement or production authority.

## Proposed keys and fingerprint

Keep existing group-wide receipt uniqueness `(tenant,G,operationKey)`, `(tenant,G,resultingRevision)`
and `(tenant,G,member)` across member kinds. A distinct scoped-V1 SHA-256 domain encodes tenant,
namespace, G, member, original Run, length-prefixed bounded owner node/index and expected group
revision/epoch/run revision without ambiguity. Operation key scopes receipt lookup; renewable
scheduler credentials and trace metadata are excluded. Same key/different fingerprint conflicts.
Unknown COMMIT retains the original command and reconciles in a fresh owning transaction.

## Bounded schema/query proposal

Existing SQL accepts only CHILD_ROOT; a separate discriminated binding review must add original
run/bounded owner/index, exact qualified-run FK and per-group path uniqueness, without invented
chain/head values. Required bounded probes: applicable chain and qualified original Run lock;
G `NOWAIT`; exact member reread; current scheduler fence; exact canonical certificate primary key;
operation receipt; optional exact holder fact; exact CAS updates. No run history, sibling payloads,
foreign business state or group registry scan. Source lookup uses immutable metadata only.

Retain immutable member tombstones, receipts, certificate and original source without expiry.
The proposed original-run FK references the already held Run. Member/holder FK and reverse active
holder checks stay under guarded G; do not casually add source-row FKs and their referenced-key/
reverse-delete edges. Existing DELETE/TRUNCATE rejection proves retention, not UPDATE preservation.
DDL/relation and uniqueness waits remain independently inventoried.

## Unclosed prerequisites

OrdinaryLeafV1 excludes continuation policy and child/nested controllers. Applicable chain locking
does not establish chain-source acceptance; the classifier does not inspect active capacity topology.
Current flat producer preservation under B1b.2a.1d.3b is recorded separately in
leaf-current-producer-preservation.md; that prerequisite alone grants no executable authority.
Broader relationship closure remains B1b.2c. Release INSERT, C0, creation/handoff, discovery/fairness
and actual scoped schema/query/port/receipt tests remain separate prerequisites. No task above those
dependencies closes from this documentary candidate.

Evidence anchors: `capacity-path-settlement.md` ownership/order; domain `collection_shape.rs`
fixed-profile exclusions; native `leaf_certification.rs` canonical proof/replay; native
`capacity_finish.rs` current CHILD_ROOT-only queries; migration20261211000000 immutability/retention/
SELECT-only grant. This candidate derives from read-only Astra/xhigh drafting at dc212855; no tests
or executable guarantees were supplied by that drafting.
