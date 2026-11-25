# Errors a document can have, and what to do about each

`sagat workflow validate`, `sagat workflow publish`, `sagat validate` and `sagat publish` check a document with the same
codec the platform uses, and print each problem as

```text
error[CODE]: what is wrong
  --> where (a node, or a JSON pointer into the document)
  help: what to do
  docs: docs/quickstarts/errors.md#code
```

The `help` and the `docs` line are told once for each code in a run, not again for the next problem of the same code.
Set `SAGAT_DOCS_URL` to the address this documentation is published at and the `docs` line becomes a link to it.
A `warning` does not stop a publication; an `error` does. The codes below are the ones the codec and the platform's
validation return. Two tests keep this page honest: one fails if a code the codec can return has no section here (or
the tool has a help for a code the codec no longer returns), and one fails if the sentence under a code is not the
`help:` line the tool prints.

Problems that only the platform can find (a secret you may not use, a script nobody built) are found by
`sagat workflow validate` when it can reach the platform, and by publishing.

### DOCUMENT_TOO_LARGE

A workflow document is at most 1 MiB. Move what repeats into child workflows and keep large constants out of the document.

### DOCUMENT_MALFORMED

The file is not one JSON or YAML object. Check the syntax at the line the parser names; a workflow is an object with apiVersion, kind, metadata, entrypoint and nodes.

### SCHEMA

The document does not match the workflow schema at the path shown (a JSON pointer). The schema is specs/001-platform-foundation/contracts/workflow.schema.json; `sagat new --list` has documents that match it.

A node of type `activity` that does not say which activity:

```json
{ "id": "charge", "type": "activity", "next": "ship" }
```

```text
error[SCHEMA]: "activity" is a required property
  --> at /nodes/0
```

Add `"activity": "payments.charge"`. The pointer counts from zero: `/nodes/0` is the first node of the list.

### UNSUPPORTED_NODE

