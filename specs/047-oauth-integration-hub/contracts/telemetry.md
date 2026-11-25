# Telemetry and audit (design 047)

Labels are closed; tenant, namespace, resource, provider, registration and credential identifiers are span attributes.
Audit records are value-free and never carry the account label.

## Control plane (builder A, Task 047-20)

| Metric | Labels and values |
|---|---|
| `oauth_token_requests_total` | `grant` (`authorization_code`, `client_credentials`), `kind` (`exchange`, `refresh`, `revocation`, `userinfo`), `outcome` (`ok`, `invalid_grant`, `client_error`, `retryable`, `timeout`) |
| `oauth_token_request_seconds` | `grant`, `kind` |
| `oauth_refresh_waits_total` | `outcome` (`delivered`, `acquired`, `timed_out`, `reauthorization`) |
| `oauth_credentials` | `state` (the five states) |
| `oauth_authorizations_total` | `outcome` (`completed`, `refused`, `incomplete`) |
| `oauth_deliveries_total` | `path` (`integration`, `script`, `activity`), `outcome` (`ok`, `refused`, `unavailable`, `ask_limit`) |
| `oauth_request_window_waits_total` | `scope` (`registration`, `tenant`) |
| `hub_index_refresh_total`, `hub_index_age_seconds` | `outcome` (`ok`, `refused`, `unreachable`); the revocations index's age has an alert at 48 hours |
| `hub_imports_total` | `kind`, `outcome` |
| `hub_blocked_total` | `point` (`admission`, `delivery`) |

Audit records: `oauth.provider.created`, `oauth.provider.published`, `oauth.registration.created`,
`oauth.registration.secret_rotated`, `oauth.registration.broken`, `oauth.offer.offered`, `oauth.offer.opted_in`,
`oauth.offer.opted_out`, `oauth.offer.withdrawn`, `oauth.authorization.started`, `oauth.authorization.completed`,
`oauth.authorization.refused`, `oauth.authorization.incomplete`, `oauth.token.refreshed`, `oauth.token.refresh_failed`,
`oauth.credential.bound`, `oauth.credential.reauthorization_required`, `oauth.credential.disconnected`,
`oauth.credential.revocation_unknown`, `oauth.credential.account_changed_refused`, the disclosure record with the resource
names, `hub.imported`, `hub.published`, `hub.item_revoked`, `hub.version_blocked`, `hub.version_unblocked`.

## Runners and SDKs (builder B)

| Metric | Labels and values |
|---|---|
| `script_resource_asks_total` | `outcome` (`ok`, `refused`, `limit`) |
