# Actual C2 failure

One test ran and failed, exit101,76.71s including1m09s compilation; harness4.40s. The fresh-full binding check returned public PermissionDenied / NOT_BOUND exactly as expected. Exact native unbind audit/lease change/physical content refund comparisons passed. The final no-audit snapshot oracle failed.

Source inspection of grpc/service.rs325–341 shows observe awaits the normal access-denial audit before returning PermissionDenied. application/identity.rs499 records access.denied.permission for endpoint grpc WorkerService/CompleteStep with reason refused: NOT_BOUND. A read-only query of the owned retained test database found one such audit while Run remains RUNNING, no stop, version117.

The first contract incorrectly prohibited every report-associated audit. Correct policy preserves this exact security denial audit; no successful-refusal audit, effect, root stop, receipt or payload is permitted. C3 must bind the denial to this call's known trace and compare all other state unchanged. C2 fresh bounded review found no remaining issue against the incomplete contract; this runtime failure refines that contract rather than proving a production security bug.
