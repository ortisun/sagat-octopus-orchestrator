//! The `lake` container profile (feature 035, T035-028, D035-048), checked as a configuration: no engine is needed to
//! see what a container would be created with. The behaviour of such a container against a real engine is T035-015
//! (`tests/integration/iceberg-writes/isolation.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use bollard::models::{ContainerCreateBody, MountTypeEnum};
use sagat_infra_sandbox::lake::{
    AttemptMount, LakeLimits, LakeProfileError, LakeSpec, MAX_SLOTS, container_config,
    host_command, uid_for_slot,
};

const ATTEMPT: &str = "0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11";

fn limits() -> LakeLimits {
    LakeLimits {
        memory_bytes: 2 * 1024 * 1024 * 1024,
        processes: 128,
        cpu_millis: 1500,
        work_bytes: 512 * 1024 * 1024,
        tmp_bytes: 64 * 1024 * 1024,
    }
}

fn spec() -> LakeSpec {
    LakeSpec {
        image: "sagat/lake-runtime@sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        attempt_id: ATTEMPT.to_owned(),
        slot: 3,
        group: 10_001,
        mount: AttemptMount {
            volume: "sagat-lake-work".to_owned(),
        },
        limits: limits(),
        owner: "lake-runner-1".to_owned(),
    }
}

fn config() -> ContainerCreateBody {
    container_config(&spec()).expect("a valid specification")
}

fn host(body: &ContainerCreateBody) -> &bollard::models::HostConfig {
    body.host_config.as_ref().expect("a host configuration")
}

#[test]
fn the_attempt_runs_as_the_unprivileged_user_of_its_slot() {
    let body = config();
    let uid = uid_for_slot(3).expect("slot 3 exists");
    // The user of the slot and the group of the lake runner (D035-086): the files the runner leaves in the attempt
    // directory are readable by that group, so the group is what lets the container read them.
    assert_eq!(body.user.as_deref(), Some(format!("{uid}:10001").as_str()));
    assert!(
        uid >= 10_000,
        "an id below 10000 can be a system account: {uid}"
    );
    assert_ne!(
        uid_for_slot(3),
        uid_for_slot(4),
        "two slots must not share a user"
    );
    assert_ne!(uid_for_slot(0), Some(0), "root is never a slot");
}

#[test]
fn every_slot_has_its_own_user_and_none_is_past_the_last() {
    let all: std::collections::BTreeSet<u32> = (0..MAX_SLOTS)
        .map(|slot| uid_for_slot(slot).expect("a slot"))
        .collect();
    assert_eq!(all.len(), MAX_SLOTS as usize);
    assert_eq!(uid_for_slot(MAX_SLOTS), None);
    let mut past = spec();
    past.slot = MAX_SLOTS;
    assert_eq!(container_config(&past).err(), Some(LakeProfileError::Slot));
}

#[test]
fn the_container_has_loopback_and_no_other_network() {
    let body = config();
    assert_eq!(host(&body).network_mode.as_deref(), Some("none"));
    assert_eq!(body.network_disabled, Some(true));
    let host = host(&body);
    assert!(host.dns.is_none() && host.extra_hosts.is_none() && host.links.is_none());
    assert!(host.port_bindings.is_none() && host.publish_all_ports != Some(true));
    assert!(body.exposed_ports.is_none());
}

#[test]
fn the_root_is_read_only_and_no_capability_survives() {
    let body = config();
    let host = host(&body);
    assert_eq!(host.readonly_rootfs, Some(true));
    assert_eq!(host.cap_drop.as_deref(), Some(&["ALL".to_owned()][..]));
    assert!(host.cap_add.is_none());
    assert_eq!(host.privileged, Some(false));
    // The default seccomp profile applies when none is named: `seccomp=unconfined` or any other value must not appear.
    assert_eq!(
        host.security_opt.as_deref(),
        Some(&["no-new-privileges".to_owned()][..])
    );
    assert!(host.devices.is_none() && host.device_cgroup_rules.is_none());
}

#[test]
fn no_namespace_is_shared_with_the_host_or_with_another_container() {
    let body = config();
    let host = host(&body);
    assert_eq!(host.ipc_mode.as_deref(), Some("private"));
    assert!(
        host.pid_mode.is_none(),
        "a pid mode of host or container:x shares the process table"
    );
    assert!(host.uts_mode.is_none() && host.userns_mode.is_none() && host.cgroup_parent.is_none());
    assert!(host.volumes_from.is_none());
    assert!(
        host.binds.is_none(),
        "the only mount is the work volume, declared as a mount"
    );
    assert_eq!(host.init, Some(true));
}

