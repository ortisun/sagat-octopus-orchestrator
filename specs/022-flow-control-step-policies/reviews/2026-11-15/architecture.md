# Review: feature 022, 0.3.0 slice foundation — architecture

- **Change:** `feature/022-throughput-control`, commits `4d044013..b5d071b3`
- **Reviewer:** `architecture-reviewer` (independent read-only agent, given the changed files and the ADRs), 2026-11-15
- **Recorded by:** the author, from the reviewer's report.

| Finding | Severity | Answer |
|---|---|---|
| 1 — the runtime role could forge counters on insert | Major | Fixed (see the security review) |
| 2 — the data model reads the key version `FOR SHARE`, which `SELECT`-only cannot do; without it a rotation can split a key | Major | Fixed: `flow_control_active_key_version()` (`SECURITY DEFINER`, `FOR SHARE`, executable by `sagat_app`) and the `BEFORE INSERT` version check on semaphores and windows |
| 3 — the activation command records the requirement on an empty fleet, after which no process of this build serves | Major | Fixed: the command's feature step (`live_capable_step`) refuses unless a live process of each kind lists the capability. Test `the_activation_of_an_empty_fleet_is_refused_by_its_own_step` (red under mutation) |
| 4 — the reset rules in the code (`EXCLUDE`) contradict the contract tables (`REMATERIALIZE`/`REFUSE`) | Major | Fixed in the contracts: 018's `journal-and-events.md` gains a row for the ten types (`EXCLUDE`, D022-035) and 022's `journal-events.md` a note. The contract test that parses the table against `RULES` is left (G1191) |
| 5 — the count triggers fix a lock order later tasks must follow; promotion order; `live_keys` triggers in T022-105 must be deferred to keep slot 8b last | Major | Recorded as D022-038 for T022-022/072/097/098/105; the empty-row delete is now deferred to commit |
| 6 — the architecture rule checked only the file of an `INSERT INTO runs` | Minor | Fixed: the guard (`declares_gates()` / `refuse_gated_successor(`) must precede the insert in the same function; negative fixtures added |
| 7 — the two run writers checked different things | Minor | Fixed (both refuse any gate) |
| 8 — `rate_limit_rows_guard` read `OLD.leased_at` on a waiter (no such column) | Minor | Fixed: branched per table; test asserts the guard's `restrict_violation` for a waiter update |
| 9 — nothing in the database computes or checks `key_shard` and the waiter `shard` | Minor | Left, G1189, for T022-097 (the grant pass that reads them) |
| 10 — key activation is not audited with the database role and keeps no history | Minor | Left, G1190, with T022-095 (per-key rotation) and its runbook |
| 11 — `grants_total` per window life | Minor | D022-039 |
| 12 — the selector missed `workflow.rs`, `workflow/flow_control.rs`, `keyed_digest.rs` and future `flow_control/` directories | Minor | Fixed, fixtures added |
| 13 — the down scans `run_events` under `SHARE ROW EXCLUSIVE` | Nit | Accepted: an operator rollback on a quiesced service, as the start-delay down does |
| 14 — `GateKind::field` keeps IR names in the domain | Nit | Accepted: the names are the domain's vocabulary of what is declared, used only in error text |

Counts: B0 M5 m7 N2 — fixed or recorded (D022-038, D022-039, G1189, G1190, G1191).
