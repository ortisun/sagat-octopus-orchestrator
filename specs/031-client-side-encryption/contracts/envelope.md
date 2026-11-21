# Envelope v1 (normative)

Status: draft contract of feature 031; not implemented. Shape check: [envelope.schema.json](envelope.schema.json).
Decision: [ADR-0066](../../../docs/adr/ADR-0066-client-side-payload-encryption.md) points 5 and 6 (v1 is decodable
forever and is bound to its context).

## 1. Shape

An encoded value replaces the plaintext JSON value at an encoded location:

```json
{"$encoded": {
  "v": 1,
  "codecs": ["zstd", "pad", "aes-256-gcm"],
  "cty": "application/json",
  "lvl": "pii",
  "ctx": {"t": "6a1e0c2e-...", "n": "0b7d4f19-...", "p": "/customer/card"},
  "key": {"kid": "orders-pii", "kv": "4", "wk": "dmF1bHQ6djQ6..."},
  "iv": "q83vEjRWeJCrze8S",
  "data": "3q2+7w..."
}}
```

| Member | Rule |
|---|---|
| `$encoded` | The only member of the outer object |
| `v` | Integer `1` |
| `codecs` | 1–3 entries, in the order applied when encoding: optionally one compression (`zstd` or `gzip`), optionally `pad`, optionally `aes-256-gcm` last; `pad` only after a compression and before a cipher; no repeats; at least one of compression or cipher |
| `cty` | `application/json` (the plaintext is the RFC 8785 canonical UTF-8 text of the JSON value) |
| `lvl` | `public`, `internal`, `pii` or `secret`: the effective level of the location (spec FR-031-003) |
| `ctx` | Required. `t`: tenant id (UUID); `n`: namespace id (UUID); `p`: the pointer pattern of the location in its carrier (`""` for the root, `*` for array elements, at most 512 bytes). Values come from the payload plan, never from user input |
| `key` | Required when a cipher is present, forbidden otherwise. `kid`: 1–128 characters `[A-Za-z0-9._:/-]`, an opaque customer key reference that MUST NOT contain a secret; `kv`: optional, 1–64 characters of the same set; `wk`: standard padded base64 of the wrapped data key, 1–1,024 bytes decoded |
| `iv` | Required when a cipher is present, forbidden otherwise; base64 of exactly 12 bytes |
| `data` | Standard padded base64: the ciphertext followed by the 16-byte GCM tag, or the compressed (and padded) bytes without a cipher |

Parsing is strict in the server, every SDK and every codec server: duplicate members at any depth, unknown members,
other types, case variants and non-canonical base64 (padding, whitespace, URL alphabet) are invalid. The header (the
`$encoded` object without `data`, serialized) is at most 2 KiB.

## 2. Encoding (customer side)

1. `P` = RFC 8785 canonical UTF-8 of the value (the canonicalization the builders use, `sdks/SPEC.md` §7).
2. With a compression codec: `P = compress(P)` (one zstd frame, or one gzip member).
3. With `pad` (required by default when a compression precedes a cipher): `P = len32_be(P) ‖ P ‖ zeros` to the bucket
   size: the next power of two that is at least `max(256, len(P) + 4)` up to 65,536 bytes, then the next multiple of
   65,536. This narrows, but does not remove, the length signal that compression adds (G1540).
4. With the cipher: obtain a data key `DEK` (32 bytes) and its wrapped form `wk` from the key provider for `kid`
   (possibly from the bounded reuse cache); draw `iv` (12 bytes from the OS CSPRNG); `AAD` = RFC 8785 canonical UTF-8 of
   the `$encoded` object **without** `data` (so `v`, `codecs`, `cty`, `lvl`, `ctx`, `key` and `iv` are authenticated);
   `C` = AES-256-GCM-Encrypt(`DEK`, `iv`, `P`, `AAD`), tag appended.
5. `data` = base64 of `C` (or of `P` without a cipher).
6. A writer that streams an envelope into a blob MUST emit the members in the order `v`, `codecs`, `cty`, `lvl`, `ctx`,
   `key`, `iv`, `data`, so the header precedes `data` (spec FR-031-012). Inline envelopes may use any member order.

