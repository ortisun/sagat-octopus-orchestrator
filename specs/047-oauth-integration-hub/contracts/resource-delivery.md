# Resource delivery (design 047)

## Worker gRPC: `ResolveDeclaredResources`

Added under this design's protocol minor (the next free one at merge, after feature 021's). Limit class
`resource_delivery`: one call at attempt start plus at most 30 re-asks per attempt, one per 10 s per resource.

```proto
message ResolveDeclaredResourcesRequest {
  TaskRef task = 1;              // tenant, namespace, task, instance, attempt, generation
  string capability_token = 2;
  repeated string resources = 3; // empty = every declared resource (start); one name = a re-ask
}
message DeclaredResource {
  string name = 1;
  string type_key = 2;
  uint32 revision = 3;
  bytes settings = 4;            // the pinned revision's non-secret settings, canonical JSON
  map<string, bytes> secrets = 5;// slot values; for an oauth slot, the access token only
  google.protobuf.Timestamp expires_at = 6; // the access token's expiry, when it has one
}
message ResolveDeclaredResourcesResponse { repeated DeclaredResource resources = 1; }
```

All or nothing: a failed check refuses the whole call with the codes of `errors.md`. The SDKs expose
`resource(name) -> {settings, secrets, expiresAt}` and refresh by asking again when `expiresAt` is less than 60 s away.

## Script host call: `resource`

On the shim protocol of feature 005: `{"type":"resource","name":"<declared name>"}` from the script; the supervisor
answers `{"type":"resourceReply","settings":{...},"secrets":{...},"expiresAt":"..."}` or a refusal. At script start the
runner has already resolved every declared resource and answers from that set; a call when the token is less than 60 s
from expiry makes the supervisor re-ask with one resource name, add the new token to the redaction set, then answer. A
test run's `resource` call is refused and audited.
