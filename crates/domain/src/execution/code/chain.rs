//! The command chain of a code run (design 044, Technical Plan 6.2; the corpus README
//! `specs/044-workflow-as-code/contracts/goldens/README.md`, "The chain").
//!
//! For command `k` with kind `kind`, target `target` and the SHA-256 `body` of its canonical body:
//!
//! ```text
//! d(k) = RFC 8785 JSON of {"seq": k, "kind": kind, "target": target, "body": hex(body)}
//! H(0) = 32 zero bytes
//! H(k) = SHA-256(H(k - 1) ‖ uint32 big-endian length of d(k) ‖ d(k))
//! ```
//!
//! The domain folds the chain itself and leaves the two byte-level operations, the SHA-256 and the RFC 8785 bytes of
//! `d(k)`, to a port, [`CommandDigester`], which `crates/infra-ir` implements: the domain gains no dependency and the
//! dependency points inward.

use std::fmt;

/// The kinds of command a workflow function makes, one per `CommandKind` value of `worker.proto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommandKind {
    Activity,
    Script,
    TimerStart,
    TimerCancel,
    WaitEvent,
    Child,
    Approval,
    StepCancel,
    SagaOpen,
    SagaClose,
    SagaAbort,
    MarkerPatch,
    MarkerSideEffect,
    SetState,
    UpdateAccept,
    UpdateReject,
    UpdateComplete,
    ConsumeSignals,
    Complete,
    Fail,
    ContinueAsNew,
    AcknowledgeCancellation,
}

impl CommandKind {
    /// Every kind, in the order of `worker.proto`.
    pub const ALL: [Self; 22] = [
        Self::Activity,
        Self::Script,
        Self::TimerStart,
        Self::TimerCancel,
        Self::WaitEvent,
        Self::Child,
        Self::Approval,
        Self::StepCancel,
        Self::SagaOpen,
        Self::SagaClose,
        Self::SagaAbort,
        Self::MarkerPatch,
        Self::MarkerSideEffect,
        Self::SetState,
        Self::UpdateAccept,
        Self::UpdateReject,
        Self::UpdateComplete,
        Self::ConsumeSignals,
        Self::Complete,
        Self::Fail,
        Self::ContinueAsNew,
        Self::AcknowledgeCancellation,
    ];

    /// The chain's `kind` string: the `worker.proto` value's name without its `COMMAND_` prefix, in lower case
    /// (`COMMAND_TIMER_START` is `timer_start`).
    #[must_use]
    pub const fn chain_name(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Script => "script",
            Self::TimerStart => "timer_start",
            Self::TimerCancel => "timer_cancel",
            Self::WaitEvent => "wait_event",
            Self::Child => "child",
            Self::Approval => "approval",
            Self::StepCancel => "step_cancel",
            Self::SagaOpen => "saga_open",
            Self::SagaClose => "saga_close",
            Self::SagaAbort => "saga_abort",
            Self::MarkerPatch => "marker_patch",
            Self::MarkerSideEffect => "marker_side_effect",
            Self::SetState => "set_state",
            Self::UpdateAccept => "update_accept",
            Self::UpdateReject => "update_reject",
            Self::UpdateComplete => "update_complete",
            Self::ConsumeSignals => "consume_signals",
            Self::Complete => "complete",
            Self::Fail => "fail",
            Self::ContinueAsNew => "continue_as_new",
            Self::AcknowledgeCancellation => "acknowledge_cancellation",
        }
    }

    /// The kind whose chain name is `name`, if any.
    #[must_use]
    pub fn from_chain_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.chain_name() == name)
    }

    /// A work command: one that creates a `c-<n>` step and carries a code-profile fragment (Technical Plan 5.1).
    #[must_use]
    pub const fn is_work(self) -> bool {
        matches!(
            self,
            Self::Activity
                | Self::Script
                | Self::TimerStart
                | Self::WaitEvent
                | Self::Child
                | Self::Approval
        )
    }

    /// A command that ends the run or continues it as new; it is sent last in its activation and may be deferred
    /// (Technical Plan 4.4).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Fail | Self::ContinueAsNew | Self::AcknowledgeCancellation
        )
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.chain_name())
    }
}

/// The two byte-level operations the chain needs, implemented outside the domain (`crates/infra-ir`).
pub trait CommandDigester {
    /// The SHA-256 of `bytes`.
    fn sha256(&self, bytes: &[u8]) -> [u8; 32];

    /// The RFC 8785 bytes of `d(k)`: `{"seq": seq, "kind": kind, "target": target, "body": <64 lowercase hexadecimal
    /// characters of body>}`.
    fn command_digest_input(&self, seq: u32, kind: &str, target: &str, body: [u8; 32]) -> Vec<u8>;
}

/// One command as the chain sees it: its number, its kind, its target and the SHA-256 of its canonical body.
///
/// The fold hashes the target it is given. The caller derives it from the command as Technical Plan 6.2's table says
/// (a timer's own `c-<n>`, `""` for the typed commands), from the server's own numbering and records rather than a
/// worker's claim; the completion path (Tasks 044-5 and 044-9) holds that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainedCommand<'a> {
    pub seq: u32,
    pub kind: CommandKind,
    pub target: &'a str,
    pub body_sha256: [u8; 32],
}