A data key MUST NOT encrypt more than 1,048,576 envelopes or be reused for longer than 15 minutes (hard ceilings; the
defaults are smaller, plan "Finite ceilings"). With random 96-bit nonces this stays far below the 2^32 messages per key
at which AES-GCM nonce collisions become a concern (OpenBao Transit guidance, <https://openbao.org/docs/secrets/transit/>).

## 3. Decoding (customer side)

Validate the shape strictly; compare `ctx` with the decoding context — a worker host with the lease's tenant,
namespace and the plan's bound pattern for that pointer, a codec server with the assertion's `sagat_tenant` and
`sagat_namespace_id` — and refuse a mismatch (`PAYLOAD_DECODE_FAILED`, codec item error `CONTEXT_MISMATCH`); obtain
`DEK` by unwrapping `wk` (bounded cache); recompute `AAD`; decrypt and verify the tag; strip padding; decompress while
counting output bytes and stop at the decoded-size ceiling (`PAYLOAD_DECODED_TOO_LARGE`); parse JSON under the SDK's
rules (`sdks/SPEC.md` §8). Any failure is a typed error ([errors.md](errors.md)); no partial plaintext is returned.

An envelope proves possession of the key, not who wrote it: any holder of encode access to a key (a worker, a client,
a codec server's `/encode`) can produce a valid envelope for any context it is given. Compression-only envelopes have no
authentication at all; their `ctx` is checked but not protected.

## 4. What the platform does with an envelope

At every write into a carrier of a version with `encoding_rules: 1`, the server parses the JSON strictly, checks the
shape against [envelope.schema.json](envelope.schema.json), checks `codecs` against the declared encoding (a cipher for
`encrypted`), `lvl` against the effective level, and `ctx` against the run's tenant id, namespace id and the location's
pointer pattern (`ENVELOPE_CONTEXT_MISMATCH`). It counts envelopes (at most 1,024 per payload) and refuses an envelope at
a location the receiving carrier does not declare (`ENCODING_CARRIAGE_MISMATCH`). It does not decode `data`, never
base64-decodes `wk`/`iv` beyond length checks, and stores the value exactly as received. Two envelopes of the same
plaintext differ (fresh nonce), so they are distinct payload rows.

Consequence for carriage (spec FR-031-005): an envelope is copied unchanged only to the same pointer pattern in a run of
the same tenant and namespace. Moving an encoded value to another pointer or namespace needs a customer worker that
decodes and re-encodes it; otherwise the write is refused.

## 5. Key providers (wrapped-key formats)

- **OpenBao Transit** (the customer's Transit): data-key generation under the named key and decryption of the wrapped
  key; `wk` is the base64 of the UTF-8 Transit ciphertext string (`vault:vN:...`), and `kv` is `N`. The exact HTTP paths
  are pinned against the deployed OpenBao version during implementation (T031-040); this contract does not claim them.
- **Static (tests and development only)**: `wk` = 12-byte nonce ‖ AES-256-GCM(KEK, nonce, DEK, AAD =
  `"sagat/static-kek/v1\n" + kid + "\n" + (kv or "")`).
- **Customer-implemented** (cloud KMS): any `wk` up to 1,024 bytes; the provider owns its format.

## 6. Test vectors

`tests/contract/sdk-conformance/vectors/payload-encryption/` holds reviewed vectors made with the static provider and
fixed `DEK`/`iv`, injected through a test-only hook that production builds do not expose: encrypted, compressed-padded-
encrypted (zstd, gzip), compression-only, every level, root and wildcard `ctx.p`, maximum header, and negative cases
(flipped tag, flipped header byte, changed `ctx.n`, changed `ctx.p`, duplicate member, wrong `kid`, `v: 2`, cipher before
compression, `pad` without compression, decompression bomb, bad padding length). Every SDK MUST decode the positive
vectors to the recorded canonical bytes and fail every negative vector with the recorded code.
