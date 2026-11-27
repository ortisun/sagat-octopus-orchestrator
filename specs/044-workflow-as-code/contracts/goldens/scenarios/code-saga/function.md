# code-saga

What it proves (design 044, Verification Plan): a saga that completes, one that aborts and compensates in reverse dependency order.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow booking(input):
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      await s.activity("inventory.reserve", {orderId: input.orderId})
      await s.activity("payments.charge", {orderId: input.orderId, amountCents: 12000}))
  try:
    await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
        hotel = await s.activity("hotel.book", {tripId: input.tripId})       // envelope: compensateWith hotel.cancel
        await s.activity("car.rent", {tripId: input.tripId, hotelBookingId: hotel.hotelBookingId})
                                                                             // envelope: compensateWith car.return
        await s.activity("flight.book", {tripId: input.tripId}))            // fails NO_SEATS
  catch SagaCompensated:
    return {orderId: input.orderId, order: "charged", trip: "compensated"}
```

## What the golden shows

- open saga takes `s-<n>`, n its own command number; activities scheduled in it carry `sagaScope`
  (Command.saga_scope), which their body hashes as `scope`.
- The first scope closes (command 4) once its members settled; the second aborts (command 9) after flight.book
  failed. The compensations are derived steps, not visible; the function sees the saga outcome `compensated`.
- `expectations.compensationOrder`: car.return before hotel.cancel, from the (issuedSeq, outcomeSeq) pairs.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- Closing a scope writes only the close command's record (journal saga.completed); a HISTORY_SAGA_OUTCOME
  exists only after an abort (compensated, partial or compensation_failed).