/// Why a chain could not be extended.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    /// The command numbers of a run go 1, 2, 3...: the next command must be numbered one more than the count.
    #[error("command {got} does not follow command {after}")]
    OutOfOrder { after: u32, got: u32 },
    /// The bytes of `d(k)` do not fit their 32-bit length prefix.
    #[error("the digest input of command {seq} is longer than 2^32 - 1 bytes")]
    InputTooLong { seq: u32 },
    /// A stored chain whose hash and count cannot belong together.
    #[error("a stored command chain of {count} commands has an impossible hash")]
    Inconsistent { count: u32 },
}

/// `H(k)` of a run after its `k`-th command, with `k`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandChain {
    hash: [u8; 32],
    count: u32,
}

impl CommandChain {
    /// `H(0)`: 32 zero bytes, before any command.
    pub const EMPTY: Self = Self {
        hash: [0; 32],
        count: 0,
    };

    /// A chain read back from storage (`runs.code_command_chain`, `runs.code_command_count`).
    ///
    /// # Errors
    /// [`ChainError::Inconsistent`] when the hash is `H(0)` (32 zero bytes) with a count, or another hash without one:
    /// only the empty chain has no command.
    pub fn restore(hash: [u8; 32], count: u32) -> Result<Self, ChainError> {
        if (hash == [0; 32]) != (count == 0) {
            return Err(ChainError::Inconsistent { count });
        }
        Ok(Self { hash, count })
    }

    /// The 32 bytes of `H(k)`.
    #[must_use]
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }

    /// `k`: the number of commands chained.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// `H(k)` from `H(k - 1)` and command `k`, which must be numbered `k`.
    ///
    /// # Errors
    ///
    /// [`ChainError::OutOfOrder`] when the command is not numbered one more than the count, and
    /// [`ChainError::InputTooLong`] when its `d(k)` does not fit its length prefix.
    pub fn extend(
        &self,
        digester: &(impl CommandDigester + ?Sized),
        command: &ChainedCommand<'_>,
    ) -> Result<Self, ChainError> {
        if self.count.checked_add(1) != Some(command.seq) {
            return Err(ChainError::OutOfOrder {
                after: self.count,
                got: command.seq,
            });
        }
        let d = digester.command_digest_input(
            command.seq,
            command.kind.chain_name(),
            command.target,
            command.body_sha256,
        );
        let length =
            u32::try_from(d.len()).map_err(|_| ChainError::InputTooLong { seq: command.seq })?;
        let mut input = Vec::with_capacity(32 + 4 + d.len());
        input.extend_from_slice(&self.hash);
        input.extend_from_slice(&length.to_be_bytes());
        input.extend_from_slice(&d);
        Ok(Self {
            hash: digester.sha256(&input),
            count: command.seq,
        })
    }

    /// The chain after every command of `commands`, in order.
    ///
    /// # Errors
    ///
    /// The first [`ChainError`] of [`CommandChain::extend`].
    pub fn extend_all<'a>(
        &self,
        digester: &(impl CommandDigester + ?Sized),
        commands: impl IntoIterator<Item = &'a ChainedCommand<'a>>,
    ) -> Result<Self, ChainError> {
        commands
            .into_iter()
            .try_fold(*self, |chain, command| chain.extend(digester, command))
    }
}

