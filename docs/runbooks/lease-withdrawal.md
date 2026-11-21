# Lease withdrawal (feature 006, ADR-0098)

A lease the execution engine committed and never handed to a host used to lapse after its time-to-live; since feature 037
a lapsed lease of an `Activity` opens a review (`step.needs_review`) for work that never ran. With the requirement
`lease-withdrawal-v1` recorded, the engine withdraws such a lease instead: the step goes back to `READY`, the attempt is
not spent, and the journal records `step.lease_withdrawn` (cause `NOT_SENT`). A host that drops a lease it received hands
it back with `FailStep` `TASK_NOT_STARTED`, and the engine withdraws that one too (cause `NOT_STARTED`) while the step is
still `LEASED`. The decisions are D006-001 to D006-018 of `specs/006-sdk-multilanguage/research.md`.

## What changes, and what does not

- Nothing changes until the operator records the requirement. Before that, the counter
  `worker_leases_undelivered_total{cause, outcome="lapsed"}` shows what activation would recover.
- A frame lost on the wire after it left the process, and a lost answer of a unary poll, still lapse into a review
  (gap G006-001). A process that crashes between a claim and its withdrawal leaves the lease to lapse (gap G006-002).
- A lease that lapses keeps its meaning: an unknown outcome, consultation or review (feature 037, FR-037-006).

## Before the activation

1. Every control plane and engine runs this build or a later one. A process of this build lists `lease-withdrawal-v1`
   unconditionally. **A binary of release 0.2 does not register in the instance-capability registry, so an activation cannot
   see it** (`docs/runbooks/instance-registry.md`, section 7): stop or upgrade every 0.2 process first. A 0.2 binary cannot
   replay `step.lease_withdrawn`.
2. Count the live processes you expect (control planes and engines together).

## Activate

Run with the schema-owning database role (the `DATABASE_URL` of `migrate`):

```sh
control-plane lease-withdrawal-activate --operator NAME --expect-live N --dry-run
control-plane lease-withdrawal-activate --operator NAME --expect-live N
```

- `--expect-live N`: the command refuses when the live count (seen within 30 seconds) differs; without it, when none is
  live. The count is taken again under the epoch lock.
- A live process that does not list the capability is refused by its instance id; nothing is recorded.
- A second run says the requirement is recorded already and exits 0.
- Each engine reads the requirement at its next heartbeat (within about ten seconds) and withdraws from then on.

## Watching it

| Signal | Meaning |
| --- | --- |
| `worker_leases_undelivered_total{outcome="withdrawn"}` | Leases given back; the steps are `READY` with their attempt unspent |
| `worker_leases_undelivered_total{outcome="fence_missed"}` | The step had moved on (a heartbeat started it, it was reported, the run ended); nothing was changed |
| `worker_leases_undelivered_total{outcome="lapsed"}` | The requirement is not recorded (or not yet read by this engine): the lease lapses as before |
| `worker_leases_undelivered_total{outcome="error"}` | The store failed or the process was stopping past its grace; the lease lapses |
| `step.lease_withdrawn` in a run's timeline | One withdrawal, with its cause (`NOT_SENT`, `NOT_STARTED`) |

A rate of `withdrawn` that keeps growing for one step means a loop (decision D006-006): look at the hosts of its activity
(a host that keeps handing back with `TASK_NOT_STARTED`, a client that keeps dropping its streams).

## Rollback

1. Remove the requirement first, with the owner role:
   `SELECT * FROM platform_requirement_remove('lease-withdrawal-v1', 'your-subject');`
   New withdrawals stop within one heartbeat.
2. Only then roll a binary back. A journal that holds `step.lease_withdrawn` cannot be replayed by a binary older than this
   feature, and the down migration `20270208000000_lease_withdrawal` refuses while any journal holds one, while the
   requirement or its history exists, or while a registered process lists the capability.
