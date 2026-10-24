# Use a managed PostgreSQL connection from an SDK

An administrator creates an OpenBao credential and a connection in **Settings →
Connections**. The workflow references the connection's name in its namespace.
Publication, admission and execution recheck current permissions. The connection
must be active; a reference may also select an immutable revision.

The server must support the `database` IR node and have an authorized
[`database-runner`](../runbooks/database-runner.md). Ordinary SDK workers continue
using their own protocol minor (1.5 for the scoped identity, older ones still served); the separate managed runner uses protocol 1.6. SDK workers do
not execute the SQL or receive its credential. An older server rejects unsupported
IR during validation/publication.

## Author the same operation in each language

Each example emits a query node using the existing `orders-db` connection. SQL
values are positional bindings. Integer and decimal literals are exact strings;
binary is base64, timestamps use RFC3339, and dates use `YYYY-MM-DD`.

### TypeScript

```ts
import { workflow } from '@sagat-octopus-orchestrator/sdk';

const definition = workflow('order-query')
  .database('order', {
    connectionRef: { name: 'orders-db' },
    operation: 'query',
    statement: 'SELECT $1::bigint AS order_id',
    parameters: [{ literal: { type: 'integer', value: '9007199254740993' } }],
  })
  .build();
```

### Go

```go
import "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"

definition, err := workflow.New("order-query").Database("order", workflow.DatabaseOptions{
    ConnectionRef: workflow.ConnectionRef{Name: "orders-db"},
    Operation: workflow.DatabaseQuery,
    Statement: "SELECT $1::bigint AS order_id",
    Parameters: []workflow.DatabaseParameter{{Literal: &workflow.DatabaseValue{
        Type: workflow.DatabaseInteger, Value: "9007199254740993",
    }}},
}).Build()
// Handle err before publishing definition.
```

### Python

```python
from sagat_octopus_orchestrator import workflow

definition = workflow("order-query").database(
    "order",
    connection_ref={"name": "orders-db"},
    operation="query",
    statement="SELECT $1::bigint AS order_id",
    parameters=[{"literal": {"type": "integer", "value": "9007199254740993"}}],
).build()
```

### C#

```csharp
using SagatOctopusOrchestrator.Sdk.Workflows;

var definition = Workflow.Create("order-query").Database("order", new DatabaseOptions
{
    ConnectionRef = new ConnectionRef("orders-db"),
    Operation = DatabaseOperation.Query,
    Statement = "SELECT $1::bigint AS order_id",
    Parameters = [DatabaseParameter.Literal(DatabaseType.Integer, "9007199254740993")],
}).Build();
```

### Java

```java
import dev.sagatoctopusorchestrator.sdk.workflow.DatabaseOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.WorkflowBuilder;
import java.util.List;

var definition = WorkflowBuilder.named("order-query").database("order",
    DatabaseOptions.create(new DatabaseOptions.ConnectionRef("orders-db"),
        DatabaseOptions.Operation.QUERY, "SELECT $1::bigint AS order_id")
    .parameters(List.of(DatabaseOptions.Parameter.literal(
        DatabaseOptions.Type.INTEGER, "9007199254740993"))))
    .build();
```

Publish and start the document through the existing workflow/run APIs. No SDK
connection string, database driver, password or OpenBao endpoint is required.

## Bind input and earlier results

The canonical expression binding is
`{"type":"integer","expression":"input.orderId"}`. In C#/Java use the
`DatabaseParameter.Expression` / `DatabaseOptions.Parameter.expression` factory;
in Go set `Type` and `Expression` with no `Literal`.

An expression can read an earlier **inline** database output, for example
`steps.order.output.rows[0][0].value`. Values stay typed through the binding; SQL
identifiers and SQL fragments are not interpolated. Caller-owned values are
snapshotted when the database node is added to the builder.

For inline results (up to 256 KiB), the ordinary activity context exposes the retained result through its existing
steps map (`ctx.steps.order` in TypeScript). It is the same JSON in every SDK:

```json
{
  "columns": [{ "name": "order_id", "type": "integer" }],
  "rows": [[{ "type": "integer", "value": "9007199254740993" }]],
  "affectedRows": "1",
  "truncated": false
}
```

Larger results use the existing `$blob` descriptor in the steps map; SDK contexts
do not expand it automatically. An authorized ordinary activity must use its
lease-bound blob reader (`ctx.readBlob` in TypeScript), finish digest/length
verification, enforce a finite decoding budget and parse the result JSON before
using rows. Database expression bindings cannot dereference blob contents. Choose
smaller result limits when a later database parameter needs a direct expression.
The shared database scenario below verifies inline downstream consumption; generic
SDK blob transport has its own conformance scenario.

Do not convert exact integer/decimal strings to floating point. Columns retain
their order, including duplicate names. A SQL null is a tagged `null` cell even
when its column has another declared type. Unsupported PostgreSQL types and
overflow fail with a safe typed category; partial success is not returned.

## Limits and writes

Optional `limits` can narrow row, result-byte, statement-byte, parameter, timeout
and connection budgets using the canonical IR names. The server and runner apply
the minimum of their policies and the requested limits. The SDK performs basic
shape validation; the server owns SQL classification and authorization.

`operation: execute` selects an explicit write and requires write permission.
Writes have no automatic retry. An ambiguous external commit enters manual review;
a Sagat request key alone does not deduplicate SQL. Reconcile the customer database
before choosing to execute another write. DDL, multiple statements, transaction
control and administrative SQL are refused. Use restricted PostgreSQL roles and
the runner's network policy as described in the runbook.

Stored output is available to later authorized execution steps. Interactive
disclosure uses the separately audited, bounded result endpoint and requires
`database:result:read`; ordinary run-read and connection-use rights do not grant
that view. Avoid logging SQL, parameter values or database results in application
handlers.

## Reproduce the shared scenario

```sh
cargo build -p sagat-database-runner --bin database-runner --locked
SAGAT_SDK_SCENARIO=database-connections cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

Requires Docker plus Node, Go, Python, .NET and Java toolchains. The fixture starts
OpenBao 2.7.1, PostgreSQL 16, the platform and a separate runner for each SDK. It
compares canonical documents and invalid inputs, exact typed results, an expression
bound to an earlier result, ordinary SDK result consumption, one retained write
and refused DDL. Independent assertions run even when regenerating goldens.
