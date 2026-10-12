//! Typed, bounded evidence sent by the sandbox supervisor when it refuses a network operation.
//!
//! Destination data is intentionally absent. A URL, hostname, headers and payload are untrusted
//! and may contain secrets; the durable evidence is the closed category tied to a validated lease.

use sagat_domain::execution::RunId;
use sagat_domain::identity::NamespaceId;
use sagat_domain::scripts::TestRunId;
use uuid::Uuid;

use crate::entropy::Token;
use crate::error::AppError;

const MAX_LEASE_TOKEN_BYTES: usize = 128;

/// The only denial categories accepted from the sandbox supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IncidentCategory {
    EgressDisabled,
    UnsupportedScheme,
    InvalidHost,
    PortNotAllowed,
    NotAllowlisted,
    ForbiddenAddress,
    NoAddress,
}

impl IncidentCategory {
    pub const ALL: [Self; 7] = [
        Self::EgressDisabled,
        Self::UnsupportedScheme,
        Self::InvalidHost,
        Self::PortNotAllowed,
        Self::NotAllowlisted,
        Self::ForbiddenAddress,
        Self::NoAddress,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EgressDisabled => "EGRESS_DISABLED",
            Self::UnsupportedScheme => "UNSUPPORTED_SCHEME",
            Self::InvalidHost => "INVALID_HOST",
            Self::PortNotAllowed => "PORT_NOT_ALLOWED",
            Self::NotAllowlisted => "NOT_ALLOWLISTED",
            Self::ForbiddenAddress => "FORBIDDEN_ADDRESS",
            Self::NoAddress => "NO_ADDRESS",
        }
    }

    /// Parses the stable wire code. No free-form value is accepted.
    pub fn parse(code: &str) -> Result<Self, AppError> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == code)
            .ok_or(AppError::NotAllowed("unknown script incident category"))
    }
}

/// A denial target as received by the application boundary. The lease token is borrowed and this
/// type deliberately has no `Debug` implementation, so ordinary diagnostics cannot print it.
pub enum IncidentTarget<'a> {
    Production {
        namespace_id: NamespaceId,
        run_id: RunId,
        step_id: Uuid,
        attempt: u32,
        generation: u64,
        lease_token: &'a str,
    },
    Test {
        namespace_id: NamespaceId,
        test_run_id: TestRunId,
        lease_token: &'a str,
    },
}

impl IncidentTarget<'_> {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Production { .. } => "production",
            Self::Test { .. } => "test",
        }
    }

    pub(super) fn for_storage(&self) -> Result<StoredIncidentTarget, AppError> {
        let token = match self {
            Self::Production { lease_token, .. } | Self::Test { lease_token, .. } => *lease_token,
        };
        if token.is_empty() || token.len() > MAX_LEASE_TOKEN_BYTES {
            return Err(AppError::NotAllowed(
                "leaseToken is missing or longer than 128 bytes",
            ));
        }
        Ok(match self {
            Self::Production {
                namespace_id,
                run_id,
                step_id,
                attempt,
                generation,
                ..
            } => StoredIncidentTarget::Production {
                namespace_id: *namespace_id,
                run_id: *run_id,
                step_id: *step_id,
                attempt: *attempt,
                generation: *generation,
                token_hash: Token::hash_of(token),
            },
            Self::Test {
                namespace_id,
                test_run_id,
                ..
            } => StoredIncidentTarget::Test {
                namespace_id: *namespace_id,
                test_run_id: *test_run_id,
                token_hash: Token::hash_of(token),
            },
        })
    }
}

/// Secret-free target passed to the storage adapter after boundary validation.
#[doc(hidden)]
pub enum StoredIncidentTarget {
    Production {
        namespace_id: NamespaceId,
        run_id: RunId,
        step_id: Uuid,
        attempt: u32,
        generation: u64,
        token_hash: Vec<u8>,
    },
    Test {
        namespace_id: NamespaceId,
        test_run_id: TestRunId,
        token_hash: Vec<u8>,
    },
}
