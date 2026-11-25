# REST routes (design 046)

Under `/v1`. Every route answers `404` alike for another tenant's object and a missing one, and audits the attempt.
Bodies are bounded at 64 KiB unless stated. Task 046-1 transcribes this table into `openapi.yaml` with the shapes named.
Runner routes are in `runner-protocol.md`.

| Method and path | Permission | Body and answer |
|---|---|---|
| `PATCH /namespaces/{ns}/scripts/{script}` (feature 005's) | `script:write` | the draft gains `manifest` (TypeScript) and `lockfile` (an author lock) |
| `POST /namespaces/{ns}/scripts/{script}/versions` (feature 005's) | `script:publish` | `201` for a framework-only version or a refused author lock's `422`; `202` with the version `BUILDING` for a manifest |
| `POST /namespaces/{ns}/scripts/{script}:resolve-again` | `script:publish` | `202`, a new version of the latest draft with a fresh resolution |
| `GET /namespaces/{ns}/scripts/{script}/versions/{n}/packages` | `script:read` | the lock summary (name, version, target set, source), the report (verdict, findings), the blocks |
| `GET /namespaces/{ns}/scripts/{script}/versions/{n}/sbom` | `script:read` | the CycloneDX document |
| `GET`, `PUT /namespaces/{ns}/script-policy` | `script:read`, `script:policy:manage` | the document with `revision`; `PUT` takes `expectedRevision` |
| `GET`, `POST /namespaces/{ns}/script-policy/package-rules`, `DELETE .../{id}` | `script:read`, `script:policy:manage` | a rule `{kind, ecosystem, name, versions?}` |
| `GET`, `POST /namespaces/{ns}/script-policy/waivers`, `DELETE .../{id}` | `script:read`, `script:policy:manage` | `{advisory, ecosystem, package, version, reason, expiresAt}` (at most 90 days; never a malicious advisory) |
| `GET /namespaces/{ns}/supply-chain/findings` | `script:read` | findings by version, paged, filterable by kind and severity |
| `GET /namespaces/{ns}/supply-chain/blocks` | `script:read` | blocks touching the namespace |
| `GET`, `POST /package-reservations`, `DELETE .../{id}` | `script:read`, `package:manage` | `{ecosystem, kind: "name" \| "npm-scope", name}` |
| `PUT /namespaces/{ns}/packages/{ecosystem}` | `package:upload` | the archive (at most 64 MiB, its own router); `201`, `200` for the same file, `409 PACKAGE_VERSION_EXISTS` |
| `GET /packages/{ecosystem}`, `DELETE /packages/{ecosystem}/{name}/{version}` | `script:read`, `package:manage` | the tenant's uploads |
| `POST /admin/advisories:import`, `POST /admin/advisories/{import}:confirm` | `advisory:import` (platform operator) | an archive upload, or the confirmation of a held import |
| `DELETE /admin/script-blocks/{id}` | `script:block:release` (platform operator) | releases a malware block once its advisory is withdrawn |
