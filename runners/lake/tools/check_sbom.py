"""Checks the SBOM that BuildKit attached to the lake runtime image.

    python3 tools/check_sbom.py <image.tar> <package>...

`docker buildx build --sbom=... --output type=oci,dest=<image.tar>` exports the image with an attestation manifest whose
SPDX layer lists every package BuildKit found in it. This reads that list and fails when there is none, or when a
package the image must contain is not in it. It is a CI tool and is not copied into the image.
"""

from __future__ import annotations

import json
import sys
import tarfile
from pathlib import Path
from typing import Any

SPDX = "https://spdx.dev/Document"
ATTESTATION = "attestation-manifest"


class NoSbom(Exception):
    """The archive holds no SPDX attestation, or not one this reader can follow."""


def _blob(archive: tarfile.TarFile, digest: str) -> Any:
    algorithm, _, value = digest.partition(":")
    member = archive.extractfile(f"blobs/{algorithm}/{value}")
    if member is None:
        raise NoSbom(f"the archive has no blob {digest}")
    return json.loads(member.read())


def package_names(path: Path) -> set[str]:
    """The names of the packages in the SPDX attestation of an image exported as an OCI archive."""
    try:
        with tarfile.open(path) as archive:
            layout = json.loads((archive.extractfile("index.json") or _missing("index.json")).read())
            names: set[str] = set()
            found = False
            for entry in layout["manifests"]:
                index = _blob(archive, entry["digest"])
                for manifest in index.get("manifests", []):
                    if manifest.get("annotations", {}).get("vnd.docker.reference.type") != ATTESTATION:
                        continue
                    for layer in _blob(archive, manifest["digest"]).get("layers", []):
                        if layer.get("annotations", {}).get("in-toto.io/predicate-type") != SPDX:
                            continue
                        found = True
                        statement = _blob(archive, layer["digest"])
                        names |= {package["name"] for package in statement["predicate"]["packages"]}
    except (tarfile.TarError, OSError, KeyError, ValueError) as error:
        raise NoSbom(f"{path} cannot be read as an OCI archive: {error}") from error
    if not found:
        raise NoSbom(f"{path} has no SPDX attestation: build with --sbom")
    return names


def _missing(name: str) -> Any:
    raise NoSbom(f"the archive has no {name}")


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: check_sbom.py <image.tar> <package>...", file=sys.stderr)
        return 2
    try:
        names = package_names(Path(argv[0]))
    except NoSbom as error:
        print(f"check_sbom: {error}", file=sys.stderr)
        return 1
    if not names:
        print("check_sbom: the SBOM lists no package", file=sys.stderr)
        return 1
    missing = [package for package in argv[1:] if package not in names]
    if missing:
        print(f"check_sbom: not in the SBOM: {', '.join(missing)}", file=sys.stderr)
        return 1
    print(f"SBOM lists {len(names)} packages, including {', '.join(argv[1:])}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
