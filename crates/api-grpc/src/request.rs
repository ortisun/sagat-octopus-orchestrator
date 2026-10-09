//! Parsing and bounds-checking of what a worker sends, before any use case runs.
//!
//! Error messages name the offending field and never echo its value (a lease token must not end up
//! in a log through a validation message).

use sagat_application::worker::TaskRef;
use sagat_domain::execution::{Fence, RunId};
use sagat_domain::workflow::NodeId;
use tonic::Status;
use uuid::Uuid;

use crate::status::invalid;

/// Longest lease token accepted (tokens are 64 hex characters).
const MAX_TOKEN: usize = 128;

/// An attempt the worker reports on, parsed from the message fields.
pub struct Attempt {
    pub run_id: RunId,
    pub node: NodeId,
    pub step_id: Option<Uuid>,
    pub fence: Fence,
    pub lease_token: String,
}

impl Attempt {
    /// Builds the attempt from the fields shared by heartbeat, complete and fail.
    ///
    /// `task_id` is `run/node/generation`, as handed out by `PollTasks`; `run_id`, `step_id` and
    /// `generation` repeat parts of it and, when present, must agree with it.
    pub fn parse(
        task_id: &str,
        run_id: &str,
        step_id: &str,
        attempt: i32,
        generation: i64,
        lease_token: &str,
    ) -> Result<Self, Status> {
        let mut parts = task_id.splitn(3, '/');
        let (Some(task_run), Some(node), Some(task_generation), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid("task_id must be run/node/generation"));
        };
        let task_run = task_run
            .parse::<RunId>()
            .map_err(|_| invalid("task_id has a malformed run id"))?;
        let node = NodeId::parse(node).map_err(|_| invalid("task_id has a malformed node id"))?;
        let task_generation: u64 = task_generation
            .parse()
            .map_err(|_| invalid("task_id has a malformed generation"))?;
        if !run_id.is_empty()
            && run_id
                .parse::<RunId>()
                .map_err(|_| invalid("run_id is malformed"))?
                != task_run
        {
            return Err(invalid("run_id does not match task_id"));
        }
        let generation = u64::try_from(generation)
            .ok()
            .filter(|g| *g >= 1)
            .ok_or_else(|| invalid("generation must be positive"))?;
        if generation != task_generation {
            return Err(invalid("generation does not match task_id"));
        }
        let attempt = u32::try_from(attempt)
            .ok()
            .filter(|a| *a >= 1)
            .ok_or_else(|| invalid("attempt must be positive"))?;
        let step_id = if step_id.is_empty() {
            None
        } else {
            Some(Uuid::parse_str(step_id).map_err(|_| invalid("step_id is malformed"))?)
        };
        if lease_token.is_empty() || lease_token.len() > MAX_TOKEN {
            return Err(invalid("lease_token is missing or too long"));
        }
        Ok(Self {
            run_id: task_run,
            node,
            step_id,
            fence: Fence {
                attempt,
                generation,
            },
            lease_token: lease_token.to_owned(),
        })
    }

    #[must_use]
    pub fn task_ref(&self) -> TaskRef<'_> {
        TaskRef {
            run_id: self.run_id,
            node: &self.node,
            step_id: self.step_id,
            fence: self.fence,
            lease_token: &self.lease_token,
        }
    }
}

/// 8 to 160 printable ASCII characters, the same rule as the REST `Idempotency-Key` header.
pub fn idempotency_key(value: &str) -> Result<&str, Status> {
    if (8..=160).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic()) {
        Ok(value)
    } else {
        Err(invalid(
            "idempotency_key must be 8 to 160 printable ASCII characters",
        ))
    }
}

pub fn worker_id(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| invalid("worker_id is malformed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = "0198c5a0-0000-7000-8000-000000000001";

    fn parse(task_id: &str, run: &str, attempt: i32, generation: i64) -> Result<Attempt, Status> {
        Attempt::parse(task_id, run, "", attempt, generation, "token")
    }

    #[test]
    fn a_consistent_attempt_is_accepted() {
        let attempt = parse(&format!("{RUN}/charge/3"), RUN, 2, 3).expect("valid");
        assert_eq!(attempt.node.as_str(), "charge");
        assert_eq!(attempt.fence.generation, 3);
        assert_eq!(attempt.fence.attempt, 2);
    }

    #[test]
    fn the_repeated_fields_must_agree_with_the_task_id() {
        let other = "0198c5a0-0000-7000-8000-000000000002";
        assert!(parse(&format!("{RUN}/charge/3"), other, 1, 3).is_err());
        assert!(parse(&format!("{RUN}/charge/3"), RUN, 1, 4).is_err());
        assert!(parse(&format!("{RUN}/charge/3"), "", 1, 3).is_ok());
    }

    #[test]
    fn malformed_or_non_positive_fields_are_refused() {
        for task in [
            "",
            "x",
            "a/b",
            &format!("{RUN}/charge"),
            &format!("{RUN}/ch arge/1"),
        ] {
            assert!(parse(task, "", 1, 1).is_err(), "{task}");
        }
        assert!(parse(&format!("{RUN}/charge/1"), "", 0, 1).is_err());
        assert!(parse(&format!("{RUN}/charge/1"), "", -1, 1).is_err());
        assert!(parse(&format!("{RUN}/charge/0"), "", 1, 0).is_err());
        assert!(parse(&format!("{RUN}/charge/1"), "", 1, -5).is_err());
        assert!(Attempt::parse(&format!("{RUN}/charge/1"), "", "", 1, 1, "").is_err());
        assert!(Attempt::parse(&format!("{RUN}/charge/1"), "", "nope", 1, 1, "t").is_err());
    }

    #[test]
    fn a_validation_message_never_echoes_the_secret() {
        let secret = "s3cr3t-lease-token-".repeat(20);
        let error = Attempt::parse(&format!("{RUN}/charge/1"), "", "", 1, 1, &secret)
            .err()
            .expect("too long");
        assert!(!error.message().contains("s3cr3t"));
    }

    #[test]
    fn idempotency_keys_follow_the_rest_rule() {
        assert!(idempotency_key("short").is_err());
        assert!(idempotency_key("has space in it").is_err());
        assert!(idempotency_key(&"k".repeat(161)).is_err());
        assert!(idempotency_key("done-0198c5a0/charge/1").is_ok());
    }
}
