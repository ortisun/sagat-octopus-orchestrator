# Manifests and locks: the closed subsets (design 046)

The control plane accepts only what this page allows and refuses the rest with `422 LOCKFILE_INVALID` (a lock) or
`422 DEPENDENCY_MANIFEST_INVALID` (a manifest), naming the rule. Parsers bound nesting to depth 16, strings to 4 KiB and
the whole document to the sizes below before they build any value.

## Names

- **npm.** A name follows the npm rules: at most 214 bytes, lower case, URL-safe characters, an optional `@scope/` prefix.
  Compared as written (npm names are already lower case).
- **Python.** A name follows PEP 508. It is compared after PEP 503 normalisation: lower case, every run of `-`, `_` and
  `.` replaced by one `-`. The normalised form is what the policy, the reservations and the advisories match.

## Python manifest: the PEP 723 block

The first block of the source between `# /// script` and `# ///`, at most 64 KiB, TOML. Keys allowed: `dependencies`
(an array of at most 200 PEP 508 requirement strings) and `requires-python` (a PEP 440 specifier set). Refused: any other
key, a `[tool]` table, a requirement with a URL (`name @ url`), a requirement whose marker cannot be evaluated against
the runtime environment (Python version of the runtime line, `sys_platform == "linux"`, `platform_machine` of each
required target), an extra that names a URL. `requires-python` must admit the runtime's interpreter version.

## TypeScript manifest

A JSON object of at most 64 KiB with `dependencies` (at most 200 entries, name to range) and optionally `overrides`
(the same shape, at most 200 entries). No other key. A range is an npm range of the node semantic-version grammar;
refused: specifiers starting `git`, `git+`, `github:`, `file:`, `link:`, `http:`, `https:`, `workspace:`, `npm:` (an
alias), a path, a tag other than `latest`.

## Python lock: PEP 751 `pylock.toml`, wheels only

At most 256 KiB, at most 2,000 packages. Top level: `lock-version = "1.0"`, `requires-python`, `environments` (the
runtime's Python version on each required target), `created-by`, `packages`. Each package: `name`, `version`,
`index` equal to the gateway's PyPI base (`https://package-gateway/pypi/simple/`, the literal placeholder the resolver
shim writes and the control plane checks), `dependencies` (names of other packages of the lock), and `wheels`, an array
of at least one wheel with `name` (the file name), `url` of the form `https://package-gateway/pypi/files/<sha256>/<name>`,
`size` and `hashes.sha256`. Refused: `sdist`, `vcs`, `directory`, `archive`, any hash algorithm other than SHA-256, a
wheel whose name's distribution and version differ from the package's, a wheel whose tags match no required target and
the runtime's interpreter, two packages with one normalised name, a dependency not in the lock, a package unreachable
from the manifest's requirements, a `marker` that cannot be evaluated.

## TypeScript lock: `package-lock.json` with `lockfileVersion: 3`

At most 256 KiB, at most 2,000 entries. Top level: `name` (`sagat-script`), `lockfileVersion: 3`, `requires: true`,
`packages`. The root entry `""` lists `dependencies` equal to the manifest's. Every other key is a canonical
`node_modules/` path (segments are package names, at most 1,024 bytes, no `..`, nothing absolute). Each entry:
`version`, `resolved` of the form `https://package-gateway/npm/<name>/-/<file>`, `integrity` (`sha512-` only),
optional `dependencies`, `optionalDependencies`, `peerDependencies` (as metadata), `os`, `cpu`, `libc`, `license`,
`hasInstallScript`, `engines`, `bin`. Refused: `link: true`, `inBundle`, a `resolved` outside the gateway, a missing
`integrity`, an entry unreachable from the root, a dependency that npm's resolution from that path would not find in
the lock, `dev: true`.

## Target selection

A required target is `linux-amd64` or `linux-arm64` (deployment setting `SAGAT_SCRIPT_DEPENDENCY_TARGETS`, default
`linux-amd64`). For each required target:

- Python: each package contributes the first wheel, in the order of the wheel tag priority of the runtime interpreter on
  that target (CPython of the runtime's minor, `manylinux_2_36` and older compatible tags, then `none-any`), whose tags
  match.
- npm: each entry is included unless its `os` excludes `linux`, its `cpu` excludes the target's (`x64`, `arm64`) or its
  `libc` excludes `glibc`. An excluded entry that is not optional for its parent refuses the target.

A lock that cannot serve every required target is refused with `PACKAGE_PLATFORM_UNSUPPORTED` naming the package and
the target.

## The closure check

The control plane compares each entry's dependencies with the dependencies the gateway reports for that exact file
(`Requires-Dist` of the wheel's metadata; `dependencies` and `optionalDependencies` of the npm version document). An
entry whose lock dependencies omit a declared, non-optional dependency that applies to a required target is refused.
