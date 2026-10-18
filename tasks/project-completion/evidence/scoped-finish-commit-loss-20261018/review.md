Fresh Astra C1 required cancellation cleanup after actual blocker resolution and a pre-recovery original-result oracle. Root fixed both: SQLx queued rollback is observed after original COMMIT; committed full capacity snapshot is captured before cold replay, linked result IDs/time bind to that retained receipt, and both branches preserve their resolved snapshot across the following replay. C2 None atfae87f9977bbb78c61fa3d92f01a7d7c326ebd69b11a1b9603cf923845043f73.

Read-only reviewer executed no tests or writes. Root inspected and integrated the artifact,
with formatting only unless explicitly stated. Optional external review was declined by user.
