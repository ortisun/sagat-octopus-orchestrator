# Q10c ordinary report-before-drain C2 notes

## Artifacts

- C2 tree: `/tmp/q10c-ordinary-inflight-report-v1-c2/`
- C2 patch: `/tmp/q10c-ordinary-inflight-report-v1-c2.patch`
- C2 patch SHA-256: `9450d16aabf1e62035735c18e72b50dd13a89d47aad0bc92f27693b4e8bf073f`
- Approved plan SHA-256: `fb7fb20ce8bd183e31ab87db069073872dd03e07948acb47fe8cdabef1390b97`

C1 remains byte-identical:

- `/tmp/q10c-ordinary-inflight-report-v1.patch`:
  `11b5d9accb012d47bdcb1a5845c2396cfb477cb3fcad49f6d7a8507f90082435`
- C1 producer:
  `901afc1c55156d2195a11b88896f4076761b0561718d7f442d0d1cf447494648`
- C1 consumer:
  `550155fd2e7224d7bfa08e0ff97d8d1474a0ff1c933a4f950a3c321153c1e986`

C2 source hashes:

- producer: `bb08f3f05f836fed7953b508161bf800aa7a68c671845d9057e7ff121a93f7e7`
- consumer: `2757dfd80d3327cb3678bdee16f527daa4223f654f26e292eddd565d199a84a0`
- additive main, unchanged from C1:
  `6cdc1b0c93cc2f897a6bed55ac8948f1228880b5d2ed076f4b5a72ceab71d693`
- coordinator, unchanged from C1:
  `051ab84196901db41a92be07952ead2bda47ac08b0e39cebbef48df9cd7b057a`
- README, unchanged from C1:
  `ef40b425ca65844f589d4121ab2d1fcae9a1ba2d704666b2b4314371a464d3e7`

## Exact C2 delta

1. Replaced the three incorrect `ScopePathSegment.definition_node_id` reads with
   `scope_node_id`. The surrounding `ExecutionInstanceIdentity.definition_node_id` assertions are
   unchanged.
2. Replaced assertions that could format retained protobuf bytes, completion body bytes, or witness
   hashes with boolean assertions carrying fixed messages. The scan covered both producer and
   consumer; normal statuses, counts, and execution identities remain ordinary comparisons.
3. Replaced the prefix and hash-length receipt oracle with an independent reconstruction of:
   - scope `step:<run>:scoped:3:c10:0;7:reserve:<first eight token-hash bytes as lowercase hex>`;
   - inline completion fingerprint parts `complete`, attempt as `u32` big endian, generation as
     `u64` big endian, and the original `output_json` bytes, each length-prefixed by `u64` big
     endian;
   - stored request hash as SHA-256 of those fingerprint bytes;
   - resource `SUCCEEDED|CANCELLING|<version>` and command `<run>:<version>`, where version equals
     both the journal maximum and telemetry head.
4. The receipt query now requires exactly one row for the tenant and key and retains every stored
   row field: tenant, scope, key, request hash, resource, and creation timestamp. Exact replay
   compares the full row through a nonprinting boolean assertion. The count-only receipt member was
   removed from the separate projection identity tuple.

The recipe was checked directly against
`specs/001-platform-foundation/contracts/worker.proto`,
`crates/application/src/worker.rs` (`report_scope`, `completion_idempotency`, `fingerprint`),
`crates/application/src/worker/output.rs` (inline output bytes), and
`crates/infra-postgres/src/definitions.rs::hash`.

## Validation boundary

- `git apply --check --whitespace=error-all /tmp/q10c-ordinary-inflight-report-v1-c2.patch`:
  PASS.
- Patch surface: five files, 2,011 insertions. Only producer and consumer bytes differ from C1.
- Repository writes, formatting, builds, tests, migrations, and commits were not run, as required.
