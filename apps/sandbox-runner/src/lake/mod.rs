//! Lake mode of the sandbox runner (feature 035, ADR-0079): the lake runner asks for an **attempt**, never for a container.
//!
//! In this mode the process serves only lake containers. It has no token of the control plane and runs no build or script
//! step (a process given the settings of the script mode refuses to start), and it holds the container-engine socket that the
//! lake runner, which holds a destination's credentials, must never hold. It listens on a node-local Unix socket for three
//! calls — start, wait, kill — whose request carries a slot, a budget class and a deadline and nothing that chooses an image,
//! a mount, a capability, a command or a limit.
//!
//! * [`config`]: the settings, and what is refused at start-up.
//! * [`engine`]: the container engine behind a port.
//! * [`admission`]: what a start, a wait and a kill do, with the state read from the engine's labels.
//! * [`api`]: the router, with a closed body for every answer and every refusal.
//! * [`serve`]: the socket, the sweeps, the drain.

pub mod admission;
pub mod api;
pub mod config;
pub mod engine;
pub mod serve;

pub use config::{LakeConfig, SOCKET_NAME};

#[cfg(test)]
mod tests {
    use sagat_infra_lake::sandbox_api as wire;
    use sagat_infra_sandbox::lake as profile;

    /// The lake runner needs the user of a slot before the container exists (the relay checks it), so it computes it from the
    /// wire crate; the container runs as the user the profile computes. Neither crate may depend on the other, and this one
    /// depends on both: the two have to agree, for every slot and for the first one past the last.
    #[test]
    fn the_lake_runner_and_the_profile_give_a_slot_the_same_user() {
        assert_eq!(wire::MAX_SLOTS, profile::MAX_SLOTS);
        for slot in (0..=wire::MAX_SLOTS + 2).chain([u32::MAX - 1, u32::MAX]) {
            assert_eq!(
                wire::uid_for_slot(slot),
                profile::uid_for_slot(slot),
                "slot {slot}"
            );
        }
        assert_eq!(wire::uid_for_slot(0), Some(wire::FIRST_UID));
    }

    /// The attempt id the API accepts is one the profile makes a directory and a container of.
    #[test]
    fn an_attempt_id_the_api_accepts_is_one_the_profile_accepts() {
        for id in [
            "0",
            "a",
            "0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11",
            &"f".repeat(60),
        ] {
            assert!(wire::attempt_id_ok(id), "{id:?}");
            let spec = profile::LakeSpec {
                image: "i".into(),
                attempt_id: id.to_owned(),
                slot: 1,
                group: 1,
                mount: profile::AttemptMount { volume: "v".into() },
                limits: profile::LakeLimits {
                    memory_bytes: 1,
                    processes: 1,
                    cpu_millis: 1,
                    work_bytes: 1,
                    tmp_bytes: 1,
                },
                owner: "o".into(),
                class: "small".into(),
                deadline_epoch_secs: 1,
            };
            assert!(profile::container_config(&spec).is_ok(), "{id:?}");
        }
        // and the other way: what the profile refuses, the API refused first
        for id in ["", "-a", "A", "a b", &"f".repeat(61)] {
            assert!(!wire::attempt_id_ok(id), "{id:?}");
        }
    }
}
