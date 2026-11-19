#!/bin/sh
# Fetches the three Jackson jars of the Java runners into /opt/sagat/lib and checks each against its pinned SHA-256.
# Maven Central has answered GitHub's x86 runners with one and the same wrong body for all three URLs (CI runs
# 43765038237 and 44446964066), so each jar is asked of Maven Central and then of Google's mirror of it, three times
# each; a body whose digest is not the pinned one is never kept. The pins are the integrity guarantee, not the host.
set -eu
dir=/opt/sagat/lib
mkdir -p "$dir"
fetch() { # name path sha256
  for base in https://repo1.maven.org/maven2 https://maven-central.storage-download.googleapis.com/maven2; do
    for try in 1 2 3; do
      if curl -fsSL --retry 2 --connect-timeout 20 -o "$dir/$1.part" "$base/$2" \
        && echo "$3  $dir/$1.part" | sha256sum -c - >/dev/null 2>&1; then
        mv "$dir/$1.part" "$dir/$1"
        return 0
      fi
      echo "fetch-jars: $1 from $base, attempt $try: not the pinned digest or no answer" >&2
      rm -f "$dir/$1.part"
      sleep "$try"
    done
  done
  echo "fetch-jars: $1 could not be fetched with its pinned digest" >&2
  return 1
}
fetch jackson-annotations.jar com/fasterxml/jackson/core/jackson-annotations/2.17.2/jackson-annotations-2.17.2.jar \
  873a606e23507969f9bbbea939d5e19274a88775ea5a169ba7e2d795aa5156e1
fetch jackson-core.jar com/fasterxml/jackson/core/jackson-core/2.17.2/jackson-core-2.17.2.jar \
  721a189241dab0525d9e858e5cb604d3ecc0ede081e2de77d6f34fa5779a5b46
fetch jackson-databind.jar com/fasterxml/jackson/core/jackson-databind/2.17.2/jackson-databind-2.17.2.jar \
  c04993f33c0f845342653784f14f38373d005280e6359db5f808701cfae73c0c
