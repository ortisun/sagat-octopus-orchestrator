#!/usr/bin/env bash
# Shows, for real, what `--strict-checksums` in .mvn/maven.config does and does not do. It builds a scratch "remote"
# repository holding one small artifact (jsr305, a transitive dependency of the SDK) with a checksum that is WRONG,
# a copy of ~/.m2 as the local repository (so that the plugins are there, about 100 MB), and resolves the artifact
# with and without the flag. Nothing in the repository is touched. Run it after building once (needs ~/.m2 filled).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
real="${M2_REPO:-$HOME/.m2/repository}"
scratch="$(mktemp -d)"
[ -n "${KEEP:-}" ] || trap 'rm -rf "$scratch"' EXIT

art=com/google/code/findbugs/jsr305/3.0.2
test -f "$real/$art/jsr305-3.0.2.jar" || { echo "build the SDK once first: ./mvnw package" >&2; exit 2; }
grep -q -- '--strict-checksums' "$here/.mvn/maven.config" || { echo ".mvn/maven.config does not say --strict-checksums" >&2; exit 2; }

echo "== scratch repositories" >&2
mkdir -p "$scratch/local"
cp -R "$real/." "$scratch/local/"
remote="$scratch/remote/$art"
mkdir -p "$remote"
cp "$real/$art/jsr305-3.0.2.jar" "$real/$art/jsr305-3.0.2.pom" "$remote/"
shasum -a 1 "$remote/jsr305-3.0.2.pom" | cut -d' ' -f1 > "$remote/jsr305-3.0.2.pom.sha1"
cat > "$scratch/settings.xml" <<XML
<settings><mirrors><mirror><id>central</id><mirrorOf>*</mirrorOf><url>file://$scratch/remote</url></mirror></mirrors></settings>
XML

# Two scratch projects: one with the real .mvn/maven.config of the SDK, one with none.
for variant in strict plain; do
  dir="$scratch/$variant"
  mkdir -p "$dir/.mvn/wrapper"
  cp "$here/mvnw" "$dir/mvnw"
  cp "$here/.mvn/wrapper/maven-wrapper.properties" "$dir/.mvn/wrapper/"
  if [ "$variant" = strict ]; then cp "$here/.mvn/maven.config" "$dir/.mvn/maven.config"; fi
  cat > "$dir/pom.xml" <<XML
<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
  <groupId>scratch</groupId><artifactId>scratch-$variant</artifactId><version>0</version><packaging>pom</packaging>
  <dependencies><dependency><groupId>com.google.code.findbugs</groupId><artifactId>jsr305</artifactId><version>3.0.2</version></dependency></dependencies>
</project>
XML
done

resolve() { # variant, label
  local dir="$scratch/$1"
  rm -rf "$scratch/local/$art"
  local log="$scratch/$1.log"
  set +e
  (cd "$dir" && ./mvnw -B -s "$scratch/settings.xml" -Dmaven.repo.local="$scratch/local" -Daether.offline.protocols=file \
      org.apache.maven.plugins:maven-dependency-plugin:3.10.0:resolve) > "$log" 2>&1
  local status=$?
  set -e
  printf '%-58s exit=%s  jar in local repository: %s\n' "$2" "$status" "$([ -f "$scratch/local/$art/jsr305-3.0.2.jar" ] && echo yes || echo no)"
  grep -i -m2 -E 'checksum' "$log" | sed 's/^/    /' | cut -c1-200 || true
}

echo "== a jar whose .sha1 is wrong"
echo 0000000000000000000000000000000000000000 > "$remote/jsr305-3.0.2.jar.sha1"
resolve strict "with .mvn/maven.config (--strict-checksums):"
resolve plain "without it:"

echo "== a jar with NO checksum file at all"
rm -f "$remote/jsr305-3.0.2.jar.sha1"
resolve strict "with .mvn/maven.config (--strict-checksums):"
resolve plain "without it:"

echo "== a jar whose .sha1 is right (the control)"
shasum -a 1 "$remote/jsr305-3.0.2.jar" | cut -d' ' -f1 > "$remote/jsr305-3.0.2.jar.sha1"
resolve strict "with .mvn/maven.config (--strict-checksums):"

echo "== a jar that is already in the local repository, remote checksum wrong"
echo 0000000000000000000000000000000000000000 > "$remote/jsr305-3.0.2.jar.sha1"
mkdir -p "$scratch/local/$art"
cp "$remote/jsr305-3.0.2.jar" "$remote/jsr305-3.0.2.pom" "$scratch/local/$art/"
shasum -a 1 "$remote/jsr305-3.0.2.pom" | cut -d' ' -f1 > "$scratch/local/$art/jsr305-3.0.2.pom.sha1"
set +e
(cd "$scratch/strict" && ./mvnw -B -o -s "$scratch/settings.xml" -Dmaven.repo.local="$scratch/local" org.apache.maven.plugins:maven-dependency-plugin:3.10.0:resolve) > "$scratch/cached.log" 2>&1
status=$?
set -e
printf '%-58s exit=%s  (nothing is downloaded, so nothing is verified)\n' "with --strict-checksums, offline, artifact cached:" "$status"
