# Targeted re-read of design 044 for approval: security, fourth pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 334676bc72471ac2048fc1f52bdf605989ca76a5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since 08580114. Previous findings 3 and 4 are closed, 1 and 2 partly closed. Every finding below was fixed in the next
commit.

## Major

1. A ceiling refusal records nothing, so the lease expiry after it still counts: record `steps.unavailable_at` and never
   count an expiry after `command_value_ceiling`.

## Nit

2. The run's cap across attempts has no answer and contradicts "never count against the next attempt".
3. A re-registered session keeps its id; record and match the token hash and namespace.
4. One namespace can fill the tenant's 1,024: a namespace share.
5. `UPLOADING` and unconfirmed `DELETING` rows in the counts; the hold must be a conditional update.

Counts: Blocker 0, Major 1, Nit 4
