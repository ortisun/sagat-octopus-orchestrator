# Collection result composition, disclosure and late Finish rollback

Base `3710a5b7` plus the 1,121 frozen native inputs retained here. The actual source later
lands in `ce6560c5`/`54b9a066` and the classification test increment; commits do not replace
the byte hashes. The unchanged source manifests cover all gates below and the separate
[old/current discovery gate](../collection-discovery-integration-20261018/reproduce.md).

Use the same owned PostgreSQL 16.15 server, loopback 65117, coordinated target and environment
as [the preceding integration evidence](../collection-finalization-integration-20261018/reproduce.md).
Every command, exit code, UTC start and wall time is in its corresponding JSON/TXT record.

## Real results

| Selection | Default | Runtime flag |
| --- | --- | --- |
| collection_end_result | 4/4, 75.48s including 62s build | 4/4, 10.80s |
| collection_child_end_result_reaches_its_parent_end | 1/1, 3.15s | 1/1, 2.71s |
| collection_finish_atomic | 1/1, 1.94s | 1/1, 2.03s |

These public Flow producers use `sagat_app` in both invocations. The flag is not a claim that
the default Flow writes as database owner. No ignored or filtered test counts as passing.
Strict integration Clippy passed in 54.60s, application/PostgreSQL library Clippy in 22.02s,
formatting in 4.75s and the 24-crate architecture check in 0.45s. No gate in this set failed.
Raw harness logs and sealed unified patches retain their original whitespace; full diff-check
reports trailing context spaces/blank final log lines there. The source/document check passed.

## What the added tests establish

- The collection End result is a schema-pinned object distinct from the owner aggregate.
  Ordinary Run GET withholds it. Explicit authorized HTTP disclosure returns public/internal
  fields without revealing PII, undeclared or secret values. Its current-authority audit has
  actor/run/reason/trace, and all payload markers and declared keys stay out of audit/logs.
- A public parent creates the actual unchained child. Its first root collection is certified,
  later actual work completes, child End/Run return scalar17, and normal parent fanout retains
  scalar17 on kid and parent End/Run. Both owner pointers retain their own expected aggregates.
  Original source/certificate bytes and cold current-fence replay remain unchanged after
  terminal fanout; both journals verify. This expression reads the later owner; a separate
  incremental assertion will explicitly require previous-owner evaluation context as well.
- An actual scheduler Finish blocks after its real collection.finished outbox insert. A second
  connection sees exactly the prior state, including every tenant payload and physical usage.
  Releasing the gate raises the specific labelled fault, rather than statement timeout, and
  the entire transaction rolls back. Removing the injector permits one real retry: owner
  aggregate, End17, Run17, journal/outbox/head and exact logical charge30 commit together.
- A fresh PgCollectionStore and fresh DefinitionCache on the runtime pool replay the original
  source-bound Finish using aggregate/checkpoint digests and End17 but no aggregate body.
  Matching replay is unapplied; changed checkpoint identity is DuplicateMismatch. Both preserve
  the full snapshot, including payload/accounting state. This is fresh-store replay, not an OS
  process restart. The constructor preserves source Some(empty) separately from legacy None;
  store validation remains authoritative.

## Remaining scope

The [review reconciliation](review.md) records exact bounded findings and corrections. This
does not close the master result/quota correction or any canonical open feature014 task.
Failure-End/pre-evaluation replay, legal End bytes crossing the Run ceiling, report late-fault
rollback, prior-stop/secret races, old in-flight reports and both orders, cold process recovery,
unknown COMMIT, complete rollout, general nesting and final resource/release gates remain.
