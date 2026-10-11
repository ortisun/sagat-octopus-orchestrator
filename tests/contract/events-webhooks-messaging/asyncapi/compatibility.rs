//! The document against its own past, and the checker against changes it must refuse.

use serde_json::{Value, json};

use crate::{BASELINE, breakages, load, shapes};

#[test]
fn the_messages_have_only_grown_since_the_baseline() {
    let current = shapes(&load());
    if std::env::var_os("SAGAT_UPDATE_BASELINE").is_some() {
        std::fs::write(
            BASELINE,
            serde_json::to_string_pretty(&current).expect("json") + "\n",
        )
        .expect("write the baseline");
        return;
    }
    let baseline: Value =
        serde_json::from_str(&std::fs::read_to_string(BASELINE).expect("the baseline"))
            .expect("baseline JSON");
    let broken = breakages(&baseline, &current);
    assert!(
        broken.is_empty(),
        "the contract was broken:\n{}",
        broken.join("\n")
    );
    // The baseline is not stale in the other direction either: a message added to the document is
    // in it, so the next change is judged against it.
    let missing: Vec<&String> = current
        .as_object()
        .expect("messages")
        .keys()
        .filter(|name| baseline.get(name.as_str()).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "messages not in the baseline (regenerate it): {missing:?}"
    );
}

fn base() -> Value {
    json!({
        "Hello": {
            "headers": {"properties": {}, "required": []},
            "payload": {
                "properties": {
                    "id": {"type": ["string"]},
                    "status": {"type": ["string"], "enum": ["A", "B"]},
                    "n": {"type": ["integer"]},
                },
                "required": ["id", "status"],
            },
        }
    })
}

fn edited(change: impl FnOnce(&mut Value)) -> Vec<String> {
    let before = base();
    let mut after = base();
    change(&mut after);
    breakages(&before, &after)
}

#[test]
fn the_checker_accepts_growth_and_refuses_everything_else() {
    assert!(edited(|_| {}).is_empty());
    // Growth: a new optional property, a new enumeration value, a new message, a required property
    // that stays required.
    assert!(
        edited(|m| {
            m["Hello"]["payload"]["properties"]["extra"] = json!({"type": ["string"]});
            m["Hello"]["payload"]["properties"]["status"]["enum"] = json!(["A", "B", "C"]);
            m["Other"] = json!({"headers": {"properties": {}, "required": []}, "payload": {"properties": {}, "required": []}});
        })
        .is_empty()
    );
    // Breakage, one kind at a time.
    let removed = edited(|m| {
        m["Hello"]["payload"]["properties"]
            .as_object_mut()
            .expect("o")
            .remove("n");
    });
    assert_eq!(removed, vec!["Hello.payload: n was removed".to_owned()]);
    let retyped = edited(|m| m["Hello"]["payload"]["properties"]["n"]["type"] = json!(["string"]));
    assert!(retyped[0].contains("n changed type"), "{retyped:?}");
    let shrunk = edited(|m| m["Hello"]["payload"]["properties"]["status"]["enum"] = json!(["A"]));
    assert!(shrunk[0].contains("lost the value \"B\""), "{shrunk:?}");
    let required = edited(|m| m["Hello"]["payload"]["required"] = json!(["id", "status", "n"]));
    assert_eq!(
        required,
        vec!["Hello.payload: \"n\" became required".to_owned()]
    );
    let gone = edited(|m| {
        m.as_object_mut().expect("o").remove("Hello");
    });
    assert_eq!(gone, vec!["Hello: the message is gone".to_owned()]);
    let header = edited(|m| {
        m["Hello"]["headers"]["properties"]["X-New"] = json!({"type": ["string"]});
        m["Hello"]["headers"]["required"] = json!(["X-New"]);
    });
    assert_eq!(
        header,
        vec!["Hello.headers: \"X-New\" became required".to_owned()]
    );
}

#[test]
fn the_document_describes_every_message_it_sends_or_receives() {
    let document = load();
    let messages = document["components"]["messages"]
        .as_object()
        .expect("messages");
    for (name, channel) in document["channels"].as_object().expect("channels") {
        for (alias, message) in channel["messages"].as_object().expect("channel messages") {
            let target = message["$ref"].as_str().expect("a reference");
            let component = target.rsplit('/').next().expect("name");
            assert!(
                messages.contains_key(component),
                "{name}.{alias} names {component}"
            );
        }
    }
    assert!(
        document["info"]["version"]
            .as_str()
            .is_some_and(|v| v != "0.1.0"),
        "the version moved with the contract"
    );
    assert!(
        document["info"]["description"]
            .as_str()
            .expect("description")
            .contains("only grows"),
        "the evolution policy is stated in the document"
    );
}
