//! Pure classification policy invariants (T016-009; FR-016-001, FR-016-007, FR-016-009).

use std::str::FromStr;

use proptest::prelude::*;
use sagat_domain::classification::{
    Audience, ClassificationError, ClassificationRules, DATABASE_RESULT, EgressCeiling, Level,
    Visibility, WithheldReason, visibility, within, written_level,
};

#[test]
fn levels_parse_only_the_four_case_sensitive_contract_names() {
    let levels = [
        ("public", Level::Public),
        ("internal", Level::Internal),
        ("pii", Level::Pii),
        ("secret", Level::Secret),
    ];

    for (name, level) in levels {
        assert_eq!(Level::parse(name), Ok(level));
        assert_eq!(Level::from_str(name), Ok(level));
        assert_eq!(level.as_str(), name);
    }

    for rejected in ["", "Public", "PII", "secret ", "restricted"] {
        assert_eq!(
            Level::from_str(rejected),
            Err(ClassificationError::UnknownLevel),
            "accepted {rejected:?}"
        );
    }
}

#[test]
fn level_order_and_unmarked_default_are_fail_closed() {
    assert!(Level::Public < Level::Internal);
    assert!(Level::Internal < Level::Pii);
    assert!(Level::Pii < Level::Secret);
    assert_eq!(Level::UNMARKED, Level::Pii);

    for left in Level::ALL {
        for right in Level::ALL {
            assert_eq!(left.stricter(right), left.max(right));
        }
    }
}

#[test]
fn visibility_covers_every_valid_audience_cell() {
    use Level::{Internal, Pii, Public, Secret};
    use Visibility::{Disclose, Omit, Withhold};
    use WithheldReason::{LevelNotPermitted, Secret as SecretReason};

    let cases = [
        (
            Audience::new(false, false, false),
            [Disclose, Omit, Omit, Omit],
        ),
        (
            Audience::new(false, false, true),
            [
                Disclose,
                Withhold(LevelNotPermitted),
                Withhold(LevelNotPermitted),
                Withhold(SecretReason),
            ],
        ),
        (
            Audience::new(true, false, false),
            [Disclose, Disclose, Omit, Omit],
        ),
        (
            Audience::new(true, false, true),
            [
                Disclose,
                Disclose,
                Withhold(LevelNotPermitted),
                Withhold(SecretReason),
            ],
        ),
        (
            Audience::new(true, true, false),
            [Disclose, Disclose, Disclose, Omit],
        ),
        (
            Audience::new(true, true, true),
            [Disclose, Disclose, Disclose, Withhold(SecretReason)],
        ),
    ];

    for (audience, expected) in cases {
        for (level, expected_visibility) in
            [Public, Internal, Pii, Secret].into_iter().zip(expected)
        {
            assert_eq!(visibility(level, audience), expected_visibility);
        }
    }
}

#[test]
fn classification_rule_versions_are_stable_and_closed() {
    assert_eq!(ClassificationRules::NONE.as_u8(), 0);
    assert_eq!(ClassificationRules::CURRENT.as_u8(), 1);
    assert_eq!(
        ClassificationRules::try_from(0),
        Ok(ClassificationRules::NONE)
    );
    assert_eq!(
        ClassificationRules::try_from(1),
        Ok(ClassificationRules::CURRENT)
    );
    assert_eq!(
        ClassificationRules::try_from(2),
        Err(ClassificationError::UnsupportedRules(2))
    );
}

proptest! {
    #[test]
    fn effective_level_never_decreases_down_a_path(
        root in 0usize..4,
        descendants in proptest::collection::vec(0usize..4, 0..64),
    ) {
        let mut effective = Level::ALL[root];

        for index in descendants {
            let declared = Level::ALL[index];
            let child = effective.stricter(declared);
            prop_assert!(child >= effective);
            effective = child;
        }
    }
}

/// No principal sees a stricter level than a looser one: an audience that could read `pii` but not
/// `internal` cannot be built, whatever the arguments.
#[test]
fn an_audience_cannot_reveal_pii_without_reading_internal() {
    for structure in [false, true] {
        let audience = Audience::new(false, true, structure);
        assert!(!audience.pii, "pii needs internal");
        assert_eq!(
            visibility(Level::Pii, audience),
            if structure {
                Visibility::Withhold(WithheldReason::LevelNotPermitted)
            } else {
                Visibility::Omit
            }
        );
    }
    assert!(Audience::new(true, true, false).pii);
}

// ---- Classification at egress (feature 035: FR-035-025, D035-051) -----------------------------------------------

/// `secret` is always refused; any other level may leave when it does not exceed the ceiling. The table is written
/// out, rows are the level of the field and columns the ceiling, both in the order public, internal, pii, secret.
#[test]
fn within_holds_for_every_pair_of_the_four_levels() {
    const TABLE: [[bool; 4]; 4] = [
        [true, true, true, true],
        [false, true, true, true],
        [false, false, true, true],
        [false, false, false, false],
    ];
    for (row, level) in Level::ALL.into_iter().enumerate() {
        for (column, ceiling) in Level::ALL.into_iter().enumerate() {
            assert_eq!(
                within(level, ceiling),
                TABLE[row][column],
                "{level} against a ceiling of {ceiling}"
            );
        }
    }
}

