# Errors (design 047)

The platform's error envelope; never a provider's body or a token.

## REST

| Code | Status | When |
|---|---|---|
| `OAUTH_PROVIDER_INVALID` | 422 | a provider definition breaks the write-time checks (`details.rule`) |
| `OAUTH_REGISTRATION_INVALID` | 422 | a registration names an unpublished provider, a method it does not support, or a missing secret |
| `OAUTH_SCOPE_NOT_IN_CATALOGUE` | 422 | a requested scope the provider definition does not list |
| `RESOURCE_DECLARATION_INVALID` | 422 | a declaration names an unknown or inaccessible resource, or more than 8 |
| `OAUTH_BROKER_UNAVAILABLE` | 422 | feature 021's code: the `oauth` scheme or a declaration while `oauth-broker-v1` is not activated |
| `OAUTH_STATE_INVALID` | 409 | a callback `state` unknown, used, expired, of another person or another registration |
| `OAUTH_ISSUER_MISMATCH` | 409 | `iss` absent where required, or not the provider's issuer |
| `OAUTH_ACCOUNT_CHANGED` | 409 | a reconnect that yields another account |
| `OAUTH_AUTHORIZATION_DENIED` | 409 | the provider answered `access_denied`, or `invalid_grant` on the exchange (the person was slow) |
| `OAUTH_PROVIDER_ERROR` | 502 | another provider error; `details.error` from the closed list `invalid_request`, `invalid_client`, `unauthorized_client`, `unsupported_grant_type`, `invalid_scope`, `server_error`, `temporarily_unavailable`, `other` |
| `HUB_ITEM_UNAVAILABLE` | 422 | feature 028's index reasons, plus `REVOKED` and `BLOCKED` |
| `HUB_DIGEST_CONFLICT` | 409 | an item version already imported with another digest |
| `IDEMPOTENCY_KEY_REUSED` | 409 | an import key reused with another body |

## Step failures, per executor path

Feature 021's closed vocabulary (`specs/021-integration-resources-triggers/contracts/error-reasons.md`), with one code
this design adds.

| Broker outcome | Code | Retryable | Paths |
|---|---|---|---|
| Broker, vault or provider unavailable; a waiter's 10 s passed | `CAPABILITY_UNAVAILABLE` | yes | integration runner, scripts, activities |
| Credential `NEEDS_REAUTHORIZATION`; registration `BROKEN` | `RESOURCE_REAUTHORIZATION_REQUIRED` (added by this design) | yes | all three |
| Credential `DISCONNECTING` or `DISCONNECTED`; authority revoked; hub version blocked | `RESOURCE_UNAVAILABLE` | no | all three |
| Granted scopes do not cover the declaration | `OAUTH_SCOPE_INSUFFICIENT` | no | all three |
| Re-ask budget spent | `RESOURCE_ASK_LIMIT` | yes | scripts, activities |
| Old binaries after a rollback | `CAPABILITY_UNAVAILABLE` | yes | activities |
