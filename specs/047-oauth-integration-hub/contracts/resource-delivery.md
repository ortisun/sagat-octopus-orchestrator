# Resource delivery (design 047)

## Worker gRPC: `ResolveDeclaredResources`

Added under this design's protocol minor (the next free one at merge, after feature 021's), in the limit class
`resource_delivery` stated in `sdks/SPEC.md`: two full calls per attempt, 30 re-asks per hour of the attempt, one per 10 s
per resource (counted in `oauth_delivery_attempts`).

```proto
message ResolveDeclaredResourcesRequest {
  TaskRef task = 1;              // tenant, namespace, task, instance, attempt, generation
  string capability_token = 2;
  repeated string resources = 3; // empty = every declared resource; one name = a re-ask
  bool force_refresh = 4;        // the provider refused the token; honoured once per access revision
}
message DeclaredResource {
  string name = 1;
  string type_key = 2;
  uint32 revision = 3;
  bytes settings = 4;            // the pinned revision's non-secret settings, canonical JSON
  map<string, bytes> secrets = 5;// slot values; for an oauth slot, the access token only
  google.protobuf.Timestamp expires_at = 6;
}
message ResolveDeclaredResourcesResponse { repeated DeclaredResource resources = 1; }
```

All or nothing: a failed check refuses the whole call with the codes of `errors.md`. An old control plane answers
`UNIMPLEMENTED`; the SDKs and shims map it to `CAPABILITY_UNAVAILABLE`, retryable (a conformance case). The SDKs expose
`resource(name) -> {settings, secrets, expiresAt}` and ask again when `expiresAt` is less than 60 s away or the provider
refused the token.

## Script host call: `resource`

On the shim protocol of feature 005: `{"op":"resource","name":"<declared name>","forceRefresh":false}` from the script;
the supervisor answers `{"op":"resourceReply","settings":{...},"secrets":{...},"expiresAt":"..."}` or a refusal. At start
the runner has resolved every declared resource and answers from that set; when the token is less than 60 s from expiry, or
on `forceRefresh`, the supervisor re-asks for that one resource, adds the new token to the redaction set, then answers. A
test run's `resource` call is refused with `SCRIPT_DENIED`, reason `RESOURCES_IN_TEST_RUN`, and audited.
