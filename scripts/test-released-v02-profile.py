#!/usr/bin/env python3
"""Build verified release/current phases in distinct targets; run a self-contained upgrade test."""
from __future__ import annotations
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import unquote, urlparse

RELEASE = 'bfda342073db71e84f30bdc79ccf3fbc97a7153c'
OVERLAYS = ('tests/fixtures/released-v02/phase.rs', 'tests/fixtures/released-v02/wait-state.sql',
            'tests/fixtures/released-v02/inflight.rs', 'tests/fixtures/released-v02/control.rs',
            'tests/fixtures/released-v02/cutover.rs', 'tests/fixtures/released-v02/callback_cutover.rs')
MAIN = 'tests/integration/workflow-primitives/main.rs'
SUFFIX = b'\n#[path = "../../fixtures/released-v02/phase.rs"]\nmod released_v02_phase;\n'
TARGET = 'integration_workflow_primitives'
SCENARIOS = {
    'redis-callback-cutover': ('released_v02_callback_cutover::released_callback_writer_is_fenced_before_redis_activation',
                              'tests/integration/workflow-primitives/released_v02_callback_cutover.rs', 'redis-callback-cutover.txt'),
    'applied-output-quota': ('released_v02_inflight::a_released_applied_report_refuses_output_and_compensates_original_effects_over_quota',
                              'tests/integration/workflow-primitives/released_v02_inflight.rs', 'applied-output-quota.txt'),
    'scoped-event-cutover': ('released_v02_cutover::released_dispatcher_credentials_and_process_are_fenced_before_public_scoped_events',
                             'tests/integration/workflow-primitives/released_v02_cutover.rs', 'scoped-event-cutover.txt'),
    'active-data': ('released_v02_upgrade::released_v02_active_rows_upgrade_progress_and_remain_readable_by_the_release',
                    'tests/integration/workflow-primitives/released_v02_upgrade.rs', 'upgrade.txt'),
    'inflight-worker': ('released_v02_inflight::a_released_worker_reports_and_replays_its_old_lease_after_server_upgrade',
                        'tests/integration/workflow-primitives/released_v02_inflight.rs', 'inflight-worker.txt'),
    'applied-lookup': ('released_v02_inflight::a_released_worker_reports_and_replays_its_old_applied_lookup_after_server_upgrade',
                        'tests/integration/workflow-primitives/released_v02_inflight.rs', 'applied-lookup.txt'),
}


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(65536), b''):
            digest.update(block)
    return digest.hexdigest()


def source_manifest(repo: Path) -> dict[str, bytes]:
    archive = subprocess.check_output(['git', 'archive', RELEASE], cwd=repo)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        return {member.name: tar.extractfile(member).read() for member in tar if member.isfile()}


def regular_tree(source: Path) -> set[str]:
    if not stat.S_ISDIR(source.lstat().st_mode):
        raise RuntimeError('release source root must be a real directory')
    actual = set()
    for base, directories, files in os.walk(source, followlinks=False):
        for name in directories:
            if not stat.S_ISDIR((Path(base) / name).lstat().st_mode):
                raise RuntimeError('release source contains a symlink or special directory')
        for name in files:
            path = Path(base) / name
            if not stat.S_ISREG(path.lstat().st_mode):
                raise RuntimeError('release source contains a symlink or special file')
            actual.add(str(path.relative_to(source)))
    return actual


def current_manifest(repo: Path) -> list[dict]:
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=repo)
    entries = []
    for raw in sorted(set(names.split(b'\0')) - {b''}):
        name = os.fsdecode(raw)
        path = repo / name
        if not stat.S_ISREG(path.lstat().st_mode):
            raise RuntimeError(f'current input must be an accounted regular file: {name}')
        entries.append({'path': name, 'sha256': sha(path)})
    return entries


def verify_source(source: Path, original: dict[str, bytes], overlays: dict[str, bytes]) -> list[dict]:
    """Every regular archive file and every additional compiled overlay is accounted for."""
    expected = dict(original)
    expected.update(overlays)
    actual = regular_tree(source)
    if actual != set(expected):
        raise RuntimeError('release source contains missing or undeclared files')
    checked = []
    for name, data in sorted(expected.items()):
        if (source / name).read_bytes() != data:
            raise RuntimeError(f'release source differs outside its declared overlay: {name}')
        checked.append({'path': name, 'sha256': hashlib.sha256(data).hexdigest()})
    return checked


