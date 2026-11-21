# Build provenance statement (feature 023 draft)

Stored per build in `script_build_provenance`, keyed by tenant and build key, and returned as
`ScriptVersion.provenance` by `getScriptVersion` (`script:read`). Produced by the control plane in the
build-completion transaction for every build after this feature, of any language. The artifact row is
content-addressed and shared by builds of the same digest; the statement is not: a digest produced again
under another toolchain is another build key and gets its own statement, and a version finished from the
build-key cache shows the statement of the build it reused (D1227). Older builds have none (G1217).

## Envelope

DSSE (https://github.com/secure-systems-lab/dsse), JSON:

```json
{ "payloadType": "application/vnd.in-toto+json",
  "payload": "<base64 of the statement bytes>",
  "signatures": [ { "keyid": "<the artifact key_id>", "sig": "<base64 Ed25519 over PAE(payloadType, payload)>" } ] }
```

`PAE(type, body) = "DSSEv1" SP len(type) SP type SP len(body) SP body` (lengths in ASCII decimal). The key
is the control plane's existing script signing key (`SAGAT_SCRIPT_SIGNING_KEY`); the DSSE payload type
separates this signature from the artifact signature (`sagat/script-artifact/v1 ‖ tenant ‖ digest ‖
language`), so neither can be replayed as the other. The envelope is at most 64 KiB.

## Statement

in-toto Statement v1 (https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md) with an
SLSA Provenance v1 predicate (https://slsa.dev/spec/v1.0/provenance):

```json
{ "_type": "https://in-toto.io/Statement/v1",
  "subject": [ { "name": "script-artifact", "digest": { "sha256": "<artifact digest hex>" } } ],
  "predicateType": "https://slsa.dev/provenance/v1",
  "predicate": {
    "buildDefinition": {
      "buildType": "https://sagat-octopus-orchestrator.io/script-build/v1",
      "externalParameters": {
        "language": "rust",
        "contentDigest": "sha256:…",
        "lockDigest": "sha256:… | null",
        "policy": "v1;memory=…;…;egress=none",
        "inputSchemaDigest": "sha256:… | null",
        "outputSchemaDigest": "sha256:… | null",
        "runtimeCatalog": "sha256:… | null (python-data only)"
      },
      "internalParameters": {
        "tenantId": "…", "namespaceId": "…", "scriptId": "…", "versionNumber": 3,
        "buildKey": "sha256:…",
        "toolchainHandedOut": "build=…;runtime=…;protocol=artifact-v3;…",
        "verifiedByControlPlane": ["contentDigest", "lockDigest", "policy", "schemas", "buildKey",
                                   "toolchainHandedOut", "bundleShape", "sbomPackagesEqualLockfile"]
      },
      "resolvedDependencies": [
        { "uri": "pkg:cargo/serde_json@1.0.x", "digest": { "sha256": "<archiveSha256 from the catalog>" } }
      ]
    },
    "runDetails": {
      "builder": { "id": "https://sagat-octopus-orchestrator.io/script-runner/v1",
                   "version": { "runnerReportedToolchain": "build=…" } },
      "metadata": { "invocationId": "<build job id>",
                    "startedOn": "<claim time, database clock>", "finishedOn": "<completion time, database clock>" },
      "byproducts": [ { "name": "sbom", "digest": { "sha256": "<sha256 of the SBOM bytes>" } },
                      { "name": "build-log", "digest": { "sha256": "<sha256 of the retained build log>" } } ]
    } } }
```

Contains no source, input, secret, URL path or credential. `resolvedDependencies` lists catalog
packages only; the platform images are named by the toolchain identities. The tenant and namespace ids
appear only inside the tenant's own artifact view.

## What it does and does not prove

- It proves the control plane accepted this artifact for this version under these verified parameters,
  at these database times, from a runner that held the build lease.
- It does not prove the binary was compiled from the source: the control plane cannot compare them
  (005 G118, G1202). A holder of the tenant's runner token can still get code signed for that tenant.
- It claims no SLSA build level.

## Verification

`crates/application/src/script_signing.rs` verifies the envelope with the configured public keys
(the runner's `SAGAT_RUNNER_TRUSTED_KEYS` or `getScriptSigningKey`), checks that the subject equals the
artifact digest and that `predicateType` and `buildType` are the values above. Contract tests verify a
fixture offline and refuse a modified payload, a swapped subject and an artifact signature presented as
a DSSE signature.
