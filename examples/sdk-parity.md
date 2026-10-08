# SDK exercises — proposed contracts, illustrative pseudocode

> The API is aspirational and subject to the final proto; the examples are not executable binaries in this package.

## TypeScript

```typescript
const run = await client.runs.start('corporate-payment', input, {
  idempotencyKey: paymentId, namespace: 'prod'
});
await client.runs.signal(run.runId, 'payment-confirmed', { paymentId });
```

## Go

```go
run, err := client.StartRun(ctx, "corporate-payment", input,
    sagatoctopusorchestrator.StartOptions{Namespace: "prod", IdempotencyKey: paymentID})
```

## Python

```python
run = await client.start_run("corporate-payment", input_data,
                             namespace="prod", idempotency_key=payment_id)
```

## C#/.NET

```csharp
builder.Services.AddSagatOctopusOrchestrator(options => options.Endpoint = new Uri("https://api.example"));
var run = await client.StartAsync("corporate-payment", payment,
    new StartOptions("prod", payment.Id), cancellationToken);
```

## Java

```java
var run = client.startRun("corporate-payment", input,
    StartOptions.builder().namespace("prod").idempotencyKey(paymentId).build());
```

## Conformance scenarios shared by all SDKs

1. Start with the same idempotency key + same input → same run.
2. Same key + different input → CONFLICT.
3. Worker with stale generation → STALE_ATTEMPT.
4. Cancel / deadline / trace propagation.
5. Ambiguous side effect → reconcile callback required per policy.
6. Protocol compatibility minor and major rejection.
7. Decimal, UTC timestamp, null, missing field and arrays in a cross-language payload.
