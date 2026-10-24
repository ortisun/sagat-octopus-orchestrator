//! Runtime walker evidence for the complete sound corpus and generated ancestor policies.
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    PiiRevealFacts, ProjectionFailure, ProjectionRequest,
};
use sagat_domain::classification::{Audience, ClassificationRules, Level};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};

fn project(document: &[u8], payload: &[u8]) -> Result<ClassifiedNode, ProjectionFailure> {
    Classifier::default()
        .project(ProjectionRequest {
            cache_key: ClassificationCacheKey {
                version: WorkflowVersionId::generate(),
                carrier: CarrierPointer::Input,
            },
            rules: ClassificationRules::CURRENT,
            document,
            payload,
            audience: Audience::new(true, true, true),
            reveal: PiiRevealFacts {
                granted: true,
                requested: true,
            },
        })
        .map(|projection| projection.tree)
}

fn level(node: &ClassifiedNode) -> Level {
    match node {
        ClassifiedNode::Value { level, .. }
        | ClassifiedNode::Container { level, .. }
        | ClassifiedNode::Withheld { level, .. } => *level,
    }
}

fn at<'a>(mut node: &'a ClassifiedNode, path: &[&str]) -> &'a ClassifiedNode {
    for part in path {
        let ClassifiedNode::Container { children, .. } = node else {
            panic!("expected a container")
        };
        node = &children
            .iter()
            .find(|child| {
                child.name.as_deref() == Some(part)
                    || (child.name.is_none() && part.parse::<u32>() == Ok(child.ordinal))
            })
            .expect("expected corpus location")
            .node;
    }
    node
}

#[test]
fn every_sound_corpus_shape_has_the_expected_effective_runtime_levels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/payload-classification/fixtures/sound");
    type CorpusCase = (&'static str, Value, Vec<(&'static [&'static str], Level)>);
    let cases: [CorpusCase; 3] = [
        (
            "levels.json",
            json!({"id":"id","staff":"staff","person":"person","credential":"secret"}),
            vec![
                (&[], Level::Public),
                (&["id"], Level::Public),
                (&["staff"], Level::Internal),
                (&["person"], Level::Pii),
                (&["credential"], Level::Secret),
            ],
        ),
        (
            "objects-and-arrays.json",
            json!({"closed":{"id":"id"},"open":{"dynamic":"value"},"tuple":["first","second",7]}),
            vec![
                (&[], Level::Internal),
                (&["closed", "id"], Level::Internal),
                (&["open", "dynamic"], Level::Pii),
                (&["tuple", "0"], Level::Internal),
                (&["tuple", "1"], Level::Pii),
                (&["tuple", "2"], Level::Secret),
            ],
        ),
        (
            "combiners.json",
            json!({"all":"all","any":"any","one":7}),
            vec![
                (&[], Level::Public),
                (&["all"], Level::Pii),
                (&["any"], Level::Internal),
                (&["one"], Level::Secret),
            ],
        ),
    ];
    for (name, payload, expected) in cases {
        let document = std::fs::read(root.join(name)).expect("sound fixture");
        let payload = serde_json::to_vec(&payload).expect("payload");
        let tree = project(&document, &payload).expect("valid corpus projection");
        for (path, expected) in expected {
            assert_eq!(level(at(&tree, path)), expected, "{name} at {path:?}");
        }
    }
}

#[test]
fn the_reference_expansion_boundary_accepts_one_thousand_and_refuses_the_next() {
    for count in [1000, 1001] {
        let document = serde_json::to_vec(&json!({"inputSchema":{
            "type":"string", "x-sagat-classification":"public",
            "$defs":{"text":{"type":"string"}},
            "allOf":(0..count).map(|_| json!({"$ref":"#/$defs/text"})).collect::<Vec<_>>()
        }}))
        .expect("bounded reference document");
        assert!(document.len() < 64 * 1024);
        let answer = project(&document, br#""text""#);
        if count == 1000 {
            assert_eq!(
                level(&answer.expect("the admitted reference boundary")),
                Level::Public
            );
        } else {
            assert_eq!(answer, Err(ProjectionFailure::SchemaUnusable));
        }
    }
}

proptest! {
    #[test]
    fn generated_runtime_paths_never_lower_an_ancestor_level(
        declarations in proptest::collection::vec(0usize..5, 1..8),
    ) {
        let mut schema = json!({"type":"string"});
        let mut payload = json!("value");
        for (index, declaration) in declarations.iter().enumerate().rev() {
            if index + 1 != declarations.len() {
                schema = json!({"type":"object","additionalProperties":false,"properties":{"child":schema}});
                payload = json!({"child":payload});
            }
            if *declaration < 4 {
                schema["x-sagat-classification"] = json!(Level::ALL[*declaration].as_str());
            }
        }
        let document = serde_json::to_vec(&json!({"inputSchema":schema})).expect("generated document");
        let payload = serde_json::to_vec(&payload).expect("generated payload");
        let tree = project(&document, &payload).expect("bounded generated projection");
        let mut node = &tree;
        let mut effective = Level::ALL.get(declarations[0]).copied().unwrap_or(Level::UNMARKED);
        for (depth, declaration) in declarations.iter().enumerate() {
            if let Some(declared) = Level::ALL.get(*declaration) {
                effective = effective.stricter(*declared);
            }
            prop_assert_eq!(level(node), effective);
            if effective == Level::Secret {
                prop_assert!(matches!(node, ClassifiedNode::Withheld { .. }), "a secret ancestor stops traversal");
                break;
            }
            if depth + 1 != declarations.len() {
                node = at(node, &["child"]);
            }
        }
    }
}
