//! Pure classification policy invariants (T016-009; FR-016-001, FR-016-007, FR-016-009).

use std::str::FromStr;

use proptest::prelude::*;
use sagat_domain::classification::{
    Audience, ClassificationError, ClassificationRules, Level, Visibility, WithheldReason,
    visibility,
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
