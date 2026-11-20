#!/usr/bin/env bash
# Build the committed Rust crate, then compile/run a consumer of its extracted artifact.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:?usage: package-rust-sdk.sh <output-directory>}"
if ! git -C "$repo_root" diff --quiet HEAD -- sdks/rust; then
  echo 'Commit Rust SDK source changes before packaging.' >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" ls-files --others --exclude-standard -- sdks/rust)" ]]; then
  echo 'Commit new Rust SDK files before packaging.' >&2
  exit 2
fi
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sagat-rust-package.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
mkdir -p "$output_dir" "$work_dir/source" "$work_dir/artifact" "$work_dir/consumer/src"
output_dir="$(cd "$output_dir" && pwd)"
source_revision="$(git -C "$repo_root" rev-parse HEAD)"
git -C "$repo_root" archive "$source_revision" sdks/rust | tar -x -C "$work_dir/source"
export RUSTUP_TOOLCHAIN="$(python3 - "$repo_root/rust-toolchain.toml" <<'PY'
import sys,tomllib
from pathlib import Path
print(tomllib.loads(Path(sys.argv[1]).read_text())['toolchain']['channel'])
PY
)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${work_dir}/target}"
python3 - "$repo_root" "$CARGO_TARGET_DIR" <<'PY'
import sys
from pathlib import Path
target=Path(sys.argv[2])
if not target.is_absolute() or target.resolve().is_relative_to(Path(sys.argv[1]).resolve()):
    raise SystemExit('CARGO_TARGET_DIR must be an owned absolute directory outside the repository')
PY
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
version="$(python3 - "$work_dir/source/sdks/rust/Cargo.toml" <<'PY'
import sys,tomllib
from pathlib import Path
package=tomllib.loads(Path(sys.argv[1]).read_text())['package']
if package['license']!='MIT':raise SystemExit('Rust SDK must carry the MIT license')
print(package['version'])
PY
)"
crate_name="sagat-octopus-orchestrator-sdk-${version}"
if [[ -e "$output_dir/${crate_name}.crate" ]]; then
  echo 'Refusing to reuse an existing Rust SDK artifact.' >&2
  exit 2
fi
df -h "$CARGO_TARGET_DIR" 2>/dev/null || df -h "$work_dir"
cargo package --manifest-path "$work_dir/source/sdks/rust/Cargo.toml" --locked
cp "$CARGO_TARGET_DIR/package/${crate_name}.crate" "$output_dir/"
python3 - "$output_dir/${crate_name}.crate" "$work_dir/artifact" "$work_dir/consumer" "$version" <<'PY'
import json,sys,tarfile
from pathlib import Path
archive,destination,consumer,version=sys.argv[1:]
with tarfile.open(archive,'r:gz') as source:
    if not any(name.endswith('/LICENSE') for name in source.getnames()):raise SystemExit('Rust crate lacks its license')
    source.extractall(destination,filter='data')
dependency=Path(destination)/f'sagat-octopus-orchestrator-sdk-{version}'
root=Path(consumer)
root.joinpath('Cargo.toml').write_text('[package]\nname="sagat-artifact-consumer"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nsagat-octopus-orchestrator-sdk={path='+json.dumps(str(dependency))+'}\n')
root.joinpath('src/main.rs').write_text('''use sagat_octopus_orchestrator_sdk::{VERSION,Client,ClientOptions,TokenSource,workflow::Builder};
fn main(){
    assert_eq!(VERSION,'''+json.dumps(version)+''');
    let flow=Builder::new("artifact-smoke");flow.end("done",None);
    assert_eq!(flow.build().expect("workflow")["metadata"]["name"],"artifact-smoke");
    let client=Client::new(ClientOptions::new("https://platform.example/v1","payments",TokenSource::bearer("fixture-token"))).expect("client");
    assert!(!format!("{client:?}").contains("fixture-token"));
    println!("{VERSION}");
}
''')
PY
cargo run --manifest-path "$work_dir/consumer/Cargo.toml"
cargo run --manifest-path "$work_dir/consumer/Cargo.toml" --locked
printf 'source_revision=%s\ncrate_version=%s\ntoolchain=%s\n' "$source_revision" "$version" "$RUSTUP_TOOLCHAIN" >"$output_dir/RUST_SDK_PROVENANCE.txt"
echo "Rust SDK package and artifact consumer passed: $output_dir/${crate_name}.crate"
