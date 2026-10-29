# Codec assertion (draft)

Issued by `POST /v1/namespaces/{namespaceId}/codec-assertions` ([openapi-fragment.yaml](openapi-fragment.yaml)); verified
only by customer codec servers ([codec-server.openapi.yaml](codec-server.openapi.yaml)), which receive it in
`X-Sagat-Codec-Assertion`. It is **a narrowing, never an authorization**: a codec server authenticates the person with
the customer's own identity provider and intersects that session's policy with the assertion's claims (ADR-0066
point 8, D1558). It is not a platform credential either: the platform API refuses it everywhere (`typ` and `aud` differ
from user tokens, and the API's OIDC verifier does not trust the assertion key).

Why it cannot authorize alone: the control plane holds the signing key, so a platform operator or a compromised control
plane could mint any claims and post envelopes copied from the database to the codec server. Requiring the customer's
session keeps the operator out of the plaintext unless the customer's own identity provider admits them.

## Header and claims

```json
{"alg": "ES256", "typ": "sagat-codec+jwt", "kid": "codec-2026-10"}
{
  "iss": "https://sagat.example.com",
  "aud": "https://codec.example.com/sagat",
  "sub": "user:3f1c...",
  "iat": 1792987200, "nbf": 1792987200, "exp": 1792987260,
  "jti": "9b2f5c0e-...",
  "sagat_tenant": "6a1e...", "sagat_namespace_id": "0b7d...", "sagat_namespace": "payments-prod",
  "sagat_purpose": "decode",
  "sagat_max_level": "internal"
}
```

| Claim | Rule |
|---|---|
| `iss` | The setting `SAGAT_CODEC_ASSERTION_ISSUER` (the deployment's public HTTPS base URL) |
| `aud` | Exactly the configured endpoint URL (origin plus path) of the namespace at issuance |
| `sub` | The authenticated subject, as the platform knows it; no e-mail, name or group is added. A codec server may compare it with its own session subject when both come from the same identity provider (013) |
| `exp - iat` | The deployment setting `SAGAT_CODEC_ASSERTION_TTL_SECS`: default 60 s, at most 300 s; no request can change it |
| `sagat_tenant`, `sagat_namespace_id` | Ids the codec server MUST check against its configuration and against each envelope's `ctx` |
| `sagat_purpose` | `decode` (needs `run:read`) or `encode` (needs `run:start`) |
| `sagat_max_level` | min(the caller's 016 audience, the endpoint's `levelCeiling`): `public` with `run:read`; `internal` with `payload:read-internal`; `pii` only when the request has `reveal: ["pii"]`, the caller holds the 016 grant and the ceiling is `pii` (coordinator C9); never `secret` |

No other claim is added. The assertion carries no run id: the platform's disclosure (016) has already decided which
envelopes the caller received. Each assertion is single-use at the reference codec server (`jti` replay cache), so the
Studio obtains one per decode batch.

## Signing keys

- ES256 (P-256). Keys come from `SAGAT_CODEC_ASSERTION_KEYS` (PKCS#8, base64, current first, previous second), held
  by the control plane only, like `SAGAT_SCRIPT_SIGNING_KEY`. `GET /v1/codec-jwks` publishes both public keys. When
  feature 015 lands, the key MAY move to OpenBao Transit signing without changing the token format.
- Rotation: add the new key as current, keep the old as previous for at least the maximum lifetime plus the codec
  servers' JWKS cache (300 s), then drop it. A production profile without a key disables the endpoint (`503`; the Studio
  shows "codec not available"); it never falls back to the user's access token.
- The keys are entries of feature 032's cell key inventory (coordinator C11): replicated with the control-plane
  configuration of each region of a cell; after a failover the JWKS of the active region MUST publish the keys that
  signed still-valid assertions, or codec servers refuse those assertions until they expire (at most 300 s).

## Audit and rate

`codec.assertion_issued` (`ResourceType::Namespace`, reason `codec:<purpose>:<maxLevel>`, trace id; no token, no jti)
commits before the token is returned; identical `public`/`internal` issuances by one actor collapse for 300 s; `pii`
issuances never collapse. A refusal is `codec.assertion_denied` (best effort, collapsing). Issuance is charged to the
namespace API admission bucket once (`ApiOperation::CodecAssertionIssue`, new) and limited per subject (plan ceilings).
