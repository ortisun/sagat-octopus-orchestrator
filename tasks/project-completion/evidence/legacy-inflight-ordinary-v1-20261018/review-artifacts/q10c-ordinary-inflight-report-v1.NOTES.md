# Q10c ordinary report-before-drain draft notes

## Provenance

- Approved plan: `/tmp/q10c-ordinary-inflight-report-fixture-plan.md`
- Approved plan SHA-256: `fb7fb20ce8bd183e31ab87db069073872dd03e07948acb47fe8cdabef1390b97`
- Frozen producer production commit: `6c44a0e49670cf19660792cd1518d241e6c99b3b`
- Final repository HEAD observed read-only: `e51b33901a0cb9dad3758644aaa3dbaf1bc70962`
- Patch: `/tmp/q10c-ordinary-inflight-report-v1.patch`
- Patch SHA-256: `11b5d9accb012d47bdcb1a5845c2396cfb477cb3fcad49f6d7a8507f90082435`

## Unchanged reviewed base hashes

- `tests/fixtures/legacy-collection-output/producer.rs`: `3b4851d6715129a7c823bc4c83dbcc5a5690517d366cf49bca4d0acafd3de10f`
- `tests/integration/workflow-primitives/legacy_collection_output_view.rs`: `e420ad4e68fa8e371fa919866e22f73d7b0b7e30c8151555e4f29eafecf5d778`
- `scripts/test-legacy-collection-output-view.sh`: `7e7f2b47bf8991b90b2433b28ac9fb130ec3bef7f32282b5a3c31bb4b9dbe5f2`
- `tests/integration/workflow-primitives/main.rs`: `1093f64cae07513469648d528a032185f6154b4a49453efa4641318557d63cc9`
- `tests/support/mod.rs`: `77b0d2c05603bdb20c0cfeebf8553c0e415f6779ab704ba7be7ea2ebc080b0c1`
- `tests/support/api.rs`: `7a535bf44c27f2a2c6e6e20e03936836b8f6f060de74551422ca4a437245f086`
- `tests/integration/workflow-primitives/api_reopen.rs`: `aa338b162bf786a4ba47219169092399b99e87bc2c70d3d89470029ab034c5b9`

## Draft file hashes

- `producer.rs`: `901afc1c55156d2195a11b88896f4076761b0561718d7f442d0d1cf447494648`
- `legacy_collection_inflight_report_v1.rs`: `550155fd2e7224d7bfa08e0ff97d8d1474a0ff1c933a4f950a3c321153c1e986`
- additive `main.rs`: `6cdc1b0c93cc2f897a6bed55ac8948f1228880b5d2ed076f4b5a72ceab71d693`
- coordinator script: `051ab84196901db41a92be07952ead2bda47ac08b0e39cebbef48df9cd7b057a`
- fixture README: `ef40b425ca65844f589d4121ab2d1fcae9a1ba2d704666b2b4314371a464d3e7`

## Review surface and claim

The patch contains exactly five files and 1,839 inserted lines. The producer overlay is the only
change placed over the frozen archive, apart from its generated old-main registration. The current
consumer uses `attach_existing_db` and `start_on_db`, reconstructs `Platform`, connects transport,
restores the persisted worker ID/session, and sends a fresh current JWT. It does not provision,
bind, grant, or register in the current phase.

The retained request is submitted and exactly replayed before the first current scheduler tick.
The assertions cover `SUCCEEDED` plus `RUN_DATA_LIMIT`, absent refused forward output, stable
ledger/source/authority, no report outbox, typed audit and worker provenance, cold journal replay,
normal durable drain discovery, and the engine's actual compensation target identity/key and
terminal `COMPENSATED` run.

The scope remains the one ordinary report-before-drain case. External provider behavior, Applied,
the other ordering, post-ACK process restart, output-dependent compensation, and the broader
authority/race/fault matrix remain open.

## Checks and execution gap

- `git apply --check --whitespace=error-all /tmp/q10c-ordinary-inflight-report-v1.patch`: PASS at
  final observed HEAD `e51b33901a0cb9dad3758644aaa3dbaf1bc70962`.
- `git apply --stat`: five files, 1,839 insertions.
- Repository writes, formatting, builds, tests, migrations, commits, and fixture execution were not
  run, as required by the drafting assignment. The exact frozen/current compile and runtime outcome
  therefore remains for the root executor; a real failure should be retained as the gap rather than
  treated as a passing compatibility guarantee.
