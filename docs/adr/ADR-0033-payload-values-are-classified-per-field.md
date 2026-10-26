# ADR-0033: Payload values are classified per field, and an unmarked field is personal data

**Status:** Accepted direction (owner decision, 2026-10-16); implementation in progress in feature
016. Production composition and focused tests exist on the integration branch; the complete canary,
compatibility, browser and release gates remain pending. See
[the integration evidence](../../tasks/project-completion/integration-notes.md).

**Refines:** ADR-0028 (authorized structural payload inspection). It does not change ADR-0031,
which already discloses the declared result of a Query or Update, nor the secret model of feature
009. **Closes as decided:** G42 (classification part), G80 (classification part), and the value
side of G144 and G21 (a run result is disclosed under this policy).

## Context

Every payload (run input and output, step input and output, event data, declared state) is treated
as confidential: ADR-0028 lets an authorized user see its shape, and nothing else. Operators asked
to see values when diagnosing a run, and a run result (G21) will need a disclosure rule. A
credential heuristic cannot decide what is safe to show: a secret can be any string, number or key
(ADR-0028, Context). Only the author of the workflow knows what a field holds.

## Decision

1. **Four levels, declared by the author on the JSON Schema of a value:** `public`, `internal`,
   `pii` and `secret`, in an annotation the workflow IR carries with the schema (name chosen by
   the feature specification). The level of a field applies to everything under it unless a
   descendant declares a stricter level; a descendant never loosens its parent.
2. **An unmarked field is `pii`.** Nothing is disclosed by omission; a schema with no annotations
   discloses no value, exactly as today.
3. **Who sees what:**

   | Level | Disclosed to |
   |---|---|
   | `public` | every role that may read the run (including `viewer`) |
   | `internal` | `developer`, `operator`, `admin` |
   | `pii` | only a principal holding a new permission `payload:reveal-pii`, granted explicitly; every reveal is audited |
   | `secret` | nobody, through any API, log, span, export or the Studio |

4. **The classification is part of the published version.** It is immutable with the version and
   pinned by every run, so a later edit cannot reveal values of runs that already ran.
5. **Disclosure is per request and audited for `pii`;** a value that does not validate against its
   schema is shown as structure only. Telemetry, logs and metrics never carry values whatever the
   level (constitution, principle VI).
6. Retention per customer or regulator is **not** part of this decision; the ratified default
   tier's retention applies.

## Consequences

- A new feature delivers it: IR and schema annotation, publication validation, the permission and
  how it is granted while the built-in roles stay six (G39 has no custom roles; the feature
  specification decides), API and Studio disclosure, audit, and tests that a
  `secret` or unmarked value never crosses any boundary (including a canary like
  `tests/fault/workflow-primitives/operations.rs`).
- Until that feature is released, every surface keeps ADR-0028's structure-only behaviour.
- The run result of G21 is stored when built, but disclosed only through this policy.

## Addendum, 2026-10-17: answers of the owner to the feature 016 specification

The specification in `specs/016-payload-classification/` filled the points this ADR leaves to it. The
owner answered the four product questions on 2026-10-17; none changes the decision above.

- **Grant mechanism (D275).** `payload:reveal-pii` is granted through a per-namespace grant list managed by
  administrators, following the worker-binding and secret-access-list pattern. The built-in roles stay six;
  no role holds the permission, `admin` included. Grants and revocations are audited. An administrator may
  grant it to themselves, audited (D306).
- **Readings of the text (D265, D269).** A field no schema declares is at least `pii`, even under a `public`
  parent. A schema with no annotation is `pii` throughout, and a holder of the reveal grant sees it. A
  payload with no schema at all is structure only for everyone.
- **Events (D302).** The consequence above that "every surface keeps ADR-0028's structure-only behaviour"
  was inexact for events: `getInboxEvent` returned `data` and connector dead letters returned `body` and
  `headers` to every holder of `event:read`. In the feature 016 increment these values require the same
  reveal grant and everyone else receives structure only. Field-by-field classification of events waits for
  event type schemas; until then the data of an event is `pii` as a whole. This is a breaking change for
  readers of those fields and is documented as such.

At adoption, this addendum recorded decisions only. Current implementation evidence is linked in
the status above; historical owner decisions do not substitute for executable proof.
