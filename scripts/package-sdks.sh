#!/usr/bin/env bash
set -euo pipefail

readonly SDK_VERSION="0.1.0"
readonly PYTHON_BUILD_VERSION="1.3.0"
readonly MAVEN_INSTALL_PLUGIN_VERSION="3.1.4"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
output_dir="${1:-${repo_root}/target/sdk-packages}"
python_bin="${PYTHON_BIN:-python3}"

if [[ -e "${output_dir}" ]] && [[ -n "$(find "${output_dir}" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
  echo "output directory is not empty: ${output_dir}" >&2
  exit 2
fi

mkdir -p "${output_dir}"
output_dir="$(cd "${output_dir}" && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sagat-sdk-packages.XXXXXX")"
dotnet_build_root=""
cleanup() {
  if [[ -n "${dotnet_build_root}" ]]; then
    chmod -R u+w "${dotnet_build_root}" 2>/dev/null || true
    rm -rf "${dotnet_build_root}"
  fi
  chmod -R u+w "${work_dir}" 2>/dev/null || true
  rm -rf "${work_dir}"
}
trap cleanup EXIT

mkdir -p "${repo_root}/target"
dotnet_build_candidate="${repo_root}/target/.sdk-package-dotnet-build"
if ! mkdir "${dotnet_build_candidate}"; then
  echo "isolated .NET package build directory is already in use: ${dotnet_build_candidate}" >&2
  exit 2
fi
# The trap owns only a directory this process created. A contender that loses this mkdir race must
# leave the winner's active build directory untouched.
dotnet_build_root="${dotnet_build_candidate}"

export LC_ALL=C
export TZ=UTC
source_revision="${SAGAT_SOURCE_REVISION:-$(git -C "${repo_root}" rev-parse HEAD)}"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${repo_root}" show -s --format=%ct "${source_revision}")}"
export PIP_DISABLE_PIP_VERSION_CHECK=1
export PIP_NO_INPUT=1
export NPM_CONFIG_AUDIT=false
export NPM_CONFIG_FUND=false
export NPM_CONFIG_UPDATE_NOTIFIER=false
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
export DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1

for command_name in git node npm "${python_bin}" go dotnet java jar; do
  command -v "${command_name}" >/dev/null || {
    echo "required command not found: ${command_name}" >&2
    exit 2
  }
done

source_root="${work_dir}/source"
mkdir -p "${source_root}"
git -C "${repo_root}" ls-files -z --cached --others --exclude-standard -- \
  sdks/typescript sdks/python sdks/java >"${work_dir}/package-source-files"
"${python_bin}" - "${repo_root}" "${source_root}" "${work_dir}/package-source-files" <<'PY'
from pathlib import Path
import shutil
import sys

root = Path(sys.argv[1])
destination = Path(sys.argv[2])
files = [Path(item.decode()) for item in Path(sys.argv[3]).read_bytes().split(b"\0") if item]
for relative in files:
    target = destination / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / relative, target)
PY
typescript_source="${source_root}/sdks/typescript"
python_source="${source_root}/sdks/python"
dotnet_source="${repo_root}/sdks/dotnet"
java_source="${source_root}/sdks/java"

assert_contains() {
  local path="$1"
  local value="$2"
  if ! grep -Fq "${value}" "${path}"; then
    echo "${path} does not contain expected value: ${value}" >&2
    exit 2
  fi
}

assert_contains "${repo_root}/sdks/typescript/package.json" "\"version\": \"${SDK_VERSION}\""
assert_contains "${repo_root}/sdks/typescript/package.json" '"license": "MIT"'
assert_contains "${repo_root}/sdks/python/pyproject.toml" "version = \"${SDK_VERSION}\""
assert_contains "${repo_root}/sdks/python/pyproject.toml" 'license = "MIT"'
assert_contains "${repo_root}/sdks/go/version.go" "const Version = \"${SDK_VERSION}\""
assert_contains "${repo_root}/sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/SagatOctopusOrchestrator.Sdk.csproj" "<Version>${SDK_VERSION}</Version>"
assert_contains "${repo_root}/sdks/java/pom.xml" "<version>${SDK_VERSION}</version>"
assert_contains "${repo_root}/sdks/java/pom.xml" '<name>MIT License</name>'
assert_contains "${repo_root}/sdks/RELEASE_NOTES.md" "## ${SDK_VERSION}"

