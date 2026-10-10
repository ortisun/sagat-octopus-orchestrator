//! The channel between the supervisor and the code in its sandbox: one JSON object per line on the
//! container's standard output (script to supervisor) and standard input (supervisor to script).
//! Everything the script prints itself goes to standard error, which is its log.

use sagat_application::sandbox::{HttpReply, HttpRequest};
use serde_json::{Value, json};

/// What the sandboxed side says.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The shim is running and the script's code has not been loaded yet.
    Ready,
    /// The script asks for an HTTP request to be made on its behalf.
    Fetch { id: u64, request: HttpRequest },
    /// The script asks for a secret by name.
    Secret { id: u64, name: String },
    /// The script returned.
    Result(Value),
    /// The script raised an error.
    Error { message: String, stack: String },
    /// A build finished: the entry file and the files of the artifact.
    Built {
        entry: String,
        files: Vec<(String, String)>,
    },
    /// A build refused the source, with the toolchain's first message.
    Rejected(String),
}

/// Reads one line. A line that is not a message the supervisor knows is ignored (`None`): the code
/// in the sandbox is not trusted to speak the protocol correctly, and gains nothing by not doing so.
#[must_use]
pub fn parse(line: &[u8]) -> Option<Message> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let id = value.get("id").and_then(Value::as_u64);
    Some(match value.get("op")?.as_str()? {
        "ready" => Message::Ready,
        "fetch" => {
            let headers = value
                .get("headers")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            Message::Fetch {
                id: id?,
                request: HttpRequest {
                    method: text("method").unwrap_or_else(|| "GET".to_owned()),
                    url: text("url")?,
                    headers,
                    body: text("body").map(String::into_bytes).unwrap_or_default(),
                },
            }
        }
        "secret" => Message::Secret {
            id: id?,
            name: text("name")?,
        },
        "result" => Message::Result(value.get("value").cloned().unwrap_or(Value::Null)),
        "error" => Message::Error {
            message: text("message").unwrap_or_default(),
            stack: text("stack").unwrap_or_default(),
        },
        "built" => {
            let files = value
                .get("files")?
                .as_object()?
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect();
            Message::Built {
                entry: text("entry")?,
                files,
            }
        }
        "rejected" => Message::Rejected(text("message").unwrap_or_default()),
        _ => return None,
    })
}

fn line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// The first thing the supervisor says: run these files with this input (JSON). The code travels
/// here and not in the image or in a file put in the container, because the container's filesystem
/// is read-only and nothing of the platform's is mounted into it.
#[must_use]
pub fn run_message(input: &Value, files: &[(String, String)], key: Option<&str>) -> Vec<u8> {
    let files: serde_json::Map<String, Value> = files
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    line(&json!({"op": "run", "input": input, "files": files, "idempotencyKey": key}))
}

/// The first thing the supervisor says to a builder: build this source.
#[must_use]
pub fn build_message(source: &str) -> Vec<u8> {
    line(&json!({"op": "build", "source": source}))
}

#[must_use]
pub fn fetch_reply(id: u64, reply: &HttpReply) -> Vec<u8> {
    let headers: serde_json::Map<String, Value> = reply
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    line(&json!({
        "id": id, "ok": true, "status": reply.status, "headers": headers,
        "body": String::from_utf8_lossy(&reply.body),
    }))
}

#[must_use]
pub fn secret_reply(id: u64, value: &str) -> Vec<u8> {
    line(&json!({"id": id, "ok": true, "value": value}))
}

#[must_use]
pub fn refusal(id: u64, why: &str) -> Vec<u8> {
    line(&json!({"id": id, "ok": false, "error": why}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_of_the_script_are_understood() {
        assert_eq!(parse(br#"{"op":"ready"}"#), Some(Message::Ready));
        assert_eq!(
            parse(br#"{"op":"result","value":{"n":1}}"#),
            Some(Message::Result(json!({"n": 1})))
        );
        assert_eq!(
            parse(br#"{"op":"fetch","id":3,"url":"https://a.example/x","method":"POST","headers":{"k":"v"},"body":"hi"}"#),
            Some(Message::Fetch {
                id: 3,
                request: HttpRequest {
                    method: "POST".into(),
                    url: "https://a.example/x".into(),
                    headers: vec![("k".into(), "v".into())],
                    body: b"hi".to_vec(),
                }
            })
        );
        assert_eq!(
            parse(br#"{"op":"secret","id":4,"name":"stripe"}"#),
            Some(Message::Secret {
                id: 4,
                name: "stripe".into()
            })
        );
        assert_eq!(
            parse(br#"{"op":"built","entry":"main.mjs","files":{"main.mjs":"1"}}"#),
            Some(Message::Built {
                entry: "main.mjs".into(),
                files: vec![("main.mjs".into(), "1".into())]
            })
        );
    }

    #[test]
    fn what_is_not_the_protocol_is_ignored_and_never_trusted() {
        for junk in [
            &b"hello"[..],
            b"",
            b"{}",
            br#"{"op":"unknown"}"#,
            br#"{"op":"fetch","url":"x"}"#,
            br#"{"op":"fetch","id":1}"#,
            br#"{"op":"secret","id":1}"#,
            br#"[1,2]"#,
            br#"{"op":5}"#,
        ] {
            assert_eq!(parse(junk), None, "{}", String::from_utf8_lossy(junk));
        }
    }

    #[test]
    fn replies_are_single_lines() {
        for reply in [
            run_message(
                &json!({"a": "line\nbreak"}),
                &[("main.py".to_owned(), "x = 1\n".to_owned())],
                Some("done-1"),
            ),
            build_message("a\nb"),
            fetch_reply(
                1,
                &HttpReply {
                    status: 200,
                    headers: vec![],
                    body: b"a\nb".to_vec(),
                },
            ),
            secret_reply(2, "va\nlue"),
            refusal(3, "denied"),
        ] {
            let text = String::from_utf8(reply).expect("utf8");
            assert_eq!(text.matches('\n').count(), 1, "{text:?}");
            assert!(text.ends_with('\n'));
        }
    }
}