This node, or this combination of nodes, is in the schema but cannot be published yet: for example an approval inside a forEach or while body (the error's dependency names what it waits for), or a shape the enabled subset does not allow. Move the part that is not allowed into a child workflow.

Collections, loops, approvals, switches and Continue-As-New are built, each in the shapes the feature's quickstart lists. What is refused is the rest: an approval inside a body (it waits for scoped compensation, `dependency: SCOPED_COMPENSATION`), a branch, timer or nested scope inside a body, and a node kind the reading platform does not enable.

### EXPRESSION

An expression does not compile in the bounded expression language. Fix the syntax; there are no calls, no loops and no assignments.

```text
error[EXPRESSION]: node 'c', field 'expression': syntax error at position 13: a value
  --> node 'c'
```

The field named in the message holds an expression that does not compile: here `input.amount >` has nothing to the right of the `>`. Expressions read the input (`input.orderId`), the outputs of steps (`steps.charge.output.chargeId`) and the run (`run.id`), compare and combine; they do not call functions and do not assign.

### NAME_MISMATCH

metadata.name must be the name the workflow is published as. Rename the document or the workflow so they agree.

The platform publishes a document as the workflow of its `metadata.name`; the workflow it is published to has to have that name. When they differ nothing is published.

### UNKNOWN_SCRIPT

A script node pins an artifactDigest that no ready script of this tenant has. Publish the script (`sagat script publish`) and use the digest it prints.

```bash
sagat script publish scripts/enrich.ts          # prints: artifactDigest sha256:...
```

Paste the digest into the `script` node's `artifactDigest`.

### INVALID_DATABASE_NODE

Check the connection reference, query or execute operation, typed parameters and finite limits. Managed database steps cannot declare automatic retries or impersonate an ordinary activity. A managed database step belongs to the root graph of a definition without loops, collections or a continuation policy.

Use a namespace connection with an OpenBao credential and positional SQL parameters.
Integer and decimal literal values are strings; expression bindings must declare their type.
See [managed database SDK examples](../guides/managed-database-sdk.md) for the canonical shape.

### LAKE_DISABLED

This deployment has not enabled the lake, so a workflow cannot hold an Apache Iceberg write. Ask the operator to turn on SAGAT_LAKE_ENABLED, or publish the workflow to a deployment that has it.

The lake is off by default. An operator turns it on with `SAGAT_LAKE_ENABLED=true` on the control plane; publishing an
Apache Iceberg write is then judged by the checks of access, allowed modes and data classification. A run that carries one is
admitted, and its step leased, only by a process that has the same setting: set it on every execution-engine replica too
(`SAGAT_LAKE_ENABLED`), or the run is refused where it is not set; so is the process that serves the worker gRPC, which hands
the credentials of the step to the lake runner. A lake runner is handed the step and its credentials, but the runner that carries out
the attempt is not built yet, so a deployment should not turn the lake on to write to a table.

### CONCURRENCY_POLICY_INVALID

A concurrency declaration breaks a rule: a limit from 1 to 10000, a group matching ^[a-z][a-z0-9.-]{0,63}$, a queue timeout from 1 ms to one day, a top-level key that reads only `input`, only on a root activity or script node, and at most 16 node-level declarations per definition unless the namespace's flow_control_node_declarations allows more (64 at most).

### RATE_LIMIT_POLICY_INVALID

A rate-limit declaration breaks a rule: `per` is second, minute or hour, the limit is at least 1 and at most 100 per second, 6000 per minute or 10000 per hour, it stands at the top level or on a root activity or script node, and a definition holds at most 16 node-level declarations unless the namespace's flow_control_node_declarations allows more (64 at most).

### FLOW_CONTROL_COMBINATION

A concurrency or rate-limit declaration cannot be combined with continueAsNew: a continuation's successor would start without passing the gate. Remove one of them.

### FLOW_CONTROL_KEY_UNBOUNDED

A concurrency or rate-limit key reads the run's identity (`run`), alone or beside other data: every run would have a key of its own, which the namespace keeps a row for. Key on what the gate protects (an account, a provider) read from `input`, or from `steps` and `state` on a step.

### FLOW_CONTROL_KEY_SECRET

A concurrency or rate-limit key reads a location classified secret (an `x-sagat-classification` of the input schema or of a step's output schema): its value would reach a key digest and a label. Key on a field that is not secret, or classify the field you key on.

### FLOW_CONTROL_KEY_PII

A concurrency or rate-limit key reads a location classified pii, or one no schema classifies (which is pii): published, and its key label is shown only to callers who may reveal personal data in the namespace. Classify the field `internal` or `public` in its schema if it is not personal data.

### CONCURRENCY_SELF_CHILD

A child node starts this workflow, whose run-level concurrency key the child may share with its parent: with a limit of 1 the child waits for the permit its parent holds until the queue timeout. Use another group, or another key, for the child.

### CONCURRENCY_LIMIT_MISMATCH

Another published workflow of this namespace declares the same explicit concurrency group with a different limit. Both are accepted and each run applies its own limit, so the group admits runs unevenly; use one limit for the group, or a group of its own.

### RATE_LIMIT_MISMATCH

Another published workflow of this namespace declares the same explicit rate-limit group with a different limit or window. Both are accepted and each run counts the grants against its own limit and window; use one declaration for the group, or a group of its own.

### FLOW_CONTROL_ACTIVATION_PENDING

The capability this workflow uses (concurrency keys or rate limits) is switched on but not activated for the fleet: the operator activates it with `control-plane flow-control-activate` once every control plane and engine runs it.

### COLLECTION_OPERATORS_DISABLED

This deployment has not enabled collection operators, so a forEach cannot declare operators. Ask the operator to turn on SAGAT_COLLECTION_OPERATORS_ENABLED once every replica runs 0.3.0, or shape the list in an activity before the collection.

The switch is read by publication only (design 042): with it off, a version published while it was on stays readable,
can be activated, rolled back and promoted where it already runs, and its runs apply their chain at scope entry. The
fleet rule and the ceilings of a chain are in `docs/runbooks/workflow-primitives.md` ("Collection operators").

At run time, a chained collection that crosses one of its running ceilings at scope entry fails the run with the
failure code `COLLECTION_OPERATOR_BUDGET`, before any iteration starts and without truncating anything: 1,000 elements
in the chain's input and in each stage's output, 256 KiB per element, 4 MiB per stage, 1 second of wall time. The reason
label of `scheduler_collection_source_failures_total` names which. A stage expression that fails, a `filter` that is not
boolean and a `flatMap` that is not a list fail the run with `EXPRESSION_ERROR` instead.

### INVALID_OPERATOR

A stage of a forEach's operators is refused at the index shown. A collection has filter, map, flatMap, skip, take, distinct and window (debounce and ignoreElements are a trigger's); a stage expression reads element, position, input, steps, run and state, never item, index, iteration, carry or bodyResult.

The details say which stage (`stage`, from 0), which `operator`, and why (`refusal`: `UNKNOWN`, `SURFACE`, `RANGE`,
`EXPRESSION` or `VARIABLE`). The body still sees `item` and `index`, the element's position in the chain's output.

### INVALID_ICEBERG_NODE

Check the connection reference, the table, the record source (exactly one of inline, blobs or a database result), the mapping and the limits. A database result must name a database step that runs before the write. An Apache Iceberg write cannot declare its own retries or effect, impersonate an ordinary activity, or name a field and a column twice.

### MODE_NOT_ALLOWED

The connection of this Apache Iceberg write does not allow the mode it writes in. Write in a mode the connection allows, or ask the owner of the connection to allow this one in a new revision.

A new connection has `append` only. The refusal names the node and the mode; the owner of the connection changes the allowed
modes in a new revision.

### CLASSIFICATION_EGRESS_REFUSED

The connection of this Apache Iceberg write does not accept the classification of a field it would send, or the fields were never classified. Map fewer fields, classify the source with x-sagat-classification and refer to it directly (input.a.b or steps.<id>.output.a.b), or use a connection whose ceiling accepts them. A secret field never leaves.

The refusal lists each field by its path and class and never by its value. A field built by an expression, taken from a list of
blobs or from a database result counts as `pii`, because the platform cannot follow its classification there.

### ICEBERG_NODE_NOT_ROOT

An Apache Iceberg write belongs to the root graph of a definition without loops, collections or a continuation policy, and never inside a body. Move the write out of the loop, or into a child workflow.

### SECRET_UNAVAILABLE

A node names a secret that does not exist in the namespace, or that you may not use. Create it there, or ask an administrator for access to it.

The platform does not say which of the two it is (it does not exist, or you may not use it): somebody who may not use a secret must not learn which names exist. Ask the owner of the namespace.

### INVALID_NODE_ID

A node id is up to 64 characters of letters, digits, '_', '.' and '-', starting with a letter or a digit.

### INVALID_ACTIVITY_NAME

An activity name is up to 128 characters of letters, digits, '_', '.', ':' and '-', starting with a letter or a digit, like payment.validate.

### INVALID_EVENT_TYPE

An event type is up to 128 characters of letters, digits, '_', '.', ':' and '-', starting with a letter or a digit, like payment.confirmed.

### INVALID_EXPRESSION

An expression is not empty, is at most 2048 bytes and has no control characters.

### NO_NODES

A workflow needs at least one node, and the entrypoint names one of them.

### TOO_MANY_NODES

A workflow has at most 10000 nodes. Split it into child workflows.

### TOO_MANY_PARALLELS

A workflow has at most 32 parallel nodes. Move some branches into child workflows.

### TOO_MANY_SECRETS

A workflow declares at most 128 distinct secrets. Share one secret between the nodes that need the same credential.

### DUPLICATE_NODE

Two nodes have the same id. Ids are unique in a document.

### UNKNOWN_ENTRYPOINT

entrypoint must be the id of a node of this document.

```text
error[UNKNOWN_ENTRYPOINT]: entrypoint 'zzz' is not a node of the workflow
```

### UNKNOWN_TARGET

A node points (next, onError, onTimeout, begin, a branch) at an id that is not a node. Fix the spelling or add the node.

```json
{ "id": "validate", "type": "activity", "activity": "order.validate", "next": "chrage" }
```

```text
error[UNKNOWN_TARGET]: node 'validate' refers to 'chrage', which does not exist
  --> node 'validate'
```

`chrage` is a typo of `charge`.

### CYCLE

The graph loops back through the node shown. A workflow has no cycles: express a retry with a retry policy and a repetition with a child workflow.

```text
error[CYCLE]: the graph has a cycle through node 'a'
  --> node 'a'
```

`a` points to `b` and `b` points back to `a`. Give the step a `retry` policy for a retry, or start a child workflow for each repetition.

### UNREACHABLE

No path from the entrypoint reaches this node. Connect it to the graph or remove it.

```text
error[UNREACHABLE]: node 'b' cannot be reached from the entrypoint
  --> node 'b'
```

Nothing points at `b` and it is not the entrypoint. Either connect it from the node before it (`next`) or delete it.

### INVALID_RETRY

A retry policy has 1 to 100 attempts, an initial delay from 1 ms to 24 h and a maximum delay that is not below the initial one.

```json
"retry": { "maxAttempts": 5, "initialDelayMs": 500, "maxDelayMs": 30000, "strategy": "exponential", "jitter": true }
```

A `maxAttempts` of `0` is a schema problem (`0 is less than the minimum of 1`); a `maxDelayMs` below `initialDelayMs` is this one.

### DURATION_OUT_OF_RANGE

A timer or a timeout is from 1 ms up to the maximum shown in the message for its kind.

### INVALID_BRANCH_COUNT

A parallel node has 2 to 256 distinct branches.

### BRANCH_OVERLAP

Two branches of a parallel node run through the same node. Give each branch nodes of its own; they meet at the join.

### BRANCH_BOUNDARY

A node inside a parallel branch is also reached from outside it. A branch is entered only through its parallel node.

### END_INSIDE_BRANCH

An end node cannot sit inside a branch of a parallel node: a branch finishes at the join.

### SAGA_BOUNDARY

A node inside a saga is also reached from outside it. A saga is entered only through its begin node.

### END_INSIDE_SAGA

An end node cannot sit inside a saga: let the saga's next carry on.

### TOO_MANY_SAGAS

A workflow has at most 32 saga nodes. Move some into child workflows.

### IRREVERSIBLE_WITH_COMPENSATION

A node cannot be irreversible and have a compensation too. Remove one of the two.

### EFFECT_ON_NON_ACTIVITY

irreversible, highRisk, the compensation retry and the compensation timeout only apply to an activity node.

### INVALID_TRIGGER

Fix the trigger as the message says; examples of every kind are in docs/runbooks/events.md and in `sagat new event-trigger` and `sagat new cron`.

### TOO_MANY_TRIGGERS

A workflow has at most 16 triggers.

### DUPLICATE_TRIGGER

The same trigger is declared twice. Remove one.

### COMPENSATION_OUTSIDE_SAGA

compensateWith only runs inside a saga. Put the node in a saga, or remove the compensation.

### RECURSIVE_CHILD

This child node starts the workflow it belongs to. It is legal, and the depth limit ends it at the latest: make sure a condition stops it before.

### INVALID_SCOPE

A structured node has an invalid body, target or boundary. Keep each loop or approval inside its named body and follow the location named by the message.

### DUPLICATE_BODY

Two structured bodies have the same name. Body names are unique in a workflow.

### UNKNOWN_BODY

A structured node refers to a body that the workflow does not declare. Fix the body name or add that body.

### UNREACHABLE_BODY

The workflow declares a body that no structured node uses. Reference it from a node or remove it.

### RECURSIVE_BODY

Structured bodies cannot refer back to themselves through a chain of body references. Break the cycle into a finite structured flow.

### BODY_DEPTH_EXCEEDED

Body references exceed the workflow's maximum nesting depth. Flatten the structured flow or raise maxDepth within the supported limit.

### INVALID_WORKFLOW_LIMIT

Set maxInstances from 1 to 100000 and maxDepth from 1 to 8.

### INVALID_CONTINUATION_POLICY

Set maxSegments from 1 to 10000 and a timeout from 1 millisecond to 366 days; a chain policy also needs a continueAsNew node.

### INVALID_SWITCH

A switch has 1 to 32 uniquely named cases with bounded predicates, and it does not declare next, onError or timeout.

### CLASSIFICATION_INVALID_LEVEL

Use one of the four classification levels: public, internal, pii or secret.

### CLASSIFICATION_LOOSENED

A child cannot be less restrictive than its effective parent. Raise the child level or lower the parent only after reviewing every child.

### CLASSIFICATION_UNSUPPORTED_KEYWORD

Move the classification annotation to a supported schema location such as a carrier root, property, item or local reference target.

### CLASSIFICATION_UNRESOLVED_REF

Use a local JSON Pointer that resolves inside this carrier. Remote and cross-carrier references are not accepted.

### CLASSIFICATION_NOT_APPLICABLE

Classification annotations apply only to inputSchema, outputSchema and supported node outputSchema carriers. Remove the annotation from this location.

### CLASSIFICATION_TOO_COMPLEX

Keep the classification carrier within the fixed schema size, depth and reference-expansion limits.

### CLASSIFICATION_INHERITED_OPEN

A declared child inherits public or internal access. Declare a stricter level when that child should not stay open.

### INPUT_SCHEMA_KEYWORD_UNSUPPORTED

The inputSchema uses a keyword outside the supported subset, or one of the subset in a form it does not support (a tuple items, a boolean exclusive bound, a fractional multipleOf, a nested $id). Rewrite that keyword with the subset's keywords: see the subset in docs/designs/043-run-input-validation.md.

### INPUT_SCHEMA_REFERENCE_INVALID

A $ref of the inputSchema is not a local JSON pointer (#/...) to a schema of the document, has sibling keywords other than annotations, or closes a cycle. Point it at a $defs entry and move its siblings into that entry or an allOf.

### INPUT_SCHEMA_PATTERN_UNSUPPORTED

A pattern of the inputSchema is outside the supported syntax (no \d, \w, \s, \b, Unicode or POSIX classes, flags or named groups), past 512 NFA states or 16 KiB of program, or one of more than 32. Write classes such as [0-9] and pair the pattern with a maxLength.

### INPUT_SCHEMA_TOO_LARGE

The inputSchema is larger than 64 KiB. Move what repeats into $defs and refer to it, or check less of the input here.

At publication a schema over 64 KiB, or nested deeper than 32, is answered `CLASSIFICATION_TOO_COMPLEX` instead: the
classification bound of feature 016 reads every carrier first (G043-020, D043-112). The schema is refused either way;
`INPUT_SCHEMA_TOO_LARGE` and `INPUT_SCHEMA_TOO_DEEP` are what the start gate and `control-plane input-schema-inventory`
answer for a version already stored.

### INPUT_SCHEMA_NAME_TOO_LONG

A name under properties or required of the inputSchema is longer than 1 KiB. Use a shorter property name; a lookup compares names, and a long one costs every input that has it.

### INPUT_SCHEMA_TOO_DEEP

The inputSchema nests deeper than 32 or applies schemas through more than 128 levels of $ref and combinators. Flatten it: refer to $defs instead of nesting, and shorten chains of references.

### INPUT_SCHEMA_TOO_COSTLY

Every input this inputSchema accepts would cost more than the validation work budget (for example a wide allOf tree of references over a large enum). Reduce the fan-out of the references or the size of the enum.

### INPUT_SCHEMA_APPLIES_TO_TRIGGERS

This version has an inputSchema and a trigger: the schema is applied to the input a trigger builds, the event envelope {event, data}. Make sure the schema accepts the envelope, or the triggered runs are refused.

### VALIDATOR_BUSY

The platform could not pre-check the inputSchema now: the run-input validator had no permit free within 5 s, or it failed with a defect of its own. Nothing was judged; send the document again after a pause, and report it if it keeps happening.

Through the API the same case is a retryable `503 UNAVAILABLE` (design 043, D043-177, D043-183).