def run(command: list[str], cwd: Path, env: dict[str, str], log: Path, limit: int) -> None:
    started = time.monotonic()
    with log.open('wb') as output:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdout=output, stderr=subprocess.STDOUT,
                                 start_new_session=True)
        try:
            code = child.wait(timeout=limit)
        except BaseException:
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
            raise
    record = {'command': command, 'exitCode': code, 'seconds': round(time.monotonic() - started, 2)}
    log.with_suffix('.json').write_text(json.dumps(record, indent=2) + '\n')
    if code:
        raise RuntimeError(f'phase failed with exit {code}; private diagnostics: {log}')


def verify_selected_phase(log: Path, case: str) -> None:
    output = log.read_text()
    summaries = [line for line in output.splitlines() if line.startswith('test result:')]
    if (f'test {case} ...' not in output or not summaries
            or not summaries[-1].startswith('test result: ok. 1 passed; 0 failed; 0 ignored;')):
        raise RuntimeError('the selected phase must execute exactly one successful outer test')


def verify_phase_result(result: Path, scenario: str) -> str:
    if not result.is_file():
        raise RuntimeError('selected phase result is missing')
    value = json.loads(result.read_text())
    if value.get('scenario') != scenario or value.get('releaseCommit') != RELEASE:
        raise RuntimeError('selected phase result identity does not match the executed scenario')
    return sha(result)


def artifact(log: Path, source: Path) -> dict:
    matches = []
    for line in log.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == TARGET and item.get('executable'):
            matches.append(item)
    if len(matches) != 1:
        raise RuntimeError('build must identify exactly one phase executable')
    item = matches[0]
    package = Path(unquote(urlparse(item['package_id'].removeprefix('path+')).path)).resolve()
    if package != (source / 'tests').resolve():
        raise RuntimeError('Cargo artifact did not come from the expected source tree')
    if Path(item['target']['src_path']).resolve() != (source / MAIN).resolve():
        raise RuntimeError('Cargo artifact has an unexpected test entry point')
    return item


