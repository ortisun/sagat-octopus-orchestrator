**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors, the questions in your request and my round-13 report.

**What I read:**
- the persona and my round-13 report;
- the round-13-to-14 diff in full;
- `sdks/SPEC.md` §6.2, lines 355 to 415, and §10, lines 840 to 851;
- `stream-completion.md`:
  - the activation, lines 400 to 480;
  - the marks, lines 481 to 584;
  - the server's acknowledgement, step 1;
  - the deadline derivation, lines 716 to 727;
  - "The host at 1.8", lines 748 to 827;
  - the rollback, lines 845 to 871;
  - the failure table rows at lines 877 to 914;
- `tasks.md`: precondition (a), Task 006-043 to Task 006-053, Task 006-058 and Task 006-059.

**What I checked in the code of the merged base:**
- `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`:
  - lines 49 to 168: `platform_requirements_guard` refuses every `UPDATE`, and `TRUNCATE` is refused too, so a `BEFORE DELETE` trigger closes the removal path completely;
  - lines 306 to 378: the activation and removal functions.
- `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`: the replay-floor guard.
- `crates/api-grpc/src/limits.rs`, lines 245 to 307, and `crates/api-grpc/src/server.rs`, lines 44 to 59: what each connection cap actually does.

## Verdict

**Count: no Blockers, no Majors, 3 Minors, 5 Nits.**

- Round-13 Minors 1, 2 and 3 are resolved.
- Round-13 Minor 4 is partly resolved: the rules are written, but its tests were not added (new Minor 3).
- The new redial rule has two defects of its own (Minors 1 and 2). Both are a few normative sentences in §6.2.
- None of the findings makes the design wrong. Every path still ends in an acknowledgement, a withdrawal, an ordinary failure or a review.

---

## Part 1: The round-13 findings

### Minor 1 (the narrowed guard let `lease-withdrawal-v2` be removed while `delivery-acknowledgement-v1` was recorded): resolved

- **The trigger now covers both directions.** It refuses to insert `delivery-acknowledgement-v1` without its preconditions, and refuses to delete `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded.
  - It is stated the same way in `stream-completion.md` lines 411 to 416, Architecture decision 0099 line 162, research line 113, and Task 006-046.
  - With the base's guard refusing `UPDATE` and `TRUNCATE`, no other path remains.
- **The rollback is unaffected.** It removes `delivery-acknowledgement-v1` first and keeps `lease-withdrawal-v2` (line 855).
- **The Gate's "engines only" case** is now reached by a fixture with the trigger disabled.
- **The three sentences are corrected** (Architecture decision 0099 line 302, research lines 753 and 860).
- **Leftovers:** two small ones (Nit 4).

### Minor 2 (Task 006-044 waited, stranding three tasks that need its columns): resolved

- Task 006-044 now holds only the columns, the session flag, the grants and its own `down`. It proceeds.
- The requirements read, the trigger and the closed checks moved into Task 006-046, which waits for the follow-up and sorts after its migration.
- I checked that Task 006-049 does not need the moved function. The acknowledgement does not read `required_since` (`stream-completion.md` lines 533 and 534), so it can be built against Task 006-044 alone.
- **Leftover:** the rollback's text still describes a single `down` (Nit 3).

### Minor 3 (how a merged call's leases share the window was ambiguous): resolved

The rule is now one reading, in `sdks/SPEC.md` lines 368 to 370, `stream-completion.md` lines 798 to 800, and research line 191:
- each lease keeps its own window, evidence and give-up;
- a merged call is "one call" only for the fallback's ordering and counting;
- retries take a free permit before new frames;
- waiting frames form a new call.

I walked through the twelve-frame case under this rule:
- Frames 1 to 4 take the four permits. Frames 5 to 12 form call E.
- At 6 s, E takes the first free permit, and three retries take the rest.
- At about 12 s, leases 1 to 3 are given up.
- At about 18.4 s, lease 4 and leases 5 to 12 are given up.

That is within "three attempts' time plus 1 s". It holds only at a 30 s time-to-live, though, because at 18 s the ordinary give-up comes at 12 s (Nit 5).

### Minor 4 (the redial rule cut its own attempts, "no answer" was undefined, and an engine answering `ACKNOWLEDGEMENT_STORE_UNREACHABLE` was never left): partly resolved

- **Written as asked:**
  - the old connection is drained;
  - an attempt ended by the host's own close counts for nothing;
  - "no answer" means a transport failure, or the host's own deadline with no status. A mesh's 503 therefore no longer redials;
  - the host also redials after two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` answers while the main connection answers `OK`.
- **Not done:**
  - no unit case for the second trigger;
  - the conformance case where one engine stays unreachable for longer than a time-to-live was not added;
  - the two-engine harness still routes "the host's second connection", so where a redialled connection lands, and the expected outcome, are undefined (new Minor 3).
