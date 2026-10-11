# compensation (a saga)

`reserve` then `charge`, inside a saga. Each step names its undo (`compensateWith`), and `charge` names how to find
out whether it happened when its answer is lost (`reconcileWith`).

```sh
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
sagat workflow run payment-saga --input '{"paymentId":"p-1","amount":10}' --wait
```

With the mock as it is, the charge is declined, the platform runs `limit.release` for the reservation, and the run
ends `COMPENSATED`: `sagat workflow inspect <run>` shows the steps in order, with the compensation. Edit
`mocks.yaml` to make the charge time out once, succeed, or lose its answer (`PROVIDER_TIMEOUT` with `retryable`),
and to make the compensation itself fail. The engine is the real one: the retry, the compensation and the
reconciliation are the platform's own, not an emulation of them.
