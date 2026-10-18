# First native scoped Finish test draft

Target integration point:

```rust
// tests/integration/workflow-primitives/leaf_certification_native.rs
#[path = "scoped_leaf_finish_native.rs"]
mod scoped_finish;
```

The draft intentionally has four bounded real-PostgreSQL tests:

1. Genuine public `Flow` sources are certified by the native certifier, source INSERT is revoked,
   a scoped member is finished through `PgScopedLeafFinishStore`, and the original exact result is
   replayed after a second genuinely certified member changes the same header and later work fails
   the original Run. Source/certificate/Run/journal/step state and replay side effects are compared.
2. One genuine source drives all four A1 outcomes: selected holder, nonholder, vacant, and last
   member. Nonselected identities are explicit storage fixtures and are compared byte-for-byte via
   JSONB snapshots; the test makes no source-closure claim for them.
3. Source absence, current scheduler fence, flat-context membership, group/epoch/Run CAS order, and
   inactive selected state all refuse with zero receipt/audit/hint rows and atomic snapshots.
4. `i64::MAX` group revision refuses the required advance with `CounterExhausted` and leaves the
   exact capacity, source, certificate, Run, journal, steps, audit, and outbox state unchanged.

Every adapter pool is created explicitly as `sagat_app` and proves `current_user`. Each disposable
database grants only the four scoped receipt extension columns while preserving a common existing
receipt column grant. The native source INSERT is proven absent before Finish. Positive paths do not
disable triggers or corrupt schema. Applied rows check the full scoped receipt identity/counters,
the linked audit, the linked canonical scoped hint, and the repository AsyncAPI validator.

Expected first RED: the integrated tree currently has no exported
`sagat_infra_postgres::PgScopedLeafFinishStore`, so registration should compile-fail at that import
before the adapter is implemented. Root should install the file, register the child module, and run
the focused real PostgreSQL test target to record the natural RED.

Deliberately remaining later units from the approved contract:

- closed receipt/source decoder corruption, exact qualification and cross-kind key conflict matrix;
- metadata-only privilege/SQL probes and diagnostic canaries;
- held-lock barriers, scheduler wait/expiry, concurrent duplicate/member order, rollback injection,
  deferred COMMIT failures and source-writer ordering;
- unknown COMMIT reconciliation, cold-store/process recovery, cancellation and full OTLP outcomes;
- production caller/authority, released grants, retention/mixed-writer gates and non-flat semantics.

No test or formatter was run while drafting, per the delegated read-only constraint.