impl fmt::Debug for CommandChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut hex = String::with_capacity(64);
        for byte in self.hash {
            hex.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
            hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
        }
        f.debug_struct("CommandChain")
            .field("hash", &hex)
            .field("count", &self.count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A digester that records what the fold asks of it and answers with values that depend on every input byte and
    /// on their order, so a fold that drops, reorders or mislengths a part is seen.
    #[derive(Default)]
    struct Recording {
        sha_inputs: RefCell<Vec<Vec<u8>>>,
        d_inputs: RefCell<Vec<DigestRequest>>,
    }

    /// The number, the kind, the target and the body digest the fold asked `d(k)` for.
    type DigestRequest = (u32, String, String, [u8; 32]);

    impl CommandDigester for Recording {
        fn sha256(&self, bytes: &[u8]) -> [u8; 32] {
            self.sha_inputs.borrow_mut().push(bytes.to_vec());
            let mut out = [0_u8; 32];
            for (index, byte) in bytes.iter().enumerate() {
                let slot = index % 32;
                out[slot] = out[slot].rotate_left(3)
                    ^ byte.wrapping_add(u8::try_from(index % 251).unwrap_or(0));
            }
            out
        }

        fn command_digest_input(
            &self,
            seq: u32,
            kind: &str,
            target: &str,
            body: [u8; 32],
        ) -> Vec<u8> {
            self.d_inputs
                .borrow_mut()
                .push((seq, kind.to_owned(), target.to_owned(), body));
            format!("{seq}|{kind}|{target}|{}", body[0]).into_bytes()
        }
    }

    fn command(seq: u32, kind: CommandKind, target: &str, first: u8) -> ChainedCommand<'_> {
        let mut body_sha256 = [0_u8; 32];
        body_sha256[0] = first;
        ChainedCommand {
            seq,
            kind,
            target,
            body_sha256,
        }
    }

    #[test]
    fn the_chain_fold_prefixes_lengths_and_keeps_order() {
        let digester = Recording::default();
        let first = command(1, CommandKind::Activity, "payments.charge", 7);
        let second = command(2, CommandKind::TimerStart, "c-2", 9);
        let one = CommandChain::EMPTY
            .extend(&digester, &first)
            .expect("command 1 follows the empty chain");
        let two = one
            .extend(&digester, &second)
            .expect("command 2 follows command 1");

        assert_eq!(
            *digester.d_inputs.borrow(),
            vec![
                (
                    1,
                    "activity".to_owned(),
                    "payments.charge".to_owned(),
                    first.body_sha256
                ),
                (
                    2,
                    "timer_start".to_owned(),
                    "c-2".to_owned(),
                    second.body_sha256
                ),
            ],
            "d(k) is asked for with the number, the chain name of the kind, the target and the body digest"
        );
        let d1 = b"1|activity|payments.charge|7".to_vec();
        let d2 = b"2|timer_start|c-2|9".to_vec();
        let mut expected_first = vec![0_u8; 32];
        expected_first.extend_from_slice(&u32::try_from(d1.len()).expect("short").to_be_bytes());
        expected_first.extend_from_slice(&d1);
        let mut expected_second = one.hash().to_vec();
        expected_second.extend_from_slice(&u32::try_from(d2.len()).expect("short").to_be_bytes());
        expected_second.extend_from_slice(&d2);
        assert_eq!(
            *digester.sha_inputs.borrow(),
            vec![expected_first, expected_second],
            "H(k) hashes H(k - 1), the big-endian length of d(k) and d(k), in that order"
        );
        assert_eq!(two.count(), 2);
        let last_input = digester.sha_inputs.borrow()[1].clone();
        assert_eq!(
            two.hash(),
            digester.sha256(&last_input),
            "H(2) is the digester's hash of its input"
        );

        let swapped = Recording::default();
        let other_first = command(1, CommandKind::TimerStart, "c-2", 9);
        let other_second = command(2, CommandKind::Activity, "payments.charge", 7);
        let reordered = CommandChain::EMPTY
            .extend_all(&swapped, [&other_first, &other_second])
            .expect("numbered in order");
        assert_ne!(
            reordered.hash(),
            two.hash(),
            "the order of the commands is part of the chain"
        );
    }

    #[test]
    fn a_command_out_of_order_does_not_extend_the_chain() {
        let digester = Recording::default();
        let skipped = command(2, CommandKind::Complete, "", 1);
        assert_eq!(
            CommandChain::EMPTY.extend(&digester, &skipped),
            Err(ChainError::OutOfOrder { after: 0, got: 2 })
        );
        let restored = CommandChain::restore([5; 32], 4).expect("a chain of four commands");
        let repeated = command(4, CommandKind::Complete, "", 1);
        assert_eq!(
            restored.extend(&digester, &repeated),
            Err(ChainError::OutOfOrder { after: 4, got: 4 })
        );
        assert!(
            digester.sha_inputs.borrow().is_empty(),
            "nothing is hashed for a refused command"
        );
    }

    #[test]
    fn a_stored_chain_must_be_the_empty_one_or_have_commands() {
        assert_eq!(CommandChain::restore([0; 32], 0), Ok(CommandChain::EMPTY));
        assert_eq!(
            CommandChain::restore([5; 32], 0),
            Err(ChainError::Inconsistent { count: 0 }),
            "no command, so the hash is H(0)"
        );
        assert_eq!(
            CommandChain::restore([0; 32], 4),
            Err(ChainError::Inconsistent { count: 4 }),
            "H(0) has no command"
        );
    }

    #[test]
    fn every_kind_has_the_chain_name_of_its_proto_value() {
        let names: Vec<&str> = CommandKind::ALL
            .iter()
            .map(|kind| kind.chain_name())
            .collect();
        assert_eq!(
            names,
            [
                "activity",
                "script",
                "timer_start",
                "timer_cancel",
                "wait_event",
                "child",
                "approval",
                "step_cancel",
                "saga_open",
                "saga_close",
                "saga_abort",
                "marker_patch",
                "marker_side_effect",
                "set_state",
                "update_accept",
                "update_reject",
                "update_complete",
                "consume_signals",
                "complete",
                "fail",
                "continue_as_new",
                "acknowledge_cancellation",
            ]
        );
        for kind in CommandKind::ALL {
            assert_eq!(CommandKind::from_chain_name(kind.chain_name()), Some(kind));
        }
        assert_eq!(CommandKind::from_chain_name("COMMAND_ACTIVITY"), None);
        let work: Vec<CommandKind> = CommandKind::ALL
            .into_iter()
            .filter(|kind| kind.is_work())
            .collect();
        assert_eq!(
            work,
            [
                CommandKind::Activity,
                CommandKind::Script,
                CommandKind::TimerStart,
                CommandKind::WaitEvent,
                CommandKind::Child,
                CommandKind::Approval
            ]
        );
    }
}
