# code-graph-mixed

What it proves (design 044, Verification Plan): a code parent, a graph child, a code grandchild.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow shipment(input):                                   // code version of shipment
  result = await child("bulk-labels", {orders: ["ord-1", "ord-2", "ord-3"]},
                       {cancelPolicy: "propagate", businessId: "labels-batch-42"})
  return {batchId: input.batchId, printed: result.printed}

graph bulk-labels (version 3):                              // a graph workflow, no code
  forEach item in input.orders: child node "print" starts label-print with {orderId: item}
  returns {printed: count}

workflow labelPrint(input):                                 // code version of label-print
  label = await activity("labels.render", {orderId: input.orderId, format: "zpl"})
  return {orderId: input.orderId, labelId: label.labelId}
```

## Runs

- this directory: the code parent: one child command, then its child.finished.
- `grandchild/`: the code grandchild of ord-1, started by the graph child's child node (one of three).

## What the golden shows

- The parent sees only the graph child's child.finished; the graph child's own journal is not code history.
- The grandchild is an ordinary code run whose start version is a code version named by a graph child node
  (Requirement 044-018).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The graph child is described (`expectations.graphChild`), not given as a full document.
- A child's terminal status in HistoryChildFinished is the string COMPLETED.
