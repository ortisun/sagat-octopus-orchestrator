"""The SBOM check of the image build: BuildKit's SPDX attestation, read from an image exported as an OCI archive."""

from __future__ import annotations

import io
import json
import tarfile
from hashlib import sha256
from pathlib import Path
from typing import Any

import pytest

from tools.check_sbom import main, package_names


def blob(content: Any) -> tuple[str, bytes]:
    data = json.dumps(content).encode()
    return "sha256:" + sha256(data).hexdigest(), data


def archive(path: Path, *, packages: list[str] | None, attestation: bool = True) -> Path:
    """An OCI archive shaped like BuildKit's: index.json, an image index, the image and its attestation manifest."""
    blobs: dict[str, bytes] = {}

    def add(content: Any) -> dict[str, Any]:
        digest, data = blob(content)
        blobs[digest.removeprefix("sha256:")] = data
        return {"digest": digest, "size": len(data)}

    image = add({"schemaVersion": 2, "layers": []})
    manifests: list[dict[str, Any]] = [{**image, "platform": {"architecture": "amd64", "os": "linux"}}]
    if attestation and packages is not None:
        spdx = add(
            {
                "_type": "https://in-toto.io/Statement/v0.1",
                "predicate": {"packages": [{"name": name} for name in packages]},
            }
        )
        spdx["annotations"] = {"in-toto.io/predicate-type": "https://spdx.dev/Document"}
        provenance = add({"predicate": {}})
        provenance["annotations"] = {"in-toto.io/predicate-type": "https://slsa.dev/provenance/v1"}
        manifest = add({"schemaVersion": 2, "layers": [spdx, provenance]})
        manifests.append({**manifest, "annotations": {"vnd.docker.reference.type": "attestation-manifest"}})
    index = add({"schemaVersion": 2, "manifests": manifests})
    layout = json.dumps({"schemaVersion": 2, "manifests": [index]}).encode()

    with tarfile.open(path, "w") as tar:
        for name, data in [("index.json", layout), *((f"blobs/sha256/{k}", v) for k, v in blobs.items())]:
            member = tarfile.TarInfo(name)
            member.size = len(data)
            tar.addfile(member, io.BytesIO(data))
    return path


def test_the_names_of_the_packages_in_the_attestation_are_read(tmp_path: Path) -> None:
    exported = archive(tmp_path / "image.tar", packages=["pyiceberg", "pyarrow", "libc6"])
    assert package_names(exported) == {"pyiceberg", "pyarrow", "libc6"}


def test_the_check_passes_when_every_named_package_is_there(tmp_path: Path) -> None:
    exported = archive(tmp_path / "image.tar", packages=["pyiceberg", "pyarrow"])
    assert main([str(exported), "pyiceberg", "pyarrow"]) == 0


def test_the_check_fails_naming_a_package_that_is_missing(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    exported = archive(tmp_path / "image.tar", packages=["pyarrow"])
    assert main([str(exported), "pyiceberg", "pyarrow"]) == 1
    assert "pyiceberg" in capsys.readouterr().err


def test_an_image_built_without_an_attestation_fails_the_check(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    exported = archive(tmp_path / "image.tar", packages=None)
    assert main([str(exported), "pyiceberg"]) == 1
    assert "no SPDX attestation" in capsys.readouterr().err


def test_an_empty_package_list_is_not_an_sbom(tmp_path: Path) -> None:
    exported = archive(tmp_path / "image.tar", packages=[])
    assert main([str(exported), "pyiceberg"]) == 1


def test_a_file_that_is_not_an_archive_fails_the_check(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    broken = tmp_path / "image.tar"
    broken.write_bytes(b"not a tar")
    assert main([str(broken), "pyiceberg"]) == 1
    assert capsys.readouterr().err