#[test]
fn the_limits_become_cgroup_limits_with_swap_off() {
    let body = config();
    let host = host(&body);
    assert_eq!(host.memory, Some(2 * 1024 * 1024 * 1024));
    assert_eq!(
        host.memory_swap, host.memory,
        "swap equal to memory means no swap"
    );
    assert_eq!(host.pids_limit, Some(128));
    assert_eq!(host.nano_cpus, Some(1_500_000_000));
    assert_eq!(host.oom_kill_disable, None);
}

#[test]
fn core_dumps_are_off_and_a_file_cannot_outgrow_the_work_budget() {
    let body = config();
    let ulimits = host(&body).ulimits.clone().expect("ulimits");
    let find = |name: &str| {
        ulimits
            .iter()
            .find(|u| u.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no {name} ulimit"))
    };
    assert_eq!((find("core").soft, find("core").hard), (Some(0), Some(0)));
    let work = 512 * 1024 * 1024;
    assert_eq!(
        (find("fsize").soft, find("fsize").hard),
        (Some(work), Some(work))
    );
    assert_eq!(
        (find("nofile").soft, find("nofile").hard),
        (Some(1024), Some(1024))
    );
}

#[test]
fn the_only_mount_is_the_attempts_own_directory_of_the_work_volume() {
    let body = config();
    let mounts = host(&body).mounts.clone().expect("mounts");
    assert_eq!(
        mounts.len(),
        1,
        "nothing but the attempt directory: {mounts:?}"
    );
    let mount = &mounts[0];
    assert_eq!(mount.typ, Some(MountTypeEnum::VOLUME));
    assert_eq!(mount.source.as_deref(), Some("sagat-lake-work"));
    assert_eq!(
        mount.target.as_deref(),
        Some(format!("/work/{ATTEMPT}").as_str())
    );
    assert_eq!(mount.read_only, Some(false));
    let options = mount.volume_options.as_ref().expect("volume options");
    assert_eq!(
        options.subpath.as_deref(),
        Some(ATTEMPT),
        "another attempt's directory must not be reachable"
    );
    assert_eq!(options.no_copy, Some(true));
}

#[test]
fn the_only_scratch_is_a_small_tmp_that_cannot_run_code() {
    let body = config();
    let tmpfs = host(&body).tmpfs.clone().expect("tmpfs");
    assert_eq!(tmpfs.len(), 1);
    let options = tmpfs.get("/tmp").expect("/tmp");
    assert_eq!(
        options,
        &format!("rw,noexec,nosuid,nodev,size={}", 64 * 1024 * 1024)
    );
}

#[test]
fn the_environment_is_the_four_names_of_the_contract_and_no_credential() {
    let body = config();
    let mut env = body.env.clone().expect("env");
    env.sort();
    assert_eq!(
        env,
        vec![
            "AWS_EC2_METADATA_DISABLED=true".to_owned(),
            "HOME=/tmp".to_owned(),
            "PATH=/usr/local/bin:/usr/bin:/bin".to_owned(),
            "TMPDIR=/tmp".to_owned(),
        ]
    );
}

#[test]
fn the_command_starts_the_host_isolated_on_the_attempt_directory() {
    let body = config();
    assert_eq!(
        body.cmd.as_deref(),
        Some(
            &[
                "python3".to_owned(),
                "-I".to_owned(),
                "-X".to_owned(),
                "utf8".to_owned(),
                "/opt/sagat/lake/sagat_lake/host.py".to_owned(),
                "--attempt-dir".to_owned(),
                format!("/work/{ATTEMPT}"),
            ][..]
        )
    );
    assert_eq!(
        host_command(ATTEMPT).last().map(String::as_str),
        Some(format!("/work/{ATTEMPT}").as_str())
    );
    assert_eq!(body.working_dir.as_deref(), Some("/tmp"));
}

#[test]
fn nothing_is_logged_and_the_container_is_labelled_for_the_sweep() {
    let body = config();
    let host = host(&body);
    assert_eq!(
        host.log_config.as_ref().and_then(|log| log.typ.as_deref()),
        Some("none")
    );
    assert_eq!(host.auto_remove, Some(false));
    let labels = body.labels.clone().expect("labels");
    assert_eq!(labels.get("sagat.lake").map(String::as_str), Some("1"));
    assert_eq!(
        labels.get("sagat.lake.owner").map(String::as_str),
        Some("lake-runner-1")
    );
    assert_eq!(
        labels.get("sagat.lake.attempt").map(String::as_str),
        Some(ATTEMPT)
    );
    assert_eq!(labels.len(), 3);
}

#[test]
fn an_attempt_id_that_could_leave_the_work_directory_is_refused() {
    for bad in [
        "",
        "..",
        "../etc",
        "a/b",
        "0198f2c4_7b1e",
        "0198F2C4-7B1E",
        "x y",
        "a\nb",
        &"a".repeat(65),
    ] {
        let mut candidate = spec();
        candidate.attempt_id = bad.to_owned();
        assert_eq!(
            container_config(&candidate).err(),
            Some(LakeProfileError::AttemptId),
            "{bad:?}"
        );
    }
}

#[test]
fn a_zero_limit_is_refused_because_zero_is_not_unlimited() {
    for change in [
        (|l: &mut LakeLimits| l.memory_bytes = 0) as fn(&mut LakeLimits),
        |l| l.processes = 0,
        |l| l.cpu_millis = 0,
        |l| l.work_bytes = 0,
        |l| l.tmp_bytes = 0,
    ] {
        let mut candidate = spec();
        change(&mut candidate.limits);
        assert_eq!(
            container_config(&candidate).err(),
            Some(LakeProfileError::Limits)
        );
    }
}

#[test]
fn a_volume_or_image_name_that_is_empty_or_has_a_control_character_is_refused() {
    for change in [
        (|s: &mut LakeSpec| s.image.clear()) as fn(&mut LakeSpec),
        |s| s.image = "img\nage".to_owned(),
        |s| s.mount.volume.clear(),
        |s| s.mount.volume = "vol/../x".to_owned(),
        |s| s.owner.clear(),
        |s| s.owner = "a\tb".to_owned(),
    ] {
        let mut candidate = spec();
        change(&mut candidate);
        assert_eq!(
            container_config(&candidate).err(),
            Some(LakeProfileError::Name),
            "{candidate:?}"
        );
    }
}

#[test]
fn only_hexadecimal_digits_and_hyphens_make_an_attempt_id() {
    for ok in ["0", "abcdef-0123456789", &"f".repeat(64)] {
        let mut candidate = spec();
        candidate.attempt_id = ok.to_owned();
        assert!(container_config(&candidate).is_ok(), "{ok:?}");
    }
    for bad in ["g", "xyz", "abcdeg", "0198f2c4-7b1e-z"] {
        let mut candidate = spec();
        candidate.attempt_id = bad.to_owned();
        assert_eq!(
            container_config(&candidate).err(),
            Some(LakeProfileError::AttemptId),
            "{bad:?}"
        );
    }
}

#[test]
fn a_space_alone_and_a_control_character_alone_each_make_a_name_unacceptable() {
    for bad in ["a b", "a\u{7}b"] {
        for change in [
            (|s: &mut LakeSpec, v: &str| s.image = v.to_owned()) as fn(&mut LakeSpec, &str),
            |s, v| s.owner = v.to_owned(),
            |s, v| s.mount.volume = v.to_owned(),
        ] {
            let mut candidate = spec();
            change(&mut candidate, bad);
            assert_eq!(
                container_config(&candidate).err(),
                Some(LakeProfileError::Name),
                "{bad:?}"
            );
        }
    }
}

#[test]
fn a_name_is_accepted_up_to_its_limit_and_refused_one_past_it() {
    type Set = fn(&mut LakeSpec, String);
    let cases: [(Set, usize); 3] = [
        (|s, v| s.image = v, 512),
        (|s, v| s.owner = v, 128),
        (|s, v| s.mount.volume = v, 255),
    ];
    for (set, longest) in cases {
        let mut at = spec();
        set(&mut at, "a".repeat(longest));
        assert!(container_config(&at).is_ok(), "{longest} characters");
        let mut past = spec();
        set(&mut past, "a".repeat(longest + 1));
        assert_eq!(
            container_config(&past).err(),
            Some(LakeProfileError::Name),
            "{} characters",
            longest + 1
        );
    }
}

#[test]
fn a_volume_name_starts_with_a_letter_or_a_digit() {
    for bad in ["-vol", "_vol", ".vol"] {
        let mut candidate = spec();
        candidate.mount.volume = bad.to_owned();
        assert_eq!(
            container_config(&candidate).err(),
            Some(LakeProfileError::Name),
            "{bad:?}"
        );
    }
    for ok in ["vol", "0vol", "Vol_1.a-b"] {
        let mut candidate = spec();
        candidate.mount.volume = ok.to_owned();
        assert!(container_config(&candidate).is_ok(), "{ok:?}");
    }
}

#[test]
fn the_group_of_the_container_is_the_one_the_spec_names_and_never_root() {
    for group in [1, 10_001, 65_534] {
        let mut candidate = spec();
        candidate.group = group;
        let body = container_config(&candidate).expect("a valid group");
        let uid = uid_for_slot(3).expect("slot");
        assert_eq!(
            body.user.as_deref(),
            Some(format!("{uid}:{group}").as_str())
        );
    }
    let mut root = spec();
    root.group = 0;
    assert_eq!(container_config(&root).err(), Some(LakeProfileError::Group));
}