echo "==> TypeScript: build, npm pack, clean consumer"
(
  cd "${typescript_source}"
  npm ci --ignore-scripts --no-audit --no-fund --cache "${work_dir}/npm-build-cache"
  npm run build
  npm pack --pack-destination "${output_dir}"
)
typescript_package="${output_dir}/sagat-octopus-orchestrator-sdk-${SDK_VERSION}.tgz"
tar -tzf "${typescript_package}" | grep -x 'package/LICENSE' >/dev/null
mkdir -p "${work_dir}/typescript-consumer"
(
  cd "${work_dir}/typescript-consumer"
  npm init --yes >/dev/null
  npm install --ignore-scripts --no-audit --no-fund --cache "${work_dir}/npm-smoke-cache" "${typescript_package}"
  node --input-type=module -e \
    "import { SDK_VERSION, workflow } from '@sagat-octopus-orchestrator/sdk'; if (SDK_VERSION !== '${SDK_VERSION}' || workflow('package-smoke').end('done').build().metadata.name !== 'package-smoke') process.exit(1);"
)

echo "==> Python: wheel and sdist, clean consumer"
"${python_bin}" -m venv "${work_dir}/python-build"
"${work_dir}/python-build/bin/python" -m pip install "build==${PYTHON_BUILD_VERSION}"
"${work_dir}/python-build/bin/python" -m build \
  --outdir "${output_dir}" \
  "${python_source}"
python_wheel="${output_dir}/sagat_octopus_orchestrator-${SDK_VERSION}-py3-none-any.whl"
python_sdist="${output_dir}/sagat_octopus_orchestrator-${SDK_VERSION}.tar.gz"
"${python_bin}" - "${python_sdist}" "${SOURCE_DATE_EPOCH}" <<'PY'
from copy import copy
import gzip
import io
from pathlib import Path
import sys
import tarfile

path = Path(sys.argv[1])
epoch = int(sys.argv[2])
temporary = path.with_name(path.name + ".tmp")
with tarfile.open(path, "r:gz") as source:
    members = []
    for original in source.getmembers():
        member = copy(original)
        data = source.extractfile(original).read() if original.isfile() else None
        member.mtime = epoch
        member.uid = 0
        member.gid = 0
        member.uname = "root"
        member.gname = "root"
        member.pax_headers = {}
        members.append((member, data))
