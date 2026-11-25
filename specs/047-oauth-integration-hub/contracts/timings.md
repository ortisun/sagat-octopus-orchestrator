# Timings and bounds (design 047, Non-functional requirement 047-003)

| Quantity | Value |
|---|---|
| Authorization request lifetime | 10 minutes; at most 10 open per person |
| Deliverable access token | valid for at least 60 s more |
| Refresh due | valid for less than 120 s |
| Expiry without `expires_in` | the provider definition's `defaultLifetimeSeconds`, otherwise 3,600 s |
| Refresh lease | 30 s; extended once by 30 s under the fence |
| Call cut-off after the acquire | 15 s (`CLOCK_BOOTTIME`) |
| Token endpoint deadline | 10 s |
| Vault write deadline | 5 s per write |
| Waiter | backoff from 200 ms to 1 s, at most 10 s |
| Request windows | per registration: 4 in flight, 10 per second; per tenant on a deployment registration: 2 and 5 |
| `invalid_client` before `BROKEN` | 3 answers within 10 minutes |
| Keep-alive | at half the provider's idle refresh-token lifetime |
| Delivery | 2 full calls per attempt; 30 re-asks per hour of the attempt; 1 per 10 s per resource |
| Offer withdrawal or opt-out | new connections stop at once; refreshes stop after 7 days; then disconnect |
| Revocation attempts | 5 over 1 hour, then `UNKNOWN` |
| Revisions per credential | at most 8 not destroyed |
| Token response | 16 KiB |
| Provider definition | 32 KiB; 200 scopes |
| Hub file | 1 MiB |
| Credentials per namespace; registrations per tenant | 200; 50 |
| Delivery rows | deleted 7 days after the attempt ends |
