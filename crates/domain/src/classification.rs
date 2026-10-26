//! Pure payload classification and disclosure policy (FR-016-001, FR-016-007, FR-016-009).

use std::fmt;
use std::str::FromStr;

/// A classification name or rules version is outside the closed contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationError {
    UnknownLevel,
    UnsupportedRules(u8),
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLevel => formatter.write_str("unknown classification level"),
            Self::UnsupportedRules(version) => {
                write!(
                    formatter,
                    "unsupported classification rules version {version}"
                )
            }
        }
    }
}

impl std::error::Error for ClassificationError {}

/// Sensitivity assigned to a payload value, from least to most restrictive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Public,
    Internal,
    Pii,
    Secret,
}

impl Level {
    pub const ALL: [Self; 4] = [Self::Public, Self::Internal, Self::Pii, Self::Secret];

    /// The conservative level used when a schema carries no annotation.
    pub const UNMARKED: Self = Self::Pii;

    pub fn parse(value: &str) -> Result<Self, ClassificationError> {
        value.parse()
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Pii => "pii",
            Self::Secret => "secret",
        }
    }

    /// Chooses the level that discloses less information.
    #[must_use]
    pub const fn stricter(self, other: Self) -> Self {
        if self as u8 >= other as u8 {
            self
        } else {
            other
        }
    }
}

impl FromStr for Level {
    type Err = ClassificationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "public" => Ok(Self::Public),
            "internal" => Ok(Self::Internal),
            "pii" => Ok(Self::Pii),
            "secret" => Ok(Self::Secret),
            _ => Err(ClassificationError::UnknownLevel),
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Effective disclosure capabilities for one request.
///
/// `pii` is true only when `internal` is true and the caller both holds a namespace reveal grant
/// and explicitly requested a reveal. The application establishes the grant and the request when
/// it builds this value from authorization facts; [`Audience::new`] enforces the order of the
/// levels (no principal sees a stricter level than a looser one), so `pii` without `internal`
/// cannot be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Audience {
    pub internal: bool,
    pub pii: bool,
    pub structure: bool,
}

impl Audience {
    #[must_use]
    pub const fn new(internal: bool, pii: bool, structure: bool) -> Self {
        Self {
            internal,
            pii: internal && pii,
            structure,
        }
    }
}

/// Why an inspector can see a node's shape but not its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WithheldReason {
    LevelNotPermitted,
    RevealNotRequested,
    Secret,
}

/// Whether a payload value may be returned to this request's audience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Disclose,
    Withhold(WithheldReason),
    Omit,
}

/// Applies the classification audience matrix without reading payload data.
///
/// A use case that knows the caller holds a reveal grant but omitted the explicit reveal request
/// replaces the PII denial reason with [`WithheldReason::RevealNotRequested`]. The effective
/// [`Audience`] deliberately does not carry grant existence independently of a reveal request.
#[must_use]
pub const fn visibility(level: Level, audience: Audience) -> Visibility {
    match level {
        Level::Public => Visibility::Disclose,
        Level::Internal if audience.internal => Visibility::Disclose,
        Level::Pii if audience.pii => Visibility::Disclose,
        Level::Secret if audience.structure => Visibility::Withhold(WithheldReason::Secret),
        Level::Internal | Level::Pii if audience.structure => {
            Visibility::Withhold(WithheldReason::LevelNotPermitted)
        }
        Level::Internal | Level::Pii | Level::Secret => Visibility::Omit,
    }
}

/// Version of the classification interpretation pinned to a published workflow version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClassificationRules(u8);

impl ClassificationRules {
    /// Legacy versions: annotations are not interpreted and values stay undisclosed.
    pub const NONE: Self = Self(0);
    /// The classification policy implemented by feature 016.
    pub const CURRENT: Self = Self(1);

    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for ClassificationRules {
    type Error = ClassificationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NONE),
            1 => Ok(Self::CURRENT),
            version => Err(ClassificationError::UnsupportedRules(version)),
        }
    }
}