def main() -> None:
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepared-run', type=Path, help='reuse this agent-owned verified archive/build cache')
    parser.add_argument('--scenario', choices=tuple(SCENARIOS), default='active-data')
    args = parser.parse_args()
    if os.name != 'posix':
        raise RuntimeError('the released-source fixtures require a Unix host')
    case, coordinator, transcript_name = SCENARIOS[args.scenario]
    repo = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip()).resolve()
    if subprocess.check_output(['git', 'rev-parse', 'v0.2.0^{commit}'], cwd=repo, text=True).strip() != RELEASE:
        raise RuntimeError('v0.2.0 does not match the immutable reviewed release')
    configured_target = os.environ.get('CARGO_TARGET_DIR')
    if not configured_target or not Path(configured_target).is_absolute():
        raise RuntimeError('set CARGO_TARGET_DIR to an owned absolute private current target')
    current_target = Path(configured_target).resolve()
    if current_target.is_relative_to(repo):
        raise RuntimeError('the current target must be outside the repository')
    if shutil.disk_usage(repo).free < 8 * 1024**3:
        raise RuntimeError('less than8GiB free; stop before heavy builds')
    root = args.prepared_run.resolve() if args.prepared_run else Path(tempfile.mkdtemp(prefix='sagat-released-v02-'))
    source = root / 'source'
    original = source_manifest(repo)
    if args.prepared_run:
        if (root / 'release-commit.txt').read_text().strip() != RELEASE:
            raise RuntimeError('prepared run has the wrong release identity')
    else:
        source.mkdir()
        # Files come only from this repository's immutable Git tree, never an external archive.
        for name, data in original.items():
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (root / 'release-commit.txt').write_text(RELEASE + '\n')
    regular_tree(source)  # Refuse linked overlay destinations before writing anything.
    overlays = {name: (repo / name).read_bytes() for name in OVERLAYS}
    overlays[MAIN] = original[MAIN] + SUFFIX
    for name, data in overlays.items():
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
    verified = verify_source(source, original, overlays)
    production = [entry for entry in verified if entry['path'].startswith(('crates/', 'apps/', '.cargo/'))
                  or entry['path'] in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')]
    overlay_manifest = [entry for entry in verified if entry['path'] in overlays]
    old_target_file = root / 'old-target.txt'
    old_target = Path(old_target_file.read_text().strip()).resolve() if old_target_file.exists() else root / 'old-target'
    if old_target == current_target or old_target.is_relative_to(source):
        raise RuntimeError('old/current targets must be distinct and outside the source tree')
    old_target.mkdir(exist_ok=True)
    marker = old_target / '.sagat-released-source.json'
    identity = {'releaseCommit': RELEASE, 'source': str(source.resolve())}
    if marker.exists():
        if json.loads(marker.read_text()) != identity:
            raise RuntimeError('old target belongs to a different release source')
    else:
        if any(old_target.iterdir()):
            raise RuntimeError('cannot adopt an unmarked nonempty old build target')
        marker.write_text(json.dumps(identity, indent=2) + '\n')
    old_target_file.write_text(str(old_target) + '\n')
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='1',
               CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    env.pop('SAGAT_TEST_DATABASE_URL', None)
    old_env = dict(env, CARGO_TARGET_DIR=str(old_target), CARGO_PROFILE_DEV_DEBUG='0',
                   CARGO_PROFILE_TEST_DEBUG='0', SAGAT_RELEASE_PROFILE_COMMIT=RELEASE)
    command = ['cargo', 'test', '-p', 'sagat-tests', '--test', TARGET, '--locked', '--no-run', '--message-format=json']
    run(command, source, old_env, root / 'old-build.txt', 1200)
    old = artifact(root / 'old-build.txt', source)
    binary = root / 'released-v02-phase'
    shutil.copy2(old['executable'], binary)
    binary.chmod(0o700)
    # Revalidate after compilation, including the main.rs registration and SQL witness overlay.
    verify_source(source, original, overlays)
    build = {'releaseCommit': RELEASE, 'packageId': old['package_id'],
             'productionFilesVerified': len(production),
             'productionTreeSha256': hashlib.sha256(json.dumps(production, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
             'overlays': overlay_manifest, 'binary': str(binary), 'binarySha256': sha(binary),
             'oldTarget': str(old_target), 'currentTarget': str(current_target),
             'currentCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()}
    build_path = root / 'build.json'
    build_path.write_text(json.dumps(build, indent=2) + '\n')
    current_head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    build['currentCommit'] = current_head
    current_before = current_manifest(repo)
    (root / 'current-source.json').write_text(json.dumps(current_before, indent=2) + '\n')
    run(command, repo, env, root / 'current-build.txt', 1200)
    if current_manifest(repo) != current_before or subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() != current_head:
        raise RuntimeError('current source or HEAD changed during compilation; discard this build')
    build['currentSourceSha256'] = hashlib.sha256(json.dumps(current_before, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    current = artifact(root / 'current-build.txt', repo)
    current_binary = root / 'current-phase'
    shutil.copy2(current['executable'], current_binary)
    current_binary.chmod(0o700)
    build['currentBinarySha256'] = sha(current_binary)
    build['scenario'] = args.scenario
    build['coordinatorPath'] = coordinator
    build['coordinatorSha256'] = next(entry['sha256'] for entry in current_before if entry['path'] == coordinator)
    build_path.write_text(json.dumps(build, indent=2) + '\n')
    result = Path(tempfile.mkdtemp(prefix='outcome-', dir=root)) / 'result.json'
    gate_env = dict(env, SAGAT_V02_BUILD=str(build_path), SAGAT_V02_RESULT=str(result),
                    SAGAT_TEST_AS_RUNTIME_ROLE='1', RUST_TEST_THREADS='1')
    transcript = root / transcript_name
    run([str(current_binary), case, '--ignored', '--exact', '--nocapture'], repo, gate_env, transcript, 360)
    verify_selected_phase(transcript, case)
    result_sha = verify_phase_result(result, args.scenario)
    (root / 'outcome.json').write_text(json.dumps({
        'scenario': args.scenario, 'resultPath': str(result.relative_to(root)),
        'resultSha256': result_sha, 'buildSha256': sha(build_path),
        'transcriptSha256': sha(transcript)}, indent=2) + '\n')
    print(f'Released-source compatibility evidence: {root}')
    print('\n'.join(transcript.read_text().splitlines()[-12:]))


if __name__ == '__main__':
    main()
