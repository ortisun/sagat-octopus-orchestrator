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

### INVALID_SCOPE

A forEach, while or repeatUntil body is not well formed (the reason says how): a body is a chain of one to eight activities ending in a return, it stays inside its own scope and nothing outside jumps into it.

### DUPLICATE_BODY

Two entries of `bodies` have the same name. Body names are unique in a document.

### UNKNOWN_BODY

A forEach, while or repeatUntil node names a body that `bodies` does not define. Add the body or fix the name.

### UNREACHABLE_BODY

A body is defined and no node uses it. Remove it, or reference it from a forEach, while or repeatUntil node.

### RECURSIVE_BODY

A body refers to itself, directly or through other bodies. Bodies nest but never form a cycle.

### BODY_DEPTH_EXCEEDED

Bodies nest deeper than the limit (8 levels). Flatten the nesting or move the inner part into a child workflow.

### INVALID_WORKFLOW_LIMIT

A top-level `limits` value is outside its range. A workflow may lower maxInstances and the depth below the platform and tenant ceilings and never raise them.

### INVALID_CONTINUATION_POLICY

The chainPolicy of a workflow that continues as new is not valid: maxSegments is 1 to 10000 and a timeout is required. Only a workflow with a continueAsNew node declares one.

### INVALID_SWITCH

A switch needs at least one case, a default route, and unique case ids; each case has a predicate and a target node (the reason says which one is missing).

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
