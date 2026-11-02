# Five-SDK conformance scenarios — feature 032 (draft, not implemented)

Location at implementation: `tests/contract/sdk-conformance/scenarios/region-*.json`, run by
`SAGAT_SDK_SCENARIO=<name> cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`
against the region-pair harness (`tests/support/region.rs`) with a resolver override that maps the cell
endpoint to one region or the other. Every scenario runs in TypeScript, Go, Python, C# and Java with the
same assertions; no language may skip one (ADR-0007, ADR-0015).

| Scenario | Steps | Assertions |
|---|---|---|
| `region-passive-reconnect` | client and worker start against the cell endpoint resolved to the passive region; the resolver is switched to the active region after 2 s | first calls get `UNAVAILABLE` with reason `REGION_PASSIVE`; the SDK closes its pooled connection and resolves again; a keyed `start` and the worker's poll succeed in the active region; no un-keyed call is repeated |
| `region-drain-handover` | a worker holds a lease; the active region drains; the worker completes; the resolver moves to the new region after activation | the completion during the drain is accepted; the next poll gets `REGION_DRAINING`; after activation the worker registers again and claims new work |
| `region-failover-reregister` | unplanned failover in `async` mode with the worker's session created after the cut point | the first poll in the new region is `UNAUTHENTICATED`; the SDK registers again once and continues; it does not loop |
| `region-stale-report` | the worker completes a lease granted by the former region after the cut point; the new authority reissued the same task id with another lease token | the report gets `ABORTED`/`STALE_ATTEMPT`; the SDK does not retry it; the reissued attempt carries the same idempotency key |
| `region-stream-reset` | a client follows a run with the run stream; transitions after the cut are lost; the client reconnects with its last `projectionSeq` and the epoch of that event | the first event has `reset: true` and the whole graph; the client's final graph equals the authority's; a second client whose last sequence replicated reconnects without a reset |
| `region-replay-durability` | `sync_quorum` cell; a keyed `start` committed, then the cross-region link is cut and a replica cancels a waiting commit; the client repeats the keyed `start` against the former region | no `200`/`201` replay during the cut (`UNAVAILABLE` with `DURABILITY_UNCONFIRMED` or `COMMIT_OUTCOME_UNKNOWN`); after failover the repeat creates or returns exactly one run in the new authority |
| `region-commit-unknown` | a keyed `start` in a `sync_quorum` cell while the cross-region link is cut past the commit deadline | the call returns `UNAVAILABLE` with `COMMIT_OUTCOME_UNKNOWN`; the SDK repeats it with the same key after the link heals; exactly one run exists |

Golden fixtures are compared byte for byte only for the request shapes (headers, `epoch` parameter);
outcomes are compared after normalization of ids and times, as the existing scenarios do.
