# Errors (design 047)

The platform's error envelope; never a provider's body or a token. `Error.code` is the closed set of
`sdks/error-catalog.json`; this design adds reasons under existing codes, and Task 047-1 adds them to the catalogue and to
feature 021's step vocabulary.

## REST: reasons under existing codes

| Code | Reason (`details.reason`) | When |
|---|---|---|
| `VALIDATION_FAILED` (422) | `OAUTH_PROVIDER_INVALID` | a provider definition breaks the write-time checks (`details.rule`) |
| `VALIDATION_FAILED` | `OAUTH_REGISTRATION_INVALID` | a registration names an unpublished provider, an unsupported method, client credentials on a deployment registration, or a missing secret |
| `VALIDATION_FAILED` | `OAUTH_SCOPE_NOT_IN_CATALOGUE` | a requested scope the definition does not list |
| `VALIDATION_FAILED` | `RESOURCE_DECLARATION_INVALID` | a declaration names an unknown or inaccessible resource, or more than 8 |
| `VALIDATION_FAILED` | `OAUTH_CREDENTIAL_HOST_MISMATCH` | an `oauth_credential` slot bound in a resource whose hosts are not the provider's API hosts |
| `VALIDATION_FAILED` | `OAUTH_BROKER_UNAVAILABLE` | feature 021's reason: the `oauth` scheme or a declaration while `oauth-broker-v1` is not activated |
| `VALIDATION_FAILED` | `HUB_ITEM_UNAVAILABLE` | feature 028's index reasons, plus `REVOKED` and `BLOCKED` |
| `CONFLICT` (409) | `OAUTH_STATE_INVALID` | a callback `state` unknown, used, expired, of another tenant, person or registration |
| `CONFLICT` | `OAUTH_ISSUER_MISMATCH` | `iss` absent where required, or not the provider's issuer |
| `CONFLICT` | `OAUTH_ACCOUNT_CHANGED` | a reconnect that yields another account, or any reconnect where the definition names no subject |
| `CONFLICT` | `OAUTH_AUTHORIZATION_DENIED` | the provider answered `access_denied`, or `invalid_grant` on the exchange (the person was slow) |
| `CONFLICT` | `OAUTH_CREDENTIAL_TERMINAL` | a connection or callback for a disconnecting or disconnected credential |
| `CONFLICT` | `HUB_DIGEST_CONFLICT` | an item version already imported with another digest |
| `CONFLICT` | `IDEMPOTENCY_KEY_REUSED` | an import key reused with another body |
| `UNAVAILABLE` (502 or 503) | `OAUTH_PROVIDER_ERROR` | another provider error; `details.error` from the closed list `invalid_request`, `invalid_client`, `unauthorized_client`, `unsupported_grant_type`, `invalid_scope`, `server_error`, `temporarily_unavailable`, `other` |

## Step failures, per executor path

Feature 021's step vocabulary (`specs/021-integration-resources-triggers/contracts/error-reasons.md`), with three rows this
design adds there.

| Broker outcome | Code | Retryable | Paths |
|---|---|---|---|
| Broker, vault or provider unavailable; a waiter's 10 s passed; an old control plane's `UNIMPLEMENTED` | `CAPABILITY_UNAVAILABLE` | yes | integration runner, scripts, activities |
| Credential `NEEDS_REAUTHORIZATION`; registration `BROKEN` | `RESOURCE_REAUTHORIZATION_REQUIRED` (added) | yes | all three |
| Credential disconnecting or disconnected; authority revoked; a hub version under a `security` revocation | `RESOURCE_UNAVAILABLE` | no | all three |
| Granted scopes do not cover the declaration; `invalid_scope` | `OAUTH_SCOPE_INSUFFICIENT` (added) | no | all three |
| Delivery budget spent | `RESOURCE_ASK_LIMIT` (added) | yes | scripts, activities |
| A test run's `resource` call | `SCRIPT_DENIED`, reason `RESOURCES_IN_TEST_RUN` | no | scripts |