#[test]
fn a_secret_field_is_within_no_ceiling() {
    for ceiling in Level::ALL {
        assert!(!within(Level::Secret, ceiling), "{ceiling}");
    }
}

/// The level of a written field is the stricter of the level it inherits from its source and the one a record schema
/// declares. A source that carries no annotation is `pii` under 016 (unmarked), so it is an inherited `pii`, not an
/// absent level: a declaration can only raise a level.
#[test]
fn the_written_level_is_the_stricter_of_the_inherited_and_the_declared_level() {
    use Level::{Internal, Pii, Public, Secret};
    // (inherited, declared, written)
    let cases: [(Option<Level>, Option<Level>, Level); 20] = [
        (None, None, Pii),
        (None, Some(Public), Pii),
        (None, Some(Internal), Pii),
        (None, Some(Pii), Pii),
        (None, Some(Secret), Secret),
        (Some(Public), None, Public),
        (Some(Public), Some(Public), Public),
        (Some(Public), Some(Internal), Internal),
        (Some(Public), Some(Pii), Pii),
        (Some(Public), Some(Secret), Secret),
        (Some(Internal), None, Internal),
        (Some(Internal), Some(Public), Internal),
        (Some(Internal), Some(Pii), Pii),
        (Some(Pii), None, Pii),
        (Some(Pii), Some(Public), Pii),
        (Some(Pii), Some(Internal), Pii),
        (Some(Secret), None, Secret),
        (Some(Secret), Some(Public), Secret),
        (Some(Secret), Some(Pii), Secret),
        (Some(Internal), Some(Secret), Secret),
    ];
    for (inherited, declared, written) in cases {
        assert_eq!(
            written_level(inherited, declared),
            written,
            "inherited {inherited:?}, declared {declared:?}"
        );
    }
}

/// D035-051: a `public` annotation of a record schema is not a disclosure carrier and cannot lower a level.
#[test]
fn a_public_declaration_never_lowers_an_inherited_pii_field() {
    assert_eq!(
        written_level(Some(Level::Pii), Some(Level::Public)),
        Level::Pii
    );
    assert_eq!(
        written_level(Some(Level::Secret), Some(Level::Public)),
        Level::Secret
    );
    assert!(!within(
        written_level(Some(Level::Pii), Some(Level::Public)),
        Level::Internal
    ));
}

/// FR-035-025 reads an unannotated field as `pii` (016, restrictive), so a `public` or `internal` record schema cannot
/// make a field of a source that never classified it leave through a `public` or `internal` ceiling.
#[test]
fn a_declaration_cannot_lower_a_field_whose_source_is_unannotated() {
    for declared in [Level::Public, Level::Internal] {
        let level = written_level(None, Some(declared));
        assert_eq!(level, Level::Pii, "declared {declared}");
        assert!(!within(level, Level::Internal));
    }
}

#[test]
fn a_field_with_neither_level_is_pii() {
    assert_eq!(written_level(None, None), Level::Pii);
    assert!(!within(written_level(None, None), Level::Internal));
    assert!(within(written_level(None, None), Level::Pii));
}

#[test]
fn the_fields_of_a_database_result_are_pii_whatever_a_declaration_says() {
    assert_eq!(DATABASE_RESULT, Level::Pii);
    assert_eq!(
        written_level(Some(DATABASE_RESULT), Some(Level::Public)),
        Level::Pii
    );
}

/// The ceiling of a connection is one of three levels; `secret` is not a value (data-model.md section 1).
#[test]
fn an_egress_ceiling_is_public_internal_or_pii_and_never_secret() {
    for (name, ceiling) in [
        ("public", EgressCeiling::Public),
        ("internal", EgressCeiling::Internal),
        ("pii", EgressCeiling::Pii),
    ] {
        assert_eq!(name.parse::<EgressCeiling>(), Ok(ceiling));
        assert_eq!(ceiling.as_str(), name);
    }
    for rejected in ["secret", "", "Pii", "restricted"] {
        assert_eq!(
            rejected.parse::<EgressCeiling>(),
            Err(ClassificationError::UnknownLevel),
            "accepted {rejected:?}"
        );
    }
}

#[test]
fn the_default_ceiling_is_internal_and_only_pii_needs_the_reveal_grant() {
    assert_eq!(EgressCeiling::default(), EgressCeiling::Internal);
    assert!(EgressCeiling::Pii.requires_reveal_grant());
    assert!(!EgressCeiling::Public.requires_reveal_grant());
    assert!(!EgressCeiling::Internal.requires_reveal_grant());
}

#[test]
fn a_ceiling_admits_exactly_the_levels_up_to_it() {
    use Level::{Internal, Pii, Public};
    for (ceiling, admitted) in [
        (EgressCeiling::Public, vec![Public]),
        (EgressCeiling::Internal, vec![Public, Internal]),
        (EgressCeiling::Pii, vec![Public, Internal, Pii]),
    ] {
        let admits: Vec<Level> = Level::ALL
            .into_iter()
            .filter(|level| ceiling.admits(*level))
            .collect();
        assert_eq!(admits, admitted, "{ceiling:?}");
    }
}
