# Reviews of the batch `a99bb05b..bdd0a3a9` (tasks T022-104, T022-111, the migration renumbering), and the rulings (2026-11-25)

The repository's read-only `correctness-reviewer` and `security-reviewer` read the code at `bdd0a3a9`. No Blocker.
Correctness: 1 Major, 7 minors, 3 nits. Security: 2 Majors, 2 minors, 3 nits and one suspicion. Every ruling is
delegate (OD-2026-11-24-4), documents first, built with a test seen failing first.

## Majors

| Finding | Ruling |
|---|---|
| Correctness Major 1, security minor 2: a page of the group view sums each of its groups whole (`flow_control.rs:1274-1312`, `:1344-1393`); one group of 100,000 keys makes every page a 503 and no cursor gets past it; the lookahead group is summed too | A page reads at most a constant number of key rows (the per-key page bound, 200); a group's partial sums, its oldest instants and the last key digest read are carried in the sealed cursor, and the group's row is emitted when its range is exhausted; the lookahead only probes that a next group exists. The contract's "summed over its keys in the page's snapshot" becomes "summed across the pages that read it, each page one snapshot" in the OpenAPI, FR-022-022's amendment and D022-086. Test: one group of 100,000 keys with holders (and the rate equivalent with grants) plus a group `zz`; a viewer reads `limit=1` then the cursor, every page 200, `zz` reached, rows read per page under a constant times the limit |
| Security Major 1: the group view's `keys` count tells a caller without the reveal right which runs share a key (`flow_control.rs:1279`, `:1302`; `flow_control_keys.rs:345-362`; `dto_flow_control.rs:88`, `:289`) | `keys` is removed from both group bodies, the OpenAPI schemas and the Studio column. Test: a group with one key and two runs gives a non-revealer nothing that changes with the number of keys |
| Security Major 2: a `secret` value carried through a node without an output carrier (a `forEach` returning a body activity's secret output, `while`/`repeatUntil` carry, `database` nodes, `state`) is graded `pii` and published with a warning (`infra-ir/src/flow_control.rs:138-173`, `infra-schema/src/classification/publication.rs:156-160`) | A location without its own carrier takes the strictest level of every carrier of the document, never less than `pii`; `steps` as a whole folds body carriers too; a carrier whose schema walk is incomplete counts as `secret` (the security suspicion: "cannot prove it is not secret"). Tests: the `forEach` fixture the reviewer describes, refused `FLOW_CONTROL_KEY_SECRET`; an incomplete walk over a `$ref` to a secret field, refused |

## minors and nits

| Finding | Ruling |
|---|---|
| Security minor 1: the group view's `nextSlotAt` links a waiting run to a granted one | Omitted from the group view (callers with the reveal right read it per key) |
| Correctness minor 1: the release-upgrade tests assert a hard-coded migration head (`release_upgrade.rs:205`, `release_upgrade_inflight.rs:181`, `release_upgrade_callback_cutover.rs:123`) | They read the head from the migrator, so a later migration never makes them stale |
| Correctness minor 2: a database that applied the old `20270210*` numbers no longer starts | A note in D022-078's renumbering entry and the quickstart: recreate a database that applied them (the branch is unreleased) |
| Correctness minor 3: the SDK branch still carries the old numbers | The coordinator merges this branch into it before it merges |
| Correctness minor 4: the pinned-limit rollback test claims a refused rollback changes nothing while later migrations were rolled back first | The test undoes to `20270213000100` first, then asserts the refusal changes nothing |
| Correctness minor 5: the group plan test seeds no holders, grants or waiters | It seeds them and bounds rows read in those tables too |
| Correctness minor 6: two assertions of the key-label test pass on an empty list | They assert `view == "GROUPS"` and `items == []` |
| Correctness minor 7: FR-022-022, `data-model.md`, the runbook (`flow-control.md:484`, `:491-492`) and the Studio panel still describe the minute cut and per-key rows for every caller | All describe the two views; the panel's dead `HIDDEN_NOTE` and its test go |
| Correctness nit 1: a reveal right changed between two pages answers "the cursor is not valid" | A distinct message: the cursor belongs to the other view |
| Correctness nit 2: a storage error reading the pinned version fails the run read | It hides the label (D022-085 item 4) and logs |
| Correctness nit 3: a field typed only through `$ref`, `enum` or `const` is raised to `pii` | The scalar check follows the schema walker's resolution |
| Security nits: `continuation.rs` never attaches key levels; a run-list page may parse up to 200 versions; `view: KEYS` labels a secret key from an earlier build | A debug assertion and a comment at the continuation; the other two stated in D022-085 |
