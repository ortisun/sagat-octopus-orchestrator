# Executable SDK examples and parity

These are working programs against the SDKs in this checkout, not sketches of a future API.
Start with the [quickstart](../docs/quickstarts/README.md), then choose a use case.

| Use case | TypeScript | Python | Go | C# | Java |
|---|---|---|---|---|---|
| Compose and deliver a greeting | [Run](hello-world/typescript/README.md) | [Run](hello-world/python/README.md) | [Run](hello-world/go/README.md) | [Run](hello-world/dotnet/README.md) | [Run](hello-world/java/README.md) |
| Route accepted/rejected orders | [Run](order-processing/typescript/README.md) | [Run](order-processing/python/README.md) | [Run](order-processing/go/README.md) | [Run](order-processing/dotnet/README.md) | [Run](order-processing/java/README.md) |
| Parallel fulfillment | [Run](order-processing/typescript/README.md) | [Run](order-processing/python/README.md) | [Run](order-processing/go/README.md) | [Run](order-processing/dotnet/README.md) | [Run](order-processing/java/README.md) |
| Durable reminder and permanent failure | [Run](order-processing/typescript/README.md) | [Run](order-processing/python/README.md) | [Run](order-processing/go/README.md) | [Run](order-processing/dotnet/README.md) | [Run](order-processing/java/README.md) |
| Payment compensation and reconciliation | [Run](payment-saga/typescript/README.md) | — | — | — | — |

The [order-processing guide](order-processing/README.md) explains publication, capability binding,
inputs, expected outcomes and troubleshooting. One canonical set of workflow documents is shared
by every language. Workers implement activities; the server controls conditions, timers, joins,
retries and durable state. The examples simulate business providers and do not claim exactly-once
external effects.

The common executable check is:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored us03:: order_processing:: saga::
```

It requires Docker and Node, Python, Go, .NET and Java toolchains. The SDK protocol conformance
suite is separate in [`tests/contract/sdk-conformance`](../tests/contract/sdk-conformance/).
The [feature 014 specification](../specs/014-workflow-primitives/spec.md) proposes additional
primitives; those contracts are not available in the released SDKs or demonstrated as runnable here.
