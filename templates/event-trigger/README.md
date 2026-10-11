# event-trigger

A run is started for each `order.created` event. The run's input is the event document, `{"event": {id, source, type, ...}, "data": {...}}`: the data you published is under `data`, and what a step reads of it is `input.data.orderId`.

```sh
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
curl -X POST "$SAGAT_SERVER/events" -H "authorization: Bearer $SAGAT_TOKEN" -H "x-octopus-namespace: $SAGAT_NAMESPACE" \
  -H 'content-type: application/json' -d '{"type":"order.created","eventId":"order-1","data":{"orderId":"o-1"}}'
sagat workflow runs on-order-created          # the run the event started
sagat workflow inspect <run>
```

Sending the same `eventId` again starts nothing: the event is recognised.
