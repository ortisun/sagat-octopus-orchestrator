# Fresh binding authority refusal C2

## Scope

Incremental correction for the single fresh binding authority integration test in `byte_ceilings.rs`. The repository was treated as read only.

## Focused delta

- Measures the matching binding rows' physical retained content before withdrawal and requires the native quota trigger to refund that exact amount.
- Preserves every other retained usage field in the full snapshot oracle.
- Identifies the native unbind audit through the HTTP response trace and validates every stored audit column against the request, tenant, namespace, actor, action, resource, reason, and request time window.
- Removes only that exact new audit from the comparison and restores only the seventeenth step's lease expiry and update timestamps before requiring full snapshot equality.
- Removes debug formatting from the lease state and uses fixed message boolean checks for lease hashes and snapshots.

## Source basis

The worker binding quota trigger measures the UTF-8 octet lengths of `pattern` and `subject`. The native unbind transaction deletes matching bindings, expires leases, closes sessions, appends the audit, and commits the quota delta together.

## Verification

`git apply --check` passed against the current C1-applied source. `git diff --no-index --check` passed. Per task constraints, no formatter, build, or test command was run.
