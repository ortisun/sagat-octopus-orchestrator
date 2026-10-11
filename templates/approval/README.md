# approval

A person's decision is an event. `humanApproval` nodes exist in the schema and are not built yet; this is how
it is done today: the run waits for `approval.decided` with the same `requestId`, and `data.approved` says what
was decided.

```sh
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
sagat workflow run change-approval --input '{"requestId":"req-1"}'         # waits
curl -X POST "$SAGAT_SERVER/events" -H "authorization: Bearer $SAGAT_TOKEN" -H "x-octopus-namespace: $SAGAT_NAMESPACE" \
  -H 'content-type: application/json' \
  -d '{"type":"approval.decided","correlationKey":"req-1","data":{"approved":true}}'
```

Nobody decides for a day: the run takes the `expire` path.
