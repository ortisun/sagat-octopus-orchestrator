# compensation (a saga)

`reserve` then `charge`, inside a saga. Each step names its undo (`compensateWith`), and `charge` names how to find
out whether it happened when its answer is lost (`reconcileWith`).

```sh
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
sagat workflow run payment-saga --input '{"paymentId":"p-1","amount":10}' --wait
```

With the mock as it is, the charge is declined, the platform runs `limit.release` for the reservation, and the run
ends `COMPENSATED`: `sagat workflow inspect <run>` shows the steps in order, with the compensation. To make the
charge time out once, succeed, or lose its answer (`PROVIDER_TIMEOUT` with `retryable`), or the compensation itself
fail, stop `worker dev` (Ctrl-C), edit `mocks.yaml` and start it again: the file is read once, and a `sequence`
counts the calls of the whole life of the process, whichever run makes them. An answer goes under the section of
what it answers (`activities` for the steps, `compensations` for their undo, `reconcilers` for the lookups): the
tool warns about a step with no answer in its section. The engine is the real one: the retry, the compensation and
the reconciliation are the platform's own, not an emulation of them.
