Round 14 has no Blockers and no Majors. It has 3 Minors and 6 Nits, all new and all caused by the round-13 fixes. Round-13 Major 1 and Minors 1 and 3 are resolved. Minor 2 is partly resolved. All seven Nits are resolved; the fix to Nit 1 leaves one sentence behind (Nit 2 below).

# Architecture review, round 14 (targeted): delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Merging the design:** nothing.
- **The owner's acceptance of Architecture decision 0099:** Minor 3 should be fixed first. Otherwise the record and its withdrawn stub, Architecture decision 0100, contradict what Task 006-046 builds.
- **Task 006-047 and Task 006-050:** Minor 2 must be fixed before they start.
- **Task 006-054:** Minor 1 must be fixed before it starts.

**Limits:**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I searched every `crates/` and `apps/` tree under `/Users/marcelo/Documents/Projects` for `lease-withdrawal-v2`, `withdrawn_on_hand_back` and `ACKNOWLEDGEMENT_STORE_UNREACHABLE`.
  - The only matches are the merged head's test in `apps/control-plane/src/capability_activation.rs`, in this worktree and in `sagat-wt-withdraw-leases`, as in round 13.
  - So the follow-up of pull request #260 has not started anywhere, and nothing is duplicated.

---

## Status of the round-13 findings

| Round 13 | Status | Reason |
|---|---|---|
| **Major 1** (three tasks that proceed need the columns of a task that waits) | **Resolved** | <ul><li>Task 006-044 (`tasks.md:179-189`) now holds only the nine `steps` columns, the `worker_sessions` column, the grants, the start-time check, the storage classification and a `down` that drops the columns. It touches no closed check and proceeds.</li><li>The registry part is in Task 006-046 (`tasks.md:195-205`), which waits, and its migration sorts after the follow-up's.</li><li>Precondition (a) (`tasks.md:144-150`) now lists Task 006-044 as proceeding.</li><li>I checked that the tasks that proceed need nothing from Task 006-046:<ul><li>the acknowledgement does not read `required_since` (`stream-completion.md:533-535`), so Task 006-049 needs neither `delivery_acknowledgement_required()` nor a requirement row;</li><li>the capability fence of Task 006-052 decides on the mark and the incarnation;</li><li>Task 006-048 writes only the session column.</li></ul></li><li>The capability-list check (`scripts/check-architecture.py:624-653`) reads the newest migration that defines each closed check. So only Task 006-046's migration has to sort after the follow-up's, and Task 006-044's migration may sort anywhere.</li><li>Residue: Nit 2 (the two `down`s).</li></ul> |
| Minor 1 (the harness cannot read the connection; the Rust proxy pools) | Resolved, with residue | <ul><li>Task 006-049 adds a per-connection counter to `CappedConnectInfo`, never an address. A test hook exposes it.</li><li>The Rust runner's proxy opens one upstream connection per downstream connection.</li><li>The text says "another connection", and each SDK compares local socket addresses.</li><li>Residues: who owns the proxy change (Nit 3), and how the hook stays out of production (Nit 4).</li></ul> |
| Minor 2 (the connection caps are not stated) | Partly resolved | <ul><li>The per-peer cap is stated in the record's Consequences, `stream-completion.md:607-610`, a failure row (`:914`), Task 006-054's runbook text, and Task 006-056, which reports connections per host.</li><li>But the process cap is described as a refusal that the per-peer counter reports, and the code does neither (Minor 1).</li></ul> |
| Minor 3 (a redial cancels the other attempts in flight) | Resolved | <ul><li>`sdks/SPEC.md` §6.2 now says the new connection carries the next attempts and the old one closes once its attempts in flight have ended.</li><li>An attempt ended by the host's own close is sent again and counts for nothing.</li><li>"No answer" is defined.</li><li>Task 006-059 has the unit case "a redial with three other attempts in flight".</li><li>Residue: Nit 5 (connections while a redial drains).</li></ul> |
| Nits 1 to 7 | Resolved | <ul><li>Nit 1: Task 006-046's `down` drops the trigger, its function and the requirements read by full signature, and restores the follow-up's lists. Residue: Nit 2.</li><li>Nit 2: "a trigger for the order alone".</li><li>Nit 3: "has not reached a writable database".</li><li>Nit 4: "Enforced by" names the trigger, the replay-floor guard and the connection assertion.</li><li>Nit 5: the journal read is replaced by a column. That brings Minor 2.</li><li>Nit 6: Task 006-046's description of the beat is updated.</li><li>Nit 7: the trigger takes the epoch row `FOR UPDATE` itself. That lock is taken again in the same transaction (re-entrant) through `platform_requirement_remove` (`20270113000000_instance_registry.up.sql:364`) and through the runbook's `platform_requirement_lock()` (`docs/runbooks/instance-registry.md:158`).</li></ul> |

