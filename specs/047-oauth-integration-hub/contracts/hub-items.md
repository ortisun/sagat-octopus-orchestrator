# Hub items (design 047)

Every item file is canonical JSON (RFC 8785), at most 1 MiB, validated by the codecs of `crates/infra-schema/src/hub/`.
Feature 028's index lists each item with its kind, version (semantic), digest and files; this design adds the kinds
`OAUTH_PROVIDER` and `INTEGRATION`, and a `revoked` list of `{digest, class: "security" | "other", reason}`.

## `OAUTH_PROVIDER`

`{slug, title, issuer, authorizationEndpoint, tokenEndpoint, revocationEndpoint?, methods: [...], pkce: "S256",
sendsIss: bool, rotatesRefreshTokens: bool, revocation: "per-token" | "grant" (default "grant": revoking one token may end the whole grant), refreshTokenIdleDays?: int, defaultLifetimeSeconds?: int,
subject: {from: "id_token" | "userinfo" | "token_response" | "none", field?}, userinfoEndpoint?, scopes: [{name, title,
required?}], extraAuthorizationParameters: {name: [allowed values]}, hostBindings?: {tokenEndpointPath?: "registration"}}`.
Endpoints are https; hosts are fixed names (no wildcard) except where `hostBindings` binds a path segment from the
registration (a Microsoft directory identifier).

## `RESOURCE_TYPE`

`{key: "hub.<publisher>.<name>", version, base: "http.request" | "graphql.execute" | the S3 set | "smtp.send" |
"slack.postMessage", hosts: ["api.github.com"] | {pattern: "*.my.salesforce.com", bindFrom: "token_response.instance_url"},
auth: {scheme: "oauth", provider: "<slug>", scopes: {required: [...], optional: [...]}} | {scheme: "bearer" |
"api_key_header" | "basic"}, settings: <a JSON Schema in design 043's subset, a narrowing of the base's>, presets: [...]}`.

A preset: `{name, title, method, path: "/repos/{owner}/{repo}/issues", parameters: {owner: {type: "string", maxLength:
100}, ...}, query?: {...}, body?: <template under the expression evaluator>, effect: "safe" | "effect",
idempotency?: {header: "Idempotency-Key"} | "none"}`. Each path parameter is one segment, percent-encoded; dot segments
are refused; `effect` must be `effect` for any method other than `GET` and `HEAD`.

## `SCRIPT`, `WORKFLOW`

Feature 028's shapes. A script template may carry a manifest (design 046).

## `INTEGRATION`

`{slug, title, summary, maintainer, licence, parts: [{kind, item, version}], prepare: [{kind: "registration" |
"egress-host" | "secret", detail}]}`. Imported in one transaction.

## Review record

`review.md` in the integration's directory: the two reviewers' names, the date, the item digests reviewed, and what was
checked (hosts, scopes, presets, effect classes, fixtures). The contract suite refuses an item whose digest no review
names.
