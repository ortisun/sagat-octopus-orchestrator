//! What a hostile script cannot do (`FR-005-004`, US04): it has no network, no host, no secrets it
//! was not given, and it cannot use more than its limits.

use std::time::Duration;

use super::{Host, quick, result_of, run, run_with, sandbox};
use sagat_domain::scripts::Language;

const PROBE_TS: &str = "export default async function (input: { url: string }) {\n  try {\n    const r = await fetch(input.url, { signal: AbortSignal.timeout(3000) });\n    return { reached: true, status: r.status };\n  } catch (e) {\n    return { reached: false, error: String((e as Error).cause ?? e) };\n  }\n}\n";

#[tokio::test]
async fn a_script_cannot_reach_the_cloud_metadata_service_or_anything_else_directly() {
    let sandbox = sandbox();
    // An address: there is no route at all, and the connection fails at once (not by timing out).
    for (url, error) in [
        ("http://169.254.169.254/latest/meta-data/", "ENETUNREACH"),
        ("http://[fd00:ec2::254]/latest/meta-data/", "ENETUNREACH"),
        ("http://172.17.0.1:2375/version", "ENETUNREACH"),
        ("http://127.0.0.1:2375/version", "ECONNREFUSED"),
    ] {
        let outcome = run(
            &sandbox,
            Language::TypeScript,
            PROBE_TS,
            &format!(r#"{{"url":"{url}"}}"#),
        )
        .await;
        let result = result_of(&outcome);
        assert!(
            result.contains(r#""reached":false"#) && result.contains(error),
            "{url}: {result} / {}",
            outcome.log
        );
    }
    // A name: there is no resolver, so it is never turned into an address.
    for url in [
        "http://metadata.google.internal/computeMetadata/v1/",
        "http://host.docker.internal/",
        "https://example.com/",
    ] {
        let outcome = run(
            &sandbox,
            Language::TypeScript,
            PROBE_TS,
            &format!(r#"{{"url":"{url}"}}"#),
        )
        .await;
        let result = result_of(&outcome);
        assert!(
            result.contains(r#""reached":false"#),
            "{url}: {result} / {}",
            outcome.log
        );
    }
}

#[tokio::test]
async fn the_only_way_out_is_the_supervisor_which_refuses_and_records_what_it_refused() {
    let sandbox = sandbox();
    let host = Host {
        allow: vec![("https://api.example.test/ok", "fine")],
        ..Host::default()
    };
    let outcome = run_with(
        &sandbox,
        Language::TypeScript,
        "export default async function (_i: unknown, octopus: any) {\n  const ok = await (await octopus.fetch('https://api.example.test/ok')).text();\n  let refused = 'no';\n  try { await octopus.fetch('http://169.254.169.254/latest/meta-data/?token=SECRET123'); } catch (e) { refused = String((e as Error).message); }\n  return { ok, refused };\n}\n",
        "{}",
        sagat_domain::scripts::Policy::default(),
        &host,
    )
    .await;
    let result = result_of(&outcome);
    assert!(
        result.contains(r#""ok":"fine""#) && result.contains("denied"),
        "{result} / {}",
        outcome.log
    );
    assert_eq!(
        outcome.denied,
        vec!["fetch http://169.254.169.254".to_owned()],
        "only the destination, never the path or query"
    );
}

#[tokio::test]
async fn a_secret_is_available_only_if_declared_and_never_reaches_the_log() {
    let sandbox = sandbox();
    let host = Host {
        known_secrets: vec![("api-key", "s3cr3t-value-0123456789")],
        ..Host::default()
    };
    let built = super::build(&sandbox, Language::Python, "def run(input, octopus):\n    key = octopus.secret('api-key')\n    print('using', key)\n    try:\n        octopus.secret('other')\n        other = 'leaked'\n    except Exception as e:\n        other = 'denied'\n    return {'len': len(key), 'other': other}\n").await.expect("builds");
    use sagat_application::sandbox::{RunRequest, Sandbox};
    let outcome = sandbox
        .run(
            RunRequest {
                language: Language::Python,
                artifact: built.artifact,
                input: b"{}".to_vec(),
                input_schema: None,
                output_schema: None,
                policy: sagat_domain::scripts::Policy::default(),
                secrets_to_hide: Vec::new(),
            },
            &host,
        )
        .await;
    assert_eq!(
        result_of(&outcome),
        r#"{"len":23,"other":"denied"}"#,
        "{}",
        outcome.log
    );
    assert!(
        !outcome.log.contains("s3cr3t-value-0123456789"),
        "the secret reached the log: {}",
        outcome.log
    );
    assert!(outcome.log.contains("[REDACTED]"), "{}", outcome.log);
    assert_eq!(outcome.denied, vec!["secret other".to_owned()]);
}

#[tokio::test]
async fn a_script_sees_neither_the_host_nor_the_engine_socket_nor_the_environment() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::Python,
        "import os\ndef run(input, octopus):\n    return {\n        'sock': os.path.exists('/var/run/docker.sock'),\n        'users': os.path.exists('/Users'),\n        'host_etc': open('/etc/hostname').read().strip() != '',\n        'uid': os.getuid(),\n        'write_root': _can_write('/opt/x'),\n        'write_tmp': _can_write('/tmp/x'),\n        'env': sorted(k for k in os.environ if 'SAGAT' in k or 'DATABASE' in k or 'SECRET' in k or 'TOKEN' in k),\n    }\ndef _can_write(p):\n    try:\n        open(p, 'w').write('x'); return True\n    except OSError:\n        return False\n",
        "{}",
    )
    .await;
    let result = result_of(&outcome);
    assert!(result.contains(r#""sock":false"#), "{result}");
    assert!(result.contains(r#""users":false"#), "{result}");
    assert!(result.contains(r#""uid":65534"#), "{result}");
    assert!(result.contains(r#""write_root":false"#), "{result}");
    assert!(result.contains(r#""write_tmp":true"#), "{result}");
    assert!(result.contains(r#""env":[]"#), "{result}");
}

#[tokio::test]
async fn a_script_cannot_become_root_or_gain_capabilities() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::Python,
        "import os, ctypes\ndef run(input, octopus):\n    status = open('/proc/self/status').read()\n    caps = {l.split(':')[0]: l.split(':')[1].strip() for l in status.splitlines() if l.startswith(('CapEff','CapPrm','CapBnd','NoNewPrivs','Seccomp:'))}\n    try:\n        os.setuid(0); root = True\n    except OSError:\n        root = False\n    libc = ctypes.CDLL(None)\n    mount = libc.mount(b'none', b'/mnt', b'tmpfs', 0, None)\n    return {'caps': caps, 'root': root, 'mount_ok': mount == 0}\n",
        "{}",
    )
    .await;
    let result = result_of(&outcome);
    assert!(
        result.contains(r#""CapEff":"0000000000000000""#),
        "{result}"
    );
    assert!(
        result.contains(r#""CapBnd":"0000000000000000""#),
        "{result}"
    );
    assert!(result.contains(r#""NoNewPrivs":"1""#), "{result}");
    assert!(
        result.contains(r#""Seccomp:":"2""#) || result.contains(r#""Seccomp":"2""#),
        "{result}"
    );
    assert!(
        result.contains(r#""root":false"#) && result.contains(r#""mount_ok":false"#),
        "{result}"
    );
}

#[tokio::test]
async fn a_memory_bomb_is_killed_by_the_memory_limit() {
    let sandbox = sandbox();
    let outcome = run_with(
        &sandbox,
        Language::Python,
        "def run(input, octopus):\n    hog = []\n    while True:\n        hog.append(bytearray(8 * 1024 * 1024))\n",
        "{}",
        quick(Duration::from_secs(30), 64),
        &Host::default(),
    )
    .await;
    assert_eq!(
        result_of(&outcome),
        "FAILED SCRIPT_OUT_OF_MEMORY",
        "{} exit={:?}",
        outcome.log,
        outcome.exit_code
    );
}

#[tokio::test]
async fn a_fork_bomb_cannot_exceed_the_process_limit_and_does_not_hurt_the_host() {
    let sandbox = sandbox();
    let outcome = run_with(
        &sandbox,
        Language::Python,
        "import os\ndef run(input, octopus):\n    made = 0\n    try:\n        while made < 5000:\n            pid = os.fork()\n            if pid == 0:\n                import time; time.sleep(20); os._exit(0)\n            made += 1\n    except OSError as e:\n        return {'made': made, 'stopped': True}\n    return {'made': made, 'stopped': False}\n",
        "{}",
        quick(Duration::from_secs(30), 128),
        &Host::default(),
    )
    .await;
    let result = result_of(&outcome);
    assert!(
        result.contains(r#""stopped":true"#),
        "{result} / {}",
        outcome.log
    );
    assert_eq!(
        sandbox.sweep(Duration::ZERO).await,
        0,
        "the children went with the container"
    );
}

#[tokio::test]
async fn a_cpu_burn_is_stopped_at_the_wall_clock() {
    let sandbox = sandbox();
    let began = std::time::Instant::now();
    let outcome = run_with(
        &sandbox,
        Language::TypeScript,
        "export default function () { for (;;) {} }\n",
        "{}",
        quick(Duration::from_secs(3), 128),
        &Host::default(),
    )
    .await;
    assert_eq!(result_of(&outcome), "FAILED SCRIPT_TIMEOUT");
    assert!(
        began.elapsed() < Duration::from_secs(20),
        "stopped in {:?}",
        began.elapsed()
    );
}

#[tokio::test]
async fn a_log_flood_is_cut_and_the_run_ends_instead_of_filling_the_supervisor() {
    let sandbox = sandbox();
    let outcome = run_with(
        &sandbox,
        Language::TypeScript,
        "export default function () { const line = 'x'.repeat(1000); for (let i = 0; i < 2_000_000; i++) console.log(line); }\n",
        "{}",
        quick(Duration::from_secs(20), 128),
        &Host::default(),
    )
    .await;
    assert!(
        outcome.log.len() <= 64 * 1024,
        "kept {} bytes",
        outcome.log.len()
    );
    assert!(outcome.log_truncated);
    assert!(outcome.result.is_err());
}

#[tokio::test]
async fn a_disk_flood_in_the_scratch_area_is_refused_at_its_size() {
    let sandbox = sandbox();
    let outcome = run_with(
        &sandbox,
        Language::Python,
        "def run(input, octopus):\n    written = 0\n    try:\n        with open('/tmp/fill', 'wb') as f:\n            while True:\n                f.write(b'0' * (1024 * 1024)); f.flush(); written += 1\n    except OSError:\n        return {'mib': written}\n",
        "{}",
        quick(Duration::from_secs(30), 128),
        &Host::default(),
    )
    .await;
    let result = result_of(&outcome);
    assert!(result.contains(r#""mib":"#), "{result} / {}", outcome.log);
    let mib: u64 = result
        .split(':')
        .nth(1)
        .and_then(|v| v.trim_end_matches('}').parse().ok())
        .unwrap_or(u64::MAX);
    assert!(
        (1..16).contains(&mib),
        "wrote {mib} MiB into a 16 MiB scratch area"
    );
}

#[tokio::test]
async fn a_script_cannot_execute_from_the_scratch_area() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::Python,
        "import os, stat, subprocess\ndef run(input, octopus):\n    open('/tmp/x.sh', 'w').write('#!/bin/sh\\necho hi\\n')\n    os.chmod('/tmp/x.sh', 0o755)\n    try:\n        out = subprocess.run(['/tmp/x.sh'], capture_output=True, timeout=5)\n        return {'ran': out.returncode == 0}\n    except Exception as e:\n        return {'ran': False}\n",
        "{}",
    )
    .await;
    assert!(
        result_of(&outcome).contains(r#""ran":false"#),
        "{} / {}",
        result_of(&outcome),
        outcome.log
    );
}
