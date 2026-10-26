//! Administrator commands for the bounded, audited PII reveal-grant resource.

use serde_json::Value;

use crate::args::Common;
use crate::client::{CallError, Client, encode, with_query};
use crate::{EXIT_DONE, Env, Io, namespace_of, one_line, session};

const PAGE_SIZE: &str = "200";
const MAX_PAGES: usize = 2;
const MAX_GRANTS: usize = 256;
const MAX_NAMESPACE_PAGES: usize = 100;

fn closed_error(error: CallError) -> String {
    match error {
        CallError::Transport(reason) => format!("the platform could not be reached ({reason})"),
        CallError::Refused(failure) => {
            let reason = failure
                .reason
                .as_deref()
                .map(|value| format!(", {}", one_line(value)))
                .unwrap_or_default();
            let trace = failure
                .trace_id
                .as_deref()
                .map(|value| format!(" Trace {}.", one_line(value)))
                .unwrap_or_default();
            format!(
                "the platform refused the request (HTTP {}, {}{reason}).{trace}",
                failure.status,
                one_line(&failure.code)
            )
        }
    }
}

async fn client_for_namespace(
    common: &Common,
    env: &Env,
) -> Result<(Client, String, String), String> {
    let namespace = namespace_of(common, env)?;
    let client = session(common, env, &namespace)?.client;
    let mut cursor: Option<String> = None;
    let mut id = None;
    for _ in 0..MAX_NAMESPACE_PAGES {
        let mut query = vec![("limit", "50")];
        if let Some(value) = cursor.as_deref() {
            query.push(("cursor", value));
        }
        let (_, page) = client
            .call("GET", &with_query("/namespaces", &query), None, None)
            .await
            .map_err(closed_error)?;
        let items = page
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| "the platform's namespace page is not valid".to_owned())?;
        id = items
            .iter()
            .find(|known| known.get("slug").and_then(Value::as_str) == Some(namespace.as_str()))
            .and_then(|known| known.get("id"))
            .and_then(Value::as_str)
            .filter(|id| uuid_shaped(id))
            .map(str::to_owned);
        if id.is_some() {
            break;
        }
        cursor = page
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|next| next.len() <= 1_024)
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    let id = id.ok_or_else(|| format!("the tenant has no namespace '{namespace}'"))?;
    Ok((client, namespace, id))
}

fn uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && value.char_indices().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

fn grant(value: &Value) -> Result<(&str, &str, &str), String> {
    let invalid = || "the platform's reveal-grant answer is not valid".to_owned();
    Ok((
        value
            .get("subject")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?,
        value
            .get("grantedBy")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?,
        value
            .get("grantedAt")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?,
    ))
}

pub(crate) async fn list(common: &Common, env: &Env, io: &mut Io<'_>) -> Result<u8, String> {
    let (client, namespace, id) = client_for_namespace(common, env).await?;
    let base = format!("/namespaces/{id}/payload-reveal-grants");
    let mut cursor: Option<String> = None;
    let mut grants = Vec::new();
    for page in 0..MAX_PAGES {
        let mut query = vec![("limit", PAGE_SIZE)];
        if let Some(value) = cursor.as_deref() {
            query.push(("cursor", value));
        }
        let (_, body) = client
            .call("GET", &with_query(&base, &query), None, None)
            .await
            .map_err(closed_error)?;
        let items = body
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| "the platform's reveal-grant page is not valid".to_owned())?;
        for item in items {
            if grants.len() == MAX_GRANTS {
                return Err("the namespace has more reveal grants than this tool reads".to_owned());
            }
            let (subject, actor, at) = grant(item)?;
            grants.push((subject.to_owned(), actor.to_owned(), at.to_owned()));
        }
        cursor = body
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        if page + 1 == MAX_PAGES {
            return Err("the namespace has more reveal grants than this tool reads".to_owned());
        }
    }
    io.say(format!("payload reveal grants in {namespace}:"));
    if grants.is_empty() {
        io.say("  none");
    }
    for (subject, actor, at) in grants {
        io.say(format!(
            "  {}  granted at {} by {}",
            one_line(&subject),
            one_line(&at),
            one_line(&actor)
        ));
    }
    Ok(EXIT_DONE)
}

pub(crate) async fn grant_subject(
    common: &Common,
    env: &Env,
    subject: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let (client, namespace, id) = client_for_namespace(common, env).await?;
    let path = format!("/namespaces/{id}/payload-reveal-grants/{}", encode(subject));
    let (_, body) = client
        .call("PUT", &path, None, None)
        .await
        .map_err(closed_error)?;
    let (returned_subject, actor, at) = grant(&body)?;
    if returned_subject != subject {
        return Err("the platform's reveal-grant answer is not valid".to_owned());
    }
    io.say(format!(
        "granted payload reveal to {} in {namespace} at {} by {}",
        one_line(returned_subject),
        one_line(at),
        one_line(actor)
    ));
    Ok(EXIT_DONE)
}

pub(crate) async fn revoke_subject(
    common: &Common,
    env: &Env,
    subject: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let (client, namespace, id) = client_for_namespace(common, env).await?;
    let path = format!("/namespaces/{id}/payload-reveal-grants/{}", encode(subject));
    client
        .call("DELETE", &path, None, None)
        .await
        .map_err(closed_error)?;
    io.say(format!(
        "revoked payload reveal from {} in {namespace}",
        one_line(subject)
    ));
    Ok(EXIT_DONE)
}