- **The new rule also has two defects of its own** (new Minors 1 and 2).

### The round-13 Nits

- **Nit 1:** resolved. "Sent early, within 200 ms" now appears in the failover row (line 913) and in research. The Fallback evidence row no longer says "at once".
- **Nit 2:** resolved. The text now reads "between the latest evidence before the deciding attempt was sent and that attempt's end", in both documents.
- **Nit 3:** resolved. Task 006-049 adds a per-connection counter to `CappedConnectInfo`, never an address, and Task 006-058 uses it.
- **Nit 4:** resolved. `withdrawn_on_hand_back` is written in the same assignment as the hash, so no journal is read. But the static check's allowed forms do not name the new column (Nit 2).
- **Nit 5:** resolved. Task 006-046's `down` drops the trigger, its function and the requirements read, and keeps `lease-withdrawal-v2` in the restored lists.
- **Nit 6:** resolved. The trigger takes the epoch row `FOR UPDATE` itself.

---

## Part 2: New findings

### Minor 1: An attempt on a connection being drained can start another redial, so a host can hold three or more connections, which the "cap doubled" sizing does not cover

**Location:**
- `sdks/SPEC.md` lines 370 to 374;
- `stream-completion.md` lines 606 to 610;
- the failure row at line 914;
- Task 006-054 (the runbook);
- Task 006-059.

**What is wrong:**
1. **The trigger is not limited to the current connection.**
   - The trigger reads "when an attempt on that connection ends with no answer while a call on the main connection ... was answered".
   - After a redial, up to three attempts are still running on the old, half-open connection, and each ends at its own deadline with no answer.
   - The main connection keeps answering, so each one meets the trigger again. That can mean up to three more redials in a row, each draining the new, healthy connection it replaces.
2. **The capacity statement is too small.**
   - "Each 1.8 host therefore holds two connections" is untrue while a connection drains. There are at least three for up to 6 s, more under the chain above.
   - The server may also keep counting a half-open connection until its idle timeout.
   - Behind shared addresses, a per-peer cap that is exactly doubled therefore refuses the new connection during a wave of redials, for example when one engine's pool breaks and every host redials.
3. **A refusal feeds the loop.**
   - A per-peer refusal drops the socket (`limits.rs` lines 274 to 279). The host sees a transport failure, which is "no answer", so it redials again on its next attempt, and is refused again.
   - That ends, through the fallback, in the fleet declining. The design's own row presents that outcome as the price of a cap that is set too low, not of a redial.

**Why it matters:**
- This is host behaviour that six SDKs build from one sentence.
- Without a qualifier, some SDKs will chain redials and others will not.
- The runbook's sizing rule then fails exactly in the event that redialling exists for.

**Fix:**
- Only an attempt on the current acknowledgement connection can trigger a redial.
- At most one old connection drains at a time. A second redial, while one drains, closes the older one, and its attempts count as ended by the host's own close.
- After a failed dial, the host waits at least one step of the §5 backoff before dialling again.
- State "two connections, three while one drains (at most 6 s)", and size the per-peer cap for that.
- Add a unit case: a half-open connection with three attempts in flight leads to exactly one redial.

### Minor 2: "No answer" includes the host's own deadline, but the attempt's bound may be shorter than the server's 4 s

**Location:**
- `stream-completion.md` line 722 ("The host bounds each attempt to the receive bound") and line 777 ("each attempt bounded to the receive bound");
- the receive bound itself at line 718 ("6 s or its shorter configured call timeout");
- against `sdks/SPEC.md` lines 386 to 400 and Task 006-059 ("the 6 s bound of a poll and of an attempt"), which say 6 s;
- the new Task 006-059 case "a 3 s call timeout on a stream host".

