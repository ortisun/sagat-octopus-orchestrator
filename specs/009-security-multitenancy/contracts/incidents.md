# Supervisor denial incident contract

Trace: FR-009-004, FR-009-005, FR-009-008, NFR-009-DUR, NFR-009-SEC, D51.

`POST /v1/script-runner/incidents` is the synchronous durability boundary between the sandbox
supervisor and the untrusted script. The supervisor may return a policy denial to script code only
after this operation returns `204`.

## Request

The request has one tagged target and one category:

```json
{
  "target": {
    "kind": "production",
    "namespaceId": "019b5d6e-0000-7000-8000-000000000001",
    "runId": "019b5d6e-0000-7000-8000-000000000002",
    "stepId": "019b5d6e-0000-7000-8000-000000000003",
    "attempt": 1,
    "generation": 2,
    "leaseToken": "…"
  },
  "category": "NOT_ALLOWLISTED"
}
```

```json
{
  "target": {
    "kind": "test",
    "namespaceId": "019b5d6e-0000-7000-8000-000000000001",
    "testRunId": "019b5d6e-0000-7000-8000-000000000004",
    "leaseToken": "…"
  },
  "category": "EGRESS_DISABLED"
}
```

The closed categories are `EGRESS_DISABLED`, `UNSUPPORTED_SCHEME`, `INVALID_HOST`,
`PORT_NOT_ALLOWED`, `NOT_ALLOWLISTED`, `FORBIDDEN_ADDRESS` and `NO_ADDRESS`. Unknown fields and
unknown categories are validation errors. The request has no actor, digest, reason, hostname, URL,
headers or payload. Authentication supplies tenant and actor; persisted execution state supplies
the trusted resource attribution.

## Production validation and transaction

The control plane requires `script:execute` and takes locks in this order:

1. the binding advisory lock for authenticated subject and requested namespace;
2. the tenant-scoped run row;
3. the step row identified by the stable `stepId`;
4. the audit deduplication advisory lock used by `insert_audit`.

Before inserting the audit row it proves that the run and step belong to the authenticated tenant
and requested namespace; the activity is a script activity with a ready artifact; attempt,
generation and lease-token hash match; the database clock is before the lease expiry after the step
lock was acquired; `leased_by` identifies a live worker session of the authenticated subject; and
the current binding covers the actual step activity. Any mismatch is `STALE_ATTEMPT` and writes no
incident.

## Test-run validation and transaction

The control plane locks the tenant-scoped `script_test_runs` row and requires `RUNNING`, the
requested namespace, the lease-token hash and an expiry later than the database clock after the
lock was acquired. It derives the script and attempt number from that row.

## Commit and retry semantics

The audit action is `script.violation`. Its resource is a deterministic production
run/step/attempt/generation or test/script/run/attempt key, and its reason is exactly the closed
category. `insert_audit` serializes and deduplicates identical categories. At most seven category
rows can exist for one attempt, so the existing cap of 100 distinct script violations cannot hide
an acknowledged category.

`204` means the audit transaction committed, including when the category was already committed.
If storage fails or the request deadline expires, the caller receives a typed unavailable error and
the supervisor does not return a policy denial. A retry after a lost response is safe and cannot
duplicate the row. A retry after lease expiry may be rejected: the earlier committed row remains.

This operation changes no run or test state and writes no outbox event. It does not claim that every
unacknowledged attempted syscall is durable, and it does not promise exactly-once external effects.

The later terminal execution/test record retains its legacy diagnostics. If category evidence for
that exact attempt already exists, the adapter omits URL-bearing `fetch …` terminal audit duplicates;
secret and host-call-limit diagnostics continue through the legacy audit path.