with temporary.open("wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as destination:
            for member, data in members:
                destination.addfile(member, io.BytesIO(data) if data is not None else None)
temporary.replace(path)
PY
"${python_bin}" - "${python_wheel}" "${python_sdist}" <<'PY'
import sys
import tarfile
import zipfile

wheel, sdist = sys.argv[1:]
with zipfile.ZipFile(wheel) as archive:
    if not any(name.endswith(".dist-info/licenses/LICENSE") for name in archive.namelist()):
        raise SystemExit("Python wheel does not contain the MIT license")
with tarfile.open(sdist, "r:gz") as archive:
    if not any(name.endswith("/LICENSE") for name in archive.getnames()):
        raise SystemExit("Python sdist does not contain the MIT license")
PY
"${python_bin}" -m venv "${work_dir}/python-consumer"
"${work_dir}/python-consumer/bin/python" -m pip install \
  --require-hashes \
  -r "${python_source}/requirements.lock.txt"
"${work_dir}/python-consumer/bin/python" -m pip install --no-deps "${python_wheel}"
"${work_dir}/python-consumer/bin/python" - <<'PY'
from sagat_octopus_orchestrator import __version__, workflow

assert __version__ == "0.1.0"
assert workflow("package-smoke").end("done").build()["metadata"]["name"] == "package-smoke"
PY

echo "==> Go: deterministic module source archive, clean consumer"
go_archive="${output_dir}/sagat-octopus-orchestrator-go-${SDK_VERSION}.tar.gz"
git -C "${repo_root}" ls-files -z --cached --others --exclude-standard -- sdks/go >"${work_dir}/go-files"
"${python_bin}" - "${repo_root}" "${work_dir}/go-files" "${go_archive}" "${SDK_VERSION}" "${SOURCE_DATE_EPOCH}" <<'PY'
import gzip
import io
import os
from pathlib import Path
import sys
import tarfile

root = Path(sys.argv[1])
files = [Path(item.decode()) for item in Path(sys.argv[2]).read_bytes().split(b"\0") if item]
destination = Path(sys.argv[3])
version = sys.argv[4]
epoch = int(sys.argv[5])
prefix = f"sagat-octopus-orchestrator-go-{version}"

with destination.open("wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
            for relative in sorted(files, key=lambda path: path.as_posix()):
                source = root / relative
                module_relative = relative.relative_to("sdks/go")
                info = tarfile.TarInfo(f"{prefix}/{module_relative.as_posix()}")
                data = source.read_bytes()
                info.size = len(data)
                info.mtime = epoch
                info.mode = 0o755 if os.access(source, os.X_OK) else 0o644
                info.uid = 0
                info.gid = 0
                info.uname = "root"
                info.gname = "root"
                archive.addfile(info, io.BytesIO(data))
PY
go_archive_root="sagat-octopus-orchestrator-go-${SDK_VERSION}"
tar -tzf "${go_archive}" | grep -x "${go_archive_root}/LICENSE" >/dev/null
mkdir -p "${work_dir}/go-consumer"
tar -xzf "${go_archive}" -C "${work_dir}"
cat >"${work_dir}/go-consumer/go.mod" <<EOF
module example.com/sagat-package-smoke

go 1.25.0

require github.com/ortisun/sagat-octopus-orchestrator/sdks/go v${SDK_VERSION}

replace github.com/ortisun/sagat-octopus-orchestrator/sdks/go => ../${go_archive_root}
EOF
cat >"${work_dir}/go-consumer/main.go" <<'EOF'
package main

import (
	"fmt"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

func main() {
	if sagat.Version != "0.1.0" {
		panic("unexpected SDK version")
	}
	fmt.Println(sagat.Version)
}
EOF
(
  cd "${work_dir}/go-consumer"
  GOWORK=off \
    GOTOOLCHAIN=go1.25.14+auto \
    GOPATH="${work_dir}/go-path" \
    GOMODCACHE="${work_dir}/go-mod-cache" \
    GOCACHE="${work_dir}/go-build-cache" \
    go run .
)

echo "==> .NET: NuGet package, clean consumer"
if dotnet --list-runtimes | grep -q '^Microsoft.NETCore.App 8\.'; then
  dotnet_smoke_framework="net8.0"
elif dotnet --list-runtimes | grep -q '^Microsoft.NETCore.App 10\.'; then
  dotnet_smoke_framework="net10.0"
else
  echo "the .NET 8 or .NET 10 runtime is required for the package smoke test" >&2
  exit 2
fi
dotnet_build_home="${work_dir}/dotnet-build-home"
mkdir -p "${dotnet_build_home}"
DOTNET_CLI_HOME="${dotnet_build_home}" \
NUGET_PACKAGES="${work_dir}/dotnet-build-packages" \
  dotnet pack \
    "${dotnet_source}/src/SagatOctopusOrchestrator.Sdk/SagatOctopusOrchestrator.Sdk.csproj" \
    --configuration Release \
    --output "${output_dir}" \
    -p:BaseIntermediateOutputPath="${dotnet_build_root}/obj/" \
    -p:BaseOutputPath="${dotnet_build_root}/bin/" \
    -p:ContinuousIntegrationBuild=true \
    -p:DebugSymbols=false \
    -p:DebugType=None \
    -p:PathMap="${dotnet_source}=/_/sdks/dotnet%2C${dotnet_build_root}=/_/build" \
    -p:RepositoryCommit="${source_revision}"
dotnet_package="${output_dir}/SagatOctopusOrchestrator.Sdk.${SDK_VERSION}.nupkg"
"${python_bin}" - "${dotnet_package}" "${SOURCE_DATE_EPOCH}" <<'PY'
from datetime import datetime, timezone
from pathlib import Path
import sys
import zipfile

path = Path(sys.argv[1])
epoch = max(int(sys.argv[2]), 315532800)  # ZIP timestamps start at 1980-01-01.
timestamp = datetime.fromtimestamp(epoch, timezone.utc).timetuple()[:6]
temporary = path.with_name(path.name + ".tmp")
with zipfile.ZipFile(path) as source:
    entries = [(info, source.read(info.filename)) for info in source.infolist()]
with zipfile.ZipFile(temporary, "w") as destination:
    for original, data in entries:
        info = zipfile.ZipInfo(original.filename, timestamp)
        info.compress_type = original.compress_type
        info.external_attr = original.external_attr
        info.internal_attr = original.internal_attr
        info.create_system = original.create_system
        destination.writestr(info, data)
temporary.replace(path)
PY
"${python_bin}" - "${dotnet_package}" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as archive:
    if "LICENSE.txt" not in archive.namelist():
        raise SystemExit("NuGet package does not contain LICENSE.txt")
PY
mkdir -p "${work_dir}/dotnet-consumer"
cat >"${work_dir}/dotnet-consumer/PackageSmoke.csproj" <<EOF
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>${dotnet_smoke_framework}</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="SagatOctopusOrchestrator.Sdk" Version="${SDK_VERSION}" />
  </ItemGroup>
</Project>
EOF
cat >"${work_dir}/dotnet-consumer/Program.cs" <<'EOF'
using SagatOctopusOrchestrator.Sdk;

if (SdkInfo.Version != "0.1.0")
{
    throw new InvalidOperationException("unexpected SDK version");
}
Console.WriteLine(SdkInfo.Version);
EOF
cat >"${work_dir}/dotnet-consumer/NuGet.Config" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="sdk-artifacts" value="${output_dir}" />
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" />
  </packageSources>
</configuration>
EOF
(
  cd "${work_dir}/dotnet-consumer"
  DOTNET_CLI_HOME="${work_dir}/dotnet-consumer-home" \
  NUGET_PACKAGES="${work_dir}/dotnet-consumer-packages" \
    dotnet restore --configfile NuGet.Config
  DOTNET_CLI_HOME="${work_dir}/dotnet-consumer-home" \
  NUGET_PACKAGES="${work_dir}/dotnet-consumer-packages" \
    dotnet run --no-restore
)

echo "==> Java: Maven package, isolated repository consumer"
(
  cd "${java_source}"
  MAVEN_USER_HOME="${work_dir}/maven-build-home" ./mvnw -q -pl sdk \
    -Prelease-artifacts -Dmaven.test.skip=true clean package
)
java_artifact="sagat-octopus-orchestrator-sdk-${SDK_VERSION}"
java_parent_artifact="sagat-octopus-orchestrator-sdk-parent-${SDK_VERSION}"
cp "${java_source}/sdk/target/${java_artifact}.jar" "${output_dir}/${java_artifact}.jar"
cp "${java_source}/sdk/target/${java_artifact}-sources.jar" \
  "${output_dir}/${java_artifact}-sources.jar"
cp "${java_source}/sdk/target/${java_artifact}-javadoc.jar" \
  "${output_dir}/${java_artifact}-javadoc.jar"
cp "${java_source}/sdk/pom.xml" "${output_dir}/${java_artifact}.pom"
cp "${java_source}/pom.xml" "${output_dir}/${java_parent_artifact}.pom"
cp "${repo_root}/sdks/RELEASE_NOTES.md" "${output_dir}/SDK_RELEASE_NOTES-${SDK_VERSION}.md"

# The main JAR already carries the project license as a resource. Classifier JARs are produced from
# source and generated documentation trees, so add the same license while normalizing their ZIP
# metadata to the release epoch.
"${python_bin}" - \
  "${java_source}/sdk/src/main/resources/META-INF/LICENSE" \
  "${SOURCE_DATE_EPOCH}" \
  "${output_dir}/${java_artifact}-sources.jar" \
  "${output_dir}/${java_artifact}-javadoc.jar" <<'PY'
from datetime import datetime, timezone
from pathlib import Path
import sys
import zipfile

license_path = Path(sys.argv[1])
epoch = max(int(sys.argv[2]), 315532800)  # ZIP timestamps start at 1980-01-01.
timestamp = datetime.fromtimestamp(epoch, timezone.utc).timetuple()[:6]
license_data = license_path.read_bytes()

for value in sys.argv[3:]:
    path = Path(value)
    temporary = path.with_name(path.name + ".tmp")
    with zipfile.ZipFile(path) as source:
        entries = [
            (info, source.read(info.filename))
            for info in source.infolist()
            if info.filename != "META-INF/LICENSE"
        ]
    with zipfile.ZipFile(temporary, "w") as destination:
        for original, data in entries:
            info = zipfile.ZipInfo(original.filename, timestamp)
            info.compress_type = original.compress_type
            info.external_attr = original.external_attr
            info.internal_attr = original.internal_attr
            info.create_system = original.create_system
            destination.writestr(info, data)
        license_info = zipfile.ZipInfo("META-INF/LICENSE", timestamp)
        license_info.compress_type = zipfile.ZIP_DEFLATED
        license_info.external_attr = 0o100644 << 16
        destination.writestr(license_info, license_data)
    temporary.replace(path)
PY
jar tf "${output_dir}/${java_artifact}.jar" | grep -x 'META-INF/LICENSE' >/dev/null
jar tf "${output_dir}/${java_artifact}-sources.jar" | grep -x 'META-INF/LICENSE' >/dev/null
jar tf "${output_dir}/${java_artifact}-javadoc.jar" | grep -x 'META-INF/LICENSE' >/dev/null

java_smoke_repo="${work_dir}/java-consumer-repository"
(
  cd "${java_source}"
  MAVEN_USER_HOME="${work_dir}/maven-smoke-home" ./mvnw -q \
    -Dmaven.repo.local="${java_smoke_repo}" \
    "org.apache.maven.plugins:maven-install-plugin:${MAVEN_INSTALL_PLUGIN_VERSION}:install-file" \
    -Dfile="${output_dir}/${java_parent_artifact}.pom" \
    -DpomFile="${output_dir}/${java_parent_artifact}.pom"
  MAVEN_USER_HOME="${work_dir}/maven-smoke-home" ./mvnw -q \
    -Dmaven.repo.local="${java_smoke_repo}" \
    "org.apache.maven.plugins:maven-install-plugin:${MAVEN_INSTALL_PLUGIN_VERSION}:install-file" \
    -Dfile="${output_dir}/${java_artifact}.jar" \
    -DpomFile="${output_dir}/${java_artifact}.pom" \
    -Dsources="${output_dir}/${java_artifact}-sources.jar" \
    -Djavadoc="${output_dir}/${java_artifact}-javadoc.jar"
)
java_smoke_coordinate="${java_smoke_repo}/dev/sagatoctopusorchestrator/sagat-octopus-orchestrator-sdk/${SDK_VERSION}"
cmp "${output_dir}/${java_artifact}-sources.jar" \
  "${java_smoke_coordinate}/${java_artifact}-sources.jar"
cmp "${output_dir}/${java_artifact}-javadoc.jar" \
  "${java_smoke_coordinate}/${java_artifact}-javadoc.jar"
mkdir -p "${work_dir}/java-consumer/src/main/java/example"
cat >"${work_dir}/java-consumer/pom.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>example</groupId>
  <artifactId>sagat-package-smoke</artifactId>
  <version>1.0.0</version>
  <properties>
    <maven.compiler.release>17</maven.compiler.release>
  </properties>
  <dependencies>
    <dependency>
      <groupId>dev.sagatoctopusorchestrator</groupId>
      <artifactId>sagat-octopus-orchestrator-sdk</artifactId>
      <version>${SDK_VERSION}</version>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-compiler-plugin</artifactId>
        <version>3.15.0</version>
      </plugin>
    </plugins>
  </build>
</project>
EOF
cat >"${work_dir}/java-consumer/src/main/java/example/PackageSmoke.java" <<'EOF'
package example;

import dev.sagatoctopusorchestrator.sdk.SagatSdk;

public final class PackageSmoke {
    private PackageSmoke() {}

    public static void main(String[] args) {
        if (!"0.1.0".equals(SagatSdk.VERSION)) {
            throw new IllegalStateException("unexpected SDK version");
        }
        System.out.println(SagatSdk.VERSION);
    }
}
EOF
(
  cd "${work_dir}/java-consumer"
  MAVEN_USER_HOME="${work_dir}/maven-smoke-home" "${java_source}/mvnw" -q \
    -Dmaven.repo.local="${java_smoke_repo}" package
  java -cp "target/classes:${output_dir}/${java_artifact}.jar" example.PackageSmoke
)

echo "==> Artifact checksums"
(
  cd "${output_dir}"
  checksum_file="${work_dir}/SHA256SUMS"
  if command -v sha256sum >/dev/null; then
    find . -maxdepth 1 -type f ! -name SHA256SUMS -print | sed 's#^./##' | sort | xargs sha256sum >"${checksum_file}"
  else
    find . -maxdepth 1 -type f ! -name SHA256SUMS -print | sed 's#^./##' | sort | xargs shasum -a 256 >"${checksum_file}"
  fi
  cp "${checksum_file}" SHA256SUMS
)

echo "SDK packages and clean-consumer smoke tests passed: ${output_dir}"