**What is wrong:**
- The server answers within its 3 s deadline plus one statement timeout, so up to 4 s (step 1 of the server's acknowledgement).
- A host with a call timeout under 4 s, bounding attempts by the receive bound as `stream-completion.md` says, cuts a slow answer that was about to arrive. That attempt then ended "with no status received", which is "no answer" under this round's definition.
- With heartbeats answered, every such attempt redials, with a TLS handshake each time. Each also counts as "no answer" toward the fallback, so a slow but healthy database pushes such hosts into declining.
- The two documents disagree on the bound: the Specification says 6 s, `stream-completion.md` says the receive bound. The same split appears in the early give-up's window ("at least 6 s" against "one receive bound").
- The new 3 s test does not say which applies.

**Fix:**
- State in both documents that an acknowledgement attempt, and the early give-up's window, are 6 s whatever the configured call timeout. Only the receive, which is not an attempt, takes the shorter timeout.
- Pin the 3 s test to that: an attempt answered at 3.5 s is a verdict, not a redial.

### Minor 3 (the residue of round-13 Minor 4): The redial after `ACKNOWLEDGEMENT_STORE_UNREACHABLE` has no unit case, and the two-engine conformance case no longer has a defined outcome

**Location:**
- Task 006-059's unit list, lines 468 to 480;
- Task 006-058's harness, lines 409 to 411, and its scenario, lines 430 and 431;
- the Fallback evidence row.

**What is wrong:**
- No unit case drives two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` answers with the main connection answering `OK`, and asserts the redial. Nor does one assert that no redial follows when the main connection fails too.
- The harness "routes the host's second connection" to the unreachable engine. After this round, the host redials, and its third connection's route is unspecified. Two runners will therefore produce different goldens.
- The expected outcome "no give-up, then `recovered`" is unchanged. It now depends on where the redial lands.
- The case "one engine unreachable for longer than a time-to-live" (Gap 006-047's new residue) was not added.

**Fix:**
- Add both unit cases to Task 006-059.
- Have the harness route by connection order: the first acknowledgement connection to the unreachable engine, every later one to the healthy engine.
- Expect a redial after two answers and then `recovered`.
- Add a variant where every acknowledgement connection goes to the unreachable engine for longer than a time-to-live. Expect the ordinary give-up and the outcome of Gap 006-047, reported.

### Nits

1. **`max_connections` does not refuse, it waits.**
   - `Incoming::poll_next` waits for a permit before it accepts (`limits.rs` lines 256 to 269; `server.rs` line 49: "the next one is accepted when one closes").
   - The connection sits in the kernel's backlog. The host sees a connect or first-call timeout, and `grpc_connections_refused_total` does not rise.
   - The failure row (line 914), `stream-completion.md` lines 608 to 610, and Task 006-054's runbook text name the wrong signal for that cap. Give the global cap its own symptom: calls time out with no status, and the open connections sit at `max_connections`.
2. **The static check does not know `withdrawn_on_hand_back`.**
   - The allowed forms (`stream-completion.md` lines 560 to 565) name only "the withdrawal sets `withdrawn_lease_token_hash` from the row it holds".
   - Task 006-050 says only that the hash is "kept by every withdrawal".
   - Add the column to the allowed form ("with the hash, in the same assignment, from a constant of the withdrawal path"), and to Task 006-047's negative tests.
3. **The rollback's item 4 (`stream-completion.md` line 869) still describes one `down` with three refusals.**
   - Write that Task 006-046's `down` refuses while the history names the capability or a live process lists it.
   - Write that Task 006-044's `down`, which runs after it, refuses while a provisional mark counts.
4. **Two things around the new `BEFORE DELETE` refusal are left unsaid.**
   - The failure table has no row for removing `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded. Add one: "Refused by the trigger".
   - The race cases now run into the trigger: Task 006-051's "the removal of each requirement held between the pass's read and its commit, in both orders", and the Gate row's "removal of either requirement". A marked lease is withdrawn only while `delivery-acknowledgement-v1` is recorded, and with it recorded, a removal of `lease-withdrawal-v2` alone is refused in both orders. State that expectation, and reach "the removal commits first, the withdrawal writes nothing" through the existing hand-SQL case that removes both in one transaction.
5. **Two leftovers of the merge wording.**
   - Host item 2 (`stream-completion.md` line 758) still says "waiting frames merged into the next call". Write "into a new call".
   - The twelve-frame unit case does not state its time-to-live. It holds at 30 s and cannot hold at 18 s, so state 30 s.

---

## Part 3: Is it complete enough to accept?

**Yes, for the owner's decision. Minors 1 to 3 must be corrected before the SDK tasks (Task 006-059 to Task 006-064) start.**

- **The server side is now closed:**
  - the trigger covers both directions;
  - the base's guard leaves no `UPDATE` or `TRUNCATE` path;
  - the task split leaves every proceeding task with the columns it needs;
  - the stale path reads a column, not the journal.
- **What remains is host text, all in `sdks/SPEC.md` §6.2:**
  - Minor 1 needs one qualifier ("the current connection"), a limit of one draining connection, and the sizing restated;
  - Minor 2 needs a fixed 6 s attempt bound;
  - Minor 3 needs unit cases and a defined harness route.
  - These decide behaviour in six languages, so they must land before those tasks.
- **The Nits can go into the tasks.**
- **The risks the owner accepts are unchanged:** Gap 006-041, Gap 006-042, Gap 006-044, Gap 006-047 (now including a stale pool that outlasts the ordinary give-up) and Gap 006-048.
- **No further full round is needed.** A targeted check of the three Minors is enough.

**Files referenced:**
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-13/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
