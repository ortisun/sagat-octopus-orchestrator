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

/// The level of every field of a database result written by a step or a destination, whatever a declaration says
/// (FR-035-025, D035-051).
pub const DATABASE_RESULT: Level = Level::Pii;

/// Whether a field of `level` may leave the platform through a destination whose ceiling is `ceiling`
/// (FR-035-025): `secret` is always refused, and any other level may leave when it does not exceed the ceiling.
#[must_use]
pub const fn within(level: Level, ceiling: Level) -> bool {
    !matches!(level, Level::Secret) && level as u8 <= ceiling as u8
}

/// The level of a written field: the stricter of the level it inherits from its source and the one a record schema
/// declares, so a declaration can raise a level and never lower it; a field with neither is `pii` (D035-051).
#[must_use]
pub const fn written_level(inherited: Option<Level>, declared: Option<Level>) -> Level {
    match (inherited, declared) {
        (Some(inherited), Some(declared)) => inherited.stricter(declared),
        (Some(level), None) | (None, Some(level)) => level,
        (None, None) => Level::UNMARKED,
    }
}

/// The highest level a connection or a subscription accepts from a step (feature 035, data-model.md section 1).
///
/// Three values: `secret` is never one, because it never leaves. A revision without a ceiling reads as `internal`.
/// Setting `pii` needs the actor to hold the reveal grant for the namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EgressCeiling {
    Public,
    #[default]
    Internal,
    Pii,
}

impl EgressCeiling {
    #[must_use]
    pub const fn as_level(self) -> Level {
        match self {
            Self::Public => Level::Public,
            Self::Internal => Level::Internal,
            Self::Pii => Level::Pii,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.as_level().as_str()
    }

    /// Whether setting this ceiling needs the reveal grant of the namespace.
    #[must_use]
    pub const fn requires_reveal_grant(self) -> bool {
        matches!(self, Self::Pii)
    }

    /// Whether a field of `level` may leave through this ceiling.
    #[must_use]
    pub const fn admits(self, level: Level) -> bool {
        within(level, self.as_level())
    }
}

impl FromStr for EgressCeiling {
    type Err = ClassificationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "public" => Ok(Self::Public),
            "internal" => Ok(Self::Internal),
            "pii" => Ok(Self::Pii),
            _ => Err(ClassificationError::UnknownLevel),
        }
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
