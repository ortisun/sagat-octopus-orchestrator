# Worker missing-output contract review

C1 required finding accepted: a merely nonempty compensation target key could match another effect. C2 compares both the target idempotency key and complete target instance with the actual refused forward lease. The reviewing agent materialized that exact correction and confirmed no other behavioral changes; its C1 review had no other required findings.

The handler parses actual retained/refused TaskLease input with one strict parser and uses only its parsed reservation ID to release a test-owned in-memory reservation. Missing output produces a fixture-defined typed FailStep, not a new engine precheck. Authorized FAILED resolution retains the failure and produces COMPENSATION_FAILED, never compensated. This is current-version worker/engine behavior, not a provider or old-process guarantee.

C2 runtime passed, but strict Clippy rejected two large query tuple types. Root mechanically replaced those two query results with named `sqlx::FromRow` structs and corresponding SQL column aliases; assertions retain the same values with named fields. C3 stores its separate executed patch and native source pins. C2 failures/results were retained and not relabeled as final-source gates.
