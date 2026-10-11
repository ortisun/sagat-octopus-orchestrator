# inline-script

A script node runs a TypeScript (or Python) function in the platform's sandbox. The document pins the script by the
digest of the artifact the platform built, so first build the script, then paste its digest.

```sh
sagat script test enrich.ts --input '{"orderId":"o-1","items":2}'      # run it in the sandbox
sagat script publish enrich.ts                                         # build it: prints artifactDigest sha256:...
# put that digest in workflow.json ("artifactDigest"), then:
sagat workflow validate workflow.json
sagat workflow publish workflow.json
sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind
sagat workflow run enrich-order --input '{"orderId":"o-1","items":2}' --wait
```

Until the digest is replaced, `sagat workflow publish` stops with `UNKNOWN_SCRIPT`: the platform has no script with
the placeholder digest. The sandbox needs a runner (`docs/runbooks/scripts.md`).
