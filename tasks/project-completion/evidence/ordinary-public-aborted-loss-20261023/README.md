# Actual aborted public refusal COMMIT reply loss — 2026-10-23

Accepted baseline16ea6a37, decisiond614b2a5, one changed child test module. The existing committed and aborted cases share the original two-gate selection, request, safe error mapping, complete teardown and fresh-API recovery sequence. There is no production, relay/proxy, schema/grant, SDK or golden change.

For abort only, the exact deferred receipt trigger acquires Gate B then raises fixed SQLSTATE23514/constraint capacity_finish_proxy_forced_abort, recognized by the unchanged wire relay. As before, Gate A selects the captured original receipt writer before arming; SAME PID must be executing actual COMMIT behind B. Client reply loss occurs while B remains held, returning existing safe UNAVAILABLE/retry5000; original Run lock and invisible pre-state are observed again BEFORE B releases. Backend ErrorResponse23514 with the exact constraint and ReadyForQuery idle independently confirm actual abort.

BEFORE ANY RETRY, owner reads require canonical serialized full snapshot byte equality, exact original live leased row/protobuf/session/expiry, no original receipt/stop/success/audit, same original journal/Run/telemetry head and released Run guard. Snapshots include all source/payload/usage/retained/iteration ledgers, complete authority, scopes/sagas/chains, capacity tables and private quota transaction deltas without granting runtime access. Safe hashes/counts from this stage are retained before later recovery overwrites its state; see final/pre-retry-rollback-evidence.json.

Faulted clients/API/store/auth caches and pools close; all gate triggers/functions are removed and asserted absent before another full API/direct sagat_app-pool reopen on the same DB. The entire rollback witness is checked again before submission. Original protobuf/key/session/worker capability is reused with current authentication, without registration/repoll/renewal/state repair. The first successful response is newly applied, SUCCEEDED/CANCELLING. Independent owner SQL then validates canonical receipt, original trace, first stop/genuine success, journal/telemetry/Run head and source preservation; the actual ACK is compared with this independent evidence. Only then are fields frozen for a second exact already-applied/no-effect replay. Fresh runtime-role journal/path reads are read-only. The committed branch retains its distinct receipt-derived expected ACK BEFORE its first recovery request.

## Real final execution

Six separate exact native-binary processes, each1passed/0failed/0ignored:

| Case | Default wall/harness | Runtime-label wall/harness |
| --- | --- | --- |
| Actual aborted reply loss |10.90s/9.24s|9.26s/9.25s|
| Existing committed reply loss |8.11s/8.09s|8.02s/8.01s|
| Existing unarmed P0b |7.00s/6.99s|7.14s/7.12s|

Both API profiles actually use sagat_app; this is not an owner/runtime privilege matrix. The first new fault execution passed; no failure is hidden and no production RED is fabricated. Three bounded fresh Astra reviews returned None. Strict Clippy18.17s, separate locked build40.18s, fmt5.07s and architecture0.46s pass. Both1127-input source before/after pairs and exact executable before/after digests match. Exact commands/exits/UTC/raw logs/source and binary hashes are retained.

Root independently read all six positive databases: one canonical original receipt/hash/resource, one stop/success/audit,4Mretained references/held slot/no candidate payload, matching journal/head, no capacity/private quota residue and zero remaining probe functions/triggers/proxy connections. Pre-retry rollback is evidenced by actual independent owner reads inside the native test, two exact safe phase records per aborted case and parsed matching snapshot hashes, not inferred from later successful replay. No tokens, bodies, credentials or SQL frames are logged.

## Reproduce / scope

Same exclusive repository Compose PostgreSQL/project/private target/profile as public-scoped-reopen-baseline-20261018/infrastructure-20261023.json; loopback65118. Compile separately, then each exact selected executable runs under GNU TERM180s/KILL+5s, never foreground/preserve-status. Cooperative90s inner deadline is supplementary. Matrix stops at the first nonzero; each phase records its uniquely created DB. No manually started or unrelated service is used. Missing agent-config tooling remains an explicit checkout gap.

Only one current ordinary inline public aborted-COMMIT recovery case is established, preserving the accepted committed/unarmed cases. No OS/process death, post-ACK scheduler/drain, Applied/declared-secret/blob/immutable-old composition, provider undo, general nesting, rollout, ceiling benchmark, full workspace/SDK or canonical completion is claimed. No new public unknown-outcome enum is invented. Local aborted coverage can close; the full exceptional-report/master correction and canonical tasks remain separately gated.
