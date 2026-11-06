#!/usr/bin/env bash
# Smoke test of the lake runtime image: what contracts/lake-runtime.md ("Image") and G035-023 promise of it.
#
#   runners/lake/smoke-image.sh <image>
#
# The container is started the way the sandbox runner starts an attempt: no network, a read-only root, no
# capabilities, no privilege escalation. Exit status 0 only if every promise holds.
set -euo pipefail

image="${1:?usage: smoke-image.sh <image>}"
failures=0

attempt() {
    docker run --rm --read-only --network none --cap-drop ALL --security-opt no-new-privileges "$image" "$@"
}

must_succeed() {
    local what="$1"
    shift
    if attempt "$@" >/dev/null 2>&1; then
        echo "ok    $what"
    else
        echo "FAIL  $what" >&2
        failures=$((failures + 1))
    fi
}

must_fail() {
    local what="$1"
    shift
    if attempt "$@" >/dev/null 2>&1; then
        echo "FAIL  $what" >&2
        failures=$((failures + 1))
    else
        echo "ok    $what"
    fi
}

must_succeed "the host package and PyIceberg 0.12.0 import" python -B -c '
import importlib.metadata
import sagat_lake
assert importlib.metadata.version("pyiceberg") == "0.12.0"
'

for module in pyiceberg.io.pyarrow s3fs gcsfs adlfs pyiceberg.catalog.rest pyiceberg.catalog.hive \
    pyiceberg.catalog.glue pyiceberg.catalog.sql psycopg2; do
    must_succeed "$module imports" python -B -c "import importlib; importlib.import_module('$module')"
done

must_fail "pip is gone" python -m pip --version
must_succeed "the bundled pip that could restore it is gone" sh -c 'test ! -e /usr/local/lib/python3.12/ensurepip'
must_fail "setuptools is gone" python -B -c "import setuptools"
must_fail "there is no compiler" sh -c 'command -v gcc || command -v cc || command -v c++ || command -v g++ || command -v make'
must_succeed "the user is not root" sh -c 'test "$(id -u)" != 0'
must_succeed "the host program is there and read-only" sh -c 'test -d /opt/sagat/lake/sagat_lake && ! touch /opt/sagat/lake/sagat_lake/changed 2>/dev/null'
must_fail "the root filesystem is read-only" sh -c 'touch /usr/local/changed'

if [ "$failures" -ne 0 ]; then
    echo "$failures promise(s) of the image do not hold" >&2
    exit 1
fi
echo "all promises of the image hold"
