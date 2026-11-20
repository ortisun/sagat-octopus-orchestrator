# Five-SDK conformance scenarios — feature 030 (draft)

Added to `tests/contract/sdk-conformance/scenarios/` with reviewed goldens in `expected/` (never
auto-blessed: independent assertions check values and codes before a golden is written, as the
existing `database-connections` and `switch-builders` scenarios do). Every scenario runs in
TypeScript, Go, Python, C# and Java against the real stack of `tests/support`, with two namespaces
of one tenant (`orders-it`, caller; `payments-it`, handler) and a second tenant for the negative
cases. Normalization removes generated ids and times only; no journal event is removed.

| Scenario | Phase | What every SDK must produce |
|---|---|---|
| `service-contract-builders` | build | Byte-identical canonical `Service` documents (RFC 8785) for the same contract; builder-side refusals (`VALIDATION_FAILED`) for a missing disclosure, a sync timeout over 60 s, more than 32 operations, a reserved error code |
| `service-call-builders` | build | Byte-identical `serviceCall` nodes, defaults omitted; refusals for a missing `input` or `inputDisclosure`, a sync `timeoutMs` over 60 s, `irreversible` with `compensateWith`, an unknown `cancelPolicy` |
| `service-call-runtime` | runtime | Async call succeeds; downstream step reads `steps.charge.output.receiptId`; a declared failure `CARD_DECLINED` and an undeclared one (`OPERATION_FAILED`) follow `onError` with effect `failed`; an invalid handler result fails `OPERATION_RESULT_INVALID` with effect `applied` and enters manual review; both journals normalized equal across SDKs |
| `service-call-cancellation` | runtime | `abandon`, `request_cancel`, `wait_cancelled` outcomes and effect classes on caller and handler; handler-side cancel gives `OPERATION_CANCELLED`; start timeout (`none`); caller deadline with the outcome found in the lookup window and with the window passing (`unknown`, manual review), and the late outcome visible on the operation |
| `service-call-sync` | runtime | Sync `quote` returns within its deadline; a slow handler gives `OPERATION_TIMED_OUT` and a stopped handler run; publication refuses `wait_cancelled` for sync |
| `service-call-security` | runtime | Non-allow-listed caller refused at publication (`SERVICE_CALLER_NOT_ALLOWED`) and at runtime (`OPERATION_NOT_ALLOWED`); foreign-tenant endpoint `NOT_FOUND`; revocation in flight gives `OPERATION_ACCESS_REVOKED` with no value and the handler's effect class; an operation input field looser than its caller source is refused (`INPUT_CLASSIFICATION_LOOSENED`); canaries absent from every SDK-visible answer |
| `service-call-faults` | runtime | Client retry of management writes with the same `Idempotency-Key` replays; changed body conflicts; a lost response on a run start that leads to a call yields one operation |
| `service-call-encoding` | build and runtime | With 031's data converter in the caller's workers: a `Service` document declaring `x-sagat-encoding` and a binding to a handler that declares encoding are refused (`ENCODING_NOT_APPLICABLE`); a `serviceCall` whose input maps an encoded caller field is refused at publication (`ENCODING_CARRIAGE_MISMATCH`); an envelope placed at an undeclared input location fails the step at admission, and one returned by a handler withholds the result, with `ENCODING_CARRIAGE_MISMATCH` or `ENVELOPE_CONTEXT_MISMATCH`; no envelope is stored in the other namespace |
| `service-call-compensation` (P3) | runtime | A saga compensates `charge` with `refund`, including after a timeout with effect `unknown`; a failing `refund` leads to manual review |

SDK client surface exercised (names per language idiom, semantics identical): publish a contract
revision, create/update/pause/resume/delete an endpoint, allow/remove a caller namespace, read the
registry, list and get operations, redrive a dead message. Every write sends an `Idempotency-Key`; a
transport retry never invents a new key (`sdks/SPEC.md` §5).