**The other changes of this round, from the architecture side:**
- **The `BEFORE DELETE` refusal:** it is needed. It closes the remove-and-record-again sequence of round-13 design Minor 1. The census can only see that sequence at an insert of `delivery-acknowledgement-v1`. The records do not yet say why the refusal exists (Minor 3).
- **`ACKNOWLEDGEMENT_STORE_UNREACHABLE` covering authentication and `25006`:**
  - It is buildable without a method-specific branch in a shared layer, because authentication runs inside each handler (`crates/api-grpc/src/service.rs:479-492`).
  - The classification it names, `registry_error`, is a private function (`crates/infra-postgres/src/instances.rs:73-93`). See Nit 6.
- **"Built by the same function as the main connection":** sound. It keeps credentials, TLS settings and interceptors on one code path, and the other five SDKs' fault interceptors then reach both connections.

---

## Minors

### Minor 1. The process cap does not refuse a connection, and the per-peer counter does not rise at it; two existing runbooks still say one connection per worker is enough

**Where:**
- `stream-completion.md:607-610` and the failure row at `:914`: "An acknowledgement connection refused at `max_connections_per_peer` or `max_connections` … `grpc_connections_refused_total{reason="per_peer"}` rises".
- `tasks.md:359-361` (Task 006-054's runbook text says the same).
- The record's Consequences (line 274): "a host refused at a cap falls back".
- The code, `crates/api-grpc/src/limits.rs:256-280`:
  - The process cap is a semaphore acquired *before* `poll_accept`. At 2,048 the listener simply stops accepting. The kernel completes the TCP handshake into the backlog, the HTTP/2 preface is never read, and no counter moves.
  - Only the per-peer check (line 274) refuses, and only it increments `grpc_connections_refused_total{reason="per_peer"}`.
- `docs/runbooks/security-governance.md:226` says "one connection per worker process is enough". `docs/runbooks/execution-engine.md:124-125` describes both counters as today's build has them.

**What is wrong:**
- At the process cap a 1.8 host's acknowledgement connection is never refused.
  - Its attempts end at the host's own deadline with no status. That is "no answer", so the host redials (Minor 3 of round 13), and the new connection waits in the backlog too.
  - The host then falls back after three counted calls.
- The outcome (the fallback) is stated correctly, but the signal is wrong. An operator told to watch `per_peer` sees nothing, and no metric shows a process at its connection cap.
- The runbook row that sizes the per-peer cap at "one connection per worker process" contradicts the new text, which Task 006-054 adds to `docs/runbooks/sdks.md` only.

**Why it matters:** operability. A ceiling must be stated with its real effect and a signal that an operator can see.

**Smallest correction:**
- Split the row and the runbook text into two causes:
  - **At `max_connections_per_peer`:** the connection is accepted and closed at once, `per_peer` rises, and the host falls back.
  - **At `max_connections`:** the connection is not accepted, the attempts end at the host's deadline with no status, the host redials and falls back, and `grpc_connections_total` stops rising.
- Either add a gauge of open connections (one series, no label) in Task 006-049, which already touches `limits.rs`, or name `grpc_connections_total` flat as the symptom.
- Add `docs/runbooks/security-governance.md:226` and `docs/runbooks/execution-engine.md:124-125` to Task 006-054 ("two connections per 1.8 host").

**Blocking:** before Task 006-054.

### Minor 2. `withdrawn_on_hand_back` is in the marks' table but not in the static check's allowed forms, nor in Decision 006-042's list of mark columns

**Where:**
- `stream-completion.md:493`: the new column is a row of the marks' table, "in one assignment" with the hash.
- `stream-completion.md:560-569`: the allowed forms name "the withdrawal sets `withdrawn_lease_token_hash` from the row it holds", and nothing else for the new column.
  - The check "fails on any other form in `crates/infra-postgres` and the migrations".
  - "No statement assigns a column of the marks from a value it read earlier", and the claim must never be conditioned "on a bound flag alone".
- `research.md:577-578` (Decision 006-042) lists seven mark columns without it.
- Task 006-047's static check (`tasks.md:243-245`) and Task 006-050's write (`tasks.md:298`) do not name it.
- The record, decision 12 (line 186), names only `steps.withdrawn_lease_token_hash`.

**What is wrong:**
- The column's value comes from the withdrawal's cause, a value the caller knows and binds. That is a bound value, not one read from the row.
- So one of two things happens:
  - the static check, as specified, rejects Task 006-050's assignment, and the builder relaxes the check on their own; or
  - the column is not a mark, and the check does not cover it.
- Either way, the rule and its fitness function disagree about one column, and two tasks (Task 006-047 writes the check, Task 006-050 writes the column) will settle it by improvising.
- Being wrong costs only the counted kind and the alert (`heartbeat` against `heartbeat_after_hand_back`), not safety. That is a reason to allow a bound form, but in writing.

**Why it matters:** rule 21 (a fitness function must match the rule it enforces), and rule 1 (the design is read before the code).

**Smallest correction:**
- Add one allowed form to `stream-completion.md:563` and Decision 006-042: "the withdrawal sets `withdrawn_lease_token_hash` and `withdrawn_on_hand_back` together, in one statement, the hash from the row it holds and the flag from the withdrawal's cause, bound only by the withdrawal service; neither is assigned alone".
- Add a negative test to Task 006-047: one column assigned without the other.
- Name the column in Task 006-050's write, and in decision 12 next to the hash.

**Blocking:** before Task 006-047 and Task 006-050.

### Minor 3. The new `BEFORE DELETE` refusal is the second half of the trigger that Architecture decision 0100 withdrew, and neither record says why it is back

**Where:**
- Architecture decision 0100:
  - Context (lines 10-12): the withdrawn trigger "would refuse recording `delivery-acknowledgement-v1` before `lease-withdrawal-v1`, and removing the second while the first is recorded".
  - Decision (lines 16-19): "The trigger, its static check and its typed refusals bought nothing more."
  - Consequences (lines 29-32): the trigger of decision 10 only "refuses recording `delivery-acknowledgement-v1` while a registered instance lacks `lease-withdrawal-v2`".
- Architecture decision 0099, considered option 10 (lines 243-247): it justifies the trigger only by the census at insert.
- What is now built (`stream-completion.md:411-416`, Task 006-046, decision 10 line 162): it also refuses deleting `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded.

**What is wrong:**
- The design now builds what Architecture decision 0100 says bought nothing, with `lease-withdrawal-v2` in place of `lease-withdrawal-v1`.
- The reason is different and sound: it protects the census invariant against a removal followed by a new recording, not the order. Only `research.md:654-655` records that reason, as "round 13: design Minor 1", without the sequence.
- An owner reading the two records at acceptance sees a decision withdrawn as useless and then built, with no recorded reason.

**Why it matters:**
- A decision record must say what is decided and why, and a withdrawn record must not contradict what ships.
- The next person to simplify the trigger will read Architecture decision 0100 as permission to drop the `DELETE` branch, and that reopens round-13 design Minor 1.

**Smallest correction:**
- One sentence in option 10 of Architecture decision 0099: "The trigger also refuses removing `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded, because a removal and a new recording would otherwise admit a registered binary without the liveness rule that the census at insert never sees; that is the census invariant, not the order".
- The same sentence in Architecture decision 0100's Consequences.
- Name the sequence (remove, a #260 binary registers, silent, record again) in Decision 006-047.

**Blocking:** before the owner's acceptance of Architecture decision 0099.

---

## Nits

1. **The record's Consequences count "one function (the requirements read)"** (line 271). Task 006-046 also adds the trigger and its function. Write "one function and one trigger with its function".
2. **The Rollback still describes one `down`.**
   - `stream-completion.md:869-871` (step 4) describes one `down` that refuses on marks, history and live processes. There are now two, run in reverse order:
     - Task 006-046's `down` first (history, live process; it drops the trigger, the functions and the lists);
     - then Task 006-044's `down` (provisional marks; it drops the columns).
   - Say also that Task 006-044's `down` evaluates "a provisional mark counts" without `required_since`. That makes it stricter, and it depends on no object of Task 006-046, which its `down` has already dropped.
3. **The Rust proxy change sits with the wrong owner.**
   - Task 006-058 (the shared conformance owner) asks for "the Rust runner's fault proxy opening one upstream connection per downstream connection".
   - Item 2 of "Implementation ownership and integration order" (`stream-completion.md:200-203`) gives each SDK's conformance runner to its SDK owner. The file `sdks/rust/conformance/src/faults/grpc.rs` is therefore Task 006-064's.
   - Move the change there. Say that Task 006-058's assertion runs in Rust only once Task 006-064 has merged.
4. **The test hook's boundary is not stated.**
   - Say that the counter reaches the harness only through a layer the conformance platform installs (reading it from the request's connect information).
   - Say that it appears in no metric label, log field or span attribute. It is an unbounded value.
5. **"Two connections per host" undercounts.** While a redial drains, a host holds a third connection for up to its attempts' bound. Say "two, three while a redial drains" in the runbook text, since the per-peer headroom is what decides whether the new dial is refused.
6. **Share the store-unreachable classification instead of copying it.** Task 006-049 should make `registry_error`'s rule (`crates/infra-postgres/src/instances.rs:73-93`) one shared crate-level function, used by the registry and the acknowledgement. A copy would drift on `08xxx` and `57P0x`. A unit test of the shared function with `25006` added is enough.

---

## The checks you asked for

- **Layers:**
  - Nothing new reaches the domain.
  - The new column, the trigger branch, the connection counter and the error classification are adapter-side.
  - The host connection rule is SDK transport.
- **Decisions:**
  - The `BEFORE DELETE` refusal needs its reason in option 10 and in Architecture decision 0100 (Minor 3).
  - "Enforced by" is now complete for the rules of round 13.
- **Compatibility:**
  - Task 006-044 is additive. Its columns are nullable apart from `declines_acknowledgement`, which has a default, and no `CHECK` ties them to the status, so an older binary still writes the row.
  - Task 006-046's `down` restores the follow-up's lists.
  - The order of the two `down`s needs one sentence (Nit 2).
- **Operability:**
  - `store` stays in a closed list.
  - The process-cap signal is wrong (Minor 1).
  - The counter must stay out of telemetry (Nit 4).
- **Scope:**
  - No duplication found.
  - Request (9) stays with the follow-up.
  - The split respects the dependency of precondition (a).

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| The tasks that proceed have their schema | Task 006-044 proceeds; Task 006-049's and Task 006-052's SQL fixtures | none |
| The closed checks list `Capability::ALL` | `capability_list_errors` in `scripts/check-architecture.py`, newest migration | none: only Task 006-046's migration has to sort after the follow-up's |
| No registered instance lacks `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded | The trigger's insert and delete branches; Task 006-046's hand-SQL cases | Record the delete branch's reason (Minor 3) |
| Every assignment to a mark has an allowed form | The static check of Task 006-047 | `withdrawn_on_hand_back` is not in it (Minor 2) |
| The acknowledgement has a connection of its own | Task 006-049's counter, Task 006-058's assertion, each SDK's socket-address test | Owner of the Rust proxy (Nit 3) |
| A 1.8 fleet fits under the connection caps | Task 006-056 reports connections per host; the runbook text | The process cap's real signal (Minor 1) |
| A redial decides nothing by itself | The unit case of Task 006-059 to Task 006-064 | none |
| A store that cannot be reached decides no give-up | Task 006-049's pool-full, authentication and `25006` cases | One shared classification (Nit 6) |

## Items for the owner

1. **The connection cost:** whether a 1.8 fleet behind shared addresses or near 1,024 hosts per engine is expected. If it is, decide whether the defaults of `max_connections_per_peer` (256) and `max_connections` (2,048) rise with Task 006-057, rather than relying on the runbook text alone.
2. **Still open:**
   - accepting Architecture decision 0099, with both assumptions of its decision 12 (after Minor 3);
   - Owner question 006-002 (the reservation of minor 1.8);
   - who builds the follow-up of pull request #260, with requests (1), (2) and (5) to (9), and when;
   - the delegate's thresholds;
   - the constraints that stay: no emergency stop for the provisional leases in flight, and two restarts for a rollback.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (144-150, 179-231, 243-245, 253-282, 298, 353, 359-361, 405-410, 485)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (200-203, 411-427, 483-493, 530-536, 560-569, 600-620, 869-871, 913-914)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (576-585, 651-662, 731)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (13, 86-93, 157-162, 186, 243-247, 271-274)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md (10-19, 29-32)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§6.2, lines 363-400)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (256-307, 327-329)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs (479-492)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs (73-93)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (98-168, 357-378)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/scripts/check-architecture.py (624-653)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/conformance/src/faults/grpc.rs (44-67)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/security-governance.md (226)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/execution-engine.md (124-125)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (152-170)
