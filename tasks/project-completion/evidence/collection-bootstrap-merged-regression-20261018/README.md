# Integrated byte-ceiling regression — 2026-10-18

Source `db2b04c6`; both the supplied-pool constructor refactor (`c7f62b4a`) and the immutable-old under-limit increment (`8b8046e5`) are integrated. This gate resolves the shared API startup regression risk.

The exact command and raw log are `runtime.json` and `runtime.txt`. GNU timeout supervises the whole cargo process: TERM at240s, KILL after5s; every nonzero exit is a failure. The owned external PostgreSQL16 is used with the runtime profile and two test threads.

**26 passed, 0 failed, 0 ignored**, 516 filtered out; 69.63s harness /76.45s wall including compilation. Ordinary/Applied successful output refusals, binding rechecks, transaction/receipt/outbox faults and collection/loop/End byte controls pass. All1125 native source input hashes match before/after. The manifest contains only allowlisted test settings, no environment dump.

This is a regression result, not evidence of actual lost-COMMIT recovery, general nesting, deployment, full workspace or canonical task completion.
