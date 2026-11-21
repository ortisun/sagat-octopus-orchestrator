# The lake runtime image (feature 035; specs/035-iceberg-write-steps/contracts/lake-runtime.md, "Image"; G035-023).
#
# Python 3.12 with PyIceberg and the closure of `requirements.lock`, and nothing else: no compiler, no pip, no
# setuptools. The sandbox runner starts a container from it for each attempt, with a read-only root and a distinct
# user, so the image is built to be read and never written to. (The Debian base keeps `apt` and `dpkg`: a user
# that is not root, without network, on a read-only root cannot use them, and the contract does not ask for more.)
#
# Build from the repository root:
#   docker build -f runners/lake/Dockerfile --target lake-runtime -t sagat-lake-runtime .
#
# The SBOM is BuildKit's attestation of this build, an SPDX document of every Python and Debian package in the image
# (`--sbom=true`; the scanner image is pinned by digest in CI), published with the image. `pip-audit` checks the lock
# itself for known vulnerabilities. The host entrypoint arrives with the host program (it is not written yet).
FROM python:3.12-slim@sha256:b18b261e1b4974b4f321451ad5251ded85abb22c2b8b262da527dda1d031ab30 AS lake-runtime

# Bytecode is compiled by the installation below; the root filesystem is read-only at run time.
ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PYTHONPATH=/opt/sagat/lake

# Wheels only (`--only-binary`), exactly the pinned files (`--require-hashes --no-deps`): a package without a wheel
# fails the build instead of pulling in a compiler. pip removes itself, and the bundled copy it could be restored
# from, in the same layer.
COPY runners/lake/requirements.lock /tmp/requirements.lock
RUN python -m pip install --no-cache-dir --disable-pip-version-check --require-hashes --no-deps --only-binary=:all: \
        -r /tmp/requirements.lock \
    && python -m pip uninstall -y pip setuptools wheel \
    && rm -rf /tmp/requirements.lock /root/.cache /usr/local/lib/python3.12/ensurepip

COPY runners/lake/sagat_lake /opt/sagat/lake/sagat_lake
RUN chmod -R a=rX /opt/sagat/lake

USER 65534:65534
