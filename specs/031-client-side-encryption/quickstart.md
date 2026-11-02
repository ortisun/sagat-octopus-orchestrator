# Quickstart: client-side payload encryption (planned demo)

Nothing below runs today: it is the demo the delivery must make reproducible (Definition of Done) and that T031-063
records with real output. Every command names files that the tasks create; none of their results is claimed here.

## Prerequisites (development only)

```sh
# Platform plus a disposable OpenBao with Transit and the reference codec server, whose development login uses
# apps/dev-idp (T031-060).
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.encryption.yaml --env-file infra/docker/.env up -d --build
# Operator settings read by the control plane at start (T031-047, T031-049):
#   SAGAT_STUDIO_CODEC_ORIGINS=http://localhost:8099
#   SAGAT_CODEC_ASSERTION_KEYS=<ES256 PKCS#8, base64>   SAGAT_CODEC_ASSERTION_ISSUER=http://localhost:8080
#   SAGAT_CODEC_ASSERTION_TTL_SECS=60
# Activate once every engine and control-plane replica reports the capability (T031-068):
control-plane payload-encoding activate
# Customer side: a Transit key the platform never sees.
export BAO_ADDR=http://127.0.0.1:8200 BAO_TOKEN=<dev token printed by the overlay>
bao secrets enable transit && bao write -f transit/keys/orders-pii
```

The overlay is loopback-only and development-only; production uses the customer's own KMS, identity provider and
codec server.

## Happy path

1. Publish `examples/payment-encrypted/workflow.json` (created by T031-063): `inputSchema` marks `/card` `encrypted`
   (`pii`) and the `charge` activity's `outputSchema` marks `/token` `encrypted` (`secret`).

   ```sh
   sagat workflow validate examples/payment-encrypted/workflow.json   # lists /card encrypted pii, /token encrypted secret
   sagat workflow publish  examples/payment-encrypted/workflow.json   # encodingRules: 1
   curl -s -H "Authorization: Bearer $TOKEN" -H "X-Octopus-Namespace: demo" \
     "$API/workflows/$WORKFLOW/versions/1/payload-plan"                 # tenantId, namespaceId, the two locations
   ```

2. Run a TypeScript worker and client configured with `Encryption(OpenBaoTransitKeyProvider({keyName: "orders-pii"}))`
   (`examples/payment-encrypted/ts/`). Expected: the run succeeds; the worker log shows plaintext only inside the
   handler; `psql -c "select convert_from(content,'UTF8') from payloads ..."` shows `{"$encoded": ...}` at `/card` and
   `/token`, each with `ctx` naming the demo tenant, namespace and its own pointer, and no card number.
3. Studio → Namespace → Payload codec → set `http://localhost:8099/sagat` with `levelCeiling: pii` (as an administrator
   holding the reveal grant), then "Test endpoint".
4. Sign in to the codec server (`http://localhost:8099/login`, development identity provider). Run detail → Input →
   "Reveal personal data", then "Decode with codec server". Expected: `/card` shows its plaintext with a "pii" marker;
   the `charge` output's `/token` (`secret`) is never disclosed or decoded; closing the panel clears the value; browser
   storage and the platform network log contain no card number.
5. CLI: `sagat workflow inspect $RUN --values --reveal-pii --decode --codec-token-file ./codec.token` prints the decoded
   input; the `charge` output shows `[withheld: secret]`.

## Failure scenarios

1. **Plaintext refused.** Start the run with a client that has no codec:
   `sagat workflow run payment-encrypted --input '{"card":{"number":"4111..."}}'` → `422 VALIDATION_FAILED`,
   `details.reason = ENCODING_REQUIRED`, `pointer = /card`; no run, and the canary is in no table or log.
2. **Transplant refused.** Copy an `/card` envelope from a run of another namespace into a start request →
   `422`, `details.reason = ENVELOPE_CONTEXT_MISMATCH`.
3. **Carriage refused.** Publish a parent whose `child.input = input` targets a child version that does not declare
   `/card` → the parent's child step fails `ENCODING_CARRIAGE_MISMATCH` and no child run exists.
4. **Assertion alone decrypts nothing.** Without signing in to the codec server, obtain an assertion from the platform
   and post an envelope to `/decode` → `401`; post the same assertion twice with a session → the second is `401`.
5. **Gating.** Stop the codec worker and start a codec-less worker that also serves `charge` → the task stays `READY`,
   `listRunAttempts` shows no new attempt, `worker_leases_withheld_for_codec_total{reason="capability"}` grows.
6. **Key rotation and disable.** `bao write -f transit/keys/orders-pii/rotate`; new runs encode under version 2 and old
   runs still decode. Then `bao write transit/keys/orders-pii/config min_decryption_version=2`, wait for the SDK unwrap
   cache TTL, rerun a step of an old run → the step fails `PAYLOAD_KEY_UNAVAILABLE` (not retryable), no plaintext.
7. **KMS outage.** `docker compose ... stop openbao` → hosts stop claiming planned tasks (they stay `READY`); a task
   already claimed fails `PAYLOAD_CODEC_UNAVAILABLE` and retries under its budget; a client start raises the same code
   before sending; restart OpenBao and the work resumes.
8. **Codec-server outage.** Stop the reference codec server → the Studio shows "codec unavailable" on each encoded node
   and the rest of the run view keeps working; the CLI exits with a typed error.
9. **Publication refusal.** Add a `condition` on `input.card.brand` → `422`, `details.reason = ENCODING`,
   `ENCODING_READ_BY_EXPRESSION` at `/nodes/.../expression`; deactivate the feature and publish any document with
   `x-sagat-encoding` → `ENCODING_NOT_ACTIVATED`.

## What this demo does not prove

Throughput or latency (see the benchmark of T031-062), protection against a malicious worker, a compromised codec
server or a person the customer's identity provider admits, protection of metadata, replay within one namespace and
location, or that blob bodies are ciphertext beyond their header (spec, threat model; G1520–G1523, G1540).
