//! Operator chains (design 042, ADR-0087): a short list of declared stages that shapes a list before anything runs on
//! it. Every predicate, key and projection is an [`Expression`] of the existing bounded language; no stage takes code.
//!
//! One vocabulary has two surfaces. A `forEach` collection (release 0.3.0) has seven operators, in any order; a trigger
//! (a later release) has all nine, in a canonical order that is not built yet. This module holds the rules that need no
//! I/O: the closed set of names, which surface has which, the parameter ranges and the chain length. The codec decodes a
//! chain and checks its expressions; evaluating one is the evaluator's (T042-7).

use std::fmt;

use crate::workflow::Expression;

/// The most stages a chain has (OD-2026-11-13-15, O042-007).
pub const MAX_STAGES: usize = 8;

/// The largest `count` of `skip`, `take` and `window` on a collection: the 0.3.0 collection ceiling (OD-2026-11-13-1).
pub const MAX_COUNT: u32 = 1_000;

/// Where a chain is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Surface {
    /// The `operators` of a `forEach` node, applied once at scope entry.
    Collection,
    /// The `operators` of a trigger (a later release; its fields enter the IR schema with slice S2).
    Trigger,
}

impl Surface {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::Trigger => "trigger",
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The closed set of operator names (FR-042-001). A name outside it is not an operator anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OperatorName {
    Filter,
    Map,
    FlatMap,
    Skip,
    Take,
    Distinct,
    Window,
    /// Trigger only.
    Debounce,
    /// Trigger only.
    IgnoreElements,
}

impl OperatorName {
    /// Every name, in the order of the catalogue (design 042, section 2).
    pub const ALL: [Self; 9] = [
        Self::Filter,
        Self::Map,
        Self::FlatMap,
        Self::Skip,
        Self::Take,
        Self::Distinct,
        Self::Window,
        Self::Debounce,
        Self::IgnoreElements,
    ];

    /// The name as the IR document writes it, the stage's one property.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::Map => "map",
            Self::FlatMap => "flatMap",
            Self::Skip => "skip",
            Self::Take => "take",
            Self::Distinct => "distinct",
            Self::Window => "window",
            Self::Debounce => "debounce",
            Self::IgnoreElements => "ignoreElements",
        }
    }

    /// The operator a document names, exactly as written; `None` for anything else.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|n| n.as_str() == name)
    }

    /// Whether `surface` has this operator: a trigger has all nine, a collection all but `debounce` and
    /// `ignoreElements`, which need time and a summary that a list applied at once does not have.
    #[must_use]
    pub const fn is_on(self, surface: Surface) -> bool {
        match surface {
            Surface::Trigger => true,
            Surface::Collection => !matches!(self, Self::Debounce | Self::IgnoreElements),
        }
    }

    /// # Errors
    /// [`OperatorError::Surface`], naming `stage`, when `surface` does not have this operator.
    pub const fn require_on(self, surface: Surface, stage: usize) -> Result<(), OperatorError> {
        if self.is_on(surface) {
            Ok(())
        } else {
            Err(OperatorError::Surface {
                stage,
                operator: self,
                surface,
            })
        }
    }
}

impl fmt::Display for OperatorName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stage of a collection chain, with its parameters (FR-042-004). A value of this type may hold a count out of
/// range; [`OperatorChain::collection`] is what refuses it, so a chain never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operator {
    /// Keeps the elements for which `where` is `true`.
    Filter { predicate: Expression },
    /// Replaces each element by the value of `select`.
    Map { select: Expression },
    /// Concatenates, in order and one level deep, the lists `select` yields.
    FlatMap { select: Expression },
    /// Drops the first `count` elements (0 to 1,000).
    Skip { count: u32 },
    /// Keeps the first `count` elements (0 to 1,000).
    Take { count: u32 },
    /// Keeps the first element of each canonical value of `by`, or of the element itself when absent. A collection has
    /// no horizon.
    Distinct { by: Option<Expression> },
    /// Replaces the list by consecutive lists of `count` elements (1 to 1,000), the last one possibly shorter.
    Window { count: u32 },
}

impl Operator {
    #[must_use]
    pub const fn name(&self) -> OperatorName {
        match self {
            Self::Filter { .. } => OperatorName::Filter,
            Self::Map { .. } => OperatorName::Map,
            Self::FlatMap { .. } => OperatorName::FlatMap,
            Self::Skip { .. } => OperatorName::Skip,
            Self::Take { .. } => OperatorName::Take,
            Self::Distinct { .. } => OperatorName::Distinct,
            Self::Window { .. } => OperatorName::Window,
        }
    }

    /// The stage's expression and the parameter that holds it, if it has one.
    #[must_use]
    pub const fn expression(&self) -> Option<(&'static str, &Expression)> {
        match self {
            Self::Filter { predicate } => Some(("where", predicate)),
            Self::Map { select } | Self::FlatMap { select } => Some(("select", select)),
            Self::Distinct { by: Some(by) } => Some(("by", by)),
            Self::Distinct { by: None }
            | Self::Skip { .. }
            | Self::Take { .. }
            | Self::Window { .. } => None,
        }
    }

    /// The count and its accepted range on a collection, for the operators that take one.
    const fn count(&self) -> Option<(u32, u32)> {
        match self {
            Self::Skip { count } | Self::Take { count } => Some((*count, 0)),
            Self::Window { count } => Some((*count, 1)),
            _ => None,
        }
    }
}

/// Why a chain is refused. It names the stage and the operator, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperatorError {
    #[error("an operator chain has 1 to {MAX_STAGES} stages, not {0}")]
    Length(usize),
    #[error("operators[{stage}] ({operator}): the count {count} is outside {min} to {max}")]
    Count {
        stage: usize,
        operator: OperatorName,
        count: u32,
        min: u32,
        max: u32,
    },
    #[error("operators[{stage}]: '{operator}' is not an operator of a {surface}")]
    Surface {
        stage: usize,
        operator: OperatorName,
        surface: Surface,
    },
}

/// A validated chain: one to eight stages, each in range, on one surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorChain {
    surface: Surface,
    stages: Vec<Operator>,
}

impl OperatorChain {
    /// The chain of a `forEach`: one to eight stages in any order (D042-008), each over the list the previous one
    /// produced.
    ///
    /// # Errors
    /// [`OperatorError::Length`] first, then [`OperatorError::Count`] for the first stage out of range.
    pub fn collection(stages: Vec<Operator>) -> Result<Self, OperatorError> {
        if !(1..=MAX_STAGES).contains(&stages.len()) {
            return Err(OperatorError::Length(stages.len()));
        }
        for (stage, operator) in stages.iter().enumerate() {
            if let Some((count, min)) = operator.count()
                && !(min..=MAX_COUNT).contains(&count)
            {
                return Err(OperatorError::Count {
                    stage,
                    operator: operator.name(),
                    count,
                    min,
                    max: MAX_COUNT,
                });
            }
        }
        Ok(Self {
            surface: Surface::Collection,
            stages,
        })
    }

    #[must_use]
    pub const fn surface(&self) -> Surface {
        self.surface
    }

    #[must_use]
    pub fn stages(&self) -> &[Operator] {
        &self.stages
    }

    /// Never zero.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// Always `false`: a chain has at least one stage. Present for the `len` convention.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Every expression of the chain with its stage index and parameter name, in stage order.
    pub fn expressions(&self) -> impl Iterator<Item = (usize, &'static str, &Expression)> {
        self.stages
            .iter()
            .enumerate()
            .filter_map(|(stage, op)| op.expression().map(|(field, e)| (stage, field, e)))
    }
}
